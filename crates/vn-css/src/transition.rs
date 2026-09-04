//! CSS `transition` — parsing and the timing model.
//!
//! lightningcss hands us either the `transition` shorthand or the four
//! longhands (`transition-property/-duration/-delay/-timing-function`). This
//! module folds a declaration block's worth of those into the per-rule
//! `Vec<TransitionSpec>` that the cascade carries on `ComputedStates`, and
//! implements the easing evaluation the animator samples every frame.
//!
//! Browser-compat notes (owner policy: never diverge silently):
//! - `transition-property` names vn-css cannot interpolate become
//!   [`TransitionProperty::Other`]; the animator emits a one-shot advisory.
//! - Shorthand names in `transition-property` (`padding`, `margin`,
//!   `border-radius`, `gap`, `background`, `border`) expand to their longhands,
//!   each inheriting that list entry's duration/delay/easing — as in CSS.
//! - Negative `transition-delay` is clamped to zero with an advisory (v1);
//!   CSS starts such a transition already partway through.

use std::rc::Rc;
use std::time::Duration;

use lightningcss::properties::{Property, PropertyId};
use lightningcss::values::easing::{EasingFunction, StepPosition as CssStepPosition};
use lightningcss::values::time::Time;

// ---------------------------------------------------------------------------
// the spec
// ---------------------------------------------------------------------------

/// A property a `transition` can name. Everything except [`Self::All`] and
/// [`Self::Other`] is a *concrete* slot in `gpui::StyleRefinement` that the
/// animator knows how to interpolate.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TransitionProperty {
    /// `opacity`
    Opacity,
    /// `color` (text color)
    Color,
    /// `background-color` (solid fills only)
    BackgroundColor,
    /// `border-color` (gpui keeps one color for all four sides)
    BorderColor,
    /// `text-decoration-color` — the colour of the underline / strikethrough,
    /// which gpui carries on the decoration itself rather than in a slot of
    /// its own.
    TextDecorationColor,
    /// SVG `fill`. Its USED value here is the element's `color`: gpui
    /// rasterises an SVG into a single-colour alpha mask and tints the sprite
    /// (`vn_tree::svg`), so a painted fill is that colour and an unpainted one
    /// (`fill: none`) is a discrete endpoint CSS does not interpolate either.
    Fill,
    /// SVG `stroke` — the same, for the other half of the geometry.
    Stroke,
    /// `width`
    Width,
    /// `height`
    Height,
    /// `min-width`
    MinWidth,
    /// `min-height`
    MinHeight,
    /// `max-width`
    MaxWidth,
    /// `max-height`
    MaxHeight,
    /// `padding-top`
    PaddingTop,
    /// `padding-right`
    PaddingRight,
    /// `padding-bottom`
    PaddingBottom,
    /// `padding-left`
    PaddingLeft,
    /// `margin-top`
    MarginTop,
    /// `margin-right`
    MarginRight,
    /// `margin-bottom`
    MarginBottom,
    /// `margin-left`
    MarginLeft,
    /// `border-top-left-radius`
    BorderTopLeftRadius,
    /// `border-top-right-radius`
    BorderTopRightRadius,
    /// `border-bottom-right-radius`
    BorderBottomRightRadius,
    /// `border-bottom-left-radius`
    BorderBottomLeftRadius,
    /// `row-gap`
    RowGap,
    /// `column-gap`
    ColumnGap,
    /// `font-size`
    FontSize,
    /// `transform` — the whole function list travels as one value, because
    /// that is how CSS interpolates it: matched lists interpolate
    /// componentwise (`rotate(0deg)` → `rotate(90deg)` sweeps through 45°),
    /// never entry-by-entry through the matrix, which would shear.
    Transform,
    /// `--shading` — the effect parameter vector.
    ///
    /// Deliberately NOT part of [`Self::animatable`], so `transition: all` does
    /// not cover it and it has to be named explicitly
    /// (`transition: --shading 300ms`). Two reasons: `all` in CSS means "every
    /// animatable *property*", and a custom property is not one — a browser
    /// would not animate `--shading` under `all` either, since animating a
    /// custom property needs an `@property` registration it does not have. And
    /// an effect quad is the one style here that can cost a render-target
    /// snapshot per frame, so opting into animating it should be explicit.
    Shading,
    /// `all` — expands to every animatable property at diff time.
    All,
    /// A property vn-css parses but cannot animate yet (one-shot advisory).
    Other(Rc<str>),
}

/// The number of concrete properties `all` expands to. `--shading`
/// ([`TransitionProperty::Shading`]) is transitionable but deliberately outside
/// this set — see that variant.
pub const ANIMATABLE_COUNT: usize = 29;

impl TransitionProperty {
    /// Every property vn-css can actually interpolate — the expansion of `all`.
    ///
    /// Returned by value (not as a `static`) because the enum carries an `Rc`
    /// in one variant and so is neither `Sync` nor const-promotable.
    pub fn animatable() -> [TransitionProperty; ANIMATABLE_COUNT] {
        use TransitionProperty::*;
        [
            Opacity,
            Color,
            BackgroundColor,
            BorderColor,
            TextDecorationColor,
            Fill,
            Stroke,
            Width,
            Height,
            MinWidth,
            MinHeight,
            MaxWidth,
            MaxHeight,
            PaddingTop,
            PaddingRight,
            PaddingBottom,
            PaddingLeft,
            MarginTop,
            MarginRight,
            MarginBottom,
            MarginLeft,
            BorderTopLeftRadius,
            BorderTopRightRadius,
            BorderBottomRightRadius,
            BorderBottomLeftRadius,
            RowGap,
            ColumnGap,
            FontSize,
            Transform,
        ]
    }

    /// The CSS name, for diagnostics.
    pub fn name(&self) -> &str {
        use TransitionProperty::*;
        match self {
            Opacity => "opacity",
            Color => "color",
            BackgroundColor => "background-color",
            BorderColor => "border-color",
            TextDecorationColor => "text-decoration-color",
            Fill => "fill",
            Stroke => "stroke",
            Width => "width",
            Height => "height",
            MinWidth => "min-width",
            MinHeight => "min-height",
            MaxWidth => "max-width",
            MaxHeight => "max-height",
            PaddingTop => "padding-top",
            PaddingRight => "padding-right",
            PaddingBottom => "padding-bottom",
            PaddingLeft => "padding-left",
            MarginTop => "margin-top",
            MarginRight => "margin-right",
            MarginBottom => "margin-bottom",
            MarginLeft => "margin-left",
            BorderTopLeftRadius => "border-top-left-radius",
            BorderTopRightRadius => "border-top-right-radius",
            BorderBottomRightRadius => "border-bottom-right-radius",
            BorderBottomLeftRadius => "border-bottom-left-radius",
            RowGap => "row-gap",
            ColumnGap => "column-gap",
            FontSize => "font-size",
            Transform => "transform",
            Shading => "--shading",
            All => "all",
            Other(name) => name,
        }
    }
}

/// Where a `steps()` function jumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepPosition {
    /// `jump-start` / `start`
    JumpStart,
    /// `jump-end` / `end` (the default)
    JumpEnd,
    /// `jump-none`
    JumpNone,
    /// `jump-both`
    JumpBoth,
}

/// A CSS easing function, with evaluation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    /// `linear`
    Linear,
    /// `ease` — `cubic-bezier(0.25, 0.1, 0.25, 1)`
    Ease,
    /// `ease-in` — `cubic-bezier(0.42, 0, 1, 1)`
    EaseIn,
    /// `ease-out` — `cubic-bezier(0, 0, 0.58, 1)`
    EaseOut,
    /// `ease-in-out` — `cubic-bezier(0.42, 0, 0.58, 1)`
    EaseInOut,
    /// `cubic-bezier(x1, y1, x2, y2)`
    CubicBezier(f32, f32, f32, f32),
    /// `steps(count, position)`
    Steps(u32, StepPosition),
}

impl Default for Easing {
    fn default() -> Self {
        Easing::Ease
    }
}

impl Easing {
    /// Maps input progress (0..=1) to output progress. Cubic-bézier output may
    /// leave 0..=1 (overshoot) — that is correct CSS.
    pub fn eval(self, t: f32) -> f32 {
        match self {
            Easing::Linear => t,
            Easing::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
            Easing::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
            Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
            Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
            Easing::Steps(count, position) => steps(count, position, t),
        }
    }
}

/// One resolved transition: which property, for how long, after what delay,
/// shaped by which easing.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionSpec {
    /// The property this entry governs.
    pub property: TransitionProperty,
    /// `transition-duration`.
    pub duration: Duration,
    /// `transition-delay` (negative values are clamped to zero in v1).
    pub delay: Duration,
    /// `transition-timing-function`.
    pub easing: Easing,
}

/// The spec that governs `property`, or `None` when nothing does. Per CSS the
/// LAST matching entry in the list wins; `all` matches every property.
pub fn spec_for<'s>(
    specs: &'s [TransitionSpec],
    property: &TransitionProperty,
) -> Option<&'s TransitionSpec> {
    specs
        .iter()
        .rev()
        .find(|s| s.property == *property || s.property == TransitionProperty::All)
}

/// The concrete properties a transition list covers, in first-mention order.
/// `all` expands to [`TransitionProperty::animatable`]; `Other` is skipped
/// (the animator reports it separately).
pub fn covered(specs: &[TransitionSpec]) -> Vec<TransitionProperty> {
    let mut out: Vec<TransitionProperty> = Vec::new();
    for s in specs {
        match &s.property {
            TransitionProperty::All => {
                for p in TransitionProperty::animatable() {
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
            TransitionProperty::Other(_) => {}
            p => {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// easing evaluation
// ---------------------------------------------------------------------------

/// The cubic-bézier basis with P0 = 0 and P3 = 1.
fn bezier(a: f32, b: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * t * a + 3.0 * mt * t * t * b + t * t * t
}

fn bezier_slope(a: f32, b: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * a + 6.0 * mt * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

/// Inverts x(t) with Newton-Raphson, falling back to bisection when the
/// derivative vanishes (flat control polygons) or Newton leaves 0..=1.
fn solve_t_for_x(x1: f32, x2: f32, x: f32) -> f32 {
    let mut t = x;
    for _ in 0..8 {
        let err = bezier(x1, x2, t) - x;
        if err.abs() < 1e-7 {
            return t;
        }
        let slope = bezier_slope(x1, x2, t);
        if slope.abs() < 1e-6 {
            break;
        }
        let next = t - err / slope;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        t = next;
    }

    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut t = x.clamp(0.0, 1.0);
    for _ in 0..48 {
        let v = bezier(x1, x2, t);
        if (v - x).abs() < 1e-7 {
            return t;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }
    t
}

fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    // CSS restricts the control points' x to 0..=1; a clamp keeps x(t)
    // monotonic so the inversion always converges.
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    bezier(y1, y2, solve_t_for_x(x1, x2, t))
}

/// The CSS Easing Level 1 step-easing algorithm.
fn steps(count: u32, position: StepPosition, t: f32) -> f32 {
    let n = count.max(1) as f32;
    let mut current = (t * n).floor();
    if matches!(position, StepPosition::JumpStart | StepPosition::JumpBoth) {
        current += 1.0;
    }
    if t >= 0.0 && current < 0.0 {
        current = 0.0;
    }
    let jumps = match position {
        StepPosition::JumpStart | StepPosition::JumpEnd => n,
        StepPosition::JumpNone => n - 1.0,
        StepPosition::JumpBoth => n + 1.0,
    };
    if jumps <= 0.0 {
        // `steps(1, jump-none)` is invalid CSS; degrade to a constant.
        return 0.0;
    }
    if t <= 1.0 && current > jumps {
        current = jumps;
    }
    current / jumps
}

// ---------------------------------------------------------------------------
// parsing
// ---------------------------------------------------------------------------

/// Accumulates one declaration block's transition longhands/shorthand.
#[derive(Default)]
pub(crate) struct Collector {
    /// One inner `Vec` per list entry (a shorthand name expands in place).
    properties: Option<Vec<Vec<TransitionProperty>>>,
    durations: Option<Vec<Duration>>,
    delays: Option<Vec<Duration>>,
    easings: Option<Vec<Easing>>,
    seen: bool,
}

impl Collector {
    /// Consumes `p` when it is a transition declaration; returns whether it did.
    pub(crate) fn take(&mut self, p: &Property, diagnostics: &mut Vec<String>) -> bool {
        match p {
            Property::Transition(list, _) => {
                self.seen = true;
                let mut properties = Vec::with_capacity(list.len());
                let mut durations = Vec::with_capacity(list.len());
                let mut delays = Vec::with_capacity(list.len());
                let mut easings = Vec::with_capacity(list.len());
                for entry in list.iter() {
                    match expand(&entry.property) {
                        Expansion::None => {}
                        Expansion::Props(props) => properties.push(props),
                    }
                    durations.push(duration_of(&entry.duration, "transition-duration", diagnostics));
                    delays.push(duration_of(&entry.delay, "transition-delay", diagnostics));
                    easings.push(easing_of(&entry.timing_function, diagnostics));
                }
                // The shorthand resets all four longhands.
                self.properties = Some(properties);
                self.durations = Some(durations);
                self.delays = Some(delays);
                self.easings = Some(easings);
                true
            }
            Property::TransitionProperty(list, _) => {
                self.seen = true;
                let mut properties = Vec::with_capacity(list.len());
                for id in list.iter() {
                    match expand(id) {
                        Expansion::None => {}
                        Expansion::Props(props) => properties.push(props),
                    }
                }
                self.properties = Some(properties);
                true
            }
            Property::TransitionDuration(list, _) => {
                self.seen = true;
                self.durations = Some(
                    list.iter()
                        .map(|t| duration_of(t, "transition-duration", diagnostics))
                        .collect(),
                );
                true
            }
            Property::TransitionDelay(list, _) => {
                self.seen = true;
                self.delays = Some(
                    list.iter()
                        .map(|t| duration_of(t, "transition-delay", diagnostics))
                        .collect(),
                );
                true
            }
            Property::TransitionTimingFunction(list, _) => {
                self.seen = true;
                self.easings = Some(list.iter().map(|e| easing_of(e, diagnostics)).collect());
                true
            }
            Property::Unparsed(u) if is_transition_id(&u.property_id) => {
                self.seen = true;
                diagnostics.push(format!(
                    "unresolved value for {} (var()/env() in transitions is not resolved yet, v1)",
                    u.property_id.name()
                ));
                true
            }
            _ => false,
        }
    }

    /// Zips the four component lists into specs, cycling the shorter ones —
    /// `transition-property` decides how many transitions there are, exactly as
    /// CSS does. `None` means the block declared no transition at all.
    pub(crate) fn finish(self) -> Option<Vec<TransitionSpec>> {
        if !self.seen {
            return None;
        }
        // The initial value of `transition-property` is `all`, so a lone
        // `transition-duration: 200ms` means "all 200ms".
        let entries = self
            .properties
            .unwrap_or_else(|| vec![vec![TransitionProperty::All]]);
        if entries.is_empty() {
            // `transition: none` / `transition-property: none` — clears.
            return Some(Vec::new());
        }
        let durations = self.durations.unwrap_or_default();
        let delays = self.delays.unwrap_or_default();
        let easings = self.easings.unwrap_or_default();

        let pick = |list: &[Duration], i: usize| -> Duration {
            if list.is_empty() {
                Duration::ZERO
            } else {
                list[i % list.len()]
            }
        };

        let mut out = Vec::new();
        for (i, props) in entries.iter().enumerate() {
            let d = pick(&durations, i);
            let delay = pick(&delays, i);
            let e = if easings.is_empty() {
                Easing::Ease
            } else {
                easings[i % easings.len()]
            };
            for property in props {
                out.push(TransitionSpec {
                    property: property.clone(),
                    duration: d,
                    delay,
                    easing: e,
                });
            }
        }
        Some(out)
    }
}

fn is_transition_id(id: &PropertyId) -> bool {
    matches!(
        id,
        PropertyId::Transition(_)
            | PropertyId::TransitionProperty(_)
            | PropertyId::TransitionDuration(_)
            | PropertyId::TransitionDelay(_)
            | PropertyId::TransitionTimingFunction(_)
    )
}

enum Expansion {
    /// The `none` keyword — contributes no entry.
    None,
    Props(Vec<TransitionProperty>),
}

/// Maps a `transition-property` name onto the concrete slots it covers.
/// Shorthand names expand; unknown/unsupported ones become `Other`.
fn expand(id: &PropertyId) -> Expansion {
    use TransitionProperty::*;
    let name = id.name();
    let one = |p: TransitionProperty| Expansion::Props(vec![p]);
    match name {
        "none" => Expansion::None,
        "all" => one(All),
        "opacity" => one(Opacity),
        "color" => one(Color),
        // `background` is a shorthand whose only animatable part here is the color.
        "background" | "background-color" => one(BackgroundColor),
        // gpui keeps a single border color, so `border`/`border-color` collapse.
        "border" | "border-color" => one(BorderColor),
        // The other three names in Tailwind's `transition-colors`.
        "text-decoration-color" => one(TextDecorationColor),
        "fill" => one(Fill),
        "stroke" => one(Stroke),
        "width" => one(Width),
        "height" => one(Height),
        "min-width" => one(MinWidth),
        "min-height" => one(MinHeight),
        "max-width" => one(MaxWidth),
        "max-height" => one(MaxHeight),
        "padding" => Expansion::Props(vec![PaddingTop, PaddingRight, PaddingBottom, PaddingLeft]),
        "padding-top" => one(PaddingTop),
        "padding-right" => one(PaddingRight),
        "padding-bottom" => one(PaddingBottom),
        "padding-left" => one(PaddingLeft),
        "margin" => Expansion::Props(vec![MarginTop, MarginRight, MarginBottom, MarginLeft]),
        "margin-top" => one(MarginTop),
        "margin-right" => one(MarginRight),
        "margin-bottom" => one(MarginBottom),
        "margin-left" => one(MarginLeft),
        "border-radius" => Expansion::Props(vec![
            BorderTopLeftRadius,
            BorderTopRightRadius,
            BorderBottomRightRadius,
            BorderBottomLeftRadius,
        ]),
        "border-top-left-radius" => one(BorderTopLeftRadius),
        "border-top-right-radius" => one(BorderTopRightRadius),
        "border-bottom-right-radius" => one(BorderBottomRightRadius),
        "border-bottom-left-radius" => one(BorderBottomLeftRadius),
        "gap" => Expansion::Props(vec![RowGap, ColumnGap]),
        "row-gap" => one(RowGap),
        "column-gap" => one(ColumnGap),
        "font-size" => one(FontSize),
        // Tailwind's `transition-transform`, and shadcn's accordion chevron.
        "transform" => one(Transform),
        // lightningcss parses an unknown property id — including a custom
        // property — into `PropertyId::Custom`, whose `name()` keeps the
        // leading dashes. `--shading` is the one custom property vue-native
        // maps, so it is the one that animates.
        "--shading" => one(Shading),
        other => one(Other(Rc::from(other))),
    }
}

/// One `<time>` value. Shared with `keyframes.rs` — `animation-duration` and
/// `transition-duration` parse identically.
pub(crate) fn duration_of(t: &Time, what: &str, diagnostics: &mut Vec<String>) -> Duration {
    let ms = t.to_ms();
    if !ms.is_finite() {
        diagnostics.push(format!("non-finite {what} treated as 0s"));
        return Duration::ZERO;
    }
    if ms < 0.0 {
        // CSS starts such a transition already partway through; v1 clamps.
        diagnostics.push(format!(
            "negative {what} ({ms}ms) is not supported yet (v1) — clamped to 0s"
        ));
        return Duration::ZERO;
    }
    Duration::from_secs_f64(ms as f64 / 1000.0)
}

/// One `<easing-function>`. Shared with `keyframes.rs`.
pub(crate) fn easing_of(e: &EasingFunction, diagnostics: &mut Vec<String>) -> Easing {
    match e {
        EasingFunction::Linear => Easing::Linear,
        EasingFunction::Ease => Easing::Ease,
        EasingFunction::EaseIn => Easing::EaseIn,
        EasingFunction::EaseOut => Easing::EaseOut,
        EasingFunction::EaseInOut => Easing::EaseInOut,
        EasingFunction::CubicBezier { x1, y1, x2, y2 } => {
            if !(0.0..=1.0).contains(x1) || !(0.0..=1.0).contains(x2) {
                diagnostics.push(format!(
                    "cubic-bezier x control points must be within 0..1 ({x1}, {x2}) — clamped"
                ));
            }
            Easing::CubicBezier(x1.clamp(0.0, 1.0), *y1, x2.clamp(0.0, 1.0), *y2)
        }
        EasingFunction::Steps { count, position } => {
            let position = match position {
                CssStepPosition::Start => StepPosition::JumpStart,
                CssStepPosition::End => StepPosition::JumpEnd,
                CssStepPosition::JumpNone => StepPosition::JumpNone,
                CssStepPosition::JumpBoth => StepPosition::JumpBoth,
            };
            let mut count = (*count).max(1) as u32;
            if position == StepPosition::JumpNone && count < 2 {
                diagnostics.push("steps(<2, jump-none) is invalid CSS — treated as 2".into());
                count = 2;
            }
            Easing::Steps(count, position)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightningcss::declaration::DeclarationBlock;
    use lightningcss::stylesheet::ParserOptions;

    /// Parses a declaration body through lightningcss and collects transitions.
    fn parse(body: &str) -> (Option<Vec<TransitionSpec>>, Vec<String>) {
        let mut diagnostics = Vec::new();
        let block = DeclarationBlock::parse_string(body, ParserOptions::default())
            .expect("declaration block parses");
        let mut collector = Collector::default();
        for p in block
            .declarations
            .iter()
            .chain(block.important_declarations.iter())
        {
            assert!(
                collector.take(p, &mut diagnostics),
                "non-transition property in test body: {p:?}"
            );
        }
        (collector.finish(), diagnostics)
    }

    fn specs(body: &str) -> Vec<TransitionSpec> {
        parse(body).0.expect("block declared a transition")
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn shorthand_single_property() {
        let s = specs("transition: opacity 200ms ease-in 50ms");
        assert_eq!(
            s,
            vec![TransitionSpec {
                property: TransitionProperty::Opacity,
                duration: ms(200),
                delay: ms(50),
                easing: Easing::EaseIn,
            }]
        );
    }

    #[test]
    fn shorthand_defaults_to_all_with_ease() {
        let s = specs("transition: 300ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].property, TransitionProperty::All);
        assert_eq!(s[0].duration, ms(300));
        assert_eq!(s[0].delay, Duration::ZERO);
        assert_eq!(s[0].easing, Easing::Ease);
    }

    #[test]
    fn shorthand_comma_list() {
        let s = specs("transition: opacity 1s linear, color 2s ease-out 100ms");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].property, TransitionProperty::Opacity);
        assert_eq!(s[0].duration, Duration::from_secs(1));
        assert_eq!(s[0].easing, Easing::Linear);
        assert_eq!(s[1].property, TransitionProperty::Color);
        assert_eq!(s[1].duration, Duration::from_secs(2));
        assert_eq!(s[1].delay, ms(100));
        assert_eq!(s[1].easing, Easing::EaseOut);
    }

    #[test]
    fn longhands_zip_and_cycle() {
        let s = specs(
            "transition-property: opacity, color, width; \
             transition-duration: 100ms, 200ms; \
             transition-delay: 10ms; \
             transition-timing-function: linear, ease-in",
        );
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].duration, ms(100));
        assert_eq!(s[1].duration, ms(200));
        assert_eq!(s[2].duration, ms(100)); // cycled
        assert!(s.iter().all(|x| x.delay == ms(10)));
        assert_eq!(s[2].easing, Easing::Linear); // cycled
    }

    #[test]
    fn duration_only_longhand_means_all() {
        let s = specs("transition-duration: 250ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].property, TransitionProperty::All);
        assert_eq!(s[0].duration, ms(250));
    }

    #[test]
    fn transition_none_clears() {
        assert_eq!(specs("transition: none"), Vec::new());
        assert_eq!(specs("transition-property: none"), Vec::new());
    }

    #[test]
    fn shorthand_resets_earlier_longhands() {
        // The shorthand comes last, so the earlier duration must not survive.
        let s = specs("transition-duration: 5s; transition: opacity 100ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].duration, ms(100));
    }

    #[test]
    fn longhand_after_shorthand_overrides_component() {
        let s = specs("transition: opacity 100ms; transition-duration: 900ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].property, TransitionProperty::Opacity);
        assert_eq!(s[0].duration, ms(900));
    }

    #[test]
    fn shorthand_names_expand_to_longhands() {
        let s = specs("transition: padding 100ms, border-radius 200ms, gap 300ms");
        let names: Vec<&str> = s.iter().map(|x| x.property.name()).collect();
        assert_eq!(
            names,
            vec![
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
                "border-top-left-radius",
                "border-top-right-radius",
                "border-bottom-right-radius",
                "border-bottom-left-radius",
                "row-gap",
                "column-gap",
            ]
        );
        assert!(s[..4].iter().all(|x| x.duration == ms(100)));
        assert!(s[4..8].iter().all(|x| x.duration == ms(200)));
        assert!(s[8..].iter().all(|x| x.duration == ms(300)));
    }

    #[test]
    fn unsupported_property_becomes_other() {
        let s = specs("transition: filter 200ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].property, TransitionProperty::Other(Rc::from("filter")));
        assert_eq!(s[0].property.name(), "filter");
    }

    /// `transform` was the canonical *unsupported* property here until m13
    /// gave a laid out box a transformation matrix. It is a first-class
    /// animatable now — which is what makes shadcn's accordion chevron and
    /// `transition-transform` on a hover work.
    #[test]
    fn transform_is_a_first_class_animatable() {
        let s = specs("transition: transform 200ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].property, TransitionProperty::Transform);
        assert_eq!(s[0].property.name(), "transform");
        assert!(TransitionProperty::animatable().contains(&TransitionProperty::Transform));
    }

    #[test]
    fn negative_delay_is_clamped_with_advisory() {
        let (s, diags) = parse("transition: opacity 1s -200ms");
        assert_eq!(s.unwrap()[0].delay, Duration::ZERO);
        assert!(
            diags.iter().any(|d| d.contains("negative transition-delay")),
            "{diags:?}"
        );
    }

    #[test]
    fn seconds_and_milliseconds_both_parse() {
        assert_eq!(specs("transition: opacity 1.5s")[0].duration, ms(1500));
        assert_eq!(specs("transition: opacity 1500ms")[0].duration, ms(1500));
    }

    #[test]
    fn no_transition_declaration_yields_none() {
        let mut diagnostics = Vec::new();
        let block =
            DeclarationBlock::parse_string("color: red", ParserOptions::default()).unwrap();
        let mut collector = Collector::default();
        for p in block.declarations.iter() {
            assert!(!collector.take(p, &mut diagnostics));
        }
        assert!(collector.finish().is_none());
    }

    // -- spec_for / covered ------------------------------------------------

    #[test]
    fn spec_for_prefers_the_last_matching_entry() {
        let s = specs("transition: all 1s, opacity 2s");
        let opacity = spec_for(&s, &TransitionProperty::Opacity).unwrap();
        assert_eq!(opacity.duration, Duration::from_secs(2));
        let color = spec_for(&s, &TransitionProperty::Color).unwrap();
        assert_eq!(color.duration, Duration::from_secs(1));
        // `all` came first, so it still governs width.
        assert!(spec_for(&s, &TransitionProperty::Width).is_some());
    }

    #[test]
    fn covered_expands_all_and_skips_unsupported() {
        let s = specs("transition: all 1s, transform 1s");
        assert_eq!(covered(&s).len(), ANIMATABLE_COUNT);
        let s = specs("transition: filter 1s");
        assert!(covered(&s).is_empty());
        let s = specs("transition: transform 1s");
        assert_eq!(covered(&s), vec![TransitionProperty::Transform]);
        let s = specs("transition: opacity 1s, opacity 2s");
        assert_eq!(covered(&s), vec![TransitionProperty::Opacity]);
    }

    // -- easing ------------------------------------------------------------

    const EPS: f32 = 1e-3;

    #[test]
    fn easing_endpoints_are_exact() {
        for e in [
            Easing::Linear,
            Easing::Ease,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::CubicBezier(0.7, -0.4, 0.3, 1.4),
        ] {
            assert_eq!(e.eval(0.0), 0.0, "{e:?}");
            assert_eq!(e.eval(1.0), 1.0, "{e:?}");
        }
    }

    #[test]
    fn cubic_bezier_matches_known_values() {
        // Reference values for the CSS keyword curves at t = 0.5.
        assert!((Easing::Ease.eval(0.5) - 0.802_4).abs() < EPS);
        assert!((Easing::EaseIn.eval(0.5) - 0.315_4).abs() < EPS);
        assert!((Easing::EaseOut.eval(0.5) - 0.684_6).abs() < EPS);
        assert!((Easing::EaseInOut.eval(0.5) - 0.5).abs() < EPS);
        // The keywords are exactly their cubic-bezier() equivalents.
        for t in [0.1, 0.25, 0.5, 0.75, 0.9] {
            assert!(
                (Easing::Ease.eval(t) - Easing::CubicBezier(0.25, 0.1, 0.25, 1.0).eval(t)).abs()
                    < 1e-5
            );
        }
    }

    #[test]
    fn cubic_bezier_identity_and_monotonicity() {
        let identity = Easing::CubicBezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            assert!((identity.eval(t) - t).abs() < EPS, "t={t}");
        }
        let mut prev = -1.0;
        for i in 0..=100 {
            let v = Easing::Ease.eval(i as f32 / 100.0);
            assert!(v >= prev - 1e-6, "ease is not monotonic at {i}");
            prev = v;
        }
    }

    /// The Newton solver must agree with a dense brute-force inversion.
    #[test]
    fn cubic_bezier_solver_agrees_with_brute_force() {
        let (x1, y1, x2, y2) = (0.68, -0.55, 0.265, 1.55); // "ease-in-out-back"
        let curve = Easing::CubicBezier(x1, y1, x2, y2);
        for i in 1..100 {
            let x = i as f32 / 100.0;
            // Brute force: scan s densely for the closest x(s).
            let mut best = (f32::MAX, 0.0f32);
            for j in 0..=200_000 {
                let s = j as f32 / 200_000.0;
                let d = (bezier(x1, x2, s) - x).abs();
                if d < best.0 {
                    best = (d, s);
                }
            }
            let reference = bezier(y1, y2, best.1);
            assert!(
                (curve.eval(x) - reference).abs() < 2e-3,
                "x={x}: {} vs {reference}",
                curve.eval(x)
            );
        }
    }

    #[test]
    fn cubic_bezier_can_overshoot() {
        let overshoot = Easing::CubicBezier(0.34, 1.56, 0.64, 1.0);
        assert!((1..100).any(|i| overshoot.eval(i as f32 / 100.0) > 1.0));
    }

    #[test]
    fn steps_follow_the_css_algorithm() {
        let start = Easing::Steps(4, StepPosition::JumpStart);
        assert_eq!(start.eval(0.0), 0.25);
        assert_eq!(start.eval(0.24), 0.25);
        assert_eq!(start.eval(0.26), 0.5);
        assert_eq!(start.eval(1.0), 1.0);

        let end = Easing::Steps(4, StepPosition::JumpEnd);
        assert_eq!(end.eval(0.0), 0.0);
        assert_eq!(end.eval(0.24), 0.0);
        assert_eq!(end.eval(0.26), 0.25);
        assert_eq!(end.eval(0.99), 0.75);
        assert_eq!(end.eval(1.0), 1.0);

        let none = Easing::Steps(3, StepPosition::JumpNone);
        assert_eq!(none.eval(0.0), 0.0);
        assert_eq!(none.eval(0.5), 0.5);
        assert_eq!(none.eval(1.0), 1.0);

        let both = Easing::Steps(3, StepPosition::JumpBoth);
        assert!((both.eval(0.0) - 0.25).abs() < 1e-6);
        assert!((both.eval(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(both.eval(1.0), 1.0);
    }

    #[test]
    fn step_keywords_and_invalid_counts() {
        let s = specs("transition: opacity 1s step-start");
        assert_eq!(s[0].easing, Easing::Steps(1, StepPosition::JumpStart));
        let s = specs("transition: opacity 1s step-end");
        assert_eq!(s[0].easing, Easing::Steps(1, StepPosition::JumpEnd));

        let (s, diags) = parse("transition: opacity 1s steps(1, jump-none)");
        assert_eq!(s.unwrap()[0].easing, Easing::Steps(2, StepPosition::JumpNone));
        assert!(diags.iter().any(|d| d.contains("jump-none")), "{diags:?}");
    }

    #[test]
    fn out_of_range_bezier_x_is_clamped_with_advisory() {
        let (s, diags) = parse("transition: opacity 1s cubic-bezier(1.5, 0, -0.2, 1)");
        assert_eq!(s.unwrap()[0].easing, Easing::CubicBezier(1.0, 0.0, 0.0, 1.0));
        assert!(diags.iter().any(|d| d.contains("cubic-bezier")), "{diags:?}");
    }
}
