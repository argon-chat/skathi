//! `lightningcss::Property` → `gpui::StyleRefinement`. The load-bearing match
//! statement: this file decides what CSS vue-native renders. Unsupported
//! declarations produce a DIAGNOSTIC, never silence (the redcss `Coverage`
//! philosophy) — a property that parses but does nothing is the worst DX bug
//! a styling system can have.

use std::rc::Rc;

use gpui::{px, relative, AbsoluteLength, DefiniteLength, Hsla, Length, Overflow, StyleRefinement};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::align::{ContentPosition, SelfPosition};
use lightningcss::properties::border::LineStyle;
use lightningcss::properties::display::{DisplayInside, DisplayKeyword};
use lightningcss::properties::flex::FlexDirection as CssFlexDirection;
use lightningcss::properties::font::{AbsoluteFontWeight, FontFamily as CssFontFamily, FontWeight as CssFontWeight, LineHeight as CssLineHeight};
use lightningcss::properties::overflow::OverflowKeyword;
use lightningcss::properties::size::Size as CssSize;
use lightningcss::properties::text::TextAlign as CssTextAlign;
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{Length as CssLength, LengthPercentage, LengthPercentageOrAuto, LengthValue};
use lightningcss::values::percentage::DimensionPercentage;

use crate::transition::{Collector, TransitionSpec};
use crate::vars::{self, Vars};

/// `(name, raw value text)` pairs — the transport for both channels that cannot
/// be mapped at sheet-compile time: `--x` declarations (which fold into the
/// element's [`Vars`] map) and known properties whose value referenced `var()`
/// (which are substituted and re-parsed per element).
pub type Decls = Vec<(String, String)>;

/// CSS `object-fit`, the sizing rule for a REPLACED element's content box.
///
/// It lives here rather than in a `StyleRefinement` because gpui keeps it out
/// of `Style` entirely: `ObjectFit` is a field of the `img()` element's own
/// `ImageStyle`, set through the `StyledImage` trait. A local mirror (rather
/// than `gpui::ObjectFit`) is what lets [`ComputedStates`](crate::ComputedStates)
/// stay `Clone + PartialEq` — gpui's enum derives neither.
///
/// The browser initial value is `fill`; gpui's `ImageStyle::default()` is
/// `Contain`, so the renderer must state `Fill` explicitly when no rule
/// declares one (see `vn-tree`'s `build_img`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

/// One component of a `translate()`, in the unit it was written in.
///
/// A percentage translate resolves against the ELEMENT'S OWN size, which no
/// layout property can express — the renderer resolves it against the box the
/// node was painted at last frame (see `vn-tree`'s `apply_computed_style`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TranslateLen {
    Px(f32),
    /// Fraction, not percent: `-50%` is `-0.5`.
    Percent(f32),
}

impl TranslateLen {
    /// Resolves to pixels against the element's own `size` along this axis.
    pub fn to_px(self, own_size: f32) -> f32 {
        match self {
            TranslateLen::Px(v) => v,
            TranslateLen::Percent(f) => f * own_size,
        }
    }
}

/// Computed values that a `gpui::StyleRefinement` has nowhere to carry.
///
/// The refinement is the cascade's whole payload for every other property, so
/// this is deliberately tiny and `Copy`: one folded value per node, refined in
/// the same (specificity, order) sweep as the refinements themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extras {
    /// `object-fit` — only meaningful on a replaced element (`<img>`); parsed
    /// and folded for every element, exactly as a browser does.
    pub object_fit: Option<ObjectFit>,
    /// `position: fixed`. gpui/taffy have one out-of-flow position (`Absolute`,
    /// resolved against the element's PARENT), so `fixed` maps onto it and the
    /// renderer additionally paints the node in a deferred layer so it lands
    /// above its siblings. See the advisory in the `Position` arm.
    pub fixed: Option<bool>,
    /// `z-index`. gpui has no per-element z, so a POSITIVE z-index becomes a
    /// deferred draw with that priority (which is a real "paints above
    /// everything else"); a negative one cannot be expressed at all.
    pub z_index: Option<i32>,
    /// `pointer-events: none` — the node takes no listeners and no hover/active
    /// rungs, which is how a hitbox stops existing in gpui.
    pub pointer_events_none: Option<bool>,
    /// The `translate()` part of a `transform`, in CSS order (x, y).
    pub translate: Option<(TranslateLen, TranslateLen)>,
}

impl Extras {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Extras::default()
    }

    /// Later-wins merge, matching `Refineable::refine` for the refinements.
    pub(crate) fn refine(&mut self, other: &Extras) {
        if other.object_fit.is_some() {
            self.object_fit = other.object_fit;
        }
        if other.fixed.is_some() {
            self.fixed = other.fixed;
        }
        if other.z_index.is_some() {
            self.z_index = other.z_index;
        }
        if other.pointer_events_none.is_some() {
            self.pointer_events_none = other.pointer_events_none;
        }
        if other.translate.is_some() {
            self.translate = other.translate;
        }
    }

    /// True when this node has to be painted in gpui's deferred layer for its
    /// stacking to be right: `position: fixed`, or a positive `z-index`.
    pub fn deferred_priority(&self) -> Option<usize> {
        let z = self.z_index.unwrap_or(0);
        if z > 0 {
            Some(z as usize)
        } else if self.fixed == Some(true) {
            Some(0)
        } else {
            None
        }
    }
}

/// One declaration block, split into the normal/important refinements and the
/// transition lists each half declares (`None` = the half declared none, which
/// is what lets a later rule's `transition` override an earlier one without an
/// empty list being mistaken for `transition: none`).
pub(crate) struct Converted {
    pub normal: Option<Rc<StyleRefinement>>,
    pub important: Option<Rc<StyleRefinement>>,
    pub normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub important_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub normal_extras: Extras,
    pub important_extras: Extras,
    /// `--x` declarations, in source order (`None` = the half declared none).
    pub normal_custom: Option<Rc<Decls>>,
    pub important_custom: Option<Rc<Decls>>,
    /// Known properties whose value referenced `var()`.
    pub normal_dynamic: Option<Rc<Decls>>,
    pub important_dynamic: Option<Rc<Decls>>,
}

/// One converted declaration half.
#[derive(Default)]
struct Half {
    refinement: Option<Rc<StyleRefinement>>,
    transitions: Option<Rc<Vec<TransitionSpec>>>,
    extras: Extras,
    custom: Option<Rc<Decls>>,
    dynamic: Option<Rc<Decls>>,
}

/// Which channel a declaration travels on.
enum Channel {
    /// `--x: …` — folds into the element's custom-property map instead of into
    /// a refinement. (`--shading` is the one exception; see [`effect`].)
    Custom(String, String),
    /// A known property whose value referenced `var()`. lightningcss hands
    /// these back as `Property::Unparsed`; they are substituted and re-parsed
    /// per element, because the value depends on where the element sits.
    Dynamic(String, String),
    /// Everything else — mapped once, here.
    Static,
}

/// The raw value text of a declaration, recovered by printing it back out.
/// `Property`'s own `ToCss` writes `name: value`, and a property name can never
/// contain `:`, so the split is exact.
fn value_text(p: &Property) -> Option<String> {
    let css = p.to_css_string(false, PrinterOptions::default()).ok()?;
    let (_, value) = css.split_once(':')?;
    Some(value.trim().to_string())
}

fn channel(p: &Property, diagnostics: &mut Vec<String>) -> Channel {
    use lightningcss::properties::custom::CustomPropertyName;
    match p {
        Property::Custom(c) => match &c.name {
            // `--shading` is a REAL mapped property that merely happens to be
            // spelled as a custom property (a GPU effect has no web equivalent,
            // so it needs a spelling a browser parses, stores and ignores). It
            // therefore keeps travelling on the static channel.
            CustomPropertyName::Custom(name) if name.0.as_ref() == effect::SHADING => Channel::Static,
            CustomPropertyName::Custom(name) => match value_text(p) {
                Some(v) => Channel::Custom(name.0.as_ref().to_string(), v),
                None => {
                    once(
                        diagnostics,
                        format!(
                            "`{}` could not be serialized back to text and was dropped (v1)",
                            name.0.as_ref()
                        ),
                    );
                    Channel::Static
                }
            },
            // An unknown/vendor-prefixed property (`-webkit-line-clamp`), or a
            // real CSS property lightningcss has no typed representation for
            // (`object-fit`). Both are handled by `apply`.
            CustomPropertyName::Unknown(_) => Channel::Static,
        },
        Property::Unparsed(u) => match value_text(p) {
            Some(v) => Channel::Dynamic(u.property_id.name().to_string(), v),
            None => Channel::Static,
        },
        _ => Channel::Static,
    }
}

/// Converts a declaration block into (normal, important) refinements plus the
/// transitions, custom properties and `var()`-composed declarations each half
/// declares.
pub(crate) fn convert_block(block: &DeclarationBlock, diagnostics: &mut Vec<String>) -> Converted {
    let mut convert = |props: &[Property]| -> Half {
        if props.is_empty() {
            return Half::default();
        }
        let mut r = StyleRefinement::default();
        let mut extras = Extras::default();
        let mut transitions = Collector::default();
        let mut custom: Decls = Vec::new();
        let mut dynamic: Decls = Vec::new();
        let mut styled = false;
        for p in props {
            match channel(p, diagnostics) {
                Channel::Custom(name, value) => custom.push((name, value)),
                Channel::Dynamic(name, value) => dynamic.push((name, value)),
                Channel::Static => {
                    if transitions.take(p, diagnostics) {
                        continue;
                    }
                    // Within one block the mapped declarations are folded now
                    // and the `var()`-composed ones per element afterwards, so
                    // a block that declares the SAME property both ways loses
                    // the source order between them. Rare enough to be worth
                    // saying rather than modelling.
                    let id = p.property_id();
                    if dynamic.iter().any(|(n, _)| *n == id.name()) {
                        once(
                            diagnostics,
                            format!(
                                "`{}` is declared twice in one rule, once through var() and once \
                                 literally — the literal one wins regardless of source order (v1)",
                                id.name()
                            ),
                        );
                    }
                    apply(p, &mut r, &mut extras, diagnostics);
                    styled = true;
                }
            }
        }
        Half {
            refinement: styled.then(|| Rc::new(r)),
            transitions: transitions.finish().map(Rc::new),
            extras,
            custom: (!custom.is_empty()).then(|| Rc::new(custom)),
            dynamic: (!dynamic.is_empty()).then(|| Rc::new(dynamic)),
        }
    };
    let normal = convert(&block.declarations);
    let important = convert(&block.important_declarations);
    Converted {
        normal: normal.refinement,
        important: important.refinement,
        normal_transitions: normal.transitions,
        important_transitions: important.transitions,
        normal_extras: normal.extras,
        important_extras: important.extras,
        normal_custom: normal.custom,
        important_custom: important.custom,
        normal_dynamic: normal.dynamic,
        important_dynamic: important.dynamic,
    }
}

// ---------------------------------------------------------------------------
// computed-value time: var() substitution + re-parse
// ---------------------------------------------------------------------------

/// Applies one `var()`-composed declaration against an element's resolved
/// custom-property map: substitute the text, re-parse it through lightningcss
/// with the ORIGINAL property id, then hand the typed property to the ordinary
/// mapper. Every property the mapper already supports therefore supports
/// `var()` with no per-property code.
///
/// Three failure modes, each with its own advisory and none of them silent:
/// * the substitution itself is invalid (missing variable with no fallback,
///   a reference cycle) — the declaration is dropped, exactly as CSS says;
/// * the substituted text does not parse as that property — also dropped, and
///   also what a browser does;
/// * the property parses but vn-css does not map it — the ordinary
///   "property not mapped" advisory, from `apply`.
pub(crate) fn apply_dynamic(
    name: &str,
    raw: &str,
    vars: &Vars,
    r: &mut StyleRefinement,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    let value = match vars::substitute_with(raw, vars) {
        Ok(v) => v,
        Err(e) => {
            once(
                diagnostics,
                format!(
                    "`{name}: {raw}` is invalid at computed-value time — {} (the declaration was \
                     dropped, as it would be in a browser)",
                    e.describe()
                ),
            );
            return;
        }
    };
    let property = match Property::parse_string(PropertyId::from(name), &value, ParserOptions::default())
    {
        Ok(p) => p,
        Err(_) => {
            once(
                diagnostics,
                format!("`{name}: {value}` did not parse after var() substitution (v1)"),
            );
            return;
        }
    };
    // A `transition` is folded into its own list at sheet-compile time and the
    // list is per rule, not per element — so a var()-composed one has nowhere
    // to land. Say so instead of applying it as an unmapped property.
    if matches!(
        property,
        Property::Transition(..)
            | Property::TransitionProperty(..)
            | Property::TransitionDuration(..)
            | Property::TransitionDelay(..)
            | Property::TransitionTimingFunction(..)
    ) {
        once(
            diagnostics,
            format!(
                "`{name}` composed with var() is not honoured (v1) — the transition list is folded \
                 once per rule, not per element; declare the transition literally"
            ),
        );
        return;
    }
    apply(&property, r, x, diagnostics);
}

/// Parses an inline `style` body ("color: red; padding: 4px") into a
/// refinement — the transport for Vue `:style` objects and `style=""` strings.
///
/// Transitions declared inline are DROPPED here (the refinement has nowhere to
/// carry them); use [`parse_inline_with_transitions`] to keep them.
pub fn parse_inline(body: &str, diagnostics: &mut Vec<String>) -> StyleRefinement {
    let (refinement, transitions, _) = parse_inline_full(body, diagnostics);
    if !transitions.is_empty() {
        diagnostics.push(
            "inline `transition` is parsed but dropped by parse_inline — the caller must use \
             parse_inline_with_transitions to honor it"
                .into(),
        );
    }
    refinement
}

/// Parses an inline `style` body into a refinement plus the transitions it
/// declares. An empty `Vec` means "declared none"; `transition: none` also
/// yields an empty `Vec` (it clears, and inline is the last cascade origin).
pub fn parse_inline_with_transitions(
    body: &str,
    diagnostics: &mut Vec<String>,
) -> (StyleRefinement, Vec<TransitionSpec>) {
    let (r, t, _) = parse_inline_full(body, diagnostics);
    (r, t)
}

/// [`parse_inline_with_transitions`] plus the [`Extras`] the body declared —
/// the form the retained tree uses, so an inline `style="object-fit: cover"`
/// wins over the sheet the way every other inline declaration does.
pub fn parse_inline_full(
    body: &str,
    diagnostics: &mut Vec<String>,
) -> (StyleRefinement, Vec<TransitionSpec>, Extras) {
    let inline = parse_inline_style(body, diagnostics);
    (
        inline.refinement.map(|r| (*r).clone()).unwrap_or_default(),
        inline.transitions.map(|t| (*t).clone()).unwrap_or_default(),
        inline.extras,
    )
}

/// Everything an inline `style` body contributes to the cascade.
///
/// The custom-property and `var()` halves cannot be folded here the way the
/// mapped declarations are: both depend on where the element sits in the tree,
/// so they ride along as text and are resolved by [`Engine::compute_node`]
/// (crate::Engine) exactly like a rule's are.
#[derive(Clone, Debug, Default)]
pub struct InlineStyle {
    pub refinement: Option<Rc<StyleRefinement>>,
    /// `None` = the body declared no `transition` at all (which is what lets a
    /// matched rule's list survive); `Some([])` is `transition: none`.
    pub transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub extras: Extras,
    /// `--x` declarations. This is the channel `element.style.setProperty("--x")`
    /// lands on — reka-ui writes its `--reka-popper-*` sizing vars that way.
    pub custom: Decls,
    /// Known properties whose value referenced `var()`.
    pub dynamic: Decls,
}

impl InlineStyle {
    pub fn is_empty(&self) -> bool {
        self.refinement.is_none()
            && self.transitions.is_none()
            && self.extras == Extras::default()
            && self.custom.is_empty()
            && self.dynamic.is_empty()
    }
}

/// Parses an inline `style` body into everything it contributes.
pub fn parse_inline_style(body: &str, diagnostics: &mut Vec<String>) -> InlineStyle {
    let mut r = StyleRefinement::default();
    let mut extras = Extras::default();
    let mut transitions = Collector::default();
    let mut custom: Decls = Vec::new();
    let mut dynamic: Decls = Vec::new();
    let mut styled = false;
    match DeclarationBlock::parse_string(body, ParserOptions::default()) {
        Ok(block) => {
            for p in block.declarations.iter().chain(block.important_declarations.iter()) {
                match channel(p, diagnostics) {
                    Channel::Custom(name, value) => custom.push((name, value)),
                    Channel::Dynamic(name, value) => dynamic.push((name, value)),
                    Channel::Static => {
                        if transitions.take(p, diagnostics) {
                            continue;
                        }
                        apply(p, &mut r, &mut extras, diagnostics);
                        styled = true;
                    }
                }
            }
        }
        Err(e) => diagnostics.push(format!("inline style parse error: {e} in {body:?}")),
    }
    InlineStyle {
        refinement: styled.then(|| Rc::new(r)),
        transitions: transitions.finish().map(Rc::new),
        extras,
        custom,
        dynamic,
    }
}

// ---------------------------------------------------------------------------
// value converters
// ---------------------------------------------------------------------------

fn color(c: &CssColor) -> Option<Hsla> {
    match c {
        CssColor::RGBA(c) => Some(
            gpui::Rgba {
                r: c.red as f32 / 255.0,
                g: c.green as f32 / 255.0,
                b: c.blue as f32 / 255.0,
                a: c.alpha as f32 / 255.0,
            }
            .into(),
        ),
        _ => None,
    }
}

fn length_value_px(v: &LengthValue) -> Option<f32> {
    match v {
        LengthValue::Px(v) => Some(*v),
        LengthValue::Rem(v) => Some(v * 16.0),
        LengthValue::Em(v) => Some(v * 16.0), // v1: no parent font-size context
        _ => None,
    }
}

/// `calc()` over ABSOLUTE lengths, evaluated to pixels.
///
/// This is the third item on the Tailwind-v4 list in
/// `packages/vue-native/tailwind.ts`, and it is what makes shadcn's own radius
/// scale work: `rounded-md` is `border-radius: calc(var(--radius) - 2px)`, so
/// without it three quarters of the corner radii in the library vanish the
/// moment the theme stops being pre-resolved.
///
/// Percentages are deliberately NOT evaluated: they need the containing block,
/// which does not exist until layout. A `calc()` that mixes one in returns
/// `None` and takes the ordinary "value is unresolved" path.
fn calc_px<V>(calc: &lightningcss::values::calc::Calc<V>, leaf: &dyn Fn(&V) -> Option<f32>) -> Option<f32> {
    use lightningcss::values::calc::{Calc, MathFunction};
    match calc {
        Calc::Value(v) => leaf(v),
        Calc::Number(n) => Some(*n),
        Calc::Sum(a, b) => Some(calc_px(a, leaf)? + calc_px(b, leaf)?),
        Calc::Product(n, v) => Some(n * calc_px(v, leaf)?),
        Calc::Function(f) => match &**f {
            MathFunction::Calc(c) => calc_px(c, leaf),
            MathFunction::Min(list) => list
                .iter()
                .map(|c| calc_px(c, leaf))
                .try_fold(f32::INFINITY, |acc, v| Some(acc.min(v?))),
            MathFunction::Max(list) => list
                .iter()
                .map(|c| calc_px(c, leaf))
                .try_fold(f32::NEG_INFINITY, |acc, v| Some(acc.max(v?))),
            MathFunction::Clamp(lo, v, hi) => {
                Some(calc_px(v, leaf)?.clamp(calc_px(lo, leaf)?, calc_px(hi, leaf)?))
            }
            MathFunction::Abs(v) => Some(calc_px(v, leaf)?.abs()),
            MathFunction::Sign(v) => Some(calc_px(v, leaf)?.signum()),
            // round()/rem()/mod()/hypot() have no user in any sheet we serve;
            // they take the "unresolved" path with the ordinary advisory.
            _ => None,
        },
    }
}

fn css_length_px(l: &CssLength) -> Option<f32> {
    match l {
        CssLength::Value(v) => length_value_px(v),
        CssLength::Calc(c) => calc_px(c, &|v: &CssLength| css_length_px(v)),
    }
}

/// The absolute half of a `<length-percentage>` inside a `calc()`.
fn lp_leaf_px(lp: &LengthPercentage) -> Option<f32> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v),
        // A percentage needs the containing block; a calc() that mixes one in
        // is left unresolved rather than guessed at.
        DimensionPercentage::Percentage(_) => None,
        DimensionPercentage::Calc(c) => calc_px(c, &lp_leaf_px),
    }
}

fn lp_definite(lp: &LengthPercentage) -> Option<DefiniteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        DimensionPercentage::Percentage(p) => Some(relative(p.0)),
        DimensionPercentage::Calc(c) => calc_px(c, &lp_leaf_px).map(|p| px(p).into()),
    }
}

fn lpa_length(v: &LengthPercentageOrAuto) -> Option<Length> {
    match v {
        LengthPercentageOrAuto::Auto => Some(Length::Auto),
        LengthPercentageOrAuto::LengthPercentage(lp) => lp_definite(lp).map(Length::Definite),
    }
}

fn size_length(v: &CssSize) -> Option<Length> {
    match v {
        CssSize::Auto => Some(Length::Auto),
        CssSize::LengthPercentage(lp) => lp_definite(lp).map(Length::Definite),
        _ => None,
    }
}

fn max_size_length(v: &lightningcss::properties::size::MaxSize) -> Option<Length> {
    use lightningcss::properties::size::MaxSize;
    match v {
        MaxSize::None => Some(Length::Auto),
        MaxSize::LengthPercentage(lp) => lp_definite(lp).map(Length::Definite),
        _ => None,
    }
}

fn lp_absolute(lp: &LengthPercentage) -> Option<AbsoluteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        DimensionPercentage::Calc(c) => calc_px(c, &lp_leaf_px).map(|p| px(p).into()),
        DimensionPercentage::Percentage(_) => None,
    }
}

fn gap_definite(v: &lightningcss::properties::align::GapValue) -> Option<DefiniteLength> {
    use lightningcss::properties::align::GapValue;
    match v {
        GapValue::LengthPercentage(lp) => lp_definite(lp),
        GapValue::Normal => None,
    }
}

/// The cross-axis alignment keywords, shared by `align-items` / `align-self`
/// (gpui aliases `AlignSelf = AlignItems`).
fn self_position(v: &SelfPosition) -> gpui::AlignItems {
    use gpui::AlignItems as G;
    match v {
        SelfPosition::Center => G::Center,
        SelfPosition::Start | SelfPosition::FlexStart | SelfPosition::SelfStart => G::FlexStart,
        SelfPosition::End | SelfPosition::FlexEnd | SelfPosition::SelfEnd => G::FlexEnd,
    }
}

/// `repeat(N, minmax(<min>, 1fr))` — the ONLY track list gpui's `GridTemplate`
/// can express, and (not by accident) exactly what Tailwind's `grid-cols-N` /
/// `grid-rows-N` emit. Anything else returns `None` and gets an advisory.
fn grid_template(v: &lightningcss::properties::grid::TrackSizing) -> Option<gpui::GridTemplate> {
    use lightningcss::properties::grid::{
        RepeatCount, TrackBreadth, TrackListItem, TrackSize, TrackSizing,
    };
    let TrackSizing::TrackList(list) = v else {
        return None; // `none`
    };
    let [TrackListItem::TrackRepeat(repeat)] = list.items.as_slice() else {
        return None;
    };
    let RepeatCount::Number(count) = repeat.count else {
        return None; // auto-fill / auto-fit
    };
    if count < 0 {
        return None;
    }
    let [TrackSize::MinMax { min, max }] = repeat.track_sizes.as_slice() else {
        return None;
    };
    // The max half must be the `1fr` that makes the tracks share the space.
    if !matches!(max, TrackBreadth::Flex(f) if *f == 1.0) {
        return None;
    }
    let min_size = match min {
        TrackBreadth::Length(DimensionPercentage::Dimension(l)) if length_value_px(l) == Some(0.0) => {
            gpui::GridTemplateMinSize::Zero
        }
        TrackBreadth::MinContent => gpui::GridTemplateMinSize::MinContent,
        TrackBreadth::MaxContent => gpui::GridTemplateMinSize::MaxContent,
        _ => return None,
    };
    Some(gpui::GridTemplate {
        repeat: count as u16,
        min_size,
    })
}

/// Advisories are ONE-SHOT: `diagnostics` accumulates for a whole stylesheet
/// compile, and a generated sheet hits the same gap over and over (a broad
/// Tailwind sweep produced 123 `--tw-*` declarations and 63 `var()`-composed
/// values). Printing each would bury every other message, so a given advisory
/// is recorded once and the rest are folded into it.
pub(crate) fn once(diagnostics: &mut Vec<String>, msg: String) {
    if !diagnostics.iter().any(|d| *d == msg) {
        diagnostics.push(msg);
    }
}

fn unsupported(diagnostics: &mut Vec<String>, what: &str) {
    once(diagnostics, format!("unsupported value for {what} (v1)"));
}

fn warn_once(diagnostics: &mut Vec<String>, msg: &str) {
    once(diagnostics, msg.to_string());
}

/// [`color`] only understands a resolved RGBA. `currentColor`, `color-mix()`,
/// `light-dark()` and friends land here — and USED to fall through silently,
/// which meant Tailwind's `bg-current` / `text-current` did nothing at all with
/// no way to find out.
fn unresolved_color(diagnostics: &mut Vec<String>, what: &str) {
    once(
        diagnostics,
        format!(
            "`{what}` value does not resolve to a concrete color (v1) — `currentColor`, \
             `color-mix()` and system colors are not computed"
        ),
    );
}


// `--shading`, the one custom property that IS mapped. Declared here rather
// than in lib.rs so the whole effect surface — the module, its interception in
// the `Custom(_)` branch below, and nothing else — lands in one place.
#[path = "effect.rs"]
pub(crate) mod effect;

// ---------------------------------------------------------------------------
// the mapper
// ---------------------------------------------------------------------------

/// `object-fit: <keyword>`. lightningcss has no typed `object-fit`, so the
/// declaration arrives as an unknown property carrying a raw token list — the
/// same shape `--shading` uses, minus the function call.
fn single_ident<'a>(
    value: &'a lightningcss::properties::custom::TokenList<'a>,
) -> Option<&'a str> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut idents = value.0.iter().filter_map(|t| match t {
        TokenOrValue::Token(Token::Ident(s)) => Some(s.as_ref()),
        _ => None,
    });
    let first = idents.next();
    // `object-fit: cover cover` is not valid CSS and a browser drops the whole
    // declaration; so does anything that is not a bare keyword.
    if idents.next().is_some() {
        return None;
    }
    first
}

fn object_fit(value: &lightningcss::properties::custom::TokenList, diagnostics: &mut Vec<String>) -> Option<ObjectFit> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let multiple = value
        .0
        .iter()
        .filter(|t| matches!(t, TokenOrValue::Token(Token::Ident(_))))
        .count()
        > 1;
    if multiple {
        once(
            diagnostics,
            "`object-fit` takes a single keyword — the declaration was dropped".into(),
        );
        return None;
    }
    let first = single_ident(value);
    match first {
        Some("fill") => Some(ObjectFit::Fill),
        Some("contain") => Some(ObjectFit::Contain),
        Some("cover") => Some(ObjectFit::Cover),
        Some("scale-down") => Some(ObjectFit::ScaleDown),
        Some("none") => Some(ObjectFit::None),
        other => {
            once(
                diagnostics,
                format!(
                    "unsupported value for object-fit: {} — expected fill | contain | cover | \
                     scale-down | none",
                    other.unwrap_or("<not a keyword>")
                ),
            );
            None
        }
    }
}

fn apply(p: &Property, r: &mut StyleRefinement, x: &mut Extras, diagnostics: &mut Vec<String>) {
    use Property::*;
    match p {
        Display(d) => {
            use lightningcss::properties::display::Display as D;
            r.display = Some(match d {
                D::Keyword(DisplayKeyword::None) => gpui::Display::None,
                D::Pair(pair) => match pair.inside {
                    DisplayInside::Flex(_) => gpui::Display::Flex,
                    DisplayInside::Grid => gpui::Display::Grid,
                    _ => gpui::Display::Block,
                },
                _ => gpui::Display::Block,
            });
        }
        FlexDirection(d, _) => {
            r.flex_direction = Some(match d {
                CssFlexDirection::Row => gpui::FlexDirection::Row,
                CssFlexDirection::RowReverse => gpui::FlexDirection::RowReverse,
                CssFlexDirection::Column => gpui::FlexDirection::Column,
                CssFlexDirection::ColumnReverse => gpui::FlexDirection::ColumnReverse,
            });
        }
        FlexGrow(v, _) => r.flex_grow = Some(*v),
        FlexShrink(v, _) => r.flex_shrink = Some(*v),
        FlexBasis(v, _) => r.flex_basis = lpa_length(v),
        // `flex: 1 1 0%` (Tailwind `flex-1`), `flex: 0 1 auto` (`flex-initial`), …
        Flex(v, _) => {
            r.flex_grow = Some(v.grow);
            r.flex_shrink = Some(v.shrink);
            r.flex_basis = lpa_length(&v.basis);
        }
        FlexWrap(v, _) => {
            use lightningcss::properties::flex::FlexWrap as W;
            r.flex_wrap = Some(match v {
                W::NoWrap => gpui::FlexWrap::NoWrap,
                W::Wrap => gpui::FlexWrap::Wrap,
                W::WrapReverse => gpui::FlexWrap::WrapReverse,
            });
        }
        AlignItems(v, _) => {
            use gpui::AlignItems as G;
            let mapped = match v {
                lightningcss::properties::align::AlignItems::Stretch => Some(G::Stretch),
                lightningcss::properties::align::AlignItems::SelfPosition { value, .. } => {
                    Some(self_position(value))
                }
                lightningcss::properties::align::AlignItems::BaselinePosition(_) => Some(G::Baseline),
                _ => None,
            };
            match mapped {
                Some(m) => r.align_items = Some(m),
                None => unsupported(diagnostics, "align-items"),
            }
        }
        AlignSelf(v, _) => {
            use lightningcss::properties::align::AlignSelf as A;
            let mapped = match v {
                // `self-auto`: inherit the parent's align-items. gpui spells
                // that as "unset", and a refinement cannot express "clear" —
                // so leave the field alone rather than pin it to Stretch.
                A::Auto => None,
                A::Stretch => Some(gpui::AlignItems::Stretch),
                A::BaselinePosition(_) => Some(gpui::AlignItems::Baseline),
                A::SelfPosition { value, .. } => Some(self_position(value)),
                A::Normal => None,
            };
            match mapped {
                Some(m) => r.align_self = Some(m),
                // `auto`/`normal` mean "defer to the parent", which is already
                // what an unset refinement does — but say so, because a rule
                // that MEANT to undo an earlier `self-center` will not.
                None => unsupported(
                    diagnostics,
                    "align-self: auto/normal — cannot undo an earlier align-self",
                ),
            }
        }
        AlignContent(v, _) => {
            use gpui::AlignContent as G;
            use lightningcss::properties::align::AlignContent as A;
            let mapped = match v {
                A::ContentDistribution(d) => {
                    use lightningcss::properties::align::ContentDistribution as CD;
                    Some(match d {
                        CD::SpaceBetween => G::SpaceBetween,
                        CD::SpaceAround => G::SpaceAround,
                        CD::SpaceEvenly => G::SpaceEvenly,
                        CD::Stretch => G::Stretch,
                    })
                }
                A::ContentPosition { value, .. } => Some(match value {
                    ContentPosition::Center => G::Center,
                    ContentPosition::Start | ContentPosition::FlexStart => G::FlexStart,
                    ContentPosition::End | ContentPosition::FlexEnd => G::FlexEnd,
                }),
                _ => None,
            };
            match mapped {
                Some(m) => r.align_content = Some(m),
                None => unsupported(diagnostics, "align-content"),
            }
        }
        JustifyContent(v, _) => {
            use gpui::JustifyContent as G;
            let mapped = match v {
                lightningcss::properties::align::JustifyContent::ContentDistribution(d) => {
                    use lightningcss::properties::align::ContentDistribution as CD;
                    Some(match d {
                        CD::SpaceBetween => G::SpaceBetween,
                        CD::SpaceAround => G::SpaceAround,
                        CD::SpaceEvenly => G::SpaceEvenly,
                        CD::Stretch => G::Stretch,
                    })
                }
                lightningcss::properties::align::JustifyContent::ContentPosition { value, .. } => {
                    match value {
                        ContentPosition::Center => Some(G::Center),
                        ContentPosition::Start | ContentPosition::FlexStart => Some(G::FlexStart),
                        ContentPosition::End | ContentPosition::FlexEnd => Some(G::FlexEnd),
                    }
                }
                _ => None,
            };
            match mapped {
                Some(m) => r.justify_content = Some(m),
                None => unsupported(diagnostics, "justify-content"),
            }
        }
        Gap(gap) => {
            if let Some(v) = gap_definite(&gap.row) {
                r.gap.height = Some(v);
            }
            if let Some(v) = gap_definite(&gap.column) {
                r.gap.width = Some(v);
            }
        }
        // Tailwind `gap-y-*` / `gap-x-*` emit the longhands, never the shorthand.
        RowGap(v) => r.gap.height = gap_definite(v),
        ColumnGap(v) => r.gap.width = gap_definite(v),
        Width(v) => r.size.width = size_length(v),
        Height(v) => r.size.height = size_length(v),
        MinWidth(v) => r.min_size.width = size_length(v),
        MinHeight(v) => r.min_size.height = size_length(v),
        MaxWidth(v) => r.max_size.width = max_size_length(v),
        MaxHeight(v) => r.max_size.height = max_size_length(v),
        Padding(v) => {
            r.padding.top = lp_definite_of_lpa(&v.top);
            r.padding.right = lp_definite_of_lpa(&v.right);
            r.padding.bottom = lp_definite_of_lpa(&v.bottom);
            r.padding.left = lp_definite_of_lpa(&v.left);
        }
        PaddingTop(v) => r.padding.top = lp_definite_of_lpa(v),
        PaddingRight(v) => r.padding.right = lp_definite_of_lpa(v),
        PaddingBottom(v) => r.padding.bottom = lp_definite_of_lpa(v),
        PaddingLeft(v) => r.padding.left = lp_definite_of_lpa(v),
        Margin(v) => {
            r.margin.top = lpa_length(&v.top);
            r.margin.right = lpa_length(&v.right);
            r.margin.bottom = lpa_length(&v.bottom);
            r.margin.left = lpa_length(&v.left);
        }
        MarginTop(v) => r.margin.top = lpa_length(v),
        MarginRight(v) => r.margin.right = lpa_length(v),
        MarginBottom(v) => r.margin.bottom = lpa_length(v),
        MarginLeft(v) => r.margin.left = lpa_length(v),
        Position(v) => {
            use lightningcss::properties::position::Position as P;
            x.fixed = Some(matches!(v, P::Fixed));
            r.position = Some(match v {
                P::Absolute => gpui::Position::Absolute,
                // `fixed` is an out-of-flow box against the VIEWPORT. taffy has
                // exactly one out-of-flow mode and it resolves against the
                // element's parent, so that is what `fixed` becomes — plus a
                // deferred paint layer (see `Extras::deferred_priority`) so the
                // box lands above its siblings the way a fixed one does.
                //
                // The two agree exactly when the fixed element's parent IS the
                // window — which is the case that matters, because a portalled
                // overlay (`<Teleport to="body">`) has the app root as its
                // parent. They diverge when a `fixed` element is left inside a
                // smaller ancestor: it is then positioned against that ancestor
                // rather than the viewport.
                P::Fixed => {
                    warn_once(
                        diagnostics,
                        "`position: fixed` is laid out as `absolute` against its PARENT (v1) — \
                         gpui/taffy have no viewport-relative box. It is painted in a deferred \
                         layer so it still stacks above its siblings; portal the element to the \
                         app root (Vue's `<Teleport>`) to make the two agree",
                    );
                    gpui::Position::Absolute
                }
                P::Static | P::Relative => gpui::Position::Relative,
                _ => {
                    unsupported(diagnostics, "position: sticky");
                    gpui::Position::Relative
                }
            });
        }
        // gpui has no per-element z. A POSITIVE z-index becomes a deferred draw
        // with that priority — a real "paints (and hit-tests) above everything
        // else", which is what every overlay uses it for. A negative one would
        // have to paint BELOW its siblings, and there is no lever for that.
        ZIndex(v) => {
            use lightningcss::properties::position::ZIndex as Z;
            match v {
                Z::Integer(n) if *n > 0 => x.z_index = Some(*n),
                Z::Integer(n) if *n < 0 => {
                    warn_once(
                        diagnostics,
                        "a negative `z-index` cannot paint below its siblings (v1) — gpui has no \
                         per-element z and its deferred layer only paints ON TOP; reorder the \
                         elements instead",
                    );
                    x.z_index = Some(*n);
                }
                Z::Integer(n) => x.z_index = Some(*n),
                Z::Auto => x.z_index = Some(0),
            }
        }
        Transform(list, _) => apply_transform(list, x, diagnostics),
        // taffy is border-box and only border-box, so `border-box` is not a
        // divergence — it is the model. `content-box` is one, and says so.
        BoxSizing(v, _) => {
            use lightningcss::properties::size::BoxSizing as BS;
            if matches!(v, BS::ContentBox) {
                warn_once(
                    diagnostics,
                    "`box-sizing: content-box` is not supported (v1) — taffy sizes every box as \
                     `border-box`, so padding and border stay INSIDE the declared width/height",
                );
            }
        }
        Top(v) => r.inset.top = lpa_length(v),
        Right(v) => r.inset.right = lpa_length(v),
        Bottom(v) => r.inset.bottom = lpa_length(v),
        Left(v) => r.inset.left = lpa_length(v),
        Inset(v) => {
            r.inset.top = lpa_length(&v.top);
            r.inset.right = lpa_length(&v.right);
            r.inset.bottom = lpa_length(&v.bottom);
            r.inset.left = lpa_length(&v.left);
        }
        Overflow(v) => {
            r.overflow.x = Some(overflow(v.x));
            r.overflow.y = Some(overflow(v.y));
        }
        // The longhands are what Tailwind's `overflow-x-*` / `overflow-y-*`
        // emit; the shorthand only covers `overflow-*`.
        OverflowX(v) => r.overflow.x = Some(overflow(*v)),
        OverflowY(v) => r.overflow.y = Some(overflow(*v)),
        Visibility(v) => {
            use lightningcss::properties::display::Visibility as V;
            r.visibility = Some(match v {
                V::Visible => gpui::Visibility::Visible,
                V::Hidden => gpui::Visibility::Hidden,
                V::Collapse => {
                    // `collapse` only differs from `hidden` inside tables and
                    // flex containers, neither of which gpui models.
                    unsupported(diagnostics, "visibility: collapse (treated as hidden)");
                    gpui::Visibility::Hidden
                }
            });
        }
        AspectRatio(v) => match &v.ratio {
            Some(ratio) if !v.auto && ratio.1 != 0.0 => r.aspect_ratio = Some(ratio.0 / ratio.1),
            // `aspect-auto` / `auto <ratio>`: a refinement cannot say "clear",
            // so an earlier ratio survives.
            _ => unsupported(diagnostics, "aspect-ratio: auto — cannot undo an earlier ratio"),
        },
        Opacity(v) => r.opacity = Some(v.0),
        Color(c) => match color(c) {
            Some(c) => r.text.color = Some(c),
            None => unresolved_color(diagnostics, "color"),
        },
        BackgroundColor(c) => match color(c) {
            Some(c) => r.background = Some(gpui::Fill::Color(c.into())),
            None => unresolved_color(diagnostics, "background-color"),
        },
        Background(layers) => {
            // Shorthand: per CSS the color lives on the LAST layer. v1 takes
            // the color only; gradients/images are a later mapper stage.
            if let Some(last) = layers.last() {
                if let Some(c) = color(&last.color) {
                    r.background = Some(gpui::Fill::Color(c.into()));
                }
            }
        }
        BorderColor(v) => {
            // gpui has one border color for all four sides — take the top.
            match color(&v.top) {
                Some(c) => r.border_color = Some(c),
                None => unresolved_color(diagnostics, "border-color"),
            }
        }
        Border(b) => {
            if let Some(w) = border_width_px(&b.width) {
                let w: AbsoluteLength = px(w).into();
                r.border_widths.top = Some(w);
                r.border_widths.right = Some(w);
                r.border_widths.bottom = Some(w);
                r.border_widths.left = Some(w);
            }
            if let Some(c) = color(&b.color) {
                r.border_color = Some(c);
            }
            set_border_style(r, b.style, diagnostics);
        }
        // Tailwind emits the `border-width` shorthand for `border-2` and the
        // per-side longhands for `border-t` / `border-x-2`.
        BorderWidth(v) => {
            r.border_widths.top = border_width_px(&v.top).map(|w| px(w).into());
            r.border_widths.right = border_width_px(&v.right).map(|w| px(w).into());
            r.border_widths.bottom = border_width_px(&v.bottom).map(|w| px(w).into());
            r.border_widths.left = border_width_px(&v.left).map(|w| px(w).into());
        }
        BorderTopWidth(v) => r.border_widths.top = border_width_px(v).map(|w| px(w).into()),
        BorderRightWidth(v) => r.border_widths.right = border_width_px(v).map(|w| px(w).into()),
        BorderBottomWidth(v) => r.border_widths.bottom = border_width_px(v).map(|w| px(w).into()),
        BorderLeftWidth(v) => r.border_widths.left = border_width_px(v).map(|w| px(w).into()),
        // gpui has ONE border style for the whole box — take the top side, the
        // same compromise `border-color` already makes.
        BorderStyle(v) => set_border_style(r, v.top, diagnostics),
        BorderTopStyle(v) | BorderRightStyle(v) | BorderBottomStyle(v) | BorderLeftStyle(v) => {
            set_border_style(r, *v, diagnostics);
        }
        BorderRadius(v, _) => {
            r.corner_radii.top_left = lp_absolute(&v.top_left.0);
            r.corner_radii.top_right = lp_absolute(&v.top_right.0);
            r.corner_radii.bottom_left = lp_absolute(&v.bottom_left.0);
            r.corner_radii.bottom_right = lp_absolute(&v.bottom_right.0);
        }
        // `rounded-t` / `rounded-tl-lg` never use the shorthand.
        BorderTopLeftRadius(v, _) => r.corner_radii.top_left = lp_absolute(&v.0),
        BorderTopRightRadius(v, _) => r.corner_radii.top_right = lp_absolute(&v.0),
        BorderBottomRightRadius(v, _) => r.corner_radii.bottom_right = lp_absolute(&v.0),
        BorderBottomLeftRadius(v, _) => r.corner_radii.bottom_left = lp_absolute(&v.0),
        // `grid-cols-N` / `grid-rows-N` -> `repeat(N, minmax(0, 1fr))`, which is
        // exactly (and only) what gpui's GridTemplate models.
        GridTemplateColumns(v) => match grid_template(v) {
            Some(t) => r.grid_cols = Some(t),
            None => unsupported(
                diagnostics,
                "grid-template-columns (only `repeat(N, minmax(0|min-content|max-content, 1fr))`)",
            ),
        },
        GridTemplateRows(v) => match grid_template(v) {
            Some(t) => r.grid_rows = Some(t),
            None => unsupported(
                diagnostics,
                "grid-template-rows (only `repeat(N, minmax(0|min-content|max-content, 1fr))`)",
            ),
        },
        BoxShadow(shadows, _) => {
            let mut out = Vec::new();
            for s in shadows.iter() {
                let (Some(c), Some(x), Some(y)) =
                    (color(&s.color), css_length_px(&s.x_offset), css_length_px(&s.y_offset))
                else {
                    continue;
                };
                out.push(gpui::BoxShadow {
                    color: c,
                    offset: gpui::point(px(x), px(y)),
                    blur_radius: px(css_length_px(&s.blur).unwrap_or(0.0)),
                    spread_radius: px(css_length_px(&s.spread).unwrap_or(0.0)),
                    inset: s.inset,
                });
            }
            r.box_shadow = Some(out);
        }
        FontFamily(families) => {
            for f in families {
                if let CssFontFamily::FamilyName(name) = f {
                    use lightningcss::traits::ToCss;
                    if let Ok(css) = name.to_css_string(Default::default()) {
                        let clean = css.trim_matches('"').to_string();
                        r.text.font_family = Some(gpui::SharedString::from(clean));
                    }
                    break;
                }
            }
        }
        FontSize(v) => {
            use lightningcss::properties::font::FontSize as FS;
            if let FS::Length(lp) = v {
                r.text.font_size = lp_absolute(lp);
            }
        }
        FontWeight(w) => {
            let n = match w {
                CssFontWeight::Absolute(AbsoluteFontWeight::Weight(n)) => Some(*n),
                CssFontWeight::Absolute(AbsoluteFontWeight::Normal) => Some(400.0),
                CssFontWeight::Absolute(AbsoluteFontWeight::Bold) => Some(700.0),
                _ => None,
            };
            match n {
                Some(n) => r.text.font_weight = Some(gpui::FontWeight(n)),
                None => unsupported(diagnostics, "font-weight (bolder/lighter)"),
            }
        }
        LineHeight(v) => {
            r.text.line_height = match v {
                CssLineHeight::Number(n) => Some(relative(*n)),
                CssLineHeight::Length(lp) => lp_definite(lp),
                CssLineHeight::Normal => Some(relative(1.2)),
            };
        }
        FontStyle(v) => {
            use lightningcss::properties::font::FontStyle as FS;
            r.text.font_style = Some(match v {
                FS::Normal => gpui::FontStyle::Normal,
                FS::Italic => gpui::FontStyle::Italic,
                FS::Oblique(_) => {
                    unsupported(diagnostics, "font-style: oblique <angle> (angle ignored)");
                    gpui::FontStyle::Oblique
                }
            });
        }
        TextAlign(v) => {
            use gpui::TextAlign as G;
            let mapped = match v {
                CssTextAlign::Left | CssTextAlign::Start => Some(G::Left),
                CssTextAlign::Center => Some(G::Center),
                CssTextAlign::Right | CssTextAlign::End => Some(G::Right),
                _ => None,
            };
            match mapped {
                Some(m) => r.text.text_align = Some(m),
                None => unsupported(diagnostics, "text-align"),
            }
        }
        WhiteSpace(v) => {
            use lightningcss::properties::text::WhiteSpace as WS;
            r.text.white_space = Some(match v {
                WS::Normal => gpui::WhiteSpace::Normal,
                WS::NoWrap => gpui::WhiteSpace::Nowrap,
                WS::Pre => {
                    // gpui collapses runs of whitespace unconditionally; only
                    // the WRAP half of `pre` survives the trip.
                    unsupported(diagnostics, "white-space: pre (whitespace is still collapsed)");
                    gpui::WhiteSpace::Nowrap
                }
                WS::PreWrap | WS::PreLine | WS::BreakSpaces => {
                    unsupported(
                        diagnostics,
                        "white-space: pre-wrap/pre-line/break-spaces (whitespace is still collapsed)",
                    );
                    gpui::WhiteSpace::Normal
                }
            });
        }
        TextOverflow(v, _) => {
            use lightningcss::properties::overflow::TextOverflow as TO;
            match v {
                TO::Ellipsis => r.text.text_overflow = Some(gpui::TextOverflow::Truncate("…".into())),
                // A refinement cannot express "clear", so `text-clip` after an
                // `text-ellipsis` is a no-op rather than a reset.
                TO::Clip => unsupported(
                    diagnostics,
                    "text-overflow: clip — cannot undo an earlier ellipsis",
                ),
            }
        }
        TextDecorationLine(v, _) => {
            use lightningcss::properties::text::TextDecorationLine as TDL;
            if v.contains(TDL::Underline) {
                r.text.underline = Some(gpui::UnderlineStyle::default());
            }
            if v.contains(TDL::LineThrough) {
                r.text.strikethrough = Some(gpui::StrikethroughStyle::default());
            }
            if v.intersects(TDL::Overline | TDL::Blink) {
                unsupported(diagnostics, "text-decoration-line: overline/blink");
            }
            if v.is_empty() {
                unsupported(
                    diagnostics,
                    "text-decoration-line: none — cannot undo an earlier underline",
                );
            }
        }
        Cursor(c) => {
            use lightningcss::properties::ui::CursorKeyword as K;
            let mapped = match c.keyword {
                K::Pointer => Some(gpui::CursorStyle::PointingHand),
                K::Default | K::Auto => Some(gpui::CursorStyle::Arrow),
                K::Text => Some(gpui::CursorStyle::IBeam),
                _ => None,
            };
            if let Some(m) = mapped {
                r.mouse_cursor = Some(m);
            }
        }
        // `backdrop-filter` — `blur()` is the one filter function the renderer
        // implements (gpui `BackdropBlurRect`, D3D11 Dual-Kawase pyramid).
        // Everything else in the list is advised rather than silently dropped.
        BackdropFilter(list, _) => apply_backdrop_filter(list, r, diagnostics),
        Custom(c) => match &c.name {
            // `--shading` is the ONE custom property vue-native interprets: a
            // GPU effect has no web equivalent, so it needs a spelling a
            // browser parses, stores and ignores rather than one a browser
            // DROPS. It is a real mapped property that merely happens to be
            // spelled as a custom property, so `channel()` keeps it on the
            // static channel and it lands here. Everything about it — grammar,
            // schemas, advisories, the inheritance divergence — lives in
            // `crate::effect`.
            lightningcss::properties::custom::CustomPropertyName::Custom(name)
                if name.0.as_ref() == effect::SHADING =>
            {
                if let Some(spec) = effect::parse(&c.value, diagnostics) {
                    r.vn_effect = Some(spec);
                }
            }
            // Every other `--x` travels the custom-property channel (see
            // `channel`) and never reaches the mapper.
            lightningcss::properties::custom::CustomPropertyName::Custom(_) => {}
            // Not a custom property at all — an unknown/vendor-prefixed one
            // (`-webkit-line-clamp`), which is a real "we do not map this".
            //
            // `object-fit` is a REAL CSS property that lightningcss simply has
            // no typed representation for, so it lands in this bucket too; it
            // is intercepted here and parsed by hand.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "object-fit" =>
            {
                if let Some(fit) = object_fit(&c.value, diagnostics) {
                    x.object_fit = Some(fit);
                }
            }
            // `pointer-events` is a real CSS property that lightningcss has no
            // typed representation for either. `none` is expressed by giving
            // the node no hitbox at all: no listeners, no hover/active rungs
            // (see `vn-tree`'s renderer). It therefore stops REACTING; a click
            // still does not fall through to whatever is painted underneath,
            // because gpui resolves hits per hitbox rather than per pixel.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "pointer-events" =>
            {
                match single_ident(&c.value) {
                    Some("none") => x.pointer_events_none = Some(true),
                    Some("auto") => x.pointer_events_none = Some(false),
                    other => once(
                        diagnostics,
                        format!(
                            "unsupported value for pointer-events: {} — only `none` and `auto` \
                             are modelled (SVG-only values have nothing to apply to)",
                            other.unwrap_or("<not a keyword>")
                        ),
                    ),
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name) => {
                once(
                    diagnostics,
                    format!("property not mapped (v1): {}", name.0.as_ref()),
                );
            }
        },
        // A KNOWN property whose value did not parse into a typed value. These
        // are `var()`-composed and travel the dynamic channel, so reaching the
        // mapper means the value was STILL unparsed after substitution — an
        // unbalanced `var(` or a token stream that is not valid for the
        // property. Kept as a defensive advisory rather than silence.
        Unparsed(u) => {
            once(
                diagnostics,
                format!(
                    "`{}` value is still unresolved after var() substitution (v1)",
                    u.property_id.name()
                ),
            );
        }
        other => {
            once(
                diagnostics,
                format!("property not mapped (v1): {}", prop_name(other)),
            );
        }
    }
}

/// `transform` — the `translate()` half only.
///
/// gpui applies transformation matrices to sprites (SVG paths), never to laid
/// out element trees, so there is no way to rotate, scale or skew a box. What
/// there IS a way to do is MOVE one, and translation is what the reused
/// component libraries actually need: shadcn centres its dialog with
/// `translate(-50%, -50%)`, slides its switch thumb with `translateX(1.25rem)`
/// and drives its progress bar with `translateX(-38%)`.
///
/// The move is applied as a layout offset by the renderer, so — unlike a real
/// transform — it participates in layout. For an absolutely positioned box
/// (every centring case) the two are identical; for an in-flow one the
/// following siblings shift too, which is what the advisory says.
///
/// Tailwind always emits the full identity chain
/// (`translate(…) rotate(0) skewX(0) skewY(0) scaleX(1) scaleY(1)`), so
/// identity components are recognised and pass silently — only a REAL rotate/
/// scale/skew is advised about.
fn apply_transform(
    list: &lightningcss::properties::transform::TransformList,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::properties::transform::Transform as T;
    use lightningcss::values::percentage::NumberOrPercentage;

    fn len(v: &LengthPercentage) -> Option<TranslateLen> {
        match v {
            DimensionPercentage::Dimension(l) => length_value_px(l).map(TranslateLen::Px),
            DimensionPercentage::Percentage(p) => Some(TranslateLen::Percent(p.0)),
            DimensionPercentage::Calc(_) => None,
        }
    }
    fn is_one(v: &NumberOrPercentage) -> bool {
        match v {
            NumberOrPercentage::Number(n) => *n == 1.0,
            NumberOrPercentage::Percentage(p) => p.0 == 1.0,
        }
    }

    let zero = TranslateLen::Px(0.0);
    let (mut tx, mut ty) = (zero, zero);
    let mut moved = false;
    let mut dropped: Vec<&str> = Vec::new();
    for t in &list.0 {
        match t {
            T::Translate(a, b) => match (len(a), len(b)) {
                (Some(a), Some(b)) => {
                    (tx, ty) = (a, b);
                    moved = true;
                }
                _ => dropped.push("translate() with a calc() length"),
            },
            T::TranslateX(a) => match len(a) {
                Some(a) => {
                    tx = a;
                    moved = true;
                }
                None => dropped.push("translateX() with a calc() length"),
            },
            T::TranslateY(b) => match len(b) {
                Some(b) => {
                    ty = b;
                    moved = true;
                }
                None => dropped.push("translateY() with a calc() length"),
            },
            // `translate3d(x, y, 0)` is what @floating-ui writes for EVERY
            // positioned element (it opts into GPU compositing); the Z axis is
            // always 0 there, and a non-zero one has nothing to mean without a
            // perspective transform.
            T::Translate3d(a, b, z) => match (len(a), len(b)) {
                (Some(a), Some(b)) => {
                    (tx, ty) = (a, b);
                    moved = true;
                    if !matches!(css_length_px(z), Some(0.0)) {
                        dropped.push("the Z axis of a 3D translate");
                    }
                }
                _ => dropped.push("translate3d() with a calc() length"),
            },
            // Identity components (what Tailwind's transform stack is made of
            // when only a translate utility is applied) are genuinely nothing.
            T::Rotate(a) | T::RotateZ(a) | T::RotateX(a) | T::RotateY(a) if a.to_degrees() == 0.0 => {}
            T::SkewX(a) | T::SkewY(a) if a.to_degrees() == 0.0 => {}
            T::Skew(a, b) if a.to_degrees() == 0.0 && b.to_degrees() == 0.0 => {}
            T::ScaleX(s) | T::ScaleY(s) | T::ScaleZ(s) if is_one(s) => {}
            T::Scale(a, b) if is_one(a) && is_one(b) => {}
            T::Rotate(_) | T::RotateX(_) | T::RotateY(_) | T::RotateZ(_) | T::Rotate3d(..) => {
                dropped.push("rotate()")
            }
            T::Scale(..) | T::ScaleX(_) | T::ScaleY(_) | T::ScaleZ(_) | T::Scale3d(..) => {
                dropped.push("scale()")
            }
            T::Skew(..) | T::SkewX(_) | T::SkewY(_) => dropped.push("skew()"),
            T::Matrix(_) | T::Matrix3d(_) => dropped.push("matrix()"),
            T::Perspective(_) => dropped.push("perspective()"),
            T::TranslateZ(_) => dropped.push("the Z axis of a 3D translate"),
        }
    }
    if moved {
        x.translate = Some((tx, ty));
    }
    if !dropped.is_empty() {
        dropped.sort_unstable();
        dropped.dedup();
        once(
            diagnostics,
            format!(
                "`transform` supports translate() only (v1) — dropped {}. gpui applies no matrix \
                 to a laid out box; the translate itself is applied as a LAYOUT offset, so an \
                 in-flow element's siblings move with it",
                dropped.join(", ")
            ),
        );
    }
}

/// `backdrop-filter` → `StyleRefinement::backdrop_blur`.
///
/// The renderer implements exactly one filter function, `blur()`. The CSS
/// grammar allows a whole list, so the compat rule is: apply the blur if there
/// is one, advise about everything that did not survive, and never fail
/// silently.
///
/// `backdrop-filter: none` maps to a 0px radius rather than "undeclared" — a
/// refinement has no way to say "explicitly nothing", and 0px is what actually
/// cancels a blur inherited from an earlier rule in the cascade.
fn apply_backdrop_filter(
    list: &lightningcss::properties::effects::FilterList,
    r: &mut StyleRefinement,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::properties::effects::{Filter, FilterList};

    let filters = match list {
        FilterList::None => {
            r.backdrop_blur = Some(px(0.0));
            return;
        }
        FilterList::Filters(filters) => filters,
    };

    let mut blur_radius = None;
    let mut others = Vec::new();
    for filter in filters.iter() {
        match filter {
            Filter::Blur(length) => match css_length_px(length) {
                Some(radius) if blur_radius.is_none() => blur_radius = Some(radius.max(0.0)),
                Some(_) => others.push("a second blur()"),
                // `blur(calc(...))`, `blur(1vw)`, … — the length converter is
                // the same one every other property uses.
                None => others.push("blur() with a non-absolute length"),
            },
            Filter::Brightness(_) => others.push("brightness()"),
            Filter::Contrast(_) => others.push("contrast()"),
            Filter::Grayscale(_) => others.push("grayscale()"),
            Filter::HueRotate(_) => others.push("hue-rotate()"),
            Filter::Invert(_) => others.push("invert()"),
            Filter::Opacity(_) => others.push("opacity()"),
            Filter::Saturate(_) => others.push("saturate()"),
            Filter::Sepia(_) => others.push("sepia()"),
            Filter::DropShadow(_) => others.push("drop-shadow()"),
            Filter::Url(_) => others.push("url() SVG filters"),
        }
    }

    if let Some(radius) = blur_radius {
        r.backdrop_blur = Some(px(radius));
    }
    if !others.is_empty() {
        once(
            diagnostics,
            format!(
                "backdrop-filter: only blur() is implemented (v1) — dropped {}",
                others.join(", ")
            ),
        );
    }
}

/// gpui models exactly two border styles. `none`/`hidden` are NOT a third: per
/// CSS they compute `border-width` to 0, which is what we do here — that is
/// also what makes Tailwind's `border-none` render as no border.
fn set_border_style(r: &mut StyleRefinement, s: LineStyle, diagnostics: &mut Vec<String>) {
    if matches!(s, LineStyle::None | LineStyle::Hidden) {
        let zero: AbsoluteLength = px(0.0).into();
        r.border_widths.top = Some(zero);
        r.border_widths.right = Some(zero);
        r.border_widths.bottom = Some(zero);
        r.border_widths.left = Some(zero);
        return;
    }
    r.border_style = Some(match s {
        LineStyle::Dashed => gpui::BorderStyle::Dashed,
        LineStyle::Solid => gpui::BorderStyle::Solid,
        _ => {
            unsupported(
                diagnostics,
                "border-style (dotted/double/groove/ridge/inset/outset render as solid)",
            );
            gpui::BorderStyle::Solid
        }
    });
}

fn lp_definite_of_lpa(v: &LengthPercentageOrAuto) -> Option<DefiniteLength> {
    match v {
        LengthPercentageOrAuto::LengthPercentage(lp) => lp_definite(lp),
        LengthPercentageOrAuto::Auto => None,
    }
}

fn overflow(k: OverflowKeyword) -> Overflow {
    match k {
        OverflowKeyword::Hidden | OverflowKeyword::Clip => Overflow::Hidden,
        OverflowKeyword::Scroll | OverflowKeyword::Auto => Overflow::Scroll,
        OverflowKeyword::Visible => Overflow::Visible,
    }
}

fn border_width_px(w: &lightningcss::properties::border::BorderSideWidth) -> Option<f32> {
    use lightningcss::properties::border::BorderSideWidth as W;
    match w {
        W::Length(l) => css_length_px(l),
        W::Thin => Some(1.0),
        W::Medium => Some(3.0),
        W::Thick => Some(5.0),
    }
}

fn prop_name(p: &Property) -> String {
    let d = format!("{p:?}");
    d.split(['(', ' ', '{']).next().unwrap_or("?").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a declaration list the way a rule body would be parsed.
    fn map(body: &str) -> (StyleRefinement, Vec<String>) {
        let mut diagnostics = Vec::new();
        let r = parse_inline(body, &mut diagnostics);
        (r, diagnostics)
    }

    fn clean(body: &str) -> StyleRefinement {
        let (r, d) = map(body);
        assert!(d.is_empty(), "{body:?} produced {d:?}");
        r
    }

    // ---- overflow longhands (Tailwind `overflow-x-*` / `overflow-y-*`) ----

    #[test]
    fn overflow_longhands_set_one_axis_each() {
        let r = clean("overflow-x: hidden");
        assert_eq!(r.overflow.x, Some(Overflow::Hidden));
        assert_eq!(r.overflow.y, None, "overflow-x must not touch the y axis");

        let r = clean("overflow-y: scroll");
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));
        assert_eq!(r.overflow.x, None);

        // `auto` is Scroll (what makes the box scrollable), `visible` is neither.
        assert_eq!(clean("overflow-y: auto").overflow.y, Some(Overflow::Scroll));
        assert_eq!(
            clean("overflow-x: visible").overflow.x,
            Some(Overflow::Visible)
        );
    }

    #[test]
    fn a_longhand_after_the_shorthand_wins_on_its_axis_only() {
        let r = clean("overflow: hidden; overflow-y: scroll");
        assert_eq!(r.overflow.x, Some(Overflow::Hidden));
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));
    }

    // ---- gap longhands (Tailwind `gap-x-*` / `gap-y-*`) ------------------

    #[test]
    fn gap_longhands_map_to_the_matching_axis() {
        // CSS row-gap is the VERTICAL gap -> gpui's gap.height.
        let r = clean("row-gap: 4px; column-gap: 8px");
        assert_eq!(r.gap.height, Some(px(4.0).into()));
        assert_eq!(r.gap.width, Some(px(8.0).into()));
    }

    // ---- border width / style / radius longhands -------------------------

    #[test]
    fn border_width_shorthand_and_longhands() {
        let r = clean("border-width: 2px");
        let two: AbsoluteLength = px(2.0).into();
        assert_eq!(r.border_widths.top, Some(two));
        assert_eq!(r.border_widths.left, Some(two));

        let r = clean("border-top-width: 4px");
        assert_eq!(r.border_widths.top, Some(px(4.0).into()));
        assert_eq!(r.border_widths.bottom, None);
    }

    #[test]
    fn border_style_none_computes_the_widths_to_zero() {
        // Exactly what a browser does — and what makes `border-none` work.
        let r = clean("border: 3px solid red; border-style: none");
        let zero: AbsoluteLength = px(0.0).into();
        assert_eq!(r.border_widths.top, Some(zero));
        assert_eq!(r.border_widths.right, Some(zero));
        assert_eq!(r.border_widths.bottom, Some(zero));
        assert_eq!(r.border_widths.left, Some(zero));
    }

    #[test]
    fn border_style_dashed_and_an_advisory_for_the_rest() {
        assert_eq!(
            clean("border-style: dashed").border_style,
            Some(gpui::BorderStyle::Dashed)
        );
        let (r, d) = map("border-style: dotted");
        assert_eq!(r.border_style, Some(gpui::BorderStyle::Solid));
        assert!(d.iter().any(|m| m.contains("border-style")), "{d:?}");
    }

    #[test]
    fn corner_radius_longhands() {
        let r = clean("border-top-left-radius: 8px; border-bottom-right-radius: 2px");
        assert_eq!(r.corner_radii.top_left, Some(px(8.0).into()));
        assert_eq!(r.corner_radii.bottom_right, Some(px(2.0).into()));
        assert_eq!(r.corner_radii.top_right, None);
    }

    // ---- flex shorthand / basis / wrap -----------------------------------

    #[test]
    fn flex_shorthand_fills_all_three_fields() {
        // Tailwind `flex-1`.
        let r = clean("flex: 1 1 0%");
        assert_eq!(r.flex_grow, Some(1.0));
        assert_eq!(r.flex_shrink, Some(1.0));
        assert_eq!(r.flex_basis, Some(Length::Definite(relative(0.0))));

        // Tailwind `flex-none`.
        let r = clean("flex: none");
        assert_eq!(r.flex_grow, Some(0.0));
        assert_eq!(r.flex_shrink, Some(0.0));
        assert_eq!(r.flex_basis, Some(Length::Auto));
    }

    #[test]
    fn flex_wrap_maps_all_three_keywords() {
        assert_eq!(clean("flex-wrap: wrap").flex_wrap, Some(gpui::FlexWrap::Wrap));
        assert_eq!(
            clean("flex-wrap: wrap-reverse").flex_wrap,
            Some(gpui::FlexWrap::WrapReverse)
        );
        assert_eq!(
            clean("flex-wrap: nowrap").flex_wrap,
            Some(gpui::FlexWrap::NoWrap)
        );
    }

    #[test]
    fn align_self_and_align_content() {
        assert_eq!(
            clean("align-self: center").align_self,
            Some(gpui::AlignItems::Center)
        );
        assert_eq!(
            clean("align-content: space-between").align_content,
            Some(gpui::AlignContent::SpaceBetween)
        );
        // `self-auto` cannot be expressed as a refinement — it must say so.
        let (r, d) = map("align-self: auto");
        assert_eq!(r.align_self, None);
        assert!(d.iter().any(|m| m.contains("align-self")), "{d:?}");
    }

    // ---- grid templates (Tailwind `grid-cols-N`) -------------------------

    #[test]
    fn grid_cols_maps_tailwinds_repeat_minmax() {
        let r = clean("grid-template-columns: repeat(3, minmax(0, 1fr))");
        assert_eq!(
            r.grid_cols,
            Some(gpui::GridTemplate {
                repeat: 3,
                min_size: gpui::GridTemplateMinSize::Zero,
            })
        );
        let r = clean("grid-template-rows: repeat(2, minmax(0, 1fr))");
        assert_eq!(r.grid_rows.map(|t| t.repeat), Some(2));
    }

    #[test]
    fn an_unmodelable_track_list_is_an_advisory_not_a_wrong_grid() {
        let (r, d) = map("grid-template-columns: 200px 1fr");
        assert_eq!(r.grid_cols, None);
        assert!(d.iter().any(|m| m.contains("grid-template-columns")), "{d:?}");
    }

    // ---- typography / visibility / aspect-ratio --------------------------

    #[test]
    fn typography_longhands() {
        assert_eq!(
            clean("font-style: italic").text.font_style,
            Some(gpui::FontStyle::Italic)
        );
        assert_eq!(
            clean("white-space: nowrap").text.white_space,
            Some(gpui::WhiteSpace::Nowrap)
        );
        assert!(clean("text-decoration-line: underline")
            .text
            .underline
            .is_some());
        assert!(clean("text-decoration-line: line-through")
            .text
            .strikethrough
            .is_some());
        assert!(matches!(
            clean("text-overflow: ellipsis").text.text_overflow,
            Some(gpui::TextOverflow::Truncate(_))
        ));
    }

    #[test]
    fn visibility_and_aspect_ratio() {
        assert_eq!(
            clean("visibility: hidden").visibility,
            Some(gpui::Visibility::Hidden)
        );
        assert_eq!(clean("aspect-ratio: 16 / 9").aspect_ratio, Some(16.0 / 9.0));
    }

    // ---- diagnostics are ONE-SHOT ----------------------------------------

    /// Custom properties are STORED, not advised about: they leave the mapper
    /// on their own channel and land in the element's [`Vars`] map.
    #[test]
    fn custom_properties_travel_their_own_channel() {
        let mut d = Vec::new();
        let inline = parse_inline_style(
            "--tw-shadow: 0 0 #0000; --tw-ring-color: red; --whatever: 3; color: blue",
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            inline.custom,
            vec![
                ("--tw-shadow".to_string(), "0 0 #0000".to_string()),
                ("--tw-ring-color".to_string(), "red".to_string()),
                ("--whatever".to_string(), "3".to_string()),
            ]
        );
        // …and the mapped declaration in the same body is unaffected.
        assert!(inline.refinement.is_some_and(|r| r.text.color.is_some()));
    }

    /// A `var()`-composed value is deferred, not dropped: it rides the dynamic
    /// channel as text and is substituted per element.
    #[test]
    fn a_var_composed_value_is_deferred_to_computed_value_time() {
        let mut d = Vec::new();
        let inline = parse_inline_style("box-shadow: var(--tw-shadow)", &mut d);
        assert!(d.is_empty(), "{d:?}");
        assert!(inline.refinement.is_none());
        assert_eq!(
            inline.dynamic,
            vec![("box-shadow".to_string(), "var(--tw-shadow)".to_string())]
        );
    }

    /// …and once the variable is known, the ordinary mapper arm runs.
    #[test]
    fn apply_dynamic_substitutes_then_maps() {
        let mut d = Vec::new();
        let vars = crate::vars::resolve(
            &Rc::new(Vars::default()),
            &[("--tw-shadow".to_string(), "0 1px 2px 0 rgb(0 0 0 / 0.5)".to_string())],
            &mut d,
        );
        let mut r = StyleRefinement::default();
        let mut x = Extras::default();
        apply_dynamic("box-shadow", "var(--tw-shadow)", &vars, &mut r, &mut x, &mut d);
        assert!(d.is_empty(), "{d:?}");
        let shadows = r.box_shadow.expect("box-shadow mapped");
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].blur_radius, px(2.0));
    }

    /// An unresolvable reference drops the declaration and says why — the CSS
    /// "invalid at computed-value time" rule, never a silent nothing.
    #[test]
    fn an_unresolvable_var_is_an_advisory_not_silence() {
        let mut d = Vec::new();
        let mut r = StyleRefinement::default();
        let mut x = Extras::default();
        apply_dynamic("box-shadow", "var(--nope)", &Vars::default(), &mut r, &mut x, &mut d);
        assert_eq!(r.box_shadow, None);
        assert!(
            d.iter().any(|m| m.contains("invalid at computed-value time") && m.contains("--nope")),
            "{d:?}"
        );
    }

    #[test]
    fn a_repeated_unmapped_property_is_reported_once() {
        let (_, d) = map("letter-spacing: 1px; letter-spacing: 2px; letter-spacing: 3px");
        assert_eq!(d.len(), 1, "{d:?}");
    }

    #[test]
    fn currentcolor_is_advised_rather_than_dropped_in_silence() {
        let (r, d) = map("background-color: currentColor");
        assert_eq!(r.background, None);
        assert!(d.iter().any(|m| m.contains("background-color")), "{d:?}");
    }

    // ---- calc() over absolute lengths ------------------------------------

    #[test]
    fn calc_sums_and_products_resolve_to_pixels() {
        // shadcn's own radius scale, once `--radius` resolves: `rounded-md` is
        // `calc(var(--radius) - 2px)`.
        assert_eq!(
            clean("border-radius: calc(0.5rem - 2px)").corner_radii.top_left,
            Some(px(6.0).into())
        );
        assert_eq!(clean("padding-top: calc(1rem + 4px)").padding.top, Some(px(20.0).into()));
        assert_eq!(clean("width: calc(2 * 8px)").size.width, Some(Length::Definite(px(16.0).into())));
        assert_eq!(
            clean("border-top-width: calc(4px + 1px)").border_widths.top,
            Some(px(5.0).into())
        );
        assert_eq!(
            clean("width: min(100px, 40px)").size.width,
            Some(Length::Definite(px(40.0).into()))
        );
        assert_eq!(
            clean("width: clamp(10px, 4px, 100px)").size.width,
            Some(Length::Definite(px(10.0).into()))
        );
    }

    /// A percentage inside a `calc()` needs the containing block, which does not
    /// exist before layout — so it stays unresolved rather than being guessed.
    #[test]
    fn a_percentage_inside_calc_is_not_guessed_at() {
        let (r, _) = map("width: calc(100% - 8px)");
        assert_eq!(r.size.width, None);
    }

    // ---- positioning (compat roadmap G3) ---------------------------------

    fn extras(body: &str) -> (Extras, Vec<String>) {
        let mut d = Vec::new();
        let inline = parse_inline_style(body, &mut d);
        (inline.extras, d)
    }

    #[test]
    fn absolute_and_inset_map_straight_through() {
        let r = clean("position: absolute; inset: 0; left: 50%; top: 8px");
        assert_eq!(r.position, Some(gpui::Position::Absolute));
        assert_eq!(r.inset.right, Some(Length::Definite(px(0.0).into())));
        assert_eq!(r.inset.left, Some(Length::Definite(relative(0.5))));
        assert_eq!(r.inset.top, Some(Length::Definite(px(8.0).into())));
    }

    #[test]
    fn fixed_is_absolute_plus_a_deferred_layer_and_says_so() {
        let (x, d) = extras("position: fixed");
        assert_eq!(x.fixed, Some(true));
        assert_eq!(x.deferred_priority(), Some(0));
        assert!(d.iter().any(|m| m.contains("position: fixed")), "{d:?}");
        // …and a later `relative` really undoes it.
        let (x, _) = extras("position: fixed; position: relative");
        assert_eq!(x.fixed, Some(false));
        assert_eq!(x.deferred_priority(), None);
    }

    #[test]
    fn a_positive_z_index_becomes_a_deferred_priority() {
        let (x, d) = extras("z-index: 50");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.z_index, Some(50));
        assert_eq!(x.deferred_priority(), Some(50));

        // A negative one has no lever at all — and must say so.
        let (x, d) = extras("z-index: -1");
        assert_eq!(x.deferred_priority(), None);
        assert!(d.iter().any(|m| m.contains("negative `z-index`")), "{d:?}");
    }

    #[test]
    fn pointer_events_maps_both_keywords() {
        assert_eq!(extras("pointer-events: none").0.pointer_events_none, Some(true));
        assert_eq!(extras("pointer-events: auto").0.pointer_events_none, Some(false));
        let (x, d) = extras("pointer-events: visiblePainted");
        assert_eq!(x.pointer_events_none, None);
        assert!(d.iter().any(|m| m.contains("pointer-events")), "{d:?}");
    }

    #[test]
    fn translate_is_kept_in_its_authored_unit() {
        let (x, d) = extras("transform: translate(-50%, -50%)");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            x.translate,
            Some((TranslateLen::Percent(-0.5), TranslateLen::Percent(-0.5)))
        );
        assert_eq!(TranslateLen::Percent(-0.5).to_px(400.0), -200.0);

        let (x, _) = extras("transform: translateX(1.25rem)");
        assert_eq!(x.translate, Some((TranslateLen::Px(20.0), TranslateLen::Px(0.0))));
    }

    /// Tailwind's `-translate-x-1/2` emits the whole identity chain; only a
    /// REAL rotate/scale/skew is worth an advisory.
    #[test]
    fn tailwinds_identity_transform_chain_is_silent() {
        let (x, d) = extras(
            "transform: translate(-50%, -50%) rotate(0) skewX(0) skewY(0) scaleX(1) scaleY(1)",
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            x.translate,
            Some((TranslateLen::Percent(-0.5), TranslateLen::Percent(-0.5)))
        );

        let (x, d) = extras("transform: translateY(4px) rotate(45deg) scale(1.5)");
        assert_eq!(x.translate, Some((TranslateLen::Px(0.0), TranslateLen::Px(4.0))));
        assert!(
            d.iter().any(|m| m.contains("rotate()") && m.contains("scale()")),
            "{d:?}"
        );
    }

    // ---- the end-to-end Tailwind card ------------------------------------

    #[test]
    fn the_demo_cards_utilities_all_render() {
        // Byte-for-byte what Tailwind v3.4 emits for
        // `flex flex-col gap-2 p-4 rounded-xl bg-white/10 text-sm font-semibold`.
        let r = clean(
            "display: flex; flex-direction: column; gap: 0.5rem; padding: 1rem; \
             border-radius: 0.75rem; background-color: rgb(255 255 255 / 0.1); \
             font-size: 0.875rem; line-height: 1.25rem; font-weight: 600",
        );
        assert_eq!(r.display, Some(gpui::Display::Flex));
        assert_eq!(r.flex_direction, Some(gpui::FlexDirection::Column));
        assert_eq!(r.gap.width, Some(px(8.0).into()));
        assert_eq!(r.gap.height, Some(px(8.0).into()));
        assert_eq!(r.padding.top, Some(px(16.0).into()));
        assert_eq!(r.corner_radii.top_left, Some(px(12.0).into()));
        assert_eq!(r.text.font_size, Some(px(14.0).into()));
        assert_eq!(r.text.font_weight, Some(gpui::FontWeight(600.0)));
        assert!(matches!(r.background, Some(gpui::Fill::Color(_))));
    }
}
