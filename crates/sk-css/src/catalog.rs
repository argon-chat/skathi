//! What CSS Skathi supports, property by property: the one list `CSS.supports`, `@supports` and the
//! docs answer from. Each entry's sample values run through the mapper in the tests below, so an entry
//! that disagrees with what the mapper does fails the build.

use crate::mapper::{accepts_declaration, verdict, Verdict};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Applied, every sample value.
    Supported,
    /// Applied, with the limitation in the note; `refused` lists values that are not.
    Partial,
    /// Accepted without an advisory and without effect, which is right for this engine.
    Ignored,
    /// Not applied; a declaration prints an advisory.
    Unsupported,
    /// A vendor-prefixed spelling dropped without an advisory, as Chromium drops one it does not know.
    Dropped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Supported => "supported",
            Status::Partial => "partial",
            Status::Ignored => "ignored",
            Status::Unsupported => "unsupported",
            Status::Dropped => "dropped",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

#[derive(Debug)]
pub struct Entry {
    pub name: &'static str,
    pub status: Status,
    /// Values the engine applies.
    pub applied: &'static [&'static str],
    /// Values it parses and does not honour; `CSS.supports` answers false for each.
    pub refused: &'static [&'static str],
    pub note: &'static str,
    /// Where it works, when not on every platform.
    pub platforms: &'static [Platform],
}

const fn e(
    name: &'static str,
    status: Status,
    applied: &'static [&'static str],
    refused: &'static [&'static str],
    note: &'static str,
) -> Entry {
    Entry {
        name,
        status,
        applied,
        refused,
        note,
        platforms: &[],
    }
}

use Status::*;

const SIZING_KEYWORDS: &[&str] = &["min-content", "max-content", "fit-content", "stretch"];
const SIZING_NOTE: &str = "Intrinsic sizing keywords are ignored; use flex or grid sizing";

#[rustfmt::skip]
pub const CATALOG: &[Entry] = &[
    e("accent-color", Supported, &["auto", "red"], &[], ""),
    e("align-content", Partial, &["center", "space-between"], &["normal", "baseline"], "`normal` and baseline alignment are not mapped"),
    e("align-items", Partial, &["center", "baseline"], &["normal"], "`normal` is not mapped; `stretch` is the default"),
    e("align-self", Partial, &["center", "stretch"], &["auto", "normal"], "`auto` and `normal` cannot undo an earlier value"),
    e("anchor-name", Unsupported, &[], &["--a"], ""),
    e("animation", Supported, &["spin 1s linear infinite", "none"], &[], ""),
    e("animation-composition", Unsupported, &[], &["add"], ""),
    e("animation-delay", Supported, &["1s", "-1s"], &[], ""),
    e("animation-direction", Supported, &["normal", "alternate"], &[], ""),
    e("animation-duration", Supported, &["1s"], &[], ""),
    e("animation-fill-mode", Supported, &["none", "both"], &[], ""),
    e("animation-iteration-count", Supported, &["1", "infinite"], &[], ""),
    e("animation-name", Supported, &["none", "spin"], &[], ""),
    e("animation-play-state", Supported, &["running", "paused"], &[], ""),
    e("animation-range", Unsupported, &[], &["normal"], ""),
    e("animation-range-end", Unsupported, &[], &["normal"], ""),
    e("animation-range-start", Unsupported, &[], &["normal"], ""),
    e("animation-timeline", Unsupported, &[], &["none"], ""),
    e("animation-timing-function", Partial, &["ease", "cubic-bezier(0.34, 1.56, 0.64, 1)"], &[], "`linear()` easing is not parsed"),
    e("app-region", Supported, &["drag", "no-drag"], &[], "A window drag region, as in Electron"),
    e("appearance", Supported, &["none", "auto"], &[], ""),
    e("aspect-ratio", Supported, &["1 / 1", "1.5", "auto", "auto 16 / 9"], &[], ""),
    e("backdrop-filter", Partial, &["none", "blur(2px)"], &["grayscale(1)", "saturate(180%)", "brightness(2)", "contrast(2)", "sepia(1)", "hue-rotate(10deg)", "invert(1)", "opacity(0.5)", "drop-shadow(0 0 1px red)"], "Only `blur()`"),
    e("backface-visibility", Unsupported, &[], &["hidden"], ""),
    e("background", Supported, &["red", "linear-gradient(red, blue)"], &[], ""),
    e("background-attachment", Ignored, &["scroll", "fixed"], &[], "A background scrolls with its element"),
    e("background-blend-mode", Unsupported, &[], &["multiply"], ""),
    e("background-clip", Partial, &["text"], &["border-box", "content-box"], "Only `text`; the background fills the padding box"),
    e("background-color", Supported, &["red", "transparent"], &[], ""),
    e("background-image", Supported, &["none", "url(a.png)"], &[], ""),
    e("background-origin", Ignored, &["border-box", "content-box"], &[], ""),
    e("background-position", Supported, &["center", "10px 20%"], &[], ""),
    e("background-position-x", Supported, &["center", "10px"], &[], ""),
    e("background-position-y", Supported, &["center", "10px"], &[], ""),
    e("background-repeat", Supported, &["repeat", "no-repeat"], &[], ""),
    e("background-size", Supported, &["cover", "10px 20px"], &[], ""),
    e("block-size", Partial, &["auto", "10px", "calc-size(auto, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("border", Supported, &["1px solid red", "none"], &[], ""),
    e("border-block", Unsupported, &[], &["1px solid red"], ""),
    e("border-block-color", Unsupported, &[], &["red"], ""),
    e("border-block-end", Unsupported, &[], &["1px solid red"], ""),
    e("border-block-end-color", Unsupported, &[], &["red"], ""),
    e("border-block-end-style", Unsupported, &[], &["solid"], ""),
    e("border-block-end-width", Unsupported, &[], &["1px"], ""),
    e("border-block-start", Unsupported, &[], &["1px solid red"], ""),
    e("border-block-start-color", Unsupported, &[], &["red"], ""),
    e("border-block-start-style", Unsupported, &[], &["solid"], ""),
    e("border-block-start-width", Unsupported, &[], &["1px"], ""),
    e("border-block-style", Unsupported, &[], &["solid"], ""),
    e("border-block-width", Unsupported, &[], &["1px"], ""),
    e("border-bottom", Supported, &["1px solid red", "none"], &[], ""),
    e("border-bottom-color", Supported, &["red", "transparent"], &[], ""),
    e("border-bottom-left-radius", Supported, &["4px", "50%"], &[], ""),
    e("border-bottom-right-radius", Supported, &["4px", "50%"], &[], ""),
    e("border-bottom-style", Supported, &["none", "solid"], &[], ""),
    e("border-bottom-width", Supported, &["0", "2px"], &[], ""),
    e("border-collapse", Unsupported, &[], &["collapse"], "Tables lay out as blocks"),
    e("border-color", Supported, &["red", "transparent"], &[], ""),
    e("border-end-end-radius", Unsupported, &[], &["4px"], ""),
    e("border-end-start-radius", Unsupported, &[], &["4px"], ""),
    e("border-image", Unsupported, &[], &["url(a.png) 10"], ""),
    e("border-image-outset", Unsupported, &[], &["1px"], ""),
    e("border-image-repeat", Unsupported, &[], &["stretch"], ""),
    e("border-image-slice", Unsupported, &[], &["10"], ""),
    e("border-image-source", Unsupported, &[], &["url(a.png)"], ""),
    e("border-image-width", Unsupported, &[], &["1px"], ""),
    e("border-inline", Unsupported, &[], &["1px solid red"], ""),
    e("border-inline-color", Unsupported, &[], &["red"], ""),
    e("border-inline-end", Unsupported, &[], &["1px solid red"], ""),
    e("border-inline-end-color", Unsupported, &[], &["red"], ""),
    e("border-inline-end-style", Unsupported, &[], &["solid"], ""),
    e("border-inline-end-width", Unsupported, &[], &["1px"], ""),
    e("border-inline-start", Unsupported, &[], &["1px solid red"], ""),
    e("border-inline-start-color", Unsupported, &[], &["red"], ""),
    e("border-inline-start-style", Unsupported, &[], &["solid"], ""),
    e("border-inline-start-width", Unsupported, &[], &["1px"], ""),
    e("border-inline-style", Unsupported, &[], &["solid"], ""),
    e("border-inline-width", Unsupported, &[], &["1px"], ""),
    e("border-left", Supported, &["1px solid red", "none"], &[], ""),
    e("border-left-color", Supported, &["red", "transparent"], &[], ""),
    e("border-left-style", Supported, &["none", "solid"], &[], ""),
    e("border-left-width", Supported, &["0", "2px"], &[], ""),
    e("border-radius", Supported, &["4px", "50%"], &[], ""),
    e("border-right", Supported, &["1px solid red", "none"], &[], ""),
    e("border-right-color", Supported, &["red", "transparent"], &[], ""),
    e("border-right-style", Supported, &["none", "solid"], &[], ""),
    e("border-right-width", Supported, &["0", "2px"], &[], ""),
    e("border-spacing", Unsupported, &[], &["2px"], "Tables lay out as blocks"),
    e("border-start-end-radius", Unsupported, &[], &["4px"], ""),
    e("border-start-start-radius", Unsupported, &[], &["4px"], ""),
    e("border-style", Supported, &["none", "solid"], &[], ""),
    e("border-top", Supported, &["1px solid red", "none"], &[], ""),
    e("border-top-color", Supported, &["red", "transparent"], &[], ""),
    e("border-top-left-radius", Supported, &["4px", "50%"], &[], ""),
    e("border-top-right-radius", Supported, &["4px", "50%"], &[], ""),
    e("border-top-style", Supported, &["none", "solid"], &[], ""),
    e("border-top-width", Supported, &["0", "2px"], &[], ""),
    e("border-width", Supported, &["0", "2px"], &[], ""),
    e("bottom", Supported, &["auto", "10px"], &[], ""),
    e("box-align", Unsupported, &[], &["center"], ""),
    e("box-decoration-break", Unsupported, &[], &["clone"], ""),
    e("box-direction", Unsupported, &[], &["normal"], ""),
    e("box-flex", Unsupported, &[], &["1"], ""),
    e("box-flex-group", Unsupported, &[], &["1"], ""),
    e("box-lines", Unsupported, &[], &["single"], ""),
    e("box-ordinal-group", Unsupported, &[], &["1"], ""),
    e("box-orient", Unsupported, &[], &["vertical"], ""),
    e("box-pack", Unsupported, &[], &["center"], ""),
    e("box-shadow", Partial, &["none", "0 2px 4px red"], &["1px 2px"], "A shadow coloured `currentColor` or a system colour is dropped"),
    e("box-sizing", Partial, &["border-box"], &["content-box"], "Every box is `border-box`"),
    e("caption-side", Unsupported, &[], &["bottom"], "Tables lay out as blocks"),
    e("caret", Unsupported, &[], &["auto"], ""),
    e("caret-color", Unsupported, &[], &["red"], ""),
    e("caret-shape", Unsupported, &[], &["block"], ""),
    e("clear", Unsupported, &[], &["both"], "Use flex or grid"),
    e("clip", Partial, &["rect(0, 0, 0, 0)"], &["rect(0, 10px, 10px, 0)"], "Only the empty rect of the visually-hidden idiom"),
    e("clip-path", Partial, &["circle(50%)", "path(\"M0 0L10 10\")"], &["url(#c)"], "Basic shapes and `path()`; `url()` is not applied"),
    e("clip-rule", Unsupported, &[], &["nonzero"], ""),
    e("color", Supported, &["red", "oklch(70% 0.1 200)", "light-dark(black, white)"], &[], ""),
    e("color-interpolation", Unsupported, &[], &["auto"], ""),
    e("color-interpolation-filters", Unsupported, &[], &["auto"], ""),
    e("color-rendering", Unsupported, &[], &["auto"], ""),
    e("color-scheme", Supported, &["normal", "light dark", "only dark"], &[], ""),
    e("column-count", Unsupported, &[], &["2"], ""),
    e("column-gap", Supported, &["0", "10px"], &[], ""),
    e("column-rule", Unsupported, &[], &["1px solid red"], ""),
    e("column-width", Unsupported, &[], &["100px"], ""),
    e("column-span", Unsupported, &[], &["all"], ""),
    e("columns", Unsupported, &[], &["2"], ""),
    e("contain", Supported, &["none", "paint", "size", "inline-size", "strict"], &[], ""),
    e("contain-intrinsic-size", Unsupported, &[], &["100px"], ""),
    e("container", Supported, &["card / inline-size", "none"], &[], ""),
    e("container-name", Supported, &["card", "card sidebar", "none"], &[], ""),
    e("container-type", Partial, &["normal", "inline-size", "size"], &["scroll-state"], "Scroll-state container queries are not supported"),
    e("content", Partial, &["none", "\"a\""], &["counter(n)", "attr(data-x)"], "A literal string; counters and `attr()` generate nothing"),
    e("content-visibility", Partial, &["visible", "hidden"], &["auto"], "Skips ordinary box content when hidden"),
    e("corner", Unsupported, &[], &["8px bevel"], "Use `border-radius` and `corner-shape`"),
    e("corner-block-end-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-block-start-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-bottom-left", Unsupported, &[], &["8px bevel"], "Use `border-bottom-left-radius` and `corner-bottom-left-shape`"),
    e("corner-bottom-left-shape", Supported, &["bevel", "superellipse(-2)"], &[], ""),
    e("corner-bottom-right", Unsupported, &[], &["8px bevel"], "Use `border-bottom-right-radius` and `corner-bottom-right-shape`"),
    e("corner-bottom-right-shape", Supported, &["bevel", "superellipse(-2)"], &[], ""),
    e("corner-bottom-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-end-end-shape", Supported, &["bevel", "squircle"], &[], ""),
    e("corner-end-start-shape", Supported, &["bevel", "squircle"], &[], ""),
    e("corner-inline-end-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-inline-start-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-left-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-right-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("corner-shape", Supported, &["round", "squircle", "bevel scoop", "notch square round superellipse(0.5)", "superellipse(infinity)"], &[], ""),
    e("corner-start-end-shape", Supported, &["bevel", "squircle"], &[], ""),
    e("corner-start-start-shape", Supported, &["bevel", "squircle"], &[], ""),
    e("corner-top-left", Unsupported, &[], &["8px bevel"], "Use `border-top-left-radius` and `corner-top-left-shape`"),
    e("corner-top-left-shape", Supported, &["bevel", "superellipse(-2)"], &[], ""),
    e("corner-top-right", Unsupported, &[], &["8px bevel"], "Use `border-top-right-radius` and `corner-top-right-shape`"),
    e("corner-top-right-shape", Supported, &["bevel", "superellipse(-2)"], &[], ""),
    e("corner-top-shape", Supported, &["bevel", "round scoop"], &[], ""),
    e("counter-increment", Unsupported, &[], &["n"], ""),
    e("counter-reset", Unsupported, &[], &["n"], ""),
    e("counter-set", Unsupported, &[], &["n 1"], ""),
    e("cursor", Partial, &["pointer", "text", "wait", "none", "url(hand.png) 4 4, pointer"], &[], "A `url()` image is not loaded: the fallback keyword shows. A keyword the OS has no cursor for shows the arrow"),
    e("cx", Supported, &["10px", "50%"], &[], ""),
    e("cy", Supported, &["10px", "50%"], &[], ""),
    e("d", Unsupported, &[], &["path(\"M0 0L10 10\")"], ""),
    e("direction", Unsupported, &[], &["rtl"], "Left-to-right text only"),
    e("display", Partial, &["flex", "grid"], &[], "No line boxes; `contents` and the table values lay out as `block`"),
    e("dominant-baseline", Unsupported, &[], &["middle"], ""),
    e("dynamic-range-limit", Partial, &["standard", "no-limit", "dynamic-range-limit-mix(standard 30%, no-limit 70%)"], &[], "Computed and inherited, and reported to `<sk-surface>` producers; windows draw SDR, so nothing on screen is limited yet"),
    e("effect-quality", Supported, &["auto", "full", "reduced", "minimal"], &[], "Local quality for a shader declaring [SkQuality]"),
    e("empty-cells", Unsupported, &[], &["hide"], "Tables lay out as blocks"),
    e("field-sizing", Partial, &["fixed", "content"], &[], "Content sizing applies to text inputs, textareas, and closed selects"),
    e("fill", Supported, &["none", "red"], &[], ""),
    e("fill-opacity", Supported, &["0.5"], &[], ""),
    e("fill-rule", Supported, &["nonzero", "evenodd"], &[], ""),
    e("filter", Partial, &["blur(2px)", "url(#f)"], &["grayscale(1)", "url(a.svg#f)", "sepia(1)", "brightness(2)", "contrast(2)", "saturate(2)", "hue-rotate(10deg)", "invert(1)", "opacity(0.5)"], "One `blur()`, one `drop-shadow()` and one same-document SVG `url(#id)`; the colour functions are not applied"),
    e("flex", Supported, &["1", "none"], &[], ""),
    e("flex-align", Unsupported, &[], &["center"], ""),
    e("flex-basis", Supported, &["auto", "10px", "calc-size(content, size * 2)"], &[], ""),
    e("flex-direction", Supported, &["row", "column"], &[], ""),
    e("flex-flow", Supported, &["row wrap", "column", "wrap-reverse"], &[], ""),
    e("flex-grow", Supported, &["0", "1"], &[], ""),
    e("flex-item-align", Unsupported, &[], &["center"], ""),
    e("flex-line-pack", Unsupported, &[], &["center"], ""),
    e("flex-negative", Unsupported, &[], &["1"], ""),
    e("flex-order", Unsupported, &[], &["1"], ""),
    e("flex-pack", Unsupported, &[], &["center"], ""),
    e("flex-positive", Unsupported, &[], &["1"], ""),
    e("flex-preferred-size", Unsupported, &[], &["auto"], ""),
    e("flex-shrink", Supported, &["0", "1"], &[], ""),
    e("flex-wrap", Supported, &["wrap", "nowrap"], &[], ""),
    e("float", Unsupported, &[], &["left"], "Use flex or grid"),
    e("flood-color", Unsupported, &[], &["red"], ""),
    e("font", Unsupported, &[], &["12px serif"], "Use the longhands"),
    e("font-family", Supported, &["serif", "\"Segoe UI\", sans-serif"], &[], ""),
    e("font-feature-settings", Supported, &["\"tnum\"", "normal"], &[], ""),
    e("font-kerning", Supported, &["none", "auto"], &[], ""),
    e("font-optical-sizing", Partial, &["auto"], &["none"], "Always `auto`"),
    e("font-palette", Unsupported, &[], &["--p"], ""),
    e("font-size", Supported, &["12px", "larger"], &[], ""),
    e("font-size-adjust", Unsupported, &[], &["0.5"], ""),
    e("font-smoothing", Supported, &["antialiased", "subpixel-antialiased", "auto"], &[], ""),
    e("font-stretch", Unsupported, &[], &["condensed"], ""),
    e("font-style", Supported, &["normal", "italic"], &[], ""),
    e("font-synthesis", Unsupported, &[], &["none"], ""),
    e("font-variant", Unsupported, &[], &["small-caps"], "`font-feature-settings: \"smcp\"` where the font has it"),
    e("font-variant-caps", Unsupported, &[], &["small-caps"], "`font-feature-settings: \"smcp\"` where the font has it"),
    e("font-variant-east-asian", Unsupported, &[], &["ruby"], ""),
    e("font-variant-ligatures", Supported, &["none", "normal"], &[], ""),
    e("font-variant-numeric", Supported, &["tabular-nums", "normal"], &[], ""),
    e("font-variation-settings", Partial, &["normal"], &["\"wght\" 400"], "`font-weight` and `font-style` reach the matching axes"),
    e("font-weight", Supported, &["bold", "450"], &[], ""),
    e("font-width", Unsupported, &[], &["condensed"], ""),
    e("forced-color-adjust", Unsupported, &[], &["none"], ""),
    e("gap", Supported, &["0", "10px 20px"], &[], ""),
    e("grid", Unsupported, &[], &["auto / 1fr 1fr"], "Use the longhands"),
    e("grid-area", Partial, &["auto", "1 / 2"], &["a"], "Named areas are not mapped"),
    e("grid-auto-columns", Supported, &["auto", "1fr"], &[], ""),
    e("grid-auto-flow", Supported, &["row", "column"], &[], ""),
    e("grid-auto-rows", Supported, &["auto", "1fr"], &[], ""),
    e("grid-column", Partial, &["auto", "1 / 3"], &["a"], "Named lines are not mapped"),
    e("grid-column-end", Partial, &["auto", "span 2"], &["a"], "Named lines are not mapped"),
    e("grid-column-start", Partial, &["auto", "2"], &["a"], "Named lines are not mapped"),
    e("grid-row", Partial, &["auto", "1 / 3"], &["a"], "Named lines are not mapped"),
    e("grid-row-end", Partial, &["auto", "span 2"], &["a"], "Named lines are not mapped"),
    e("grid-row-start", Partial, &["auto", "2"], &["a"], "Named lines are not mapped"),
    e("grid-template", Unsupported, &[], &["none"], "Use `grid-template-columns` / `-rows`"),
    e("grid-template-areas", Unsupported, &[], &["\"a b\""], "Place items by line number"),
    e("grid-template-columns", Partial, &["1fr 1fr", "repeat(3, 10px)"], &["none"], "`none` cannot undo an earlier track list"),
    e("grid-template-rows", Partial, &["1fr 1fr", "repeat(3, 10px)"], &["none"], "`none` cannot undo an earlier track list"),
    e("haptic", Supported, &["none", "press tick 0.4", "step tick 0.2, boundary edge 0.7", "activate pattern(--confirm)"], &[], "Skathi gesture feedback."),
    e("haptic-effect", Supported, &["tick", "hint, align", "pattern(--confirm)", "none"], &[], "Semantic effect or named pulse sequence."),
    e("haptic-intensity", Supported, &["0", "0.4, 1"], &[], "Zero silences the effect. Positive values use the controller's intensity capability."),
    e("haptic-min-interval", Supported, &["0ms", "30ms, 0.2s"], &[], "Minimum time between requests for each element, trigger and target."),
    e("haptic-sharpness", Partial, &["auto", "0.8"], &["2"], "Stored as a hint. Current AppKit and Windows pointing-device APIs use fixed waveform shapes."),
    e("haptic-target", Supported, &["auto", "pen", "mouse"], &[], "Defaults to the interaction device. Mouse includes touchpads."),
    e("haptic-texture", Supported, &["none", "ink", "pencil", "brush", "eraser"], &[], "Continuous texture on a contacting compatible pen."),
    e("haptic-texture-intensity", Supported, &["0", "0.35"], &[], "Strength of the pen texture on devices that support intensity."),
    e("haptic-trigger", Supported, &["none", "press", "step, boundary", "hover, activate, change, snap"], &[], "Non-inherited list of gestures."),
    e("height", Partial, &["auto", "10px", "calc-size(auto, size + 10px)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("hyphenate-character", Unsupported, &[], &["\"-\""], ""),
    e("hyphens", Partial, &["none", "manual"], &["auto"], "`auto` breaks only at soft hyphens"),
    e("image-orientation", Unsupported, &[], &["from-image"], ""),
    e("image-rendering", Partial, &["auto", "pixelated"], &["crisp-edges"], "Sets the sampling filter on native surfaces."),
    e("initial-letter", Unsupported, &[], &["2"], ""),
    e("inline-size", Partial, &["auto", "10px", "calc-size(fit-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("inset", Supported, &["auto", "0"], &[], ""),
    e("inset-block", Supported, &["auto", "0"], &[], ""),
    e("inset-block-end", Supported, &["auto", "0"], &[], ""),
    e("inset-block-start", Supported, &["auto", "0"], &[], ""),
    e("inset-inline", Supported, &["auto", "0"], &[], ""),
    e("inset-inline-end", Supported, &["auto", "0"], &[], ""),
    e("inset-inline-start", Supported, &["auto", "0"], &[], ""),
    e("interactivity", Unsupported, &[], &["inert"], ""),
    e("interpolate-size", Supported, &["numeric-only", "allow-keywords"], &[], ""),
    e("isolation", Supported, &["auto", "isolate"], &[], ""),
    e("justify-content", Partial, &["center", "space-between"], &["normal", "left"], "`normal`, `left` and `right` are not mapped"),
    e("justify-items", Supported, &["normal", "center"], &[], ""),
    e("justify-self", Supported, &["auto", "center"], &[], ""),
    e("left", Supported, &["auto", "10px"], &[], ""),
    e("letter-spacing", Supported, &["normal", "1px"], &[], ""),
    e("lighting-color", Unsupported, &[], &["red"], ""),
    e("line-break", Unsupported, &[], &["strict"], ""),
    e("line-clamp", Partial, &["2"], &["none"], "`none` cannot undo an earlier clamp"),
    e("line-height", Supported, &["normal", "1.5"], &[], ""),
    e("list-style", Supported, &["none", "disc inside"], &[], ""),
    e("list-style-image", Partial, &["none"], &["url(a.png)"], "Marker images are not drawn"),
    e("list-style-position", Supported, &["inside", "outside"], &[], ""),
    e("list-style-type", Partial, &["disc", "decimal"], &["hebrew"], "disc, circle, square, decimal, decimal-leading-zero, the alpha and roman styles"),
    e("margin", Supported, &["auto", "10px 20px"], &[], ""),
    e("margin-block", Supported, &["auto", "10px"], &[], ""),
    e("margin-block-end", Supported, &["auto", "10px"], &[], ""),
    e("margin-block-start", Supported, &["auto", "10px"], &[], ""),
    e("margin-bottom", Supported, &["auto", "10px"], &[], ""),
    e("margin-inline", Supported, &["auto", "10px"], &[], ""),
    e("margin-inline-end", Supported, &["auto", "10px"], &[], ""),
    e("margin-inline-start", Supported, &["auto", "10px"], &[], ""),
    e("margin-left", Supported, &["auto", "10px"], &[], ""),
    e("margin-right", Supported, &["auto", "10px"], &[], ""),
    e("margin-top", Supported, &["auto", "10px"], &[], ""),
    e("marker", Unsupported, &[], &["none"], ""),
    e("marker-end", Unsupported, &[], &["none"], ""),
    e("marker-mid", Unsupported, &[], &["none"], ""),
    e("marker-side", Unsupported, &[], &["match-self"], ""),
    e("marker-start", Unsupported, &[], &["none"], ""),
    e("mask", Partial, &["none", "url(a.png) center / contain no-repeat"], &[], "`url(#id)` references to an SVG `<mask>` are not applied; fixed-position descendants escape the mask"),
    e("mask-border", Unsupported, &[], &["none"], ""),
    e("mask-border-mode", Unsupported, &[], &["luminance"], ""),
    e("mask-border-outset", Unsupported, &[], &["0"], ""),
    e("mask-border-repeat", Unsupported, &[], &["stretch"], ""),
    e("mask-border-slice", Unsupported, &[], &["0"], ""),
    e("mask-border-source", Unsupported, &[], &["none"], ""),
    e("mask-border-width", Unsupported, &[], &["auto"], ""),
    e("mask-box-image", Unsupported, &[], &["none"], ""),
    e("mask-box-image-outset", Unsupported, &[], &["none"], ""),
    e("mask-box-image-repeat", Unsupported, &[], &["none"], ""),
    e("mask-box-image-slice", Unsupported, &[], &["none"], ""),
    e("mask-box-image-source", Unsupported, &[], &["none"], ""),
    e("mask-box-image-width", Unsupported, &[], &["none"], ""),
    e("mask-clip", Partial, &["border-box"], &["content-box"], "Only `border-box`"),
    e("mask-composite", Supported, &["add"], &[], ""),
    e("mask-image", Partial, &["none", "url(a.png)", "linear-gradient(#000, transparent)"], &[], "`url(#id)` references to an SVG `<mask>` are not applied"),
    e("mask-mode", Partial, &["alpha", "luminance"], &[], "`match-source` is treated as `alpha`; one mode for all layers"),
    e("mask-origin", Partial, &["border-box"], &["content-box"], "Only `border-box`"),
    e("mask-position", Supported, &["center", "10px 20%"], &[], ""),
    e("mask-position-x", Supported, &["center", "10px"], &[], ""),
    e("mask-position-y", Supported, &["center", "20%"], &[], ""),
    e("mask-repeat", Supported, &["repeat", "no-repeat"], &[], ""),
    e("mask-size", Supported, &["auto", "contain", "16px 16px"], &[], ""),
    e("mask-source-type", Unsupported, &[], &["none"], ""),
    e("mask-type", Unsupported, &[], &["luminance"], ""),
    e("max-block-size", Partial, &["none", "10px", "calc-size(fit-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("max-height", Partial, &["none", "10px", "calc-size(fit-content, size * .5)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("max-inline-size", Partial, &["none", "10px", "calc-size(max-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("max-width", Partial, &["none", "10px", "calc-size(max-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("min-block-size", Partial, &["auto", "10px", "calc-size(min-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("min-height", Partial, &["auto", "10px", "calc-size(min-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("min-inline-size", Partial, &["auto", "10px", "calc-size(min-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("min-width", Partial, &["auto", "10px", "calc-size(min-content, size)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("mix-blend-mode", Partial, &["normal", "plus-lighter"], &["multiply"], "`plus-lighter` blends the old and new view transition images"),
    e("-moz-osx-font-smoothing", Dropped, &[], &["grayscale"], ""),
    e("object-fit", Supported, &["cover", "contain"], &[], ""),
    e("object-position", Supported, &["center", "10px 20%"], &[], ""),
    e("offset-path", Unsupported, &[], &["none"], ""),
    e("opacity", Supported, &["0.5", "50%"], &[], ""),
    e("order", Supported, &["0", "-1"], &[], ""),
    e("outline", Supported, &["none", "2px solid red"], &[], ""),
    e("outline-color", Supported, &["red", "transparent"], &[], ""),
    e("outline-offset", Supported, &["0", "2px"], &[], ""),
    e("outline-style", Supported, &["none", "solid"], &[], ""),
    e("outline-width", Supported, &["0", "2px"], &[], ""),
    e("overflow", Supported, &["auto", "hidden"], &[], ""),
    e("overflow-clip-margin", Unsupported, &[], &["10px"], ""),
    e("overflow-wrap", Supported, &["normal", "break-word"], &[], ""),
    e("overflow-x", Supported, &["auto", "hidden"], &[], ""),
    e("overflow-y", Supported, &["auto", "hidden"], &[], ""),
    e("overscroll-behavior", Supported, &["none", "contain"], &[], ""),
    e("overscroll-behavior-block", Supported, &["none", "auto"], &[], ""),
    e("overscroll-behavior-inline", Supported, &["none", "auto"], &[], ""),
    e("overscroll-behavior-x", Supported, &["none", "auto"], &[], ""),
    e("overscroll-behavior-y", Supported, &["none", "auto"], &[], ""),
    e("padding", Supported, &["0", "10px 20px"], &[], ""),
    e("padding-block", Supported, &["0", "10px"], &[], ""),
    e("padding-block-end", Supported, &["0", "10px"], &[], ""),
    e("padding-block-start", Supported, &["0", "10px"], &[], ""),
    e("padding-bottom", Supported, &["0", "10px"], &[], ""),
    e("padding-inline", Supported, &["0", "10px"], &[], ""),
    e("padding-inline-end", Supported, &["0", "10px"], &[], ""),
    e("padding-inline-start", Supported, &["0", "10px"], &[], ""),
    e("padding-left", Supported, &["0", "10px"], &[], ""),
    e("padding-right", Supported, &["0", "10px"], &[], ""),
    e("padding-top", Supported, &["0", "10px"], &[], ""),
    e("paint-order", Unsupported, &[], &["stroke"], ""),
    e("perspective", Unsupported, &[], &["100px"], ""),
    e("perspective-origin", Unsupported, &[], &["center"], ""),
    e("place-content", Unsupported, &[], &["center"], "Use the `align-*` / `justify-*` longhands"),
    e("place-items", Supported, &["center", "start end"], &[], ""),
    e("place-self", Unsupported, &[], &["center"], "Use the `align-*` / `justify-*` longhands"),
    e("pointer-events", Partial, &["none", "auto"], &["visiblePainted"], "Only `none` and `auto`"),
    e("position", Supported, &["absolute", "sticky"], &[], ""),
    e("position-anchor", Unsupported, &[], &["--a"], ""),
    e("print-color-adjust", Unsupported, &[], &["exact"], ""),
    e("quotes", Unsupported, &[], &["none"], ""),
    e("r", Supported, &["5px", "50%"], &[], ""),
    e("resize", Partial, &["none"], &["both"], "There is no user-resize affordance"),
    e("right", Supported, &["auto", "10px"], &[], ""),
    e("rotate", Partial, &["none", "45deg", "calc(360deg / sibling-count() * sibling-index())"], &["x 45deg"], "Z-axis rotations"),
    e("row-gap", Supported, &["0", "10px"], &[], ""),
    e("rx", Supported, &["5px", "50%"], &[], ""),
    e("ry", Supported, &["5px", "50%"], &[], ""),
    e("scale", Unsupported, &[], &["2"], "Use `transform`"),
    e("scroll-behavior", Supported, &["auto", "smooth"], &[], ""),
    e("scroll-margin", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-block", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-block-end", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-block-start", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-bottom", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-inline", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-inline-end", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-inline-start", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-left", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-right", Supported, &["0", "10px"], &[], ""),
    e("scroll-margin-top", Supported, &["0", "10px"], &[], ""),
    e("scroll-padding", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-block", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-block-end", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-block-start", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-bottom", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-inline", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-inline-end", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-inline-start", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-left", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-right", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-padding-top", Supported, &["auto", "10px", "10%"], &[], ""),
    e("scroll-snap-align", Supported, &["start", "none", "start end"], &[], ""),
    e("scroll-snap-stop", Supported, &["normal", "always"], &[], ""),
    e("scroll-snap-type", Supported, &["none", "x mandatory", "both proximity"], &[], ""),
    e("scrollbar-color", Supported, &["auto", "red blue"], &[], ""),
    e("scrollbar-gutter", Unsupported, &[], &["stable"], ""),
    e("scrollbar-width", Supported, &["none", "thin"], &[], ""),
    e("shape-rendering", Supported, &["auto", "crispEdges"], &[], ""),
    e("stop-color", Supported, &["red", "currentColor"], &[], ""),
    e("stop-opacity", Supported, &["0.5", "50%"], &[], ""),
    e("stroke", Supported, &["none", "red"], &[], ""),
    e("stroke-dasharray", Supported, &["none", "4 2"], &[], ""),
    e("stroke-dashoffset", Supported, &["0", "2"], &[], ""),
    e("stroke-linecap", Supported, &["round", "butt"], &[], ""),
    e("stroke-linejoin", Supported, &["round", "bevel"], &[], ""),
    e("stroke-miterlimit", Supported, &["4"], &[], ""),
    e("stroke-opacity", Supported, &["0.5"], &[], ""),
    e("stroke-width", Supported, &["0", "2"], &[], ""),
    e("tab-size", Supported, &["4", "2.5", "24px", "1em"], &[], ""),
    e("table-layout", Unsupported, &[], &["fixed"], "Tables lay out as blocks"),
    e("text-align", Supported, &["center", "start"], &[], ""),
    e("text-align-last", Unsupported, &[], &["center"], ""),
    e("text-anchor", Unsupported, &[], &["middle"], ""),
    e("text-box", Partial, &["normal", "trim-start", "trim-both cap alphabetic"], &["ideographic"], "Applies to direct block text"),
    e("text-box-edge", Partial, &["auto", "text", "cap", "ex", "text alphabetic", "cap alphabetic", "ex alphabetic"], &["ideographic", "ideographic-ink"], "Uses the font's ascent, descent, cap height and x-height"),
    e("text-box-trim", Partial, &["none", "trim-start", "trim-end", "trim-both"], &[], "Applies to direct block text"),
    e("text-decoration", Partial, &["underline", "line-through red"], &["none"], "`none` cannot undo an earlier line; use `text-decoration-line: none`"),
    e("text-decoration-color", Supported, &["red", "transparent"], &[], ""),
    e("text-decoration-line", Supported, &["none", "underline"], &[], ""),
    e("text-decoration-skip-ink", Partial, &["none"], &["auto"], "The underline is one unbroken line"),
    e("text-decoration-style", Supported, &["solid", "wavy"], &[], ""),
    e("text-decoration-thickness", Supported, &["1px", "2px"], &[], ""),
    e("text-emphasis", Unsupported, &[], &["dot"], ""),
    e("text-emphasis-color", Unsupported, &[], &["red"], ""),
    e("text-emphasis-position", Unsupported, &[], &["under"], ""),
    e("text-emphasis-style", Unsupported, &[], &["dot"], ""),
    e("text-indent", Partial, &["0"], &["10px"], "There is no first line to indent; use `padding-left`"),
    e("text-justify", Unsupported, &[], &["inter-word"], ""),
    e("text-orientation", Unsupported, &[], &["upright"], ""),
    e("text-overflow", Partial, &["ellipsis"], &["clip"], "`clip` cannot undo an earlier ellipsis"),
    e("text-rendering", Unsupported, &[], &["optimizeLegibility"], ""),
    e("text-shadow", Supported, &["none", "1px 1px 2px red"], &[], ""),
    e("text-size-adjust", Ignored, &["none", "100%"], &[], ""),
    e("text-transform", Supported, &["none", "uppercase"], &[], ""),
    e("text-underline-offset", Supported, &["auto", "2px"], &[], ""),
    e("text-underline-position", Unsupported, &[], &["under"], "The underline is one unbroken line"),
    e("text-wrap", Unsupported, &[], &["balance"], ""),
    e("text-wrap-mode", Unsupported, &[], &["nowrap"], ""),
    e("text-wrap-style", Unsupported, &[], &["balance"], ""),
    e("top", Supported, &["auto", "10px"], &[], ""),
    e("touch-action", Ignored, &["none", "manipulation"], &[], ""),
    e("transform", Partial, &["none", "translate(1px) rotate(10deg)"], &["rotateX(10deg)", "rotateY(10deg)", "translateZ(1px)", "perspective(100px)", "rotate3d(1, 1, 0, 45deg)", "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)"], "2D only: rotation about the X or Y axis, depth and `perspective()` are not applied"),
    e("transform-box", Unsupported, &[], &["border-box"], ""),
    e("transform-origin", Supported, &["center", "0 0"], &[], ""),
    e("transition", Partial, &["opacity 1s ease", "display 1s allow-discrete", "none"], &[], "`linear()` easing is not parsed"),
    e("transition-behavior", Partial, &["normal", "allow-discrete", "normal, allow-discrete"], &[], "Discrete display, content visibility, cursor, flex flow and overflow"),
    e("transition-delay", Supported, &["1s", "-1s", "calc(sibling-index() * 40ms)"], &[], ""),
    e("transition-duration", Supported, &["1s"], &[], ""),
    e("transition-property", Supported, &["none", "opacity"], &[], ""),
    e("transition-timing-function", Partial, &["ease", "linear"], &[], "`linear()` easing is not parsed"),
    e("translate", Partial, &["none", "10px", "10% 2px"], &["10px 10px 5px"], "X and Y axes"),
    e("unicode-bidi", Unsupported, &[], &["isolate"], "Left-to-right text only"),
    e("user-select", Partial, &["none", "text"], &["contain"], "`contain` selects like `text`"),
    e("vector-effect", Unsupported, &[], &["non-scaling-stroke"], ""),
    e("vertical-align", Partial, &["baseline", "middle", "top", "bottom", "text-top", "text-bottom", "sub", "super", "4px", "-0.2em", "50%"], &[], "Places the items of a line of mixed inline content and the atoms of `<sk-editor>`. Ignored where CSS ignores it: on block-level, flex and grid items"),
    e("view-transition-class", Supported, &["card", "a b"], &[], ""),
    e("view-transition-group", Unsupported, &[], &["normal"], ""),
    e("view-transition-name", Supported, &["card", "none"], &[], ""),
    e("visibility", Supported, &["hidden", "visible"], &[], ""),
    e("-webkit-app-region", Supported, &["drag", "no-drag"], &[], ""),
    e("-webkit-box-reflect", Unsupported, &[], &["below"], ""),
    e("-webkit-line-clamp", Partial, &["2"], &["none"], "`none` cannot undo an earlier clamp"),
    e("-webkit-mask-composite", Supported, &["source-over"], &[], ""),
    e("-webkit-tap-highlight-color", Dropped, &[], &["transparent"], ""),
    e("-webkit-text-fill-color", Supported, &["red", "transparent"], &[], ""),
    e("-webkit-text-stroke", Unsupported, &[], &["1px red"], ""),
    e("-webkit-text-stroke-color", Unsupported, &[], &["red"], ""),
    e("-webkit-text-stroke-width", Unsupported, &[], &["1px"], ""),
    e("-webkit-touch-callout", Dropped, &[], &["none"], ""),
    e("white-space", Supported, &["normal", "nowrap"], &[], ""),
    e("white-space-collapse", Unsupported, &[], &["preserve"], ""),
    e("width", Partial, &["auto", "10px", "calc-size(fit-content, size + 2em)"], SIZING_KEYWORDS, SIZING_NOTE),
    e("will-change", Ignored, &["transform", "auto"], &[], ""),
    e("window-drag", Supported, &["drag", "no-drag"], &[], ""),
    e("word-break", Supported, &["normal", "break-all"], &[], ""),
    e("word-spacing", Supported, &["normal", "2px"], &[], ""),
    e("word-wrap", Supported, &["normal", "break-word"], &[], ""),
    e("writing-mode", Unsupported, &[], &["vertical-rl"], "Horizontal text only"),
    e("x", Supported, &["10px", "50%"], &[], ""),
    e("y", Supported, &["10px", "50%"], &[], ""),
    e("z-index", Supported, &["auto", "10"], &[], ""),
    e("zoom", Unsupported, &[], &["1.5"], ""),
];

pub fn entry(name: &str) -> Option<&'static Entry> {
    let name = name.trim();
    CATALOG.iter().find(|e| e.name.eq_ignore_ascii_case(name))
}

/// `CSS.supports(property, value)`: the mapper applies the declaration, and the catalog does not list
/// the value as one it ignores (`width: fit-content` is accepted and changes nothing).
pub fn supports(name: &str, value: &str) -> bool {
    if !accepts_declaration(name, value) {
        return false;
    }
    let name = name.trim();
    let name = name
        .strip_prefix("-webkit-")
        .filter(|base| {
            crate::prefix::WEBKIT_APPLIED
                .iter()
                .any(|n| n.eq_ignore_ascii_case(base))
        })
        .unwrap_or(name);
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    entry(name).is_none_or(|e| !e.refused.iter().any(|r| r.eq_ignore_ascii_case(&value)))
}

/// Is the `-webkit-` spelling read as this property?
pub fn webkit_applied(name: &str) -> bool {
    crate::prefix::WEBKIT_APPLIED
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
}

fn json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_list(out: &mut String, items: impl IntoIterator<Item = impl AsRef<str>>) {
    out.push('[');
    for (i, s) in items.into_iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        json_string(out, s.as_ref());
    }
    out.push(']');
}

/// The catalog as JSON, one object per property, with the advisory the engine prints for the first
/// refused value.
pub fn to_json() -> String {
    let mut out = String::from("[\n");
    for (i, e) in CATALOG.iter().enumerate() {
        out.push_str("  {\"name\": ");
        json_string(&mut out, e.name);
        out.push_str(", \"status\": ");
        json_string(&mut out, e.status.as_str());
        out.push_str(", \"note\": ");
        json_string(&mut out, e.note);
        out.push_str(", \"applied\": ");
        json_list(&mut out, e.applied);
        out.push_str(", \"refused\": ");
        json_list(&mut out, e.refused);
        out.push_str(", \"webkit\": ");
        out.push_str(if webkit_applied(e.name) {
            "true"
        } else {
            "false"
        });
        out.push_str(", \"platforms\": ");
        json_list(
            &mut out,
            e.platforms.iter().map(|p| match p {
                Platform::Windows => "windows",
                Platform::MacOs => "macos",
                Platform::Linux => "linux",
            }),
        );
        out.push_str(", \"advisory\": ");
        match e.refused.first().map(|v| verdict(e.name, v)) {
            Some(Verdict::Rejected(msg)) => json_string(&mut out, &msg),
            _ => out.push_str("null"),
        }
        out.push('}');
        if i + 1 < CATALOG.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The anti-drift guard: every entry's samples get the verdict its status promises.
    #[test]
    fn the_catalog_agrees_with_the_mapper() {
        let mut wrong = Vec::new();
        for e in CATALOG {
            let mut check = |value: &str, ok: bool, want: &str| {
                if !ok {
                    wrong.push(format!(
                        "{}: {value} is {:?}, the catalog says {want} ({:?})",
                        e.name,
                        verdict(e.name, value),
                        e.status
                    ));
                }
            };
            for v in e.applied {
                let got = verdict(e.name, v);
                match e.status {
                    Supported | Partial => check(
                        v,
                        matches!(got, Verdict::Applied | Verdict::Inert) && supports(e.name, v),
                        "applied",
                    ),
                    Ignored => check(v, got == Verdict::Inert && supports(e.name, v), "inert"),
                    Unsupported | Dropped => check(v, false, "nothing applied"),
                }
            }
            for v in e.refused {
                let got = verdict(e.name, v);
                match e.status {
                    Partial => check(
                        v,
                        matches!(got, Verdict::Rejected(_) | Verdict::Inert)
                            && !supports(e.name, v),
                        "refused",
                    ),
                    Unsupported => check(v, matches!(got, Verdict::Rejected(_)), "rejected"),
                    Dropped => check(v, got == Verdict::Dropped, "dropped"),
                    Supported | Ignored => check(v, false, "no refused values"),
                }
            }
            let shape = match e.status {
                Supported => {
                    !e.applied.is_empty()
                        && e.applied
                            .iter()
                            .any(|v| verdict(e.name, v) == Verdict::Applied)
                }
                Partial => !e.applied.is_empty() && !e.note.is_empty(),
                Ignored => !e.applied.is_empty(),
                Unsupported | Dropped => e.applied.is_empty() && !e.refused.is_empty(),
            };
            if !shape {
                wrong.push(format!("{}: samples do not fit {:?}", e.name, e.status));
            }
        }
        assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
    }

    #[test]
    fn every_property_the_mapper_names_is_catalogued() {
        let src = include_str!("mapper.rs");
        let mut missing = Vec::new();
        for (at, _) in src.match_indices("CustomPropertyName::Unknown(name)") {
            let guard = &src[at..];
            let guard = &guard[..guard.find("=>").unwrap_or(0)];
            for literal in guard.split('"').skip(1).step_by(2) {
                let base = literal
                    .strip_prefix("-webkit-")
                    .filter(|b| webkit_applied(b))
                    .unwrap_or(literal);
                if entry(literal).is_none() && entry(base).is_none() {
                    missing.push(literal.to_string());
                }
            }
        }
        for name in crate::prefix::WEBKIT_APPLIED {
            let known = lightningcss::properties::PropertyId::from(*name);
            let unknown = matches!(known, lightningcss::properties::PropertyId::Custom(_));
            if entry(name).is_none() && entry(&format!("-webkit-{name}")).is_none() && unknown {
                missing.push(format!("-webkit-{name}"));
            }
        }
        let mut names: Vec<_> = CATALOG.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CATALOG.len(), "a property is listed twice");
        assert!(missing.is_empty(), "not in the catalog: {missing:?}");
    }

    #[test]
    fn supports_answers_what_the_engine_applies() {
        assert!(supports("order", "2"));
        assert!(supports("object-position", "center top"));
        assert!(supports("contain", "paint"));
        assert!(supports("scroll-behavior", "smooth"));
        assert!(supports("filter", "blur(4px)"));
        assert!(supports("box-shadow", "none"));
        assert!(supports("clip-path", "path(\"M0 0L10 10Z\")"));
        assert!(supports("width", "var(--w)"));
        assert!(supports("--anything", "at all"));
        assert!(supports("-webkit-user-select", "none"));
        assert!(!supports("width", "fit-content"));
        assert!(!supports("WIDTH", " Fit-Content "));
        assert!(!supports("animation-timing-function", "linear(0, 1)"));
        assert!(!supports("transition", "opacity 1s linear(0, 1)"));
        assert!(!supports("float", "left"));
        assert!(!supports("width", "red"));
        assert!(!supports("-moz-appearance", "none"));
        assert!(!supports("display", "flex; color: red"));
    }

    #[test]
    fn condition_text_is_evaluated_as_at_supports_is() {
        use crate::supports_condition_text as cond;
        assert!(cond("order: 2"));
        assert!(cond("(display: flex) and (not (float: left))"));
        assert!(cond(
            "(-webkit-backdrop-filter: blur(2px)) or (backdrop-filter: blur(2px))"
        ));
        assert!(!cond("(width: fit-content)"));
        assert!(!cond("(-moz-appearance: none)"));
        assert!(!cond("display: flex; color: red"));
        assert!(!cond(""));
    }

    #[test]
    fn the_json_dump_is_one_object_per_property() {
        let parsed: serde_json::Value = serde_json::from_str(&to_json()).expect("valid JSON");
        let list = parsed.as_array().unwrap();
        assert_eq!(list.len(), CATALOG.len());
        let width = list.iter().find(|p| p["name"] == "width").unwrap();
        assert_eq!(width["status"], "partial");
        assert_eq!(width["refused"][0], "min-content");
        let float = list.iter().find(|p| p["name"] == "float").unwrap();
        assert!(float["advisory"].as_str().is_some_and(|a| !a.is_empty()));
    }
}
