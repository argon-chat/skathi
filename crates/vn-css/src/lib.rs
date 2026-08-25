//! vn-css — the CSS layer: lightningcss parses stylesheets, parcel_selectors
//! matches them against the retained tree, and the cascade folds matched rules
//! into precomputed per-state `gpui::StyleRefinement`s that the renderer
//! installs into gpui's own state ladder (base / hover / active).
//!
//! Design (docs/ARCHITECTURE.md §7, proven by the research spikes):
//! - The cascade runs ONCE per tree mutation (the restyle pass in the frame
//!   pump) — never inside gpui's per-frame style computation.
//! - `:hover` / `:active` never round-trip through JS: the cascade emits one
//!   folded refinement per state and gpui's hitbox machinery picks it live.
//! - Selectors the matcher cannot run (`:has()`, unflattened nesting,
//!   `:nth-child(.. of S)`) PANIC inside parcel_selectors — they are probed
//!   and rejected at sheet-compile time with a diagnostic.

mod element;
mod mapper;

use std::rc::Rc;

use gpui::{Refineable, StyleRefinement};
use lightningcss::rules::CssRule;
use lightningcss::selector::{PseudoClass, Selector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use parcel_selectors::context::{MatchingContext, MatchingMode, QuirksMode};
use parcel_selectors::matching::matches_selector;

pub use element::MatchDom;
pub use mapper::parse_inline;

const STATE_HOVER: u8 = 1;
const STATE_ACTIVE: u8 = 2;

/// The folded cascade result for one node — exactly the shape gpui's
/// interactivity ladder consumes.
#[derive(Clone, Default)]
pub struct ComputedStates {
    pub base: Rc<StyleRefinement>,
    pub hover: Option<Rc<StyleRefinement>>,
    pub active: Option<Rc<StyleRefinement>>,
}

struct RuleEntry {
    selector: &'static Selector<'static>,
    normal: Option<Rc<StyleRefinement>>,
    important: Option<Rc<StyleRefinement>>,
    specificity: u32,
    order: u32,
    states: u8,
}

/// A compiled stylesheet: rules with matchable selectors, pre-converted
/// declaration blocks, specificity and state classification.
///
/// The source is intentionally leaked: `lightningcss::StyleSheet` borrows it,
/// and a stylesheet set lives until replaced (a few KB per HMR swap — the
/// documented tradeoff until `into_owned` is wired).
pub struct Engine {
    rules: Vec<RuleEntry>,
    pub diagnostics: Vec<String>,
}

impl Engine {
    pub fn compile(css: &str) -> Engine {
        let mut diagnostics = Vec::new();
        let src: &'static str = Box::leak(css.to_string().into_boxed_str());
        let sheet: &'static StyleSheet<'static> = match StyleSheet::parse(
            src,
            ParserOptions::default(),
        ) {
            Ok(s) => Box::leak(Box::new(s)),
            Err(e) => {
                diagnostics.push(format!("stylesheet parse error: {e}"));
                return Engine { rules: Vec::new(), diagnostics };
            }
        };

        let mut rules = Vec::new();
        let mut order = 0u32;
        for rule in &sheet.rules.0 {
            let CssRule::Style(style) = rule else {
                diagnostics.push(format!("at-rule ignored (v1): {rule:?}").chars().take(120).collect());
                continue;
            };
            let (normal, important) = mapper::convert_block(&style.declarations, &mut diagnostics);
            for sel in &style.selectors.0 {
                let Some(states) = classify_states(sel, &mut diagnostics) else {
                    continue;
                };
                if !probe_runnable(sel) {
                    diagnostics.push(format!(
                        "selector rejected (matcher cannot run it — :has()/nesting?): {}",
                        sel_text(sel)
                    ));
                    continue;
                }
                // `sheet` is a leaked &'static, so its selectors are 'static too.
                rules.push(RuleEntry {
                    selector: sel,
                    normal: normal.clone(),
                    important: important.clone(),
                    specificity: sel.specificity(),
                    order,
                    states,
                });
                order += 1;
            }
        }
        Engine { rules, diagnostics }
    }

    /// The cascade for one node: match every rule (state pseudos treated as
    /// "would match"), bucket by state, fold by (specificity, order) with
    /// `!important` on top, inline style last.
    pub fn compute<D: MatchDom>(&self, node: D, inline: Option<&StyleRefinement>) -> ComputedStates {
        let el = element::El(node);
        let mut matched: Vec<&RuleEntry> = Vec::new();
        for rule in &self.rules {
            let mut ctx: MatchingContext<'_, '_, element::Impl> =
                MatchingContext::new(MatchingMode::Normal, None, None, QuirksMode::NoQuirks);
            if matches_selector(rule.selector, 0, None, &el, &mut ctx, &mut |_, _| {}) {
                matched.push(rule);
            }
        }
        matched.sort_by_key(|r| (r.specificity, r.order));

        let mut base = StyleRefinement::default();
        let mut hover = StyleRefinement::default();
        let mut active = StyleRefinement::default();
        let (mut any_hover, mut any_active) = (false, false);
        let mut fold = |important_pass: bool| {
            for r in &matched {
                let Some(refinement) = (if important_pass { &r.important } else { &r.normal }) else {
                    continue;
                };
                match r.states {
                    0 => base.refine(refinement),
                    s if s & STATE_ACTIVE != 0 => {
                        active.refine(refinement);
                        any_active = true;
                    }
                    _ => {
                        hover.refine(refinement);
                        any_hover = true;
                    }
                }
            }
        };
        fold(false);
        fold(true);
        if let Some(inline) = inline {
            base.refine(inline);
        }
        ComputedStates {
            base: Rc::new(base),
            hover: any_hover.then(|| Rc::new(hover)),
            active: any_active.then(|| Rc::new(active)),
        }
    }
}

/// Classifies a selector's interaction-state bucket from its RIGHTMOST
/// compound. State pseudos further left (`.a:hover .b`) need gpui's group
/// mechanism — unsupported in v1, rejected with a diagnostic (`None`).
fn classify_states(sel: &Selector<'_>, diagnostics: &mut Vec<String>) -> Option<u8> {
    let mut states = 0u8;
    let mut iter = sel.iter();
    // Rightmost compound.
    for component in &mut iter {
        if let parcel_selectors::parser::Component::NonTSPseudoClass(pc) = component {
            match pc {
                PseudoClass::Hover => states |= STATE_HOVER,
                PseudoClass::Active => states |= STATE_ACTIVE,
                _ => {}
            }
        }
    }
    // Remaining compounds: any state pseudo there is an ancestor-state rule.
    while iter.next_sequence().is_some() {
        for component in &mut iter {
            if let parcel_selectors::parser::Component::NonTSPseudoClass(pc) = component {
                if matches!(pc, PseudoClass::Hover | PseudoClass::Active) {
                    diagnostics.push(format!(
                        "ancestor-state selector unsupported (v1): {}",
                        sel_text(sel)
                    ));
                    return None;
                }
            }
        }
    }
    Some(states)
}

/// parcel_selectors' matcher `panic!`s on components its parser accepts
/// (`:has()`, `&` nesting, `:nth-child(of S)`). Probe once at compile time
/// against a dummy element so a bad selector costs a diagnostic, not the app.
fn probe_runnable(sel: &Selector<'_>) -> bool {
    #[derive(Clone, Copy)]
    struct Probe;
    impl MatchDom for Probe {
        fn parent(&self) -> Option<Self> {
            None
        }
        fn prev_sibling(&self) -> Option<Self> {
            None
        }
        fn next_sibling(&self) -> Option<Self> {
            None
        }
        fn has_tag(&self, _: &str) -> bool {
            false
        }
        fn tag_eq(&self, _: &Self) -> bool {
            true
        }
        fn has_class(&self, _: &str) -> bool {
            false
        }
        fn has_scope(&self, _: &str) -> bool {
            false
        }
        fn is_empty(&self) -> bool {
            true
        }
        fn is_root(&self) -> bool {
            true
        }
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let el = element::El(Probe);
        let mut ctx: MatchingContext<'_, '_, element::Impl> =
            MatchingContext::new(MatchingMode::Normal, None, None, QuirksMode::NoQuirks);
        matches_selector(sel, 0, None, &el, &mut ctx, &mut |_, _| {});
    }))
    .is_ok()
}

fn sel_text(sel: &Selector<'_>) -> String {
    use lightningcss::traits::ToCss;
    sel.to_css_string(lightningcss::printer::PrinterOptions::default())
        .unwrap_or_else(|_| "<selector>".into())
}
