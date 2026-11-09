//! Inline `<svg>` — the element becomes a LEAF that gpui rasterises.
//!
//! ## Why serialize instead of lowering
//!
//! gpui can paint an SVG (`gpui::svg().data(&bytes)` → `Window::paint_svg` →
//! resvg), but only from a **document**: a byte string it hands to usvg. It has
//! no notion of "an element tree that happens to be SVG". Our retained tree, on
//! the other hand, holds the inline subtree Vue built — `<svg>` with `<path>`,
//! `<circle>`, `<rect>` under it, each a real node with real attributes.
//!
//! So the bridge is a serializer: walk the subtree once, write the XML resvg
//! expects, hand the bytes to gpui. The `<svg>` element becomes a **leaf** at
//! render level — its children are deliberately NOT lowered into taffy nodes or
//! painted divs, because they are already inside the rasterised document and
//! lowering them too would double-render every icon (a stack of empty boxes
//! behind the glyph, each one a flex item distorting the parent's `gap`).
//!
//! ## What the cascade contributes
//!
//! Two things, and they arrive by different routes:
//!
//! * **size** — `w-4 h-4` is ordinary CSS on the `<svg>` element itself, so it
//!   rides `apply_computed_style` like any other box. resvg rasterises into
//!   whatever rectangle gpui ends up laying the leaf out at, in device pixels,
//!   so the picture is crisp at every size and every scale factor.
//! * **colour** — through one of two paths, chosen per document by
//!   [`Paint`]:
//!
//!   - **the mask path** ([`Paint::Mono`]): gpui renders the SVG as a
//!     single-colour **alpha mask** tinted by `style.text.color`
//!     (`MonochromeSprite`). It is exact whenever every painted part is
//!     `currentColor`, which is what the icon sets everybody ships (lucide,
//!     heroicons) say, because then the picture IS "`color` × coverage". It
//!     is cheap and atlas-cached, and a `:hover` that changes `color`
//!     re-tints the sprite without re-rasterising. Opacity, `fill-opacity` and
//!     overlaps stay on this path: they only change the coverage, which the
//!     mask carries per pixel. See `render::SvgTint` for where the inherited
//!     colour is read.
//!   - **the colour path** ([`Paint::Colour`]): anything else (a concrete
//!     colour, two colours, a gradient or pattern, an `<image>`, a filter, a
//!     blend mode, a `<style>` element) is rasterised in FULL COLOUR by resvg,
//!     with `currentColor` resolved to the element's CSS `color`, and painted
//!     as an image ([`SvgRaster`]). The raster is cached per node by
//!     [`RasterKey`]: the serialized source, the device-pixel size and scale,
//!     and the resolved `currentColor` (only when the document uses it). A
//!     colour change re-rasterises at most once per [`RECOLOUR_EVERY`] per
//!     node, so a hover transition on a multicolour SVG costs a handful of
//!     rasters, not one a frame; the last colour always lands.
//!
//!   The viewport of a colour raster is the element's box: the root's
//!   `width`/`height` are replaced by the laid-out CSS size, and `viewBox` /
//!   `preserveAspectRatio` map the content into it, as a browser does with a
//!   CSS-sized `<svg>`. A root with NO `viewBox` always takes this path: its
//!   user units are the CSS pixels of its box, which only the raster's sizing
//!   gives (the mask path scales the document from usvg's own size, 100x100
//!   when the root has no width/height either).
//!
//! ## The cascade's presentation properties
//!
//! Every element of the document, the `<svg>` itself and each `<g>` and shape
//! inside it, has its own computed style (the cascade walks the whole tree).
//! What it computed for SVG's presentation properties is written on that
//! element in place of the markup's attribute of the same name
//! ([`cascaded`]):
//!
//! * `fill` and `stroke` (`vn_css::SvgPaint`): `none`, `currentColor`, or the
//!   concrete colour, `var()` resolved and alpha kept — Tailwind's
//!   `fill-current` that makes the shadcn radio dot visible, and the EXOBIO
//!   overlay's `fill: var(--notch-surface)`, alike. A paint server
//!   (`url(…)`) or `context-fill` from a rule is drawn in `currentColor`, and
//!   the advisory `svg-css-paint-server` says so;
//! * `stroke-width`, `stroke-opacity`, `fill-opacity`, `fill-rule`, the dash,
//!   cap, join and miter properties and `shape-rendering`, as CSS text
//!   (`vn_css::Extras::svg_presentation`);
//! * inside the document only, `opacity` and `display: none` (on the `<svg>`
//!   itself those are the box's, and gpui applies them).
//!
//! That is the browser's order: a presentation attribute loses to every
//! author rule, and the inline `style` — already folded into the computed
//! value with the rules, `!important` included — wins over both. Inheritance
//! is resvg's: a value written on the `<svg>` or a `<g>` is inherited by the
//! shapes under it exactly as the computed value is in a browser. What is NOT
//! read: a `color` a rule gives an element inside the document (its
//! `currentColor` is the `<svg>`'s), and a `:hover`/`:focus`/`:active` or
//! `group-` rule's values (vn-css drops them with an advisory: `Extras` have
//! no state ladder, so they would be drawn at rest).
//!
//! ## `filter: drop-shadow()`
//!
//! A browser blurs the alpha of what the element painted, so the shadow of an
//! icon is the icon's silhouette. resvg has the same primitive, so a
//! `drop-shadow()` on the `<svg>` puts the document on the colour path and
//! wraps it in a `<filter><feDropShadow/></filter>` (see [`raster_source`]).
//! The raster is grown by the shadow's reach on each side ([`Shadow::margins`])
//! and painted that much outside the box. The svg's own CSS background and
//! border, which gpui paints, cast no shadow (render.rs advises).
//!
//! ## Re-serialization
//!
//! Keyed on a hash of everything that can change the bytes: every tag, every
//! attribute name and value, every inline-style declaration, every text node in
//! the subtree, and every element's cascaded presentation properties. The
//! serialization behind it runs only when the hash moves. The colour is
//! deliberately not in the key (see above).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, Bounds, Pixels, RenderImage, Window};

use crate::tree::{Kind, Tree};
use crate::windows::{self, WinKey};

/// One serialized `<svg>`, and what the renderer needs to know about it.
pub(crate) struct SvgDoc {
    /// The document resvg reads.
    pub(crate) bytes: Vec<u8>,
    /// A hash of `bytes`: the "serialized source" part of a [`RasterKey`].
    pub(crate) source: u64,
    /// Which path paints it.
    pub(crate) paint: Paint,
    /// The document says `currentColor` somewhere, so its colour raster
    /// depends on the element's CSS `color`.
    pub(crate) uses_current: bool,
    /// The cascade gave an element a paint vn-css does not carry (a paint
    /// server, `context-fill`), which is therefore drawn in `currentColor`.
    pub(crate) css_paint_server: bool,
}

/// How a document paints: the colour-independent mask, or full colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    /// Every painted part is `currentColor`, or nothing is painted.
    Mono,
    /// Anything else, with the first reason found.
    Colour(&'static str),
}

thread_local! {
    /// node id → (subtree hash, serialized document). One entry per live
    /// `<svg>` element; GC'd with the rest of the per-node state.
    static CACHE: RefCell<HashMap<u32, (u64, Rc<SvgDoc>)>> = RefCell::new(HashMap::new());
}

/// Drops cached documents and colour rasters for nodes that no longer exist.
/// A dropped raster's atlas tile goes back to the window that painted it, at
/// that window's next frame.
pub(crate) fn gc(alive: impl Fn(u32) -> bool) {
    CACHE.with(|m| m.borrow_mut().retain(|&id, _| alive(id)));
    let dead: Vec<NodeRaster> = RASTERS.with(|m| {
        let mut m = m.borrow_mut();
        let ids: Vec<u32> = m.keys().copied().filter(|&id| !alive(id)).collect();
        ids.into_iter().filter_map(|id| m.remove(&id)).collect()
    });
    for r in dead {
        windows::queue_drops(r.win, [r.image]);
    }
}

/// The SVG document for this `<svg>` node, re-serialized only if the subtree or
/// a computed presentation property in it moved since the last call.
pub(crate) fn document(t: &Tree, id: u32) -> Rc<SvgDoc> {
    let key = hash_subtree(t, id);
    CACHE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((cached, doc)) = m.get(&id) {
            if *cached == key {
                return doc.clone();
            }
        }
        let doc = Rc::new(build(t, id));
        m.insert(id, (key, doc.clone()));
        doc
    })
}

/// Whether replay skips no raster clock or dependency outside this SVG.
pub(crate) fn can_replay(t: &Tree, id: u32) -> bool {
    if !replayable_geometry(t, id, true, false) {
        return false;
    }
    let doc = document(t, id);
    doc.paint == Paint::Mono && !doc.css_paint_server
}

fn replayable_geometry(t: &Tree, id: u32, root: bool, metadata: bool) -> bool {
    let Some(node) = t.node(id) else { return false };
    match node.kind {
        Kind::Anchor => return !root && node.children.iter().all(|&child| replayable_geometry(t, child, false, metadata)),
        Kind::Text => return !root && (metadata || node.text.trim().is_empty()),
        Kind::Element => {}
    }
    let tag = t.name(node.tag);
    let metadata = matches!(tag, "title" | "desc");
    if (root && tag != "svg") || (!root && !matches!(
        tag, "g" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "title" | "desc"
    )) {
        return false;
    }
    // A strict geometry whitelist keeps text/font loading, <use>, embedded
    // images, filters, foreignObject, and SMIL timelines out of a recording.
    if (!metadata && !node.text.trim().is_empty()) || t.is_group_anchor(id) {
        return false;
    }
    if let Some(cs) = &node.computed {
        if cs.extras.drop_shadow.is_some() || cs.extras.filter_url.is_some() || !cs.animations.is_empty()
            || cs.extras.background_urls.as_ref().is_some_and(|urls| !urls.is_empty())
            || cs.extras.mask_images.is_some() {
            return false;
        }
        // Descendants are serialized, not GPUI interactive elements. Restrict
        // them to their base presentation; the root's own states still use
        // GPUI's replayed listeners and the ordinary HOT animation exclusion.
        if !root && (cs.hover.is_some() || cs.active.is_some() || cs.focus.is_some()
            || cs.focus_visible.is_some() || cs.focus_within.is_some()
            || cs.group_hover.is_some() || cs.group_active.is_some()
            || !cs.transitions.is_empty()) {
            return false;
        }
    }
    for (key, value, _) in &node.props {
        let name = t.name(*key);
        if matches_ascii(name, &["href", "xlink:href", "src", "xml:base", "externalResourcesRequired",
            "begin", "end", "dur", "repeatCount", "repeatDur", "keyTimes", "keySplines",
            "calcMode", "attributeName", "attributeType", "restart", "syncBehavior", "syncTolerance"])
            || (name.eq_ignore_ascii_case("style") && !value.trim().is_empty())
            || replay_context_value(value)
        {
            return false;
        }
    }
    for (key, value) in &node.style {
        if t.name(*key).starts_with("animation") || replay_context_value(value) {
            return false;
        }
    }
    node.children.iter().all(|&child| replayable_geometry(t, child, false, metadata))
}

fn matches_ascii(value: &str, names: &[&str]) -> bool {
    names.iter().any(|name| value.eq_ignore_ascii_case(name))
}

fn replay_context_value(value: &str) -> bool {
    ["url(", "context-fill", "context-stroke", "@import"].into_iter().any(|dependency| {
        value.as_bytes().windows(dependency.len()).any(|part| part.eq_ignore_ascii_case(dependency.as_bytes()))
    })
}

fn build(t: &Tree, id: u32) -> SvgDoc {
    let mut out = String::with_capacity(256);
    let mut class = Class::default();
    write_element(t, id, true, &Inherited::root(), &mut class, &mut out);
    let uses_current = out.to_ascii_lowercase().contains("currentcolor");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    out.hash(&mut h);
    let css_paint_server = class.server;
    SvgDoc {
        source: h.finish(),
        paint: class.finish(),
        uses_current,
        css_paint_server,
        bytes: out.into_bytes(),
    }
}

// ---------------------------------------------------------------------------
// what the cascade gives each element
// ---------------------------------------------------------------------------

/// The presentation properties the cascade computed for one element, as the
/// attributes written in place of the markup's own of the same name. Empty for
/// an element the cascade has not styled.
///
/// `opacity` and `display: none` are written only INSIDE the document: on the
/// `<svg>` they are the box's, which gpui applies to the whole picture. The
/// root's `overflow` is written for the drop-shadow wrapper, where the root
/// becomes a nested `<svg>`: `visible` lets its shapes past the box, as far as
/// the shadow's margins reach, and anything else clips them to it.
fn cascaded(n: &crate::tree::Node, root: bool) -> Vec<(&'static str, Cow<'static, str>)> {
    let Some(cs) = &n.computed else { return Vec::new() };
    let x = &cs.extras;
    let mut out: Vec<(&'static str, Cow<'static, str>)> = Vec::new();
    if let Some(p) = x.fill {
        out.push(("fill", p.as_attr()));
    }
    if let Some(p) = x.stroke {
        out.push(("stroke", p.as_attr()));
    }
    if let Some(list) = &x.svg_presentation {
        for (name, value) in list.iter() {
            out.push((name, Cow::Owned(value.to_string())));
        }
    }
    if root {
        match cs.base.overflow.x {
            Some(gpui::Overflow::Visible) => out.push(("overflow", "visible".into())),
            Some(_) => out.push(("overflow", "hidden".into())),
            None => {}
        }
    } else {
        if let Some(o) = cs.base.opacity {
            out.push(("opacity", Cow::Owned(format!("{}", o.clamp(0.0, 1.0)))));
        }
        if cs.base.display == Some(gpui::Display::None) {
            out.push(("display", "none".into()));
        }
    }
    out
}

/// [`cascaded`]'s inputs, hashed without building the attribute text.
fn hash_cascaded(n: &crate::tree::Node, root: bool, h: &mut impl Hasher) {
    let Some(cs) = &n.computed else {
        0u8.hash(h);
        return;
    };
    1u8.hash(h);
    let x = &cs.extras;
    x.fill.hash(h);
    x.stroke.hash(h);
    x.svg_presentation.hash(h);
    if root {
        cs.base.overflow.x.map(|o| o == gpui::Overflow::Visible).hash(h);
    } else {
        cs.base.opacity.map(f32::to_bits).hash(h);
        (cs.base.display == Some(gpui::Display::None)).hash(h);
    }
}

/// Does the cascade write this attribute name on the element?
fn overridden(declared: &[(&'static str, Cow<'static, str>)], name: &str) -> bool {
    declared.iter().any(|(p, _)| name.eq_ignore_ascii_case(p))
}

// ---------------------------------------------------------------------------
// mono or colour: what the shapes paint with
// ---------------------------------------------------------------------------

/// A paint as it reaches a shape, after inheritance.
#[derive(Clone, Debug, PartialEq)]
enum Eff {
    None,
    /// `currentColor`, resolved per shape (CSS Color 4: it inherits as the
    /// keyword, so a child with its own `color` resolves it differently).
    Current,
    /// `url(#…)`: a gradient or a pattern.
    Url,
    /// A concrete colour, lowercased with its whitespace collapsed.
    Concrete(String),
}

/// What an element says about a paint of its own.
enum Spec {
    Unset,
    Inherit,
    Is(Eff),
}

fn normalized(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

fn paint_spec(v: &str) -> Spec {
    let l = normalized(v);
    match l.as_str() {
        "" => Spec::Unset,
        "inherit" => Spec::Inherit,
        "none" | "transparent" => Spec::Is(Eff::None),
        "currentcolor" => Spec::Is(Eff::Current),
        _ if l.starts_with("url(") => Spec::Is(Eff::Url),
        _ => Spec::Is(Eff::Concrete(l)),
    }
}

/// The inherited half of the paint context.
#[derive(Clone)]
struct Inherited {
    fill: Eff,
    stroke: Eff,
    /// A `color` set INSIDE the document; `None` is the element's CSS `color`.
    color: Option<String>,
    /// False inside `<clipPath>` / `<mask>`: their content shapes coverage,
    /// which the mask carries, and paints no colour of its own.
    painted: bool,
}

impl Inherited {
    /// SVG's initial values: `fill: black`, `stroke: none`.
    fn root() -> Self {
        Inherited { fill: Eff::Concrete("black".into()), stroke: Eff::None, color: None, painted: true }
    }
}

/// What the shapes of one document paint with, collected while it is written.
#[derive(Default)]
struct Class {
    /// Distinct concrete paints.
    paints: Vec<String>,
    /// Why the document needs full colour, when something other than its
    /// paints decides that.
    reason: Option<&'static str>,
    /// Some element's computed paint is one vn-css does not carry
    /// (`vn_css::SvgPaint::Server`).
    server: bool,
}

impl Class {
    fn colour(&mut self, why: &'static str) {
        self.reason.get_or_insert(why);
    }

    fn paint(&mut self, p: &Eff, color: &Option<String>) {
        match p {
            Eff::None => {}
            Eff::Current => {
                if let Some(c) = color {
                    self.concrete(c);
                }
            }
            Eff::Url => self.colour("a gradient or pattern paint (`url(…)`)"),
            Eff::Concrete(c) => self.concrete(c),
        }
    }

    fn concrete(&mut self, c: &str) {
        if !self.paints.iter().any(|p| p == c) {
            self.paints.push(c.to_string());
        }
    }

    fn finish(self) -> Paint {
        match (self.reason, self.paints.len()) {
            (Some(why), _) => Paint::Colour(why),
            (None, 0) => Paint::Mono,
            (None, 1) => Paint::Colour("a concrete fill/stroke colour (SVG's default fill is black)"),
            (None, _) => Paint::Colour("more than one fill/stroke colour"),
        }
    }
}

/// Why a root with no `viewBox` takes the colour path (see `classify`).
const NO_VIEWBOX: &str = "no viewBox (its user units are the CSS pixels of its box)";

/// The elements whose geometry is painted with `fill` and `stroke`.
fn is_shape(tag: &str) -> bool {
    matches!(
        tag,
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "text" | "tspan" | "textPath"
    )
}

/// An element that paints colours a mask cannot carry, whatever the paints say.
fn colour_element(tag: &str) -> Option<&'static str> {
    match tag {
        "linearGradient" | "radialGradient" | "pattern" => Some("a gradient or pattern"),
        "image" => Some("an <image>"),
        "filter" => Some("a filter"),
        "style" => Some("a <style> element (its rules are not analysed)"),
        _ => None,
    }
}

/// Folds one element's presentation attributes, then what the cascade
/// computed for it, then the inline declarations the cascade does not carry
/// (the order `write_element` writes them in, and the browser's) into its
/// context, and records what a shape paints.
#[allow(clippy::too_many_arguments)]
fn classify(
    t: &Tree,
    n: &crate::tree::Node,
    tag: &str,
    root: bool,
    declared: &[(&'static str, Cow<'static, str>)],
    parent: &Inherited,
    class: &mut Class,
) -> Inherited {
    let mut own_fill = Spec::Unset;
    let mut own_stroke = Spec::Unset;
    let mut own_color: Option<String> = None;
    let mut color_inherit = false;
    let mut look = |name: &str, value: &str, own_fill: &mut Spec, own_stroke: &mut Spec, class: &mut Class| {
        match name {
            "fill" => *own_fill = paint_spec(value),
            "stroke" => *own_stroke = paint_spec(value),
            "color" => match normalized(value).as_str() {
                "" | "inherit" | "currentcolor" => color_inherit = true,
                c => {
                    color_inherit = false;
                    own_color = Some(c.to_string());
                }
            },
            "filter" if !matches!(normalized(value).as_str(), "" | "none") => class.colour("a filter"),
            "mix-blend-mode" if !matches!(normalized(value).as_str(), "" | "normal") => {
                class.colour("a blend mode")
            }
            _ => {}
        }
    };
    for (k, v, _) in &n.props {
        look(t.name(*k), v, &mut own_fill, &mut own_stroke, class);
    }
    for (name, value) in declared {
        look(name, value, &mut own_fill, &mut own_stroke, class);
    }
    for (k, v) in &n.style {
        let name = t.name(*k);
        if !overridden(declared, name) {
            look(name, v, &mut own_fill, &mut own_stroke, class);
        }
    }
    if let Some(cs) = &n.computed {
        let server = |p: Option<vn_css::SvgPaint>| p == Some(vn_css::SvgPaint::Server);
        class.server |= server(cs.extras.fill) || server(cs.extras.stroke);
    }
    let resolve = |spec: Spec, inherited: &Eff| match spec {
        Spec::Unset | Spec::Inherit => inherited.clone(),
        Spec::Is(e) => e,
    };
    let here = Inherited {
        fill: resolve(own_fill, &parent.fill),
        stroke: resolve(own_stroke, &parent.stroke),
        color: if own_color.is_some() && !color_inherit { own_color } else { parent.color.clone() },
        painted: parent.painted && !matches!(tag, "clipPath" | "mask"),
    };
    if let Some(why) = colour_element(tag) {
        class.colour(why);
    }
    // Without a `viewBox` the document's user units ARE the CSS pixels of the
    // element's box (no scaling at all), whatever `width`/`height` say. The
    // mask path scales the document to the box from its own size, and usvg
    // sizes a root with neither at 100x100, so Argon's broadcast connector
    // (a path in px down a list hundreds of pixels tall) was scaled and cut at
    // y = 100 into nothing. The colour raster is drawn in the box's own size.
    if root && !n.props.iter().any(|(k, _, _)| t.name(*k) == "viewBox") {
        class.colour(NO_VIEWBOX);
    }
    if here.painted && is_shape(tag) {
        // A line has no interior, so its fill paints nothing.
        if tag != "line" {
            class.paint(&here.fill, &here.color);
        }
        class.paint(&here.stroke, &here.color);
    }
    here
}

// ---------------------------------------------------------------------------
// the colour path: a raster per node
// ---------------------------------------------------------------------------

/// What one full-colour raster is a picture of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RasterKey {
    /// [`SvgDoc::source`].
    pub(crate) source: u64,
    /// The element's box, in device pixels.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// The scale factor ×1000. Without a `viewBox` the content is drawn 1:1
    /// in CSS pixels, so one device size at two scales is two pictures.
    pub(crate) scale_milli: u32,
    /// What `currentColor` resolved to, 8 bits a channel. `None` when the
    /// document never says `currentColor`, so its colour cannot matter.
    pub(crate) color: Option<[u8; 4]>,
    /// The `drop-shadow()`, as [`Shadow::key`]. With one, `width`/`height`
    /// are the box grown by its margins.
    pub(crate) shadow: Option<([u32; 3], [u8; 4])>,
}

/// A `filter: drop-shadow()` on the `<svg>`, its colour resolved: drawn by
/// resvg around the shapes (`feDropShadow`), not around the box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shadow {
    /// Offset, CSS px.
    pub(crate) dx: f32,
    pub(crate) dy: f32,
    /// The Gaussian's standard deviation, CSS px (CSS's third length).
    pub(crate) sigma: f32,
    /// 8 bits a channel.
    pub(crate) color: [u8; 4],
}

/// The `drop-shadow()` as the cascade gave it (`vn_css::Extras::drop_shadow`):
/// its colour `None` is CSS's default, `currentColor`, resolved at paint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShadowSpec {
    pub(crate) dx: f32,
    pub(crate) dy: f32,
    pub(crate) blur: f32,
    pub(crate) color: Option<gpui::Hsla>,
}

impl Shadow {
    /// `shadow` with `currentColor` resolved to `color`.
    pub(crate) fn of(shadow: &ShadowSpec, color: Option<gpui::Hsla>) -> Shadow {
        Shadow {
            dx: shadow.dx,
            dy: shadow.dy,
            sigma: shadow.blur.max(0.0),
            color: rgba8(shadow.color.or(color).unwrap_or_else(gpui::black)),
        }
    }

    /// How far the shadow reaches past the box on each side, in whole CSS
    /// px: `[left, top, right, bottom]`. Three sigmas is where a Gaussian's
    /// tail is below one part in 300, invisible in 8 bits.
    pub(crate) fn margins(&self) -> [f32; 4] {
        let reach = (3.0 * self.sigma).ceil();
        let side = |offset: f32| (reach + offset).max(0.0).ceil();
        [side(-self.dx), side(-self.dy), side(self.dx), side(self.dy)]
    }

    fn key(&self) -> ([u32; 3], [u8; 4]) {
        ([self.dx.to_bits(), self.dy.to_bits(), self.sigma.to_bits()], self.color)
    }
}

/// A colour, 8 bits a channel.
fn rgba8(c: gpui::Hsla) -> [u8; 4] {
    let c = gpui::Rgba::from(c);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(c.r), q(c.g), q(c.b), q(c.a)]
}

/// The id of the wrapper's filter. A document's own ids cannot clash with it
/// in practice; the prefix is not one an author writes.
const SHADOW_ID: &str = "__vn_drop_shadow";

impl RasterKey {
    /// The same picture but for the colour: a recolour, which is throttled.
    fn recolour_of(&self, other: &RasterKey) -> bool {
        self.color != other.color && RasterKey { color: other.color, ..*self } == *other
    }
}

/// The key for `doc` in a box of `size` CSS pixels at `scale`, with `color` as
/// the element's CSS colour, and the box grown by `shadow`'s margins. `None`
/// for an empty box.
pub(crate) fn raster_key(
    doc: &SvgDoc,
    size: gpui::Size<Pixels>,
    scale: f32,
    color: Option<gpui::Hsla>,
    shadow: Option<&Shadow>,
) -> Option<RasterKey> {
    let (bw, bh) = (f32::from(size.width), f32::from(size.height));
    if !((bw * scale).round() >= 1.0 && (bh * scale).round() >= 1.0) {
        return None;
    }
    let [l, t, r, b] = shadow.map_or([0.0; 4], Shadow::margins);
    let w = ((bw + l + r) * scale).round();
    let h = ((bh + t + b) * scale).round();
    let color = doc.uses_current.then(|| rgba8(color.unwrap_or_else(gpui::black)));
    Some(RasterKey {
        source: doc.source,
        width: w as u32,
        height: h as u32,
        scale_milli: (scale * 1000.0).round() as u32,
        color,
        shadow: shadow.map(Shadow::key),
    })
}

/// The document as the colour raster reads it: the root's `width`/`height`
/// are the element's laid-out CSS size (so `viewBox` and
/// `preserveAspectRatio` fit the content into the box, as in a browser), and
/// `currentColor` resolves to `color` through the root's `color` property,
/// unless the root sets a `color` of its own.
///
/// With a `shadow`, that root becomes a nested `<svg>` at the shadow's
/// margins inside a wrapper the size of the grown raster, under a `<g>` whose
/// filter is the `feDropShadow` CSS's `drop-shadow()` is defined as. The
/// filter region is the whole wrapper, so nothing of the shadow is cut at
/// resvg's default 10% margin.
pub(crate) fn raster_source(
    doc: &SvgDoc,
    css_w: f32,
    css_h: f32,
    color: Option<[u8; 4]>,
    shadow: Option<&Shadow>,
) -> Vec<u8> {
    let inner = root_source(doc, css_w, css_h, color, shadow.map(|s| s.margins()));
    let Some(s) = shadow else { return inner };
    let Ok(inner) = String::from_utf8(inner) else { return doc.bytes.clone() };
    let [l, t, r, b] = s.margins();
    let (w, h) = (css_w + l + r, css_h + t + b);
    let [cr, cg, cb, ca] = s.color;
    format!(
        "<svg xmlns=\"{SVG_NS}\" width=\"{w}\" height=\"{h}\">\
         <filter id=\"{SHADOW_ID}\" filterUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" \
         color-interpolation-filters=\"sRGB\">\
         <feDropShadow dx=\"{dx}\" dy=\"{dy}\" stdDeviation=\"{sd}\" flood-color=\"rgb({cr},{cg},{cb})\" \
         flood-opacity=\"{fo}\"/></filter>\
         <g filter=\"url(#{SHADOW_ID})\">{inner}</g></svg>",
        dx = s.dx,
        dy = s.dy,
        sd = s.sigma,
        fo = ca as f32 / 255.0,
    )
    .into_bytes()
}

/// [`raster_source`]'s root: sized to the box, and placed at `offset`'s left
/// and top margins when it is nested in the shadow wrapper.
fn root_source(doc: &SvgDoc, css_w: f32, css_h: f32, color: Option<[u8; 4]>, offset: Option<[f32; 4]>) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(&doc.bytes) else {
        return doc.bytes.clone();
    };
    // The root's start tag ends at the first `>`: every attribute value was
    // written by `write_attr`, which escapes `>`.
    let Some(end) = text.find('>') else {
        return doc.bytes.clone();
    };
    let Some(mut rest) = text[..end].strip_prefix("<svg") else {
        return doc.bytes.clone();
    };
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str("<svg");
    let mut has_color = false;
    // ` name="value"`, exactly as `write_attr` wrote them.
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq].trim();
        let after = &rest[eq + 2..];
        let Some(close) = after.find('"') else { break };
        let value = &after[..close];
        rest = &after[close + 1..];
        // `x` / `y` do nothing on an outermost `<svg>`, and would move the
        // nested one the shadow wrapper makes of it.
        if matches!(name, "width" | "height" | "x" | "y") {
            continue;
        }
        has_color |= name == "color";
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(value);
        out.push('"');
    }
    if let Some([l, t, _, _]) = offset {
        out.push_str(&format!(" x=\"{l}\" y=\"{t}\""));
    }
    out.push_str(&format!(" width=\"{css_w}\" height=\"{css_h}\""));
    if let (Some([r, g, b, a]), false) = (color, has_color) {
        out.push_str(&format!(" color=\"rgba({r},{g},{b},{})\"", a as f32 / 255.0));
    }
    out.push_str(&text[end..]);
    out.into_bytes()
}

/// resvg, at exactly the key's device size, through gpui's renderer (its
/// fonts, its BGRA layout).
pub(crate) fn rasterize(renderer: &gpui::SvgRenderer, src: &[u8], key: &RasterKey) -> Option<Arc<RenderImage>> {
    let parsed = renderer.parse_svg(src).ok()?;
    renderer
        .render_parsed(
            &parsed,
            gpui::SvgSize::ExactSize(gpui::size(
                gpui::DevicePixels(key.width as i32),
                gpui::DevicePixels(key.height as i32),
            )),
        )
        .ok()
}

/// A colour change re-rasterises at most this often per node. Between two
/// rasters the previous one stays on screen and another frame is asked for,
/// so the colour the element settles on is always the one painted last. A
/// 150 ms `transition: color` therefore costs about four rasters, where one a
/// frame would be ten, and a colour animation that never stops costs twenty a
/// second, not one a frame.
pub(crate) const RECOLOUR_EVERY: Duration = Duration::from_millis(50);

/// One `<svg>` node's current colour raster.
struct NodeRaster {
    key: RasterKey,
    image: Arc<RenderImage>,
    at: Instant,
    /// The window whose atlas holds the tile (docs/MULTIWINDOW.md §5.3).
    win: WinKey,
}

thread_local! {
    static RASTERS: RefCell<HashMap<u32, NodeRaster>> = RefCell::new(HashMap::new());
}

/// The node paints through the mask path now: its raster, if any, goes.
pub(crate) fn forget_raster(id: u32) {
    if let Some(r) = RASTERS.with(|m| m.borrow_mut().remove(&id)) {
        windows::queue_drops(r.win, [r.image]);
    }
}

/// How many nodes hold a colour raster (tests, devtools).
#[allow(dead_code)]
pub(crate) fn raster_count() -> usize {
    RASTERS.with(|m| m.borrow().len())
}

/// `(parsed documents, colour rasters, their BGRA bytes)` — devtools `mem`.
pub(crate) fn mem_stats() -> (usize, usize, usize) {
    let docs = CACHE.with(|c| c.borrow().len());
    let (n, bytes) = RASTERS.with(|m| {
        let m = m.borrow();
        (m.len(), m.values().map(|r| r.image.as_bytes(0).map_or(0, <[u8]>::len)).sum())
    });
    (docs, n, bytes)
}

/// Paints `doc` in full colour into `bounds`. Runs inside the element's style
/// paint, so its transform, opacity and clip apply to the image.
fn paint_colour(
    id: u32,
    doc: &SvgDoc,
    bounds: Bounds<Pixels>,
    color: Option<gpui::Hsla>,
    shadow: Option<&ShadowSpec>,
    window: &mut Window,
    cx: &mut App,
) {
    let shadow = shadow.map(|s| Shadow::of(s, color));
    let Some(key) = raster_key(doc, bounds.size, window.scale_factor(), color, shadow.as_ref()) else {
        return;
    };
    // The raster covers the box and the shadow's reach past it.
    let [l, t, r, b] = shadow.as_ref().map_or([0.0; 4], Shadow::margins);
    let area = Bounds {
        origin: gpui::point(bounds.origin.x - gpui::px(l), bounds.origin.y - gpui::px(t)),
        size: gpui::size(bounds.size.width + gpui::px(l + r), bounds.size.height + gpui::px(t + b)),
    };
    let now = Instant::now();
    let held = RASTERS.with(|m| m.borrow().get(&id).map(|r| (r.key, r.image.clone(), r.at)));
    let image = match held {
        Some((k, image, _)) if k == key => image,
        Some((k, image, at)) if k.recolour_of(&key) && now.duration_since(at) < RECOLOUR_EVERY => {
            window.request_animation_frame();
            image
        }
        _ => {
            let src = raster_source(
                doc,
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                key.color,
                shadow.as_ref(),
            );
            let Some(image) = rasterize(&cx.svg_renderer(), &src, &key) else {
                crate::render::warn_compat(
                    "svg-raster",
                    "an inline <svg> could not be rasterised in full colour (resvg rejected the serialized \
                     document); it paints nothing",
                );
                return;
            };
            let win = windows::key(window);
            let old = RASTERS.with(|m| {
                m.borrow_mut().insert(id, NodeRaster { key, image: image.clone(), at: now, win })
            });
            if let Some(old) = old {
                windows::queue_drops(old.win, [old.image]);
            }
            image
        }
    };
    let _ = window.paint_image(area, area, gpui::Corners::default(), image, 0, false);
}

/// The colour path's element: `gpui::svg()`'s layout, hit-testing and style
/// painting, with the full-colour raster painted where `Svg` would paint its
/// mask (inside the style's paint, so `transform`, `opacity` and clipping
/// apply). The colour `currentColor` resolves to is the paint style's
/// `text.color`, which `render::SvgTint` fills in exactly as for the mask.
pub(crate) struct SvgRaster {
    interactivity: gpui::Interactivity,
    id: u32,
    doc: Rc<SvgDoc>,
    shadow: Option<ShadowSpec>,
}

pub(crate) fn raster(id: u32, doc: Rc<SvgDoc>, shadow: Option<ShadowSpec>) -> SvgRaster {
    SvgRaster { interactivity: gpui::Interactivity::new(), id, doc, shadow }
}

impl gpui::Element for SvgRaster {
    type RequestLayoutState = ();
    type PrepaintState = Option<gpui::Hitbox>;

    fn id(&self) -> Option<gpui::ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let layout_id = self.interactivity.request_layout(global_id, inspector_id, window, cx, |style, window, cx| {
            window.request_layout(style, None, cx)
        });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::Hitbox> {
        self.interactivity
            .prepaint(global_id, inspector_id, bounds, bounds.size, window, cx, |_, _, hitbox, _, _| hitbox)
    }

    fn paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        hitbox: &mut Option<gpui::Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (id, doc, shadow) = (self.id, self.doc.clone(), self.shadow);
        self.interactivity
            .paint(global_id, inspector_id, bounds, hitbox.as_ref(), window, cx, |style, window, cx| {
                paint_colour(id, &doc, bounds, style.text.color, shadow.as_ref(), window, cx);
            });
    }
}

impl gpui::IntoElement for SvgRaster {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Styled for SvgRaster {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl gpui::InteractiveElement for SvgRaster {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        &mut self.interactivity
    }
}

// ---------------------------------------------------------------------------
// the change key
// ---------------------------------------------------------------------------

fn hash_subtree(t: &Tree, id: u32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_node(t, id, true, &mut h);
    h.finish()
}

fn hash_node(t: &Tree, id: u32, root: bool, h: &mut impl Hasher) {
    let Some(n) = t.node(id) else { return };
    (n.kind as u8).hash(h);
    match n.kind {
        Kind::Anchor => return,
        Kind::Text => {
            n.text.hash(h);
            return;
        }
        Kind::Element => {}
    }
    t.name(n.tag).hash(h);
    n.text.hash(h);
    for (k, v) in &n.props.iter().map(|(k, v, _)| (*k, v)).collect::<Vec<_>>() {
        t.name(*k).hash(h);
        v.hash(h);
    }
    for (k, v) in &n.style {
        t.name(*k).hash(h);
        v.hash(h);
    }
    hash_cascaded(n, root, h);
    for &c in &n.children {
        hash_node(t, c, false, h);
    }
}

// ---------------------------------------------------------------------------
// serialization
// ---------------------------------------------------------------------------

/// SVG's own namespace. usvg refuses a document without it, and an inline
/// `<svg>` in HTML does not have to carry it — the HTML parser supplies it.
const SVG_NS: &str = "http://www.w3.org/2000/svg";

fn write_element(t: &Tree, id: u32, root: bool, parent: &Inherited, class: &mut Class, out: &mut String) {
    let Some(n) = t.node(id) else { return };
    match n.kind {
        // A fragment anchor is a comment in a browser: not rendered, and not
        // part of the SVG document either.
        Kind::Anchor => return,
        Kind::Text => {
            escape_text(&n.text, out);
            return;
        }
        Kind::Element => {}
    }
    let tag = t.name(n.tag);
    if !is_name(tag) {
        return;
    }
    let declared = cascaded(n, root);
    let here = classify(t, n, tag, root, &declared, parent, class);
    out.push('<');
    out.push_str(tag);

    let mut wrote_ns = false;
    for (k, v, _) in &n.props {
        let name = t.name(*k);
        if !is_name(name) {
            continue;
        }
        // Every author rule beats a presentation attribute, so what the
        // cascade computed replaces the attribute the markup carries.
        if overridden(&declared, name) {
            continue;
        }
        if name == "xmlns" {
            wrote_ns = true;
        }
        write_attr(name, v, out);
    }
    for (name, value) in &declared {
        write_attr(name, value, out);
    }
    if root && !wrote_ns {
        write_attr("xmlns", SVG_NS, out);
    }
    // Inline `style` travels on its own channel (`__vn_set_style`), not in
    // `props`, so it has to be rebuilt into the declaration body resvg reads.
    // A property the cascade carries is left out: its computed value, written
    // above, already has the inline declaration folded in (with `var()`
    // resolved, and an `!important` rule still beating it).
    if n.style.iter().any(|(k, _)| !overridden(&declared, t.name(*k))) {
        let mut body = String::new();
        for (k, v) in &n.style {
            let name = t.name(*k);
            if overridden(&declared, name) {
                continue;
            }
            if !body.is_empty() {
                body.push(';');
            }
            body.push_str(name);
            body.push(':');
            body.push_str(v);
        }
        write_attr("style", &body, out);
    }
    out.push('>');

    if !n.text.is_empty() {
        escape_text(&n.text, out);
    } else {
        for &c in &n.children {
            write_element(t, c, false, &here, class, out);
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn write_attr(name: &str, value: &str, out: &mut String) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

fn escape_text(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

/// A conservative XML Name test. Vue can write a prop under any string at all
/// (`onUpdate:modelValue` leaks onto elements), and one bad attribute name
/// makes the WHOLE document unparseable — one icon-shaped hole instead of an
/// icon — so anything questionable is dropped rather than risked.
///
/// A colon is an XML namespace prefix, and an UNDECLARED prefix is a parse
/// error, not a stray character: only the two prefixes SVG itself defines are
/// allowed through.
fn is_name(s: &str) -> bool {
    let local = match s.split_once(':') {
        None => s,
        Some((prefix @ ("xlink" | "xml"), local)) if !prefix.is_empty() => local,
        Some(_) => return false,
    };
    let mut chars = local.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::with_tree;

    fn serialize(t: &Tree, id: u32) -> String {
        String::from_utf8(build(t, id).bytes).unwrap()
    }

    /// What the cascade computed for one node: `f` fills its `Extras`, `g`
    /// its base refinement.
    fn computed(
        t: &mut Tree,
        id: u32,
        f: impl FnOnce(&mut vn_css::Extras),
        g: impl FnOnce(&mut gpui::StyleRefinement),
    ) {
        let mut cs = vn_css::ComputedStates::default();
        f(&mut cs.extras);
        g(Rc::make_mut(&mut cs.base));
        t.node_mut(id).unwrap().computed = Some(cs);
    }

    /// [`computed`] with only the paints.
    fn paints(t: &mut Tree, id: u32, fill: Option<vn_css::SvgPaint>, stroke: Option<vn_css::SvgPaint>) {
        computed(
            t,
            id,
            |x| {
                x.fill = fill;
                x.stroke = stroke;
            },
            |_| {},
        );
    }

    fn colour(hex: u32) -> vn_css::SvgPaint {
        vn_css::SvgPaint::Color(gpui::Hsla::from(gpui::rgb(hex)))
    }

    /// One element with attributes, inserted under `parent` (0 = none).
    fn el(t: &mut Tree, tag: &str, attrs: &[(&str, &str)], parent: u32) -> u32 {
        let tag = t.intern(tag);
        let id = t.create_element(tag);
        for (k, v) in attrs {
            let key = t.intern(k);
            t.set_prop(id, key, Some(v.to_string()));
        }
        if parent != 0 {
            t.insert(id, parent, 0);
        }
        id
    }

    fn doc_of(f: impl FnOnce(&mut Tree) -> u32) -> SvgDoc {
        with_tree(|t| {
            let id = f(t);
            build(t, id)
        })
    }

    const NS: (&str, &str) = ("xmlns", SVG_NS);

    #[test]
    fn currentcolor_icons_are_mono() {
        // lucide: a stroke in currentColor, no fill.
        let lucide = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "none"), ("stroke", "currentColor")], 0);
            el(t, "path", &[("d", "M20 6 9 17l-5-5")], s);
            el(t, "line", &[("x1", "0"), ("y1", "0"), ("x2", "4"), ("y2", "4")], s);
            s
        });
        assert_eq!(lucide.paint, Paint::Mono);
        // heroicons solid: the fill is currentColor on the root and inherited.
        let solid = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "currentColor")], 0);
            el(t, "path", &[("d", "M0 0h4v4z")], s);
            s
        });
        assert_eq!(solid.paint, Paint::Mono);
        assert!(solid.uses_current);
    }

    #[test]
    fn opacity_and_clipping_stay_mono_because_the_mask_carries_coverage() {
        // A duotone icon: one colour at two opacities is still `color × alpha`.
        let duotone = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "currentColor")], 0);
            el(t, "path", &[("d", "M0 0h4v4z"), ("opacity", "0.2")], s);
            el(t, "path", &[("d", "M4 4h4v4z"), ("fill-opacity", "0.5")], s);
            // A clip path's shapes are coverage, whatever they would paint.
            let clip = el(t, "clipPath", &[("id", "c")], s);
            el(t, "rect", &[("width", "4"), ("height", "4")], clip);
            s
        });
        assert_eq!(duotone.paint, Paint::Mono);
    }

    #[test]
    fn what_needs_full_colour() {
        let reason = |f: &dyn Fn(&mut Tree, u32)| {
            doc_of(|t| {
                let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "currentColor")], 0);
                f(t, s);
                s
            })
            .paint
        };
        // SVG's initial fill is black, which is a colour the mask cannot keep.
        let bare = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24")], 0);
            el(t, "path", &[("d", "M0 0h4v4z")], s);
            s
        });
        assert!(matches!(bare.paint, Paint::Colour(_)));
        assert!(matches!(reason(&|t, s| { el(t, "rect", &[("fill", "#3b82f6")], s); }), Paint::Colour(_)));
        assert_eq!(
            reason(&|t, s| {
                el(t, "rect", &[("fill", "#f00")], s);
                el(t, "circle", &[("stroke", "#00f"), ("fill", "none")], s);
            }),
            Paint::Colour("more than one fill/stroke colour")
        );
        assert_eq!(
            reason(&|t, s| {
                let g = el(t, "radialGradient", &[("id", "g")], s);
                el(t, "stop", &[("stop-color", "#000")], g);
                el(t, "rect", &[("fill", "url(#g)")], s);
            }),
            Paint::Colour("a gradient or pattern")
        );
        assert_eq!(reason(&|t, s| { el(t, "image", &[("href", "a.png")], s); }), Paint::Colour("an <image>"));
        assert_eq!(reason(&|t, s| { el(t, "rect", &[("filter", "url(#f)")], s); }), Paint::Colour("a filter"));
        assert_eq!(reason(&|t, s| { el(t, "rect", &[("mix-blend-mode", "multiply")], s); }), Paint::Colour("a blend mode"));
        // currentColor under a `color` set INSIDE the document is that colour.
        assert!(matches!(
            reason(&|t, s| {
                let g = el(t, "g", &[("color", "red")], s);
                el(t, "rect", &[], g);
            }),
            Paint::Colour(_)
        ));
    }

    #[test]
    fn a_root_with_no_viewbox_is_drawn_in_its_box_s_own_pixels() {
        // Argon's broadcast connector: CSS-sized, a currentColor stroke, no
        // viewBox, path coordinates in px of the list it is laid over.
        let rope = with_tree(|t| {
            let s = el(t, "svg", &[("class", "broadcast-connectors")], 0);
            el(t, "path", &[("d", "M3.5 106.5 V169.5 M3.5 169.5 h4")], s);
            paints(t, s, Some(vn_css::SvgPaint::None), Some(vn_css::SvgPaint::Current));
            build(t, s)
        });
        assert_eq!(rope.paint, Paint::Colour(NO_VIEWBOX));
        // …and the raster writes the box's size as its width and height, so
        // usvg's user unit is one CSS pixel.
        let src = String::from_utf8(raster_source(&rope, 240.0, 372.0, Some([0, 0, 0, 255]), None)).unwrap();
        assert!(src.contains(r#"width="240" height="372""#), "{src}");
        assert!(!src.contains("viewBox"), "{src}");
        // Its CSS `stroke-opacity` / `stroke-width` reach the root, where the
        // cascade's value replaces the markup's attribute of the same name.
        let faint: Vec<(&'static str, Rc<str>)> = vec![("stroke-opacity", "0.28".into()), ("stroke-width", "1".into())];
        let doc = with_tree(|t| {
            let s = el(t, "svg", &[("stroke-width", "2")], 0);
            el(t, "path", &[("d", "M3.5 106.5 V169.5")], s);
            computed(
                t,
                s,
                |x| {
                    x.stroke = Some(vn_css::SvgPaint::Current);
                    x.svg_presentation = Some(Rc::new(faint));
                },
                |_| {},
            );
            serialize(t, s)
        });
        assert!(doc.contains(r#"stroke-opacity="0.28""#), "{doc}");
        assert!(doc.contains(r#"stroke-width="1""#) && !doc.contains(r#"stroke-width="2""#), "{doc}");
        // The same icon WITH a viewBox stays on the mask.
        let icon = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "none"), ("stroke", "currentColor")], 0);
            el(t, "path", &[("d", "M3 3h4")], s);
            s
        });
        assert_eq!(icon.paint, Paint::Mono);
    }

    #[test]
    fn an_inline_style_beats_the_presentation_attribute_in_the_classifier() {
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24")], 0);
            let p = el(t, "path", &[("d", "M0 0h4v4z"), ("fill", "red")], s);
            let fill = t.intern("fill");
            t.set_style(p, fill, "currentColor".into());
            s
        });
        assert_eq!(doc.paint, Paint::Mono);
    }

    #[test]
    fn a_paint_server_from_a_rule_is_drawn_in_currentcolor_and_said() {
        let doc = with_tree(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "#f00")], 0);
            el(t, "path", &[("d", "M0 0h4v4z")], s);
            paints(t, s, Some(vn_css::SvgPaint::Server), None);
            build(t, s)
        });
        assert_eq!(doc.paint, Paint::Mono);
        assert!(doc.css_paint_server);
    }

    /// The EXOBIO overlay's notch: `.notch-outline { fill: var(--notch-surface);
    /// stroke: rgba(219, 232, 255, 0.17); stroke-width: 1 }` on the `<svg>`,
    /// one `<path>` inside with no attributes but `d`.
    fn notch(stroke: vn_css::SvgPaint) -> SvgDoc {
        with_tree(|t| {
            let s = el(t, "svg", &[("width", "40"), ("height", "20")], 0);
            el(t, "path", &[("d", "M0 0 H40 V20 H0 Z")], s);
            computed(
                t,
                s,
                |x| {
                    x.fill = Some(colour(0x06080b));
                    x.stroke = Some(stroke);
                    x.svg_presentation = Some(Rc::new(vec![("stroke-width", "1px".into())]));
                },
                |_| {},
            );
            build(t, s)
        })
    }

    #[test]
    fn a_concrete_paint_from_a_rule_is_drawn_as_computed() {
        let faint = vn_css::SvgPaint::Color(gpui::Hsla::from(gpui::Rgba { r: 219.0 / 255.0, g: 232.0 / 255.0, b: 1.0, a: 0.17 }));
        let doc = notch(faint);
        let src = String::from_utf8(doc.bytes.clone()).unwrap();
        assert!(src.contains(r#"fill="rgb(6,8,11)""#), "{src}");
        assert!(src.contains(r#"stroke="rgba(219,232,255,0.17)""#), "{src}");
        assert!(src.contains(r#"stroke-width="1px""#), "{src}");
        assert!(!doc.css_paint_server);
        assert!(matches!(doc.paint, Paint::Colour(_)));
        // resvg draws it: the inside is the fill, not the element's colour.
        let white = Some(gpui::Hsla::from(gpui::rgb(0xf4f4f6)));
        let image = render(&doc, (40.0, 20.0), 1.0, white);
        assert!(close(pixel(&image, 20, 10), [6, 8, 11, 255]), "{:?}", pixel(&image, 20, 10));
        // A class that swaps the stroke is another document.
        let other = notch(colour(0x7c99f4));
        assert_ne!(doc.source, other.source);
    }

    #[test]
    fn rules_on_shapes_inside_win_over_attributes_and_lose_to_inline_style() {
        let src = with_tree(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 30 10")], 0);
            // a <g> with a rule fill: the shapes under it inherit it…
            let g = el(t, "g", &[("fill", "#00f")], s);
            let a = el(t, "rect", &[("x", "0"), ("width", "10"), ("height", "10")], g);
            // …a shape whose ATTRIBUTE says otherwise loses to its own rule…
            let b = el(t, "rect", &[("x", "10"), ("width", "10"), ("height", "10"), ("fill", "#000")], g);
            // …and an inline style beats the rule (the cascade folded it in).
            let c = el(t, "rect", &[("x", "20"), ("width", "10"), ("height", "10")], g);
            paints(t, g, Some(colour(0xff0000)), None);
            paints(t, b, Some(colour(0x00ff00)), None);
            let fill = t.intern("fill");
            t.set_style(c, fill, "var(--x)".into());
            paints(t, c, Some(colour(0x0000ff)), None);
            // Inside the document `opacity` and `display: none` are the
            // shape's own.
            computed(t, a, |_| {}, |r| r.opacity = Some(0.5));
            let hidden = el(t, "circle", &[("r", "3")], s);
            computed(t, hidden, |_| {}, |r| r.display = Some(gpui::Display::None));
            serialize(t, s)
        });
        assert!(src.contains(r##"<g fill="rgb(255,0,0)">"##), "{src}");
        assert!(src.contains(r#"opacity="0.5""#), "{src}");
        assert!(src.contains(r#"x="10" width="10" height="10" fill="rgb(0,255,0)""#), "{src}");
        assert!(!src.contains("#000") && !src.contains("#00f"), "{src}");
        // The raw `var()` never reaches resvg: the computed value replaced it.
        assert!(src.contains(r#"fill="rgb(0,0,255)""#) && !src.contains("var("), "{src}");
        assert!(src.contains(r#"display="none""#), "{src}");
        assert!(usvg::Tree::from_data(src.as_bytes(), &usvg::Options::default()).is_ok(), "{src}");
    }

    #[test]
    fn a_drop_shadow_follows_the_shape_and_grows_the_raster() {
        // A 20x20 box with a 10x10 square in its top-left quarter, and
        // `drop-shadow(10px 10px 0 #ff0000)`: a hard copy of the square,
        // moved into the bottom-right quarter. A box shadow would fill the
        // whole box's offset copy instead.
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 20 20")], 0);
            el(t, "rect", &[("width", "10"), ("height", "10"), ("fill", "#0000ff")], s);
            s
        });
        let shadow = Shadow { dx: 10.0, dy: 10.0, sigma: 0.0, color: [255, 0, 0, 255] };
        assert_eq!(shadow.margins(), [0.0, 0.0, 10.0, 10.0]);
        let key = raster_key(&doc, gpui::size(gpui::px(20.0), gpui::px(20.0)), 1.0, None, Some(&shadow)).unwrap();
        assert_eq!((key.width, key.height), (30, 30));
        let src = raster_source(&doc, 20.0, 20.0, key.color, Some(&shadow));
        let image = rasterize(&gpui::SvgRenderer::new(Arc::new(())), &src, &key).expect("resvg renders it");
        assert_eq!(pixel(&image, 5, 5), [0, 0, 255, 255], "the shape");
        assert_eq!(pixel(&image, 15, 15), [255, 0, 0, 255], "its shadow");
        assert_eq!(pixel(&image, 15, 5)[3], 0, "beside the shape, no shadow");
        assert_eq!(pixel(&image, 25, 25)[3], 0, "the box's shadow corner stays empty");
        // A blurred one reaches three sigmas past the box on every side.
        let soft = Shadow { dx: 0.0, dy: 18.0, sigma: 30.0, color: [0, 0, 0, 115] };
        assert_eq!(soft.margins(), [90.0, 72.0, 90.0, 108.0]);
    }

    fn sample_doc(uses_current: bool) -> SvgDoc {
        SvgDoc { bytes: Vec::new(), source: 7, paint: Paint::Colour("t"), uses_current, css_paint_server: false }
    }

    #[test]
    fn the_raster_key_is_source_device_size_scale_and_the_resolved_colour() {
        let size = gpui::size(gpui::px(24.0), gpui::px(24.0));
        let blue = Some(gpui::Hsla::from(gpui::rgb(0x3b82f6)));
        let red = Some(gpui::Hsla::from(gpui::rgb(0xff0000)));
        let doc = sample_doc(true);
        let a = raster_key(&doc, size, 1.25, blue, None).unwrap();
        assert_eq!((a.width, a.height, a.scale_milli), (30, 30, 1250));
        assert!(close(a.color.unwrap(), [0x3b, 0x82, 0xf6, 255]), "{:?}", a.color);
        assert_eq!(raster_key(&doc, size, 1.25, blue, None), Some(a));
        // Another colour is a recolour of the same picture…
        let b = raster_key(&doc, size, 1.25, red, None).unwrap();
        assert!(a.recolour_of(&b));
        // …another size or scale is another picture, not a recolour.
        let c = raster_key(&doc, gpui::size(gpui::px(48.0), gpui::px(24.0)), 1.25, red, None).unwrap();
        assert!(!a.recolour_of(&c) && a != c);
        let d = raster_key(&doc, size, 1.5, blue, None).unwrap();
        assert_eq!(d.width, 36);
        assert!(!a.recolour_of(&d) && a != d);
        // A document that never says currentColor does not depend on colour.
        let fixed = sample_doc(false);
        assert_eq!(raster_key(&fixed, size, 1.0, blue, None), raster_key(&fixed, size, 1.0, red, None));
        // An empty box has no raster.
        assert_eq!(raster_key(&doc, gpui::size(gpui::px(0.0), gpui::px(24.0)), 1.0, blue, None), None);
    }

    #[test]
    fn the_raster_source_takes_the_box_as_its_viewport_and_the_colour_as_currentcolor() {
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("width", "24"), ("height", "24"), ("viewBox", "0 0 24 24"), ("fill", "currentColor")], 0);
            el(t, "path", &[("d", "M0 0h4v4z")], s);
            s
        });
        let src = String::from_utf8(raster_source(&doc, 48.0, 32.0, Some([1, 2, 3, 255]), None)).unwrap();
        assert!(src.contains(r#"width="48" height="32""#), "{src}");
        assert!(!src.contains(r#"width="24""#), "{src}");
        assert!(src.contains(r#"color="rgba(1,2,3,1)""#), "{src}");
        assert!(src.ends_with("</svg>"), "{src}");
        let tree = usvg::Tree::from_data(src.as_bytes(), &usvg::Options::default()).expect("usvg parses it");
        assert_eq!((tree.size().width(), tree.size().height()), (48.0, 32.0));
        // A root that sets its own colour keeps it.
        let own = doc_of(|t| el(t, "svg", &[NS, ("color", "red")], 0));
        let src = String::from_utf8(raster_source(&own, 10.0, 10.0, Some([1, 2, 3, 255]), None)).unwrap();
        assert!(!src.contains("rgba"), "{src}");
    }

    /// Equal within one step a channel (an HSL round trip may move one).
    fn close(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
    }

    /// The pixel at (x, y) of a BGRA raster, as RGBA.
    fn pixel(image: &RenderImage, x: usize, y: usize) -> [u8; 4] {
        let w = image.size(0).width.0 as usize;
        let b = image.as_bytes(0).unwrap();
        let i = (y * w + x) * 4;
        [b[i + 2], b[i + 1], b[i], b[i + 3]]
    }

    fn render(doc: &SvgDoc, css: (f32, f32), scale: f32, color: Option<gpui::Hsla>) -> Arc<RenderImage> {
        let key = raster_key(doc, gpui::size(gpui::px(css.0), gpui::px(css.1)), scale, color, None).unwrap();
        let src = raster_source(doc, css.0, css.1, key.color, None);
        let renderer = gpui::SvgRenderer::new(Arc::new(()));
        rasterize(&renderer, &src, &key).expect("resvg renders it")
    }

    #[test]
    fn a_two_colour_svg_rasterises_with_both_colours_at_the_device_size() {
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 2 1")], 0);
            el(t, "rect", &[("x", "0"), ("width", "1"), ("height", "1"), ("fill", "#ff0000")], s);
            el(t, "rect", &[("x", "1"), ("width", "1"), ("height", "1"), ("fill", "#0000ff")], s);
            s
        });
        assert!(matches!(doc.paint, Paint::Colour(_)));
        // 20×10 CSS px at 150%: 30×15 device pixels, crisp.
        let image = render(&doc, (20.0, 10.0), 1.5, None);
        assert_eq!((image.size(0).width.0, image.size(0).height.0), (30, 15));
        assert_eq!(pixel(&image, 7, 7), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 22, 7), [0, 0, 255, 255]);
    }

    #[test]
    fn currentcolor_in_a_colour_raster_is_the_elements_color() {
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 2 1")], 0);
            el(t, "rect", &[("x", "0"), ("width", "1"), ("height", "1"), ("fill", "currentColor")], s);
            el(t, "rect", &[("x", "1"), ("width", "1"), ("height", "1"), ("fill", "#ff0000")], s);
            s
        });
        let green = Some(gpui::Hsla::from(gpui::rgb(0x00ff00)));
        let image = render(&doc, (20.0, 10.0), 1.0, green);
        assert!(close(pixel(&image, 5, 5), [0, 255, 0, 255]), "{:?}", pixel(&image, 5, 5));
        assert_eq!(pixel(&image, 15, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn preserve_aspect_ratio_fits_the_viewbox_into_the_box() {
        // A square picture in a 2:1 box: `xMidYMid meet` centres it.
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 10 10"), ("width", "10"), ("height", "10")], 0);
            el(t, "rect", &[("width", "10"), ("height", "10"), ("fill", "#ff0000")], s);
            s
        });
        let image = render(&doc, (20.0, 10.0), 1.0, None);
        assert_eq!(pixel(&image, 2, 5)[3], 0, "left of the centred square is empty");
        assert_eq!(pixel(&image, 10, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 17, 5)[3], 0, "right of it too");
        // `none` stretches it instead.
        let stretched = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 10 10"), ("preserveAspectRatio", "none")], 0);
            el(t, "rect", &[("width", "10"), ("height", "10"), ("fill", "#ff0000")], s);
            s
        });
        let image = render(&stretched, (20.0, 10.0), 1.0, None);
        assert_eq!(pixel(&image, 2, 5), [255, 0, 0, 255]);
    }

    /// Builds `<svg …><path d="…"/></svg>` and returns the serialized document.
    fn lucide_check() -> (u32, String) {
        with_tree(|t| {
            let svg_tag = t.intern("svg");
            let path_tag = t.intern("path");
            let svg = t.create_element(svg_tag);
            for (k, v) in [
                ("xmlns", SVG_NS),
                ("viewBox", "0 0 24 24"),
                ("fill", "none"),
                ("stroke", "currentColor"),
                ("stroke-width", "2"),
            ] {
                let key = t.intern(k);
                t.set_prop(svg, key, Some(v.to_string()));
            }
            let path = t.create_element(path_tag);
            let d = t.intern("d");
            t.set_prop(path, d, Some("M20 6 9 17l-5-5".to_string()));
            t.insert(path, svg, 0);
            let doc = serialize(t, svg);
            (svg, doc)
        })
    }

    #[test]
    fn serializes_the_subtree_as_one_document() {
        let (_, doc) = lucide_check();
        assert!(doc.starts_with("<svg "), "{doc}");
        assert!(doc.contains(r#"viewBox="0 0 24 24""#), "{doc}");
        assert!(doc.contains(r#"stroke="currentColor""#), "{doc}");
        assert!(doc.contains(r#"<path d="M20 6 9 17l-5-5">"#), "{doc}");
        assert!(doc.ends_with("</svg>"), "{doc}");
    }

    #[test]
    fn resvg_parses_what_we_write() {
        let (_, doc) = lucide_check();
        // The real acceptance: the bytes we hand gpui are a document usvg
        // accepts. A silently-unparseable document paints nothing at all.
        let tree = usvg::Tree::from_data(doc.as_bytes(), &usvg::Options::default());
        assert!(tree.is_ok(), "usvg rejected {doc}: {:?}", tree.err());
        let tree = tree.unwrap();
        assert_eq!(tree.size().width(), 24.0);
        assert_eq!(tree.size().height(), 24.0);
    }

    #[test]
    fn the_cascade_overrides_a_presentation_attribute() {
        let (svg, _) = lucide_check();
        let doc = with_tree(|t| {
            paints(t, svg, Some(vn_css::SvgPaint::Current), None);
            serialize(t, svg)
        });
        // `fill="none"` in the markup, `fill: currentColor` from the sheet
        // (Tailwind's `fill-current`) — CSS wins, which is what makes the
        // shadcn radio dot visible.
        assert!(doc.contains(r#"fill="currentColor""#), "{doc}");
        assert!(!doc.contains(r#"fill="none""#), "{doc}");
        assert!(usvg::Tree::from_data(doc.as_bytes(), &usvg::Options::default()).is_ok());
    }

    #[test]
    fn re_serializes_only_when_the_subtree_moves() {
        let (svg, _) = lucide_check();
        let a = with_tree(|t| document(t, svg));
        let b = with_tree(|t| document(t, svg));
        assert!(Rc::ptr_eq(&a, &b), "an unchanged subtree must not re-serialize");
        with_tree(|t| {
            let d = t.intern("d");
            let child = t.node(svg).unwrap().children[0];
            t.set_prop(child, d, Some("M0 0 1 1".to_string()));
        });
        let c = with_tree(|t| document(t, svg));
        assert!(!Rc::ptr_eq(&a, &c), "a changed `d` must re-serialize");
        // …and so must a changed computed paint, on the root or inside.
        let d = with_tree(|t| {
            paints(t, svg, Some(vn_css::SvgPaint::None), None);
            document(t, svg)
        });
        assert!(!Rc::ptr_eq(&c, &d));
        let e = with_tree(|t| {
            let child = t.node(svg).unwrap().children[0];
            paints(t, child, Some(colour(0x123456)), None);
            document(t, svg)
        });
        assert!(!Rc::ptr_eq(&d, &e));
    }

    #[test]
    fn a_malformed_attribute_name_cannot_break_the_document() {
        let doc = with_tree(|t| {
            let svg_tag = t.intern("svg");
            let svg = t.create_element(svg_tag);
            let bad = t.intern("onUpdate:modelValue");
            t.set_prop(svg, bad, Some("x".to_string()));
            let worse = t.intern("2bad");
            t.set_prop(svg, worse, Some("y".to_string()));
            let vb = t.intern("viewBox");
            t.set_prop(svg, vb, Some("0 0 10 10".to_string()));
            serialize(t, svg)
        });
        assert!(!doc.contains("2bad"), "{doc}");
        assert!(usvg::Tree::from_data(doc.as_bytes(), &usvg::Options::default()).is_ok(), "{doc}");
    }
}
