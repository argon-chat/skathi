//! `@media` conditions reduced to the features a native window can answer, plus the runtime [`Env`]
//! they are evaluated against. Anything outside that set is rejected with an advisory and the block
//! dropped, never compiled into a rule that cannot match.

use lightningcss::media_query::{
    MediaCondition, MediaFeature, MediaFeatureComparison, MediaFeatureId, MediaFeatureName,
    MediaFeatureValue, MediaList, MediaQuery, MediaType, Operator, Qualifier, QueryFeature,
};
use lightningcss::traits::ToCss;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Env {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub dark: bool,
    pub reduce_motion: bool,
    pub scale_factor: f32,
    pub display_mode: DisplayMode,
    /// `None` while the overlay is not visible (OS chrome, fullscreen, minimised): the variables are
    /// then undefined and `env(titlebar-area-height, 40px)` takes its fallback, as in a browser.
    pub titlebar_area: Option<[f32; 4]>,
    /// The primary pointer of a desktop window is always the mouse (`pointer: fine`, `hover:
    /// hover`); only `any-pointer` can widen to other hardware.
    pub coarse_pointer: bool,
    /// Exists so `env(safe-area-inset-*)` from PWA sheets resolves: a fallback-less unknown `env()`
    /// invalidates its declaration.
    pub safe_area_insets: [f32; 4],
    /// The OS asks for opaque surfaces (Windows "Transparency effects" off, macOS "Reduce
    /// transparency").
    pub reduce_transparency: bool,
    /// `device-width` / `device-height`: the window's monitor, CSS px.
    pub device_size: (f32, f32),
    /// `dynamic-range: high`: the window's display is in HDR and the window can draw it.
    pub dynamic_range_high: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Standalone,
    WindowControlsOverlay,
    Fullscreen,
}

impl DisplayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayMode::Standalone => "standalone",
            DisplayMode::WindowControlsOverlay => "window-controls-overlay",
            DisplayMode::Fullscreen => "fullscreen",
        }
    }
}

impl Default for Env {
    fn default() -> Self {
        Env {
            viewport_width: 1024.0,
            viewport_height: 768.0,
            dark: false,
            reduce_motion: false,
            scale_factor: 1.0,
            display_mode: DisplayMode::WindowControlsOverlay,
            titlebar_area: Some([0.0, 0.0, 1024.0, 32.0]),
            coarse_pointer: false,
            safe_area_insets: [0.0; 4],
            reduce_transparency: false,
            device_size: (1920.0, 1080.0),
            dynamic_range_high: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Cmp {
    fn from(op: MediaFeatureComparison) -> Cmp {
        match op {
            MediaFeatureComparison::LessThan => Cmp::Lt,
            MediaFeatureComparison::LessThanEqual => Cmp::Le,
            MediaFeatureComparison::GreaterThan => Cmp::Gt,
            MediaFeatureComparison::GreaterThanEqual => Cmp::Ge,
            MediaFeatureComparison::Equal => Cmp::Eq,
        }
    }

    fn flip(self) -> Cmp {
        match self {
            Cmp::Lt => Cmp::Gt,
            Cmp::Le => Cmp::Ge,
            Cmp::Gt => Cmp::Lt,
            Cmp::Ge => Cmp::Le,
            Cmp::Eq => Cmp::Eq,
        }
    }

    fn eval(self, lhs: f32, rhs: f32) -> bool {
        match self {
            Cmp::Lt => lhs < rhs,
            Cmp::Le => lhs <= rhs,
            Cmp::Gt => lhs > rhs,
            Cmp::Ge => lhs >= rhs,
            // Exact float equality is what `(width: 800px)` means in CSS; the viewport is an
            // integral pixel count, so this is not as fragile as it looks.
            Cmp::Eq => lhs == rhs,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Feature {
    Width(Cmp, f32),
    Height(Cmp, f32),
    ColorScheme(bool),
    ReducedMotion(bool),
    Resolution(Cmp, f32),
    Display(DisplayMode),
    Pointer { any: bool, want: PointerKind },
    Hover { any: bool, want: bool },
    ReducedTransparency(bool),
    DeviceWidth(Cmp, f32),
    DeviceHeight(Cmp, f32),
    DynamicRange(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerKind {
    None,
    Coarse,
    Fine,
}

impl Feature {
    fn eval(&self, env: &Env) -> bool {
        match *self {
            Feature::Width(cmp, px) => cmp.eval(env.viewport_width, px),
            Feature::Height(cmp, px) => cmp.eval(env.viewport_height, px),
            Feature::ColorScheme(dark) => env.dark == dark,
            Feature::ReducedMotion(reduce) => env.reduce_motion == reduce,
            Feature::Resolution(cmp, dppx) => cmp.eval(env.scale_factor, dppx),
            Feature::Display(mode) => env.display_mode == mode,
            // The PRIMARY pointer of a desktop window is the mouse gpui delivers `PlatformInput`
            // from, whatever else is plugged in — the same answer a browser gives in a desktop
            // window on a touch laptop.
            Feature::Pointer { any, want } => match want {
                PointerKind::None => false,
                PointerKind::Fine => true,
                PointerKind::Coarse => any && env.coarse_pointer,
            },
            Feature::Hover { any, want } => {
                let _ = any;
                want
            }
            Feature::ReducedTransparency(reduce) => env.reduce_transparency == reduce,
            Feature::DeviceWidth(cmp, px) => cmp.eval(env.device_size.0, px),
            Feature::DeviceHeight(cmp, px) => cmp.eval(env.device_size.1, px),
            Feature::DynamicRange(high) => !high || env.dynamic_range_high,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cond {
    Always,
    Never,
    Feature(Feature),
    Not(Box<Cond>),
    And(Vec<Cond>),
    Or(Vec<Cond>),
}

impl Cond {
    pub(crate) fn eval(&self, env: &Env) -> bool {
        match self {
            Cond::Always => true,
            Cond::Never => false,
            Cond::Feature(f) => f.eval(env),
            Cond::Not(c) => !c.eval(env),
            Cond::And(cs) => cs.iter().all(|c| c.eval(env)),
            Cond::Or(cs) => cs.iter().any(|c| c.eval(env)),
        }
    }

    pub(crate) fn and(self, other: Cond) -> Cond {
        match (self, other) {
            (Cond::Never, _) | (_, Cond::Never) => Cond::Never,
            (Cond::Always, c) | (c, Cond::Always) => c,
            (Cond::And(mut a), Cond::And(b)) => {
                a.extend(b);
                Cond::And(a)
            }
            (Cond::And(mut a), c) => {
                a.push(c);
                Cond::And(a)
            }
            (c, Cond::And(mut b)) => {
                b.insert(0, c);
                Cond::And(b)
            }
            (a, b) => Cond::And(vec![a, b]),
        }
    }
}

pub(crate) type Unsupported = &'static str;

pub(crate) const SUPPORTED: &str = "supported: `screen`/`all`/`print` media types, \
     `width`/`min-width`/`max-width`, `height`/`min-height`/`max-height` (px lengths, \
     range and interval syntax), `device-width`/`device-height` (and their `min-`/`max-` forms), \
     `prefers-color-scheme`, `prefers-reduced-motion`, `prefers-reduced-transparency`, \
     `resolution`/`min-resolution`/`max-resolution` (dppx/dpi/dpcm), `display-mode`, \
     `pointer`/`any-pointer`, `hover`/`any-hover`, `dynamic-range`/`video-dynamic-range`, \
     combined with `and`/`or`/`not`/`,`";

pub(crate) fn parse_list(list: &MediaList<'_>) -> Result<Cond, Unsupported> {
    let mut queries = Vec::with_capacity(list.media_queries.len());
    for q in &list.media_queries {
        queries.push(parse_query(q)?);
    }
    Ok(match queries.len() {
        0 => Cond::Always,
        1 => queries.pop().unwrap(),
        _ => Cond::Or(queries),
    })
}

fn parse_query(q: &MediaQuery<'_>) -> Result<Cond, Unsupported> {
    // `print` blocks parse fine and simply never apply — the browser-honest answer, and it keeps
    // `print:` utilities from becoming an error every Tailwind app would hit.
    let type_cond = match &q.media_type {
        MediaType::All | MediaType::Screen => Cond::Always,
        MediaType::Print => Cond::Never,
        MediaType::Custom(_) => return Err("an unknown media type"),
    };
    let cond = match &q.condition {
        Some(c) => type_cond.and(parse_condition(c)?),
        None => type_cond,
    };
    Ok(match q.qualifier {
        Some(Qualifier::Not) => match cond {
            Cond::Always => Cond::Never,
            Cond::Never => Cond::Always,
            c => Cond::Not(Box::new(c)),
        },
        Some(Qualifier::Only) | None => cond,
    })
}

fn parse_condition(c: &MediaCondition<'_>) -> Result<Cond, Unsupported> {
    Ok(match c {
        MediaCondition::Feature(f) => parse_feature(f)?,
        MediaCondition::Not(inner) => Cond::Not(Box::new(parse_condition(inner)?)),
        MediaCondition::Operation {
            operator,
            conditions,
        } => {
            let mut parsed = Vec::with_capacity(conditions.len());
            for c in conditions {
                parsed.push(parse_condition(c)?);
            }
            match operator {
                Operator::And => Cond::And(parsed),
                Operator::Or => Cond::Or(parsed),
            }
        }
        MediaCondition::Unknown(_) => return Err("tokens the media parser could not interpret"),
    })
}

fn standard(name: &MediaFeatureName<'_, MediaFeatureId>) -> Result<MediaFeatureId, Unsupported> {
    match name {
        MediaFeatureName::Standard(id) => Ok(*id),
        MediaFeatureName::Custom(_) => Err("a custom media feature (`--var`)"),
        MediaFeatureName::Unknown(_) => Err("an unknown media feature"),
    }
}

fn length_px(value: &MediaFeatureValue<'_>) -> Result<f32, Unsupported> {
    match value {
        MediaFeatureValue::Length(l) => l
            .to_px()
            .ok_or("a media length that is not convertible to px (`em`/`%`/`calc()`/`env()`)"),
        _ => Err("a media feature value of the wrong type"),
    }
}

fn ident<'a>(value: &'a MediaFeatureValue<'_>) -> Result<&'a str, Unsupported> {
    match value {
        MediaFeatureValue::Ident(i) => Ok(i.0.as_ref()),
        _ => Err("a media feature value of the wrong type"),
    }
}

fn dppx(value: &MediaFeatureValue<'_>) -> Result<f32, Unsupported> {
    use lightningcss::values::resolution::Resolution;
    match value {
        MediaFeatureValue::Resolution(r) => Ok(match r {
            Resolution::Dppx(v) => *v,
            Resolution::Dpi(v) => *v / 96.0,
            Resolution::Dpcm(v) => *v * 2.54 / 96.0,
        }),
        _ => Err("a media feature value of the wrong type"),
    }
}

/// `browser` and `minimal-ui` parse but can never match — we are permanently an installed app
/// window — which is the browser-honest answer and keeps a ported PWA's `browser` branch compiling.
fn display_mode(value: &MediaFeatureValue<'_>) -> Result<Cond, Unsupported> {
    Ok(match ident(value)? {
        "standalone" => Cond::Feature(Feature::Display(DisplayMode::Standalone)),
        "window-controls-overlay" => {
            Cond::Feature(Feature::Display(DisplayMode::WindowControlsOverlay))
        }
        "fullscreen" => Cond::Feature(Feature::Display(DisplayMode::Fullscreen)),
        "browser" | "minimal-ui" | "picture-in-picture" => Cond::Never,
        _ => return Err("an unknown `display-mode` value"),
    })
}

fn sized(id: MediaFeatureId, cmp: Cmp, px: f32) -> Feature {
    match id {
        MediaFeatureId::Width => Feature::Width(cmp, px),
        MediaFeatureId::DeviceWidth => Feature::DeviceWidth(cmp, px),
        MediaFeatureId::DeviceHeight => Feature::DeviceHeight(cmp, px),
        _ => Feature::Height(cmp, px),
    }
}

fn pointer(value: &MediaFeatureValue<'_>, any: bool) -> Result<Cond, Unsupported> {
    Ok(Cond::Feature(Feature::Pointer {
        any,
        want: match ident(value)? {
            "none" => PointerKind::None,
            "coarse" => PointerKind::Coarse,
            "fine" => PointerKind::Fine,
            _ => return Err("an unknown `pointer` value"),
        },
    }))
}

fn hover(value: &MediaFeatureValue<'_>, any: bool) -> Result<Cond, Unsupported> {
    Ok(Cond::Feature(Feature::Hover {
        any,
        want: match ident(value)? {
            "hover" => true,
            "none" => false,
            _ => return Err("an unknown `hover` value"),
        },
    }))
}

fn dynamic_range(value: &MediaFeatureValue<'_>) -> Result<Cond, Unsupported> {
    Ok(Cond::Feature(Feature::DynamicRange(match ident(value)? {
        "high" => true,
        "standard" => false,
        _ => return Err("an unknown `dynamic-range` value"),
    })))
}

fn parse_feature(f: &MediaFeature<'_>) -> Result<Cond, Unsupported> {
    use MediaFeatureId::{
        AnyHover, AnyPointer, DeviceHeight, DeviceWidth, DisplayMode as DisplayModeId,
        DynamicRange, Height, Hover as HoverId, Pointer as PointerId, PrefersColorScheme,
        PrefersReducedMotion, PrefersReducedTransparency, Resolution, VideoDynamicRange, Width,
    };
    Ok(match f {
        QueryFeature::Plain { name, value } => match standard(name)? {
            id @ (Width | Height | DeviceWidth | DeviceHeight) => {
                Cond::Feature(sized(id, Cmp::Eq, length_px(value)?))
            }
            Resolution => Cond::Feature(Feature::Resolution(Cmp::Eq, dppx(value)?)),
            DisplayModeId => display_mode(value)?,
            PointerId => pointer(value, false)?,
            AnyPointer => pointer(value, true)?,
            HoverId => hover(value, false)?,
            AnyHover => hover(value, true)?,
            DynamicRange | VideoDynamicRange => dynamic_range(value)?,
            PrefersColorScheme => match ident(value)? {
                "dark" => Cond::Feature(Feature::ColorScheme(true)),
                "light" => Cond::Feature(Feature::ColorScheme(false)),
                _ => return Err("an unknown `prefers-color-scheme` value"),
            },
            PrefersReducedMotion => match ident(value)? {
                "reduce" => Cond::Feature(Feature::ReducedMotion(true)),
                "no-preference" => Cond::Feature(Feature::ReducedMotion(false)),
                _ => return Err("an unknown `prefers-reduced-motion` value"),
            },
            PrefersReducedTransparency => match ident(value)? {
                "reduce" => Cond::Feature(Feature::ReducedTransparency(true)),
                "no-preference" => Cond::Feature(Feature::ReducedTransparency(false)),
                _ => return Err("an unknown `prefers-reduced-transparency` value"),
            },
            _ => return Err("an unsupported media feature"),
        },
        QueryFeature::Boolean { name } => match standard(name)? {
            Width | Height | DeviceWidth | DeviceHeight => Cond::Always,
            PrefersReducedTransparency => Cond::Feature(Feature::ReducedTransparency(true)),
            PrefersColorScheme => Cond::Always,
            PrefersReducedMotion => Cond::Feature(Feature::ReducedMotion(true)),
            PointerId | AnyPointer | HoverId | AnyHover => Cond::Always,
            Resolution | DisplayModeId | DynamicRange | VideoDynamicRange => Cond::Always,
            _ => return Err("an unsupported media feature"),
        },
        QueryFeature::Range {
            name,
            operator,
            value,
        } => match standard(name)? {
            id @ (Width | Height | DeviceWidth | DeviceHeight) => {
                Cond::Feature(sized(id, Cmp::from(*operator), length_px(value)?))
            }
            Resolution => Cond::Feature(Feature::Resolution(Cmp::from(*operator), dppx(value)?)),
            _ => return Err("a range comparison on a feature that has no numeric value"),
        },
        QueryFeature::Interval {
            name,
            start,
            start_operator,
            end,
            end_operator,
        } => match standard(name)? {
            id @ (Width | Height | DeviceWidth | DeviceHeight) => Cond::And(vec![
                Cond::Feature(sized(
                    id,
                    Cmp::from(*start_operator).flip(),
                    length_px(start)?,
                )),
                Cond::Feature(sized(id, Cmp::from(*end_operator), length_px(end)?)),
            ]),
            Resolution => Cond::And(vec![
                Cond::Feature(Feature::Resolution(
                    Cmp::from(*start_operator).flip(),
                    dppx(start)?,
                )),
                Cond::Feature(Feature::Resolution(Cmp::from(*end_operator), dppx(end)?)),
            ]),
            _ => return Err("a range comparison on a feature that has no numeric value"),
        },
    })
}

/// `Err` is the reason a prelude was rejected; a browser treats an invalid media list as `not all`,
/// so callers report `matches: false` with the advisory.
pub fn match_media(query: &str, env: &Env) -> Result<bool, String> {
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};

    let css = format!("@media {query} {{ }}");
    let sheet = StyleSheet::parse(&css, ParserOptions::default())
        .map_err(|e| format!("could not be parsed ({e})"))?;
    let Some(CssRule::Media(m)) = sheet.rules.0.first() else {
        return Err("is not a media query".to_string());
    };
    parse_list(&m.query)
        .map(|cond| cond.eval(env))
        .map_err(|why| format!("uses {why} — {SUPPORTED}"))
}

pub(crate) fn list_text(list: &MediaList<'_>) -> String {
    list.to_css_string(lightningcss::printer::PrinterOptions::default())
        .unwrap_or_else(|_| "<media query>".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};

    fn cond(prelude: &str) -> Result<Cond, Unsupported> {
        let css = format!("@media {prelude} {{ .x {{ color: red }} }}");
        let sheet = StyleSheet::parse(&css, ParserOptions::default()).expect("parses");
        match &sheet.rules.0[0] {
            CssRule::Media(m) => parse_list(&m.query),
            other => panic!("expected @media, got {other:?}"),
        }
    }

    fn env(w: f32, dark: bool, reduce: bool) -> Env {
        Env {
            viewport_width: w,
            viewport_height: 800.0,
            dark,
            reduce_motion: reduce,
            ..Env::default()
        }
    }

    #[test]
    fn min_width_becomes_a_ge_range() {
        let c = cond("(min-width: 640px)").unwrap();
        assert_eq!(c, Cond::Feature(Feature::Width(Cmp::Ge, 640.0)));
        assert!(!c.eval(&env(639.0, false, false)));
        assert!(c.eval(&env(640.0, false, false)));
        assert!(c.eval(&env(1200.0, false, false)));
    }

    #[test]
    fn max_width_is_inclusive_like_a_browser() {
        let c = cond("(max-width: 640px)").unwrap();
        assert_eq!(c, Cond::Feature(Feature::Width(Cmp::Le, 640.0)));
        assert!(c.eval(&env(640.0, false, false)));
        assert!(!c.eval(&env(641.0, false, false)));
    }

    #[test]
    fn range_and_interval_syntax() {
        assert!(cond("(width > 400px)")
            .unwrap()
            .eval(&env(500.0, false, false)));
        assert!(!cond("(width > 400px)")
            .unwrap()
            .eval(&env(400.0, false, false)));
        let i = cond("(400px < width < 800px)").unwrap();
        assert!(!i.eval(&env(400.0, false, false)));
        assert!(i.eval(&env(401.0, false, false)));
        assert!(i.eval(&env(799.0, false, false)));
        assert!(!i.eval(&env(800.0, false, false)));
    }

    #[test]
    fn screen_and_all_match_print_never_does() {
        assert!(cond("screen").unwrap().eval(&Env::default()));
        assert!(cond("all").unwrap().eval(&Env::default()));
        assert_eq!(cond("print").unwrap(), Cond::Never);
        assert!(!cond("print").unwrap().eval(&Env::default()));
        assert_eq!(
            cond("screen and (min-width: 100px)").unwrap(),
            Cond::Feature(Feature::Width(Cmp::Ge, 100.0))
        );
    }

    #[test]
    fn color_scheme_and_reduced_motion() {
        let dark = cond("(prefers-color-scheme: dark)").unwrap();
        assert!(dark.eval(&env(800.0, true, false)));
        assert!(!dark.eval(&env(800.0, false, false)));
        let light = cond("(prefers-color-scheme: light)").unwrap();
        assert!(light.eval(&env(800.0, false, false)));
        let rm = cond("(prefers-reduced-motion: reduce)").unwrap();
        assert!(rm.eval(&env(800.0, false, true)));
        assert!(!rm.eval(&env(800.0, false, false)));
        assert!(cond("(prefers-reduced-motion: no-preference)")
            .unwrap()
            .eval(&env(800.0, false, false)));
    }

    #[test]
    fn reduced_transparency_and_device_size() {
        let reduced = Env {
            reduce_transparency: true,
            device_size: (2560.0, 1440.0),
            ..Env::default()
        };
        let plain = Env {
            device_size: (1280.0, 800.0),
            ..Env::default()
        };
        let matches = |query: &str, env: &Env| cond(query).unwrap().eval(env);
        assert!(matches("(prefers-reduced-transparency: reduce)", &reduced));
        assert!(matches("(prefers-reduced-transparency)", &reduced));
        assert!(!matches("(prefers-reduced-transparency)", &plain));
        assert!(matches(
            "(prefers-reduced-transparency: no-preference)",
            &plain
        ));
        assert!(matches("(min-device-width: 1920px)", &reduced));
        assert!(!matches("(min-device-width: 1920px)", &plain));
        assert!(!matches("(max-device-height: 1080px)", &reduced));
        assert!(matches("(device-width: 1280px)", &plain));
        assert!(matches("(1000px < device-height <= 1440px)", &reduced));
        assert!(
            !matches("(min-width: 1500px)", &reduced),
            "the viewport is not the device"
        );
    }

    #[test]
    fn and_or_not_and_comma_combine() {
        let c = cond("(min-width: 640px) and (prefers-color-scheme: dark)").unwrap();
        assert!(c.eval(&env(700.0, true, false)));
        assert!(!c.eval(&env(700.0, false, false)));
        assert!(!c.eval(&env(600.0, true, false)));

        let c = cond("(max-width: 400px), (min-width: 1200px)").unwrap();
        assert!(c.eval(&env(300.0, false, false)));
        assert!(!c.eval(&env(800.0, false, false)));
        assert!(c.eval(&env(1300.0, false, false)));

        let c = cond("not all and (min-width: 640px)").unwrap();
        assert!(c.eval(&env(600.0, false, false)));
        assert!(!c.eval(&env(700.0, false, false)));

        let c = cond("(width > 300px) or (prefers-reduced-motion: reduce)").unwrap();
        assert!(c.eval(&env(100.0, false, true)));
        assert!(!c.eval(&env(100.0, false, false)));
    }

    #[test]
    fn unsupported_features_are_rejected_not_silently_compiled() {
        for prelude in [
            "(hover: sometimes)",
            "(orientation: landscape)",
            "(prefers-contrast: more)",
            "tv",
            "(min-width: 40em)",
            "(aspect-ratio: 16/9)",
        ] {
            assert!(cond(prelude).is_err(), "{prelude} should be rejected");
        }
    }

    #[test]
    fn dynamic_range_high_follows_the_window_and_standard_always_matches() {
        let sdr = Env::default();
        let hdr = Env {
            dynamic_range_high: true,
            ..Env::default()
        };
        let t = |q: &str, env: &Env| cond(q).unwrap().eval(env);
        for feature in ["dynamic-range", "video-dynamic-range"] {
            assert!(!t(&format!("({feature}: high)"), &sdr));
            assert!(t(&format!("({feature}: high)"), &hdr));
            assert!(t(&format!("({feature}: standard)"), &sdr));
            assert!(t(&format!("({feature}: standard)"), &hdr));
            assert!(cond(&format!("({feature}: wide)")).is_err());
        }
    }

    #[test]
    fn the_pointer_and_hover_features_answer_for_a_desktop_window() {
        let mouse = Env::default();
        let touch = Env {
            coarse_pointer: true,
            ..Env::default()
        };
        let t = |q: &str, env: &Env| cond(q).unwrap().eval(env);

        for env in [&mouse, &touch] {
            assert!(
                t("(pointer: fine)", env),
                "the primary pointer is the mouse"
            );
            assert!(!t("(pointer: coarse)", env));
            assert!(!t("(pointer: none)", env));
            assert!(t("(hover: hover)", env));
            assert!(!t("(hover: none)", env));
            assert!(t("(any-hover: hover)", env), "the mouse can always hover");
            assert!(!t("(any-hover: none)", env));
            assert!(t("(any-pointer: fine)", env));
            assert!(t("(pointer)", env) && t("(hover)", env));
        }
        assert!(!t("(any-pointer: coarse)", &mouse));
        assert!(t("(any-pointer: coarse)", &touch));

        assert!(t("not (pointer: coarse)", &mouse));
        assert!(t("(hover: hover) and (pointer: fine)", &mouse));
    }

    /// `resolution` is the ONLY way a stylesheet (or `matchMedia`) can observe a DPI change —
    /// `devicePixelRatio` has no event in any spec.
    #[test]
    fn resolution_normalises_every_unit_to_dppx() {
        let hidpi = Env {
            scale_factor: 2.0,
            ..Env::default()
        };
        let lodpi = Env {
            scale_factor: 1.0,
            ..Env::default()
        };

        let c = cond("(min-resolution: 2dppx)").unwrap();
        assert_eq!(c, Cond::Feature(Feature::Resolution(Cmp::Ge, 2.0)));
        assert!(c.eval(&hidpi));
        assert!(!c.eval(&lodpi));

        assert_eq!(
            cond("(min-resolution: 192dpi)").unwrap(),
            Cond::Feature(Feature::Resolution(Cmp::Ge, 2.0))
        );
        let dpcm = cond("(min-resolution: 96dpcm)").unwrap();
        assert!(dpcm.eval(&Env {
            scale_factor: 3.0,
            ..Env::default()
        }));

        assert!(cond("(resolution: 1.5dppx)").unwrap().eval(&Env {
            scale_factor: 1.5,
            ..Env::default()
        }));
        assert!(cond("(resolution > 1dppx)").unwrap().eval(&hidpi));
        let i = cond("(1dppx < resolution < 3dppx)").unwrap();
        assert!(i.eval(&hidpi));
        assert!(!i.eval(&lodpi));
        assert_eq!(cond("(resolution)").unwrap(), Cond::Always);
    }

    #[test]
    fn display_mode_reports_the_chrome_mode() {
        let wco = Env {
            display_mode: DisplayMode::WindowControlsOverlay,
            ..Env::default()
        };
        let standalone = Env {
            display_mode: DisplayMode::Standalone,
            ..Env::default()
        };
        let fs = Env {
            display_mode: DisplayMode::Fullscreen,
            ..Env::default()
        };

        let c = cond("(display-mode: window-controls-overlay)").unwrap();
        assert!(c.eval(&wco));
        assert!(!c.eval(&standalone));
        assert!(!c.eval(&fs));

        assert!(cond("(display-mode: standalone)")
            .unwrap()
            .eval(&standalone));
        assert!(cond("(display-mode: fullscreen)").unwrap().eval(&fs));

        let browser = cond("(display-mode: browser)").unwrap();
        assert_eq!(browser, Cond::Never);
        assert!(!browser.eval(&wco));

        assert!(cond("(display-mode: nonsense)").is_err());
    }
}
