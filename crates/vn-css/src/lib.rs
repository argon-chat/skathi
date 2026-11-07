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
mod has;
mod keyframes;
mod mapper;
mod path_data;
mod media;
mod prefix;
pub mod perf;
mod share;
mod transition;
mod vars;
#[cfg(test)]
mod em_tests;

use std::rc::Rc;

use gpui::{Refineable, StyleRefinement};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::rules::CssRule;
use lightningcss::selector::{PseudoClass, Selector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use parcel_selectors::context::{MatchingContext, MatchingMode, QuirksMode};
use parcel_selectors::matching::matches_selector;
use parcel_selectors::parser::Component;

pub use animator::{
    script_keyframes, strip_transitioned, AnimEvent, AnimEventKind, Animator, ScriptClock, ScriptFrame,
    ScriptKeyframeNotes, ScriptTiming,
};
pub use element::{Interaction, MatchDom};
pub use mapper::{set_composed_font_families, take_unreached_fonts};
pub use mapper::{
    accepts_declaration, parse_inline, parse_inline_full, parse_inline_style, parse_inline_with_transitions, rem_basis,
    set_font_source, set_font_metric_source, FontRelativeMetrics, set_rem_basis, set_viewport_basis, set_web_font_families, take_font_misses, uses_viewport_units,
    viewport_basis, BackgroundLayer, BackgroundSize, BackgroundUrl, PaintLayer,
    CurrentColorTargets, Decls, Extras, InlineMotion, InlineStyle, ListMarker, MaskClips, MaskCompositing,
    MaskImages, ObjectFit, OverscrollBehavior, RadiusInherit, ScrollbarWidth, SvgPaint, TextFill, TextTransform,
    TileLength, UserSelect, WhiteSpaceMode, WindowControl, WindowDrag,
    INITIAL_REM_BASIS, INITIAL_VIEWPORT_BASIS,
};
pub use media::{match_media, DisplayMode, Env};
pub use keyframes::{
    AnimationPatch, AnimationSpec, Direction as AnimationDirection, FillMode, Keyframe, Keyframes,
    PlayState,
};
#[doc(hidden)]
pub use keyframes::AnimationDeclarations;
pub use vars::Vars;
pub use transition::{
    covered, parse_easing, spec_for, Easing, StepPosition, TransitionProperty, TransitionSpec, ANIMATABLE_COUNT,
};

/// Interaction-state buckets. These are exactly the states gpui's own
/// interactivity ladder can pick live, in gpui's application order (later wins
/// — see `Interactivity::compute_style_internal`).
const STATE_FOCUS: u8 = 1;
const STATE_FOCUS_VISIBLE: u8 = 2;
const STATE_HOVER: u8 = 4;
const STATE_ACTIVE: u8 = 8;
/// `:focus-within` — gpui's `.in_focus()` rung, which is focus anywhere in the
/// element's own focus subtree. Weakest of the focus states, as in gpui's
/// application order.
const STATE_FOCUS_WITHIN: u8 = 16;

/// How many rungs the ladder has above the base state — the width of the
/// per-rung custom-property maps in [`Engine::compute_node`].
const STATE_RUNGS: usize = 5;

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
        s if s & STATE_FOCUS != 0 => Some(3),
        _ => Some(4),
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
    /// `:focus-within` — gpui's `.in_focus()` rung: focus anywhere inside this
    /// element's focus subtree, which is what the CSS pseudo-class means. Like
    /// `:focus`, an element that has one MUST be given a tracked focus handle
    /// by the renderer for gpui to have anything to key it on.
    pub focus_within: Option<Rc<StyleRefinement>>,
    /// `.group:hover .this` — what this element looks like while an ANCESTOR is
    /// hovered, and which ancestor that is (its [`MatchDom::key`]).
    ///
    /// The renderer folds the rung in while the ancestor is on the hover path
    /// (`vn_tree::pointer::resolve_hover`), and a flip on the ancestor renders
    /// this element again without restyling anything — the m11 perf
    /// invariants are untouched.
    pub group_hover: Option<(u64, Rc<StyleRefinement>)>,
    /// `.group:active .this`. gpui's own group mechanism: the ancestor is given
    /// `.group("vn-g<key>")` and this element `.group_active("vn-g<key>", …)`,
    /// which gpui resolves from the group's hitbox at paint time.
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
    /// `::selection` — the highlight the text-input widget paints behind a
    /// selected range. Only `background-color` reaches the widget: gpui's
    /// selection is a quad behind the glyphs, and the glyphs keep the colour
    /// the field draws them in.
    pub selection: Option<Rc<StyleRefinement>>,
    /// `::file-selector-button` — the "Choose File" button an
    /// `<input type=file>` paints (vn-tree `build_file_input`). A box of its
    /// own, so box properties apply; base state only, like `::placeholder`.
    /// Tailwind's `file:` variants and shadcn-vue's `<Input type="file">`
    /// write exactly these rules.
    pub file_selector_button: Option<Rc<StyleRefinement>>,
    /// The element's resolved custom properties: what it inherited from its
    /// parent, overridden by what it declared. Children read this one, which is
    /// why the restyle pass must walk in document order.
    ///
    /// Shared with the parent by `Rc` when the element declares none of its own
    /// — so the `PartialEq` above stays O(1) for almost every node.
    pub vars: Rc<Vars>,
    /// The element's effective `color`: its own declaration, or the value
    /// threaded down from its parent.
    ///
    /// vn-css folds no inherited text property — gpui's `TextStyleRefinement`
    /// cascades at paint time and "take the parent's value" is what not writing
    /// the field means — so this exists for exactly one job: `currentColor`,
    /// which is a computed value the cascade has to produce even though nothing
    /// else needs the fold. `vn_tree::tree::restyle_with` threads it down the
    /// document-order walk, the same way it threads `pointer-events`.
    pub color: Option<gpui::Hsla>,
    /// The element's computed `font-size` in px, base state: its own
    /// declaration resolved against its parent's, or the parent's. Every `em`
    /// in the element's declarations resolved against it, and its children's
    /// `font-size: 1.2em` resolve against it ([`Engine::compute_node_at`]).
    /// Threaded down the walk beside [`Self::color`].
    pub font_size: f32,
    /// The inherited face resolved before `ch`/`ex`, including features and weight.
    pub computed_font: gpui::Font,
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
    /// `::selection`. Also not a generated box — it styles the highlight the
    /// native text-input widget paints behind a selected range, which is the
    /// one selection that exists here (gpui paints ordinary text; only the
    /// field has a selection model). The renderer reads `background` off it.
    Selection,
    /// `::file-selector-button`, the button inside `<input type=file>`. Not
    /// a generated box either: the renderer builds the button and refines it
    /// with this.
    FileSelectorButton,
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

/// The corner radii an element hands to a child's `border-radius: inherit`:
/// its computed ones, which are 0 where it declared none. `Copy`, so it rides
/// down `vn_tree`'s restyle walk beside the custom-property map.
///
/// A percentage stays a percentage, as CSS's computed value does, so the child
/// resolves it against its OWN box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InheritedRadii {
    /// top-left, top-right, bottom-right, bottom-left, as gpui orders corners.
    lengths: [Option<gpui::AbsoluteLength>; 4],
    relative: [Option<Option<gpui::VnCornerRadius>>; 4],
}

impl InheritedRadii {
    /// What an element whose computed base style is `style` hands down.
    pub fn of(style: &StyleRefinement) -> Self {
        let (c, v) = (&style.corner_radii, &style.vn_corner_radii);
        Self {
            lengths: [c.top_left, c.top_right, c.bottom_right, c.bottom_left],
            relative: [v.top_left, v.top_right, v.bottom_right, v.bottom_left],
        }
    }

    fn fill(&self, style: &mut StyleRefinement, corners: u8) {
        let (c, v) = (&mut style.corner_radii, &mut style.vn_corner_radii);
        let slots = [
            (&mut c.top_left, &mut v.top_left),
            (&mut c.top_right, &mut v.top_right),
            (&mut c.bottom_right, &mut v.bottom_right),
            (&mut c.bottom_left, &mut v.bottom_left),
        ];
        for (i, (length, relative)) in slots.into_iter().enumerate() {
            if corners & (1 << i) != 0 {
                *length = Some(self.lengths[i].unwrap_or(gpui::px(0.).into()));
                *relative = Some(self.relative[i].flatten());
            }
        }
    }
}

/// Undoes what a :hover/:focus/:active (or `group-`) rule did to
/// [`Extras::radius_inherit`], which is shared by every rung while the inherited
/// radius is filled into the BASE style only: `.x:hover { border-radius: 8px }`
/// must not cancel `.x { border-radius: inherit }` outside the hover, nor a
/// hover's `inherit` apply where the pointer is not. The second is advised.
fn keep_base_radius_inherit(extras: &mut Extras, before: RadiusInherit, diagnostics: &mut Vec<String>) {
    if extras.radius_inherit.inherit & !before.inherit != 0 {
        mapper::once(
            diagnostics,
            "`border-radius: inherit` in a :hover/:focus/:active (or group-) rule is not \
             applied (v1): the parent's radius is inherited into the element's base style \
             only. Declare the inherit without the state"
                .to_string(),
        );
    }
    extras.radius_inherit = before;
}

/// An element's SVG presentation properties as the base rules left them.
type SvgBase = (Option<SvgPaint>, Option<SvgPaint>, Option<Rc<Vec<(&'static str, Rc<str>)>>>);

fn svg_base(extras: &Extras) -> SvgBase {
    (extras.fill, extras.stroke, extras.svg_presentation.clone())
}

/// [`keep_base_radius_inherit`] for `fill`, `stroke` and the other SVG
/// presentation properties: `Extras` have no state ladder, so a state rule's
/// value would be drawn ALL the time (`.btn:hover svg { fill: red }` a red
/// icon at rest). The base value is kept, and the drop is said.
fn keep_base_svg_paint(extras: &mut Extras, before: SvgBase, diagnostics: &mut Vec<String>) {
    let (fill, stroke, presentation) = before;
    if extras.fill != fill || extras.stroke != stroke || extras.svg_presentation != presentation {
        mapper::once(
            diagnostics,
            "`fill`, `stroke` and the other SVG presentation properties in a \
             :hover/:focus/:active (or group-) rule are not applied (v1): they are drawn into \
             the <svg>'s rasterised document from the base state only. Toggle a class instead"
                .to_string(),
        );
    }
    extras.fill = fill;
    extras.stroke = stroke;
    extras.svg_presentation = presentation;
}

/// Fills in `border-radius: inherit` ([`mapper::RadiusInherit`]) once the
/// parent's radii are known: the element's base style from `parent`, then its
/// `::before`/`::after` boxes from the element itself. Answers what the element
/// hands its own children.
///
/// The renderer's restyle walk calls this, parent first, so a chain of
/// `inherit`s (`.zone` from `.card`, `.body` from `.zone`) resolves top-down.
/// A transition or animation on the parent's radius is not followed: the child
/// takes the parent's cascaded value, not the frame being drawn.
pub fn resolve_inherited_radii(states: &mut ComputedStates, parent: InheritedRadii) -> InheritedRadii {
    let corners = states.extras.radius_inherit.inherit;
    if corners != 0 {
        parent.fill(Rc::make_mut(&mut states.base), corners);
    }
    let own = InheritedRadii::of(&states.base);
    for slot in [&mut states.before, &mut states.after].into_iter().flatten() {
        let corners = slot.extras.radius_inherit.inherit;
        if corners != 0 {
            own.fill(Rc::make_mut(&mut Rc::make_mut(slot).style), corners);
        }
    }
    own
}

impl ComputedStates {
    /// True when the renderer has to give this node a focus handle for gpui to
    /// have anything to hang the focus rungs on.
    pub fn needs_focus_handle(&self) -> bool {
        self.focus.is_some() || self.focus_visible.is_some() || self.focus_within.is_some()
    }

    /// True when a pointer flip on this element (or on the `.group` ancestor it
    /// watches) has to be routed through the animator instead of gpui's
    /// paint-time ladder — because the author declared a `transition` and at
    /// least one hover rung to run it on.
    pub fn hover_transitions(&self) -> bool {
        !self.transitions.is_empty() && (self.hover.is_some() || self.group_hover.is_some())
    }

    /// The refinement this element's ladder produces with the pointer over it
    /// (`own`) and/or over the `.group` ancestor a `group-hover:` rule watches
    /// (`group`) — base first, then the rungs in gpui's own application order
    /// (`compute_style_internal`: group-hover, then hover). `None` when neither
    /// is on, which is the plain base state.
    ///
    /// This is what [`Animator::on_state`] transitions towards, so it has to
    /// fold exactly the way gpui would: a property both rungs set must end on
    /// the same value whether the animator or the ladder applied it.
    pub fn hovered(&self, own: bool, group: bool) -> Option<StyleRefinement> {
        use gpui::Refineable as _;
        if !own && !group {
            return None;
        }
        let mut out = (*self.base).clone();
        if group {
            if let Some((_, style)) = &self.group_hover {
                out.refine(style);
            }
        }
        if own {
            if let Some(style) = &self.hover {
                out.refine(style);
            }
        }
        Some(out)
    }
}

struct RuleEntry {
    /// Owned (`Selector::into_owned`): the parsed sheet it came from is gone
    /// once the engine is built (see [`parse_author_sheet`]).
    selector: Selector<'static>,
    normal: Option<Rc<StyleRefinement>>,
    important: Option<Rc<StyleRefinement>>,
    normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    important_transitions: Option<Rc<Vec<TransitionSpec>>>,
    normal_transition_declarations: Option<Rc<transition::Collector>>,
    important_transition_declarations: Option<Rc<transition::Collector>>,
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
    /// Font-relative declarations: applied again per element under its font
    /// size ([`mapper::FontDecls`]).
    normal_font: Option<mapper::FontDecls>,
    important_font: Option<mapper::FontDecls>,
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
    /// The advisories this rule's declarations produced, emitted the first time
    /// the rule actually MATCHES an element rather than when the sheet was
    /// compiled — see [`mapper::Converted::diagnostics`].
    diagnostics: Option<Rc<Vec<String>>>,
    /// The rule's declarations as the AUTHOR wrote them, `(property, value,
    /// important)` in source order — captured only when
    /// [`set_capture_rule_text`] was on at compile time, because a devtools
    /// styles panel is the only thing that wants them and a Tailwind sheet is
    /// tens of thousands of strings. `None` = capture was off.
    decls: Option<Rc<Vec<(String, String, bool)>>>,
}

impl RuleEntry {
    /// Replaces what the rule's declarations converted to, and nothing else
    /// ([`Engine::rebase_viewport`]). Every field `collect` fills from a
    /// `Converted` belongs here, except the advisories.
    fn set_declarations(&mut self, c: &mapper::Converted) {
        self.normal = c.normal.clone();
        self.important = c.important.clone();
        self.normal_transitions = c.normal_transitions.clone();
        self.important_transitions = c.important_transitions.clone();
        self.normal_transition_declarations = c.normal_transition_declarations.clone();
        self.important_transition_declarations = c.important_transition_declarations.clone();
        self.normal_animations = c.normal_animations.clone();
        self.important_animations = c.important_animations.clone();
        self.normal_animation_patch = c.normal_animation_patch.clone();
        self.important_animation_patch = c.important_animation_patch.clone();
        self.normal_extras = c.normal_extras.clone();
        self.important_extras = c.important_extras.clone();
        self.normal_custom = c.normal_custom.clone();
        self.important_custom = c.important_custom.clone();
        self.normal_dynamic = c.normal_dynamic.clone();
        self.important_dynamic = c.important_dynamic.clone();
        self.normal_font = c.normal_font.clone();
        self.important_font = c.important_font.clone();
    }

    /// What one half folds: the compiled refinement, the font-relative
    /// declarations and the `var()`-composed ones.
    fn half(&self, important: bool) -> Half<'_> {
        if important {
            Half {
                refinement: self.important.as_ref(),
                font: self.important_font.as_ref(),
                dynamic: self.important_dynamic.as_deref().map(Vec::as_slice),
            }
        } else {
            Half {
                refinement: self.normal.as_ref(),
                font: self.normal_font.as_ref(),
                dynamic: self.normal_dynamic.as_deref().map(Vec::as_slice),
            }
        }
    }
}

fn refine_font(parent: &gpui::Font, refinement: &gpui::TextStyleRefinement) -> gpui::Font {
    gpui::Font {
        family: refinement.font_family.clone().unwrap_or_else(|| parent.family.clone()),
        features: refinement.font_features.clone().unwrap_or_else(|| parent.features.clone()),
        fallbacks: refinement.font_fallbacks.clone().or_else(|| parent.fallbacks.clone()),
        weight: refinement.font_weight.unwrap_or(parent.weight),
        style: refinement.font_style.unwrap_or(parent.style),
    }
}

/// One half of a matched rule (or an inline style), as the fold applies it.
#[derive(Clone, Copy)]
struct Half<'a> {
    refinement: Option<&'a Rc<StyleRefinement>>,
    font: Option<&'a mapper::FontDecls>,
    dynamic: Option<&'a [(String, String)]>,
}

impl<'a> Half<'a> {
    fn of_inline(inline: &'a InlineStyle) -> Self {
        Half {
            refinement: inline.refinement.as_ref(),
            font: inline.font.as_ref(),
            dynamic: (!inline.dynamic.is_empty()).then_some(inline.dynamic.as_slice()),
        }
    }

    fn is_empty(&self) -> bool {
        self.refinement.is_none() && self.font.is_none() && self.dynamic.is_none()
    }

    /// Whether anything in it may depend on the font size.
    fn font_relative(&self) -> bool {
        self.font.is_some() || self.dynamic.is_some()
    }

    /// Folds the half into `bucket`: the refinement, then the font-relative
    /// declarations with `1em` = `basis` (`parent` for `font-size`), then the
    /// `var()`-composed ones under the same rule.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &self,
        vars: &Vars,
        env: &Env,
        basis: f32,
        parent: f32,
        font: &gpui::Font,
        parent_font: &gpui::Font,
        bucket: &mut StyleRefinement,
        extras: &mut Extras,
        diagnostics: &mut Vec<String>,
    ) {
        if let Some(r) = self.refinement {
            bucket.refine(r);
        }
        if let Some(list) = self.font {
            for p in list.iter() {
                let at = if mapper::sets_font_size(p) { parent } else { basis };
                let face = if mapper::sets_font_size(p) { parent_font } else { font };
                mapper::with_font_face(face, || mapper::apply_at(p, at, bucket, extras, diagnostics));
            }
        }
        if let Some(list) = self.dynamic {
            for (name, raw) in list {
                let at = if name == "font-size" { parent } else { basis };
                let face = if name == "font-size" { parent_font } else { font };
                mapper::with_font_face(face, || mapper::with_font_basis(at, || {
                    mapper::apply_dynamic(name, raw, vars, env, bucket, extras, diagnostics)
                }));
            }
        }
    }

    fn font_descriptor(&self, vars: &Vars, env: &Env, into: &mut gpui::TextStyleRefinement) {
        if let Some(refinement) = self.refinement { into.refine(&refinement.text); }
        let mut scratch = StyleRefinement::default();
        let mut extras = Extras::default();
        let mut quiet = Vec::new();
        if let Some(properties) = self.font {
            for property in properties.iter().filter(|property| matches!(property,
                lightningcss::properties::Property::FontFamily(_)
                | lightningcss::properties::Property::FontWeight(_)
                | lightningcss::properties::Property::FontStyle(_))) {
                mapper::apply_at(property, mapper::rem_basis(), &mut scratch, &mut extras, &mut quiet);
            }
        }
        if let Some(properties) = self.dynamic {
            for (name, raw) in properties.iter().filter(|(name, _)| matches!(name.as_str(), "font-family" | "font-weight" | "font-style" | "font-feature-settings" | "font-variant-numeric")) {
                mapper::apply_dynamic(name, raw, vars, env, &mut scratch, &mut extras, &mut quiet);
            }
        }
        into.refine(&scratch.text);
    }

    /// The `font-size` this half leaves, if it sets one, against a parent of
    /// `parent` px.
    fn font_size(&self, vars: &Vars, env: &Env, parent: f32) -> Option<f32> {
        let mut size = self.refinement.and_then(|r| r.text.font_size).map(mapper::font_size_px);
        let sets = self.font.is_some_and(|l| l.iter().any(mapper::sets_font_size))
            || self.dynamic.is_some_and(|l| l.iter().any(|(n, _)| n == "font-size"));
        if sets {
            let mut scratch = StyleRefinement::default();
            let mut extras = Extras::default();
            let mut quiet = Vec::new();
            if let Some(list) = self.font {
                for p in list.iter().filter(|p| mapper::sets_font_size(p)) {
                    mapper::apply_at(p, parent, &mut scratch, &mut extras, &mut quiet);
                }
            }
            if let Some(list) = self.dynamic {
                for (name, raw) in list.iter().filter(|(n, _)| n == "font-size") {
                    mapper::with_font_basis(parent, || {
                        mapper::apply_dynamic(name, raw, vars, env, &mut scratch, &mut extras, &mut quiet)
                    });
                }
            }
            if let Some(l) = scratch.text.font_size {
                size = Some(mapper::font_size_px(l));
            }
        }
        size
    }
}

/// The few user-agent defaults vue-native supplies, collected ahead of every
/// author sheet.
///
/// A browser's UA sheet is its own cascade origin, which ANY author
/// declaration beats, whatever its specificity. Wrapping each selector in
/// `:where()` (specificity 0) and collecting it first (the earliest order) is
/// the same thing for normal declarations: every author rule that sets the
/// property is at least as specific and comes later. vn-css models no
/// `@layer`, so no author rule can sort below these.
///
/// - `[hidden]` is `display: none` (HTML's rendering section). Without it a
///   `hidden` element stayed visible unless the app's own CSS hid it.
///   `hidden="until-found"` is hidden the same way: a browser keeps its box and
///   hides only its content (`content-visibility: hidden`) until find-in-page
///   or a fragment reveals it, and neither exists here (crates/vn-css/README.md
///   §5 item 11).
pub(crate) const UA_SHEET: &str = ":where([hidden]) { display: none }";

/// A compiled stylesheet: rules with matchable selectors, pre-converted
/// declaration blocks, specificity and state classification.
///
/// It owns everything it holds: the source text and lightningcss's parse are
/// gone once [`Engine::compile_sheets`] returns ([`parse_author_sheet`]).
pub struct Engine {
    rules: Vec<RuleEntry>,
    /// The rule index — see [`RuleIndex`].
    index: RuleIndex,
    /// One entry per distinct `@media` prelude in the sheet; rules point at
    /// theirs by index. Evaluated against the [`Env`] on every cascade.
    conditions: Vec<media::Cond>,
    /// The prelude TEXT of each entry in `conditions`, `and`-ed the same way
    /// nesting `and`s the conditions themselves. Devtools reports which
    /// `@media` block a matched rule came from; the parsed condition cannot be
    /// printed back and a reader needs the words, not the tree.
    condition_text: Vec<String>,
    /// Any `+` / `~` anywhere in the sheet — see
    /// [`Engine::uses_sibling_combinators`].
    sibling_combinators: bool,
    /// Any `.group:hover .child` anywhere in the sheet — see
    /// [`Engine::uses_group_states`].
    group_states: bool,
    /// `@keyframes` blocks by name. Document-scoped rather than per element:
    /// an element's `animation-name` is a REFERENCE into this map, resolved by
    /// the animator when a track starts.
    ///
    /// A LIST per name, in document order, because a `@keyframes` block can sit
    /// inside an `@media` one — and which of them is "the" block is then a
    /// question with a different answer per WINDOW, not per document
    /// (docs/MULTIWINDOW.md §6). [`Engine::keyframes`] picks the last entry
    /// whose condition holds in the environment it is asked about; with no
    /// `@media` around any of them that is exactly CSS's "a later block with
    /// the same name wins", at the cost of one `Vec` of length 1.
    keyframes: std::collections::HashMap<Box<str>, Vec<KeyframesBlock>>,
    /// The declaration blocks that resolved a viewport unit, kept with the
    /// rules they fill, so a resize converts just these again
    /// ([`Engine::rebase_viewport`]).
    viewport_blocks: Vec<ViewportBlock>,
    /// A block resolved a viewport unit and was not kept: one inside
    /// `@keyframes`, or one that did not print. Such a sheet is compiled again
    /// on a resize.
    viewport_unkept: bool,
    /// Which compile, and which viewport rebase of it, this is: the
    /// matched-properties cache keys on it, since its rule indices mean nothing
    /// across sheets and a rebase rewrites declarations in place ([`share`]).
    generation: u64,
    /// The matched-properties cache is on (`VN_CSS_NO_SHARE` turns it off).
    share: bool,
    pub diagnostics: Vec<String>,
}

/// A new [`Engine::generation`].
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// One style rule's declarations that resolved a viewport unit (`85vh`,
/// `calc(100vw - 2rem)`), and the [`Engine::rules`] they fill, one per
/// selector in its list.
///
/// Kept as lightningcss PRINTS them, and parsed again on a resize. An owned
/// block (`DeclarationBlock::into_owned`) would skip that parse, but its code
/// added 248 KB to the exe; the print and the parse are linked already, and a
/// few dozen short blocks parse in well under a millisecond.
struct ViewportBlock {
    css: Box<str>,
    rules: std::ops::Range<u32>,
}

/// One `@keyframes` block, and the `@media` condition it was declared under.
///
/// `condition` indexes [`Engine::conditions`], exactly as a style rule's does,
/// so a media-scoped block is "not there" in a window the query does not hold
/// in — which is what CSS means by it, and what makes the answer per window.
struct KeyframesBlock {
    condition: Option<u32>,
    block: Rc<keyframes::Keyframes>,
}

/// Which rules an element could possibly match, keyed off the RIGHTMOST
/// compound of each selector.
///
/// A compound is a conjunction: `.card .title` can only match an element that
/// carries `title`, and `button.ghost` only one whose tag is `button`. So a
/// rule can be filed under one key its subject must have, and an element only
/// ever has to run the selectors filed under the keys it actually carries plus
/// the ones that could not be keyed at all (`*`, `[data-x]`, `.a *`).
/// [`bucket_of`] also keys a pseudo-element rule by its originating element,
/// `:root` by the root, and a subject that names nothing by its PARENT
/// (`.space-x-2 > :not([hidden]) ~ :not([hidden])`).
///
/// This is the browsers' oldest cascade optimization and it is what turns
/// `elements × rules-in-the-sheet` into `elements × a-handful`: on the shadcn
/// app a 25 KB Tailwind sheet went from ~1.5 M selector runs per restyle to
/// ~28 K (research/reports/m11/perf-restyle-rebuild.md). On the Argon client's
/// 12 K rules the unkeyed rules alone were ~290 runs per element, 95% of all
/// of them, until the pseudo-element, `:root` and parent keys took them from
/// 284 rules to 100 (research/reports/m26/fixes-lag-1.md).
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
    /// Rules whose subject is `:root` (and names no id or class): offered to
    /// the document root alone.
    root: Vec<u32>,
    /// Rules whose subject names nothing, filed by what its PARENT must carry:
    /// `.space-x-2 > :not([hidden]) ~ :not([hidden])`, `.card > *`, `ul > li`
    /// is the tag bucket's. See [`bucket_of`].
    by_parent_id: std::collections::HashMap<Box<str>, Vec<u32>>,
    by_parent_class: std::collections::HashMap<Box<str>, Vec<u32>>,
    by_parent_tag: std::collections::HashMap<Box<str>, Vec<u32>>,
    /// Rules no key could file — every element has to run these.
    any: Vec<u32>,
    enabled: bool,
}

/// Which bucket offered a candidate, as [`perf::Split::by_bucket`] counts it.
const BUCKET_ANY: usize = 0;
const BUCKET_TAG: usize = 1;
const BUCKET_CLASS: usize = 2;
const BUCKET_ID: usize = 3;
const BUCKET_ROOT: usize = 4;
const BUCKET_PARENT: usize = 5;

/// One compound's strongest key.
enum Key {
    Id(Box<str>),
    Class(Box<str>),
    Tag(Box<str>),
}

/// Where a selector is filed: under a key its subject must carry, under one
/// its parent must carry, or nowhere.
enum Bucket {
    Subject(Key),
    Root,
    Parent(Key),
    Any,
}

/// The strongest key in the compound `iter` is at (id, then class, then tag),
/// and whether it holds `:root`. Consumes the compound, so `next_sequence`
/// answers the combinator to its left afterwards. (A macro because the
/// iterator's type names lightningcss's private selector `Impl`.)
///
/// Only TOP-LEVEL components count. A class inside `:not()` / `:is()` /
/// `:where()` guarantees nothing about the element (it may even be a
/// negation).
macro_rules! compound_key {
    ($iter:expr) => {{
        let mut id: Option<Box<str>> = None;
        let mut class: Option<Box<str>> = None;
        let mut tag: Option<Box<str>> = None;
        let mut root = false;
        for component in &mut $iter {
            match component {
                Component::ID(i) if id.is_none() => id = Some(i.0.as_ref().into()),
                Component::Class(c) if class.is_none() => class = Some(c.0.as_ref().into()),
                // `is_html_element_in_html_document()` is false for us, so the
                // matcher compares against `name` as written, never `lower_name`.
                Component::LocalName(ln) if tag.is_none() => tag = Some(ln.name.0.as_ref().into()),
                Component::Root => root = true,
                _ => {}
            }
        }
        let key = id.map(Key::Id).or(class.map(Key::Class)).or(tag.map(Key::Tag));
        (key, root)
    }};
}

/// Files a selector by the strongest key it guarantees the element carries,
/// read off its SUBJECT (the rightmost compound):
///
/// * the subject's own id, class or tag;
/// * for a pseudo-element rule (`.a::before`, `.b::placeholder`), the
///   ORIGINATING element's compound: the selectors crate keeps the
///   pseudo-element as a compound of its own, behind a `PseudoElement`
///   combinator, and only the element left of it can carry a key;
/// * `:root` — offered to the document root alone;
/// * a subject that names nothing (`*`, `:not([hidden])`, `[data-x]`) whose
///   parent does: `.p > *`, and across sibling combinators, which share the
///   parent, `.space-x-2 > :not([hidden]) ~ :not([hidden])`. Tailwind's
///   `space-*`, `divide-*` and `*:` variants are that shape, as are many
///   component sheets' `.list > li + li`.
///
/// Anything else (a key only on an ANCESTOR, `.a *`; none at all, `*`) goes
/// to [`RuleIndex::any`] and is tested against every element — conservative,
/// never wrong. Every key is one the matcher itself requires, so filing can
/// only drop candidates that could not match.
fn bucket_of(sel: &Selector<'_>) -> Bucket {
    use parcel_selectors::parser::Combinator;
    let mut iter = sel.iter();
    let (mut key, mut root) = compound_key!(iter);
    let mut next = iter.next_sequence();
    if next == Some(Combinator::PseudoElement) {
        // The compound read was the pseudo-element's; the subject is next.
        (key, root) = compound_key!(iter);
        next = iter.next_sequence();
    }
    match (key, root) {
        (Some(Key::Tag(_)), true) => return Bucket::Root,
        (Some(key), _) => return Bucket::Subject(key),
        (None, true) => return Bucket::Root,
        (None, false) => {}
    }
    // The subject names nothing: its siblings share its parent, so walk over
    // them to the first parent compound.
    loop {
        match next {
            Some(Combinator::NextSibling | Combinator::LaterSibling) => {
                let _ = compound_key!(iter);
                next = iter.next_sequence();
            }
            Some(Combinator::Child) => {
                return match compound_key!(iter) {
                    (Some(key), _) => Bucket::Parent(key),
                    (None, _) => Bucket::Any,
                };
            }
            _ => return Bucket::Any,
        }
    }
}

type KeyMap = std::collections::HashMap<Box<str>, Vec<u32>>;

/// Hands `f` the rules filed under `node`'s tag, classes and id in `maps`
/// (`[by tag, by class, by id]`), each with its `buckets` entry.
fn offer_keyed<D: MatchDom>(node: &D, maps: [&KeyMap; 3], buckets: [usize; 3], f: &mut dyn FnMut(u32, usize)) {
    let [by_tag, by_class, by_id] = maps;
    if !by_tag.is_empty() {
        if let Some(v) = by_tag.get(node.tag_name()) {
            for &i in v {
                f(i, buckets[0]);
            }
        }
        // The document root answers to `html` and `body` as well as to its own
        // tag (`MatchDom::has_tag`), so the index has to offer it those rules
        // too or a `body { … }` rule would never be a candidate and could never
        // match. The root's own tag is never one of the two, so this cannot
        // hand the same rule back twice.
        if node.is_root() {
            for name in ["html", "body"] {
                if let Some(v) = by_tag.get(name) {
                    for &i in v {
                        f(i, buckets[0]);
                    }
                }
            }
        }
    }
    if !by_class.is_empty() {
        node.each_class(&mut |c| {
            if let Some(v) = by_class.get(c) {
                for &i in v {
                    f(i, buckets[1]);
                }
            }
        });
    }
    if !by_id.is_empty() {
        if let Some(id) = node.attr("id") {
            if let Some(v) = by_id.get(id) {
                for &i in v {
                    f(i, buckets[2]);
                }
            }
        }
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
            Bucket::Subject(Key::Id(k)) => self.by_id.entry(k).or_default().push(rule),
            Bucket::Subject(Key::Class(k)) => self.by_class.entry(k).or_default().push(rule),
            Bucket::Subject(Key::Tag(k)) => self.by_tag.entry(k).or_default().push(rule),
            Bucket::Root => self.root.push(rule),
            Bucket::Parent(Key::Id(k)) => self.by_parent_id.entry(k).or_default().push(rule),
            Bucket::Parent(Key::Class(k)) => self.by_parent_class.entry(k).or_default().push(rule),
            Bucket::Parent(Key::Tag(k)) => self.by_parent_tag.entry(k).or_default().push(rule),
            Bucket::Any => self.any.push(rule),
        }
    }

    fn shrink(&mut self) {
        for map in [
            &mut self.by_id,
            &mut self.by_class,
            &mut self.by_tag,
            &mut self.by_parent_id,
            &mut self.by_parent_class,
            &mut self.by_parent_tag,
        ] {
            map.values_mut().for_each(Vec::shrink_to_fit);
            map.shrink_to_fit();
        }
        self.root.shrink_to_fit();
        self.any.shrink_to_fit();
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

/// One author sheet as parsed: the sheet itself (or why it failed) and the
/// advisories its error recovery produced. See [`parse_author_sheet`].
struct ParsedSheet<'i> {
    sheet: Option<StyleSheet<'i>>,
    diagnostics: Vec<String>,
}

thread_local! {
    /// The UA defaults, parsed once (per thread — the cascade runs on one).
    static UA_PARSED: std::cell::OnceCell<Option<&'static StyleSheet<'static>>> =
        const { std::cell::OnceCell::new() };
    /// [`reparse_selector`]'s answers, by the rewritten selector's text.
    static REPARSED: std::cell::RefCell<std::collections::HashMap<String, Option<&'static Selector<'static>>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Parses one author sheet, for the compile in progress only.
///
/// What a compile keeps of a sheet is what the engine holds: the rules own
/// their selectors (`Selector::into_owned`) and their converted declarations,
/// so the source text and lightningcss's AST go when the compile returns.
///
/// History, because each step was a measured cost:
/// - Until m26 every compile LEAKED its parse (the rules borrowed `&'static`
///   selectors out of it): ~21.5 MB for the Argon client's 1.27 MB sheet, on
///   every recompile the viewport (`vh`/`vw`), the root font-size (`rem`) or
///   the web fonts asked for — 159 MB after boot, +21.5 MB per resize step
///   (research/reports/m26/perf-mem-1.md §3).
/// - Then a parse cache kept ONE parse per distinct text and recompiles reused
///   it: flat, but 22.6 MB held for the life of the process
///   (perf-mem-2.md §1).
/// - Now nothing is kept, and a recompile parses again: ~30 ms more per
///   recompile on that sheet (perf-mem-3.md §2). A resize, the one recompile
///   that came in bursts, is no longer one ([`Engine::rebase_viewport`]); the
///   rem basis and web fonts still recompile, a few times per boot.
fn parse_author_sheet(src: &str) -> ParsedSheet<'_> {
    // CSS's own error handling (CSS Syntax §2.2: an invalid rule or
    // declaration is dropped and parsing resumes), which every browser
    // applies. Without it, ONE stray `}` anywhere — the Argon client ships
    // one after a `@keyframes` — failed the whole sheet, and the app
    // painted unstyled. A sheet with no error parses exactly as before.
    let warnings = std::sync::Arc::new(std::sync::RwLock::new(Vec::new()));
    let options = ParserOptions {
        error_recovery: true,
        warnings: Some(warnings.clone()),
        ..ParserOptions::default()
    };
    let mut diagnostics = Vec::new();
    let sheet = match StyleSheet::parse(src, options) {
        Ok(s) => Some(s),
        Err(e) => {
            diagnostics.push(format!("stylesheet parse error: {e}"));
            None
        }
    };
    if sheet.is_some() {
        if let Ok(skipped) = warnings.read() {
            const SHOWN: usize = 20;
            for e in skipped.iter().take(SHOWN) {
                diagnostics.push(format!("stylesheet: {e} — skipped, as a browser skips it"));
            }
            if skipped.len() > SHOWN {
                diagnostics.push(format!(
                    "stylesheet: {} more invalid rules or declarations skipped",
                    skipped.len() - SHOWN
                ));
            }
        }
    }
    ParsedSheet { sheet, diagnostics }
}

impl Engine {
    pub fn compile(css: &str) -> Engine {
        Engine::compile_sheets(&[css])
    }

    /// Compiles several author sheets as ONE cascade, in order: a later
    /// sheet's rules come later, so at equal specificity they win.
    ///
    /// Each sheet is parsed on its own, the way a browser parses each
    /// `<style>`: a stray brace in one cannot swallow the next. The parses
    /// last only as long as this call ([`parse_author_sheet`]).
    pub fn compile_sheets(sheets: &[&str]) -> Engine {
        let mut engine = Engine {
            rules: Vec::new(),
            index: RuleIndex { enabled: rule_index_enabled(), ..RuleIndex::default() },
            conditions: Vec::new(),
            condition_text: Vec::new(),
            sibling_combinators: false,
            group_states: false,
            keyframes: std::collections::HashMap::new(),
            viewport_blocks: Vec::new(),
            viewport_unkept: false,
            generation: next_generation(),
            share: share::enabled(),
            diagnostics: Vec::new(),
        };
        // The flag describes THIS sheet: a resize only recompiles when the sheet
        // currently installed actually resolves a viewport unit.
        mapper::reset_viewport_units_seen();
        // UA defaults first, so they hold even when the author's sheet fails.
        let mut order = 0u32;
        engine.collect_ua(&mut order);
        for css in sheets {
            let parsed = parse_author_sheet(css);
            engine.diagnostics.extend(parsed.diagnostics);
            if let Some(sheet) = &parsed.sheet {
                engine.collect(&sheet.rules.0, None, &mut order);
            }
        }
        engine.shrink();
        engine
    }

    /// Gives back what the vectors grew past their length. The rule table is
    /// the big one: `Vec`'s doubling left 16,384 slots for the Argon client's
    /// 12,332 rules, 2.8 MB of nothing (research/reports/m26/perf-mem-3.md §4).
    fn shrink(&mut self) {
        self.rules.shrink_to_fit();
        self.index.shrink();
        self.conditions.shrink_to_fit();
        self.condition_text.shrink_to_fit();
        self.viewport_blocks.shrink_to_fit();
    }

    /// The UA defaults, collected before the author's sheet (see [`UA_SHEET`]).
    /// Parsed once: the text is a constant.
    fn collect_ua(&mut self, order: &mut u32) {
        let parsed = UA_PARSED.with(|ua| {
            *ua.get_or_init(|| match StyleSheet::parse(UA_SHEET, ParserOptions::default()) {
                Ok(sheet) => Some(&*Box::leak(Box::new(sheet))),
                Err(_) => None,
            })
        });
        match parsed {
            Some(sheet) => self.collect(&sheet.rules.0, None, order),
            None => self.diagnostics.push("vue-native's UA defaults did not parse".into()),
        }
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

    /// The class names the rule index answers about an element's PARENT
    /// without the matcher ever asking.
    ///
    /// `.stack > *` is offered only to the children of a `.stack`
    /// (`RuleIndex`'s parent buckets), so the child of a plain `div` never
    /// asks its parent about `stack`. A host that learns how far a change
    /// reaches from the matcher's own questions (vn-tree's `ReadOnOthers`)
    /// would then see nobody read `stack`, and a `div` that gains it would
    /// restyle alone, leaving its children on their old match. A change to one
    /// of these names, on any element, reaches that element's children.
    ///
    /// A parent's id needs no such list: the index reads it through
    /// [`MatchDom::attr`], which the host sees as a question.
    pub fn classes_keyed_on_the_parent(&self) -> impl Iterator<Item = &str> {
        self.index.by_parent_class.keys().map(|k| &**k)
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

    /// The `@keyframes` block `name` refers to in THIS environment, if the
    /// sheet has one.
    ///
    /// Shared by `Rc` because a running animation holds the block it started
    /// with (see `Animator`'s `AnimRun`): the render walk has a node id and a
    /// clock, and no stylesheet to look the name up in again.
    ///
    /// `env` is not decoration. A `@keyframes` inside an `@media` block is only
    /// declared where that query holds, and "where" is a WINDOW: the same name
    /// can resolve to a different animation in a 380 px pop-out and in the
    /// 1280 px document that opened it (docs/MULTIWINDOW.md §6). The last
    /// entry in document order whose condition holds wins, which is CSS's rule
    /// for duplicate names with the inactive blocks removed first.
    pub fn keyframes(&self, name: &str, env: &Env) -> Option<Rc<keyframes::Keyframes>> {
        self.keyframes.get(name)?.iter().rev().find_map(|entry| {
            match entry.condition {
                Some(c) if !self.conditions[c as usize].eval(env) => None,
                _ => Some(entry.block.clone()),
            }
        })
    }

    /// The `@keyframes` block `name` refers to, with its `var()`-composed
    /// declarations substituted against ONE ELEMENT's custom properties.
    ///
    /// A `@keyframes` block is document-scoped; a custom property is not. CSS
    /// resolves the two at computed-value time on the animating element, which
    /// is what makes `tailwindcss-animate` work at all: `enter` and `exit`
    /// declare nothing but `var(--tw-enter-opacity, 1)` and
    /// `scale3d(var(--tw-enter-scale, 1), …)`, and the utilities that go with
    /// them (`fade-in-0`, `zoom-in-95`, `slide-in-from-top-2`) are pure
    /// custom-property declarations on the element. Resolve them against the
    /// document and every animation in the library is the identity.
    ///
    /// Called once when a run STARTS, not per frame — the resolved block is
    /// what `AnimRun` then holds. A block with no `var()` in it takes the
    /// `Rc::clone` fast path and costs nothing.
    pub fn keyframes_for(
        &self,
        name: &str,
        vars: &Vars,
        env: &Env,
        diagnostics: &mut Vec<String>,
    ) -> Option<Rc<keyframes::Keyframes>> {
        let block = self.keyframes(name, env)?;
        if !block.dynamic {
            return Some(block);
        }
        let steps = block
            .steps
            .iter()
            .map(|step| {
                let mut style = (*step.style).clone();
                // A keyframe cannot animate an `Extras` value (none of them is
                // in `TransitionProperty`), but `apply_dynamic` needs somewhere
                // to put one it maps — so it gets a scratch bucket that is
                // dropped, rather than a silent panic path.
                let mut extras = Extras::default();
                if let Some(list) = &step.dynamic {
                    for (property, raw) in list.iter() {
                        mapper::apply_dynamic(
                            property,
                            raw,
                            vars,
                            env,
                            &mut style,
                            &mut extras,
                            diagnostics,
                        );
                    }
                }
                keyframes::Keyframe {
                    offset: step.offset,
                    easing: step.easing,
                    style: Rc::new(style),
                    dynamic: None,
                }
            })
            .collect();
        Some(Rc::new(keyframes::Keyframes { steps, dynamic: false }))
    }

    /// Does the sheet define any animation at all? The animator can skip its
    /// whole pass when not — the common case for a hand-written sheet.
    pub fn has_keyframes(&self) -> bool {
        !self.keyframes.is_empty()
    }

    /// Hands `f` every rule this element could match, once each. With the index
    /// off (or on a sheet compiled before it existed) that is simply every rule.
    /// The second argument is the bucket that offered it (`BUCKET_*`), for the
    /// perf probe's split.
    fn candidates<D: MatchDom>(&self, node: &D, f: &mut dyn FnMut(u32, usize)) {
        if !self.index.enabled {
            for i in 0..self.rules.len() as u32 {
                f(i, BUCKET_ANY);
            }
            return;
        }
        let ix = &self.index;
        for &i in &ix.any {
            f(i, BUCKET_ANY);
        }
        offer_keyed(node, [&ix.by_tag, &ix.by_class, &ix.by_id], [BUCKET_TAG, BUCKET_CLASS, BUCKET_ID], f);
        if node.is_root() {
            for &i in &ix.root {
                f(i, BUCKET_ROOT);
            }
        }
        if !(ix.by_parent_tag.is_empty() && ix.by_parent_class.is_empty() && ix.by_parent_id.is_empty()) {
            // The matcher's `>` reads `MatchDom::parent`, so this does too.
            if let Some(parent) = node.parent() {
                offer_keyed(
                    &parent,
                    [&ix.by_parent_tag, &ix.by_parent_class, &ix.by_parent_id],
                    [BUCKET_PARENT; 3],
                    f,
                );
            }
        }
    }

    /// Walks one rule list, descending into `@media` blocks so their style
    /// rules join the same flat list with a condition index. `order` is shared
    /// across the whole walk, so document order (the cascade tiebreak) survives
    /// nesting.
    fn collect(&mut self, rules: &[CssRule<'_>], condition: Option<u32>, order: &mut u32) {
        for rule in rules {
            let style = match rule {
                CssRule::Style(style) => style,
                CssRule::Media(m) => {
                    match media::parse_list(&m.query) {
                        Ok(cond) => {
                            let idx = self.push_condition(cond, condition, &media::list_text(&m.query));
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
                    // The enclosing `@media` rides along: a block declared
                    // inside one exists only where that query holds, which is
                    // a per-WINDOW question (docs/MULTIWINDOW.md §6).
                    self.collect_keyframes(kf, condition);
                    continue;
                }
                // `@supports` is a feature TEST, and the honest answer to it is
                // whatever vn-css actually does with the declaration in the
                // prelude. A block guarded on something unimplemented is
                // supposed to be skipped — a browser skips it too when the
                // feature is missing — so evaluating the condition is both the
                // implementation and the reason there is nothing to advise.
                CssRule::Supports(sup) => {
                    if supports_condition(&sup.condition) {
                        self.collect(&sup.rules.0, condition, order);
                    }
                    continue;
                }
                other => {
                    at_rule_ignored(other, &mut self.diagnostics);
                    continue;
                }
            };
            let (converted, viewport) =
                mapper::noting_viewport_units(|| mapper::convert_block(&style.declarations));
            let first_rule = self.rules.len() as u32;
            // The author's own words, for the devtools styles panel. Off by
            // default: it is one `String` pair per declaration in the sheet,
            // and nothing in the RENDER path ever reads them.
            let decls = capture_rule_text()
                .then(|| Rc::new(mapper::decl_pairs(&style.declarations)));
            let declares_transition =
                converted.normal_transitions.is_some() || converted.important_transitions.is_some();
            let declares_extras = {
                let mut e = converted.normal_extras.clone();
                e.refine(&converted.important_extras);
                e
            };
            for sel in &style.selectors.0 {
                // `:has()` is REWRITTEN before anything else looks at the
                // selector: parcel's matcher panics on `Component::Has`, so the
                // relational question is moved onto a custom pseudo-class the
                // matcher does route to the element (see `crate::has`). The
                // rewrite re-parses, so from here down this is an ordinary
                // selector with an ordinary pseudo-class in it.
                //
                // Either way the rule gets its OWN copy of the selector: the
                // sheet it was parsed from is dropped when the compile ends.
                use lightningcss::traits::IntoOwned as _;
                let owned: Selector<'static> = if has::present(&sel_text(sel)) {
                    match has::rewrite(&sel_text(sel)).and_then(|text| reparse_selector(&text)) {
                        Some(s) => s.clone(),
                        None => {
                            mapper::once(
                                &mut self.diagnostics,
                                format!(
                                    "`:has()` with an argument that cannot be compiled — rules                                      using it were dropped (first seen in `{}`)",
                                    sel_text(sel)
                                ),
                            );
                            continue;
                        }
                    }
                } else {
                    sel.clone().into_owned()
                };
                let sel = &owned;
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
                // Both of the advisories below are about this rule's
                // DECLARATIONS, so they ride with them and are said when the
                // rule MATCHES an element — a Tailwind sheet carries a
                // `focus:z-10` utility whether or not anything is focusable.
                let mut rule_diagnostics: Vec<String> =
                    converted.diagnostics.as_deref().cloned().unwrap_or_default();
                if states != 0 || ancestor.is_some() {
                    // The `Extras` have no state ladder to install into — gpui's
                    // `img()` carries ONE `ObjectFit` for the element, and the
                    // window hit test reads an already-PAINTED frame, so a
                    // state-dependent drag region would be a hit-test/paint race
                    // (docs/WINDOWING.md D-16). So a state rule's extras apply
                    // all the time; say so rather than let them look conditional.
                    //
                    // Only the fields an author actually WROTE count. Several
                    // `Extras` are derived rather than declared —
                    // `background_key_color` is computed from whatever
                    // `background` a rule sets — so guarding on "any extra at
                    // all" reported every `hover:bg-*` utility in the sheet,
                    // with an empty list of names because there was nothing to
                    // name.
                    let names = extras_names(&declares_extras);
                    if !names.is_empty() {
                        rule_diagnostics.push(format!(
                            "{names} on a state selector applies unconditionally (v1) — these                              values are read from the base state only, not from gpui's state                              ladder: {}",
                            sel_text(sel)
                        ));
                    }
                    if declares_transition {
                        // State refinements are installed into gpui's own state
                        // ladder and never pass through our animator, so a
                        // `transition` declared on a state rule cannot run.
                        // Declare it on the base rule instead (that is also what
                        // authors mean 99% of the time).
                        rule_diagnostics.push(format!(
                            "transition declared on a state selector is ignored (v1) — move it                              to the base rule: {}",
                            sel_text(sel)
                        ));
                    }
                }
                self.sibling_combinators |= has_sibling_combinator(sel);
                self.group_states |= ancestor.is_some();
                self.index.file(sel, self.rules.len() as u32);
                let specificity = sel.specificity();
                self.rules.push(RuleEntry {
                    selector: owned,
                    normal: converted.normal.clone(),
                    important: converted.important.clone(),
                    normal_transitions: converted.normal_transitions.clone(),
                    important_transitions: converted.important_transitions.clone(),
                    normal_transition_declarations: converted.normal_transition_declarations.clone(),
                    important_transition_declarations: converted.important_transition_declarations.clone(),
                    normal_animations: converted.normal_animations.clone(),
                    important_animations: converted.important_animations.clone(),
                    normal_animation_patch: converted.normal_animation_patch.clone(),
                    important_animation_patch: converted.important_animation_patch.clone(),
                    normal_extras: converted.normal_extras.clone(),
                    important_extras: converted.important_extras.clone(),
                    normal_custom: converted.normal_custom.clone(),
                    important_custom: converted.important_custom.clone(),
                    normal_dynamic: converted.normal_dynamic.clone(),
                    important_dynamic: converted.important_dynamic.clone(),
                    normal_font: converted.normal_font.clone(),
                    important_font: converted.important_font.clone(),
                    specificity,
                    order: *order,
                    states,
                    ancestor,
                    pseudo,
                    condition,
                    diagnostics: (!rule_diagnostics.is_empty())
                        .then(|| Rc::new(std::mem::take(&mut rule_diagnostics))),
                    decls: decls.clone(),
                });
                *order += 1;
            }
            // The block's own text: the parse goes when the compile ends, and
            // a resize converts this block again ([`ViewportBlock`]).
            let rules = first_rule..self.rules.len() as u32;
            if viewport && !rules.is_empty() {
                use lightningcss::traits::ToCss as _;
                match style.declarations.to_css_string(lightningcss::printer::PrinterOptions::default()) {
                    Ok(css) => self.viewport_blocks.push(ViewportBlock { css: css.into(), rules }),
                    Err(_) => self.viewport_unkept = true,
                }
            }
        }
    }

    /// Takes the engine apart for `tests/sheet_memory.rs`, which drops the
    /// parts one at a time under a counting allocator to see what each holds
    /// (research/reports/m26/perf-mem-3.md §4). Each part comes with its INLINE
    /// bytes: what its values take in the vector they were moved into, which is
    /// the same as their share of the rule table. What a part frees beyond that
    /// is the heap behind it.
    #[doc(hidden)]
    pub fn into_parts_for_measuring(self) -> Vec<(&'static str, usize, Box<dyn std::any::Any>)> {
        fn part<T: 'static>(name: &'static str, v: Vec<T>) -> (&'static str, usize, Box<dyn std::any::Any>) {
            (name, v.len() * std::mem::size_of::<T>(), Box::new(v))
        }
        let table = self.rules.capacity() * std::mem::size_of::<RuleEntry>();
        let n = self.rules.len();
        let mut selectors = Vec::with_capacity(n);
        let mut refinements = Vec::with_capacity(n);
        let mut transitions = Vec::with_capacity(n);
        let mut animations = Vec::with_capacity(n);
        let mut extras = Vec::with_capacity(n);
        let mut custom = Vec::with_capacity(n);
        let mut dynamic = Vec::with_capacity(n);
        let mut font = Vec::with_capacity(n);
        let mut diagnostics = Vec::with_capacity(n);
        let mut decls = Vec::with_capacity(n);
        for r in self.rules {
            font.push((r.normal_font, r.important_font));
            selectors.push(r.selector);
            refinements.push((r.normal, r.important));
            transitions.push((r.normal_transitions, r.important_transitions));
            animations.push((r.normal_animations, r.important_animations, r.normal_animation_patch, r.important_animation_patch));
            extras.push((r.normal_extras, r.important_extras));
            custom.push((r.normal_custom, r.important_custom));
            dynamic.push((r.normal_dynamic, r.important_dynamic));
            diagnostics.push(r.diagnostics);
            decls.push(r.decls);
        }
        vec![
            ("rule table (every field inline, before the parts below moved out)", table, Box::new(())),
            part("selectors", selectors),
            part("style refinements (normal, !important)", refinements),
            part("extras (normal, !important)", extras),
            part("transitions", transitions),
            part("animations", animations),
            part("custom properties", custom),
            part("var()-composed declarations", dynamic),
            part("font-relative declarations", font),
            part("per-rule advisories", diagnostics),
            part("author text (devtools capture)", decls),
            ("rule index", 0, Box::new(self.index)),
            ("@media conditions and their text", 0, Box::new((self.conditions, self.condition_text))),
            ("@keyframes", 0, Box::new(self.keyframes)),
            ("viewport blocks", 0, Box::new(self.viewport_blocks)),
            ("sheet advisories", 0, Box::new(self.diagnostics)),
        ]
    }

    /// Resolves the sheet's viewport units (`vw`, `vh`, `vmin`, …) again, in
    /// place, against the basis [`set_viewport_basis`] holds now. `false` = it
    /// cannot (a `@keyframes` block uses one, say), and the sheet has to be
    /// compiled again.
    ///
    /// vn-css flattens a viewport length to px when it converts a declaration,
    /// so a resize used to recompile the whole sheet: ~30 ms for the Argon
    /// client's 1.27 MB, ~60 ms since the compile keeps no parse to reuse
    /// (perf-mem-3.md §2), on every step of a window drag. Only the few
    /// blocks that resolved a viewport unit change, and the engine keeps
    /// exactly those ([`ViewportBlock`]), so this converts a few dozen blocks
    /// instead.
    ///
    /// Every other part of a rule (selector, specificity, order, state
    /// classification, `@media` condition) does not depend on the viewport,
    /// nor do the advisories, which name properties and selectors. Custom
    /// properties and `var()`-composed declarations are resolved per element
    /// at cascade time already. The caller restyles the whole document after
    /// either path, as it does on any resize.
    pub fn rebase_viewport(&mut self) -> bool {
        if self.viewport_unkept {
            return false;
        }
        // Folded results from before the rebase are not results of this sheet.
        self.generation = next_generation();
        let options = || ParserOptions { error_recovery: true, ..ParserOptions::default() };
        for block in &self.viewport_blocks {
            let Ok(declarations) = DeclarationBlock::parse_string(&block.css, options()) else {
                // It printed, so it parses; if it ever does not, the
                // recompile replaces every rule this loop already touched.
                return false;
            };
            let converted = mapper::convert_block(&declarations);
            for i in block.rules.clone() {
                self.rules[i as usize].set_declarations(&converted);
            }
        }
        true
    }

    /// Interns a condition, `and`-ing it with the enclosing block's when
    /// `@media` blocks nest. Identical conditions share one slot — a Tailwind
    /// sheet repeats `(min-width: 640px)` once per emitted block.
    fn push_condition(&mut self, cond: media::Cond, parent: Option<u32>, text: &str) -> u32 {
        let cond = match parent {
            Some(p) => self.conditions[p as usize].clone().and(cond),
            None => cond,
        };
        let text = match parent {
            Some(p) => format!("{} and {text}", self.condition_text[p as usize]),
            None => text.to_string(),
        };
        if let Some(i) = self.conditions.iter().position(|c| *c == cond) {
            return i as u32;
        }
        self.conditions.push(cond);
        self.condition_text.push(text);
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
            None,
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
    /// `parent_color` is the parent's [`ComputedStates::color`], threaded the
    /// same way and for the same reason: `currentColor` resolves against the
    /// element's computed `color`, which is inherited when it declares none.
    pub fn compute_node<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&InlineStyle>,
        parent_vars: &Rc<Vars>,
        parent_color: Option<gpui::Hsla>,
        env: &Env,
        diagnostics: &mut Vec<String>,
    ) -> ComputedStates {
        self.compute_node_at(node, inline, parent_vars, parent_color, mapper::rem_basis(), env, diagnostics)
    }

    /// [`Self::compute_node`] for an element whose parent's computed font size
    /// is `parent_font_size` px: the walk threads each element's
    /// [`ComputedStates::font_size`] into its children, as it does `color`.
    /// The document root's parent is the initial `medium`, 16px.
    ///
    /// `em` resolves against the element's own font size, except in
    /// `font-size` itself, where it is the parent's (CSS Values 4 §6.1.1). So
    /// the font size is folded first, then everything else under it. A `:hover`
    /// (or other state) rule that changes the font size gets its rung folded
    /// at that size: the rung's own em declarations, and the base's em
    /// declarations again, so a `padding: 1em` grows with a hovered
    /// `font-size`. A `group-hover:` rule's em uses the base size.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_node_at<D: MatchDom>(
        &self,
        node: D,
        inline: Option<&InlineStyle>,
        parent_vars: &Rc<Vars>,
        parent_color: Option<gpui::Hsla>,
        parent_font_size: f32,
        env: &Env,
        diagnostics: &mut Vec<String>,
    ) -> ComputedStates {
        self.compute_node_with_font(node, inline, parent_vars, parent_color, parent_font_size, &gpui::Font::default(), env, diagnostics)
    }

    /// The full cascade with the inherited face used by font-relative glyph units.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_node_with_font<D: MatchDom>(
        &self, node: D, inline: Option<&InlineStyle>, parent_vars: &Rc<Vars>,
        parent_color: Option<gpui::Hsla>, parent_font_size: f32, parent_font: &gpui::Font,
        env: &Env, diagnostics: &mut Vec<String>,
    ) -> ComputedStates {
        let probe = perf_on();
        let t0 = probe.then(std::time::Instant::now);
        let el = element::El(node);
        let mut matched: Vec<&RuleEntry> = Vec::new();
        let mut tested = 0u64;
        let mut by_bucket = [0u32; 6];
        self.candidates(&node, &mut |i, bucket| {
            let rule = &self.rules[i as usize];
            // Cheaper than matching, and it has to run first anyway: a rule
            // inside a `@media` block that does not hold is simply not there.
            if let Some(c) = rule.condition {
                if !self.conditions[c as usize].eval(env) {
                    return;
                }
            }
            tested += 1;
            by_bucket[bucket] += 1;
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
            if matches_selector(&rule.selector, 0, None, &el, &mut ctx, &mut |_, _| {}) {
                matched.push(rule);
            }
        });
        // The index hands the candidates back bucket by bucket, so this sort is
        // what restores the cascade's document order — it is not an
        // optimization detail, it is the cascade. Entries that tie came from one
        // rule's selector list and carry the same declarations; the position in
        // the rule table breaks the tie only so that two elements that matched
        // the same rules list them the same way ([`share`]).
        let base = self.rules.as_ptr() as usize;
        let index_of =
            |r: &RuleEntry| ((r as *const RuleEntry as usize - base) / std::mem::size_of::<RuleEntry>()) as u32;
        matched.sort_by_key(|r| (r.specificity, r.order, index_of(r)));
        let t1 = probe.then(std::time::Instant::now);

        // ---- 0. the matched-properties cache: an earlier element that matched
        // these rules under the same inherited inputs folded to this result.
        let share_key = (self.share && inline.is_none() && !matched.iter().any(|r| r.ancestor.is_some()))
            .then(|| {
                let rules = matched.iter().map(|r| index_of(r)).collect();
                share::Key::new(self.generation, rules, parent_vars, parent_color, parent_font_size, parent_font, env)
            });
        if let Some(key) = &share_key {
            let hit = share::lookup(key);
            if probe {
                perf::note_share(hit.is_some());
            }
            if let Some(states) = hit {
                if let (Some(t0), Some(t1)) = (t0, t1) {
                    let ms = t1.saturating_duration_since(t0).as_secs_f32() * 1000.0;
                    perf::note_node(tested, ms, 0.0, 0.0);
                    perf::note_split(by_bucket, matched.len());
                }
                return states;
            }
        }

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

        // ---- 1b. the font size, before anything that reads an em: the base
        // state's, and each state rung's that declares its own. Each resolves
        // its own em against the parent's.
        let mut font_size: Option<f32> = None;
        let mut rung_font: [Option<f32>; STATE_RUNGS] = [None; STATE_RUNGS];
        for important_pass in [false, true] {
            for r in &matched {
                if r.pseudo.is_some() || r.ancestor.is_some() {
                    continue;
                }
                let rung = rung_of(r.states);
                let scope = rung.and_then(|i| state_vars[i].as_ref()).unwrap_or(&vars);
                if let Some(size) = mapper::with_font_face(parent_font, || r.half(important_pass).font_size(scope, env, parent_font_size)) {
                    match rung {
                        None => font_size = Some(size),
                        Some(i) => rung_font[i] = Some(size),
                    }
                }
            }
        }
        if let Some(size) = mapper::with_font_face(parent_font, || inline.and_then(|i| Half::of_inline(i).font_size(&vars, env, parent_font_size))) {
            font_size = Some(size);
        }
        let font_size = font_size.unwrap_or(parent_font_size);
        let mut descriptor = gpui::TextStyleRefinement::default();
        let mut rung_descriptors: [gpui::TextStyleRefinement; STATE_RUNGS] = Default::default();
        for important in [false, true] {
            for rule in matched.iter().filter(|rule| rule.pseudo.is_none() && rule.ancestor.is_none()) {
                let rung = rung_of(rule.states);
                let scope = rung.and_then(|index| state_vars[index].as_ref()).unwrap_or(&vars);
                let target = rung.map_or(&mut descriptor, |index| &mut rung_descriptors[index]);
                rule.half(important).font_descriptor(scope, env, target);
            }
        }
        if let Some(inline) = inline { Half::of_inline(inline).font_descriptor(&vars, env, &mut descriptor); }
        let computed_font = refine_font(parent_font, &descriptor);
        let rung_faces: [gpui::Font; STATE_RUNGS] = std::array::from_fn(|index| refine_font(&computed_font, &rung_descriptors[index]));
        let t2 = probe.then(std::time::Instant::now);

        // A matched rule's declaration advisories are the ones that describe
        // THIS element, so this is where they are said. `matched` is the set
        // that survived the media query and the selector, which is exactly the
        // set whose declarations reach a box.
        for r in &matched {
            if let Some(list) = &r.diagnostics {
                for d in list.iter() {
                    mapper::once(diagnostics, d.clone());
                }
            }
        }

        // ---- 2. the refinements.
        let mut base = StyleRefinement::default();
        let mut hover = StyleRefinement::default();
        let mut active = StyleRefinement::default();
        let mut focus = StyleRefinement::default();
        let mut focus_visible = StyleRefinement::default();
        let mut focus_within = StyleRefinement::default();
        let (mut any_hover, mut any_active) = (false, false);
        let (mut any_focus, mut any_focus_visible, mut any_focus_within) = (false, false, false);
        // `.group:hover .this` — one group per state, because gpui carries one
        // `group_hover` / one `group_active` per element. The key is resolved
        // once per (rule, element) pair and memoised for the pass: a Tailwind
        // card typically matches three or four `group-hover:` rules pointing at
        // the same ancestor.
        let mut group = GroupFold::default();
        // Aggregate lists remain the compatibility fallback. Parsed motion
        // component lists are folded independently in priority order below.
        // State-rule transitions are rejected at compile time.
        let mut transitions: Option<Rc<Vec<TransitionSpec>>> = None;
        // Animation component lists cascade independently; zip them only after
        // all matched rules and the inline style have declared their longhands.
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
        let mut selection = (StyleRefinement::default(), Extras::default());
        let mut file_button = (StyleRefinement::default(), Extras::default());
        let (mut any_before, mut any_after, mut any_placeholder) = (false, false, false);
        let (mut any_selection, mut any_file_button) = (false, false);
        for important_pass in [false, true] {
            for r in &matched {
                if let Some(which) = r.pseudo {
                    let (slot, seen) = match which {
                        PseudoElement::Before => (&mut before, &mut any_before),
                        PseudoElement::After => (&mut after, &mut any_after),
                        PseudoElement::Placeholder => (&mut placeholder, &mut any_placeholder),
                        PseudoElement::Selection => (&mut selection, &mut any_selection),
                        PseudoElement::FileSelectorButton => (&mut file_button, &mut any_file_button),
                    };
                    *seen = true;
                    slot.1.refine(if important_pass {
                        &r.important_extras
                    } else {
                        &r.normal_extras
                    });
                    // A generated box is the element's child: its `font-size`
                    // em is the element's size, and so, here, is every other
                    // em it declares (a box that also sets its own font size
                    // resolves the rest against the element's, v1).
                    r.half(important_pass).apply(
                        &vars, env, font_size, font_size, &computed_font, &computed_font, &mut slot.0, &mut slot.1, diagnostics,
                    );
                    continue;
                }
                // `border-radius: inherit` is filled into the BASE style after
                // the cascade, so only a base rule may say which corners
                // inherit: `.x:hover { border-radius: 8px }` must not cancel
                // `.x { border-radius: inherit }` outside the hover, nor a
                // hover's `inherit` apply when the pointer is elsewhere.
                let state_rule = r.states != 0 || r.ancestor.is_some();
                let radius_inherit = extras.radius_inherit;
                let svg_before = state_rule.then(|| svg_base(&extras));
                extras.refine(if important_pass {
                    &r.important_extras
                } else {
                    &r.normal_extras
                });
                if state_rule {
                    keep_base_radius_inherit(&mut extras, radius_inherit, diagnostics);
                }
                if let Some(before) = svg_before {
                    keep_base_svg_paint(&mut extras, before, diagnostics);
                }
                let half = r.half(important_pass);
                if half.is_empty() {
                    continue;
                }
                // `.group:hover .this` goes to its own bucket, keyed by the
                // ancestor it watches. Resolving the ancestor is a walk up from
                // this element re-matching the selector's PREFIX, which the
                // full match (run with `:hover` assumed true) already proved
                // has a solution — so a `None` here means the tree moved under
                // us and the rule simply does not apply.
                if let Some(a) = r.ancestor {
                    let Some(key) = nearest_group_ancestor(&node, &r.selector, a.offset) else {
                        continue;
                    };
                    let Some(bucket) = group.bucket(a.states, key) else {
                        continue;
                    };
                    // Its em is the base state's: the size a hovered ANCESTOR
                    // gives this element is not modelled (v1).
                    let svg_before = svg_base(&extras);
                    half.apply(&vars, env, font_size, parent_font_size, &computed_font, parent_font, bucket, &mut extras, diagnostics);
                    if half.font_relative() {
                        keep_base_radius_inherit(&mut extras, radius_inherit, diagnostics);
                    }
                    keep_base_svg_paint(&mut extras, svg_before, diagnostics);
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
                    Some(3) => {
                        any_focus = true;
                        &mut focus
                    }
                    _ => {
                        any_focus_within = true;
                        &mut focus_within
                    }
                };
                // A rung's `var()`-composed declarations read that rung's
                // custom properties (base map + whatever the rung declares
                // on top), never another rung's. `hover:shadow-lg` sets
                // `--tw-shadow` and reads it back in the same `:hover`
                // rule; `focus-visible:ring-2` does the same one rung over,
                // and the two must not see each other. Its em is the rung's
                // font size when the rung declares one.
                let scope = rung.and_then(|i| state_vars[i].as_ref()).unwrap_or(&vars);
                let basis = rung.and_then(|i| rung_font[i]).unwrap_or(font_size);
                let radius_inherit = extras.radius_inherit;
                let svg_before = rung.is_some().then(|| svg_base(&extras));
                half.apply(scope, env, basis, parent_font_size, rung.map_or(&computed_font, |index| &rung_faces[index]), parent_font, bucket, &mut extras, diagnostics);
                if rung.is_some() && half.font_relative() {
                    keep_base_radius_inherit(&mut extras, radius_inherit, diagnostics);
                }
                if let Some(before) = svg_before {
                    keep_base_svg_paint(&mut extras, before, diagnostics);
                }
            }
        }
        if let Some(inline) = inline {
            Half::of_inline(inline).apply(
                &vars, env, font_size, parent_font_size, &computed_font, parent_font, &mut base, &mut extras, diagnostics,
            );
            extras.refine(&inline.extras);
        }
        // Motion has its own declared-component metadata. Fold it in CSS
        // priority order without changing general StyleRefinement handling:
        // normal sheet -> normal inline -> important sheet -> important inline.
        let mut transition_components = transition::Collector::default();
        for important in [false, true] {
            for rule in &matched {
                if rule.pseudo.is_some() || rule.states != 0 || rule.ancestor.is_some() { continue; }
                let declarations = if important { &rule.important_transition_declarations } else { &rule.normal_transition_declarations };
                if let Some(declarations) = declarations { transition_components.merge(declarations); }
                let patch = if important { &rule.important_animation_patch } else { &rule.normal_animation_patch };
                if let Some(patch) = patch { patch.apply_over(&mut animation_patch); }
            }
            if let Some(inline) = inline {
                if inline.normal_motion.is_some() || inline.important_motion.is_some() {
                    let motion = if important { &inline.important_motion } else { &inline.normal_motion };
                    if let Some(motion) = motion {
                        if let Some(declarations) = &motion.transitions { transition_components.merge(declarations); }
                        if let Some(patch) = &motion.animations { patch.apply_over(&mut animation_patch); }
                    }
                } else if !important {
                    // Legacy callers construct InlineStyle's aggregate public
                    // fields directly. Treat those as normal inline shorthand.
                    if let Some(list) = &inline.transitions { transition_components.merge(&transition::Collector::from_specs(list)); }
                    if let Some(list) = &inline.animations {
                        animations = Some(list.clone());
                        if let Some(patch) = keyframes::AnimationCollector::from_specs(list).finish().1 { patch.apply_over(&mut animation_patch); }
                    }
                    if let Some(patch) = &inline.animation_patch { patch.apply_over(&mut animation_patch); }
                }
            }
        }
        if let Some(list) = transition_components.finish() { transitions = Some(Rc::new(list)); }

        // A state rung that changes the font size changes every em the BASE
        // resolved too: `.x { padding: 1em } .x:hover { font-size: 20px }`
        // pads 20px under the pointer. gpui's ladder lays the rung over the
        // base, so the rung gets the base folded again at its size, under its
        // own declarations. Only for a rung that declares a size, and only
        // when the base has something font-relative to fold again.
        let base_font_relative = matched
            .iter()
            .filter(|r| r.pseudo.is_none() && r.ancestor.is_none() && r.states == 0)
            .any(|r| r.half(false).font_relative() || r.half(true).font_relative())
            || inline.is_some_and(|i| Half::of_inline(i).font_relative());
        if base_font_relative {
            let rungs = [&mut active, &mut hover, &mut focus_visible, &mut focus, &mut focus_within];
            for (i, bucket) in rungs.into_iter().enumerate() {
                let size = rung_font[i].unwrap_or(font_size);
                if size == font_size && rung_faces[i] == computed_font {
                    continue;
                }
                let mut again = StyleRefinement::default();
                let mut scratch = Extras::default();
                let mut quiet = Vec::new();
                for important_pass in [false, true] {
                    for r in matched.iter().filter(|r| r.pseudo.is_none() && r.ancestor.is_none() && r.states == 0) {
                        r.half(important_pass).apply(
                            &vars, env, size, parent_font_size, &rung_faces[i], parent_font, &mut again, &mut scratch, &mut quiet,
                        );
                    }
                }
                if let Some(inline) = inline {
                    Half::of_inline(inline).apply(
                        &vars, env, size, parent_font_size, &rung_faces[i], parent_font, &mut again, &mut scratch, &mut quiet,
                    );
                }
                again.refine(bucket);
                *bucket = again;
            }
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
            perf::note_split(by_bucket, matched.len());
        }
        // `currentColor` is the element's computed `color`, so it can only be
        // substituted now — after the whole cascade has run. Every rung gets
        // the same value: `color` itself is not in gpui's state ladder, so a
        // `:hover` rule cannot change what `currentColor` means.
        let effective_color = base.text.color.or(parent_color);
        if extras.current_color != mapper::CurrentColorTargets::default()
            || [&base, &hover, &active, &focus, &focus_visible, &focus_within]
                .iter().any(|refinement| mapper::border_needs_current_color(refinement))
        {
            let value = effective_color.unwrap_or_else(gpui::black);
            let targets = extras.current_color;
            for refinement in [
                Some(&mut base),
                any_hover.then_some(&mut hover),
                any_active.then_some(&mut active),
                any_focus.then_some(&mut focus),
                any_focus_visible.then_some(&mut focus_visible),
                any_focus_within.then_some(&mut focus_within),
            ]
            .into_iter()
            .flatten()
            {
                mapper::apply_current_color(refinement, targets, value);
            }
        }
        for (_, refinement) in group.hover.iter_mut().chain(group.active.iter_mut()) {
            if mapper::border_needs_current_color(refinement) {
                let color = refinement.text.color.or(effective_color).unwrap_or_else(gpui::black);
                mapper::apply_current_color(refinement, CurrentColorTargets::default(), color);
            }
        }
        for (refinement, _) in [&mut before, &mut after] {
            if mapper::border_needs_current_color(refinement) {
                let color = refinement.text.color.or(effective_color).unwrap_or_else(gpui::black);
                mapper::apply_current_color(refinement, CurrentColorTargets::default(), color);
            }
        }
        // `text-underline-offset` and `text-decoration-line` are two properties
        // and, in Tailwind, two classes — `.underline .underline-offset-4` —
        // so the offset is applied to whatever underline the WHOLE cascade
        // produced rather than to whatever one rule happened to declare.
        if let Some(offset) = extras.underline_offset {
            for refinement in [
                Some(&mut base),
                any_hover.then_some(&mut hover),
                any_active.then_some(&mut active),
                any_focus.then_some(&mut focus),
                any_focus_visible.then_some(&mut focus_visible),
                any_focus_within.then_some(&mut focus_within),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(underline) = refinement.text.underline.as_mut() {
                    underline.offset = gpui::px(offset);
                }
            }
        }
        // `overflow-x` and `overflow-y` compute as a pair (`overflow-y: auto`
        // alone scrolls both axes), and one rule may have declared each.
        mapper::compute_overflow(&mut base, &mut extras);
        mapper::compute_overflow(&mut before.0, &mut before.1);
        mapper::compute_overflow(&mut after.0, &mut after.1);

        let states = ComputedStates {
            base: Rc::new(base),
            hover: any_hover.then(|| Rc::new(hover)),
            active: any_active.then(|| Rc::new(active)),
            focus: any_focus.then(|| Rc::new(focus)),
            focus_visible: any_focus_visible.then(|| Rc::new(focus_visible)),
            focus_within: any_focus_within.then(|| Rc::new(focus_within)),
            group_hover: group.hover.map(|(key, r)| (key, Rc::new(r))),
            group_active: group.active.map(|(key, r)| (key, Rc::new(r))),
            transitions: transitions.unwrap_or_default(),
            animations: {
                if let Some(list) = animation_patch.computed_list() {
                    Rc::new(list)
                } else {
                    let list = animations.unwrap_or_default();
                    if animation_patch.is_empty() || list.is_empty() {
                        list
                    } else {
                        // Legacy scalar patches remain usable independently
                        // of the declaration lists carried by parsed CSS.
                        let mut patched = (*list).clone();
                        for spec in patched.iter_mut() {
                            animation_patch.apply(spec);
                        }
                        Rc::new(patched)
                    }
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
            selection: any_selection.then(|| Rc::new(selection.0)),
            file_selector_button: any_file_button.then(|| Rc::new(file_button.0)),
            vars,
            color: effective_color,
            font_size,
            computed_font,
        };
        if let Some(key) = share_key {
            share::insert(key, &states);
        }
        states
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
        rule: &lightningcss::rules::keyframes::KeyframesRule<'_>,
        condition: Option<u32>,
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
            let (converted, viewport) =
                mapper::noting_viewport_units(|| mapper::convert_block(&frame.declarations));
            // Not kept for [`Engine::rebase_viewport`]: rare, and a running
            // animation holds the block it started with.
            self.viewport_unkept |= viewport;
            // A keyframe block has no selector to match, so its declarations
            // can only be reported here.
            if let Some(list) = &converted.diagnostics {
                for d in list.iter() {
                    mapper::once(&mut self.diagnostics, d.clone());
                }
            }
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
            // A declaration whose value referenced `var()` or `env()` cannot be
            // mapped HERE: a `@keyframes` block is document-scoped and a custom
            // property belongs to an element. It travels with the step as raw
            // text and is substituted per element by `keyframes_for`.
            //
            // The whole of `tailwindcss-animate` lives on this path — `enter`
            // and `exit` declare nothing BUT var()-composed values — so before
            // this the two blocks compiled to zero steps and every `animate-in`
            // ran its clock, fired `animationstart`/`animationend` on time, and
            // interpolated nothing.
            let dynamic = match (converted.normal_dynamic, converted.important_dynamic) {
                (None, None) => None,
                (Some(only), None) | (None, Some(only)) => Some(only),
                (Some(normal), Some(important)) => {
                    let mut all = (*normal).clone();
                    all.extend(important.iter().cloned());
                    Some(Rc::new(all))
                }
            };
            // `transform` used to need a second channel here, because it
            // computed into `Extras` rather than into the refinement. It is a
            // real refinement property now (`Style::vn_transform`, m13 item 2),
            // so `any` above already sees it and a keyframe carries it like any
            // other animatable value.
            if !any && easing.is_none() && dynamic.is_none() {
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
                    dynamic: dynamic.clone(),
                });
            }
        }
        steps.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        let dynamic = steps.iter().any(|s| s.dynamic.is_some());
        // Appended, not replaced: "a later block with the same name wins" is
        // still the rule, but only among the blocks that are DECLARED — and a
        // media-scoped one is declared per window. [`Engine::keyframes`] does
        // the last-one-that-holds pick at lookup time.
        self.keyframes.entry(name).or_default().push(KeyframesBlock {
            condition,
            block: Rc::new(keyframes::Keyframes { steps, dynamic }),
        });
    }
}

/// One advisory PER AT-RULE KIND, not per rule: a generated sheet carries one
/// `@media` block per responsive/dark variant in the app (a broad Tailwind
/// sweep produced nine), and nine truncated `Debug` dumps bury everything else.
/// The message says what silently will not apply, which is the part that costs
/// an author an afternoon.
fn at_rule_ignored(rule: &CssRule<'_>, diagnostics: &mut Vec<String>) {
    let msg = match rule {
        // Not the engine's to apply: the host reads `@font-face` off the same
        // source, loads the fonts and compiles the sheet again once they are
        // installed (vn-tree's `fonts.rs`, m26 #29).
        CssRule::FontFace(_) => return,
        other => format!("at-rule ignored (v1): {other:?}")
            .chars()
            .take(120)
            .collect(),
    };
    mapper::once(diagnostics, msg);
}

/// Evaluates an `@supports` condition against what vn-css can actually do.
///
/// `@supports (display: grid)` is a question about the ENGINE, so the truthful
/// answer is the one the mapper gives: a declaration supports-tests true when
/// parsing it produces a real value and mapping it produces no advisory. That
/// makes a guarded block behave the way it does in a browser missing the same
/// feature — skipped — rather than being dropped wholesale whether or not the
/// feature is there.
///
/// `selector()` is answered by the same audit the cascade uses, so
/// `@supports selector(:has(a))` tracks the matcher.
fn supports_condition(condition: &lightningcss::rules::supports::SupportsCondition<'_>) -> bool {
    use lightningcss::rules::supports::SupportsCondition as C;
    match condition {
        C::Not(inner) => !supports_condition(inner),
        C::And(list) => list.iter().all(supports_condition),
        C::Or(list) => list.iter().any(supports_condition),
        C::Declaration { property_id, value } => {
            let text = format!("{}:{}", property_id.name(), value);
            let Ok(block) = DeclarationBlock::parse_string(&text, ParserOptions::default()) else {
                return false;
            };
            let converted = mapper::convert_block(&block);
            // A declaration vn-css maps produces a refinement, an extra or a
            // custom property, and no advisory. Anything else — an unknown
            // property, a value shape with no model here — is unsupported, and
            // saying so is the correct answer rather than a gap.
            converted.diagnostics.is_none()
                && (converted.normal.is_some()
                    || converted.normal_custom.is_some()
                    || converted.normal_dynamic.is_some()
                    || converted.normal_extras != Extras::default())
        }
        C::Selector(text) => {
            // Spliced into a one-rule sheet to reach the same parser the
            // cascade uses, exactly as `query_all` does.
            if text.contains(['{', '}']) {
                return false;
            }
            // Nothing parsed here outlives the answer (a rewritten `:has()`
            // comes back from `reparse_selector`, which keeps its own), so the
            // probe sheet is a local and is dropped with it.
            let src = format!("{text}{{color:red}}");
            let Ok(sheet) = StyleSheet::parse(&src, ParserOptions::default()) else {
                return false;
            };
            let mut probe = Vec::new();
            sheet.rules.0.iter().all(|rule| match rule {
                CssRule::Style(style) => style.selectors.0.iter().all(|sel| {
                    // Same rewrite the cascade does, and for the same reason:
                    // asking parcel about a raw `Component::Has` aborts the
                    // process rather than answering.
                    // (Two calls, not one rebinding: `Selector` is invariant in
                    // its lifetime, so the `'static` rewrite and this local
                    // parse cannot share a binding.)
                    fn runs(sel: &Selector<'_>, probe: &mut Vec<String>) -> bool {
                        classify_states(sel, probe).is_some()
                            && probe_runnable(sel)
                            && audit_selector(sel, probe)
                    }
                    if has::present(&sel_text(sel)) {
                        match has::rewrite(&sel_text(sel)).and_then(|t| reparse_selector(&t)) {
                            Some(s) => runs(s, &mut probe),
                            None => false,
                        }
                    } else {
                        runs(sel, &mut probe)
                    }
                }),
                _ => false,
            })
        }
        C::Unknown(_) => false,
    }
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
        PseudoClass::FocusWithin => STATE_FOCUS_WITHIN,
        _ => 0,
    }
}

/// An interaction state named on a compound that is NOT the selector's subject
/// — `.group:hover .child`, Tailwind's `group-hover:` variant.
///
/// The state belongs to an ANCESTOR, so it cannot ride the subject's own rung.
/// The descendant carries the rung keyed by that ancestor, and the renderer
/// refines it in while the ancestor is hovered (on the hover path) or, for
/// `:active`, through gpui's `.group(name)` / `.group_active(name, style)` —
/// no restyle, no cascade re-run (see `vn_tree::render`, and
/// research/reports/m18/live-test-round2.md §2).
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
pub(crate) fn classify_states(
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
        PE::Selection(_) => Some(PseudoElement::Selection),
        // `::-webkit-file-upload-button` is the same thing; lightningcss folds
        // the spellings into one variant here too.
        PE::FileSelectorButton(_) => Some(PseudoElement::FileSelectorButton),
        _ => None,
    }
}

/// Pseudo-elements with no box to style here, for a reason that is not a CSS
/// gap — so dropping their rules is exact rather than approximate.
///
/// Two families:
///
/// * `::backdrop` styles the layer behind an OPEN `<dialog>` or a fullscreen
///   element. vue-native has neither, and Tailwind's preflight writes a
///   `::backdrop` rule into every sheet, so this would fire on every app.
/// * The native form-control parts (`::-webkit-inner-spin-button`, …).
///   vue-native paints NO native control skin — that is the same fact
///   `appearance` rests on — so there is no spin button to style. The
///   divergence is the absence of those widgets, which is a forms question,
///   not a selector the cascade got wrong. `::file-selector-button` left
///   this family in m25: `<input type=file>` paints its button now, and the
///   rule refines it ([`ComputedStates::file_selector_button`]).
///
/// `::-webkit-scrollbar` used to be in the second family and is not any more:
/// there IS a scrollbar now (`vn_elements::scrollbar`), so a rule that restyles
/// it is a real gap rather than a rule about nothing, and it gets an advisory
/// naming the standard properties that do reach the painter.
///
/// `::first-line`, `::first-letter`, `::marker` and `::selection` deliberately
/// stay OUT of this list: each names something a browser really does paint over
/// content vue-native also has, so a rule using one is a real gap and still
/// says so.
fn never_matches_anywhere(
    pe: &<element::Impl as parcel_selectors::parser::SelectorImpl<'_>>::PseudoElement,
) -> bool {
    use lightningcss::selector::PseudoElement as PE;
    match pe {
        PE::Backdrop(_) => true,
        PE::WebKitScrollbar(_) => false,
        PE::Custom { name } => {
            let name = name.as_ref();
            name.starts_with("-webkit-") || name.starts_with("-moz-") || name.starts_with("-ms-")
        }
        _ => false,
    }
}

/// `::-webkit-scrollbar` and its eight parts (`-thumb`, `-track`, `-corner`, …).
fn is_webkit_scrollbar(
    pe: &<element::Impl as parcel_selectors::parser::SelectorImpl<'_>>::PseudoElement,
) -> bool {
    use lightningcss::selector::PseudoElement as PE;
    matches!(pe, PE::WebKitScrollbar(_))
}

pub(crate) fn audit_selector(sel: &Selector<'_>, diagnostics: &mut Vec<String>) -> bool {
    audit_components(sel, sel.iter_raw_match_order(), false, false, diagnostics)
}

/// [`audit_selector`] for a script's query against a view that answers the
/// LIVE interaction state ([`MatchDom::LIVE_STATE`]): a state inside
/// `:not()` / `:is()` / `:where()` is a plain yes or no there, so it is fine.
fn audit_live_selector(sel: &Selector<'_>, diagnostics: &mut Vec<String>) -> bool {
    audit_components(sel, sel.iter_raw_match_order(), false, true, diagnostics)
}

fn audit_components<'a, 'i>(
    sel: &Selector<'i>,
    components: impl Iterator<Item = &'a Component<'i, element::Impl>>,
    nested: bool,
    live: bool,
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
                if nested && !live && ladder_state(pc) != 0 {
                    // Outside a functional pseudo an interaction state is a
                    // ladder bucket; inside one the matcher has to answer it
                    // with a bare yes/no and always says "yes", which silently
                    // inverts `:not(:hover)`.
                    Some(format!("`:{}` inside :not()/:is()/:where()", pseudo_name(pc)))
                } else if ladder_state(pc) == 0 && !matches_from_props(pc) {
                    // A `-moz-`/`-webkit-`-prefixed pseudo-class is a browser's
                    // own; Tailwind's preflight carries `:-moz-focusring` and
                    // `:-moz-ui-invalid` in every sheet. Neither matches in the
                    // Chromium these apps are compared against, so dropping
                    // them here diverges from nothing.
                    let name = pseudo_name(pc);
                    if name.starts_with("-moz-") || name.starts_with("-webkit-") {
                        None
                    } else {
                        Some(format!("the pseudo-class `:{name}`"))
                    }
                } else {
                    None
                }
            }
            // `::before` / `::after` are REAL here: they generate a box the
            // renderer builds as a child of the originating element. Every
            // other pseudo-element still styles something that does not exist.
            Component::PseudoElement(pe) if !nested && pseudo_box(pe).is_some() => None,
            // `::backdrop` styles the layer behind an OPEN `<dialog>` or a
            // fullscreen element. vue-native has neither — no `<dialog>`
            // element and no element fullscreen — so the rule matches nothing
            // here AND nothing in the browser build of the same app. Dropping
            // it is not a divergence, so it is silent.
            Component::PseudoElement(pe) if !nested && never_matches_anywhere(pe) => None,
            // The scrollbar parts get an advisory of their own, because
            // unlike the rest of the family they name something that now
            // EXISTS — and the alternative is two standard properties rather
            // than "nothing here paints that".
            Component::PseudoElement(pe) if !nested && is_webkit_scrollbar(pe) => Some(
                "a `::-webkit-scrollbar*` pseudo-element — vue-native draws overlay scrollbars                  but does not expose them as styleable boxes; use the standard                  `scrollbar-width: auto | thin | none` and `scrollbar-color: <thumb> <track>`,                  which both reach the painter"
                    .into(),
            ),
            // One advisory for the whole family, not one per spelling: a
            // Tailwind preflight sheet names eight of them and eight lines
            // would bury everything else. The example selector rides along.
            Component::PseudoElement(_) => Some(
                "a pseudo-element other than `::before` / `::after` / `::placeholder` (nothing here paints part of a box separately from the box)"
                    .into(),
            ),
            // Shadow-DOM selectors need a shadow ROOT to match through, and
            // there is none in vue-native's tree — nor in the browser build,
            // where a Vue SFC creates no shadow root either. Tailwind's
            // preflight writes `html, :host { … }` in every sheet, and in a
            // browser that `:host` matches nothing too, so dropping it is exact
            // rather than approximate.
            Component::Slotted(_) | Component::Part(_) | Component::Host(_) => None,
            // `:scope` is answered by parcel itself: it compares the candidate
            // against `MatchingContext::scope_element`, and falls back to the
            // ROOT element when there is none. Both halves are the spec's:
            // Selectors-4 §3.5 makes `:scope` in a stylesheet — which has no
            // scoping root — match the document element, and `query_all_in`
            // sets the field for `el.querySelector(":scope > .x")`, where the
            // scoping root is `el`. Nothing here has to be dropped.
            Component::Scope => None,
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
                    if !audit_components(inner, inner.iter_raw_match_order(), true, live, diagnostics) {
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
    match pc {
        PseudoClass::Disabled | PseudoClass::Enabled | PseudoClass::Checked | PseudoClass::Fullscreen(_) => true,
        // The custom pseudo-class `crate::has`'s rewrite produces. It is
        // answered against the element's subtree by `El::match_non_ts_pseudo_class`,
        // so the audit must let it through — an unrecognised custom
        // pseudo-class from a stylesheet still cannot match and still says so.
        PseudoClass::Custom { name } => crate::has::is_marker(name.as_ref()),
        _ => false,
    }
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
pub(crate) fn probe_runnable(sel: &Selector<'_>) -> bool {
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

/// Re-parses one rewritten selector back into the `'static` form the rule
/// table holds. Leaks, like the sheet it stands in for — but once per distinct
/// text: every recompile of the same sheet rewrites to the same text
/// ([`has::rewrite`] files each `:has()` argument once), so it gets the same
/// selector back instead of leaking another one-rule sheet.
fn reparse_selector(text: &str) -> Option<&'static Selector<'static>> {
    if text.contains(['{', '}']) {
        return None;
    }
    if let Some(hit) = REPARSED.with(|r| r.borrow().get(text).copied()) {
        return hit;
    }
    let parsed = (|| {
        let source: &'static str = Box::leak(format!("{text}{{color:red}}").into_boxed_str());
        let sheet: &'static StyleSheet<'static> =
            Box::leak(Box::new(StyleSheet::parse(source, ParserOptions::default()).ok()?));
        match sheet.rules.0.first() {
            Some(CssRule::Style(style)) => style.selectors.0.first(),
            _ => None,
        }
    })();
    REPARSED.with(|r| r.borrow_mut().insert(text.to_string(), parsed));
    parsed
}

fn sel_text(sel: &Selector<'_>) -> String {
    use lightningcss::traits::ToCss;
    sel.to_css_string(lightningcss::printer::PrinterOptions::default())
        .unwrap_or_else(|_| "<selector>".into())
}

// ---------------------------------------------------------------------------
// rule provenance (devtools styles panel)
// ---------------------------------------------------------------------------

/// Whether [`Engine::compile`] keeps each rule's declarations as TEXT.
///
/// Off by default and read once per rule at compile time. A devtools host turns
/// it on at boot (before the app's sheet is installed) so the styles panel can
/// show a rule the way its author wrote it; a prod build never calls it and
/// therefore never allocates a single one of those strings.
static CAPTURE_RULE_TEXT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Turns rule-text capture on (or off) for every sheet compiled AFTER this
/// call — the sheet already installed keeps whatever it was compiled with.
pub fn set_capture_rule_text(on: bool) {
    CAPTURE_RULE_TEXT.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn capture_rule_text() -> bool {
    CAPTURE_RULE_TEXT.load(std::sync::atomic::Ordering::Relaxed)
}

/// One rule that MATCHED an element — the provenance a devtools styles panel
/// shows above the folded values: which selector, from which `@media` block, at
/// which rung of the state ladder, in cascade order.
///
/// The list [`Engine::matched_rules`] returns is sorted by
/// `(specificity, order)` — the cascade's own order — so a reader can fold it
/// front to back and the LAST declaration of a property is the one in effect,
/// exactly as the cascade did it.
#[derive(Debug, Clone)]
pub struct MatchedRule {
    /// The selector, printed back from the parsed form (so `:has()` shows the
    /// rewritten spelling the matcher actually ran).
    pub selector: String,
    pub specificity: u32,
    /// Position in the sheet — the cascade's tiebreak within one specificity.
    pub order: u32,
    /// The `@media` prelude this rule sits under, `and`-ed through nesting.
    pub media: Option<String>,
    /// `base` | `hover` | `active` | `focus` | `focus-visible` | `focus-within`
    /// — the bucket this rule folds into, i.e. which of `styles`' buckets its
    /// declarations landed in.
    pub state: &'static str,
    /// `.group:hover .this` — the state named by the selector belongs to an
    /// ANCESTOR, so the rule folds into the `group-*` bucket instead.
    pub ancestor_state: bool,
    /// `::before` / `::after` / `::placeholder` / `::selection` — the rule
    /// styles a generated box or a widget part, not the element itself.
    pub pseudo: Option<&'static str>,
    /// `(property, value, important)` in source order. Empty when the sheet was
    /// compiled with [`set_capture_rule_text`] off.
    pub declarations: Vec<(String, String, bool)>,
    /// What vn-css has to say about THIS rule's declarations — the advisories
    /// that normally reach the log the first time the rule matches something.
    /// The one thing a browser's styles panel cannot show and ours can: which
    /// of your declarations this engine does not implement.
    pub diagnostics: Vec<String>,
}

fn state_name(states: u8) -> &'static str {
    match states {
        0 => "base",
        s if s & STATE_ACTIVE != 0 => "active",
        s if s & STATE_HOVER != 0 => "hover",
        s if s & STATE_FOCUS_VISIBLE != 0 => "focus-visible",
        s if s & STATE_FOCUS != 0 => "focus",
        _ => "focus-within",
    }
}

impl Engine {
    /// Every rule in the sheet that matches `node` right now, in cascade order.
    ///
    /// This is [`Engine::compute_node`]'s matching half and nothing else: the
    /// same candidate set, the same `@media` gate, the same
    /// `matches_selector` call, the same `(specificity, order)` sort. It folds
    /// nothing — the fold is what `styles` already reports, and the point here
    /// is to show WHERE those values came from.
    pub fn matched_rules<D: MatchDom>(&self, node: D, env: &Env) -> Vec<MatchedRule> {
        let el = element::El(node);
        let mut matched: Vec<&RuleEntry> = Vec::new();
        self.candidates(&node, &mut |i, _| {
            let rule = &self.rules[i as usize];
            if let Some(c) = rule.condition {
                if !self.conditions[c as usize].eval(env) {
                    return;
                }
            }
            let mode = if rule.pseudo.is_some() {
                MatchingMode::ForStatelessPseudoElement
            } else {
                MatchingMode::Normal
            };
            let mut ctx: MatchingContext<'_, '_, element::Impl> =
                MatchingContext::new(mode, None, None, QuirksMode::NoQuirks);
            if matches_selector(&rule.selector, 0, None, &el, &mut ctx, &mut |_, _| {}) {
                matched.push(rule);
            }
        });
        matched.sort_by_key(|r| (r.specificity, r.order));
        matched
            .into_iter()
            .map(|r| MatchedRule {
                selector: sel_text(&r.selector),
                specificity: r.specificity,
                order: r.order,
                media: r.condition.map(|c| self.condition_text[c as usize].clone()),
                state: state_name(r.ancestor.map_or(r.states, |a| a.states)),
                ancestor_state: r.ancestor.is_some(),
                pseudo: r.pseudo.map(|p| match p {
                    PseudoElement::Before => "::before",
                    PseudoElement::After => "::after",
                    PseudoElement::Placeholder => "::placeholder",
                    PseudoElement::Selection => "::selection",
                    PseudoElement::FileSelectorButton => "::file-selector-button",
                }),
                declarations: r.decls.as_deref().cloned().unwrap_or_default(),
                diagnostics: r.diagnostics.as_deref().cloned().unwrap_or_default(),
            })
            .collect()
    }
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
/// The interaction pseudo-classes are the one place the query parts ways with
/// the cascade: when the view answers the live state ([`MatchDom::LIVE_STATE`],
/// vue-native's tree does), `:hover` / `:active` / `:focus` / `:focus-visible`
/// / `:focus-within` filter the query as a browser's do, and
/// [`classify_states`] (whose limits are gpui's state ladder's) is skipped.
///
/// Candidates rather than a root node because [`MatchDom`] deliberately has no
/// child accessor (the matcher never walks downwards); the caller owns the tree
/// and hands us its elements in document order.
///
/// `:scope` resolves to the ROOT element here — the same answer a stylesheet
/// gets. A `querySelector` call has a scoping root and must use
/// [`query_all_in`].
pub fn query_all<D: MatchDom>(selector: &str, candidates: &[D]) -> Result<QueryResult, String> {
    query_all_in(selector, candidates, None)
}

/// [`query_all`], with the element `:scope` names.
///
/// `el.querySelector(":scope > .x")` must anchor at `el`: the DOM gives every
/// `querySelector` / `querySelectorAll` / `matches` / `closest` call a *scoping
/// root*, which is the node the method was invoked on, and `:scope` is that
/// node (Selectors-4 §3.5, DOM §4.2.6 "scope-matching a selectors string").
/// Without it `:scope > div` asks whether the candidate is a child of the
/// DOCUMENT ROOT, which is a different question with a different answer — the
/// one leftover on the compat list that changed a result rather than describing
/// a rendering limit.
///
/// `scope` is `Option` because the cascade has no scoping root: a `:scope` in a
/// stylesheet matches the document element, which is what `parcel_selectors`
/// does when the field is `None`.
pub fn query_all_in<D: MatchDom>(
    selector: &str,
    candidates: &[D],
    scope: Option<D>,
) -> Result<QueryResult, String> {
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
            if !probe_runnable(s) {
                return Err(format!(
                    "selector rejected (matcher cannot run it — :has()/nesting?): {}",
                    sel_text(s)
                ));
            }
            // A view with the live state answers `:hover` / `:active` /
            // `:focus` / `:focus-visible` / `:focus-within` as a browser does,
            // anywhere in the selector: `.a:hover + .b`, `:not(:focus)` and
            // `:hover:focus` are plain conjunctions there. The limits
            // `classify_states` reports belong to gpui's state ladder, which a
            // query does not go through.
            if D::LIVE_STATE {
                if !audit_live_selector(s, &mut diagnostics) {
                    return Err(join_diagnostics(&diagnostics));
                }
                selectors.push(s);
                continue;
            }
            let (states, ancestor) = match classify_states(s, &mut diagnostics) {
                Some(pair) => pair,
                None => return Err(join_diagnostics(&diagnostics)),
            };
            if !audit_selector(s, &mut diagnostics) {
                return Err(join_diagnostics(&diagnostics));
            }
            if states != 0 || ancestor.is_some() {
                // A view with no live state (a test double, the compile-time
                // probe) can only answer an interaction pseudo the cascade's
                // way, "would match when in that state", so it cannot filter.
                diagnostics.push(format!(
                    "`{}` names an interaction state, and this element view has no live state: \
                     `:hover`/`:active`/`:focus`/`:focus-visible`/`:focus-within` match as \
                     \"would match when in that state\", so the query is NOT filtered by it",
                    sel_text(s)
                ));
            }
            selectors.push(s);
        }
    }
    if selectors.is_empty() {
        return Err(format!("`{sel}` matched no selector"));
    }
    // The scoping root's identity token, taken once. `El::opaque` is derived
    // from `MatchDom::key`, so it is the same token whichever `El` wrapper the
    // matcher happens to be holding when it asks.
    let scope_element = scope.map(|s| {
        use parcel_selectors::Element as _;
        element::El(s).opaque()
    });
    let mut matched = Vec::new();
    for (i, candidate) in candidates.iter().enumerate() {
        let el = element::El(*candidate);
        let hit = selectors.iter().any(|s| {
            let mut ctx: MatchingContext<'_, '_, element::Impl> =
                MatchingContext::new(MatchingMode::Normal, None, None, QuirksMode::NoQuirks);
            ctx.scope_element = scope_element;
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

    /// Recompiling the same text — what every viewport / rem / font basis move
    /// does — must retain nothing new: no re-parsed `:has()` selector, no
    /// `:has()` registry entry (perf-mem-1 §3: it used to leave ~21.5 MB per
    /// compile of the Argon client's sheet). The parse itself is not kept at
    /// all (`parse_author_sheet`). The caches are per-thread and each test runs
    /// on its own thread, so the counts are this test's alone.
    #[test]
    fn recompiling_the_same_text_retains_nothing_new() {
        let css = ".box { height: 10vh } .box:has(> .c) { opacity: 0.5 } \
                   @supports selector(:has(a)) { .box { width: 1px } }";
        let counts = || {
            (REPARSED.with(|r| r.borrow().len()), has::registry_len())
        };
        let first = Engine::compile(css);
        let after_first = counts();
        assert!(after_first.1 >= 1, "the :has() argument was filed");
        // (The basis itself is process-global and other tests read it, so it
        // is not moved here; what a basis move triggers is this same call.)
        for _ in 0..5 {
            let again = Engine::compile(css);
            assert_eq!(again.rules.len(), first.rules.len());
        }
        assert_eq!(counts(), after_first);
    }

    /// Several sheets compile as one cascade in order (a later `<style>` wins
    /// at equal specificity), each parsed on its own: a stray brace in one does
    /// not swallow the next.
    #[test]
    fn compile_sheets_is_one_ordered_cascade_of_separate_parses() {
        let engine = Engine::compile_sheets(&[".box { opacity: 0.1 }", ".box { opacity: 0.5 }"]);
        assert_eq!(engine.compute(box_node(), None, &Env::default()).base.opacity, Some(0.5));
        let engine = Engine::compile_sheets(&[".box { opacity: 0.1 } }", ".box { opacity: 0.6 }"]);
        assert_eq!(engine.compute(box_node(), None, &Env::default()).base.opacity, Some(0.6));
    }

    fn node_with(attrs: &'static [(&'static str, &'static str)]) -> Node {
        Node { tag: "div", class: "box", attrs }
    }

    fn transitions(css: &str) -> Vec<TransitionSpec> {
        let engine = Engine::compile(css);
        (*engine.compute(box_node(), None, &Env::default()).transitions).clone()
    }

    /// CSS error recovery: a stray `}` (the Argon client ships one after a
    /// `@keyframes`) costs what it costs in a browser, not the whole sheet. It
    /// becomes part of the NEXT rule's prelude, so that one rule is dropped,
    /// and parsing goes on after it.
    #[test]
    fn a_syntax_error_skips_one_rule_not_the_sheet() {
        let css = "@keyframes pulse { 0% { opacity: 0 } 100% { opacity: 1 } }\n\
                   }\n\
                   .eaten { opacity: 0 }\n\
                   .box { transition: opacity 200ms linear }";
        let engine = Engine::compile(css);
        assert!(
            !engine.diagnostics.iter().any(|d| d.starts_with("stylesheet parse error")),
            "the sheet failed as a whole: {:?}",
            engine.diagnostics
        );
        assert!(
            engine.diagnostics.iter().any(|d| d.contains("skipped, as a browser skips it")),
            "the skipped rule is named: {:?}",
            engine.diagnostics
        );
        assert!(engine.keyframes.contains_key("pulse"), "the @keyframes before the stray brace survives");
        assert_eq!(transitions(css).len(), 1, "parsing resumed: the rule after the damage applies");
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
        let said = advisories(".box:hover { transition: opacity 1s; opacity: 0.5 }");
        assert!(
            said.iter().any(|d| d.contains("transition declared on a state selector")),
            "{said:?}"
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

    /// Tailwind splits transition properties/easing and duration across utility
    /// classes. A duration-only rule overrides that component while preserving
    /// the earlier property list and timing curve, as CSS longhands require.
    #[test]
    fn tailwinds_transition_colors_plus_duration_300_preserves_properties_and_easing() {
        let t = transitions(
            ".box { /* .transition-colors */ \
               transition-property: color, background-color, border-color, \
                                    text-decoration-color, fill, stroke; \
               transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1); \
               transition-duration: 150ms } \
             .box { /* .duration-300 */ transition-duration: 300ms }",
        );
        assert_eq!(t.iter().map(|transition| transition.property.clone()).collect::<Vec<_>>(), vec![
            TransitionProperty::Color, TransitionProperty::BackgroundColor,
            TransitionProperty::BorderColor, TransitionProperty::TextDecorationColor,
            TransitionProperty::Fill, TransitionProperty::Stroke,
        ]);
        assert!(t.iter().all(|transition| transition.duration == Duration::from_millis(300)
            && transition.easing == Easing::CubicBezier(0.4, 0.0, 0.2, 1.0)));
        assert!(spec_for(&t, &TransitionProperty::BackgroundColor).is_some());
        assert!(spec_for(&t, &TransitionProperty::Width).is_none(), "the color-only utility must not start animating layout");
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
        // Every one of Tailwind's six names is a property the animator carries
        // — which is what took the last three "not animatable yet" lines out of
        // the stream (m22). `fill` and `stroke` animate the colour the SVG
        // sprite is tinted with, because that is the only colour it has here.
        assert_eq!(t[3].property, TransitionProperty::TextDecorationColor);
        assert_eq!(t[4].property, TransitionProperty::Fill);
        assert_eq!(t[5].property, TransitionProperty::Stroke);
        assert!(
            !t.iter().any(|s| matches!(s.property, TransitionProperty::Other(_))),
            "nothing in `transition-colors` should still be unrecognised: {t:?}",
        );
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

    /// A resize re-resolves the viewport rules in place, and the engine then
    /// cascades exactly as one compiled fresh under the new basis would.
    #[test]
    fn a_resize_rebases_the_viewport_rules_in_place() {
        assert!(set_viewport_basis(1000.0, 800.0));
        let css = ".box, .other { width: 90vw; color: red } .plain { width: 10px } \
                   .box { max-height: calc(85vh - 80px); transition: max-height 1s } \
                   .other { padding-top: 10vmin }";
        let mut engine = Engine::compile(css);
        assert!(uses_viewport_units());
        assert!(set_viewport_basis(1200.0, 600.0));
        assert!(engine.rebase_viewport());
        let fresh = Engine::compile(css);
        for class in ["box", "other", "plain"] {
            let node = Node { tag: "div", class, attrs: &[] };
            assert_eq!(
                engine.compute(node, None, &Env::default()),
                fresh.compute(node, None, &Env::default()),
                ".{class}"
            );
        }
        let s = engine.compute(box_node(), None, &Env::default());
        assert_eq!(s.base.size.width, Some(gpui::px(1080.0).into()));
        assert_eq!(s.base.max_size.height, Some(gpui::px(430.0).into()));
        assert_eq!(s.transitions.len(), 1, "the rebased rule keeps its transition");
        set_viewport_basis(INITIAL_VIEWPORT_BASIS.0, INITIAL_VIEWPORT_BASIS.1);
    }

    /// A viewport unit inside `@keyframes` is not kept for a rebase: the
    /// caller compiles the sheet again.
    #[test]
    fn a_viewport_unit_in_keyframes_asks_for_a_recompile() {
        let mut engine = Engine::compile("@keyframes grow { from { height: 50vh } to { height: 10px } }");
        assert!(uses_viewport_units());
        assert!(!engine.rebase_viewport());
        let mut plain = Engine::compile(".box { width: 10px }");
        assert!(plain.rebase_viewport(), "nothing to re-resolve is not a failure");
    }

    /// What a compiled engine says, one line per rule (and per index bucket,
    /// `@media` condition, `@keyframes` block, viewport block and advisory):
    /// the VALUES, never their layout, so a change to how the engine stores a
    /// rule can be checked against the engine before it on a real sheet.
    fn engine_digest(engine: &Engine) -> Vec<String> {
        let mut out = Vec::new();
        for r in &engine.rules {
            out.push(format!(
                "{} | {} {} {} {:?} {:?} {:?} | {:?} | {:?} | {:?} {:?} | {:?} {:?} | {:?} {:?} | {:?} | {:?} | \
                 {:?} {:?} | {:?} {:?} | {:?} | {:?}",
                sel_text(&r.selector),
                r.specificity,
                r.order,
                r.states,
                r.ancestor,
                r.pseudo,
                r.condition,
                r.normal,
                r.important,
                r.normal_transitions,
                r.important_transitions,
                r.normal_animations,
                r.important_animations,
                r.normal_animation_patch,
                r.important_animation_patch,
                r.normal_extras,
                r.important_extras,
                r.normal_custom,
                r.important_custom,
                r.normal_dynamic,
                r.important_dynamic,
                r.diagnostics,
                r.decls,
            ));
        }
        let mut buckets: Vec<String> = [
            ("id", &engine.index.by_id),
            ("class", &engine.index.by_class),
            ("tag", &engine.index.by_tag),
            ("parent id", &engine.index.by_parent_id),
            ("parent class", &engine.index.by_parent_class),
            ("parent tag", &engine.index.by_parent_tag),
        ]
        .into_iter()
        .flat_map(|(kind, map)| map.iter().map(move |(k, v)| format!("index {kind} {k}: {v:?}")))
        .collect();
        buckets.sort();
        out.extend(buckets);
        out.push(format!("index root: {:?}", engine.index.root));
        out.push(format!("index any: {:?} enabled {}", engine.index.any, engine.index.enabled));
        for (c, t) in engine.conditions.iter().zip(&engine.condition_text) {
            out.push(format!("media {t}: {c:?}"));
        }
        let mut names: Vec<&Box<str>> = engine.keyframes.keys().collect();
        names.sort();
        for name in names {
            for b in &engine.keyframes[name] {
                let steps: Vec<String> = b
                    .block
                    .steps
                    .iter()
                    .map(|k| format!("{:?} {:?} {:?} {:?}", k.offset, k.easing, k.style, k.dynamic))
                    .collect();
                out.push(format!("keyframes {name} {:?} {} {steps:?}", b.condition, b.block.dynamic));
            }
        }
        for b in &engine.viewport_blocks {
            out.push(format!("viewport {:?} {}", b.rules, b.css));
        }
        out.push(format!(
            "flags {} {} {}",
            engine.sibling_combinators, engine.group_states, engine.viewport_unkept
        ));
        out.extend(engine.diagnostics.iter().map(|d| format!("advisory {d}")));
        out
    }

    /// An engine change that should not change what the engine SAYS, checked
    /// on a real sheet: the first run (no digest file yet) writes one hash per
    /// line of [`engine_digest`], and every later run compares against it.
    ///
    /// ```text
    /// VN_SHEET=app.css VN_ENGINE_DIGEST=before.txt cargo test -p vn-css --lib engine_digest -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs VN_SHEET=<css file> and VN_ENGINE_DIGEST=<file>"]
    fn engine_digest_matches_the_recorded_one() {
        use std::hash::{Hash as _, Hasher as _};
        let (Ok(sheet), Ok(file)) = (std::env::var("VN_SHEET"), std::env::var("VN_ENGINE_DIGEST")) else { return };
        let css = std::fs::read_to_string(&sheet).expect("VN_SHEET is readable");
        set_viewport_basis(INITIAL_VIEWPORT_BASIS.0, INITIAL_VIEWPORT_BASIS.1);
        let lines = engine_digest(&Engine::compile(&css));
        let hashed: Vec<String> = lines
            .iter()
            .map(|l| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                l.hash(&mut h);
                format!("{:016x}", h.finish())
            })
            .collect();
        match std::fs::read_to_string(&file) {
            Err(_) => {
                std::fs::write(&file, hashed.join("\n")).expect("VN_ENGINE_DIGEST is writable");
                eprintln!("recorded {} lines to {file}", hashed.len());
            }
            Ok(recorded) => {
                let recorded: Vec<&str> = recorded.lines().collect();
                for (i, (a, b)) in recorded.iter().zip(&hashed).enumerate() {
                    assert_eq!(*a, b.as_str(), "line {i} differs; it now reads: {}", lines[i]);
                }
                assert_eq!(recorded.len(), hashed.len(), "line count");
                eprintln!("{} lines, all equal to {file}", hashed.len());
            }
        }
    }

    /// The same check on a real sheet, rule by rule: every rule a rebase
    /// touched holds what a fresh compile under the new basis holds, so the
    /// print-and-parse of [`ViewportBlock`] loses nothing the mapper reads.
    ///
    /// `VN_SHEET=path/to/app.css cargo test -p vn-css --lib rebase_matches -- --ignored`
    #[test]
    #[ignore = "needs VN_SHEET=<css file>"]
    fn rebase_matches_a_fresh_compile_on_a_real_sheet() {
        let Ok(path) = std::env::var("VN_SHEET") else { return };
        let css = std::fs::read_to_string(&path).expect("VN_SHEET is readable");
        set_viewport_basis(1000.0, 800.0);
        let mut engine = Engine::compile(&css);
        set_viewport_basis(1371.0, 655.0);
        assert!(engine.rebase_viewport());
        let fresh = Engine::compile(&css);
        assert_eq!(engine.rules.len(), fresh.rules.len());
        let touched: usize = engine.viewport_blocks.iter().map(|b| b.rules.len()).sum();
        for (i, (a, b)) in engine.rules.iter().zip(&fresh.rules).enumerate() {
            let same = a.normal == b.normal
                && a.important == b.important
                && a.normal_transitions == b.normal_transitions
                && a.important_transitions == b.important_transitions
                && a.normal_transition_declarations == b.normal_transition_declarations
                && a.important_transition_declarations == b.important_transition_declarations
                && a.normal_animations == b.normal_animations
                && a.important_animations == b.important_animations
                && a.normal_animation_patch == b.normal_animation_patch
                && a.important_animation_patch == b.important_animation_patch
                && a.normal_extras == b.normal_extras
                && a.important_extras == b.important_extras
                && a.normal_custom == b.normal_custom
                && a.important_custom == b.important_custom
                && a.normal_dynamic == b.normal_dynamic
                && a.important_dynamic == b.important_dynamic
                && a.normal_font == b.normal_font
                && a.important_font == b.important_font;
            assert!(same, "rule {i} ({}) differs after the rebase", sel_text(&a.selector));
        }
        eprintln!(
            "{} rules, {} viewport blocks filling {touched} of them: all equal to a fresh compile",
            engine.rules.len(),
            engine.viewport_blocks.len()
        );
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

    /// m26 #10 (b): `[hidden]` is `display: none` from the UA defaults, and
    /// any author `display` beats it whatever its specificity, as the UA
    /// cascade origin does in a browser.
    #[test]
    fn hidden_is_display_none_below_every_author_rule() {
        use gpui::Display::{Flex, Grid, None as Gone};
        let display = |css: &str, attrs: &'static [(&'static str, &'static str)]| {
            let engine = Engine::compile(css);
            assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
            engine.compute(node_with(attrs), None, &Env::default()).base.display
        };
        assert_eq!(display("", &[("hidden", "")]), Some(Gone));
        assert_eq!(display("", &[("hidden", "until-found")]), Some(Gone), "treated as hidden");
        assert_eq!(display("", &[]), None, "no attribute, no rule");
        assert_eq!(display("div { display: flex }", &[("hidden", "")]), Some(Flex), "a type selector");
        assert_eq!(display("* { display: grid }", &[("hidden", "")]), Some(Grid), "even `*`");
        assert_eq!(
            display(".box[hidden] { display: none } .box { display: flex }", &[("hidden", "")]),
            Some(Gone),
            "the author's own [hidden] rule is an ordinary rule"
        );
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
        let engine = Engine::compile("@media (prefers-contrast: more) { .box { opacity: 0.9 } }");
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
            (".box:required", "required"),
            (".box:placeholder-shown", "placeholder-shown"),
            (".box:target", "target"),
            (".box:lang(en)", "lang()"),
            (".box:indeterminate", "indeterminate"),
            // `::before` / `::after` / `::placeholder` DO compile now; the rest
            // of the family still cannot match anything.
            (".box::first-line", "pseudo-element"),
            // (`.box:scope` used to be here. It is answerable now — see
            // `scope_anchors_a_query_at_the_element_it_was_called_on`.)
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

    /// `:nth-child(… of S)` makes parcel's matcher `unreachable!()` — a
    /// process abort, not a failed match. `probe_runnable` only catches that
    /// when its dummy element happens to REACH the component (matching stops at
    /// the first failing one, and the dummy fails `.box` immediately), so the
    /// audit is what actually keeps it out.
    ///
    /// `:has()` used to be in this list and is not any more: the compiler
    /// rewrites it into a custom pseudo-class the matcher DOES route to the
    /// element, so the relational question is asked for real (`crate::has`).
    #[test]
    fn selectors_that_would_crash_the_matcher_are_rejected() {
        for selector in [".box:nth-child(1 of .other)"] {
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


    /// The advisories a sheet produces for `.box`, from the two places they can
    /// now come from: the sheet compile (selector-level) and the cascade
    /// (declaration-level, said when a rule matches).
    ///
    /// The split is m19's: a declaration advisory rides with its rule and is
    /// emitted by `compute_node`, so a generated sheet's unused utilities stay
    /// quiet. A test that wants to see one therefore has to MATCH the rule.
    fn advisories(css: &str) -> Vec<String> {
        let engine = Engine::compile(css);
        let mut out = engine.diagnostics.clone();
        engine.compute_node(
            box_node(),
            None,
            &Rc::new(Vars::default()),
            None,
            &Env::default(),
            &mut out,
        );
        out
    }

    /// A bad keyword must leave an advisory, never silently do nothing.
    #[test]
    fn bad_object_fit_value_is_advised() {
        let engine = Engine::compile(".box { object-fit: squish }");
        let said = advisories(".box { object-fit: squish }");
        assert!(said.iter().any(|d| d.contains("object-fit")), "{said:?}");
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
        let said = advisories(".box:hover { object-fit: cover }");
        assert!(
            said.iter().any(|d| d.contains("object-fit") && d.contains("unconditionally")),
            "{said:?}"
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
        let said = advisories(".box { window-drag: sideways }");
        assert!(
            said.iter().any(|d| d.contains("window-drag") && d.contains("sideways")),
            "{said:?}"
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
            None,
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
        let said = advisories(".box { --window-control: restore }");
        assert!(
            said.iter().any(|d| d.contains("--window-control") && d.contains("restore")),
            "{said:?}"
        );
    }

    /// The hit test reads an already-PAINTED frame, so a `:hover` drag region
    /// would be a paint race. It applies unconditionally and says so (A-12) —
    /// and the advisory names the property that was actually written.
    #[test]
    fn window_drag_on_a_state_selector_is_advised_by_name() {
        let engine = Engine::compile(".box:hover { window-drag: none }");
        let said = advisories(".box:hover { window-drag: none }");
        assert!(
            said.iter().any(|d| d.contains("`window-drag`") && d.contains("unconditionally")),
            "{said:?}"
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
            engine.compute_node(box_node(), None, &Rc::new(Vars::default()), None, env, &mut diags);
        (states.base, diags)
    }

    #[test]
    fn drawer_pseudo_inset_initial_and_variable_fallback_reset_to_auto() {
        let engine = Engine::compile(
            ".box::after { content: \"\"; inset: 12px } \
             [data-vaul-drawer][data-vaul-drawer-direction=bottom]::after { \
                 top: 100%; bottom: initial; left: 0; right: 0; height: 200% } \
             .box { bottom: 16px; bottom: var(--missing-edge, initial) }",
        );
        let mut diagnostics = engine.diagnostics.clone();
        let states = engine.compute_node(
            Node {
                tag: "div",
                class: "box",
                attrs: &[("data-vaul-drawer", ""), ("data-vaul-drawer-direction", "bottom")],
            },
            None,
            &Rc::new(Vars::default()),
            None,
            &Env::default(),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(states.base.inset.bottom, Some(gpui::Length::Auto));
        let after = states.after.expect("the drawer extension is generated");
        assert_eq!(after.style.inset.top, Some(gpui::relative(1.0).into()));
        assert_eq!(after.style.inset.bottom, Some(gpui::Length::Auto));
        assert_eq!(after.style.inset.left, Some(gpui::px(0.0).into()));
        assert_eq!(after.style.inset.right, Some(gpui::px(0.0).into()));
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

    /// Vaul moves a scaled background with a safe-area expression, rather
    /// than a literal pixel translation. The desktop env substitution must
    /// preserve the authored transform and its transition.
    #[test]
    fn drawer_background_transform_resolves_safe_area_calc() {
        let (style, diagnostics) = computed_with_env(
            ".box { transform: scale(0.96) translate3d(0, calc(env(safe-area-inset-top) + 14px), 0); \
             transition: transform .5s cubic-bezier(.32,.72,0,1) }",
            &Env::default(),
        );
        let transform = style.vn_transform.expect("the background transform is retained");
        assert_eq!(transform.translate.1, gpui::TransformLength::Pixels(14.0));
        assert!((transform.scale.0 - 0.96).abs() < 1e-6);
        assert!((transform.scale.1 - 0.96).abs() < 1e-6);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
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
        fn each_child(&self, f: &mut dyn FnMut(Self)) {
            for j in 1..self.doc.nodes.len() {
                if self.doc.nodes[j].2 == self.i {
                    f(DocRef { doc: self.doc, i: j });
                }
            }
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
    // the rule index
    // -----------------------------------------------------------------------

    /// Where `bucket_of` files each shape: under a key the subject must
    /// carry (a pseudo-element rule's ORIGINATING element included), under
    /// `:root`, under a key the subject's parent must carry (across sibling
    /// combinators, which share that parent), or in `any`.
    #[test]
    fn the_index_files_a_rule_where_its_subject_or_parent_must_carry_a_key() {
        let engine = Engine::compile(concat!(
            ".a::before { content: \"x\" }\n",
            ":root { --x: 1px }\n",
            ".p > :not([hidden]) ~ :not([hidden]) { margin-left: 1px }\n",
            "ul > * { margin-top: 1px }\n",
            ".p > * + * { margin-top: 2px }\n",
            ".a * { color: red }\n",
            "* { margin: 0 }\n",
            ".a ~ * { color: blue }\n",
            ":is(.x) > * { color: green }\n",
            "#i > * { color: black }\n",
            ".b::placeholder { color: gray }\n",
            "[data-x] { color: white }\n",
        ));
        let ix = &engine.index;
        // Selector text, not indices: the UA sheet's rules come first.
        let names = |v: Option<&Vec<u32>>| -> Vec<String> {
            v.map_or(vec![], |v| v.iter().map(|&i| sel_text(&engine.rules[i as usize].selector)).collect())
        };
        assert_eq!(names(ix.by_class.get("a")), [".a:before"]);
        assert_eq!(names(ix.by_class.get("b")), [".b::placeholder"]);
        assert_eq!(names(Some(&ix.root)), [":root"]);
        assert_eq!(names(ix.by_parent_class.get("p")), [".p > :not([hidden]) ~ :not([hidden])", ".p > * + *"]);
        assert_eq!(names(ix.by_parent_tag.get("ul")), ["ul > *"]);
        assert_eq!(names(ix.by_parent_id.get("i")), ["#i > *"]);
        // `:is(.x)` prints as `.x`, but the parent's class is inside `:is()`, which
        // guarantees nothing at the top level, so it stays in `any`.
        assert_eq!(names(Some(&ix.any)), [":where([hidden])", ".a *", "*", ".a ~ *", ".x > *", "[data-x]"]);
    }

    /// The index is a filter and nothing else: for every element, the rules
    /// it hands back and that match are exactly the rules that match when
    /// every rule is tried (`index.enabled = false`, `VN_CSS_NO_RULE_INDEX`).
    #[test]
    fn the_index_hands_back_exactly_what_matching_every_rule_would() {
        let css = concat!(
            ":root { --x: 1px }\n",
            ".app:root { --y: 1px }\n",
            ".a::before { content: \"a\" }\n",
            ".a:hover::after { content: \"h\" }\n",
            "::before { width: 1px }\n",
            ".p > :not([hidden]) ~ :not([hidden]) { margin-left: 1px }\n",
            ".p > * + * { margin-top: 1px }\n",
            ".p > *:first-child { margin-top: 2px }\n",
            "ul > * { padding: 1px }\n",
            "ul > li + li { padding: 2px }\n",
            "#i > * { color: black }\n",
            "#i b { color: gray }\n",
            ".a * { color: red }\n",
            ".app .p > span { color: green }\n",
            "* + span { color: blue }\n",
            "span ~ .a { color: white }\n",
            ":is(.p, ul) > * { margin: 3px }\n",
            ":not(.p) > span { margin: 4px }\n",
            "[hidden] { display: none }\n",
            "* { box-sizing: border-box }\n",
            ".missing > * { margin: 5px }\n",
        );
        let indexed = Engine::compile(css);
        let mut every = Engine::compile(css);
        every.index.enabled = false;
        let d = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "app", 0, vec![]),                 // 1, the root
                ("div", "p", 1, vec![]),                   // 2
                ("span", "", 2, vec![]),                   // 3
                ("span", "", 2, vec![("hidden", "")]),     // 4
                ("span", "a", 2, vec![]),                  // 5
                ("ul", "", 1, vec![]),                     // 6
                ("li", "", 6, vec![]),                     // 7
                ("li", "a", 6, vec![]),                    // 8
                ("div", "", 1, vec![("id", "i")]),         // 9
                ("b", "", 9, vec![]),                      // 10
                ("p", "a", 1, vec![]),                     // 11
                ("em", "", 11, vec![]),                    // 12
            ],
        };
        let env = Env::default();
        let selectors = |e: &Engine, n: DocRef<'_>| {
            let mut v: Vec<(u32, String)> = e.matched_rules(n, &env).into_iter().map(|r| (r.order, r.selector)).collect();
            v.sort();
            v
        };
        let mut matched_somewhere = 0;
        for n in refs(&d) {
            let want = selectors(&every, n);
            matched_somewhere += want.len();
            assert_eq!(selectors(&indexed, n), want, "node {}", n.i);
        }
        assert!(matched_somewhere > 30, "the sheet exercises the tree ({matched_somewhere} matches)");
    }

    // -----------------------------------------------------------------------
    // the matched-properties cache (`share`)
    // -----------------------------------------------------------------------

    /// Cascades `d` top-down as the restyle walk does, each element under its
    /// parent's custom properties and colour, once with the cache and once
    /// without. Answers both runs' states by node (slot 0 is unused).
    fn cascade_both(css: &str, d: &Doc, env: &Env) -> (Vec<ComputedStates>, Vec<ComputedStates>) {
        let cached = Engine::compile(css);
        let mut plain = Engine::compile(css);
        plain.share = false;
        assert!(cached.share, "VN_CSS_NO_SHARE is set in the test environment");
        let run = |e: &Engine| {
            let root_vars = Rc::new(Vars::default());
            let mut out = vec![ComputedStates::default()];
            for i in 1..d.nodes.len() {
                let p = d.nodes[i].2;
                let (vars, color) = if p == 0 { (root_vars.clone(), None) } else { (out[p].vars.clone(), out[p].color) };
                let mut diags = Vec::new();
                out.push(e.compute_node(DocRef { doc: d, i }, None, &vars, color, env, &mut diags));
            }
            out
        };
        let (with, without) = (run(&cached), run(&plain));
        for i in 1..d.nodes.len() {
            assert_eq!(with[i], without[i], "node {i} folds the same with the cache as without");
        }
        (with, without)
    }

    /// Did node `b` take node `a`'s fold? A fold allocates its own `Rc`s; a
    /// cache hit clones the kept ones.
    fn shared(s: &[ComputedStates], a: usize, b: usize) -> bool {
        Rc::ptr_eq(&s[a].base, &s[b].base)
    }

    /// Two message rows with the same structure: every element of the second
    /// takes the first one's fold, the grandchildren included, since a hit
    /// hands its parent's custom properties down by the same address.
    #[test]
    fn the_cache_folds_once_for_cousins() {
        let css = ".row { padding: 4px } .label { color: #ff0000; opacity: 0.5 } \
                   .meta { --gap: 3px } .meta > .t { margin-left: var(--gap); border-color: currentColor }";
        let d = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "list", 0, vec![]), // 1
                ("div", "row", 1, vec![]),  // 2
                ("span", "label", 2, vec![]),
                ("div", "meta", 2, vec![]),
                ("span", "t", 4, vec![]), // 5
                ("div", "row", 1, vec![]), // 6
                ("span", "label", 6, vec![]),
                ("div", "meta", 6, vec![]),
                ("span", "t", 8, vec![]), // 9
            ],
        };
        let (s, _) = cascade_both(css, &d, &Env::default());
        for (a, b) in [(2, 6), (3, 7), (4, 8), (5, 9)] {
            assert!(shared(&s, a, b), "node {b} takes node {a}'s fold");
        }
        assert_eq!(s[9].base.margin.left, Some(gpui::px(3.0).into()));
    }

    /// Each pair has the same tag and classes, and one thing a selector reads
    /// tells them apart: a position, an attribute, a sibling, a descendant, a
    /// child list. The match gives them different rules, so the cache never
    /// hands one the other's fold.
    #[test]
    fn the_cache_keeps_apart_what_a_selector_tells_apart() {
        type Nodes = Vec<(&'static str, &'static str, usize, Vec<(&'static str, &'static str)>)>;
        let pair = |a: Vec<(&'static str, &'static str)>, b: Vec<(&'static str, &'static str)>| -> Nodes {
            vec![("", "", 0, vec![]), ("div", "list", 0, vec![]), ("div", "i", 1, a), ("div", "i", 1, b)]
        };
        let cases: Vec<(&str, &str, Nodes)> = vec![
            ("position", ".i:first-child { opacity: 0.5 }", pair(vec![], vec![])),
            ("nth-child", ".i:nth-child(2n) { opacity: 0.5 }", pair(vec![], vec![])),
            ("attribute", ".i[data-state=open] { opacity: 0.5 }", pair(vec![("data-state", "open")], vec![])),
            ("attribute presence", ".i:not([hidden]) { opacity: 0.5 }", pair(vec![], vec![("hidden", "")])),
            ("sibling", ".i + .i { opacity: 0.5 }", pair(vec![], vec![])),
            ("later sibling", ".i ~ .i { opacity: 0.5 }", pair(vec![], vec![])),
            (
                ":has()",
                ".i:has(.badge) { opacity: 0.5 }",
                vec![
                    ("", "", 0, vec![]),
                    ("div", "list", 0, vec![]),
                    ("div", "i", 1, vec![]),
                    ("div", "i", 1, vec![]),
                    ("b", "badge", 3, vec![]),
                ],
            ),
            (
                ":empty",
                ".i:empty { opacity: 0.5 }",
                vec![
                    ("", "", 0, vec![]),
                    ("div", "list", 0, vec![]),
                    ("div", "i", 1, vec![]),
                    ("div", "i", 1, vec![]),
                    ("b", "", 3, vec![]),
                ],
            ),
        ];
        for (what, rule, nodes) in cases {
            let css = format!(".i {{ opacity: 0.9; padding: 2px }} {rule}");
            let d = Doc { nodes };
            let (s, _) = cascade_both(&css, &d, &Env::default());
            assert!(!shared(&s, 2, 3), "{what}: the second element folds on its own");
            assert_ne!(s[2].base.opacity, s[3].base.opacity, "{what}: and the two differ");
        }
    }

    /// The same rules under parents that hand down different custom properties
    /// or a different `color` fold differently, and the cache sees it.
    #[test]
    fn the_cache_keeps_apart_what_the_parent_hands_down() {
        let css = ".p1 { --w: 10px; color: #ff0000 } .p2 { --w: 20px; color: #0000ff } \
                   .c { width: var(--w); border-color: currentColor }";
        let d = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "list", 0, vec![]),
                ("div", "p1", 1, vec![]),
                ("div", "c", 2, vec![]), // 3
                ("div", "p2", 1, vec![]),
                ("div", "c", 4, vec![]), // 5
                ("div", "p1", 1, vec![]),
                ("div", "c", 6, vec![]), // 7
            ],
        };
        let (s, _) = cascade_both(css, &d, &Env::default());
        assert!(!shared(&s, 3, 5));
        assert_ne!(s[3].base.size.width, s[5].base.size.width);
        assert_ne!(s[3].base.border_color, s[5].base.border_color);
        assert!(shared(&s, 3, 7), "and the same parent inputs again meet");
    }

    /// An element with an inline style, or one styled by a `.group:hover`
    /// rule (whose bucket names the ancestor it watches), never asks.
    #[test]
    fn the_cache_leaves_out_inline_styles_and_group_rules() {
        let css = ".group:hover .c { opacity: 0.5 } .c { padding: 2px } .d { padding: 3px }";
        let d = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "list", 0, vec![]),
                ("div", "group", 1, vec![]),
                ("div", "c", 2, vec![]), // 3
                ("div", "group", 1, vec![]),
                ("div", "c", 4, vec![]), // 5
            ],
        };
        let (s, _) = cascade_both(css, &d, &Env::default());
        assert!(!shared(&s, 3, 5));
        assert_eq!(s[3].group_hover.as_ref().map(|g| g.0), Some(2));
        assert_eq!(s[5].group_hover.as_ref().map(|g| g.0), Some(4));

        let engine = Engine::compile(css);
        let inline = parse_inline_style("opacity: 0.25", &mut Vec::new());
        let vars = Rc::new(Vars::default());
        let fold = |i: usize| {
            engine.compute_node(DocRef { doc: &d, i }, Some(&inline), &vars, None, &Env::default(), &mut Vec::new())
        };
        let doc_d = Doc { nodes: vec![("", "", 0, vec![]), ("div", "d", 0, vec![]), ("div", "d", 0, vec![])] };
        let (a, b) = (
            engine.compute_node(DocRef { doc: &doc_d, i: 1 }, Some(&inline), &vars, None, &Env::default(), &mut Vec::new()),
            engine.compute_node(DocRef { doc: &doc_d, i: 2 }, Some(&inline), &vars, None, &Env::default(), &mut Vec::new()),
        );
        assert!(!Rc::ptr_eq(&a.base, &b.base), "an inline style folds on its own");
        assert_eq!(a, b);
        assert_eq!(fold(3).base.opacity, Some(0.25));
    }

    /// What a fold reads besides the rules and the parent: the environment
    /// (`env()`, here), the viewport after a rebase, and the rem basis a
    /// `var()`-composed length resolves against. The same element under a new
    /// one of each folds again.
    #[test]
    fn the_cache_does_not_outlive_what_the_fold_read() {
        let node = |class| Node { tag: "div", class, attrs: &[] };
        let vars = Rc::new(Vars::default());
        let fold = |e: &Engine, class, env: &Env| e.compute_node(node(class), None, &vars, None, env, &mut Vec::new());

        let engine = Engine::compile(".c { height: env(titlebar-area-height, 32px) }");
        let low = fold(&engine, "c", &titlebar_env(0.0, 0.0, 940.0, 36.0));
        let high = fold(&engine, "c", &titlebar_env(0.0, 0.0, 940.0, 48.0));
        assert_eq!(low.base.size.height, Some(gpui::px(36.0).into()));
        assert_eq!(high.base.size.height, Some(gpui::px(48.0).into()));

        assert!(set_viewport_basis(1000.0, 800.0) || viewport_basis() == (1000.0, 800.0));
        let mut engine = Engine::compile(".c { width: 50vw }");
        assert_eq!(fold(&engine, "c", &Env::default()).base.size.width, Some(gpui::px(500.0).into()));
        assert!(set_viewport_basis(1200.0, 600.0));
        assert!(engine.rebase_viewport());
        assert_eq!(fold(&engine, "c", &Env::default()).base.size.width, Some(gpui::px(600.0).into()));
        set_viewport_basis(INITIAL_VIEWPORT_BASIS.0, INITIAL_VIEWPORT_BASIS.1);

        let css = ".c { --w: 2rem; width: var(--w) }";
        let engine = Engine::compile(css);
        let before = fold(&engine, "c", &Env::default());
        let old = rem_basis();
        assert!(set_rem_basis(old + 4.0));
        let after = fold(&engine, "c", &Env::default());
        let mut plain = Engine::compile(css);
        plain.share = false;
        assert_eq!(after, fold(&plain, "c", &Env::default()), "a new rem basis folds again");
        assert_ne!(before.base.size.width, after.base.size.width);
        set_rem_basis(old);
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

    /// `::file-selector-button` styles the button `<input type=file>` paints,
    /// so it folds into its own refinement, box properties included. This is
    /// Tailwind's `file:` variant, verbatim.
    #[test]
    fn file_selector_button_folds_into_its_own_refinement() {
        let engine = Engine::compile(
            ".file\\:border-0::file-selector-button { border-width: 0px } \
             input::file-selector-button { background-color: rgb(1 2 3); padding-left: 12px }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = doc();
        let states = cascade(&engine, &d, &Env::default());
        let input = &states[3];
        let button = input.file_selector_button.as_ref().expect("the rule reaches the input's button");
        assert!(button.background.is_some());
        assert!(button.padding.left.is_some());
        assert!(input.base.background.is_none(), "the input itself is untouched");
        assert!(states[0].file_selector_button.is_none(), "only an element a rule matched gets one");
    }

    /// …and the devtools' rule list names it, as it names `::placeholder`.
    #[test]
    fn file_selector_button_rules_are_listed_with_their_pseudo() {
        let engine = Engine::compile("input::-webkit-file-upload-button { color: red }");
        assert!(engine.diagnostics.is_empty(), "the -webkit- spelling is the same pseudo: {:?}", engine.diagnostics);
        let d = doc();
        let rules = engine.matched_rules(DocRef { doc: &d, i: 4 }, &Env::default());
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert_eq!(rules[0].pseudo, Some("::file-selector-button"));
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

    /// Two `.tab`s at different depths, so `:scope > .tab` has a different
    /// answer from `.tab` and from a root-anchored `> .tab`.
    fn nested_doc() -> Doc {
        Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "page", 0, vec![]),  // 1 — the root
                ("div", "nav", 1, vec![]),   // 2 — the scoping root below
                ("a", "tab", 2, vec![]),     // 3 — a CHILD of .nav
                ("div", "inner", 2, vec![]), // 4
                ("a", "tab", 4, vec![]),     // 5 — a grandchild of .nav
            ],
        }
    }

    /// `el.querySelector(":scope > .x")` anchors at `el`, per Selectors-4 §3.5.
    /// This is the case that made `:scope` worth implementing: the two answers
    /// below differ, so dropping the selector was not "matches nothing", it was
    /// the WRONG set.
    #[test]
    fn scope_anchors_a_query_at_the_element_it_was_called_on() {
        let d = nested_doc();
        let all = refs(&d);
        // `.nav.querySelectorAll(...)` — candidates are .nav's descendants
        // (ids 3, 4, 5 → indices 0, 1, 2 of this slice), scope is .nav itself.
        let nav = all[1];
        let under_nav: Vec<DocRef<'_>> = all[2..].to_vec();

        // A CHILD combinator off the scope: only the direct child `.tab`.
        let r = query_all_in(":scope > .tab", &under_nav, Some(nav)).unwrap();
        assert_eq!(r.matched, vec![0], "only the .tab that is a child of .nav");

        // A DESCENDANT combinator off the scope: both.
        let r = query_all_in(":scope .tab", &under_nav, Some(nav)).unwrap();
        assert_eq!(r.matched, vec![0, 2]);

        // …and the answer really is different from the unscoped one, which
        // resolves `:scope` to the document element (`.page`, id 1). Neither
        // `.tab` is a child of `.page`.
        let r = query_all(":scope > .tab", &under_nav).unwrap();
        assert!(r.matched.is_empty(), "unscoped, `:scope` is the root");
    }

    /// The other two halves of the same field: a bare `:scope` is the scoping
    /// root itself (which is how `el.matches(":scope")` answers true), and a
    /// query with no scoping root falls back to the document element — which
    /// is what a `:scope` written in a STYLESHEET means.
    #[test]
    fn scope_with_no_scoping_root_is_the_document_element() {
        let d = nested_doc();
        let all = refs(&d);
        // `matches()` hands the matcher a one-element candidate list.
        let nav = all[1];
        assert_eq!(query_all_in(":scope", &[nav], Some(nav)).unwrap().matched, vec![0]);
        assert!(query_all_in(":scope", &[all[2]], Some(nav)).unwrap().matched.is_empty());
        // No scoping root: only `.page`, the root, answers to `:scope`.
        assert_eq!(query_all(":scope", &all).unwrap().matched, vec![0]);
    }

    /// A `:scope` rule in a stylesheet is no longer dropped: it compiles, and
    /// it lands on the root element, which is what a browser does with one.
    #[test]
    fn a_scope_rule_in_a_sheet_styles_the_root() {
        let engine = Engine::compile(":scope { background-color: rgb(1 2 3) }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let d = themed_doc();
        let states = cascade(&engine, &d, &Env::default());
        assert!(states[0].base.background.is_some(), "the root is styled");
        assert!(states[1].base.background.is_none(), "and nothing below it is");
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
                None,
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

    /// The EXOBIO overlay's notch: `fill: var(--notch-surface)` on an inline
    /// `<svg>`, the token declared on an ancestor, reaches the element as the
    /// concrete colour the serializer draws.
    #[test]
    fn an_svg_paint_resolves_through_var() {
        let engine = Engine::compile(
            ".page { --surface: #06080b } .btn { fill: var(--surface); stroke: rgba(219, 232, 255, 0.17) } \
             button { filter: drop-shadow(0 18px 30px rgba(0, 0, 0, 0.45)) } .page .btn { stroke-width: 1 }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let states = cascade(&engine, &themed_doc(), &Env::default());
        let fill = states[2].extras.fill.expect("fill computed");
        assert_eq!(fill.as_attr(), "rgb(6,8,11)");
        assert_eq!(states[2].extras.stroke.expect("stroke computed").as_attr(), "rgba(219,232,255,0.169)");
        // The `drop-shadow()` survives the fold of the rules after it (it was
        // dropped by `Extras::refine`, so no rule's shadow ever reached a node).
        let shadow = states[2].extras.drop_shadow.expect("drop-shadow computed");
        assert_eq!((shadow.dx, shadow.dy, shadow.blur), (0.0, 18.0, 30.0));
        // …and a later `filter` takes it away.
        let engine = Engine::compile(".btn { filter: drop-shadow(0 1px 2px #000) } .page .btn { filter: none }");
        let states = cascade(&engine, &themed_doc(), &Env::default());
        assert_eq!(states[2].extras.drop_shadow, None);
    }

    /// `Extras` have no state ladder, so a `:hover` paint would be drawn at
    /// rest: it is dropped, and said, and the base paint stays.
    #[test]
    fn a_state_rule_does_not_paint_an_svg_at_rest() {
        let engine = Engine::compile(
            ".btn { fill: #f00 } .btn:hover { fill: #00f; stroke-width: 3 } \
             .page:hover .btn { stroke: var(--x, #0f0) }",
        );
        let d = themed_doc();
        let mut diagnostics = Vec::new();
        let vars = Rc::new(Vars::default());
        let btn = engine.compute_node(DocRef { doc: &d, i: 3 }, None, &vars, None, &Env::default(), &mut diagnostics);
        assert_eq!(btn.extras.fill.map(|p| p.as_attr().into_owned()).as_deref(), Some("rgb(255,0,0)"));
        assert_eq!(btn.extras.svg_presentation, None);
        assert_eq!(btn.extras.stroke, None);
        assert!(diagnostics.iter().any(|m| m.contains("SVG presentation properties")), "{diagnostics:?}");
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
    fn an_unresolvable_var_drops_one_declaration_silently() {
        let engine = Engine::compile(".btn { opacity: 0.5; color: var(--nope) }");
        let d = themed_doc();
        let mut diagnostics = Vec::new();
        let s = engine.compute_node(
            DocRef { doc: &d, i: 3 },
            None,
            &Rc::new(Vars::default()),
            None,
            &Env::default(),
            &mut diagnostics,
        );
        assert_eq!(s.base.opacity, Some(0.5), "the sibling declaration survives");
        assert_eq!(s.base.text.color, None);
        // Silently, and correctly so: CSS says a declaration that is invalid at
        // computed-value time is thrown away, which is what happened, and
        // Tailwind ships the shape by design (`--tw-shadow-colored` reads a
        // `--tw-shadow-color` only a `shadow-<color>` utility ever sets).
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
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
            let s = engine.compute_node(DocRef { doc: &d, i }, None, &vars, None, &Env::default(), &mut diagnostics);
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

    /// [`DocRef`] with a live state: `hovered` and `focused` are node indices,
    /// focus within is the focused node's ancestors, and `keyboard` is the
    /// modality `:focus-visible` reads.
    #[derive(Clone, Copy)]
    struct LiveRef<'d> {
        r: DocRef<'d>,
        hovered: &'d [usize],
        focused: usize,
        keyboard: bool,
    }

    impl<'d> LiveRef<'d> {
        fn wrap(&self, r: DocRef<'d>) -> Self {
            LiveRef { r, ..*self }
        }
    }

    impl<'d> MatchDom for LiveRef<'d> {
        const LIVE_STATE: bool = true;
        fn interaction(&self, state: Interaction) -> Option<bool> {
            let i = self.r.i;
            let within = || {
                let mut at = self.focused;
                while at != 0 {
                    if at == i {
                        return true;
                    }
                    at = self.r.doc.nodes[at].2;
                }
                false
            };
            Some(match state {
                Interaction::Hover => self.hovered.contains(&i),
                Interaction::Active => false,
                Interaction::Focus => self.focused == i,
                Interaction::FocusVisible => self.focused == i && self.keyboard,
                Interaction::FocusWithin => within(),
            })
        }
        fn key(&self) -> u64 {
            self.r.key()
        }
        fn parent(&self) -> Option<Self> {
            self.r.parent().map(|r| self.wrap(r))
        }
        fn prev_sibling(&self) -> Option<Self> {
            self.r.prev_sibling().map(|r| self.wrap(r))
        }
        fn next_sibling(&self) -> Option<Self> {
            self.r.next_sibling().map(|r| self.wrap(r))
        }
        fn has_tag(&self, name: &str) -> bool {
            self.r.has_tag(name)
        }
        fn tag_eq(&self, other: &Self) -> bool {
            self.r.tag_eq(&other.r)
        }
        fn has_class(&self, name: &str) -> bool {
            self.r.has_class(name)
        }
        fn tag_name(&self) -> &str {
            self.r.tag_name()
        }
        fn each_class(&self, f: &mut dyn FnMut(&str)) {
            self.r.each_class(f)
        }
        fn has_scope(&self, name: &str) -> bool {
            self.r.has_scope(name)
        }
        fn attr(&self, name: &str) -> Option<&str> {
            self.r.attr(name)
        }
        fn is_empty(&self) -> bool {
            self.r.is_empty()
        }
        fn is_root(&self) -> bool {
            self.r.is_root()
        }
    }

    /// A view with the live state filters a query by it, as a browser's
    /// `querySelectorAll` / `matches` does, with no advisory; and the forms
    /// gpui's state ladder cannot take (a state inside `:not()`, a sibling's
    /// state, two states at once) are plain conjunctions there.
    #[test]
    fn a_live_view_filters_a_query_by_the_interaction_state() {
        fn live<'d>(d: &'d Doc, hovered: &'d [usize], focused: usize, keyboard: bool) -> Vec<LiveRef<'d>> {
            refs(d).into_iter().map(|r| LiveRef { r, hovered, focused, keyboard }).collect()
        }
        fn q(s: &str, c: &[LiveRef<'_>]) -> Result<(Vec<usize>, Vec<String>), String> {
            query_all(s, c).map(|r| (r.matched, r.warnings))
        }
        let d = doc();
        // nav (0) and the first tab (1) hovered; the second tab (2) focused by mouse.
        let c = live(&d, &[1, 2], 3, false);
        assert_eq!(q(".tab:hover", &c).unwrap(), (vec![1], vec![]));
        assert_eq!(q(":hover", &c).unwrap().0, vec![0, 1]);
        assert_eq!(q(":focus", &c).unwrap().0, vec![2]);
        assert!(q(":focus-visible", &c).unwrap().0.is_empty());
        assert_eq!(q(":focus-within", &c).unwrap().0, vec![0, 2]);
        assert!(q(":active", &c).unwrap().0.is_empty());
        assert_eq!(q(".tab:not(:hover)", &c).unwrap().0, vec![2]);
        assert_eq!(q(".tab:hover + .tab", &c).unwrap().0, vec![2]);
        assert_eq!(q(".nav:focus-within .tab:focus", &c).unwrap().0, vec![2]);
        assert!(q(".tab:hover:focus", &c).unwrap().0.is_empty());
        // The same focus by keyboard is visible.
        let k = live(&d, &[], 3, true);
        assert_eq!(q(":focus-visible", &k).unwrap().0, vec![2]);
        assert!(q(":hover", &k).unwrap().0.is_empty());
    }


    // ---- m19: :has(), :focus-within, @supports ---------------------------

    /// `:has()` is answered for real. The rule is rewritten into a custom
    /// pseudo-class before the matcher sees it (parcel's own `Component::Has`
    /// is an `unreachable!()`), and the relational question is then asked
    /// against the element's subtree.
    #[test]
    fn has_matches_a_descendant_for_real() {
        // doc: 0 = root, 1 = `.card` with a `.badge` under it, 2 = the badge,
        // 3 = a second `.card` with nothing under it.
        let doc = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "card", 0, vec![]),
                ("span", "badge", 1, vec![]),
                ("div", "card", 0, vec![]),
            ],
        };
        let engine = Engine::compile(".card:has(.badge) { opacity: 0.5 }");
        assert!(
            engine.diagnostics.is_empty(),
            "no advisory: it renders now — {:?}",
            engine.diagnostics
        );
        let opacity = |i: usize| {
            let mut d = Vec::new();
            engine
                .compute_node(
                    DocRef { doc: &doc, i },
                    None,
                    &Rc::new(Vars::default()),
                    None,
                    &Env::default(),
                    &mut d,
                )
                .base
                .opacity
        };
        assert_eq!(opacity(1), Some(0.5), "the card that HAS a badge");
        assert_eq!(opacity(3), None, "the card that does not");
        assert_eq!(opacity(2), None, "the badge itself is not the subject");
    }

    /// The child form only looks one level down, which is the whole point of
    /// writing it.
    #[test]
    fn has_with_a_child_combinator_does_not_reach_a_grandchild() {
        let doc = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "card", 0, vec![]),
                ("div", "wrap", 1, vec![]),
                ("span", "badge", 2, vec![]),
            ],
        };
        let direct = Engine::compile(".card:has(> .badge) { opacity: 0.5 }");
        let nested = Engine::compile(".card:has(.badge) { opacity: 0.5 }");
        let opacity = |engine: &Engine| {
            let mut d = Vec::new();
            engine
                .compute_node(
                    DocRef { doc: &doc, i: 1 },
                    None,
                    &Rc::new(Vars::default()),
                    None,
                    &Env::default(),
                    &mut d,
                )
                .base
                .opacity
        };
        assert_eq!(opacity(&direct), None, "the badge is a GRANDchild");
        assert_eq!(opacity(&nested), Some(0.5), "…but it is a descendant");
    }

    /// `:focus-within` is gpui's `.in_focus()` rung — focus anywhere in the
    /// element's focus subtree — so it folds into its own bucket rather than
    /// being dropped.
    #[test]
    fn focus_within_folds_into_its_own_rung() {
        let engine = Engine::compile(".box:focus-within { opacity: 0.5 }");
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let states = engine.compute(box_node(), None, &Env::default());
        assert_eq!(states.base.opacity, None, "not in the base state");
        assert_eq!(
            states.focus_within.as_ref().and_then(|f| f.opacity),
            Some(0.5)
        );
        assert!(states.needs_focus_handle(), "gpui needs a handle to key it on");
    }

    /// `@supports` is a question about the ENGINE, and the truthful answer is
    /// whatever the mapper does with the declaration in the prelude. A guarded
    /// block is then skipped or applied the way a browser missing the same
    /// feature would treat it — which is also why there is nothing to advise.
    #[test]
    fn supports_conditions_are_evaluated_rather_than_dropped() {
        let applies = |css: &str| {
            let engine = Engine::compile(css);
            assert!(engine.diagnostics.is_empty(), "{css}: {:?}", engine.diagnostics);
            engine.compute(box_node(), None, &Env::default()).base.opacity
        };
        assert_eq!(
            applies("@supports (display: flex) { .box { opacity: 0.5 } }"),
            Some(0.5),
            "a property vn-css maps"
        );
        assert_eq!(
            applies("@supports (view-transition-name: none) { .box { opacity: 0.5 } }"),
            None,
            "a property it does not"
        );
        assert_eq!(
            applies("@supports not (view-transition-name: none) { .box { opacity: 0.5 } }"),
            Some(0.5)
        );
        assert_eq!(
            applies(
                "@supports (display: flex) and (view-transition-name: none) \
                 { .box { opacity: 0.5 } }"
            ),
            None
        );
        assert_eq!(
            applies(
                "@supports (display: flex) or (view-transition-name: none) \
                 { .box { opacity: 0.5 } }"
            ),
            Some(0.5)
        );
        // `selector()` tracks the matcher, which now answers `:has()`.
        assert_eq!(
            applies("@supports selector(:has(a)) { .box { opacity: 0.5 } }"),
            Some(0.5)
        );
        assert_eq!(
            applies("@supports selector(::first-line) { .box { opacity: 0.5 } }"),
            None
        );
    }

    /// `currentColor` is a COMPUTED value — this element's `color` — so it is
    /// recorded during the fold and substituted after it, including when the
    /// colour is inherited from an ancestor rather than declared here.
    #[test]
    fn current_color_resolves_against_the_computed_color() {
        let engine = Engine::compile(
            ".parent { color: #ff0000 } .child { border-color: currentColor } \
             .own { color: #0000ff; border-color: currentColor }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let doc = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "parent", 0, vec![]),
                ("div", "child", 0, vec![]),
                ("div", "own", 0, vec![]),
            ],
        };
        let compute = |i: usize, parent_color: Option<gpui::Hsla>| {
            let mut d = Vec::new();
            let states = engine.compute_node(
                DocRef { doc: &doc, i },
                None,
                &Rc::new(Vars::default()),
                parent_color,
                &Env::default(),
                &mut d,
            );
            assert!(d.is_empty(), "{d:?}");
            states
        };
        let red = compute(1, None).color.expect("declared");
        // Inherited: the child declares no colour of its own.
        let inherited = compute(2, Some(red));
        assert_eq!(inherited.base.border_color, Some(red));
        assert_eq!(inherited.color, Some(red), "and it passes the same colour on");
        // Declared: its own `color` wins over the inherited one.
        let own = compute(3, Some(red));
        assert_ne!(own.base.border_color, Some(red));
        assert_eq!(own.base.border_color, own.color);
    }

    /// The profile card's layers round themselves with `border-radius:
    /// inherit` (Argon's UserProfilePopover): a background under `overflow:
    /// hidden`, and a glass zone and body that take the bottom corners down two
    /// levels. Dropping the keyword left all three square.
    #[test]
    fn border_radius_inherit_takes_the_parents_radius() {
        use gpui::{AbsoluteLength, DefiniteLength, VnCornerRadius};
        let engine = Engine::compile(
            ".card { border-radius: 16px } \
             .bg { border-radius: inherit } \
             .zone { border-bottom-left-radius: inherit; border-bottom-right-radius: inherit } \
             .body { border-bottom-left-radius: inherit; border-bottom-right-radius: inherit } \
             .pct { border-radius: 50% 4px } \
             .late { border-radius: inherit } .late.flat { border-radius: 2px } \
             .hov { border-radius: inherit } .hov:hover { border-radius: 8px } \
             .ring { border-radius: 10px } .ring::after { content: ''; border-radius: inherit }",
        );
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
        let doc = Doc {
            nodes: vec![
                ("", "", 0, vec![]),
                ("div", "card", 0, vec![]),
                ("div", "bg", 1, vec![]),
                ("div", "zone", 1, vec![]),
                ("div", "body", 3, vec![]),
                ("div", "pct", 0, vec![]),
                ("div", "bg", 5, vec![]),
                ("div", "late flat", 1, vec![]),
                ("div", "hov", 1, vec![]),
                ("div", "ring", 0, vec![]),
            ],
        };
        // The renderer's walk, parent first: each element resolves against
        // what its parent handed down.
        let mut handed: Vec<InheritedRadii> = vec![InheritedRadii::default(); doc.nodes.len()];
        let mut computed = Vec::new();
        for i in 1..doc.nodes.len() {
            let mut d = Vec::new();
            let mut states = engine.compute_node(
                DocRef { doc: &doc, i },
                None,
                &Rc::new(Vars::default()),
                None,
                &Env::default(),
                &mut d,
            );
            assert!(d.is_empty(), "{d:?}");
            let parent = doc.nodes[i].2;
            let parent = if parent == 0 { InheritedRadii::default() } else { handed[parent] };
            handed[i] = resolve_inherited_radii(&mut states, parent);
            computed.push(states);
        }
        let px = |v: f32| Some(AbsoluteLength::Pixels(gpui::px(v)));
        let at = |i: usize| &computed[i - 1];

        // Every corner of the background, from the card.
        let bg = &at(2).base;
        assert_eq!(
            [bg.corner_radii.top_left, bg.corner_radii.bottom_right],
            [px(16.), px(16.)]
        );
        assert_eq!(bg.vn_corner_radii.top_left, Some(None), "a length, not a percentage");
        // Two corners of the zone, and through it, the body's.
        let zone = &at(3).base;
        assert_eq!(zone.corner_radii.bottom_left, px(16.));
        assert_eq!(zone.corner_radii.top_left, None, "an undeclared corner stays undeclared");
        let body = &at(4).base;
        assert_eq!(
            [body.corner_radii.bottom_left, body.corner_radii.bottom_right],
            [px(16.), px(16.)]
        );
        // A percentage stays one, for the child to resolve against its own box.
        let from_pct = &at(6).base;
        assert_eq!(
            from_pct.vn_corner_radii.top_left,
            Some(Some(VnCornerRadius {
                x: DefiniteLength::Fraction(0.5),
                y: DefiniteLength::Fraction(0.5),
            }))
        );
        assert_eq!(from_pct.corner_radii.top_right, px(4.));
        assert_eq!(from_pct.vn_corner_radii.top_right, Some(None));
        // A later length wins over an earlier inherit.
        assert_eq!(at(7).base.corner_radii.top_left, px(2.));
        // A state rule neither cancels the base's inherit nor adds one.
        let hov = at(8);
        assert_eq!(hov.base.corner_radii.top_left, px(16.));
        assert_eq!(hov.hover.as_ref().unwrap().corner_radii.top_left, px(8.));
        // `::after` inherits from the element it belongs to.
        let ring = at(9).after.as_ref().expect("an ::after box");
        assert_eq!(ring.style.corner_radii.bottom_right, px(10.));
        // A parent with no radius hands down 0, which is CSS's initial value.
        let mut orphan = engine.compute_node(
            DocRef { doc: &doc, i: 2 },
            None,
            &Rc::new(Vars::default()),
            None,
            &Env::default(),
            &mut Vec::new(),
        );
        resolve_inherited_radii(&mut orphan, InheritedRadii::default());
        assert_eq!(orphan.base.corner_radii.top_left, px(0.));
    }

    #[test]
    fn border_radius_inherit_on_a_state_rule_is_advised() {
        let engine = Engine::compile(".x:hover { border-radius: inherit }");
        let doc = Doc { nodes: vec![("", "", 0, vec![]), ("div", "x", 0, vec![])] };
        let mut d = Vec::new();
        let states = engine.compute_node(
            DocRef { doc: &doc, i: 1 },
            None,
            &Rc::new(Vars::default()),
            None,
            &Env::default(),
            &mut d,
        );
        assert_eq!(states.extras.radius_inherit, RadiusInherit::default());
        assert!(d.iter().any(|m| m.contains("border-radius: inherit")), "{d:?}");
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
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));

        let base = StyleRefinement::default();
        let at = |ms: u64| a.overlay(1, &base, now + Duration::from_millis(ms)).unwrap();
        assert!((opacity(&at(0)) - 0.0).abs() < 1e-6);
        assert!((opacity(&at(500)) - 0.5).abs() < 1e-3);
        assert!((opacity(&at(999)) - 1.0).abs() < 1e-2);
    }

    /// **A keyframe AT 100 % is the end of the timeline, not a step before an
    /// implicit one.**
    ///
    /// `@keyframes x { to { … } }` is the commonest block there is, and with
    /// `animation-fill-mode: forwards` it used to SNAP BACK to the element's
    /// own value the instant it finished: at `progress == 1.0` the 100 %
    /// keyframe sorted as the segment's `from`, and the segment's `to` became
    /// the implicit 100 % — the element's value — at t = 1. CSS adds that
    /// implicit end only for a property no keyframe declares there.
    ///
    /// It survived because it only shows with a fill mode that paints the end:
    /// with `fill: none` the overlay is dropped at that same moment anyway.
    /// Found by the per-window `@keyframes` spec, which needed a SETTLED value
    /// to assert on (research/reports/m23/tails.md §2).
    #[test]
    fn a_100_percent_keyframe_holds_under_fill_forwards() {
        let engine = Engine::compile(
            ".box { animation: pulse 80ms linear 1 forwards }              @keyframes pulse { to { opacity: 0.2 } }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));

        let base = StyleRefinement::default();
        let at = |ms: u64| opacity(&a.overlay(1, &base, now + Duration::from_millis(ms)).unwrap());
        // The element's own value at 0, interpolating across the whole timeline.
        assert!((at(0) - 1.0).abs() < 1e-6);
        assert!((at(40) - 0.6).abs() < 1e-3);
        // …and it STAYS at the keyframe's value once finished, which is what
        // `forwards` means. This was 1.0.
        assert!((at(200) - 0.2).abs() < 1e-6, "forwards holds the 100 % keyframe");
        assert!((at(5000) - 0.2).abs() < 1e-6, "…and keeps holding it");
    }

    /// `fill-mode` is what decides whether the ends are painted at all — the
    /// difference between an element that snaps back and one that stays put.
    #[test]
    fn fill_mode_decides_what_the_ends_paint() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));
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
        a.on_computed_animations(1, &cs, now, &|name| forwards.keyframes(name, &Env::default()));
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
        a.on_computed_animations(1, &cs, now, &|name| backwards.keyframes(name, &Env::default()));
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
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));
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
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));
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
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));

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
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));
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
        a.on_computed_animations(1, &cs, now, &|name| engine.keyframes(name, &Env::default()));
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
        let frames = engine.keyframes("spin", &Env::default()).unwrap();
        assert_eq!(frames.steps.len(), 1);
        assert_eq!(frames.steps[0].offset, 1.0);
    }

    /// **A `@keyframes` block inside an `@media` one is declared per WINDOW.**
    ///
    /// The condition used to be thrown away at collection time: every
    /// `@keyframes` went into one by-name slot and the last one compiled won,
    /// whatever query it sat under. So a sheet that defines `slide`
    /// differently for a narrow viewport gave BOTH windows the narrow block
    /// (or the wide one — whichever came last in the file), and no
    /// environment could change it.
    ///
    /// Now the blocks are kept in document order with their condition, and the
    /// lookup picks the last one that HOLDS. With no `@media` around any of
    /// them that is exactly CSS's "a later block with the same name wins"
    /// (docs/MULTIWINDOW.md §6, research/reports/m23/tails.md §2).
    #[test]
    fn media_scoped_keyframes_resolve_per_environment() {
        let engine = Engine::compile(
            "@keyframes slide { to { opacity: 0.25 } }              @media (max-width: 420px) { @keyframes slide { to { opacity: 0.75 } } }",
        );
        let wide = Env { viewport_width: 1280.0, ..Env::default() };
        let narrow = Env { viewport_width: 380.0, ..Env::default() };

        let opacity = |env: &Env| {
            engine.keyframes("slide", env).map(|k| k.steps.last().unwrap().style.opacity.unwrap())
        };
        assert_eq!(opacity(&wide), Some(0.25), "the unconditional block, in a wide window");
        assert_eq!(
            opacity(&narrow),
            Some(0.75),
            "the @media block, in a window the query holds in — the SAME sheet, the same name"
        );

        // …and a name that only ever exists inside an `@media` block is simply
        // not there where the query does not hold, which is what CSS means by
        // an undeclared `animation-name`.
        let only_narrow = Engine::compile(
            "@media (max-width: 420px) { @keyframes pop { to { opacity: 0.5 } } }",
        );
        assert!(only_narrow.keyframes("pop", &narrow).is_some(), "declared in the narrow window");
        assert!(only_narrow.keyframes("pop", &wide).is_none(), "not declared in the wide one");

        // Document order still decides between two blocks that BOTH hold.
        let twice = Engine::compile(
            "@keyframes dup { to { opacity: 0.1 } } @keyframes dup { to { opacity: 0.9 } }",
        );
        assert_eq!(
            twice.keyframes("dup", &wide).map(|k| k.steps.last().unwrap().style.opacity.unwrap()),
            Some(0.9),
            "a later block with the same name wins, as in CSS"
        );
    }

    // -----------------------------------------------------------------------
    // m19 round 4: `var()` inside `@keyframes`
    //
    // A `@keyframes` block is document-scoped and a custom property belongs to
    // an element, so a var()-composed keyframe declaration can only resolve
    // when a run starts, against the animating element. Compiling it away made
    // the whole of `tailwindcss-animate` — `animate-in`, `animate-out`,
    // `fade-*`, `zoom-*`, `slide-*` — a correctly-timed identity.
    // -----------------------------------------------------------------------

    /// `tailwindcss-animate`, reproduced to the character: `enter` declares
    /// NOTHING but var()-composed values, and the utilities are pure custom
    /// properties on the element.
    const TW_ANIMATE: &str = "@keyframes enter { from { \
            opacity: var(--tw-enter-opacity, 1); \
            transform: translate3d(var(--tw-enter-translate-x, 0), var(--tw-enter-translate-y, 0), 0) \
                       scale3d(var(--tw-enter-scale, 1), var(--tw-enter-scale, 1), var(--tw-enter-scale, 1)) } } \
        .box { animation-name: enter; animation-duration: 200ms; animation-timing-function: linear; \
               --tw-enter-opacity: 0; --tw-enter-scale: 0.95 }";

    #[test]
    fn a_var_composed_keyframe_resolves_against_the_animating_element() {
        let engine = Engine::compile(TW_ANIMATE);
        // The block survives compilation instead of collapsing to zero steps.
        let raw = engine.keyframes("enter", &Env::default()).expect("@keyframes enter");
        assert_eq!(raw.steps.len(), 1, "the `from` step is kept, unresolved");
        assert!(raw.dynamic, "and it is flagged as needing per-element work");

        let cs = engine.compute(box_node(), None, &Env::default());
        assert_eq!(cs.vars.get("--tw-enter-opacity"), Some("0"));
        let mut diagnostics = Vec::new();
        let frames = engine
            .keyframes_for("enter", &cs.vars, &Env::default(), &mut diagnostics)
            .expect("resolved");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let from = &frames.steps[0].style;
        assert_eq!(from.opacity, Some(0.0), "var(--tw-enter-opacity) substituted");
        let t = from.vn_transform.expect("scale3d reached the refinement");
        assert!((t.scale.0 - 0.95).abs() < 1e-6, "{:?}", t.scale);
        assert!((t.scale.1 - 0.95).abs() < 1e-6, "{:?}", t.scale);

        // …and the animator interpolates it, which is the part the owner sees.
        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| {
            engine.keyframes_for(name, &cs.vars, &Env::default(), &mut Vec::new())
        });
        let base = StyleRefinement::default();
        let at = |ms: u64| a.overlay(1, &base, now + Duration::from_millis(ms)).unwrap();
        assert!((opacity(&at(0)) - 0.0).abs() < 1e-6);
        assert!((opacity(&at(100)) - 0.5).abs() < 1e-2, "halfway through the fade");
        let half = at(100).vn_transform.expect("the zoom is interpolated too");
        assert!((half.scale.0 - 0.975).abs() < 1e-2, "{:?}", half.scale);
    }

    /// The element's own variables decide the endpoints, so two elements
    /// sharing one `@keyframes` block animate differently — which is the whole
    /// reason the resolution cannot happen at compile time.
    #[test]
    fn two_elements_resolve_the_same_keyframes_differently() {
        let engine = Engine::compile(
            "@keyframes enter { from { opacity: var(--tw-enter-opacity, 1) } } \
             .box { animation: enter 200ms linear; --tw-enter-opacity: 0 } \
             .half { --tw-enter-opacity: 0.5 }",
        );
        let env = Env::default();
        let faint = engine.compute(box_node(), None, &env);
        let half = engine.compute(Node { tag: "div", class: "box half", attrs: &[] }, None, &env);
        let mut d = Vec::new();
        let a = engine.keyframes_for("enter", &faint.vars, &env, &mut d).unwrap();
        let b = engine.keyframes_for("enter", &half.vars, &env, &mut d).unwrap();
        assert_eq!(a.steps[0].style.opacity, Some(0.0));
        assert_eq!(b.steps[0].style.opacity, Some(0.5));
        assert!(d.is_empty(), "{d:?}");
    }

    /// A block with no `var()` in it is SHARED, not rebuilt per element: the
    /// resolver must cost nothing for the ordinary case.
    #[test]
    fn a_var_free_keyframes_block_takes_the_shared_fast_path() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        let shared = engine.keyframes("spin", &Env::default()).unwrap();
        assert!(!shared.dynamic);
        let resolved = engine
            .keyframes_for("spin", &cs.vars, &Env::default(), &mut Vec::new())
            .unwrap();
        assert!(Rc::ptr_eq(&shared, &resolved), "the same Rc, not a rebuild");
    }

    /// An unset variable with no fallback makes the DECLARATION invalid, as CSS
    /// says — the keyframe keeps its other declarations and the animation runs.
    #[test]
    fn an_unresolvable_keyframe_declaration_drops_only_itself() {
        let engine = Engine::compile(
            "@keyframes enter { from { opacity: var(--missing); width: 10px } } \
             .box { animation: enter 200ms linear }",
        );
        let env = Env::default();
        let cs = engine.compute(box_node(), None, &env);
        let mut d = Vec::new();
        let frames = engine.keyframes_for("enter", &cs.vars, &env, &mut d).unwrap();
        assert_eq!(frames.steps[0].style.opacity, None);
        assert!(frames.steps[0].style.size.width.is_some(), "the sibling survives");
        assert!(d.is_empty(), "an unset var() is silent — a browser drops it too");
    }

    /// The shape shadcn's dialog actually has: the keyframe RESETS `transform`
    /// to `translate3d(0,0,0)`, and the implicit end of a one-keyframe
    /// `@keyframes` is the element's own `translate(-50%,-50%)` centring. So
    /// the browser slides the box by half its own size while it zooms, and the
    /// two units — a pixel zero and a percentage — have to interpolate as one
    /// track. (Also the exact `rotate(0)` tailwindcss-animate writes, which is
    /// a unitless zero angle and has to parse.)
    #[test]
    fn the_shadcn_dialog_enter_slides_as_well_as_zooms() {
        let engine = Engine::compile(
            "@keyframes enter { from { \
                opacity: var(--tw-enter-opacity, 1); \
                transform: translate3d(var(--tw-enter-translate-x, 0), var(--tw-enter-translate-y, 0), 0) \
                           scale3d(var(--tw-enter-scale, 1), var(--tw-enter-scale, 1), var(--tw-enter-scale, 1)) \
                           rotate(var(--tw-enter-rotate, 0)) } } \
             .animate-in { animation: enter 200ms linear; \
                    --tw-enter-opacity: initial; --tw-enter-scale: initial; \
                    --tw-enter-rotate: initial; --tw-enter-translate-x: initial; \
                    --tw-enter-translate-y: initial } \
             .fade-in-0 { --tw-enter-opacity: 0 } .zoom-in-95 { --tw-enter-scale: 0.95 } \
             .box { transform: translate(-50%, -50%) }",
        );
        let env = Env::default();
        let node = Node { tag: "div", class: "box animate-in fade-in-0 zoom-in-95", attrs: &[] };
        let cs = engine.compute(node, None, &env);
        let mut d = Vec::new();
        let frames = engine.keyframes_for("enter", &cs.vars, &env, &mut d).unwrap();
        assert!(d.is_empty(), "`rotate(0)` and the rest must all parse: {d:?}");
        let start = frames.steps[0].style.vn_transform.expect("the keyframe's transform");
        assert_eq!(start.translate.0, gpui::TransformLength::Pixels(0.0));
        assert!((start.scale.0 - 0.95).abs() < 1e-6, "{:?}", start.scale);
        assert_eq!(frames.steps[0].style.opacity, Some(0.0));

        let mut a = Animator::new();
        let now = Instant::now();
        a.on_computed_animations(1, &cs, now, &|name| {
            engine.keyframes_for(name, &cs.vars, &env, &mut Vec::new())
        });
        a.tick(now);
        let half = a
            .overlay(1, &cs.base, now + Duration::from_millis(100))
            .unwrap()
            .vn_transform
            .expect("an interpolated transform");
        // Halfway: the centring offset is half applied, and so is the zoom.
        assert_eq!(half.translate.0, gpui::TransformLength::Fraction(-0.25));
        assert_eq!(half.translate.1, gpui::TransformLength::Fraction(-0.25));
        assert!((half.scale.0 - 0.975).abs() < 1e-4, "{:?}", half.scale);
    }

    /// An `animation-*` longhand is an ordinary declaration and loses to a
    /// MORE SPECIFIC rule that declares it, even when that rule reaches the
    /// element as the "list" half rather than as a patch. shadcn's dialog is
    /// exactly this collision: `duration-200` and
    /// `data-[state=open]:animate-in` both say `animation-duration`, and the
    /// second one wins in a browser.
    #[test]
    fn a_longhand_patch_loses_to_a_more_specific_rule_that_declares_the_list() {
        let engine = Engine::compile(
            "@keyframes enter { from { opacity: 0 } } \
             .duration-200 { animation-duration: 200ms } \
             .animate-in[data-state=open] { animation-name: enter; animation-duration: 150ms }",
        );
        let node = Node {
            tag: "div",
            class: "duration-200 animate-in",
            attrs: &[("data-state", "open")],
        };
        let cs = engine.compute(node, None, &Env::default());
        assert_eq!(cs.animations.len(), 1);
        assert_eq!(cs.animations[0].duration, Duration::from_millis(150));

        // The patch is not disabled, only ordered: a MORE specific longhand
        // still overrides the list it lost to nothing.
        let engine = Engine::compile(
            "@keyframes enter { from { opacity: 0 } } \
             .animate-in { animation-name: enter; animation-duration: 150ms } \
             .duration-200[data-state=open] { animation-duration: 200ms }",
        );
        let cs = engine.compute(node, None, &Env::default());
        assert_eq!(cs.animations[0].duration, Duration::from_millis(200));
    }

    #[test]
    fn animation_name_does_not_reset_separately_cascaded_component_lists() {
        let engine = Engine::compile(
            "@keyframes slide { from { transform: translateY(100%) } to { transform: translateY(0) } } \
             [data-drawer] { animation-duration: 500ms; animation-timing-function: linear; animation-delay: 20ms } \
             [data-drawer][data-state=open] { animation-name: slide }",
        );
        let node = Node { tag: "div", class: "", attrs: &[("data-drawer", ""), ("data-state", "open")] };
        let computed = engine.compute(node, None, &Env::default());
        assert_eq!(computed.animations[0].duration, Duration::from_millis(500));
        assert_eq!(computed.animations[0].delay, Duration::from_millis(20));
        assert_eq!(computed.animations[0].easing, Easing::Linear);
    }

    #[test]
    fn animation_component_lists_cycle_after_names_and_inline_longhands_cascade() {
        let engine = Engine::compile(
            ".box { animation-duration: 100ms, 250ms; animation-iteration-count: 2, 3; animation-delay: 30ms } \
             .box.on { animation-name: first, second, third }",
        );
        let node = Node { tag: "div", class: "box on", attrs: &[] };
        let computed = engine.compute(node, None, &Env::default());
        assert_eq!(computed.animations.iter().map(|animation| animation.duration).collect::<Vec<_>>(), vec![Duration::from_millis(100), Duration::from_millis(250), Duration::from_millis(100)]);
        assert_eq!(computed.animations.iter().map(|animation| animation.iterations).collect::<Vec<_>>(), vec![2.0, 3.0, 2.0]);
        let fold_inline = |source: &str| {
            let inline = parse_inline_style(source, &mut Vec::new());
            engine.compute_node(node, Some(&inline), &Rc::new(Vars::default()), None, &Env::default(), &mut Vec::new())
        };
        let computed = fold_inline("animation-duration: 400ms, 600ms; animation-fill-mode: forwards");
        assert_eq!(computed.animations.iter().map(|animation| animation.duration).collect::<Vec<_>>(), vec![Duration::from_millis(400), Duration::from_millis(600), Duration::from_millis(400)]);
        assert!(computed.animations.iter().all(|animation| animation.fill == FillMode::Forwards && animation.delay == Duration::from_millis(30)));
        let computed = fold_inline("animation: replacement 75ms ease-in");
        assert_eq!(computed.animations.len(), 1);
        assert_eq!(computed.animations[0].duration, Duration::from_millis(75));
        assert_eq!(computed.animations[0].delay, Duration::ZERO);
        assert_eq!(computed.animations[0].iterations, 1.0);
        assert_eq!(computed.animations[0].easing, Easing::EaseIn);
    }

    #[test]
    fn important_motion_longhands_override_normal_inline_but_preserve_other_components() {
        let engine = Engine::compile(".box { animation: slide 2s ease-in-out; transition: opacity 2s linear } .box { animation-duration: 10ms !important; animation-iteration-count: 1 !important; transition-duration: 12ms !important }");
        let fold = |source: &str| {
            let inline = parse_inline_style(source, &mut Vec::new());
            engine.compute_node(box_node(), Some(&inline), &Rc::new(Vars::default()), None, &Env::default(), &mut Vec::new())
        };
        let computed = fold("animation-duration: 500ms; animation-fill-mode: forwards; animation-iteration-count: 3; transition-property: transform, border-radius; transition-duration: 500ms; transition-timing-function: ease-in");
        assert_eq!(computed.animations[0].duration, Duration::from_millis(10));
        assert_eq!(computed.animations[0].iterations, 1.0);
        assert_eq!(computed.animations[0].fill, FillMode::Forwards);
        assert_eq!(computed.transitions.len(), 5);
        assert_eq!(computed.transitions[0].property, TransitionProperty::Transform);
        assert!(computed.transitions.iter().all(|transition| transition.duration == Duration::from_millis(12) && transition.easing == Easing::EaseIn));
        let computed = fold("animation-duration: 500ms; animation-duration: 75ms !important; transition-property: transform; transition-duration: 500ms; transition-duration: 80ms !important");
        assert_eq!(computed.animations[0].duration, Duration::from_millis(75));
        assert_eq!(computed.transitions[0].property, TransitionProperty::Transform);
        assert_eq!(computed.transitions[0].duration, Duration::from_millis(80));
        let legacy = Rc::new(vec![TransitionSpec { property: TransitionProperty::Width, duration: Duration::from_millis(900), delay: Duration::ZERO, easing: Easing::Linear }]);
        let computed = engine.compute_with(box_node(), None, Some(&legacy), &Env::default());
        assert_eq!(computed.transitions[0].property, TransitionProperty::Width);
        assert_eq!(computed.transitions[0].duration, Duration::from_millis(12));
    }

    #[test]
    fn important_motion_shorthand_resets_normal_inline_components_and_inline_important_wins() {
        let engine = Engine::compile(".box { animation: sheet 10ms linear !important; transition: transform 20ms ease-in !important }");
        let fold = |source: &str| {
            let inline = parse_inline_style(source, &mut Vec::new());
            engine.compute_node(box_node(), Some(&inline), &Rc::new(Vars::default()), None, &Env::default(), &mut Vec::new())
        };
        let computed = fold("animation: normal 500ms ease-out; animation-fill-mode: forwards; transition: opacity 500ms linear");
        assert_eq!(&*computed.animations[0].name, "sheet");
        assert_eq!(computed.animations[0].duration, Duration::from_millis(10));
        assert_eq!(computed.animations[0].fill, FillMode::None);
        assert_eq!(computed.transitions[0].property, TransitionProperty::Transform);
        assert_eq!(computed.transitions[0].duration, Duration::from_millis(20));
        let computed = fold("animation: own 75ms ease-out !important; animation-duration: 900ms; transition: opacity 80ms linear !important; transition-duration: 900ms");
        assert_eq!(&*computed.animations[0].name, "own");
        assert_eq!(computed.animations[0].duration, Duration::from_millis(75));
        assert_eq!(computed.transitions[0].property, TransitionProperty::Opacity);
        assert_eq!(computed.transitions[0].duration, Duration::from_millis(80));
    }

    /// A run's clock is anchored to the first FRAME, not to the cascade that
    /// created it. `getComputedStyle` forces a cascade mid-wave, so a dialog's
    /// keyframes can be resolved a whole mount before anything paints — and a
    /// 150 ms animation whose clock started there would be nearly over by the
    /// time it was first drawn.
    #[test]
    fn an_animations_clock_starts_at_the_first_frame() {
        let engine = Engine::compile(SPIN);
        let cs = engine.compute(box_node(), None, &Env::default());
        let mut a = Animator::new();
        let created = Instant::now();
        a.on_computed_animations(1, &cs, created, &|name| engine.keyframes(name, &Env::default()));

        // 300 ms of mount work happen before the first frame renders.
        let first_frame = created + Duration::from_millis(300);
        a.tick(first_frame);
        let base = StyleRefinement::default();
        let halfway = first_frame + Duration::from_millis(500);
        assert!(
            (opacity(&a.overlay(1, &base, first_frame).unwrap()) - 0.0).abs() < 1e-6,
            "the first painted frame is the animation's frame 0"
        );
        assert!((opacity(&a.overlay(1, &base, halfway).unwrap()) - 0.5).abs() < 1e-2);
        // …and the re-anchor happens once: a later tick does not restart it.
        a.tick(halfway);
        assert!((opacity(&a.overlay(1, &base, halfway).unwrap()) - 0.5).abs() < 1e-2);
    }

    /// `scale3d(s, s, s)` is what every `zoom-in-*` / `zoom-out-*` compiles to.
    /// It decomposes to the 2D scale instead of being dropped whole.
    #[test]
    fn scale3d_decomposes_to_the_two_axes_that_exist() {
        let (r, d) = computed_with_env(".box { transform: scale3d(0.95, 0.8, 1) }", &Env::default());
        let t = r.vn_transform.expect("a transform");
        assert!((t.scale.0 - 0.95).abs() < 1e-6);
        assert!((t.scale.1 - 0.8).abs() < 1e-6);
        assert!(!d.iter().any(|m| m.contains("scale3d")), "{d:?}");
        // The Z factor is invisible without a perspective projection, which is
        // reported on its own line, so it is dropped in silence — a `zoom-in-95`
        // must not cost a log line per screen.
        let (_, d) = computed_with_env(".box { transform: scale3d(1, 1, 2) }", &Env::default());
        assert!(!d.iter().any(|m| m.contains("transform")), "{d:?}");
    }

    // -----------------------------------------------------------------------
    // m19: transitions driven by an INTERACTION-STATE flip
    //
    // In a browser a `transition` on the base rule animates whatever a `:hover`
    // rule changes. Our hover rung rides gpui's paint-time ladder, which has no
    // clock, so the value snapped. These cover the two halves of the fix: the
    // rung is split (transitioned properties out, the rest left to snap), and
    // the animator interpolates the half it took.
    // -----------------------------------------------------------------------

    /// The split itself: `transition-property` decides what leaves the rung.
    #[test]
    fn a_hover_rung_gives_up_exactly_its_transitioned_properties() {
        let engine = Engine::compile(
            ".box { transition: background-color 200ms linear; background: #000 } \
             .box:hover { background: #fff; padding-left: 8px }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        let rung = cs.hover.as_ref().expect("a hover rung");
        assert!(rung.background.is_some());
        assert!(rung.padding.left.is_some());

        let split = crate::strip_transitioned(rung, &cs.transitions).expect("something to strip");
        // Transitioned: the animator's now.
        assert!(split.background.is_none());
        // Not named in `transition`: still gpui's, still instant — CSS agrees.
        assert!(split.padding.left.is_some());
    }

    /// Nothing is stripped when the author declared no transition, so an app
    /// that never uses `transition` keeps the exact rung it had before.
    #[test]
    fn a_rung_without_a_transition_is_left_alone() {
        let engine = Engine::compile(".box { background: #000 } .box:hover { background: #fff }");
        let cs = engine.compute(box_node(), None, &Env::default());
        let rung = cs.hover.as_ref().unwrap();
        assert!(crate::strip_transitioned(rung, &cs.transitions).is_none());
    }

    /// The fold the animator aims at has to match what gpui's ladder would
    /// produce: base, then group-hover, then hover (`compute_style_internal`).
    #[test]
    fn the_hover_fold_follows_gpuis_own_rung_order() {
        let engine = Engine::compile(
            ".group:hover .icon { opacity: 0.5 } .icon:hover { opacity: 0.9 } \
             .icon { opacity: 0.1 }",
        );
        let d = group_doc();
        let states = cascade(&engine, &d, &Env::default());
        let icon = &states[2];
        assert!(icon.hovered(false, false).is_none(), "base state is not a fold");
        assert_eq!(icon.hovered(false, true).unwrap().opacity, Some(0.5));
        assert_eq!(icon.hovered(true, false).unwrap().opacity, Some(0.9));
        // Hover wins over group-hover with both on: gpui refines hover LAST.
        assert_eq!(icon.hovered(true, true).unwrap().opacity, Some(0.9));
    }

    /// The end-to-end behaviour the owner asked for: a hover flip INTERPOLATES,
    /// and the settled value survives the track being dropped.
    #[test]
    fn a_hover_flip_interpolates_and_then_holds() {
        use crate::animator::Animator;
        let engine = Engine::compile(
            ".box { transition: opacity 200ms linear; opacity: 0 } .box:hover { opacity: 1 }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        let base = (*cs.base).clone();
        let mut a = Animator::new();
        let now = Instant::now();

        // Pointer in.
        let target = cs.hovered(true, false);
        a.on_state(1, &cs, target.as_ref(), now);
        let half = a.overlay(1, &base, now + Duration::from_millis(100)).unwrap();
        assert!(
            (half.opacity.unwrap() - 0.5).abs() < 1e-2,
            "midway through, not snapped: {:?}",
            half.opacity
        );

        // Settled, and the track is dropped — the HOLD is what keeps it there,
        // because the rung no longer carries the value.
        let late = now + Duration::from_millis(400);
        a.finish_settled(late);
        assert!(!a.any_active(late));
        let held = a.overlay(1, &base, late).unwrap();
        assert_eq!(held.opacity, Some(1.0));

        // Pointer out: back down, from the held value.
        a.on_state(1, &cs, None, late);
        let back = a.overlay(1, &base, late + Duration::from_millis(100)).unwrap();
        assert!(
            (back.opacity.unwrap() - 0.5).abs() < 1e-2,
            "reverses: {:?}",
            back.opacity
        );
        // …and once it lands, nothing is held: the base is the answer again.
        let done = late + Duration::from_millis(400);
        a.finish_settled(done);
        assert!(!a.holds_state(1));
    }

    /// `prefers-reduced-motion` still APPLIES the state style — it only refuses
    /// to animate it. Without the hold the rung would have been stripped and
    /// nothing would paint the hovered value at all.
    #[test]
    fn reduced_motion_snaps_a_state_flip_rather_than_dropping_it() {
        use crate::animator::Animator;
        let engine = Engine::compile(
            ".box { transition: opacity 200ms linear; opacity: 0 } .box:hover { opacity: 1 }",
        );
        let cs = engine.compute(box_node(), None, &Env::default());
        let base = (*cs.base).clone();
        let mut a = Animator::new();
        a.set_reduce_motion(true);
        let now = Instant::now();
        a.on_state(1, &cs, cs.hovered(true, false).as_ref(), now);
        assert!(!a.any_active(now));
        assert_eq!(a.overlay(1, &base, now).unwrap().opacity, Some(1.0));
    }

    // -----------------------------------------------------------------------
    // rule provenance (devtools — docs/DEVTOOLS-UI.md §4)
    // -----------------------------------------------------------------------

    /// The devtools styles panel folds this list front-to-back to decide which
    /// declarations were overridden, so the ORDER is load-bearing: it has to be
    /// the cascade's own `(specificity, order)`, and each rule has to say which
    /// bucket it folds into or a `:hover` declaration would look like it lost
    /// to a base one.
    #[test]
    fn matched_rules_report_the_cascade_in_its_own_order() {
        set_capture_rule_text(true);
        let engine = Engine::compile(
            r#"@media (min-width: 100px) { .tab { color: #f00 } }
               a { color: #001 }
               .tab { color: #002; padding: 4px }
               .tab:hover { color: #003 }
               .tab::after { content: "x" }"#,
        );
        set_capture_rule_text(false);
        let d = doc();
        // nodes[2] is `a.tab.router-link-active`.
        let rules = engine.matched_rules(DocRef { doc: &d, i: 2 }, &Env::default());

        let seen: Vec<(&str, &str)> = rules.iter().map(|r| (r.selector.as_str(), r.state)).collect();
        assert_eq!(
            seen,
            vec![
                // (0,0,1) — a tag.
                ("a", "base"),
                // (0,1,0), and the `@media` block was walked first, so it took
                // the lower source order.
                (".tab", "base"),
                (".tab", "base"),
                // (0,1,1) — a class plus a pseudo-ELEMENT, which counts as an
                // element and therefore sorts under…
                (".tab:after", "base"),
                // …(0,2,0) — a pseudo-CLASS counts as a class.
                (".tab:hover", "hover"),
            ],
            "ascending (specificity, order) — the cascade's own order"
        );
        // Sorted by (specificity, order), the way the cascade folds.
        let keys: Vec<(u32, u32)> = rules.iter().map(|r| (r.specificity, r.order)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);

        // The `@media` prelude rides along, so a panel can say WHY a rule is
        // (or is not) in the list.
        let media: Vec<Option<&str>> = rules.iter().map(|r| r.media.as_deref()).collect();
        // lightningcss prints a prelude back in RANGE syntax, which is the same
        // query — the panel shows what the engine understood, not the bytes.
        assert_eq!(media[1], Some("(width >= 100px)"));
        assert_eq!(media[2], None);
        // …and the pseudo-element rule is flagged rather than looking like a
        // rule on the element itself.
        assert_eq!(rules[3].pseudo, Some("::after"));

        // Declarations are the author's own text, in source order.
        let tab = &rules[2];
        assert_eq!(
            tab.declarations,
            vec![
                ("color".to_string(), "#002".to_string(), false),
                ("padding".to_string(), "4px".to_string(), false),
            ]
        );
    }

    /// Declaration text is captured only when the host asks for it — a prod
    /// build must not pay one `String` per declaration in the sheet for a panel
    /// it does not have.
    ///
    /// Both halves live in ONE test because the flag is process-global and
    /// `cargo test` runs these in parallel: two tests toggling it would race
    /// each other rather than test it.
    #[test]
    fn rule_text_is_captured_only_when_asked_for() {
        let d = doc();
        let matched = |engine: &Engine| engine.matched_rules(DocRef { doc: &d, i: 2 }, &Env::default());

        set_capture_rule_text(false);
        let off = Engine::compile(".tab { color: #002 }");
        let rules = matched(&off);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].declarations.is_empty());
        // The selector and its place in the cascade are exact either way — the
        // panel degrades to "which rules matched", not to nothing.
        assert_eq!(rules[0].selector, ".tab");

        set_capture_rule_text(true);
        let on = Engine::compile(".tab { color: #002 }");
        set_capture_rule_text(false);
        assert_eq!(matched(&on)[0].declarations.len(), 1);
        // …and the sheet compiled BEFORE the flag flipped keeps what it was
        // compiled with: capture is a property of the sheet, not of the reader.
        assert!(matched(&off)[0].declarations.is_empty());
    }
    #[test]
    fn border_currentcolor_cascades_per_side_without_overwriting_literal_edges() {
        let engine = Engine::compile(".box { color:#0000ff; border:1px solid currentColor } .box { border-left-color:#ff0000 }");
        let style=engine.compute(box_node(), None, &Env::default());
        assert_eq!(style.base.vn_border_colors.top, Some(Some(gpui::rgb(0x0000ff).into())));
        assert_eq!(style.base.vn_border_colors.left, Some(Some(gpui::rgb(0xff0000).into())));
        let engine=Engine::compile(".box { color:#0000ff; border:1px solid #ff0000 } .box { border-left-color:currentColor }");
        let style=engine.compute(box_node(), None, &Env::default());
        assert_eq!(style.base.vn_border_colors.top, Some(Some(gpui::rgb(0xff0000).into())));
        assert_eq!(style.base.vn_border_colors.left, Some(Some(gpui::rgb(0x0000ff).into())));
    }

    #[test]
    fn scrollbar_color_uses_theme_variables_and_transparent_track() {
        let engine=Engine::compile(".box { --muted-foreground:240 5% 64%; scrollbar-color:hsl(var(--muted-foreground) / .35) transparent }");
        let style=engine.compute(box_node(), None, &Env::default());
        let (thumb,track)=style.extras.scrollbar_color.expect("theme colour pair is retained");
        assert!((thumb.a-0.35).abs()<0.01, "{thumb:?}");
        assert!(track.is_transparent());
        assert!(engine.diagnostics.is_empty(), "{:?}", engine.diagnostics);
    }

    fn test_font_metrics() -> impl Drop {
        set_font_source(|| vec!["Arial".into(), "VN Metric Narrow".into(), "VN Metric Wide".into()]);
        set_font_metric_source(|font, size| {
            let ratio = match font.family.as_ref() {
                "VN Metric Narrow" => if font.weight == gpui::FontWeight::BOLD { 0.6 } else { 0.55 },
                "VN Metric Wide" => 0.8,
                _ => return None,
            };
            Some(FontRelativeMetrics { zero_advance: ratio * size, x_height: 0.4 * size })
        });
        struct Reset;
        impl Drop for Reset { fn drop(&mut self) { set_font_metric_source(|_, _| None); } }
        Reset
    }

    #[test]
    fn character_units_use_the_computed_face_weight_size_and_hover_face() {
        let _metrics = test_font_metrics();
        let engine = Engine::compile(".box { width:1ch; height:2ex; font-family:'VN Metric Narrow'; font-weight:bold; font-size:9px } .box:hover {font-family:'VN Metric Wide'; font-size:12px}");
        let computed = engine.compute(box_node(), None, &Env::default());
        assert_eq!(computed.base.size.width, Some(gpui::px(5.4).into()));
        assert_eq!(computed.base.size.height, Some(gpui::px(9.0 * 0.4 * 2.0).into()));
        assert_eq!(computed.computed_font.weight, gpui::FontWeight::BOLD);
        let hovered = computed.hover.expect("font state retained");
        assert_eq!(hovered.size.width, Some(gpui::px(9.6).into()));
    }

    #[test]
    fn character_units_in_font_size_use_the_parent_face() {
        let _metrics = test_font_metrics();
        let engine = Engine::compile(".box {font-family:'VN Metric Wide';font-size:2ch;width:1ch}");
        let computed = engine.compute_node_with_font(box_node(), None, &Rc::new(Vars::default()), None, 10.0, &gpui::font("VN Metric Narrow"), &Env::default(), &mut Vec::new());
        assert_eq!(computed.font_size, 11.0);
        assert_eq!(computed.base.size.width, Some(gpui::px(11.0 * 0.8).into()));
    }

    #[test]
    fn inherited_faces_separate_character_unit_share_cache_entries() {
        let _metrics = test_font_metrics();
        let engine = Engine::compile(".box {width:2ch}");
        let vars = Rc::new(Vars::default());
        let read = |family| engine.compute_node_with_font(box_node(), None, &vars, None, 10.0, &gpui::font(family), &Env::default(), &mut Vec::new());
        assert_eq!(read("VN Metric Narrow").base.size.width, Some(gpui::px(11.0).into()));
        assert_eq!(read("VN Metric Wide").base.size.width, Some(gpui::px(16.0).into()));
        assert_eq!(read("VN Metric Narrow").base.size.width, Some(gpui::px(11.0).into()));
    }

    #[test]
    fn dynamic_character_units_use_variable_font_family_and_provider_changes_invalidate_cache() {
        let _metrics = test_font_metrics();
        let engine = Engine::compile(".box {--face:'VN Metric Wide'; font-family:var(--face); width:calc(2ch + 1px);font-size:10px}");
        assert_eq!(engine.compute(box_node(), None, &Env::default()).base.size.width, Some(gpui::px(17.0).into()));
        set_font_metric_source(|_, size| Some(FontRelativeMetrics {zero_advance:size * 0.9,x_height:size * 0.5}));
        assert_eq!(engine.compute(box_node(), None, &Env::default()).base.size.width, Some(gpui::px(19.0).into()));
        set_font_metric_source(|_, _| None);
        assert_eq!(engine.compute(box_node(), None, &Env::default()).base.size.width, Some(gpui::px(11.0).into()), "an unavailable glyph retains the CSS half-em fallback");
    }

    #[test]
    fn border_currentcolor_is_resolved_per_state_without_mask_bleed() {
        for (base_color, hover_color) in [("currentColor", "#0000ff"), ("#0000ff", "currentColor")] {
            let css=format!(".box {{ color:#ff0000; border:2px solid {base_color} }} .box:hover {{ border-top-color:{hover_color} }}");
            let computed=Engine::compile(&css).compute(box_node(), None, &Env::default());
            let red=gpui::rgb(0xff0000).into();
            let blue=gpui::rgb(0x0000ff).into();
            let base=if base_color=="currentColor" {red} else {blue};
            let hover=if hover_color=="currentColor" {red} else {blue};
            assert_eq!(computed.base.vn_border_colors.top, Some(Some(base)), "{css}: base top");
            assert_eq!(computed.base.vn_border_colors.right, Some(Some(base)), "{css}: base right");
            assert_eq!(computed.hover.as_ref().expect("hover").vn_border_colors.top, Some(Some(hover)), "{css}: hovered top");
        }
    }

    #[test]
    fn border_currentcolor_resolves_group_and_generated_box_placeholders() {
        let engine=Engine::compile(".icon {color:#ff0000;border:2px solid blue} .group:hover .icon {border-top-color:currentColor} .group:active .icon {border-left-color:currentColor}");
        let document=group_doc();
        let computed=cascade(&engine,&document,&Env::default());
        let icon=&computed[2];
        let red=gpui::rgb(0xff0000).into();
        let blue=gpui::rgb(0x0000ff).into();
        assert_eq!(icon.base.vn_border_colors.top,Some(Some(blue)));
        assert_eq!(icon.group_hover.as_ref().expect("group hover").1.vn_border_colors.top,Some(Some(red)));
        assert_eq!(icon.group_active.as_ref().expect("group active").1.vn_border_colors.left,Some(Some(red)));
        let engine=Engine::compile(".box {color:blue;border:2px solid red} .box::before {content:'x';color:#00ff00;border-top:2px solid currentColor}");
        let computed=engine.compute(box_node(),None,&Env::default());
        assert_eq!(computed.base.vn_border_colors.top,Some(Some(red)));
        assert_eq!(computed.before.as_ref().expect("generated box").style.vn_border_colors.top,Some(Some(gpui::rgb(0x00ff00).into())));
    }

}