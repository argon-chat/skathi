//! `@media` conditions, reduced to the handful of features a native window can
//! actually answer, plus the runtime [`Env`] they are evaluated against.
//!
//! A browser re-evaluates media queries whenever the environment changes; we do
//! the same, only the "environment" is four numbers a gpui window can report
//! (`viewport_size()`, `appearance()`, `App::reduce_motion()`). The parsed
//! condition tree is stored ONCE per `@media` block at compile time and
//! evaluated per cascade — a resize or a theme flip re-runs the cascade for the
//! whole tree (rare events, so the whole-tree invalidation is free).
//!
//! Anything outside the supported feature set (`print`-only blocks aside, which
//! reduce to "never") is rejected with an advisory and the block is dropped —
//! never compiled into a rule that silently cannot match.

use lightningcss::media_query::{
    MediaCondition, MediaFeature, MediaFeatureComparison, MediaFeatureId, MediaFeatureName,
    MediaFeatureValue, MediaList, MediaQuery, MediaType, Operator, Qualifier, QueryFeature,
};
use lightningcss::traits::ToCss;

/// The runtime inputs `@media` conditions are evaluated against — the native
/// stand-in for a browser's "environment".
///
/// `viewport_width`/`viewport_height` are CSS pixels (gpui `Pixels` are already
/// logical, so no DPI scaling is involved) and describe the WINDOW, which is
/// what `@media (min-width: ...)` means in a browser too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Env {
    pub viewport_width: f32,
    pub viewport_height: f32,
    /// `prefers-color-scheme: dark`.
    pub dark: bool,
    /// `prefers-reduced-motion: reduce`.
    pub reduce_motion: bool,
    /// `resolution` in `dppx` — the window's DPI scale factor. This is the only
    /// observable a DPI change moves: `devicePixelRatio` has no event in any
    /// spec, so `matchMedia("(resolution: 1.5dppx)")` + a `change` listener is
    /// the canonical way to notice one (docs/WINDOWING.md §1.6).
    pub scale_factor: f32,
    /// `display-mode` — which chrome mode the window is in.
    pub display_mode: DisplayMode,
}

/// The `display-mode` media feature. Never `browser`: we are permanently an
/// installed app window (docs/WINDOWING.md §0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    /// OS-drawn chrome (`titleBarStyle: "default"`).
    Standalone,
    /// Custom chrome (`titleBarStyle: "hidden"`) — the titlebar strip belongs
    /// to content, which is exactly what WCO means (D-11).
    WindowControlsOverlay,
    /// The window is fullscreen, in either chrome mode.
    Fullscreen,
}

impl DisplayMode {
    /// The CSS keyword, which is also what `matchMedia` and the JS-side
    /// chrome-mode read report.
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayMode::Standalone => "standalone",
            DisplayMode::WindowControlsOverlay => "window-controls-overlay",
            DisplayMode::Fullscreen => "fullscreen",
        }
    }
}

impl Default for Env {
    /// A plain light 1024x768 desktop at 1x — the value tests and any caller
    /// that has no window yet see. The host overwrites it on the first frame.
    fn default() -> Self {
        Env {
            viewport_width: 1024.0,
            viewport_height: 768.0,
            dark: false,
            reduce_motion: false,
            scale_factor: 1.0,
            display_mode: DisplayMode::WindowControlsOverlay,
        }
    }
}

/// The comparison in `feature OP value` (already normalised into that
/// orientation by lightningcss: `(400px < width)` parses as `width > 400px`).
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

    /// `a < b` ⇔ `b > a`. The interval syntax (`(400px < width < 800px)`) states
    /// its first comparison value-first, so it needs flipping.
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
            // Exact float equality is what `(width: 800px)` means in CSS; the
            // viewport is an integral pixel count, so this is not as fragile as
            // it looks.
            Cmp::Eq => lhs == rhs,
        }
    }
}

/// One supported media feature test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Feature {
    Width(Cmp, f32),
    Height(Cmp, f32),
    /// `prefers-color-scheme: dark` (`true`) / `light` (`false`).
    ColorScheme(bool),
    /// `prefers-reduced-motion: reduce` (`true`) / `no-preference` (`false`).
    ReducedMotion(bool),
    /// `resolution` / `min-resolution` / `max-resolution`, normalised to dppx.
    Resolution(Cmp, f32),
    /// `display-mode: standalone | window-controls-overlay | fullscreen`.
    Display(DisplayMode),
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
        }
    }
}

/// A whole `@media` prelude as a boolean tree.
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

    /// `a and b`, flattened and constant-folded so the common
    /// `@media screen and (min-width: 640px)` ends up as a single feature.
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

/// Why a `@media` prelude could not be compiled. The caller pairs it with the
/// prelude's source text so the advisory names the exact block.
pub(crate) type Unsupported = &'static str;

/// The supported surface, quoted verbatim in every advisory so an author never
/// has to guess.
pub(crate) const SUPPORTED: &str = "supported: `screen`/`all`/`print` media types, \
     `width`/`min-width`/`max-width`, `height`/`min-height`/`max-height` (px lengths, \
     range and interval syntax), `prefers-color-scheme`, `prefers-reduced-motion`, \
     `resolution`/`min-resolution`/`max-resolution` (dppx/dpi/dpcm), `display-mode`, \
     combined with `and`/`or`/`not`/`,`";

/// Parses a whole media list (the comma-separated prelude) into one condition.
/// Commas are `or`.
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
    // We are a screen. `print` blocks parse fine and simply never apply — the
    // browser-honest answer, and it keeps `print:` utilities from becoming an
    // error every Tailwind app would hit.
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
        // `only` is the legacy MSIE guard and means nothing to a modern parser.
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
        MediaCondition::Operation { operator, conditions } => {
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

/// The feature name, once lightningcss has stripped any `min-`/`max-` prefix
/// (it turns those into a range comparator, so `min-width: 640px` arrives here
/// as `width >= 640px`).
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

/// `resolution` in any of its three units, normalised to `dppx` — which is what
/// `devicePixelRatio` and gpui's `scale_factor()` both are. 96 dpi = 1 dppx is
/// the CSS definition of a reference pixel, not an approximation.
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

/// `display-mode: <ident>`. `browser` and `minimal-ui` parse but can never
/// match — we are permanently an installed app window — which is the
/// browser-honest answer and keeps a ported PWA's `browser` branch compiling.
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
        _ => Feature::Height(cmp, px),
    }
}

fn parse_feature(f: &MediaFeature<'_>) -> Result<Cond, Unsupported> {
    use MediaFeatureId::{
        DisplayMode as DisplayModeId, Height, PrefersColorScheme, PrefersReducedMotion, Resolution,
        Width,
    };
    Ok(match f {
        QueryFeature::Plain { name, value } => match standard(name)? {
            id @ (Width | Height) => Cond::Feature(sized(id, Cmp::Eq, length_px(value)?)),
            Resolution => Cond::Feature(Feature::Resolution(Cmp::Eq, dppx(value)?)),
            DisplayModeId => display_mode(value)?,
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
            _ => return Err("an unsupported media feature"),
        },
        // Boolean form: "the feature is present and not its zero/none value".
        QueryFeature::Boolean { name } => match standard(name)? {
            // A window always has a non-zero size.
            Width | Height => Cond::Always,
            // `(prefers-color-scheme)` is true whenever a preference exists,
            // which on every platform we run on it does.
            PrefersColorScheme => Cond::Always,
            PrefersReducedMotion => Cond::Feature(Feature::ReducedMotion(true)),
            // A screen always has a non-zero resolution, and we are always in
            // SOME display mode.
            Resolution | DisplayModeId => Cond::Always,
            _ => return Err("an unsupported media feature"),
        },
        QueryFeature::Range { name, operator, value } => match standard(name)? {
            id @ (Width | Height) => {
                Cond::Feature(sized(id, Cmp::from(*operator), length_px(value)?))
            }
            Resolution => Cond::Feature(Feature::Resolution(Cmp::from(*operator), dppx(value)?)),
            _ => return Err("a range comparison on a feature that has no numeric value"),
        },
        QueryFeature::Interval { name, start, start_operator, end, end_operator } => {
            match standard(name)? {
                id @ (Width | Height) => Cond::And(vec![
                    // `start OP feature` — stated value-first, so flip it.
                    Cond::Feature(sized(id, Cmp::from(*start_operator).flip(), length_px(start)?)),
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
            }
        }
    })
}

/// `window.matchMedia(query)` — parse one prelude and evaluate it against
/// `env`, through the SAME compiler and the SAME evaluator the cascade uses.
///
/// Sharing the code is the whole point: a `matchMedia` that answered from a
/// second implementation could drift from `@media`, and one that answered
/// `false` forever (which is what redjs ships) is the exact failure the
/// compat-first policy exists to prevent (D-09).
///
/// `Err` carries the human reason a prelude was rejected, so the JS side can
/// turn it into the same advisory the stylesheet compiler would have printed.
/// A browser treats a syntactically invalid media list as `not all` — always
/// false — so the caller reports `matches: false` alongside the advisory.
pub fn match_media(query: &str, env: &Env) -> Result<bool, String> {
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};

    // lightningcss exposes no standalone media-list parser, so the query is
    // compiled as the prelude of a throwaway `@media` block — the same path a
    // stylesheet takes, which is what guarantees identical acceptance.
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

/// The prelude's source text, for advisories.
pub(crate) fn list_text(list: &MediaList<'_>) -> String {
    list.to_css_string(lightningcss::printer::PrinterOptions::default())
        .unwrap_or_else(|_| "<media query>".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};

    /// Parses `@media <prelude> { .x { color: red } }` and returns the compiled
    /// condition (or the rejection reason).
    fn cond(prelude: &str) -> Result<Cond, Unsupported> {
        let css = format!("@media {prelude} {{ .x {{ color: red }} }}");
        let sheet = StyleSheet::parse(&css, ParserOptions::default()).expect("parses");
        match &sheet.rules.0[0] {
            CssRule::Media(m) => parse_list(&m.query),
            other => panic!("expected @media, got {other:?}"),
        }
    }

    fn env(w: f32, dark: bool, reduce: bool) -> Env {
        Env { viewport_width: w, viewport_height: 800.0, dark, reduce_motion: reduce, ..Env::default() }
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
        assert!(cond("(width > 400px)").unwrap().eval(&env(500.0, false, false)));
        assert!(!cond("(width > 400px)").unwrap().eval(&env(400.0, false, false)));
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
        // The type folds away when it is trivially true.
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
    fn and_or_not_and_comma_combine() {
        let c = cond("(min-width: 640px) and (prefers-color-scheme: dark)").unwrap();
        assert!(c.eval(&env(700.0, true, false)));
        assert!(!c.eval(&env(700.0, false, false)));
        assert!(!c.eval(&env(600.0, true, false)));

        // A comma-separated list is `or`.
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
            "(hover: hover)",
            "(orientation: landscape)",
            "(prefers-contrast: more)",
            "tv",
            "(min-width: 40em)",
            "(aspect-ratio: 16/9)",
        ] {
            assert!(cond(prelude).is_err(), "{prelude} should be rejected");
        }
    }

    /// `resolution` is the ONLY way a stylesheet (or `matchMedia`) can observe
    /// a DPI change — `devicePixelRatio` has no event in any spec.
    #[test]
    fn resolution_normalises_every_unit_to_dppx() {
        let hidpi = Env { scale_factor: 2.0, ..Env::default() };
        let lodpi = Env { scale_factor: 1.0, ..Env::default() };

        let c = cond("(min-resolution: 2dppx)").unwrap();
        assert_eq!(c, Cond::Feature(Feature::Resolution(Cmp::Ge, 2.0)));
        assert!(c.eval(&hidpi));
        assert!(!c.eval(&lodpi));

        // 96dpi === 1dppx by the CSS definition of a reference pixel.
        assert_eq!(
            cond("(min-resolution: 192dpi)").unwrap(),
            Cond::Feature(Feature::Resolution(Cmp::Ge, 2.0))
        );
        // 1dpcm = 2.54/96 dppx.
        let dpcm = cond("(min-resolution: 96dpcm)").unwrap();
        assert!(dpcm.eval(&Env { scale_factor: 3.0, ..Env::default() }));

        // Exact form and the range/interval syntaxes.
        assert!(cond("(resolution: 1.5dppx)")
            .unwrap()
            .eval(&Env { scale_factor: 1.5, ..Env::default() }));
        assert!(cond("(resolution > 1dppx)").unwrap().eval(&hidpi));
        let i = cond("(1dppx < resolution < 3dppx)").unwrap();
        assert!(i.eval(&hidpi));
        assert!(!i.eval(&lodpi));
        // The boolean form: a screen always has SOME resolution.
        assert_eq!(cond("(resolution)").unwrap(), Cond::Always);
    }

    /// `display-mode` is the CSS-side feature detect for custom chrome, and the
    /// branch a component ported from a real PWA takes (D-11).
    #[test]
    fn display_mode_reports_the_chrome_mode() {
        let wco = Env { display_mode: DisplayMode::WindowControlsOverlay, ..Env::default() };
        let standalone = Env { display_mode: DisplayMode::Standalone, ..Env::default() };
        let fs = Env { display_mode: DisplayMode::Fullscreen, ..Env::default() };

        let c = cond("(display-mode: window-controls-overlay)").unwrap();
        assert!(c.eval(&wco));
        assert!(!c.eval(&standalone));
        assert!(!c.eval(&fs));

        assert!(cond("(display-mode: standalone)").unwrap().eval(&standalone));
        assert!(cond("(display-mode: fullscreen)").unwrap().eval(&fs));

        // We are permanently an installed app window, so `browser` parses (a
        // ported PWA's fallback branch stays compilable) and never matches.
        let browser = cond("(display-mode: browser)").unwrap();
        assert_eq!(browser, Cond::Never);
        assert!(!browser.eval(&wco));

        assert!(cond("(display-mode: nonsense)").is_err());
    }
}
