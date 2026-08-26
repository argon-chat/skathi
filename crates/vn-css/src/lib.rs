//! vn-css — the CSS layer: lightningcss parses stylesheets, parcel_selectors
//! matches them against the retained tree, and the cascade folds matched rules
//! into precomputed per-state `gpui::StyleRefinement`s that the renderer
//! installs into gpui's own state ladder (base / focus / focus-visible /
//! hover / active).
//!
//! Design (docs/ARCHITECTURE.md §7, proven by the research spikes):
//! - The cascade runs ONCE per tree mutation (the restyle pass in the frame
//!   pump) — never inside gpui's per-frame style computation.
//! - `:hover` / `:active` / `:focus` / `:focus-visible` never round-trip
//!   through JS: the cascade emits one folded refinement per state and gpui's
//!   hitbox / focus machinery picks it live.
//! - `:disabled` / `:checked` are NOT ladder states: the element property IS
//!   the state, so they are matched for real and fold into the base bucket.
//! - `@media` preludes are parsed into a small condition tree ([`media`]) and
//!   evaluated against an [`Env`] the host refreshes every frame.
//! - Selectors the matcher cannot run (`:has()`, unflattened nesting,
//!   `:nth-child(.. of S)`) PANIC inside parcel_selectors — they are probed
//!   and rejected at sheet-compile time with a diagnostic. Selectors it *can*
//!   run but could only ever answer "no" (`:visited`, `::before`, …) are caught
//!   by the compile-time audit ([`audit_selector`]) and reported, never
//!   compiled into a rule that silently never matches.

mod animator;
mod element;
mod mapper;
mod media;
mod transition;

use std::rc::Rc;

use gpui::{Refineable, StyleRefinement};
use lightningcss::rules::CssRule;
use lightningcss::selector::{PseudoClass, Selector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use parcel_selectors::context::{MatchingContext, MatchingMode, QuirksMode};
use parcel_selectors::matching::matches_selector;
use parcel_selectors::parser::Component;

pub use animator::Animator;
pub use element::MatchDom;
pub use mapper::{
    parse_inline, parse_inline_full, parse_inline_with_transitions, Extras, ObjectFit,
};
pub use media::Env;
pub use transition::{
    covered, spec_for, Easing, StepPosition, TransitionProperty, TransitionSpec, ANIMATABLE_COUNT,
};

/// Interaction-state buckets. These are exactly the states gpui's own
/// interactivity ladder can pick live, in gpui's application order (later wins
/// — see `Interactivity::compute_style_internal`).
const STATE_FOCUS: u8 = 1;
const STATE_FOCUS_VISIBLE: u8 = 2;
const STATE_HOVER: u8 = 4;
const STATE_ACTIVE: u8 = 8;

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
    /// `:focus` — gpui's `.focus()` rung. A node that has one MUST be given a
    /// tracked focus handle by the renderer, or gpui has nothing to key it on.
    pub focus: Option<Rc<StyleRefinement>>,
    /// `:focus-visible` — gpui's `.focus_visible()` rung (focused *and* the
    /// last input was a keystroke), the real browser semantics.
    pub focus_visible: Option<Rc<StyleRefinement>>,
    /// The folded `transition` list. Empty means "no transitions".
    pub transitions: Rc<Vec<TransitionSpec>>,
    /// Computed properties a `StyleRefinement` cannot carry (`object-fit`).
    /// Folded from the BASE state only — see [`Extras`].
    pub extras: Extras,
}

impl ComputedStates {
    /// True when the renderer has to give this node a focus handle for gpui to
    /// have anything to hang the focus rungs on.
    pub fn needs_focus_handle(&self) -> bool {
        self.focus.is_some() || self.focus_visible.is_some()
    }
}

struct RuleEntry {
    selector: &'static Selector<'static>,
    normal: Option<Rc<StyleRefinement>>,
    important: Option<Rc<StyleRefinement>>,
    normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    important_transitions: Option<Rc<Vec<TransitionSpec>>>,
    normal_extras: Extras,
    important_extras: Extras,
    specificity: u32,
    order: u32,
    states: u8,
    /// Index into [`Engine::conditions`] — the `@media` block this rule came
    /// from, if any.
    condition: Option<u32>,
}

/// A compiled stylesheet: rules with matchable selectors, pre-converted
/// declaration blocks, specificity and state classification.
///
/// The source is intentionally leaked: `lightningcss::StyleSheet` borrows it,
/// and a stylesheet set lives until replaced (a few KB per HMR swap — the
/// documented tradeoff until `into_owned` is wired).
pub struct Engine {
    rules: Vec<RuleEntry>,
    /// One entry per distinct `@media` prelude in the sheet; rules point at
    /// theirs by index. Evaluated against the [`Env`] on every cascade.
    conditions: Vec<media::Cond>,
    pub diagnostics: Vec<String>,
}

impl Engine {
    pub fn compile(css: &str) -> Engine {
        let mut engine = Engine { rules: Vec::new(), conditions: Vec::new(), diagnostics: Vec::new() };
        let src: &'static str = Box::leak(css.to_string().into_boxed_str());
        let sheet: &'static StyleSheet<'static> =
            match StyleSheet::parse(src, ParserOptions::default()) {
                Ok(s) => Box::leak(Box::new(s)),
                Err(e) => {
                    engine.diagnostics.push(format!("stylesheet parse error: {e}"));
                    return engine;
                }
            };
        let mut order = 0u32;
        engine.collect(&sheet.rules.0, None, &mut order);
        engine
    }

    /// Walks one rule list, descending into `@media` blocks so their style
    /// rules join the same flat list with a condition index. `order` is shared
    /// across the whole walk, so document order (the cascade tiebreak) survives
    /// nesting.
    fn collect(
        &mut self,
        rules: &'static [CssRule<'static>],
        condition: Option<u32>,
        order: &mut u32,
    ) {
        for rule in rules {
            let style = match rule {
                CssRule::Style(style) => style,
                CssRule::Media(m) => {
                    match media::parse_list(&m.query) {
                        Ok(cond) => {
                            let idx = self.push_condition(cond, condition);
                            self.collect(&m.rules.0, Some(idx), order);
                        }
                        Err(reason) => mapper::once(
                            &mut self.diagnostics,
                            format!(
                                "@media {} uses {reason} — the whole block was dropped ({})",
                                media::list_text(&m.query),
                                media::SUPPORTED
                            ),
                        ),
                    }
                    continue;
                }
                other => {
                    at_rule_ignored(other, &mut self.diagnostics);
                    continue;
                }
            };
            let converted = mapper::convert_block(&style.declarations, &mut self.diagnostics);
            let declares_transition =
                converted.normal_transitions.is_some() || converted.important_transitions.is_some();
            let declares_extras = {
                let mut e = converted.normal_extras;
                e.refine(&converted.important_extras);
                e
            };
            for sel in &style.selectors.0 {
                let Some(states) = classify_states(sel, &mut self.diagnostics) else {
                    continue;
                };
                if !probe_runnable(sel) {
                    self.diagnostics.push(format!(
                        "selector rejected (matcher cannot run it — :has()/nesting?): {}",
                        sel_text(sel)
                    ));
                    continue;
                }
                if !audit_selector(sel, &mut self.diagnostics) {
                    continue;
                }
                // gpui's `img()` carries ONE `ObjectFit` for the element, not a
                // per-state ladder like the refinements — so `object-fit` in a
                // `:hover` rule would apply all the time. Say so rather than
                // let it look conditional.
                if states != 0 && !declares_extras.is_empty() {
                    self.diagnostics.push(format!(
                        "`object-fit` on a state selector applies unconditionally (v1) — gpui's \
                         image element has a single object-fit, not a state ladder: {}",
                        sel_text(sel)
                    ));
                }
                if states != 0 && declares_transition {
                    // State refinements are installed into gpui's own state
                    // ladder and never pass through our animator, so a
                    // `transition` declared on a state rule cannot run. Declare
                    // it on the base rule instead (that is also what authors
                    // mean 99% of the time).
                    self.diagnostics.push(format!(
                        "transition declared on a state selector is ignored (v1) — move it to the \
                         base rule: {}",
                        sel_text(sel)
                    ));
                }
                // `sheet` is a leaked &'static, so its selectors are 'static too.
                self.rules.push(RuleEntry {
                    selector: sel,
                    normal: converted.normal.clone(),
                    important: converted.important.clone(),
                    normal_transitions: converted.normal_transitions.clone(),
                    important_transitions: converted.important_transitions.clone(),
                    normal_extras: converted.normal_extras,
                    important_extras: converted.important_extras,
                    specificity: sel.specificity(),
                    order: *order,
                    states,
                    condition,
                });
                *order += 1;
            }
        }
    }

    /// Interns a condition, `and`-ing it with the enclosing block's when
    /// `@media` blocks nest. Identical conditions share one slot — a Tailwind
    /// sheet repeats `(min-width: 640px)` once per emitted block.
    fn push_condition(&mut self, cond: media::Cond, parent: Option<u32>) -> u32 {
        let cond = match parent {
            Some(p) => self.conditions[p as usize].clone().and(cond),
            None => cond,
        };
        if let Some(i) = self.conditions.iter().position(|c| *c == cond) {
            return i as u32;
        }
        self.conditions.push(cond);
        (self.conditions.len() - 1) as u32
    }

    /// The cascade for one node: match every rule whose `@media` condition
    /// holds in `env` (ladder-state pseudos treated as "would match", property
    /// pseudos like `:disabled` matched for real), bucket by state, fold by
    /// (specificity, order) with `!important` on top, inline style last.
    pub fn compute<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&StyleRefinement>,
        env: &Env,
    ) -> ComputedStates {
        self.compute_with(node, inline, None, env)
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
        env: &Env,
    ) -> ComputedStates {
        self.compute_full(node, inline, inline_transitions, Extras::default(), env)
    }

    /// [`Self::compute_with`], additionally folding the [`Extras`] an inline
    /// `style` declared (parse them with [`parse_inline_full`]).
    pub fn compute_full<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&StyleRefinement>,
        inline_transitions: Option<&Rc<Vec<TransitionSpec>>>,
        inline_extras: Extras,
        env: &Env,
    ) -> ComputedStates {
        let el = element::El(node);
        let mut matched: Vec<&RuleEntry> = Vec::new();
        for rule in &self.rules {
            // Cheaper than matching, and it has to run first anyway: a rule
            // inside a `@media` block that does not hold is simply not there.
            if let Some(c) = rule.condition {
                if !self.conditions[c as usize].eval(env) {
                    continue;
                }
            }
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
        let mut focus = StyleRefinement::default();
        let mut focus_visible = StyleRefinement::default();
        let (mut any_hover, mut any_active) = (false, false);
        let (mut any_focus, mut any_focus_visible) = (false, false);
        // Transitions do not merge: the last base-state rule that declares any
        // `transition` replaces the whole list (`transition: none` therefore
        // clears it). State-rule transitions are rejected at compile time.
        let mut transitions: Option<Rc<Vec<TransitionSpec>>> = None;
        // `object-fit` has no state ladder to install into (compile time already
        // advised about that), so every matched rule folds into one value in
        // cascade order — the last, most specific declaration wins.
        let mut extras = Extras::default();
        let mut fold = |important_pass: bool| {
            for r in &matched {
                extras.refine(if important_pass {
                    &r.important_extras
                } else {
                    &r.normal_extras
                });
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
                // One rung per rule. A compound that names several ladder
                // states (`:hover:focus`) cannot be expressed as a conjunction
                // in gpui's ladder, so it lands on the strongest rung it names
                // — `classify_states` already warned about the approximation.
                match r.states {
                    0 => base.refine(refinement),
                    s if s & STATE_ACTIVE != 0 => {
                        active.refine(refinement);
                        any_active = true;
                    }
                    s if s & STATE_HOVER != 0 => {
                        hover.refine(refinement);
                        any_hover = true;
                    }
                    s if s & STATE_FOCUS_VISIBLE != 0 => {
                        focus_visible.refine(refinement);
                        any_focus_visible = true;
                    }
                    _ => {
                        focus.refine(refinement);
                        any_focus = true;
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
        extras.refine(&inline_extras);
        ComputedStates {
            base: Rc::new(base),
            hover: any_hover.then(|| Rc::new(hover)),
            active: any_active.then(|| Rc::new(active)),
            focus: any_focus.then(|| Rc::new(focus)),
            focus_visible: any_focus_visible.then(|| Rc::new(focus_visible)),
            transitions: transitions.unwrap_or_default(),
            extras,
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

/// The ladder bucket a pseudo-class belongs to (`0` = not a ladder state).
///
/// `:disabled` / `:checked` / `:enabled` are deliberately absent: the element
/// property *is* the state, so the matcher answers them for real and the rule
/// folds into the base bucket, exactly like `.foo.bar` would.
fn ladder_state(pc: &PseudoClass<'_>) -> u8 {
    match pc {
        PseudoClass::Hover => STATE_HOVER,
        PseudoClass::Active => STATE_ACTIVE,
        PseudoClass::Focus => STATE_FOCUS,
        PseudoClass::FocusVisible => STATE_FOCUS_VISIBLE,
        _ => 0,
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
        if let Component::NonTSPseudoClass(pc) = component {
            states |= ladder_state(pc);
        }
    }
    if states.count_ones() > 1 {
        // gpui's ladder has one refinement per rung and no conjunction, so
        // `:hover:focus` becomes "whichever rung gpui applies last".
        mapper::once(
            diagnostics,
            format!(
                "a selector naming several interaction states at once cannot be an AND in gpui's \
                 state ladder — it was installed on the strongest rung it names \
                 (active > hover > focus-visible > focus): {}",
                sel_text(sel)
            ),
        );
    }
    // Remaining compounds: any state pseudo there is an ancestor-state rule.
    while iter.next_sequence().is_some() {
        for component in &mut iter {
            if let Component::NonTSPseudoClass(pc) = component {
                if ladder_state(pc) != 0 {
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

// ---------------------------------------------------------------------------
// the compile-time selector audit
// ---------------------------------------------------------------------------

/// Walks every component of a selector (including the ones nested inside
/// `:not()` / `:is()` / `:where()`) and reports anything the matcher can only
/// ever answer "no" to. Returns `false` when the rule must be dropped.
///
/// This is the anti-silent-divergence pass: without it a `focus:`-style
/// variant, a `[data-x]` attribute selector or a `::before` rule compiles into
/// a perfectly valid `RuleEntry` that simply never matches, and an author has
/// no way to find out. Everything the matcher *can* answer
/// ([`element::El`]) passes through untouched.
fn audit_selector(sel: &Selector<'_>, diagnostics: &mut Vec<String>) -> bool {
    audit_components(sel, sel.iter_raw_match_order(), false, diagnostics)
}

fn audit_components<'a, 'i>(
    sel: &Selector<'i>,
    components: impl Iterator<Item = &'a Component<'i, element::Impl>>,
    nested: bool,
    diagnostics: &mut Vec<String>,
) -> bool
where
    'i: 'a,
{
    let mut ok = true;
    for c in components {
        // `Some(reason)` = this component can never be answered truthfully.
        let reason: Option<String> = match c {
            Component::NonTSPseudoClass(pc) => {
                if nested && ladder_state(pc) != 0 {
                    // Outside a functional pseudo an interaction state is a
                    // ladder bucket; inside one the matcher has to answer it
                    // with a bare yes/no and always says "yes", which silently
                    // inverts `:not(:hover)`.
                    Some(format!("`:{}` inside :not()/:is()/:where()", pseudo_name(pc)))
                } else if ladder_state(pc) == 0 && !matches_from_props(pc) {
                    Some(format!("the pseudo-class `:{}`", pseudo_name(pc)))
                } else {
                    None
                }
            }
            Component::PseudoElement(_) => {
                Some("a pseudo-element (`::before`, `::after`, …)".into())
            }
            Component::Slotted(_) | Component::Part(_) | Component::Host(_) => {
                Some("a shadow-DOM selector (`::slotted()`, `::part()`, `:host`)".into())
            }
            Component::Scope => Some("`:scope`".into()),
            Component::Nesting => Some("an unflattened `&` nesting selector".into()),
            Component::Namespace(..) | Component::DefaultNamespace(_) => {
                Some("a namespaced selector (`@namespace` is not implemented)".into())
            }
            Component::AttributeOther(attr) if attr.namespace.is_some() => {
                Some("a namespaced attribute selector (`[ns|attr]`)".into())
            }
            // These two do not merely fail to match — parcel's matcher hits an
            // `unreachable!()` / `todo!()` on them and takes the process with
            // it. `probe_runnable` only catches that when the dummy element
            // happens to reach the component (it stops at the first failing
            // one), so the audit is what actually keeps them out.
            Component::Has(_) => Some("`:has()` (the matcher has no implementation)".into()),
            Component::NthOf(_) => {
                Some("`:nth-child(… of S)` (the matcher has no implementation)".into())
            }
            // Recurse: the matcher runs these for real, but their contents are
            // subject to the same rules.
            Component::Negation(list)
            | Component::Where(list)
            | Component::Is(list)
            | Component::Any(_, list) => {
                for inner in list.iter() {
                    if !audit_components(inner, inner.iter_raw_match_order(), true, diagnostics) {
                        ok = false;
                    }
                }
                None
            }
            // `:has()` and `:nth-child(.. of S)` panic inside the matcher and
            // are already rejected by `probe_runnable`; everything else
            // (classes, ids, tags, attributes, `:root`, `:empty`, `:nth-*`,
            // combinators) the matcher answers for real.
            _ => None,
        };
        if let Some(what) = reason {
            ok = false;
            // Deduped on the FEATURE, not on the whole message: a Tailwind
            // sheet trips over the same gap in hundreds of rules, and one line
            // per rule buries everything else. The first offending selector
            // rides along as the example.
            if !diagnostics.iter().any(|d| d.starts_with(&what)) {
                diagnostics.push(format!(
                    "{what} — rules using it can never match and were dropped \
                     (first seen in `{}`)",
                    sel_text(sel)
                ));
            }
        }
    }
    ok
}

/// Pseudo-classes answered from the element's property map (see
/// [`element::El::match_non_ts_pseudo_class`]).
fn matches_from_props(pc: &PseudoClass<'_>) -> bool {
    matches!(
        pc,
        PseudoClass::Disabled | PseudoClass::Enabled | PseudoClass::Checked
    )
}

/// A pseudo-class's CSS name, for advisories. Deliberately exhaustive-ish so a
/// message names the offender instead of saying "some pseudo-class" — that is
/// the whole point of the audit.
fn pseudo_name(pc: &PseudoClass<'_>) -> String {
    let name = match pc {
        PseudoClass::Lang { .. } => "lang()",
        PseudoClass::Dir { .. } => "dir()",
        PseudoClass::Hover => "hover",
        PseudoClass::Active => "active",
        PseudoClass::Focus => "focus",
        PseudoClass::FocusVisible => "focus-visible",
        PseudoClass::FocusWithin => "focus-within",
        PseudoClass::Current => "current",
        PseudoClass::Past => "past",
        PseudoClass::Future => "future",
        PseudoClass::Playing => "playing",
        PseudoClass::Paused => "paused",
        PseudoClass::Seeking => "seeking",
        PseudoClass::Buffering => "buffering",
        PseudoClass::Stalled => "stalled",
        PseudoClass::Muted => "muted",
        PseudoClass::VolumeLocked => "volume-locked",
        PseudoClass::Fullscreen(_) => "fullscreen",
        PseudoClass::Open => "open",
        PseudoClass::Closed => "closed",
        PseudoClass::Modal => "modal",
        PseudoClass::PictureInPicture => "picture-in-picture",
        PseudoClass::PopoverOpen => "popover-open",
        PseudoClass::Defined => "defined",
        PseudoClass::AnyLink(_) => "any-link",
        PseudoClass::Link => "link",
        PseudoClass::LocalLink => "local-link",
        PseudoClass::Target => "target",
        PseudoClass::TargetCurrent => "target-current",
        PseudoClass::TargetBefore => "target-before",
        PseudoClass::TargetAfter => "target-after",
        PseudoClass::TargetWithin => "target-within",
        PseudoClass::Visited => "visited",
        PseudoClass::Enabled => "enabled",
        PseudoClass::Disabled => "disabled",
        PseudoClass::ReadOnly(_) => "read-only",
        PseudoClass::ReadWrite(_) => "read-write",
        PseudoClass::PlaceholderShown(_) => "placeholder-shown",
        PseudoClass::Default => "default",
        PseudoClass::Checked => "checked",
        PseudoClass::Indeterminate => "indeterminate",
        PseudoClass::Blank => "blank",
        PseudoClass::Valid => "valid",
        PseudoClass::Invalid => "invalid",
        PseudoClass::InRange => "in-range",
        PseudoClass::OutOfRange => "out-of-range",
        PseudoClass::Required => "required",
        PseudoClass::Optional => "optional",
        PseudoClass::UserValid => "user-valid",
        PseudoClass::UserInvalid => "user-invalid",
        PseudoClass::Autofill(_) => "autofill",
        PseudoClass::ActiveViewTransition => "active-view-transition",
        PseudoClass::ActiveViewTransitionType { .. } => "active-view-transition-type()",
        PseudoClass::State { .. } => "state()",
        PseudoClass::Local { .. } => "local()",
        PseudoClass::Global { .. } => "global()",
        PseudoClass::WebKitScrollbar(_) => "-webkit-scrollbar-*",
        PseudoClass::Custom { name } => return name.as_ref().to_string(),
        PseudoClass::CustomFunction { name, .. } => return format!("{}()", name.as_ref()),
    };
    name.to_string()
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
        fn attr(&self, _: &str) -> Option<&str> {
            None
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
        /// `(name, value)` pairs — the property/attribute channel.
        attrs: &'static [(&'static str, &'static str)],
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
        fn attr(&self, name: &str) -> Option<&str> {
            self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
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
            attrs: &[],
        }
    }

    fn node_with(attrs: &'static [(&'static str, &'static str)]) -> Node {
        Node { tag: "div", class: "box", attrs }
    }

    fn transitions(css: &str) -> Vec<TransitionSpec> {
        let engine = Engine::compile(css);
        (*engine.compute(box_node(), None, &Env::default()).transitions).clone()
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
        let states = engine.compute(box_node(), None, &Env::default());
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
            &Env::default(),
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
        let states = engine.compute(box_node(), None, &Env::default());
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
        let a = engine.compute(box_node(), None, &Env::default());
        let b = engine.compute(box_node(), None, &Env::default());
        assert_eq!(a, b);

        let other = Engine::compile(".box { transition: opacity 1s; opacity: 0.6 }")
            .compute(box_node(), None, &Env::default());
        assert_ne!(a, other);
    }

    // -----------------------------------------------------------------------
    // :focus / :focus-visible — the ladder rungs
    // -----------------------------------------------------------------------

    fn states_of(css: &str) -> ComputedStates {
        Engine::compile(css).compute(box_node(), None, &Env::default())
    }

    #[test]
    fn focus_rules_land_on_their_own_rung_not_in_base() {
        let s = states_of(".box { opacity: 0.2 } .box:focus { opacity: 0.9 }");
        assert_eq!(s.base.opacity, Some(0.2));
        assert_eq!(s.focus.as_ref().and_then(|f| f.opacity), Some(0.9));
        assert!(s.focus_visible.is_none());
        assert!(s.hover.is_none() && s.active.is_none());
        assert!(s.needs_focus_handle());
    }

    #[test]
    fn focus_visible_is_its_own_rung() {
        let s = states_of(".box:focus-visible { opacity: 0.7 }");
        assert_eq!(s.focus_visible.as_ref().and_then(|f| f.opacity), Some(0.7));
        assert!(s.focus.is_none());
        assert!(s.needs_focus_handle());
        // …and a node with no focus rule needs no handle.
        assert!(!states_of(".box { opacity: 0.5 }").needs_focus_handle());
    }

    /// Tailwind's `focus:` variant escapes the colon into the class name and
    /// appends the real pseudo — the exact string a JIT build emits.
    #[test]
    fn tailwinds_focus_variant_compiles_onto_the_focus_rung() {
        let engine = Engine::compile(r".focus\:opacity-100:focus { opacity: 1 }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let node = Node { tag: "div", class: "focus:opacity-100", attrs: &[] };
        let s = engine.compute(node, None, &Env::default());
        assert_eq!(s.focus.as_ref().and_then(|f| f.opacity), Some(1.0));
    }

    #[test]
    fn several_states_in_one_compound_pick_the_strongest_rung_and_warn() {
        // gpui's ladder has no AND, so the rule rides the rung gpui applies
        // last (active > hover > focus-visible > focus).
        let engine = Engine::compile(".box:hover:focus { opacity: 0.4 }");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("several interaction states")),
            "{:?}",
            engine.diagnostics
        );
        let s = engine.compute(box_node(), None, &Env::default());
        assert_eq!(s.hover.as_ref().and_then(|f| f.opacity), Some(0.4));
        assert!(s.focus.is_none());
    }

    #[test]
    fn an_ancestor_focus_selector_is_rejected_like_an_ancestor_hover_one() {
        let engine = Engine::compile(".box:focus .child { opacity: 0.4 }");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("ancestor-state selector")),
            "{:?}",
            engine.diagnostics
        );
    }

    // -----------------------------------------------------------------------
    // :disabled / :checked — states that are element properties
    // -----------------------------------------------------------------------

    #[test]
    fn disabled_matches_the_property_and_folds_into_base() {
        let engine = Engine::compile(".box { opacity: 1 } .box:disabled { opacity: 0.4 }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);

        let off = engine.compute(node_with(&[("disabled", "")]), None, &Env::default());
        // The property IS the state: it belongs in base, not on a rung.
        assert_eq!(off.base.opacity, Some(0.4));
        assert!(off.focus.is_none() && off.hover.is_none() && off.active.is_none());

        let on = engine.compute(box_node(), None, &Env::default());
        assert_eq!(on.base.opacity, Some(1.0));
    }

    /// `:disabled="false"` in a template reaches the host as the STRING
    /// "false"; treating a present-but-false property as disabled is the classic
    /// way this goes wrong.
    #[test]
    fn disabled_false_is_not_disabled() {
        let engine = Engine::compile(".box:disabled { opacity: 0.4 }");
        let s = engine.compute(node_with(&[("disabled", "false")]), None, &Env::default());
        assert_eq!(s.base.opacity, None);
        let s = engine.compute(node_with(&[("disabled", "true")]), None, &Env::default());
        assert_eq!(s.base.opacity, Some(0.4));
    }

    #[test]
    fn enabled_is_the_complement_and_checked_reads_its_own_property() {
        let engine = Engine::compile(".box:enabled { opacity: 1 } .box:checked { opacity: 0.5 }");
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).base.opacity,
            Some(1.0)
        );
        assert_eq!(
            engine
                .compute(node_with(&[("disabled", "")]), None, &Env::default())
                .base
                .opacity,
            None
        );
        assert_eq!(
            engine
                .compute(node_with(&[("checked", "true")]), None, &Env::default())
                .base
                .opacity,
            Some(0.5)
        );
    }

    /// The Tailwind `disabled:` variant, verbatim.
    #[test]
    fn tailwinds_disabled_variant_works() {
        let engine = Engine::compile(r".disabled\:opacity-50:disabled { opacity: 0.5 }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let node = Node {
            tag: "input",
            class: "disabled:opacity-50",
            attrs: &[("disabled", "")],
        };
        assert_eq!(
            engine.compute(node, None, &Env::default()).base.opacity,
            Some(0.5)
        );
    }

    // -----------------------------------------------------------------------
    // attribute selectors over the property map
    // -----------------------------------------------------------------------

    /// One rule per operator, all setting `opacity`, so a match is observable.
    fn attr_matches(selector: &str, attrs: &'static [(&'static str, &'static str)]) -> bool {
        let engine = Engine::compile(&format!("{selector} {{ opacity: 0.5 }}"));
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        engine
            .compute(node_with(attrs), None, &Env::default())
            .base
            .opacity
            .is_some()
    }

    #[test]
    fn attribute_presence_and_exact_value() {
        assert!(attr_matches("[disabled]", &[("disabled", "")]));
        assert!(!attr_matches("[disabled]", &[]));
        assert!(attr_matches(r#"[data-state="open"]"#, &[("data-state", "open")]));
        assert!(!attr_matches(r#"[data-state="open"]"#, &[("data-state", "closed")]));
        assert!(!attr_matches(r#"[data-state="open"]"#, &[]));
        // aria-* is nothing special — just an attribute.
        assert!(attr_matches(
            r#"[aria-expanded="true"]"#,
            &[("aria-expanded", "true")]
        ));
    }

    #[test]
    fn attribute_substring_operators() {
        let a: &'static [(&'static str, &'static str)] = &[("data-x", "alpha-beta gamma")];
        assert!(attr_matches(r#"[data-x^="alpha"]"#, a));
        assert!(!attr_matches(r#"[data-x^="beta"]"#, a));
        assert!(attr_matches(r#"[data-x$="gamma"]"#, a));
        assert!(!attr_matches(r#"[data-x$="alpha"]"#, a));
        assert!(attr_matches(r#"[data-x*="beta"]"#, a));
        assert!(!attr_matches(r#"[data-x*="delta"]"#, a));
        // whitespace-list and dash-prefix operators come free with parcel's
        // own `eval_str`.
        assert!(attr_matches(r#"[data-x~="gamma"]"#, a));
        assert!(!attr_matches(r#"[data-x~="alpha"]"#, a));
        assert!(attr_matches(r#"[lang|="en"]"#, &[("lang", "en-GB")]));
        assert!(!attr_matches(r#"[lang|="en"]"#, &[("lang", "eng")]));
        // …and so does the `i` case-insensitivity flag.
        assert!(attr_matches(r#"[data-state="OPEN" i]"#, &[("data-state", "open")]));
        assert!(!attr_matches(r#"[data-state="OPEN"]"#, &[("data-state", "open")]));
    }

    /// The Tailwind `data-[state=open]:` variant, verbatim.
    #[test]
    fn tailwinds_data_variant_works() {
        let engine = Engine::compile(r#".data-\[state\=open\]\:opacity-100[data-state="open"] { opacity: 1 }"#);
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let on = Node {
            tag: "div",
            class: "data-[state=open]:opacity-100",
            attrs: &[("data-state", "open")],
        };
        let off = Node { attrs: &[("data-state", "closed")], ..on };
        assert_eq!(engine.compute(on, None, &Env::default()).base.opacity, Some(1.0));
        assert_eq!(engine.compute(off, None, &Env::default()).base.opacity, None);
    }

    #[test]
    fn id_selectors_read_the_id_attribute() {
        assert!(attr_matches("#main", &[("id", "main")]));
        assert!(!attr_matches("#main", &[("id", "other")]));
        assert!(!attr_matches("#main", &[]));
    }

    /// The scoped-CSS hash keeps its own fast path and must not be answered
    /// from the property map (`data-v-*` lives in `Node::scopes`).
    #[test]
    fn scope_hashes_still_take_the_scope_path() {
        // The test node reports no scopes, so a scoped rule must not match even
        // though the property map is empty too.
        assert!(!attr_matches("[data-v-abc123]", &[("data-v-abc123", "")]));
    }

    // -----------------------------------------------------------------------
    // @media
    // -----------------------------------------------------------------------

    fn env_at(width: f32, dark: bool, reduce_motion: bool) -> Env {
        Env { viewport_width: width, viewport_height: 800.0, dark, reduce_motion }
    }

    #[test]
    fn media_rules_apply_only_when_their_condition_holds() {
        let engine = Engine::compile(
            ".box { opacity: 0.1 } @media (min-width: 640px) { .box { opacity: 0.9 } }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let narrow = engine.compute(box_node(), None, &env_at(500.0, false, false));
        let wide = engine.compute(box_node(), None, &env_at(700.0, false, false));
        assert_eq!(narrow.base.opacity, Some(0.1));
        assert_eq!(wide.base.opacity, Some(0.9));
        // The change in `Env` alone changes the result — which is exactly why
        // the host has to mark the whole tree dirty when it moves.
        assert_ne!(narrow, wide);
    }

    #[test]
    fn dark_and_reduced_motion_conditions_reach_the_cascade() {
        let engine = Engine::compile(
            ".box { opacity: 0.1 } \
             @media (prefers-color-scheme: dark) { .box { opacity: 0.8 } } \
             @media (prefers-reduced-motion: reduce) { .box { opacity: 0.3 } }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let base = engine.compute(box_node(), None, &env_at(800.0, false, false));
        let dark = engine.compute(box_node(), None, &env_at(800.0, true, false));
        let calm = engine.compute(box_node(), None, &env_at(800.0, false, true));
        assert_eq!(base.base.opacity, Some(0.1));
        assert_eq!(dark.base.opacity, Some(0.8));
        // Later block, same specificity → the reduced-motion rule wins.
        assert_eq!(calm.base.opacity, Some(0.3));
    }

    #[test]
    fn media_state_rules_still_reach_their_rung() {
        let engine =
            Engine::compile("@media (min-width: 640px) { .box:focus { opacity: 0.9 } }");
        assert!(engine
            .compute(box_node(), None, &env_at(500.0, false, false))
            .focus
            .is_none());
        assert_eq!(
            engine
                .compute(box_node(), None, &env_at(700.0, false, false))
                .focus
                .as_ref()
                .and_then(|f| f.opacity),
            Some(0.9)
        );
    }

    #[test]
    fn nested_media_blocks_and_the_document_order_tiebreak() {
        let engine = Engine::compile(
            "@media (min-width: 640px) { \
                @media (prefers-color-scheme: dark) { .box { opacity: 0.7 } } \
             } \
             .box { opacity: 0.2 }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        // Both conditions must hold…
        assert_eq!(
            engine.compute(box_node(), None, &env_at(700.0, false, false)).base.opacity,
            Some(0.2)
        );
        assert_eq!(
            engine.compute(box_node(), None, &env_at(500.0, true, false)).base.opacity,
            Some(0.2)
        );
        // …and even then the LATER rule wins at equal specificity, exactly as
        // it would in a browser: nesting must not reorder the cascade.
        assert_eq!(
            engine.compute(box_node(), None, &env_at(700.0, true, false)).base.opacity,
            Some(0.2)
        );
    }

    #[test]
    fn an_unsupported_media_feature_drops_the_block_with_an_advisory() {
        let engine = Engine::compile("@media (hover: hover) { .box { opacity: 0.9 } }");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("@media") && d.contains("dropped")),
            "{:?}",
            engine.diagnostics
        );
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).base.opacity,
            None
        );
    }

    #[test]
    fn print_only_rules_parse_and_simply_never_apply() {
        let engine = Engine::compile("@media print { .box { opacity: 0.9 } }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).base.opacity,
            None
        );
    }

    /// The four variants a Tailwind build actually emits, in the exact shape
    /// its JIT writes them.
    #[test]
    fn tailwind_responsive_and_dark_variants() {
        let engine = Engine::compile(concat!(
            r"@media (min-width: 640px) { .sm\:opacity-100 { opacity: 1 } }",
            r"@media (min-width: 768px) { .md\:opacity-75 { opacity: 0.75 } }",
            r"@media (prefers-color-scheme: dark) { .dark\:opacity-50 { opacity: 0.5 } }",
            r"@media (prefers-reduced-motion: reduce) { .motion-reduce\:opacity-25 { opacity: .25 } }",
        ));
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let node = |class: &'static str| Node { tag: "div", class, attrs: &[] };
        let get = |class: &'static str, env: &Env| {
            engine.compute(node(class), None, env).base.opacity
        };
        assert_eq!(get("sm:opacity-100", &env_at(500.0, false, false)), None);
        assert_eq!(get("sm:opacity-100", &env_at(700.0, false, false)), Some(1.0));
        assert_eq!(get("md:opacity-75", &env_at(700.0, false, false)), None);
        assert_eq!(get("md:opacity-75", &env_at(800.0, false, false)), Some(0.75));
        assert_eq!(get("dark:opacity-50", &env_at(800.0, false, false)), None);
        assert_eq!(get("dark:opacity-50", &env_at(800.0, true, false)), Some(0.5));
        assert_eq!(get("motion-reduce:opacity-25", &env_at(800.0, false, false)), None);
        assert_eq!(
            get("motion-reduce:opacity-25", &env_at(800.0, false, true)),
            Some(0.25)
        );
    }

    // -----------------------------------------------------------------------
    // the selector audit
    // -----------------------------------------------------------------------

    /// A rule the audit rejects must (a) say so and (b) not be compiled.
    fn audited(selector: &str) -> Engine {
        Engine::compile(&format!("{selector} {{ opacity: 0.5 }}"))
    }

    #[test]
    fn unanswerable_pseudo_classes_are_reported_and_dropped() {
        for (selector, needle) in [
            (".box:visited", "visited"),
            (".box:focus-within", "focus-within"),
            (".box:required", "required"),
            (".box:placeholder-shown", "placeholder-shown"),
            (".box:target", "target"),
            (".box:lang(en)", "lang()"),
            (".box:indeterminate", "indeterminate"),
            (".box::before", "pseudo-element"),
            (".box:scope", "scope"),
        ] {
            let engine = audited(selector);
            assert!(
                engine.diagnostics.iter().any(|d| d.contains(needle)),
                "{selector}: {:?}",
                engine.diagnostics
            );
            assert_eq!(
                engine.compute(box_node(), None, &Env::default()).base.opacity,
                None,
                "{selector} must not compile into a never-matching rule"
            );
        }
    }

    /// The structural pseudos parcel answers for real must survive untouched —
    /// the audit is not allowed to be a blanket "reject anything unusual".
    #[test]
    fn structural_and_answerable_selectors_pass_the_audit() {
        for selector in [
            ".box",
            "div.box",
            ".box:nth-child(1)",
            ".box:first-child",
            ".box:only-child",
            ":root",
            ".box:empty",
            ".box:not(.other)",
            ".box:is(.box, .other)",
            ".box:where(.box)",
            ".box:disabled, .box:enabled",
            "[data-state]",
            "#main",
        ] {
            let engine = audited(selector);
            assert!(
                engine.diagnostics.is_empty(),
                "{selector} should be accepted: {:?}",
                engine.diagnostics
            );
        }
    }

    /// A ladder state inside `:not()` silently INVERTS: the matcher answers
    /// "would match" for `:hover`, so `:not(:hover)` could only ever be false.
    #[test]
    fn a_ladder_state_inside_a_functional_pseudo_is_rejected() {
        let engine = audited(".box:not(:hover)");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains(":not()/:is()/:where()")),
            "{:?}",
            engine.diagnostics
        );
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).base.opacity,
            None
        );
    }

    /// One advisory per unsupported FEATURE, not per rule — a Tailwind sheet
    /// repeats the same gap hundreds of times.
    #[test]
    fn audit_advisories_are_one_shot() {
        let engine = Engine::compile(
            ".a:visited { opacity: 0.1 } .b:visited { opacity: 0.2 } .c:visited { opacity: 0.3 }",
        );
        assert_eq!(
            engine.diagnostics.iter().filter(|d| d.contains("visited")).count(),
            1,
            "{:?}",
            engine.diagnostics
        );
    }

    /// `:has()` and `:nth-child(… of S)` make parcel's matcher `unreachable!()`
    /// — a process abort, not a failed match. `probe_runnable` only catches
    /// that when its dummy element happens to REACH the component (matching
    /// stops at the first failing one, and the dummy fails `.box` immediately),
    /// so the audit is what actually keeps them out.
    #[test]
    fn selectors_that_would_crash_the_matcher_are_rejected() {
        for selector in [".box:has(.child)", ".box:nth-child(1 of .other)"] {
            let engine = audited(selector);
            assert!(
                engine
                    .diagnostics
                    .iter()
                    .any(|d| d.contains("no implementation") || d.contains("matcher cannot run it")),
                "{selector}: {:?}",
                engine.diagnostics
            );
            // The real proof: matching a node that DOES carry `.box` must not
            // take the process down.
            assert_eq!(
                engine.compute(box_node(), None, &Env::default()).base.opacity,
                None
            );
        }
    }

    // -----------------------------------------------------------------------
    // object-fit — the one computed value that rides `Extras`, not the
    // refinement (gpui keeps ObjectFit on the img element, not in Style).
    // -----------------------------------------------------------------------

    #[test]
    fn object_fit_keywords_all_map() {
        for (css, want) in [
            ("fill", ObjectFit::Fill),
            ("contain", ObjectFit::Contain),
            ("cover", ObjectFit::Cover),
            ("scale-down", ObjectFit::ScaleDown),
            ("none", ObjectFit::None),
        ] {
            let engine = Engine::compile(&format!(".box {{ object-fit: {css} }}"));
            assert!(engine.diagnostics.is_empty(), "{css}: {:?}", engine.diagnostics);
            assert_eq!(
                engine.compute(box_node(), None, &Env::default()).extras.object_fit,
                Some(want),
                "object-fit: {css}"
            );
        }
    }

    /// Undeclared stays `None` — the renderer is what supplies the browser
    /// initial value (`fill`), so "nobody said" and "somebody said fill" stay
    /// distinguishable through the cascade.
    #[test]
    fn object_fit_is_absent_when_undeclared() {
        let engine = Engine::compile(".box { opacity: 0.5 }");
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).extras.object_fit,
            None
        );
    }

    #[test]
    fn object_fit_folds_by_specificity_and_order() {
        let engine = Engine::compile(
            ".box { object-fit: contain } div.box { object-fit: cover } .box { object-fit: none }",
        );
        // `div.box` (0,1,1) beats both `.box` rules (0,1,0) regardless of order.
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).extras.object_fit,
            Some(ObjectFit::Cover)
        );
    }

    #[test]
    fn important_object_fit_wins() {
        let engine = Engine::compile("div.box { object-fit: cover } .box { object-fit: none !important }");
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).extras.object_fit,
            Some(ObjectFit::None)
        );
    }

    #[test]
    fn inline_object_fit_beats_the_sheet() {
        let engine = Engine::compile("div.box { object-fit: cover }");
        let mut diags = Vec::new();
        let (r, t, x) = parse_inline_full("object-fit: contain", &mut diags);
        assert!(diags.is_empty(), "{diags:?}");
        let computed = engine.compute_full(
            box_node(),
            Some(&r),
            (!t.is_empty()).then(|| Rc::new(t)).as_ref(),
            x,
            &Env::default(),
        );
        assert_eq!(computed.extras.object_fit, Some(ObjectFit::Contain));
    }

    /// A bad keyword must leave an advisory, never silently do nothing.
    #[test]
    fn bad_object_fit_value_is_advised() {
        let engine = Engine::compile(".box { object-fit: squish }");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("object-fit")),
            "{:?}",
            engine.diagnostics
        );
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).extras.object_fit,
            None
        );
    }

    /// gpui's image element has ONE object-fit, not a hover/active ladder — so
    /// a state rule that declares it is a real divergence and gets said out loud.
    #[test]
    fn object_fit_on_a_state_selector_is_advised() {
        let engine = Engine::compile(".box:hover { object-fit: cover }");
        assert!(
            engine
                .diagnostics
                .iter()
                .any(|d| d.contains("object-fit") && d.contains("unconditionally")),
            "{:?}",
            engine.diagnostics
        );
        // And it does apply — unconditionally, as advertised.
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).extras.object_fit,
            Some(ObjectFit::Cover)
        );
    }

    /// `object-fit` must not be swallowed by the "property not mapped" fold —
    /// that advisory is for properties we genuinely do not implement.
    #[test]
    fn object_fit_does_not_report_as_unmapped() {
        let engine = Engine::compile(".box { object-fit: cover }");
        assert!(
            !engine.diagnostics.iter().any(|d| d.contains("not mapped")),
            "{:?}",
            engine.diagnostics
        );
    }
}
