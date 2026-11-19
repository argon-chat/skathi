//! `transition-*` parsing and the timing model. Names that cannot be interpolated become
//! `TransitionProperty::Other` (the animator advises once); shorthands expand to their longhands; a
//! negative `transition-delay` starts the transition part-way through.

use std::rc::Rc;
use std::time::Duration;

use lightningcss::properties::{Property, PropertyId};
use lightningcss::values::easing::{EasingFunction, StepPosition as CssStepPosition};
use lightningcss::values::time::Time;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TransitionProperty {
    Opacity,
    Color,
    BackgroundColor,
    BorderColor,
    TextDecorationColor,
    TextShadow,
    /// Used value is the element's `color`: gpui tints SVG sprites with one colour; `fill: none` is
    /// a discrete endpoint.
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
    /// Interpolates the normalized half corner, not `K`.
    CornerTopLeftShape,
    CornerTopRightShape,
    CornerBottomRightShape,
    CornerBottomLeftShape,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    RowGap,
    ColumnGap,
    FontSize,
    ObjectPosition,
    /// Interpolates the logarithm of the ratio, as browsers do; `auto` and `auto <ratio>` flip.
    AspectRatio,
    /// Discrete: `Element.animate()` and `@keyframes` flip it halfway; a transition never animates it
    /// (no `transition-behavior: allow-discrete`).
    OverflowX,
    OverflowY,
    /// The whole function list travels as one value: matched lists interpolate componentwise, never
    /// through the matrix, which would shear.
    Transform,
    /// Excluded from `all`: a custom property is not an animatable property without an `@property`
    /// registration.
    Shading,
    All,
    Other(Rc<str>),
}

pub const ANIMATABLE_COUNT: usize = 40;

impl TransitionProperty {
    /// By value: the enum carries an `Rc`, so it is neither `Sync` nor const-promotable.
    pub fn animatable() -> [TransitionProperty; ANIMATABLE_COUNT] {
        use TransitionProperty::*;
        [
            Opacity,
            Color,
            BackgroundColor,
            BorderColor,
            TextDecorationColor,
            TextShadow,
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
            CornerTopLeftShape,
            CornerTopRightShape,
            CornerBottomRightShape,
            CornerBottomLeftShape,
            BorderTopWidth,
            BorderRightWidth,
            BorderBottomWidth,
            BorderLeftWidth,
            RowGap,
            ColumnGap,
            FontSize,
            ObjectPosition,
            AspectRatio,
            Transform,
        ]
    }

    /// What keyframes animate: the [`animatable`](Self::animatable) set and the discrete properties.
    pub fn keyframed() -> impl Iterator<Item = TransitionProperty> {
        Self::animatable().into_iter().chain([
            TransitionProperty::OverflowX,
            TransitionProperty::OverflowY,
            TransitionProperty::Shading,
        ])
    }

    pub fn is_discrete(&self) -> bool {
        matches!(
            self,
            TransitionProperty::OverflowX | TransitionProperty::OverflowY
        )
    }

    pub fn name(&self) -> &str {
        use TransitionProperty::*;
        match self {
            Opacity => "opacity",
            Color => "color",
            BackgroundColor => "background-color",
            BorderColor => "border-color",
            TextDecorationColor => "text-decoration-color",
            TextShadow => "text-shadow",
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
            CornerTopLeftShape => "corner-top-left-shape",
            CornerTopRightShape => "corner-top-right-shape",
            CornerBottomRightShape => "corner-bottom-right-shape",
            CornerBottomLeftShape => "corner-bottom-left-shape",
            BorderTopWidth => "border-top-width",
            BorderRightWidth => "border-right-width",
            BorderBottomWidth => "border-bottom-width",
            BorderLeftWidth => "border-left-width",
            OverflowX => "overflow-x",
            OverflowY => "overflow-y",
            RowGap => "row-gap",
            ColumnGap => "column-gap",
            FontSize => "font-size",
            ObjectPosition => "object-position",
            AspectRatio => "aspect-ratio",
            Transform => "transform",
            Shading => "--shading",
            All => "all",
            Other(name) => name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepPosition {
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
    Steps(u32, StepPosition),
}

impl Default for Easing {
    fn default() -> Self {
        Easing::Ease
    }
}

impl Easing {
    pub fn eval(self, t: f32) -> f32 {
        self.eval_f64(t as f64) as f32
    }

    /// The same curve for script timing, without rounding progress to GPU precision.
    pub fn eval_f64(self, t: f64) -> f64 {
        match self {
            Easing::Linear => t,
            Easing::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
            Easing::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
            Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
            Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
            Easing::CubicBezier(x1, y1, x2, y2) => {
                cubic_bezier(x1 as f64, y1 as f64, x2 as f64, y2 as f64, t)
            }
            Easing::Steps(count, position) => steps(count, position, t),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransitionSpec {
    pub property: TransitionProperty,
    pub duration: Duration,
    pub delay: Delay,
    pub easing: Easing,
}

impl TransitionSpec {
    /// A combined duration (duration plus delay) of zero or less runs no transition.
    pub fn is_instant(&self) -> bool {
        self.delay.wait().is_zero() && self.delay.skip() >= self.duration
    }
}

pub fn spec_for<'s>(
    specs: &'s [TransitionSpec],
    property: &TransitionProperty,
) -> Option<&'s TransitionSpec> {
    specs
        .iter()
        .rev()
        .find(|s| s.property == *property || s.property == TransitionProperty::All)
}

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

fn bezier(a: f64, b: f64, t: f64) -> f64 {
    let mt = 1.0 - t;
    3.0 * mt * mt * t * a + 3.0 * mt * t * t * b + t * t * t
}

fn bezier_slope(a: f64, b: f64, t: f64) -> f64 {
    let mt = 1.0 - t;
    3.0 * mt * mt * a + 6.0 * mt * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

fn solve_t_for_x(x1: f64, x2: f64, x: f64) -> f64 {
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

    let (mut lo, mut hi) = (0.0f64, 1.0f64);
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

fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    bezier(y1, y2, solve_t_for_x(x1, x2, t))
}

fn steps(count: u32, position: StepPosition, t: f64) -> f64 {
    let n = count.max(1) as f64;
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
        return 0.0;
    }
    if t <= 1.0 && current > jumps {
        current = jumps;
    }
    current / jumps
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Collector {
    properties: Option<Vec<Vec<TransitionProperty>>>,
    durations: Option<Vec<Duration>>,
    delays: Option<Vec<Delay>>,
    easings: Option<Vec<Easing>>,
    // From the first substitution onward, declaration order matters for shorthand/longhand pairs.
    deferred: Vec<(String, String)>,
    seen: bool,
}

impl Collector {
    pub(crate) fn declarations(&self) -> Option<Rc<Self>> {
        self.seen.then(|| Rc::new(self.clone()))
    }

    pub(crate) fn merge(&mut self, later: &Self) {
        if later.properties.is_some() {
            self.properties = later.properties.clone();
        }
        if later.durations.is_some() {
            self.durations = later.durations.clone();
        }
        if later.delays.is_some() {
            self.delays = later.delays.clone();
        }
        if later.easings.is_some() {
            self.easings = later.easings.clone();
        }
        self.seen |= later.seen;
    }

    pub(crate) fn from_specs(specs: &[TransitionSpec]) -> Self {
        Self {
            properties: Some(
                specs
                    .iter()
                    .map(|spec| vec![spec.property.clone()])
                    .collect(),
            ),
            durations: Some(specs.iter().map(|spec| spec.duration).collect()),
            delays: Some(specs.iter().map(|spec| spec.delay).collect()),
            easings: Some(specs.iter().map(|spec| spec.easing).collect()),
            seen: true,
            deferred: Vec::new(),
        }
    }

    pub(crate) fn take(&mut self, p: &Property, diagnostics: &mut Vec<String>) -> bool {
        let id = p.property_id();
        if !is_transition_id(&id) {
            return false;
        }
        if !self.deferred.is_empty() || matches!(p, Property::Unparsed(_)) {
            if let Some(value) = crate::mapper::value_text(p) {
                self.deferred.push((id.name().to_owned(), value));
                self.seen = true;
            }
            return true;
        }
        self.take_literal(p, diagnostics)
    }

    pub(crate) fn merge_resolved(
        &mut self,
        other: &Self,
        vars: &crate::Vars,
        env: &crate::Env,
        diagnostics: &mut Vec<String>,
    ) {
        use lightningcss::stylesheet::ParserOptions;
        self.merge(other);
        for (name, raw) in &other.deferred {
            if let Ok(value) = crate::vars::substitute_with(raw, vars, env) {
                if let Ok(property) = Property::parse_string(
                    PropertyId::from(name.as_str()),
                    &value,
                    ParserOptions::default(),
                ) {
                    if self.take_literal(&property, diagnostics) {
                        continue;
                    }
                }
                crate::mapper::once(
                    diagnostics,
                    format!("invalid `{name}: {value}` after substitution"),
                );
            }
            // Invalid at computed-value time: the affected longhands take their initial values.
            match PropertyId::from(name.as_str()) {
                PropertyId::Transition(_) => {
                    *self = Self {
                        seen: true,
                        ..Default::default()
                    }
                }
                PropertyId::TransitionProperty(_) => self.properties = None,
                PropertyId::TransitionDuration(_) => self.durations = None,
                PropertyId::TransitionDelay(_) => self.delays = None,
                PropertyId::TransitionTimingFunction(_) => self.easings = None,
                _ => unreachable!(),
            }
        }
    }

    fn take_literal(&mut self, p: &Property, diagnostics: &mut Vec<String>) -> bool {
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
                        Expansion::Props(props) => properties.push(transitioned(props)),
                    }
                    durations.push(duration_of(
                        &entry.duration,
                        "transition-duration",
                        diagnostics,
                    ));
                    delays.push(delay_of(&entry.delay, "transition-delay", diagnostics));
                    easings.push(easing_of(&entry.timing_function, diagnostics));
                }
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
                        Expansion::Props(props) => properties.push(transitioned(props)),
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
                        .map(|t| delay_of(t, "transition-delay", diagnostics))
                        .collect(),
                );
                true
            }
            Property::TransitionTimingFunction(list, _) => {
                self.seen = true;
                self.easings = Some(list.iter().map(|e| easing_of(e, diagnostics)).collect());
                true
            }
            _ => false,
        }
    }

    pub(crate) fn finish(self) -> Option<Vec<TransitionSpec>> {
        if !self.seen {
            return None;
        }
        let entries = self
            .properties
            .unwrap_or_else(|| vec![vec![TransitionProperty::All]]);
        if entries.is_empty() {
            return Some(Vec::new());
        }
        let durations = self.durations.unwrap_or_default();
        let delays = self.delays.unwrap_or_default();
        let easings = self.easings.unwrap_or_default();

        fn pick<T: Copy + Default>(list: &[T], i: usize) -> T {
            if list.is_empty() {
                T::default()
            } else {
                list[i % list.len()]
            }
        }

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
    None,
    Props(Vec<TransitionProperty>),
}

fn transitioned(mut props: Vec<TransitionProperty>) -> Vec<TransitionProperty> {
    props.retain(|p| !p.is_discrete());
    props
}

fn expand(id: &PropertyId) -> Expansion {
    use TransitionProperty::*;
    let name = id.name();
    let one = |p: TransitionProperty| Expansion::Props(vec![p]);
    match name {
        "none" => Expansion::None,
        "all" => one(All),
        "opacity" => one(Opacity),
        "color" => one(Color),
        "background" | "background-color" => one(BackgroundColor),
        "border" => Expansion::Props(vec![
            BorderColor,
            BorderTopWidth,
            BorderRightWidth,
            BorderBottomWidth,
            BorderLeftWidth,
        ]),
        "border-color" => one(BorderColor),
        "border-width" => Expansion::Props(vec![
            BorderTopWidth,
            BorderRightWidth,
            BorderBottomWidth,
            BorderLeftWidth,
        ]),
        "border-top-width" => one(BorderTopWidth),
        "border-right-width" => one(BorderRightWidth),
        "border-bottom-width" => one(BorderBottomWidth),
        "border-left-width" => one(BorderLeftWidth),
        "overflow" => Expansion::Props(vec![OverflowX, OverflowY]),
        "overflow-x" => one(OverflowX),
        "overflow-y" => one(OverflowY),
        "text-decoration-color" => one(TextDecorationColor),
        "text-shadow" => one(TextShadow),
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
        "padding-inline" => Expansion::Props(vec![PaddingLeft, PaddingRight]),
        "padding-block" => Expansion::Props(vec![PaddingTop, PaddingBottom]),
        "padding-inline-start" => one(PaddingLeft),
        "padding-inline-end" => one(PaddingRight),
        "padding-block-start" => one(PaddingTop),
        "padding-block-end" => one(PaddingBottom),
        "margin-inline" => Expansion::Props(vec![MarginLeft, MarginRight]),
        "margin-block" => Expansion::Props(vec![MarginTop, MarginBottom]),
        "margin-inline-start" => one(MarginLeft),
        "margin-inline-end" => one(MarginRight),
        "margin-block-start" => one(MarginTop),
        "margin-block-end" => one(MarginBottom),
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
        "corner-shape" => Expansion::Props(vec![
            CornerTopLeftShape,
            CornerTopRightShape,
            CornerBottomRightShape,
            CornerBottomLeftShape,
        ]),
        "corner-top-left-shape" | "corner-start-start-shape" => one(CornerTopLeftShape),
        "corner-top-right-shape" | "corner-start-end-shape" => one(CornerTopRightShape),
        "corner-bottom-right-shape" | "corner-end-end-shape" => one(CornerBottomRightShape),
        "corner-bottom-left-shape" | "corner-end-start-shape" => one(CornerBottomLeftShape),
        "corner-top-shape" | "corner-block-start-shape" => {
            Expansion::Props(vec![CornerTopLeftShape, CornerTopRightShape])
        }
        "corner-bottom-shape" | "corner-block-end-shape" => {
            Expansion::Props(vec![CornerBottomLeftShape, CornerBottomRightShape])
        }
        "corner-left-shape" | "corner-inline-start-shape" => {
            Expansion::Props(vec![CornerTopLeftShape, CornerBottomLeftShape])
        }
        "corner-right-shape" | "corner-inline-end-shape" => {
            Expansion::Props(vec![CornerTopRightShape, CornerBottomRightShape])
        }
        "gap" => Expansion::Props(vec![RowGap, ColumnGap]),
        "row-gap" => one(RowGap),
        "column-gap" => one(ColumnGap),
        "font-size" => one(FontSize),
        "object-position" => one(ObjectPosition),
        "aspect-ratio" => one(AspectRatio),
        "transform" => one(Transform),
        // lightningcss parses an unknown property id — including a custom property — into
        // `PropertyId::Custom`, whose `name()` keeps the leading dashes.
        "--shading" => one(Shading),
        other => one(Other(Rc::from(other))),
    }
}

pub(crate) fn animates(name: &str) -> bool {
    match expand(&PropertyId::from(name)) {
        Expansion::None => false,
        Expansion::Props(list) => list
            .iter()
            .any(|p| !matches!(p, TransitionProperty::All | TransitionProperty::Other(_))),
    }
}

/// `None` for anything `animation-timing-function` would not take either, including `linear(...)`,
/// which the lightningcss in use does not parse.
pub fn parse_easing(text: &str) -> Option<Easing> {
    use lightningcss::stylesheet::ParserOptions;
    let id = PropertyId::from("animation-timing-function");
    match Property::parse_string(id, text.trim(), ParserOptions::default()).ok()? {
        Property::AnimationTimingFunction(list, _) if list.len() == 1 => {
            let mut diagnostics = Vec::new();
            let easing = easing_of(&list[0], &mut diagnostics);
            diagnostics.is_empty().then_some(easing)
        }
        _ => None,
    }
}

pub(crate) fn duration_of(t: &Time, what: &str, diagnostics: &mut Vec<String>) -> Duration {
    let ms = t.to_ms();
    if !ms.is_finite() {
        diagnostics.push(format!("non-finite {what} treated as 0s"));
        return Duration::ZERO;
    }
    if ms < 0.0 {
        diagnostics.push(format!(
            "negative {what} ({ms}ms) is not supported yet (v1) — clamped to 0s"
        ));
        return Duration::ZERO;
    }
    // `transition-delay: 9999999s` is an idiom for "never"; the cap says the same.
    Duration::from_secs_f64((ms as f64 / 1000.0).min(MAX_TIME_SECS))
}

pub(crate) fn delay_of(t: &Time, what: &str, diagnostics: &mut Vec<String>) -> Delay {
    let ms = t.to_ms();
    if !ms.is_finite() {
        diagnostics.push(format!("non-finite {what} treated as 0s"));
        return Delay::ZERO;
    }
    Delay::from_secs_f64(ms as f64 / 1000.0)
}

/// A signed `transition-delay` / `animation-delay`: a negative one starts the timeline that far in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Delay {
    nanos: i64,
}

impl Delay {
    pub const ZERO: Delay = Delay { nanos: 0 };

    pub fn from_secs_f64(secs: f64) -> Self {
        let secs = secs.clamp(-MAX_TIME_SECS, MAX_TIME_SECS);
        Delay {
            nanos: (secs * 1e9).round() as i64,
        }
    }

    pub fn is_zero(self) -> bool {
        self.nanos == 0
    }

    pub fn as_secs_f64(self) -> f64 {
        self.nanos as f64 / 1e9
    }

    /// The wait before the timeline starts; zero when negative.
    pub fn wait(self) -> Duration {
        Duration::from_nanos(self.nanos.max(0) as u64)
    }

    /// How far into the timeline it starts; zero when positive.
    pub fn skip(self) -> Duration {
        Duration::from_nanos(self.nanos.min(0).unsigned_abs())
    }

    /// The time into the timeline `elapsed` after the start, `None` while still waiting.
    pub fn active(self, elapsed: Duration) -> Option<Duration> {
        let wait = self.wait();
        (elapsed >= wait).then(|| elapsed - wait + self.skip())
    }

    /// `run` measured from the start, so less the skipped part.
    pub fn end_after(self, run: Duration) -> Option<Duration> {
        Some(self.wait().checked_add(run)?.saturating_sub(self.skip()))
    }
}

impl From<Duration> for Delay {
    fn from(d: Duration) -> Self {
        Delay::from_secs_f64(d.as_secs_f64())
    }
}

pub(crate) const MAX_TIME_SECS: f64 = 1.0e9;

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
                delay: ms(50).into(),
                easing: Easing::EaseIn,
            }]
        );
    }

    #[test]
    fn logical_margin_and_padding_names_transition_their_physical_sides() {
        let props = |body: &str| -> Vec<TransitionProperty> {
            specs(body).into_iter().map(|s| s.property).collect()
        };
        use TransitionProperty::*;
        assert_eq!(
            props("transition: margin-inline 1s"),
            vec![MarginLeft, MarginRight]
        );
        assert_eq!(
            props("transition: padding-block 1s"),
            vec![PaddingTop, PaddingBottom]
        );
        assert_eq!(props("transition: margin-inline-end 1s"), vec![MarginRight]);
        assert_eq!(
            props("transition: padding-block-start 1s"),
            vec![PaddingTop]
        );
    }

    #[test]
    fn shorthand_defaults_to_all_with_ease() {
        let s = specs("transition: 300ms");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].property, TransitionProperty::All);
        assert_eq!(s[0].duration, ms(300));
        assert_eq!(s[0].delay, Delay::ZERO);
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
        assert_eq!(s[1].delay, ms(100).into());
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
        assert_eq!(s[2].duration, ms(100));
        assert!(s.iter().all(|x| x.delay == ms(10).into()));
        assert_eq!(s[2].easing, Easing::Linear);
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
    fn a_huge_time_is_capped_and_stays_addable() {
        let s = specs("transition: opacity 1e20s 1e20s");
        assert_eq!(s[0].duration, Duration::from_secs(1_000_000_000));
        assert_eq!(s[0].delay, Duration::from_secs(1_000_000_000).into());
        let _ = std::time::Instant::now() + s[0].delay.wait() + s[0].duration;
    }

    #[test]
    fn a_negative_delay_is_kept() {
        let (s, _) = parse("transition: opacity 1s -200ms");
        assert_eq!(s.unwrap()[0].delay.skip(), ms(200));
    }

    #[test]
    fn spec_for_prefers_the_last_matching_entry() {
        let s = specs("transition: all 1s, opacity 2s");
        let opacity = spec_for(&s, &TransitionProperty::Opacity).unwrap();
        assert_eq!(opacity.duration, Duration::from_secs(2));
        let color = spec_for(&s, &TransitionProperty::Color).unwrap();
        assert_eq!(color.duration, Duration::from_secs(1));
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

    #[test]
    fn border_widths_transition_and_overflow_does_not() {
        let s = specs("transition: border-width 1s, overflow 1s");
        assert_eq!(
            covered(&s),
            vec![
                TransitionProperty::BorderTopWidth,
                TransitionProperty::BorderRightWidth,
                TransitionProperty::BorderBottomWidth,
                TransitionProperty::BorderLeftWidth,
            ]
        );
        assert!(!covered(&specs("transition: all 1s")).contains(&TransitionProperty::OverflowX));
        assert!(animates("overflow") && animates("border-top-width"));
    }

    const EPS: f32 = 1e-3;

    #[test]
    fn script_easing_keeps_double_progress_and_step_boundaries() {
        let progress = 0.123456789123;
        assert_eq!(Easing::Linear.eval_f64(progress), progress);
        let step = Easing::Steps(10, StepPosition::JumpEnd);
        assert_eq!(step.eval_f64(0.3 - 1e-12), 0.2);
        assert_eq!(step.eval_f64(0.3), 0.3);
        assert!(
            (Easing::Ease.eval_f64(0.5) - Easing::CubicBezier(0.25, 0.1, 0.25, 1.0).eval_f64(0.5))
                .abs()
                < 1e-8
        );
    }

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
        assert!((Easing::Ease.eval(0.5) - 0.802_4).abs() < EPS);
        assert!((Easing::EaseIn.eval(0.5) - 0.315_4).abs() < EPS);
        assert!((Easing::EaseOut.eval(0.5) - 0.684_6).abs() < EPS);
        assert!((Easing::EaseInOut.eval(0.5) - 0.5).abs() < EPS);
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

    #[test]
    fn cubic_bezier_solver_agrees_with_brute_force() {
        let (x1, y1, x2, y2) = (0.68, -0.55, 0.265, 1.55);
        let curve = Easing::CubicBezier(x1, y1, x2, y2);
        for i in 1..100 {
            let x = i as f64 / 100.0;
            let mut best = (f64::MAX, 0.0f64);
            for j in 0..=200_000 {
                let s = j as f64 / 200_000.0;
                let d = (bezier(x1 as f64, x2 as f64, s) - x).abs();
                if d < best.0 {
                    best = (d, s);
                }
            }
            let reference = bezier(y1 as f64, y2 as f64, best.1);
            assert!(
                (curve.eval_f64(x) - reference).abs() < 2e-3,
                "x={x}: {} vs {reference}",
                curve.eval_f64(x)
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

        let (s, _) = parse("transition: opacity 1s steps(1, jump-none)");
        assert_eq!(
            s.unwrap()[0].easing,
            Easing::Steps(2, StepPosition::JumpNone)
        );
    }

    #[test]
    fn out_of_range_bezier_x_is_clamped() {
        let (s, _) = parse("transition: opacity 1s cubic-bezier(1.5, 0, -0.2, 1)");
        assert_eq!(
            s.unwrap()[0].easing,
            Easing::CubicBezier(1.0, 0.0, 0.0, 1.0)
        );
    }
}
