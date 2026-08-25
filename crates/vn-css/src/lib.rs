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

mod animator;
mod element;
mod mapper;
mod transition;

use std::rc::Rc;

use gpui::{Refineable, StyleRefinement};
use lightningcss::rules::CssRule;
use lightningcss::selector::{PseudoClass, Selector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use parcel_selectors::context::{MatchingContext, MatchingMode, QuirksMode};
use parcel_selectors::matching::matches_selector;

pub use animator::Animator;
pub use element::MatchDom;
pub use mapper::{parse_inline, parse_inline_with_transitions};
pub use transition::{
    covered, spec_for, Easing, StepPosition, TransitionProperty, TransitionSpec, ANIMATABLE_COUNT,
};

const STATE_HOVER: u8 = 1;
const STATE_ACTIVE: u8 = 2;

/// The folded cascade result for one node — exactly the shape gpui's
/// interactivity ladder consumes, plus the `transition` list the animator
/// diffs against.
///
/// `PartialEq` is the cheap "did this node's style actually change?" test the
/// restyle pass needs before calling [`Animator::on_computed`] (gpui's
/// `StyleRefinement` derives `PartialEq` through the refineable macro).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComputedStates {
    pub base: Rc<StyleRefinement>,
    pub hover: Option<Rc<StyleRefinement>>,
    pub active: Option<Rc<StyleRefinement>>,
    /// The folded `transition` list. Empty means "no transitions".
    pub transitions: Rc<Vec<TransitionSpec>>,
}

struct RuleEntry {
    selector: &'static Selector<'static>,
    normal: Option<Rc<StyleRefinement>>,
    important: Option<Rc<StyleRefinement>>,
    normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    important_transitions: Option<Rc<Vec<TransitionSpec>>>,
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
                at_rule_ignored(rule, &mut diagnostics);
                continue;
            };
            let converted = mapper::convert_block(&style.declarations, &mut diagnostics);
            let declares_transition =
                converted.normal_transitions.is_some() || converted.important_transitions.is_some();
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
                if states != 0 && declares_transition {
                    // `:hover`/`:active` refinements are installed into gpui's
                    // own state ladder and never pass through our animator, so
                    // a `transition` declared on a state rule cannot run.
                    // Declare it on the base rule instead (that is also what
                    // authors mean 99% of the time).
                    diagnostics.push(format!(
                        "transition declared on a state selector is ignored (v1) — move it to the \
                         base rule: {}",
                        sel_text(sel)
                    ));
                }
                // `sheet` is a leaked &'static, so its selectors are 'static too.
                rules.push(RuleEntry {
                    selector: sel,
                    normal: converted.normal.clone(),
                    important: converted.important.clone(),
                    normal_transitions: converted.normal_transitions.clone(),
                    important_transitions: converted.important_transitions.clone(),
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
        self.compute_with(node, inline, None)
    }

    /// [`Self::compute`], additionally folding the transitions an inline
    /// `style` declared (parse them with
    /// [`parse_inline_with_transitions`]). Inline is the last cascade origin,
    /// so an inline `transition` wins over every matched rule.
    pub fn compute_with<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&StyleRefinement>,
        inline_transitions: Option<&Rc<Vec<TransitionSpec>>>,
    ) -> ComputedStates {
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
        // Transitions do not merge: the last base-state rule that declares any
        // `transition` replaces the whole list (`transition: none` therefore
        // clears it). State-rule transitions are rejected at compile time.
        let mut transitions: Option<Rc<Vec<TransitionSpec>>> = None;
        let mut fold = |important_pass: bool| {
            for r in &matched {
                if r.states == 0 {
                    let declared = if important_pass {
                        &r.important_transitions
                    } else {
                        &r.normal_transitions
                    };
                    if let Some(t) = declared {
                        transitions = Some(t.clone());
                    }
                }
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
        if let Some(t) = inline_transitions {
            transitions = Some(t.clone());
        }
        ComputedStates {
            base: Rc::new(base),
            hover: any_hover.then(|| Rc::new(hover)),
            active: any_active.then(|| Rc::new(active)),
            transitions: transitions.unwrap_or_default(),
        }
    }
}

/// One advisory PER AT-RULE KIND, not per rule: a generated sheet carries one
/// `@media` block per responsive/dark variant in the app (a broad Tailwind
/// sweep produced nine), and nine truncated `Debug` dumps bury everything else.
/// The message says what silently will not apply, which is the part that costs
/// an author an afternoon.
fn at_rule_ignored(rule: &CssRule<'_>, diagnostics: &mut Vec<String>) {
    let msg = match rule {
        CssRule::Media(_) => "@media rules are dropped (v1) — responsive and `dark:` variants \
                              (`sm:`, `md:`, `lg:`, `print:`, `motion-reduce:`) do not apply"
            .to_string(),
        CssRule::Supports(_) => {
            "@supports rules are dropped (v1) — the whole block never applies".to_string()
        }
        CssRule::Keyframes(_) => "@keyframes are dropped (v1) — CSS animations are not implemented; \
                                  `transition` is"
            .to_string(),
        CssRule::FontFace(_) => "@font-face is dropped (v1) — only system fonts resolve".to_string(),
        other => format!("at-rule ignored (v1): {other:?}")
            .chars()
            .take(120)
            .collect(),
    };
    mapper::once(diagnostics, msg);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A single classed element with no parent or siblings.
    #[derive(Clone, Copy)]
    struct Node {
        tag: &'static str,
        class: &'static str,
    }

    impl MatchDom for Node {
        fn parent(&self) -> Option<Self> {
            None
        }
        fn prev_sibling(&self) -> Option<Self> {
            None
        }
        fn next_sibling(&self) -> Option<Self> {
            None
        }
        fn has_tag(&self, name: &str) -> bool {
            self.tag == name
        }
        fn tag_eq(&self, other: &Self) -> bool {
            self.tag == other.tag
        }
        fn has_class(&self, name: &str) -> bool {
            self.class.split_whitespace().any(|c| c == name)
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

    fn box_node() -> Node {
        Node {
            tag: "div",
            class: "box",
        }
    }

    fn transitions(css: &str) -> Vec<TransitionSpec> {
        let engine = Engine::compile(css);
        (*engine.compute(box_node(), None).transitions).clone()
    }

    #[test]
    fn cascade_carries_a_rules_transition() {
        let t = transitions(".box { transition: opacity 200ms linear }");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, TransitionProperty::Opacity);
        assert_eq!(t[0].duration, Duration::from_millis(200));
    }

    #[test]
    fn a_node_with_no_transition_rule_gets_an_empty_list() {
        assert!(transitions(".box { color: red }").is_empty());
        // A non-matching rule must not leak its transition.
        assert!(transitions(".other { transition: opacity 1s }").is_empty());
    }

    #[test]
    fn later_rules_replace_the_whole_list() {
        let t = transitions(
            ".box { transition: opacity 1s, color 1s } \
             div { transition: width 300ms ease-in }",
        );
        // `.box` has higher specificity, so it wins regardless of source order.
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].property, TransitionProperty::Opacity);

        let t = transitions(
            ".box { transition: opacity 1s, color 1s } \
             .box { transition: width 300ms ease-in }",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, TransitionProperty::Width);
        assert_eq!(t[0].easing, Easing::EaseIn);
    }

    #[test]
    fn transition_none_in_a_later_rule_clears() {
        let t = transitions(
            ".box { transition: opacity 1s } \
             .box { transition: none }",
        );
        assert!(t.is_empty());
    }

    #[test]
    fn important_transitions_win_over_normal_ones() {
        let t = transitions(
            ".box { transition: opacity 1s !important } \
             div.box { transition: width 2s }",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, TransitionProperty::Opacity);
    }

    #[test]
    fn state_rule_transitions_are_rejected_with_a_diagnostic() {
        let engine = Engine::compile(".box:hover { transition: opacity 1s; opacity: 0.5 }");
        assert!(
            engine
                .diagnostics
                .iter()
                .any(|d| d.contains("transition declared on a state selector")),
            "{:?}",
            engine.diagnostics
        );
        let states = engine.compute(box_node(), None);
        assert!(states.transitions.is_empty());
        // …but the state refinement itself still lands.
        assert!(states.hover.is_some());
    }

    #[test]
    fn inline_transitions_override_the_sheet() {
        let engine = Engine::compile(".box { transition: opacity 1s }");
        let mut diagnostics = Vec::new();
        let (inline, inline_transitions) =
            parse_inline_with_transitions("transition: width 50ms linear", &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let states = engine.compute_with(
            box_node(),
            Some(&inline),
            Some(&Rc::new(inline_transitions)),
        );
        assert_eq!(states.transitions.len(), 1);
        assert_eq!(states.transitions[0].property, TransitionProperty::Width);
        assert_eq!(states.transitions[0].duration, Duration::from_millis(50));
    }

    #[test]
    fn transition_declarations_do_not_leak_into_the_refinement_diagnostics() {
        let engine = Engine::compile(
            ".box { transition: opacity 1s; transition-delay: 20ms; opacity: 0.5 }",
        );
        assert!(
            !engine
                .diagnostics
                .iter()
                .any(|d| d.contains("property not mapped")),
            "{:?}",
            engine.diagnostics
        );
        let states = engine.compute(box_node(), None);
        assert_eq!(states.base.opacity, Some(0.5));
        assert_eq!(states.transitions[0].delay, Duration::from_millis(20));
    }

    /// Tailwind splits one CSS `transition` across TWO utility classes, and
    /// vn-css folds transitions whole-list (README deviation 2). Pinned here
    /// because the combination is what every Tailwind app writes:
    ///
    ///   .transition-colors  property list + cubic-bezier(.4,0,.2,1) + 150ms
    ///   .duration-300       transition-duration: 300ms          <- emitted LAST
    ///
    /// A browser cascades the two per-longhand and keeps the property list. We
    /// let the later rule replace the whole list, so the result is `all 300ms
    /// ease`. Background-color still transitions (it is inside `all`), but the
    /// property narrowing and Tailwind's easing curve are lost.
    #[test]
    fn tailwinds_transition_colors_plus_duration_300_folds_to_all() {
        let t = transitions(
            ".box { /* .transition-colors */ \
               transition-property: color, background-color, border-color, \
                                    text-decoration-color, fill, stroke; \
               transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1); \
               transition-duration: 150ms } \
             .box { /* .duration-300 */ transition-duration: 300ms }",
        );
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].property, TransitionProperty::All);
        assert_eq!(t[0].duration, Duration::from_millis(300));
        assert_eq!(t[0].easing, Easing::Ease, "Tailwind's curve does not survive");
        // The point that matters for the demo card: background-color IS covered.
        assert!(spec_for(&t, &TransitionProperty::BackgroundColor).is_some());
    }

    /// `transition-colors` on its own keeps Tailwind's list and curve.
    #[test]
    fn tailwinds_transition_colors_alone_keeps_its_list_and_curve() {
        let t = transitions(
            ".box { /* .transition-colors */ \
               transition-property: color, background-color, border-color, \
                                    text-decoration-color, fill, stroke; \
               transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1); \
               transition-duration: 150ms }",
        );
        assert_eq!(t.len(), 6);
        assert_eq!(t[1].property, TransitionProperty::BackgroundColor);
        assert_eq!(t[1].duration, Duration::from_millis(150));
        assert_eq!(t[1].easing, Easing::CubicBezier(0.4, 0.0, 0.2, 1.0));
        // `fill` / `stroke` have no gpui slot — they become Other, not an error.
        assert!(matches!(t[4].property, TransitionProperty::Other(_)));
    }

    #[test]
    fn computed_states_compare_by_value() {
        let engine = Engine::compile(".box { transition: opacity 1s; opacity: 0.5 }");
        let a = engine.compute(box_node(), None);
        let b = engine.compute(box_node(), None);
        assert_eq!(a, b);

        let other = Engine::compile(".box { transition: opacity 1s; opacity: 0.6 }")
            .compute(box_node(), None);
        assert_ne!(a, other);
    }
}
