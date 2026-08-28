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
mod keyframes;
mod mapper;
mod media;
pub mod perf;
mod transition;
mod vars;

use std::rc::Rc;

use gpui::{Refineable, StyleRefinement};
use lightningcss::rules::CssRule;
use lightningcss::selector::{PseudoClass, Selector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use parcel_selectors::context::{MatchingContext, MatchingMode, QuirksMode};
use parcel_selectors::matching::matches_selector;
use parcel_selectors::parser::Component;

pub use animator::{AnimEvent, AnimEventKind, Animator};
pub use element::MatchDom;
pub use mapper::{
    parse_inline, parse_inline_full, parse_inline_style, parse_inline_with_transitions, rem_basis,
    set_rem_basis, set_viewport_basis, uses_viewport_units, viewport_basis, Decls, Extras,
    InlineStyle, ObjectFit, SvgPaint, TextTransform, WindowControl, WindowDrag,
    INITIAL_REM_BASIS, INITIAL_VIEWPORT_BASIS,
};
pub use media::{match_media, DisplayMode, Env};
pub use keyframes::{
    AnimationPatch, AnimationSpec, Direction as AnimationDirection, FillMode, Keyframe, Keyframes,
    PlayState,
};
pub use vars::Vars;
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

/// How many rungs the ladder has above the base state — the width of the
/// per-rung custom-property maps in [`Engine::compute_node`].
const STATE_RUNGS: usize = 4;

/// Which rung a rule's `states` mask lands on, or `None` for the base state.
///
/// The order mirrors the bucket selection in `compute_node` exactly (a compound
/// naming several states lands on the strongest one it names), because the two
/// have to agree: a rung's custom properties must be the ones its own
/// declarations read.
fn rung_of(states: u8) -> Option<usize> {
    match states {
        0 => None,
        s if s & STATE_ACTIVE != 0 => Some(0),
        s if s & STATE_HOVER != 0 => Some(1),
        s if s & STATE_FOCUS_VISIBLE != 0 => Some(2),
        _ => Some(3),
    }
}

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
    /// `.group:hover .this` — what this element looks like while an ANCESTOR is
    /// hovered, and which ancestor that is (its [`MatchDom::key`]).
    ///
    /// The renderer turns the pair into gpui's own group mechanism: the
    /// ancestor is given `.group("vn-g<key>")` and this element
    /// `.group_hover("vn-g<key>", …)`, which gpui resolves from the group's
    /// hitbox at PAINT time. Nothing here subscribes, and nothing restyles when
    /// the ancestor's hover flips — the m11 perf invariants are untouched.
    pub group_hover: Option<(u64, Rc<StyleRefinement>)>,
    /// `.group:active .this`. Same mechanism, gpui's `.group_active()`.
    pub group_active: Option<(u64, Rc<StyleRefinement>)>,
    /// The folded `transition` list. Empty means "no transitions".
    pub transitions: Rc<Vec<TransitionSpec>>,
    /// The folded `animation` list. Empty means "no animations". Unlike a
    /// transition (which is a diff of THIS struct against the previous one), an
    /// animation reads its values out of the engine's `@keyframes` — so this
    /// carries only the clock, and the animator joins the two.
    pub animations: Rc<Vec<AnimationSpec>>,
    /// Computed properties a `StyleRefinement` cannot carry (`object-fit`).
    /// Folded from the BASE state only — see [`Extras`].
    pub extras: Extras,
    /// The `::before` generated box, when the cascade gave this element one.
    /// `None` means no `::before` rule matched, or the ones that did declared
    /// no `content` — which in CSS means no box is generated at all.
    pub before: Option<Rc<PseudoBox>>,
    /// The `::after` generated box. See [`ComputedStates::before`].
    pub after: Option<Rc<PseudoBox>>,
    /// `::placeholder` — the declarations the text-input widget paints its
    /// placeholder string with. Only `color` and the font properties can reach
    /// the widget; a box property declared here has nothing to apply to.
    pub placeholder: Option<Rc<StyleRefinement>>,
    /// The element's resolved custom properties: what it inherited from its
    /// parent, overridden by what it declared. Children read this one, which is
    /// why the restyle pass must walk in document order.
    ///
    /// Shared with the parent by `Rc` when the element declares none of its own
    /// — so the `PartialEq` above stays O(1) for almost every node.
    pub vars: Rc<Vars>,
}

/// Which generated box a rule targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    /// `::placeholder`. Not a generated box: it styles the placeholder STRING
    /// the native text-input widget paints, so it folds into its own
    /// refinement and the renderer reads `text.color` off it.
    Placeholder,
}

/// A `::before` / `::after` generated box, folded out of the cascade.
///
/// It is a real box in the layout — the renderer builds it as a first (or last)
/// child of the originating element — because that is what CSS says it is: a
/// child of the element, before or after its content, laid out by the same
/// rules as any sibling.
///
/// It carries its own `content` because CSS generates NO box at all when
/// `content` is absent, so the string is what decides whether this struct
/// exists rather than a property of a box that would exist anyway.
#[derive(Clone, Debug, PartialEq)]
pub struct PseudoBox {
    /// The `content` string. Empty is legal and generates an empty box, which
    /// is what every decorative `content: ""` indicator relies on.
    pub content: Rc<str>,
    /// The folded declarations, base state only.
    pub style: Rc<StyleRefinement>,
    /// `object-fit` and friends off the same rules — chiefly `position`,
    /// `z-index` and `pointer-events`, which decorative pseudo boxes use.
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
    normal_animations: Option<Rc<Vec<AnimationSpec>>>,
    important_animations: Option<Rc<Vec<AnimationSpec>>>,
    normal_animation_patch: Option<keyframes::AnimationPatch>,
    important_animation_patch: Option<keyframes::AnimationPatch>,
    normal_extras: Extras,
    important_extras: Extras,
    /// `--x` declarations and `var()`-composed ones: both are resolved per
    /// element (see [`Engine::compute_node`]), so the rule carries their text.
    normal_custom: Option<Rc<Decls>>,
    important_custom: Option<Rc<Decls>>,
    normal_dynamic: Option<Rc<Decls>>,
    important_dynamic: Option<Rc<Decls>>,
    specificity: u32,
    order: u32,
    states: u8,
    /// `.group:hover .child` — the state belongs to an ANCESTOR. See
    /// [`AncestorState`].
    ancestor: Option<AncestorState>,
    /// `::before` / `::after`. A rule that carries one styles a GENERATED box,
    /// not the element it matched, so it is folded separately.
    pseudo: Option<PseudoElement>,
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
    /// The rule index — see [`RuleIndex`].
    index: RuleIndex,
    /// One entry per distinct `@media` prelude in the sheet; rules point at
    /// theirs by index. Evaluated against the [`Env`] on every cascade.
    conditions: Vec<media::Cond>,
    /// Any `+` / `~` anywhere in the sheet — see
    /// [`Engine::uses_sibling_combinators`].
    sibling_combinators: bool,
    /// Any `.group:hover .child` anywhere in the sheet — see
    /// [`Engine::uses_group_states`].
    group_states: bool,
    /// `@keyframes` blocks by name. Document-scoped rather than per element:
    /// an element's `animation-name` is a REFERENCE into this map, resolved by
    /// the animator when a track starts. A later block with the same name wins,
    /// as in CSS.
    keyframes: std::collections::HashMap<Box<str>, Rc<keyframes::Keyframes>>,
    pub diagnostics: Vec<String>,
}

/// Which rules an element could possibly match, keyed off the RIGHTMOST
/// compound of each selector.
///
/// A compound is a conjunction: `.card .title` can only match an element that
/// carries `title`, and `button.ghost` only one whose tag is `button`. So a
/// rule can be filed under one key its subject must have, and an element only
/// ever has to run the selectors filed under the keys it actually carries plus
/// the ones that could not be keyed at all (`*`, `[data-x]`, `:root`).
///
/// This is the browsers' oldest cascade optimization and it is what turns
/// `elements × rules-in-the-sheet` into `elements × a-handful`: on the shadcn
/// app a 25 KB Tailwind sheet went from ~1.5 M selector runs per restyle to
/// ~28 K (research/reports/m11/perf-restyle-rebuild.md).
///
/// It is a pure FILTER: everything it hands back still goes through
/// `matches_selector`, and the cascade sorts by `(specificity, order)`
/// afterwards, so bucket order cannot affect the result. `VN_CSS_NO_RULE_INDEX=1`
/// turns it off and tests every rule against every element — the A/B switch the
/// perf report's before/after column was measured with, and the first thing to
/// try if a rule ever seems not to apply.
#[derive(Default)]
struct RuleIndex {
    by_id: std::collections::HashMap<Box<str>, Vec<u32>>,
    by_class: std::collections::HashMap<Box<str>, Vec<u32>>,
    by_tag: std::collections::HashMap<Box<str>, Vec<u32>>,
    /// Rules whose rightmost compound names no id, class or tag — every
    /// element has to run these.
    any: Vec<u32>,
    enabled: bool,
}

/// The key a selector's rightmost compound can be filed under.
enum Bucket {
    Id(Box<str>),
    Class(Box<str>),
    Tag(Box<str>),
    Any,
}

/// Files a selector by the strongest key its rightmost compound guarantees.
///
/// Only TOP-LEVEL components of that compound count. A class inside
/// `:not()` / `:is()` / `:where()` guarantees nothing about the element (it may
/// even be a negation), so such a selector falls into [`RuleIndex::any`] and is
/// simply always tested — conservative, never wrong.
fn bucket_of(sel: &Selector<'_>) -> Bucket {
    let mut class: Option<Box<str>> = None;
    let mut tag: Option<Box<str>> = None;
    // `iter()` yields the RIGHTMOST compound and stops at the first combinator
    // (the same contract `classify_states` relies on).
    for component in sel.iter() {
        match component {
            Component::ID(id) => return Bucket::Id(id.0.as_ref().into()),
            Component::Class(c) if class.is_none() => class = Some(c.0.as_ref().into()),
            // `is_html_element_in_html_document()` is false for us, so the
            // matcher compares against `name` as written, never `lower_name`.
            Component::LocalName(ln) if tag.is_none() => tag = Some(ln.name.0.as_ref().into()),
            _ => {}
        }
    }
    match (class, tag) {
        (Some(c), _) => Bucket::Class(c),
        (None, Some(t)) => Bucket::Tag(t),
        (None, None) => Bucket::Any,
    }
}

/// `.a + .b` / `.a ~ .b` anywhere in this selector, INCLUDING inside a
/// functional pseudo — `:is(.a + .b)` reads a sibling just as literally.
fn has_sibling_combinator(sel: &Selector<'_>) -> bool {
    let mut iter = sel.iter();
    loop {
        for component in &mut iter {
            let nested = match component {
                Component::Negation(list)
                | Component::Where(list)
                | Component::Is(list)
                | Component::Any(_, list) => list,
                _ => continue,
            };
            if list_has_sibling_combinator(nested) {
                return true;
            }
        }
        match iter.next_sequence() {
            Some(parcel_selectors::parser::Combinator::NextSibling)
            | Some(parcel_selectors::parser::Combinator::LaterSibling) => return true,
            Some(_) => continue,
            None => return false,
        }
    }
}

fn list_has_sibling_combinator(list: &[Selector<'_>]) -> bool {
    list.iter().any(has_sibling_combinator)
}

impl RuleIndex {
    fn file(&mut self, sel: &Selector<'_>, rule: u32) {
        match bucket_of(sel) {
            Bucket::Id(k) => self.by_id.entry(k).or_default().push(rule),
            Bucket::Class(k) => self.by_class.entry(k).or_default().push(rule),
            Bucket::Tag(k) => self.by_tag.entry(k).or_default().push(rule),
            Bucket::Any => self.any.push(rule),
        }
    }
}

/// Is the `VN_PERF` probe on? vn-css cannot see vn-tree's copy of the flag and
/// the three `Instant`s per element are only free when nobody is looking.
fn perf_on() -> bool {
    static ON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    match ON.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("VN_PERF")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            ON.store(if on { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
            on
        }
    }
}

/// The escape hatch, read once per process.
fn rule_index_enabled() -> bool {
    static ON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    match ON.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let off = std::env::var("VN_CSS_NO_RULE_INDEX")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            ON.store(if off { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
            !off
        }
    }
}

impl Engine {
    pub fn compile(css: &str) -> Engine {
        let mut engine = Engine {
            rules: Vec::new(),
            index: RuleIndex { enabled: rule_index_enabled(), ..RuleIndex::default() },
            conditions: Vec::new(),
            sibling_combinators: false,
            group_states: false,
            keyframes: std::collections::HashMap::new(),
            diagnostics: Vec::new(),
        };
        // The flag describes THIS sheet: a resize only recompiles when the sheet
        // currently installed actually resolves a viewport unit.
        mapper::reset_viewport_units_seen();
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

    /// Does any selector in the sheet use `+` or `~`?
    ///
    /// The restyle pass needs this to know how far a mutation reaches. A change
    /// to one element can only affect elements the selectors can *see* from it:
    /// descendants always (a descendant combinator reads every ancestor), and
    /// FOLLOWING SIBLINGS only if the sheet actually contains a sibling
    /// combinator. Most sheets do not, and asking the question once per
    /// stylesheet is what lets a hover flip restyle a subtree instead of a
    /// document (docs/ARCHITECTURE.md §11).
    pub fn uses_sibling_combinators(&self) -> bool {
        self.sibling_combinators
    }

    /// Does any rule in the sheet put an interaction state on an ANCESTOR
    /// compound (`.group:hover .child`)?
    ///
    /// The renderer asks once per sheet: maintaining the set of elements that
    /// need gpui's `.group(name)` costs a pass over the node slab, and a sheet
    /// without a single `group-hover:` rule must not pay for it.
    pub fn uses_group_states(&self) -> bool {
        self.group_states
    }

    /// The `@keyframes` block `name` refers to, if the sheet has one.
    ///
    /// Shared by `Rc` because a running animation holds the block it started
    /// with (see `Animator`'s `AnimRun`): the render walk has a node id and a
    /// clock, and no stylesheet to look the name up in again.
    pub fn keyframes(&self, name: &str) -> Option<Rc<keyframes::Keyframes>> {
        self.keyframes.get(name).cloned()
    }

    /// Does the sheet define any animation at all? The animator can skip its
    /// whole pass when not — the common case for a hand-written sheet.
    pub fn has_keyframes(&self) -> bool {
        !self.keyframes.is_empty()
    }

    /// Hands `f` every rule this element could match, once each. With the index
    /// off (or on a sheet compiled before it existed) that is simply every rule.
    fn candidates<D: MatchDom>(&self, node: &D, f: &mut dyn FnMut(u32)) {
        if !self.index.enabled {
            for i in 0..self.rules.len() as u32 {
                f(i);
            }
            return;
        }
        for &i in &self.index.any {
            f(i);
        }
        if !self.index.by_tag.is_empty() {
            if let Some(v) = self.index.by_tag.get(node.tag_name()) {
                for &i in v {
                    f(i);
                }
            }
            // The document root answers to `html` and `body` as well as to its
            // own tag (`MatchDom::has_tag`), so the index has to offer it those
            // rules too or a `body { … }` rule would never be a candidate and
            // could never match. The root's own tag is never one of the two, so
            // this cannot hand the same rule back twice.
            if node.is_root() {
                for name in ["html", "body"] {
                    if let Some(v) = self.index.by_tag.get(name) {
                        for &i in v {
                            f(i);
                        }
                    }
                }
            }
        }
        if !self.index.by_class.is_empty() {
            node.each_class(&mut |c| {
                if let Some(v) = self.index.by_class.get(c) {
                    for &i in v {
                        f(i);
                    }
                }
            });
        }
        if !self.index.by_id.is_empty() {
            if let Some(id) = node.attr("id") {
                if let Some(v) = self.index.by_id.get(id) {
                    for &i in v {
                        f(i);
                    }
                }
            }
        }
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
                CssRule::Keyframes(kf) => {
                    self.collect_keyframes(kf);
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
                let mut e = converted.normal_extras.clone();
                e.refine(&converted.important_extras);
                e
            };
            for sel in &style.selectors.0 {
                let Some((states, ancestor)) = classify_states(sel, &mut self.diagnostics) else {
                    continue;
                };
                let pseudo = sel.pseudo_element().and_then(pseudo_box);
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
                // A generated box is not in gpui's interactivity ladder — it is
                // a plain child div with no hitbox of its own — so a
                // `:hover::after` rule has nothing to install into.
                if pseudo.is_some() && (states != 0 || ancestor.is_some()) {
                    self.diagnostics.push(format!(
                        "a state-qualified pseudo-element rule is dropped (v1) — the generated                          box is not in the pointer-state ladder. Drive it from an ATTRIBUTE on                          the originating element instead (`[data-state=open]::after`), which does                          match: {}",
                        sel_text(sel)
                    ));
                    continue;
                }
                // The `Extras` have no state ladder to install into — gpui's
                // `img()` carries ONE `ObjectFit` for the element, and the
                // window hit test reads an already-PAINTED frame, so a
                // state-dependent drag region would be a hit-test/paint race
                // (docs/WINDOWING.md D-16). So a state rule's extras apply all
                // the time; say so rather than let them look conditional.
                if (states != 0 || ancestor.is_some()) && !declares_extras.is_empty() {
                    self.diagnostics.push(format!(
                        "{} on a state selector applies unconditionally (v1) — these values are \
                         read from the base state only, not from gpui's state ladder: {}",
                        extras_names(&declares_extras),
                        sel_text(sel)
                    ));
                }
                if (states != 0 || ancestor.is_some()) && declares_transition {
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
                self.sibling_combinators |= has_sibling_combinator(sel);
                self.group_states |= ancestor.is_some();
                self.index.file(sel, self.rules.len() as u32);
                // `sheet` is a leaked &'static, so its selectors are 'static too.
                self.rules.push(RuleEntry {
                    selector: sel,
                    normal: converted.normal.clone(),
                    important: converted.important.clone(),
                    normal_transitions: converted.normal_transitions.clone(),
                    important_transitions: converted.important_transitions.clone(),
                    normal_animations: converted.normal_animations.clone(),
                    important_animations: converted.important_animations.clone(),
                    normal_animation_patch: converted.normal_animation_patch,
                    important_animation_patch: converted.important_animation_patch,
                    normal_extras: converted.normal_extras.clone(),
                    important_extras: converted.important_extras.clone(),
                    normal_custom: converted.normal_custom.clone(),
                    important_custom: converted.important_custom.clone(),
                    normal_dynamic: converted.normal_dynamic.clone(),
                    important_dynamic: converted.important_dynamic.clone(),
                    specificity: sel.specificity(),
                    order: *order,
                    states,
                    ancestor,
                    pseudo,
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
    ///
    /// Custom properties are NOT resolved on this path (the element has no
    /// parent map to inherit from) — use [`Self::compute_node`] for that. Kept
    /// because it is the shape every non-tree caller and every unit test wants.
    pub fn compute_full<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&StyleRefinement>,
        inline_transitions: Option<&Rc<Vec<TransitionSpec>>>,
        inline_extras: Extras,
        env: &Env,
    ) -> ComputedStates {
        let style = InlineStyle {
            refinement: inline.map(|r| Rc::new(r.clone())),
            transitions: inline_transitions.cloned(),
            extras: inline_extras,
            ..Default::default()
        };
        let mut diagnostics = Vec::new();
        self.compute_node(
            node,
            Some(&style),
            &Rc::new(Vars::default()),
            env,
            &mut diagnostics,
        )
    }

    /// The full cascade for one node, including CSS custom properties.
    ///
    /// `parent_vars` is the resolved custom-property map of this node's PARENT
    /// (the tree root passes an empty one). Custom properties inherit, so the
    /// caller must walk the tree in document order and thread each node's
    /// [`ComputedStates::vars`] into its children — that ordering requirement is
    /// the one structural difference custom properties make to the cascade.
    ///
    /// `diagnostics` collects the advisories that can only be known here rather
    /// than at sheet-compile time: a `var()` that resolves to nothing, a
    /// reference cycle, a substituted value that does not parse. They are
    /// per-element, so the caller is expected to dedupe before logging.
    pub fn compute_node<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&InlineStyle>,
        parent_vars: &Rc<Vars>,
        env: &Env,
        diagnostics: &mut Vec<String>,
    ) -> ComputedStates {
        let probe = perf_on();
        let t0 = probe.then(std::time::Instant::now);
        let el = element::El(node);
        let mut matched: Vec<&RuleEntry> = Vec::new();
        let mut tested = 0u64;
        self.candidates(&node, &mut |i| {
            let rule = &self.rules[i as usize];
            // Cheaper than matching, and it has to run first anyway: a rule
            // inside a `@media` block that does not hold is simply not there.
            if let Some(c) = rule.condition {
                if !self.conditions[c as usize].eval(env) {
                    return;
                }
            }
            tested += 1;
            // A `::before` selector only matches under
            // `ForStatelessPseudoElement`, which consumes the pseudo component
            // and runs the REST of the selector against the originating
            // element — exactly the question being asked here.
            let mode = if rule.pseudo.is_some() {
                MatchingMode::ForStatelessPseudoElement
            } else {
                MatchingMode::Normal
            };
            let mut ctx: MatchingContext<'_, '_, element::Impl> =
                MatchingContext::new(mode, None, None, QuirksMode::NoQuirks);
            if matches_selector(rule.selector, 0, None, &el, &mut ctx, &mut |_, _| {}) {
                matched.push(rule);
            }
        });
        // The index hands the candidates back bucket by bucket, so this sort is
        // what restores the cascade's document order — it is not an
        // optimization detail, it is the cascade.
        matched.sort_by_key(|r| (r.specificity, r.order));
        let t1 = probe.then(std::time::Instant::now);

        // ---- 1. custom properties, first: every other declaration reads them.
        // Duplicates are left in — `vars::resolve` folds by name with last-wins,
        // which is exactly the cascade order they are pushed in.
        // Borrowed, not cloned: Tailwind's universal block alone is ~40
        // declarations, and cloning two `String`s each per element is 16 K
        // allocations per restyle on a 200-element page for values nobody
        // mutates.
        //
        // A custom property declared inside a STATE rule (`:hover`, `:focus`,
        // `:focus-visible`, `:active`) belongs to that state and to no other.
        // Folding all four rungs into one flat map — which is what this did
        // before m18 — is not a rounding error, it is visibly wrong on any
        // Tailwind page, because Tailwind builds `ring` and `shadow` out of
        // custom properties and reads them back from a BASE-state declaration:
        //
        //     .focus-visible\:ring-2:focus-visible { --tw-ring-shadow: <a red ring>; … }
        //     .shadow-none                         { box-shadow: var(--tw-ring-offset-shadow),
        //                                                        var(--tw-ring-shadow),
        //                                                        var(--tw-shadow) }
        //
        // With one flat map the focus ring is permanently lit: every
        // shadcn tab, button and input paints its focus ring at rest. (On
        // apps/meet-probe that was a solid red rectangle behind the active tab
        // — gpui paints a shadow as a filled quad behind a transparent element,
        // so a 4px-spread ring reads as a fill.)
        //
        // So: `own_custom` is the BASE map, and each rung that declares custom
        // properties of its own gets a map resolved on top of it, used only
        // when that rung's `var()`-composed declarations are applied.
        let mut own_custom: Vec<&(String, String)> = Vec::new();
        let mut state_custom: [Vec<&(String, String)>; STATE_RUNGS] = Default::default();
        for important_pass in [false, true] {
            for r in &matched {
                let declared = if important_pass { &r.important_custom } else { &r.normal_custom };
                if let Some(list) = declared {
                    match rung_of(r.states) {
                        None => own_custom.extend(list.iter()),
                        Some(rung) => state_custom[rung].extend(list.iter()),
                    }
                }
            }
        }
        if let Some(inline) = inline {
            own_custom.extend(inline.custom.iter());
        }
        let vars = vars::resolve(parent_vars, &own_custom, env, diagnostics);
        // Lazily, and only for the rungs that actually declare something: the
        // resolve is the expensive half of the cascade, and the overwhelming
        // majority of elements declare nothing on any rung.
        let state_vars: [Option<Rc<Vars>>; STATE_RUNGS] = std::array::from_fn(|rung| {
            if state_custom[rung].is_empty() {
                return None;
            }
            let mut combined = own_custom.clone();
            combined.extend(state_custom[rung].iter().copied());
            Some(vars::resolve(parent_vars, &combined, env, diagnostics))
        });
        let t2 = probe.then(std::time::Instant::now);

        // ---- 2. the refinements.
        let mut base = StyleRefinement::default();
        let mut hover = StyleRefinement::default();
        let mut active = StyleRefinement::default();
        let mut focus = StyleRefinement::default();
        let mut focus_visible = StyleRefinement::default();
        let (mut any_hover, mut any_active) = (false, false);
        let (mut any_focus, mut any_focus_visible) = (false, false);
        // `.group:hover .this` — one group per state, because gpui carries one
        // `group_hover` / one `group_active` per element. The key is resolved
        // once per (rule, element) pair and memoised for the pass: a Tailwind
        // card typically matches three or four `group-hover:` rules pointing at
        // the same ancestor.
        let mut group = GroupFold::default();
        // Transitions do not merge: the last base-state rule that declares any
        // `transition` replaces the whole list (`transition: none` therefore
        // clears it). State-rule transitions are rejected at compile time.
        let mut transitions: Option<Rc<Vec<TransitionSpec>>> = None;
        // Same rule for `animation`: the last base-state rule that declares any
        // `animation-*` replaces the whole list.
        let mut animations: Option<Rc<Vec<AnimationSpec>>> = None;
        // Longhand-only `animation-*` declarations, folded in cascade order and
        // applied over whichever list wins.
        let mut animation_patch = keyframes::AnimationPatch::default();
        // `object-fit` has no state ladder to install into (compile time already
        // advised about that), so every matched rule folds into one value in
        // cascade order — the last, most specific declaration wins.
        let mut extras = Extras::default();
        // The two generated boxes fold in parallel with the element's own
        // declarations, out of the rules that carry a `::before` / `::after`.
        // They stay OUT of `base`: a `::after { content: "" }` must not give
        // the originating element itself an empty content string.
        let mut before = (StyleRefinement::default(), Extras::default());
        let mut after = (StyleRefinement::default(), Extras::default());
        let mut placeholder = (StyleRefinement::default(), Extras::default());
        let (mut any_before, mut any_after, mut any_placeholder) = (false, false, false);
        for important_pass in [false, true] {
            for r in &matched {
                if let Some(which) = r.pseudo {
                    let (slot, seen) = match which {
                        PseudoElement::Before => (&mut before, &mut any_before),
                        PseudoElement::After => (&mut after, &mut any_after),
                        PseudoElement::Placeholder => (&mut placeholder, &mut any_placeholder),
                    };
                    *seen = true;
                    slot.1.refine(if important_pass {
                        &r.important_extras
                    } else {
                        &r.normal_extras
                    });
                    let refinement = if important_pass { &r.important } else { &r.normal };
                    if let Some(refinement) = refinement {
                        slot.0.refine(refinement);
                    }
                    let dynamic =
                        if important_pass { &r.important_dynamic } else { &r.normal_dynamic };
                    if let Some(list) = dynamic {
                        for (name, raw) in list.iter() {
                            mapper::apply_dynamic(
                                name, raw, &vars, env, &mut slot.0, &mut slot.1, diagnostics,
                            );
                        }
                    }
                    continue;
                }
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
                    let declared = if important_pass {
                        &r.important_animations
                    } else {
                        &r.normal_animations
                    };
                    if let Some(a) = declared {
                        animations = Some(a.clone());
                    }
                    let patch = if important_pass {
                        &r.important_animation_patch
                    } else {
                        &r.normal_animation_patch
                    };
                    if let Some(p) = patch {
                        p.apply_over(&mut animation_patch);
                    }
                }
                let refinement = if important_pass { &r.important } else { &r.normal };
                let dynamic = if important_pass { &r.important_dynamic } else { &r.normal_dynamic };
                if refinement.is_none() && dynamic.is_none() {
                    continue;
                }
                // `.group:hover .this` goes to its own bucket, keyed by the
                // ancestor it watches. Resolving the ancestor is a walk up from
                // this element re-matching the selector's PREFIX, which the
                // full match (run with `:hover` assumed true) already proved
                // has a solution — so a `None` here means the tree moved under
                // us and the rule simply does not apply.
                if let Some(a) = r.ancestor {
                    let Some(key) = nearest_group_ancestor(&node, r.selector, a.offset) else {
                        continue;
                    };
                    let Some(bucket) = group.bucket(a.states, key) else {
                        continue;
                    };
                    if let Some(refinement) = refinement {
                        bucket.refine(refinement);
                    }
                    if let Some(list) = dynamic {
                        for (name, raw) in list.iter() {
                            mapper::apply_dynamic(
                                name, raw, &vars, env, bucket, &mut extras, diagnostics,
                            );
                        }
                    }
                    continue;
                }
                // One rung per rule. A compound that names several ladder
                // states (`:hover:focus`) cannot be expressed as a conjunction
                // in gpui's ladder, so it lands on the strongest rung it names
                // — `classify_states` already warned about the approximation.
                let rung = rung_of(r.states);
                let bucket = match rung {
                    None => &mut base,
                    Some(0) => {
                        any_active = true;
                        &mut active
                    }
                    Some(1) => {
                        any_hover = true;
                        &mut hover
                    }
                    Some(2) => {
                        any_focus_visible = true;
                        &mut focus_visible
                    }
                    _ => {
                        any_focus = true;
                        &mut focus
                    }
                };
                if let Some(refinement) = refinement {
                    bucket.refine(refinement);
                }
                if let Some(list) = dynamic {
                    // A rung's `var()`-composed declarations read that rung's
                    // custom properties (base map + whatever the rung declares
                    // on top), never another rung's. `hover:shadow-lg` sets
                    // `--tw-shadow` and reads it back in the same `:hover`
                    // rule; `focus-visible:ring-2` does the same one rung over,
                    // and the two must not see each other.
                    let scope = rung
                        .and_then(|i| state_vars[i].as_ref())
                        .unwrap_or(&vars);
                    for (name, raw) in list.iter() {
                        mapper::apply_dynamic(name, raw, scope, env, bucket, &mut extras, diagnostics);
                    }
                }
            }
        }
        if let Some(inline) = inline {
            if let Some(r) = &inline.refinement {
                base.refine(r);
            }
            for (name, raw) in &inline.dynamic {
                mapper::apply_dynamic(name, raw, &vars, env, &mut base, &mut extras, diagnostics);
            }
            if let Some(t) = &inline.transitions {
                transitions = Some(t.clone());
            }
            if let Some(a) = &inline.animations {
                animations = Some(a.clone());
            }
            if let Some(p) = &inline.animation_patch {
                p.apply_over(&mut animation_patch);
            }
            extras.refine(&inline.extras);
        }
        if group.conflict {
            diagnostics.push(
                "this element is styled by `group-hover:` rules watching TWO different \
                 ancestors — gpui gives an element one group to watch, so only the first \
                 (most specific) one applies. Give the outer group its own named variant \
                 (`group//name` + `group-hover/name:`) on a different element"
                    .to_string(),
            );
        }
        if let (Some(t0), Some(t1), Some(t2)) = (t0, t1, t2) {
            let ms = |a: std::time::Instant, b: std::time::Instant| {
                b.saturating_duration_since(a).as_secs_f32() * 1000.0
            };
            perf::note_node(tested, ms(t0, t1), ms(t1, t2), ms(t2, std::time::Instant::now()));
        }
        ComputedStates {
            base: Rc::new(base),
            hover: any_hover.then(|| Rc::new(hover)),
            active: any_active.then(|| Rc::new(active)),
            focus: any_focus.then(|| Rc::new(focus)),
            focus_visible: any_focus_visible.then(|| Rc::new(focus_visible)),
            group_hover: group.hover.map(|(key, r)| (key, Rc::new(r))),
            group_active: group.active.map(|(key, r)| (key, Rc::new(r))),
            transitions: transitions.unwrap_or_default(),
            animations: {
                let list = animations.unwrap_or_default();
                if animation_patch.is_empty() || list.is_empty() {
                    list
                } else {
                    // A longhand set without a name patches the list rather
                    // than replacing it (see `AnimationPatch`).
                    let mut patched = (*list).clone();
                    for spec in patched.iter_mut() {
                        animation_patch.apply(spec);
                    }
                    Rc::new(patched)
                }
            },
            extras,
            // CSS generates NO box when `content` is absent, which is what
            // makes `content` the gate here rather than a property of a box
            // that would exist anyway.
            before: any_before
                .then(|| before.1.content.clone())
                .flatten()
                .map(|content| {
                    Rc::new(PseudoBox {
                        content,
                        style: Rc::new(before.0),
                        extras: before.1,
                    })
                }),
            after: any_after
                .then(|| after.1.content.clone())
                .flatten()
                .map(|content| {
                    Rc::new(PseudoBox {
                        content,
                        style: Rc::new(after.0),
                        extras: after.1,
                    })
                }),
            placeholder: any_placeholder.then(|| Rc::new(placeholder.0)),
            vars,
        }
    }
}

impl Engine {
    /// Compiles one `@keyframes` block.
    ///
    /// The steps are flattened (`0%, 100% { … }` is two steps sharing one
    /// declaration block), sorted by offset, and each one keeps the
    /// `animation-timing-function` it declares — which in CSS governs the
    /// SEGMENT STARTING at that keyframe, not the animation as a whole.
    fn collect_keyframes(
        &mut self,
        rule: &'static lightningcss::rules::keyframes::KeyframesRule<'static>,
    ) {
        use lightningcss::rules::keyframes::{KeyframeSelector, KeyframesName};
        let name: Box<str> = match &rule.name {
            KeyframesName::Ident(i) => i.0.as_ref().into(),
            KeyframesName::Custom(c) => c.as_ref().into(),
        };
        let mut steps: Vec<keyframes::Keyframe> = Vec::new();
        for frame in &rule.keyframes {
            // A keyframe block is an ordinary declaration block, so it goes
            // through the SAME mapper every rule does — which is what makes the
            // animatable property set identical to `transition`'s by
            // construction rather than by a second list that could drift.
            let converted = mapper::convert_block(&frame.declarations, &mut self.diagnostics);
            let mut style = StyleRefinement::default();
            let mut any = false;
            if let Some(r) = &converted.normal {
                style.refine(r);
                any = true;
            }
            if let Some(r) = &converted.important {
                // `!important` inside a keyframe is ignored by CSS Animations
                // §3; folding it is closer to intent than dropping the
                // declaration, and the difference is unobservable here.
                style.refine(r);
                any = true;
            }
            // `animation-timing-function` inside a keyframe is the segment's
            // easing; the collector picked it up as a one-entry list.
            // `animation-timing-function` inside a keyframe arrives as a
            // longhand-only PATCH (no name), which is exactly what it is.
            let easing = converted
                .normal_animation_patch
                .or(converted.important_animation_patch)
                .and_then(|p| p.easing);
            // `transform` used to need a second channel here, because it
            // computed into `Extras` rather than into the refinement. It is a
            // real refinement property now (`Style::vn_transform`, m13 item 2),
            // so `any` above already sees it and a keyframe carries it like any
            // other animatable value.
            if !any && easing.is_none() {
                continue;
            }
            let style = Rc::new(style);
            for selector in &frame.selectors {
                let offset = match selector {
                    KeyframeSelector::From => 0.0,
                    KeyframeSelector::To => 1.0,
                    KeyframeSelector::Percentage(p) => p.0,
                    KeyframeSelector::TimelineRangePercentage(_) => {
                        mapper::once(
                            &mut self.diagnostics,
                            format!(
                                "a named timeline range in @keyframes {name} is not supported                                  (v1) — that keyframe was dropped"
                            ),
                        );
                        continue;
                    }
                };
                steps.push(keyframes::Keyframe {
                    offset: offset.clamp(0.0, 1.0),
                    easing,
                    style: style.clone(),
                });
            }
        }
        steps.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        // A later block with the same name replaces the earlier one, as in CSS.
        self.keyframes.insert(name, Rc::new(keyframes::Keyframes { steps }));
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

/// An interaction state named on a compound that is NOT the selector's subject
/// — `.group:hover .child`, Tailwind's `group-hover:` variant.
///
/// The state belongs to an ANCESTOR, so it cannot ride the subject's own rung
/// in gpui's per-element ladder. gpui has the matching primitive: an element
/// declares `.group(name)` and a descendant `.group_hover(name, style)`, and
/// the descendant's style is refined at PAINT time from the group's hitbox —
/// no restyle, no cascade re-run, no subscription of our own (see
/// `vn_tree::render`, and research/reports/m18/live-test-round2.md §2).
///
/// What the cascade has to supply is WHICH ancestor. `offset` is the index, in
/// the selector's match-order storage, of the first component of the compound
/// carrying the pseudo — exactly the `offset` argument
/// [`parcel_selectors::matching::matches_selector`] takes, so "does this
/// ancestor satisfy everything from the left edge up to and including
/// `.group:hover`" is answered by the real matcher rather than by a
/// re-implementation of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AncestorState {
    offset: usize,
    states: u8,
}

/// The nearest ancestor of `node` that satisfies everything in `selector` from
/// its left edge up to and including the compound at `offset` — i.e. the
/// element a `group-hover:` rule is watching.
///
/// NEAREST is the right answer and not an approximation: CSS says the rule
/// applies while ANY matching ancestor is hovered, and gpui's `GroupHitboxes`
/// is a stack whose `get` returns the innermost group of that name, so the two
/// agree. It is also what an author means — a `.group` inside a `.group` styles
/// its own card.
fn nearest_group_ancestor<D: MatchDom>(
    node: &D,
    selector: &Selector<'static>,
    offset: usize,
) -> Option<u64> {
    let mut cursor = node.parent();
    // A corrupt parent chain must not hang the cascade (same bound the tree's
    // own ancestor walk uses).
    for _ in 0..1024 {
        let anc = cursor?;
        let el = element::El(anc);
        let mut ctx: MatchingContext<'_, '_, element::Impl> =
            MatchingContext::new(MatchingMode::Normal, None, None, QuirksMode::NoQuirks);
        if matches_selector(selector, offset, None, &el, &mut ctx, &mut |_, _| {}) {
            return Some(anc.key());
        }
        cursor = anc.parent();
    }
    None
}

/// The `.group:hover .this` half of one element's fold.
///
/// One slot per state, because gpui carries one `group_hover` and one
/// `group_active` per element. A second, DIFFERENT ancestor claiming the same
/// slot is a real limit rather than a rounding error, so it is advised rather
/// than silently merged.
#[derive(Default)]
struct GroupFold {
    hover: Option<(u64, StyleRefinement)>,
    active: Option<(u64, StyleRefinement)>,
    conflict: bool,
}

impl GroupFold {
    /// The refinement bucket for `key`'s `states`, or `None` when a different
    /// ancestor already owns that slot.
    fn bucket(&mut self, states: u8, key: u64) -> Option<&mut StyleRefinement> {
        let conflict = &mut self.conflict;
        let slot = if states & STATE_ACTIVE != 0 { &mut self.active } else { &mut self.hover };
        match slot.as_ref().map(|(owner, _)| *owner) {
            Some(owner) if owner != key => {
                *conflict = true;
                return None;
            }
            Some(_) => {}
            None => *slot = Some((key, StyleRefinement::default())),
        }
        slot.as_mut().map(|(_, r)| r)
    }
}

/// Classifies a selector's interaction-state bucket from its RIGHTMOST
/// compound, plus the ancestor-state descriptor when a state pseudo sits on a
/// compound further left (`.group:hover .child`).
fn classify_states(
    sel: &Selector<'_>,
    diagnostics: &mut Vec<String>,
) -> Option<(u8, Option<AncestorState>)> {
    let mut states = 0u8;
    let mut ancestor: Option<AncestorState> = None;
    let mut iter = sel.iter();
    // Rightmost compound. `compound_start` tracks where the compound currently
    // being read begins in the match-order storage: a compound's components are
    // contiguous and every sequence boundary is exactly one stored
    // `Component::Combinator`.
    let mut compound_start = 0usize;
    let mut compound_len = 0usize;
    for component in &mut iter {
        compound_len += 1;
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
    // Remaining compounds: any state pseudo there is an ancestor-state rule…
    //
    // …EXCEPT across a pseudo-element boundary. parcel models `::after` as its
    // own sequence joined by `Combinator::PseudoElement`, so in
    // `.nav:hover::after` the `:hover` sits in the "next" sequence while still
    // being on the SAME element. Reading it as an ancestor state produced a
    // wrong advisory for what is an ordinary — if unhonourable — state rule.
    let mut same_element = true;
    while let Some(combinator) = iter.next_sequence() {
        compound_start += compound_len + 1; // the compound, then its combinator
        compound_len = 0;
        same_element &= matches!(
            combinator,
            parcel_selectors::parser::Combinator::PseudoElement
                | parcel_selectors::parser::Combinator::SlotAssignment
                | parcel_selectors::parser::Combinator::Part
        );
        let sibling = matches!(
            combinator,
            parcel_selectors::parser::Combinator::NextSibling
                | parcel_selectors::parser::Combinator::LaterSibling
        );
        let mut here = 0u8;
        for component in &mut iter {
            compound_len += 1;
            if let Component::NonTSPseudoClass(pc) = component {
                here |= ladder_state(pc);
            }
        }
        if here == 0 {
            continue;
        }
        if same_element {
            states |= here;
            continue;
        }
        // A SIBLING's state (`.a:hover + .b`) has no group primitive: gpui keys
        // a group off an ancestor's hitbox, and a sibling is not one. Dropped
        // with the same diagnostic ancestor states used to get.
        if sibling {
            diagnostics.push(format!(
                "a sibling-state selector (`+` / `~` with `:hover` on the LEFT) has nothing to \
                 hang on — gpui's group mechanism keys off an ANCESTOR's hitbox. The rule was \
                 dropped: {}",
                sel_text(sel)
            ));
            return None;
        }
        // Only hover and active have a group primitive.
        if here & !(STATE_HOVER | STATE_ACTIVE) != 0 {
            mapper::once(
                diagnostics,
                format!(
                    "`:focus` / `:focus-visible` on an ANCESTOR compound (`group-focus:` and \
                     friends) is not implemented — gpui has `group_hover` / `group_active` and no \
                     group focus. The rule was dropped: {}",
                    sel_text(sel)
                ),
            );
            return None;
        }
        // Two ancestor states in one selector would need two groups on one
        // element, and gpui carries one `group_hover` / one `group_active`.
        if ancestor.is_some() {
            mapper::once(
                diagnostics,
                format!(
                    "two ancestor-state compounds in one selector — only the INNERMOST is \
                     honoured, because gpui gives an element one group to watch: {}",
                    sel_text(sel)
                ),
            );
            continue;
        }
        ancestor = Some(AncestorState { offset: compound_start, states: here });
    }
    Some((states, ancestor))
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
/// `::before` / `::after` -> the generated box they target. Everything else is
/// `None`: it styles part of a box (`::first-line`, `::marker`, `::selection`)
/// rather than generating one, and there is nothing to build.
fn pseudo_box(
    pe: &<element::Impl as parcel_selectors::parser::SelectorImpl<'_>>::PseudoElement,
) -> Option<PseudoElement> {
    use lightningcss::selector::PseudoElement as PE;
    match pe {
        PE::Before => Some(PseudoElement::Before),
        PE::After => Some(PseudoElement::After),
        // `::-webkit-input-placeholder` and `::-moz-placeholder` are the same
        // thing; lightningcss folds every spelling into one variant.
        PE::Placeholder(_) => Some(PseudoElement::Placeholder),
        _ => None,
    }
}

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
            // `::before` / `::after` are REAL here: they generate a box the
            // renderer builds as a child of the originating element. Every
            // other pseudo-element still styles something that does not exist.
            Component::PseudoElement(pe) if !nested && pseudo_box(pe).is_some() => None,
            // One advisory for the whole family, not one per spelling: a
            // Tailwind preflight sheet names eight of them and eight lines
            // would bury everything else. The example selector rides along.
            Component::PseudoElement(_) => Some(
                "a pseudo-element other than `::before` / `::after` / `::placeholder` (nothing here paints part of a box separately from the box)"
                    .into(),
            ),
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
        fn key(&self) -> u64 {
            0
        }
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
        fn tag_name(&self) -> &str {
            ""
        }
        fn each_class(&self, _: &mut dyn FnMut(&str)) {}
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

/// Names the extras a block actually declared, so the state-selector advisory
/// says `window-drag` when that is what was written and `object-fit` when it
/// is not.
fn extras_names(e: &Extras) -> String {
    let mut names: Vec<&str> = Vec::new();
    if e.object_fit.is_some() {
        names.push("`object-fit`");
    }
    if e.window_drag.is_some() {
        names.push("`window-drag`");
    }
    if e.window_control.is_some() {
        names.push("`--window-control`");
    }
    names.join(" / ")
}

fn sel_text(sel: &Selector<'_>) -> String {
    use lightningcss::traits::ToCss;
    sel.to_css_string(lightningcss::printer::PrinterOptions::default())
        .unwrap_or_else(|_| "<selector>".into())
}

// ---------------------------------------------------------------------------
// standalone selector queries (devtools / future `vn test`)
// ---------------------------------------------------------------------------

/// What [`query_all`] found.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Indices into the `candidates` slice, in the order they were given
    /// (document order, if the caller passed them that way).
    pub matched: Vec<usize>,
    /// Advisories that did NOT stop the query — the same text the sheet
    /// compiler would print. The caller is expected to surface them.
    pub warnings: Vec<String>,
}

/// Matches a standalone selector against a list of candidate nodes.
///
/// This is deliberately the SAME code path the cascade uses — the selector is
/// parsed by lightningcss, run through [`classify_states`], [`probe_runnable`]
/// and [`audit_selector`], and matched by `parcel_selectors` against
/// [`element::El`]. A selector the sheet compiler would reject comes back as
/// `Err(advisory)` rather than silently matching nothing, which is the whole
/// point: `query(":has(.x)")` must say why, exactly like a rule using it does.
///
/// Candidates rather than a root node because [`MatchDom`] deliberately has no
/// child accessor (the matcher never walks downwards); the caller owns the tree
/// and hands us its elements in document order.
pub fn query_all<D: MatchDom>(selector: &str, candidates: &[D]) -> Result<QueryResult, String> {
    let sel = selector.trim();
    if sel.is_empty() {
        return Err("empty selector".into());
    }
    // The selector is spliced into a one-rule stylesheet to reach exactly the
    // parser the cascade uses; `{`/`}` would let it declare rules of its own.
    if sel.contains(['{', '}']) {
        return Err("a selector cannot contain `{` or `}`".into());
    }
    let src = format!("{sel}{{color:red}}");
    let sheet = StyleSheet::parse(&src, ParserOptions::default())
        .map_err(|e| format!("selector parse error: {e}"))?;
    let mut diagnostics: Vec<String> = Vec::new();
    let mut selectors: Vec<&Selector<'_>> = Vec::new();
    for rule in &sheet.rules.0 {
        let CssRule::Style(style) = rule else {
            return Err(format!("`{sel}` is not a selector"));
        };
        for s in &style.selectors.0 {
            let (states, ancestor) = match classify_states(s, &mut diagnostics) {
                Some(pair) => pair,
                None => return Err(join_diagnostics(&diagnostics)),
            };
            if !probe_runnable(s) {
                return Err(format!(
                    "selector rejected (matcher cannot run it — :has()/nesting?): {}",
                    sel_text(s)
                ));
            }
            if !audit_selector(s, &mut diagnostics) {
                return Err(join_diagnostics(&diagnostics));
            }
            if states != 0 || ancestor.is_some() {
                // Honest about the one place a query diverges from a browser's
                // `querySelectorAll`: the cascade answers interaction pseudos
                // with "would match when in that state" (gpui's ladder picks
                // them live), so they cannot filter a query either.
                diagnostics.push(format!(
                    "`{}` names an interaction state — the matcher answers `:hover`/`:active`/\
                     `:focus`/`:focus-visible` as \"would match when in that state\" (the cascade's \
                     own semantics), so the query is NOT filtered by the live state",
                    sel_text(s)
                ));
            }
            selectors.push(s);
        }
    }
    if selectors.is_empty() {
        return Err(format!("`{sel}` matched no selector"));
    }
    let mut matched = Vec::new();
    for (i, candidate) in candidates.iter().enumerate() {
        let el = element::El(*candidate);
        let hit = selectors.iter().any(|s| {
            let mut ctx: MatchingContext<'_, '_, element::Impl> =
                MatchingContext::new(MatchingMode::Normal, None, None, QuirksMode::NoQuirks);
            matches_selector(s, 0, None, &el, &mut ctx, &mut |_, _| {})
        });
        if hit {
            matched.push(i);
        }
    }
    Ok(QueryResult { matched, warnings: diagnostics })
}

fn join_diagnostics(diagnostics: &[String]) -> String {
    if diagnostics.is_empty() {
        "selector rejected".to_string()
    } else {
        diagnostics.join("; ")
    }
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
        fn key(&self) -> u64 {
            0
        }
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
        fn tag_name(&self) -> &str {
            self.tag
        }
        fn each_class(&self, f: &mut dyn FnMut(&str)) {
            for c in self.class.split_whitespace() {
                f(c);
            }
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

    // -----------------------------------------------------------------------
    // group-hover: an interaction state on an ANCESTOR compound
    // -----------------------------------------------------------------------

    /// A three-level document: `.card.group > .row > .icon`, plus a second card
    /// so "nearest ancestor" has something to be wrong about.
    fn group_doc() -> Doc {
        Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "card group", 0, vec![]),   // 1
                ("div", "row", 1, vec![]),          // 2
                ("span", "icon", 2, vec![]),        // 3
                ("div", "card group", 0, vec![]),   // 4
                ("span", "icon", 4, vec![]),        // 5
            ],
        }
    }

    #[test]
    fn group_hover_folds_onto_its_ancestor_and_not_onto_the_element() {
        let engine = Engine::compile(
            ".group:hover .icon { color: rgb(255 0 0) } .icon { color: rgb(0 0 0) }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        assert!(engine.uses_group_states());
        let d = group_doc();
        let states = cascade(&engine, &d, &Env::default());
        let icon = &states[2]; // node 3
        // The base state is untouched — the group rule is not a base rule…
        assert_eq!(icon.base.text.color, Some(gpui::rgb(0x000000).into()));
        // …the element's own `:hover` rung is untouched — this is not its state…
        assert!(icon.hover.is_none());
        // …and the group bucket names the ANCESTOR that carries `.group`.
        let (anchor, style) = icon.group_hover.as_ref().expect("a group-hover bucket");
        assert_eq!(*anchor, 1);
        assert_eq!(style.text.color, Some(gpui::rgb(0xff0000).into()));
    }

    #[test]
    fn group_hover_picks_the_nearest_matching_ancestor() {
        // Both cards carry `.group`; each icon must watch its OWN card, which is
        // also what gpui's `GroupHitboxes` stack resolves a name to.
        let engine = Engine::compile(".group:hover .icon { opacity: 0.5 }");
        let d = group_doc();
        let states = cascade(&engine, &d, &Env::default());
        assert_eq!(states[2].group_hover.as_ref().map(|(k, _)| *k), Some(1));
        assert_eq!(states[4].group_hover.as_ref().map(|(k, _)| *k), Some(4));
    }

    #[test]
    fn group_active_has_its_own_bucket() {
        let engine = Engine::compile(".group:active .icon { opacity: 0.5 }");
        let d = group_doc();
        let states = cascade(&engine, &d, &Env::default());
        assert!(states[2].group_hover.is_none());
        assert_eq!(states[2].group_active.as_ref().map(|(k, _)| *k), Some(1));
    }

    /// The rule still has to MATCH: an element outside the group gets nothing.
    #[test]
    fn group_hover_does_not_reach_outside_the_group() {
        let engine = Engine::compile(".group:hover .icon { opacity: 0.5 }");
        let d = doc(); // no `.group` anywhere
        let states = cascade(&engine, &d, &Env::default());
        assert!(states.iter().all(|s| s.group_hover.is_none()));
    }

    #[test]
    fn an_ancestor_focus_selector_is_dropped_with_a_diagnostic() {
        // gpui has `group_hover` and `group_active` and no group focus, so
        // `group-focus:` is refused rather than quietly applied at rest.
        let engine = Engine::compile(".box:focus .child { opacity: 0.4 }");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("on an ANCESTOR compound")),
            "{:?}",
            engine.diagnostics
        );
        assert!(!engine.uses_group_states());
    }

    #[test]
    fn a_sibling_state_selector_is_dropped_with_a_diagnostic() {
        // A group is keyed off an ANCESTOR's hitbox; a sibling is not one.
        let engine = Engine::compile(".a:hover + .b { opacity: 0.4 }");
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("sibling-state selector")),
            "{:?}",
            engine.diagnostics
        );
    }

    /// `.nav:hover::after` is NOT an ancestor state — parcel models a
    /// pseudo-element as its own sequence, so the `:hover` sits one sequence
    /// over while still being on the same element.
    #[test]
    fn a_state_before_a_pseudo_element_is_not_an_ancestor_state() {
        let engine = Engine::compile(r#".nav:hover::after { content: "x" }"#);
        assert!(!engine.uses_group_states(), "{:?}", engine.diagnostics);
    }

    // -----------------------------------------------------------------------
    // viewport units
    // -----------------------------------------------------------------------

    #[test]
    fn viewport_units_resolve_against_the_published_viewport() {
        assert!(set_viewport_basis(1000.0, 800.0));
        let engine = Engine::compile(
            ".box { width: 90vw; max-height: 85vh; min-width: 50vmin; padding-top: 10vmax }",
        );
        assert!(uses_viewport_units());
        let s = engine.compute(box_node(), None, &Env::default());
        assert_eq!(s.base.size.width, Some(gpui::px(900.0).into()));
        assert_eq!(s.base.max_size.height, Some(gpui::px(680.0).into()));
        assert_eq!(s.base.min_size.width, Some(gpui::px(400.0).into()));
        assert_eq!(s.base.padding.top, Some(gpui::px(100.0).into()));
        // Same basis twice is not a change — the caller must not recompile.
        assert!(!set_viewport_basis(1000.0, 800.0));
        assert!(set_viewport_basis(1280.0, 720.0));
        set_viewport_basis(INITIAL_VIEWPORT_BASIS.0, INITIAL_VIEWPORT_BASIS.1);
    }

    #[test]
    fn a_sheet_without_viewport_units_does_not_ask_for_a_recompile() {
        Engine::compile(".box { width: 10px }");
        assert!(!uses_viewport_units());
    }

    #[test]
    fn calc_over_viewport_units_resolves_too() {
        assert!(set_viewport_basis(1000.0, 800.0));
        // reka's dialog body: `h-[calc(85vh-80px)]`.
        let engine = Engine::compile(".box { height: calc(85vh - 80px) }");
        let s = engine.compute(box_node(), None, &Env::default());
        assert_eq!(s.base.size.height, Some(gpui::px(600.0).into()));
        set_viewport_basis(INITIAL_VIEWPORT_BASIS.0, INITIAL_VIEWPORT_BASIS.1);
    }

    #[test]
    fn inline_level_display_is_recorded_for_the_renderer() {
        let engine = Engine::compile(
            ".a { display: inline-flex } .b { display: flex } .c { display: inline }",
        );
        let a = engine.compute(Node { tag: "div", class: "a", attrs: &[] }, None, &Env::default());
        let b = engine.compute(Node { tag: "div", class: "b", attrs: &[] }, None, &Env::default());
        let c = engine.compute(Node { tag: "div", class: "c", attrs: &[] }, None, &Env::default());
        assert_eq!(a.extras.inline_level, Some(true));
        assert_eq!(a.base.display, Some(gpui::Display::Flex));
        assert_eq!(b.extras.inline_level, Some(false));
        assert_eq!(c.extras.inline_level, Some(true));
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
        Env { viewport_width: width, viewport_height: 800.0, dark, reduce_motion, ..Env::default() }
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
            // `::before` / `::after` / `::placeholder` DO compile now; the rest
            // of the family still cannot match anything.
            (".box::first-line", "pseudo-element"),
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

    // -----------------------------------------------------------------------
    // window-drag / --window-control (docs/WINDOWING.md §2.1, §2.2)
    // -----------------------------------------------------------------------

    fn drag_of(css: &str) -> Option<WindowDrag> {
        Engine::compile(css)
            .compute(box_node(), None, &Env::default())
            .extras
            .window_drag
    }

    /// All three property spellings, both value vocabularies, no advisory —
    /// Chromium keeps `app-region`/`-webkit-app-region` permanently, so warning
    /// about them would be pure noise (D-13).
    #[test]
    fn every_window_drag_spelling_parses_without_an_advisory() {
        for (css, want) in [
            ("window-drag: move", WindowDrag::Move),
            ("window-drag: none", WindowDrag::None),
            ("app-region: drag", WindowDrag::Move),
            ("app-region: no-drag", WindowDrag::None),
            ("-webkit-app-region: drag", WindowDrag::Move),
            ("-webkit-app-region: no-drag", WindowDrag::None),
        ] {
            let engine = Engine::compile(&format!(".box {{ {css} }}"));
            assert!(engine.diagnostics.is_empty(), "{css}: {:?}", engine.diagnostics);
            assert_eq!(
                engine.compute(box_node(), None, &Env::default()).extras.window_drag,
                Some(want),
                "{css}"
            );
        }
    }

    /// Undeclared must stay `None`, distinct from an explicit `none`: the
    /// renderer INHERITS the parent's effective value through the `None` case
    /// and stops at the explicit one.
    #[test]
    fn undeclared_window_drag_is_distinct_from_an_explicit_none() {
        assert_eq!(drag_of(".box { opacity: 0.5 }"), None);
        assert_eq!(drag_of(".box { window-drag: none }"), Some(WindowDrag::None));
    }

    #[test]
    fn window_drag_folds_by_specificity_and_importance() {
        assert_eq!(
            drag_of(".box { window-drag: move } div.box { app-region: no-drag }"),
            Some(WindowDrag::None)
        );
        assert_eq!(
            drag_of("div.box { window-drag: none } .box { window-drag: move !important }"),
            Some(WindowDrag::Move)
        );
    }

    #[test]
    fn bad_window_drag_value_is_advised_and_dropped() {
        let engine = Engine::compile(".box { window-drag: sideways }");
        assert!(
            engine
                .diagnostics
                .iter()
                .any(|d| d.contains("window-drag") && d.contains("sideways")),
            "{:?}",
            engine.diagnostics
        );
        assert_eq!(
            engine.compute(box_node(), None, &Env::default()).extras.window_drag,
            None
        );
    }

    #[test]
    fn window_control_keywords_all_map() {
        for (css, want) in [
            ("minimize", WindowControl::Minimize),
            ("maximize", WindowControl::Maximize),
            ("close", WindowControl::Close),
            ("none", WindowControl::None),
        ] {
            let engine = Engine::compile(&format!(".box {{ --window-control: {css} }}"));
            assert!(engine.diagnostics.is_empty(), "{css}: {:?}", engine.diagnostics);
            assert_eq!(
                engine.compute(box_node(), None, &Env::default()).extras.window_control,
                Some(want),
                "--window-control: {css}"
            );
        }
    }

    /// `--window-control` must not fall into the custom-property channel that
    /// every `--tw-*` declaration travels: it is an interpreted property that
    /// merely borrows the spelling, it lands on `Extras`, and — unlike a real
    /// custom property — it is deliberately NOT inherited (D-14).
    ///
    /// (Before compat round 6 this was observable as an "custom properties are
    /// stored nowhere" advisory at sheet-compile time. Custom properties are
    /// stored for real now, so the assertion moved to where the value lands.)
    #[test]
    fn window_control_does_not_fold_into_the_custom_property_map() {
        let mut diags = Vec::new();
        let engine = Engine::compile(".box { --window-control: close; --tw-shadow: 0 0 0 red }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let computed = engine.compute_node(
            box_node(),
            None,
            &Rc::new(Vars::default()),
            &Env::default(),
            &mut diags,
        );
        // Interpreted: it reached `Extras`…
        assert_eq!(computed.extras.window_control, Some(WindowControl::Close));
        // …and it is NOT in the map children would inherit, while a genuine
        // custom property declared in the same block is.
        assert_eq!(computed.vars.get(mapper::WINDOW_CONTROL), None);
        assert!(computed.vars.get("--tw-shadow").is_some(), "{:?}", computed.vars);
    }

    #[test]
    fn bad_window_control_value_is_advised() {
        let engine = Engine::compile(".box { --window-control: restore }");
        assert!(
            engine
                .diagnostics
                .iter()
                .any(|d| d.contains("--window-control") && d.contains("restore")),
            "{:?}",
            engine.diagnostics
        );
    }

    /// The hit test reads an already-PAINTED frame, so a `:hover` drag region
    /// would be a paint race. It applies unconditionally and says so (A-12) —
    /// and the advisory names the property that was actually written.
    #[test]
    fn window_drag_on_a_state_selector_is_advised_by_name() {
        let engine = Engine::compile(".box:hover { window-drag: none }");
        assert!(
            engine
                .diagnostics
                .iter()
                .any(|d| d.contains("`window-drag`") && d.contains("unconditionally")),
            "{:?}",
            engine.diagnostics
        );
        assert!(
            !engine.diagnostics.iter().any(|d| d.contains("object-fit")),
            "the advisory must name the property that was written: {:?}",
            engine.diagnostics
        );
    }

    /// `env()` is `var()` with a scope the page cannot write. An `env()`-composed
    /// declaration already arrives as `Property::Unparsed` and therefore already
    /// rides the DYNAMIC channel, which is why the whole feature is one lookup
    /// table plus one arm in the scanner (docs/WINDOWING.md §2.4; A-15 was the
    /// placeholder these tests replace).
    ///
    /// `(computed base refinement, diagnostics)` for `.box` under `env`.
    fn computed_with_env(css: &str, env: &Env) -> (Rc<StyleRefinement>, Vec<String>) {
        let engine = Engine::compile(css);
        let mut diags = engine.diagnostics.clone();
        let states =
            engine.compute_node(box_node(), None, &Rc::new(Vars::default()), env, &mut diags);
        (states.base, diags)
    }

    fn titlebar_env(x: f32, y: f32, w: f32, h: f32) -> Env {
        Env { titlebar_area: [x, y, w, h], ..Env::default() }
    }

    /// The whole point of A-15 becoming real: the number the host publishes is
    /// the number the cascade uses, with no advisory at all.
    #[test]
    fn env_titlebar_area_resolves_from_the_window_state() {
        let (base, diags) = computed_with_env(
            ".box { height: env(titlebar-area-height, 32px); width: env(titlebar-area-width) }",
            &titlebar_env(0.0, 0.0, 940.0, 36.0),
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(base.size.height, Some(gpui::px(36.0).into()));
        assert_eq!(base.size.width, Some(gpui::px(940.0).into()));

        // …and it MOVES with the window: same sheet, different env.
        let (base, _) = computed_with_env(
            ".box { height: env(titlebar-area-height) }",
            &titlebar_env(0.0, 0.0, 940.0, 48.0),
        );
        assert_eq!(base.size.height, Some(gpui::px(48.0).into()));
    }

    /// macOS puts the traffic lights on the LEFT, so `titlebar-area-x` is
    /// non-zero there — the reason the spec insists correct code reads the rect
    /// instead of assuming a side (§2.3).
    #[test]
    fn env_titlebar_area_x_is_a_real_offset_not_always_zero() {
        let (base, diags) = computed_with_env(
            ".box { margin-left: env(titlebar-area-x) }",
            &titlebar_env(78.0, 0.0, 862.0, 28.0),
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(base.margin.left, Some(gpui::px(78.0).into()));
    }

    /// The overlay is not visible in OS chrome / fullscreen, and the host
    /// publishes the zero rect there — exactly what a PWA in a tab reports. The
    /// declaration must still RESOLVE (to 0px), not be dropped.
    #[test]
    fn env_titlebar_area_is_zero_when_the_overlay_is_hidden() {
        let (base, diags) = computed_with_env(
            ".box { height: env(titlebar-area-height) }",
            &titlebar_env(0.0, 0.0, 0.0, 0.0),
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(base.size.height, Some(gpui::px(0.0).into()));
    }

    /// `safe-area-inset-*` resolve to zero on desktop rather than invalidating
    /// their declarations. A ported PWA stylesheet is full of them, and a
    /// fallback-less `env()` with no value drops the whole declaration.
    #[test]
    fn env_safe_area_insets_resolve_to_zero() {
        let (base, diags) = computed_with_env(
            ".box { padding-left: env(safe-area-inset-left); \
             padding-bottom: env(safe-area-inset-bottom) }",
            &Env::default(),
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(base.padding.left, Some(gpui::px(0.0).into()));
        assert_eq!(base.padding.bottom, Some(gpui::px(0.0).into()));
    }

    /// An unknown name behaves exactly as CSS says: the fallback applies, and
    /// without one the declaration is invalid at computed-value time — with an
    /// advisory that lists the names that DO exist, because an author cannot set
    /// an environment variable and "declare it" would be useless advice.
    #[test]
    fn an_unknown_env_uses_its_fallback_and_otherwise_drops_the_declaration() {
        let (base, diags) =
            computed_with_env(".box { height: env(keyboard-inset-height, 12px) }", &Env::default());
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(base.size.height, Some(gpui::px(12.0).into()));

        let (base, diags) =
            computed_with_env(".box { height: env(keyboard-inset-height) }", &Env::default());
        assert_eq!(base.size.height, None);
        assert!(
            diags.iter().any(|d| d.contains("keyboard-inset-height")
                && d.contains("titlebar-area-x")
                && d.contains("invalid at computed-value time")),
            "{diags:?}"
        );
    }

    /// `env()` and `var()` nest in each other's fallbacks and in custom
    /// properties — they are ONE substitution pass, so this is a property of the
    /// design rather than a case that had to be handled.
    #[test]
    fn env_and_var_nest_in_each_others_fallbacks_and_in_custom_properties() {
        let (base, diags) = computed_with_env(
            ".box { --bar: env(titlebar-area-height); height: var(--bar); \
             padding-top: var(--nope, env(titlebar-area-height)); \
             margin-top: env(nope, var(--bar)) }",
            &titlebar_env(0.0, 0.0, 800.0, 40.0),
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(base.size.height, Some(gpui::px(40.0).into()));
        assert_eq!(base.padding.top, Some(gpui::px(40.0).into()));
        assert_eq!(base.margin.top, Some(gpui::px(40.0).into()));
    }

    /// The scanner must not fire on the tail of a longer identifier — the same
    /// rule `var()` already obeyed for `--my-var(`.
    #[test]
    fn the_env_scanner_skips_identifier_tails() {
        let (base, diags) =
            computed_with_env(".box { height: my-env(titlebar-area-height) }", &Env::default());
        assert_eq!(base.size.height, None, "`my-env(` is not `env(`");
        assert!(
            !diags.iter().any(|d| d.contains("is not an environment variable")),
            "{diags:?}"
        );
    }

    #[test]
    fn inline_window_drag_beats_the_sheet() {
        let engine = Engine::compile("div.box { window-drag: move }");
        let mut diags = Vec::new();
        let (r, t, x) = parse_inline_full("window-drag: none", &mut diags);
        assert!(diags.is_empty(), "{diags:?}");
        let computed = engine.compute_full(
            box_node(),
            Some(&r),
            (!t.is_empty()).then(|| Rc::new(t)).as_ref(),
            x,
            &Env::default(),
        );
        assert_eq!(computed.extras.window_drag, Some(WindowDrag::None));
    }

    // -----------------------------------------------------------------------
    // query_all
    // -----------------------------------------------------------------------

    /// A flat forest with real parent/sibling links, enough for descendant and
    /// `:nth-child()` selectors.
    struct Doc {
        /// `(tag, classes, parent, attrs)`
        nodes: Vec<(&'static str, &'static str, usize, Vec<(&'static str, &'static str)>)>,
    }

    #[derive(Clone, Copy)]
    struct DocRef<'d> {
        doc: &'d Doc,
        i: usize,
    }

    impl<'d> DocRef<'d> {
        fn siblings(&self) -> Vec<usize> {
            let parent = self.doc.nodes[self.i].2;
            (0..self.doc.nodes.len())
                .filter(|&j| j != 0 && self.doc.nodes[j].2 == parent)
                .collect()
        }
    }

    impl<'d> MatchDom for DocRef<'d> {
        fn key(&self) -> u64 {
            self.i as u64
        }
        fn parent(&self) -> Option<Self> {
            let p = self.doc.nodes[self.i].2;
            (p != 0).then(|| DocRef { doc: self.doc, i: p })
        }
        fn prev_sibling(&self) -> Option<Self> {
            let sibs = self.siblings();
            let pos = sibs.iter().position(|&j| j == self.i)?;
            (pos > 0).then(|| DocRef { doc: self.doc, i: sibs[pos - 1] })
        }
        fn next_sibling(&self) -> Option<Self> {
            let sibs = self.siblings();
            let pos = sibs.iter().position(|&j| j == self.i)?;
            sibs.get(pos + 1).map(|&j| DocRef { doc: self.doc, i: j })
        }
        fn has_tag(&self, name: &str) -> bool {
            self.doc.nodes[self.i].0 == name
        }
        fn tag_eq(&self, other: &Self) -> bool {
            self.doc.nodes[self.i].0 == other.doc.nodes[other.i].0
        }
        fn has_class(&self, name: &str) -> bool {
            self.doc.nodes[self.i].1.split_whitespace().any(|c| c == name)
        }
        fn tag_name(&self) -> &str {
            self.doc.nodes[self.i].0
        }
        fn each_class(&self, f: &mut dyn FnMut(&str)) {
            for c in self.doc.nodes[self.i].1.split_whitespace() {
                f(c);
            }
        }
        fn has_scope(&self, _name: &str) -> bool {
            false
        }
        fn attr(&self, name: &str) -> Option<&str> {
            self.doc.nodes[self.i].3.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
        }
        fn is_empty(&self) -> bool {
            !self.doc.nodes.iter().any(|n| n.2 == self.i)
        }
        fn is_root(&self) -> bool {
            self.doc.nodes[self.i].2 == 0
        }
    }

    /// index 0 is the reserved "no parent" slot, exactly like the real tree.
    fn doc() -> Doc {
        Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "nav", 0, vec![]),
                ("a", "tab router-link-active", 1, vec![("href", "/")]),
                ("a", "tab", 1, vec![("href", "/list")]),
                ("input", "field", 1, vec![("disabled", "true")]),
            ],
        }
    }

    fn refs(d: &Doc) -> Vec<DocRef<'_>> {
        (1..d.nodes.len()).map(|i| DocRef { doc: d, i }).collect()
    }

    // -----------------------------------------------------------------------
    // ::before / ::after / ::placeholder
    // -----------------------------------------------------------------------

    /// A `::before` rule styles a GENERATED BOX, not the element it matched —
    /// so its declarations must land on `ComputedStates::before` and nowhere
    /// near `base`. (Before this, the rule was dropped at compile time.)
    #[test]
    fn a_pseudo_element_rule_becomes_its_own_box() {
        let engine = Engine::compile(
            r#".nav::before { content: "|"; width: 2px; background-color: rgb(255 0 0) }"#,
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = doc();
        let states = cascade(&engine, &d, &Env::default());
        let nav = &states[0];
        // The generated box exists, carries its content and its declarations…
        let before = nav.before.as_ref().expect("a ::before box");
        assert_eq!(&*before.content, "|");
        assert_eq!(before.style.size.width, Some(gpui::px(2.0).into()));
        assert!(before.style.background.is_some());
        // …and the ORIGINATING element got none of it.
        assert_eq!(nav.base.size.width, None);
        assert_eq!(nav.base.background, None);
        assert!(nav.after.is_none());
    }

    /// CSS generates no box at all when `content` is absent, and that is the
    /// gate here too — otherwise every Tailwind preflight sheet (which styles
    /// `*, ::before, ::after` to seed `--tw-*`) would give every element two
    /// empty boxes.
    #[test]
    fn a_pseudo_element_without_content_generates_nothing() {
        let engine = Engine::compile(".nav::after { width: 2px }");
        let d = doc();
        let states = cascade(&engine, &d, &Env::default());
        assert!(states[0].after.is_none(), "no content, no box");
    }

    /// The two boxes are independent, they cascade among themselves by
    /// (specificity, order), and an ATTRIBUTE-qualified rule works — which is
    /// how reka/shadcn drive an indicator (`[data-state=open]::after`).
    #[test]
    fn pseudo_boxes_cascade_and_follow_attributes() {
        let engine = Engine::compile(
            r#"a::before { content: "-"; width: 1px }
               a.tab::before { width: 4px }
               a::after { content: "+" }
               a[href="/list"]::after { content: "!" }"#,
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = doc();
        let states = cascade(&engine, &d, &Env::default());
        // nodes[2] is `a.tab[href="/"]`, nodes[3] is `a.tab[href="/list"]`.
        let first = &states[1];
        assert_eq!(first.before.as_ref().unwrap().style.size.width, Some(gpui::px(4.0).into()));
        assert_eq!(&*first.after.as_ref().unwrap().content, "+");
        let second = &states[2];
        assert_eq!(&*second.after.as_ref().unwrap().content, "!");
    }

    /// A generated box is not in gpui's pointer-state ladder, so a
    /// `:hover::after` rule cannot be honoured. It must SAY so and name the
    /// alternative rather than compile into a rule that never fires.
    #[test]
    fn a_state_qualified_pseudo_element_rule_advises() {
        let engine = Engine::compile(r#".nav:hover::after { content: "x" }"#);
        assert!(
            engine
                .diagnostics
                .iter()
                .any(|d| d.contains("state-qualified pseudo-element")),
            "{:?}",
            engine.diagnostics
        );
        let d = doc();
        assert!(cascade(&engine, &d, &Env::default())[0].after.is_none());
    }

    /// `::placeholder` is not a box — it styles the string the text-input
    /// widget paints — so it folds into its own refinement.
    #[test]
    fn placeholder_folds_into_its_own_refinement() {
        let engine = Engine::compile("input::placeholder { color: rgb(1 2 3) }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = doc();
        let states = cascade(&engine, &d, &Env::default());
        // nodes[4] is the `<input>`.
        let input = &states[3];
        assert!(input.placeholder.as_ref().unwrap().text.color.is_some());
        assert_eq!(input.base.text.color, None, "the input itself is untouched");
    }

    /// Every OTHER pseudo-element still cannot match, and the whole family
    /// shares ONE advisory — a preflight sheet names eight of them.
    #[test]
    fn the_non_generating_pseudo_elements_share_one_advisory() {
        let engine = Engine::compile(
            "::first-line { color: red } ::selection { color: red } ::marker { color: red }",
        );
        let hits: Vec<_> = engine
            .diagnostics
            .iter()
            .filter(|d| d.contains("pseudo-element"))
            .collect();
        assert_eq!(hits.len(), 1, "{:?}", engine.diagnostics);
    }

    // -----------------------------------------------------------------------
    // the document root
    // -----------------------------------------------------------------------

    /// The app root IS `document.documentElement` and `document.body`, so
    /// `html`, `body` and `:root` all have to reach it — that is what makes a
    /// real app's `.dark` palette and Tailwind preflight's `body { … }` apply.
    #[test]
    fn html_and_body_rules_reach_the_document_root() {
        let engine = Engine::compile(
            "html { --fg: rgb(1 2 3) } \
             body { opacity: 0.5 } \
             :root { flex-grow: 3 } \
             html.dark { --fg: rgb(9 9 9) } \
             .tab { color: var(--fg) }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let mut d = doc();
        // Make the root a real element the way the tree does: tag "root",
        // parent 0 — the two facts `MatchDom` answers `html`/`body`/`:root` on.
        d.nodes[1] = ("root", "dark", 0, vec![]);
        let states = cascade(&engine, &d, &Env::default());
        let root = &states[0];
        assert_eq!(root.base.opacity, Some(0.5), "a `body` rule matched");
        assert_eq!(root.base.flex_grow, Some(3.0), "a `:root` rule matched");
        // …and the root's custom properties inherit, with `.dark` winning.
        assert_eq!(root.vars.get("--fg").as_deref(), Some("#090909"), "the .dark override won");
        let tab = &states[1];
        assert!(tab.base.text.color.is_some(), "the palette reached a descendant");
    }

    #[test]
    fn query_all_uses_the_real_matcher() {
        let d = doc();
        let c = refs(&d);
        assert_eq!(query_all(".tab", &c).unwrap().matched, vec![1, 2]);
        assert_eq!(query_all(".tab.router-link-active", &c).unwrap().matched, vec![1]);
        assert_eq!(query_all(".nav a", &c).unwrap().matched, vec![1, 2]);
        assert_eq!(query_all("input:disabled", &c).unwrap().matched, vec![3]);
        assert_eq!(query_all("[href=\"/list\"]", &c).unwrap().matched, vec![2]);
        // A selector list is a union, in candidate order.
        assert_eq!(query_all("input, .router-link-active", &c).unwrap().matched, vec![1, 3]);
        assert!(query_all(".nope", &c).unwrap().matched.is_empty());
    }

    /// The audit is the whole reason this shares the cascade's code: an
    /// unsupported selector must SAY so, not answer "no matches".
    #[test]
    fn query_all_reports_unsupported_selectors_like_the_cascade_does() {
        let d = doc();
        let c = refs(&d);
        let err = query_all(".nav:has(.tab)", &c).unwrap_err();
        assert!(err.contains(":has()"), "{err}");
        let err = query_all("a::first-line", &c).unwrap_err();
        assert!(err.contains("pseudo-element"), "{err}");
        // …but a `::before` selector is answerable: it matches its ORIGINATING
        // element, which is what a caller asking "who has a ::before?" means.
        assert!(query_all("a::before", &c).is_ok());
        let err = query_all("a:visited", &c).unwrap_err();
        assert!(err.contains("visited"), "{err}");
        // Injection guard: a selector may not carry a declaration block.
        assert!(query_all(".a { color: red } .b", &c).unwrap_err().contains('{'));
    }

    // -----------------------------------------------------------------------
    // custom properties + var()  (compat roadmap G1)
    // -----------------------------------------------------------------------

    /// `:root` + a class subtree + a leaf, with real parent links — the exact
    /// shape shadcn's theme relies on (`:root { --x }` … `.dark { --x }` …).
    fn themed_doc() -> Doc {
        Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("root", "", 0, vec![]),      // 1 — the tree root, `:root`
                ("div", "page", 1, vec![]),   // 2
                ("button", "btn", 2, vec![]), // 3
            ],
        }
    }

    /// Runs the real restyle ORDER — parent first, child inheriting its map —
    /// which is what the tree's document-order walk does for real.
    fn cascade(engine: &Engine, doc: &Doc, env: &Env) -> Vec<ComputedStates> {
        let mut out: Vec<ComputedStates> = Vec::new();
        let mut diagnostics = Vec::new();
        for i in 1..doc.nodes.len() {
            let parent = doc.nodes[i].2;
            let parent_vars = if parent == 0 {
                Rc::new(Vars::default())
            } else {
                out[parent - 1].vars.clone()
            };
            out.push(engine.compute_node(
                DocRef { doc, i },
                None,
                &parent_vars,
                env,
                &mut diagnostics,
            ));
        }
        out
    }

    #[test]
    fn custom_properties_inherit_down_the_tree_and_substitute() {
        let engine = Engine::compile(
            ":root { --fg: rgb(10 20 30) } .btn { color: var(--fg) }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = themed_doc();
        let states = cascade(&engine, &d, &Env::default());
        // Declared on the root. (The stored text is lightningcss's own
        // serialization of the token stream — `rgb(10 20 30)` round-trips as
        // `#0a141e`. Equivalent for every consumer; the one visible difference
        // from a browser is what `getComputedStyle().getPropertyValue("--fg")`
        // reads back.)
        assert_eq!(states[0].vars.get("--fg"), Some("#0a141e"));
        // …inherited two levels down (and the Rc is SHARED, not rebuilt)…
        assert!(Rc::ptr_eq(&states[0].vars, &states[2].vars));
        // …and the button's `color` resolved through it.
        assert!(states[2].base.text.color.is_some());
    }

    /// The dark-mode mechanism: a class on an ancestor redefines the token and
    /// everything under it repaints, with no rule of its own changing.
    #[test]
    fn a_class_on_an_ancestor_can_redefine_an_inherited_token() {
        let engine = Engine::compile(
            ":root { --bg: #ffffff } .dark { --bg: #000000 } .btn { background-color: var(--bg) }",
        );
        let light = themed_doc();
        let mut dark = themed_doc();
        dark.nodes[2].1 = "page dark";

        let l = cascade(&engine, &light, &Env::default());
        let k = cascade(&engine, &dark, &Env::default());
        assert_eq!(l[2].vars.get("--bg"), Some("#fff"));
        assert_eq!(k[2].vars.get("--bg"), Some("#000"));
        assert_ne!(l[2].base.background, k[2].base.background);
        // The root itself is untouched by the class on its child.
        assert_eq!(k[0].vars.get("--bg"), Some("#fff"));
    }

    /// `@media (prefers-color-scheme: dark)` redefining tokens is the OTHER
    /// half of dark mode, and it must reach the same var map.
    #[test]
    fn media_scoped_redefinitions_reach_the_var_map() {
        let engine = Engine::compile(
            ":root { --bg: #ffffff } \
             @media (prefers-color-scheme: dark) { :root { --bg: #000000 } } \
             .btn { background-color: var(--bg) }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = themed_doc();
        let light = cascade(&engine, &d, &env_at(800.0, false, false));
        let dark = cascade(&engine, &d, &env_at(800.0, true, false));
        assert_eq!(light[0].vars.get("--bg"), Some("#fff"));
        assert_eq!(dark[0].vars.get("--bg"), Some("#000"));
        assert_ne!(light[2].base.background, dark[2].base.background);
    }

    /// Custom properties cascade like any other declaration: specificity, then
    /// document order, with `!important` on top.
    #[test]
    fn custom_property_declarations_obey_the_cascade() {
        let engine = Engine::compile(
            ".btn { --v: a } button.btn { --v: b } .btn { --v: c } \
             .page { --w: 1 } .page { --w: 2 !important } .page { --w: 3 }",
        );
        let d = themed_doc();
        let states = cascade(&engine, &d, &Env::default());
        // `button.btn` (0,1,1) beats both `.btn` rules (0,1,0).
        assert_eq!(states[2].vars.get("--v"), Some("b"));
        // `!important` beats a later normal declaration.
        assert_eq!(states[1].vars.get("--w"), Some("2"));
    }

    /// An unresolvable reference drops ONLY that declaration, and says so.
    #[test]
    fn an_unresolvable_var_drops_one_declaration_with_an_advisory() {
        let engine = Engine::compile(".btn { opacity: 0.5; color: var(--nope) }");
        let d = themed_doc();
        let mut diagnostics = Vec::new();
        let s = engine.compute_node(
            DocRef { doc: &d, i: 3 },
            None,
            &Rc::new(Vars::default()),
            &Env::default(),
            &mut diagnostics,
        );
        assert_eq!(s.base.opacity, Some(0.5), "the sibling declaration survives");
        assert_eq!(s.base.text.color, None);
        assert!(
            diagnostics.iter().any(|m| m.contains("--nope")),
            "{diagnostics:?}"
        );
    }

    /// …and a fallback makes it resolve instead.
    #[test]
    fn a_fallback_rescues_an_unset_variable() {
        let engine = Engine::compile(".btn { opacity: var(--nope, 0.25) }");
        let d = themed_doc();
        let s = cascade(&engine, &d, &Env::default());
        assert_eq!(s[2].base.opacity, Some(0.25));
    }

    #[test]
    fn a_reference_cycle_is_reported_and_invalidates_the_users() {
        let engine = Engine::compile(":root { --a: var(--b); --b: var(--a) } .btn { opacity: var(--a, 1) }");
        let d = themed_doc();
        let mut diagnostics = Vec::new();
        let mut vars = Rc::new(Vars::default());
        let mut opacity = None;
        for i in 1..d.nodes.len() {
            let s = engine.compute_node(DocRef { doc: &d, i }, None, &vars, &Env::default(), &mut diagnostics);
            vars = s.vars.clone();
            opacity = s.base.opacity;
        }
        assert!(diagnostics.iter().any(|m| m.contains("cycle")), "{diagnostics:?}");
        // The cycle members are unset, so the fallback applies.
        assert_eq!(opacity, Some(1.0));
    }

    /// A custom property declared in a `<style scoped>` rule reaches the map
    /// through the scope hash like every other declaration does. (`DocRef`
    /// reports no scopes, so the scoped rule must NOT match — the negative half
    /// of the same test.)
    #[test]
    fn scoped_rules_carry_custom_properties_through_the_normal_matcher() {
        let engine = Engine::compile(".page[data-v-abc] { --v: 1 } .page { --v: 2 }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = themed_doc();
        let s = cascade(&engine, &d, &Env::default());
        assert_eq!(s[1].vars.get("--v"), Some("2"), "the unscoped rule is the only match");
    }

    /// Tailwind's whole `--tw-*` compound stack, end to end: the shadow the
    /// `shadow-sm` utility composes must actually reach `box_shadow`.
    #[test]
    fn tailwinds_shadow_compound_renders() {
        let engine = Engine::compile(
            ".btn { --tw-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.05); \
                    box-shadow: var(--tw-ring-offset-shadow, 0 0 #0000), \
                                var(--tw-ring-shadow, 0 0 #0000), var(--tw-shadow) }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = themed_doc();
        let s = cascade(&engine, &d, &Env::default());
        let shadows = s[2].base.box_shadow.as_ref().expect("box-shadow");
        assert_eq!(shadows.len(), 3, "{shadows:?}");
        assert_eq!(shadows[2].blur_radius, gpui::px(2.0));
    }

    /// The shadcn theme shape, verbatim: `--primary: H S% L%` used through
    /// `hsl(var(--primary))`, which is what makes `bg-primary` a real colour.
    #[test]
    fn the_shadcn_token_shape_resolves_to_a_colour() {
        let engine = Engine::compile(
            ":root { --primary: 240 5.9% 10% } .btn { background-color: hsl(var(--primary)) }",
        );
        let d = themed_doc();
        let s = cascade(&engine, &d, &Env::default());
        assert!(
            matches!(s[2].base.background, Some(gpui::Fill::Color(_))),
            "{:?}",
            s[2].base.background
        );
    }

    /// A node that declares nothing shares its parent's map by pointer, so the
    /// `ComputedStates` change test stays cheap.
    #[test]
    fn a_node_that_declares_nothing_shares_the_parents_map() {
        let engine = Engine::compile(":root { --a: 1 }");
        let d = themed_doc();
        let s = cascade(&engine, &d, &Env::default());
        assert!(Rc::ptr_eq(&s[0].vars, &s[1].vars));
        assert!(Rc::ptr_eq(&s[1].vars, &s[2].vars));
    }

    /// `:hover` matches in a query the same way it matches in the cascade —
    /// "would match" — and the divergence rides back as a warning.
    #[test]
    fn query_all_warns_about_interaction_states() {
        let d = doc();
        let c = refs(&d);
        let r = query_all(".tab:hover", &c).unwrap();
        assert_eq!(r.matched, vec![1, 2]);
        assert!(
            r.warnings.iter().any(|w| w.contains("interaction state")),
            "{:?}",
            r.warnings
        );
    }

    // ---- @keyframes / animation (m12 item 3) -----------------------------

    use crate::animator::{AnimEventKind, Animator};
    use std::time::Instant;

    fn animated(css: &str) -> ComputedStates {
        Engine::compile(css).compute(box_node(), None, &Env::default())
    }

    fn opacity(r: &StyleRefinement) -> f32 {
        r.opacity.expect("opacity was not animated")
    }

    const SPIN: &str = "@keyframes spin { from { opacity: 0 } to { opacity: 1 } } \
                        .box { animation: spin 1000ms linear }";

    /// The end-to-end path: a sheet compiles `@keyframes`, the element's
    /// `animation` shorthand folds, the animator resolves the name by reference
    /// and interpolates.
    #[test]
    fn an_animation_runs_from_the_sheets_keyframes() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        assert_eq!(cs.animations.len(), 1);
        assert_eq!(&*cs.animations[0].name, "spin");
        assert_eq!(cs.animations[0].duration, Duration::from_millis(1000));

        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));

        let base = StyleRefinement::default();
        let at = |ms: u64| a.overlay(1, &base, now + Duration::from_millis(ms)).unwrap();
        assert!((opacity(&at(0)) - 0.0).abs() < 1e-6);
        assert!((opacity(&at(500)) - 0.5).abs() < 1e-3);
        assert!((opacity(&at(999)) - 1.0).abs() < 1e-2);
    }

    /// `fill-mode` is what decides whether the ends are painted at all — the
    /// difference between an element that snaps back and one that stays put.
    #[test]
    fn fill_mode_decides_what_the_ends_paint() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));
        let base = StyleRefinement::default();
        // `animation-fill-mode` defaults to `none`, so past the end nothing is
        // applied and the element's own style shows through again.
        assert!(a.overlay(1, &base, now + Duration::from_millis(2000)).is_none());

        let forwards = Engine::compile(
            "@keyframes spin { from { opacity: 0 } to { opacity: 1 } } \
             .box { animation: spin 1000ms linear forwards }",
        );
        let cs = forwards.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        a.on_computed_animations(1, &cs, now, &|name| forwards.keyframes(name));
        let held = a.overlay(1, &base, now + Duration::from_millis(2000)).unwrap();
        assert!((opacity(&held) - 1.0).abs() < 1e-6);

        // `backwards` freezes the START value during the delay; without it the
        // delay paints nothing at all.
        let backwards = Engine::compile(
            "@keyframes spin { from { opacity: 0.25 } to { opacity: 1 } } \
             .box { animation: spin 1000ms linear 500ms backwards }",
        );
        let cs = backwards.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        a.on_computed_animations(1, &cs, now, &|name| backwards.keyframes(name));
        let during_delay = a.overlay(1, &base, now + Duration::from_millis(100)).unwrap();
        assert!((opacity(&during_delay) - 0.25).abs() < 1e-6);
    }

    /// A property no keyframe names at one end falls back to the element's own
    /// computed value — the implicit 0 % / 100 % of CSS Animations §4.
    #[test]
    fn a_missing_keyframe_end_is_the_elements_own_value() {
        let engine = Engine::compile(
            "@keyframes fade { to { opacity: 1 } } .box { animation: fade 1000ms linear }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));
        // There is no `from`, so 0 % is whatever the element computes to.
        let mut base = StyleRefinement::default();
        base.opacity = Some(0.2);
        let half = a.overlay(1, &base, now + Duration::from_millis(500)).unwrap();
        assert!((opacity(&half) - 0.6).abs() < 1e-2, "{:?}", half.opacity);
    }

    /// `infinite` is what `animate-spin` and `animate-pulse` are made of.
    #[test]
    fn infinite_animations_keep_the_pump_awake() {
        let engine = Engine::compile(
            "@keyframes pulse { 0%,100% { opacity: 1 } 50% { opacity: 0.5 } } \
             .box { animation: pulse 2s cubic-bezier(0.4,0,0.6,1) infinite }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        assert!(!cs.animations[0].iterations.is_finite());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));
        assert!(a.any_animation(now + Duration::from_secs(3600)));
        let base = StyleRefinement::default();
        let mid = a.overlay(1, &base, now + Duration::from_millis(1000)).unwrap();
        assert!((opacity(&mid) - 0.5).abs() < 1e-3);
        // …and it comes back up, which is what makes a Skeleton PULSE rather
        // than fade out once.
        let back = a.overlay(1, &base, now + Duration::from_millis(1999)).unwrap();
        assert!(opacity(&back) > 0.9, "{:?}", back.opacity);
    }

    #[test]
    fn animation_start_and_end_are_reported() {
        let engine = Engine::compile(
            "@keyframes fade { from { opacity: 0 } to { opacity: 1 } } \
             .box { animation: fade 100ms linear 50ms }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));

        // Still inside the delay: nothing yet.
        a.tick(now + Duration::from_millis(10));
        assert!(a.take_events().is_empty());

        a.tick(now + Duration::from_millis(60));
        let started = a.take_events();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].kind, AnimEventKind::AnimationStart);
        assert_eq!(started[0].name, "fade");

        a.tick(now + Duration::from_millis(500));
        let ended = a.take_events();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].kind, AnimEventKind::AnimationEnd);
        // `elapsedTime` excludes the delay: 100 ms of animation, not 150.
        assert!((ended[0].elapsed - 0.1).abs() < 1e-6, "{}", ended[0].elapsed);

        // Once ended, removing the animation is NOT a cancel — the DOM fires
        // exactly one of `animationend` / `animationcancel`.
        let none = animated(".box { color: red }");
        a.on_computed_animations(1, &none, now + Duration::from_millis(600), &|_| None);
        assert!(a.take_events().is_empty());
    }

    #[test]
    fn removing_a_running_animation_cancels_it() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));
        a.tick(now);
        a.take_events();

        let none = animated(".box { color: red }");
        a.on_computed_animations(1, &none, now + Duration::from_millis(300), &|_| None);
        let events = a.take_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AnimEventKind::AnimationCancel);
        assert!((events[0].elapsed - 0.3).abs() < 1e-2);
        assert!(a.is_idle());
    }

    /// `prefers-reduced-motion` suppresses animations exactly as it suppresses
    /// transitions.
    #[test]
    fn reduced_motion_suppresses_animations() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        a.set_reduce_motion(true);
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name));
        assert!(a.is_idle());
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    /// The transition half of the same event channel (m12 item 6).
    #[test]
    fn a_finished_transition_reports_transitionend() {
        let old = animated(".box { opacity: 0; transition: opacity 200ms linear }");
        let new = animated(".box { opacity: 1; transition: opacity 200ms linear }");
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed(1, Some(&old), &new, now);

        // Mid-flight: nothing reported yet.
        a.finish_settled(now + Duration::from_millis(100));
        assert!(a.take_events().is_empty());

        a.finish_settled(now + Duration::from_millis(250));
        let events = a.take_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AnimEventKind::TransitionEnd);
        assert_eq!(events[0].name, "opacity");
        // `elapsedTime` is the DURATION, not the wall clock since the start.
        assert!((events[0].elapsed - 0.2).abs() < 1e-6);
        assert_eq!(events[0].node, 1);
    }

    /// A `@keyframes` name the sheet does not define animates nothing, which is
    /// what a browser does too — so it is an author advisory, not a divergence.
    #[test]
    fn an_unknown_animation_name_is_advised_not_faked() {
        let cs = animated(".box { animation: nope 1s linear }");
        let mut a = Animator::new();
        a.on_computed_animations(1, &cs, Instant::now(), &|_| None);
        assert!(a.is_idle());
        let diagnostics = a.take_diagnostics();
        assert!(
            diagnostics.iter().any(|d| d.contains("nope") && d.contains("@keyframes")),
            "{diagnostics:?}"
        );
    }

    /// `@keyframes` no longer produce the "dropped" advisory, and the
    /// `animation-*` longhands are no longer "not mapped".
    #[test]
    fn keyframes_and_animation_stopped_being_advisories() {
        let engine = Engine::compile(
            "@keyframes spin { to { transform: rotate(360deg) } } \
             .box { animation-name: spin; animation-duration: 1s; \
                    animation-iteration-count: infinite; animation-timing-function: linear }",
        );
        assert!(
            !engine.diagnostics.iter().any(|d| d.contains("@keyframes are dropped")
                || d.contains("not mapped (v1): Animation")),
            "{:?}",
            engine.diagnostics
        );
        assert!(engine.has_keyframes());
        let cs = engine.compute(box_node(), None, &Env::default());
        assert_eq!(cs.animations.len(), 1);
        assert!(!cs.animations[0].iterations.is_finite());
        // The longhands zip the same way the shorthand does.
        assert_eq!(cs.animations[0].duration, Duration::from_secs(1));
        assert_eq!(cs.animations[0].easing, crate::Easing::Linear);
        // …and `rotate()` reaches the keyframe as a computed value (item 4).
        let frames = engine.keyframes("spin").unwrap();
        assert_eq!(frames.steps.len(), 1);
        assert_eq!(frames.steps[0].offset, 1.0);
    }
}