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
//!   CSS-sized `<svg>`.
//!
//! `fill` / `stroke` from the cascade (Tailwind's `fill-current`, `fill-none`)
//! reach the document as "painted or not": `currentColor` or `none`. That is
//! what makes the shadcn radio dot, a `<circle class="fill-current">`, visible
//! at all. A CONCRETE colour from a CSS rule is carried by vn-css only as
//! "painted" (`SvgPaint::Color`), so it is drawn in `color`; the advisory
//! `svg-css-paint-colour` says so. A colour written as an attribute or an
//! inline style on the shapes is drawn as written.
//!
//! ## Re-serialization
//!
//! Keyed on a hash of everything that can change the bytes: every tag, every
//! attribute name and value, every inline-style declaration, every text node in
//! the subtree, plus the two computed paints. The serialization behind it runs
//! only when the hash moves. The colour is deliberately not in the key (see
//! above).

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
    /// The cascade gave the root a concrete `fill`/`stroke` colour, which
    /// vn-css carries only as "painted" and which is therefore drawn in
    /// `currentColor`.
    pub(crate) css_paint_colour: bool,
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
/// the computed paints moved since the last call.
pub(crate) fn document(
    t: &Tree,
    id: u32,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
) -> Rc<SvgDoc> {
    let key = hash_subtree(t, id, fill, stroke);
    CACHE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((cached, doc)) = m.get(&id) {
            if *cached == key {
                return doc.clone();
            }
        }
        let doc = Rc::new(build(t, id, fill, stroke));
        m.insert(id, (key, doc.clone()));
        doc
    })
}

fn build(
    t: &Tree,
    id: u32,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
) -> SvgDoc {
    let mut out = String::with_capacity(256);
    let mut class = Class::default();
    write_element(t, id, true, fill, stroke, &Inherited::root(), &mut class, &mut out);
    let css_paint_colour = fill == Some(vn_css::SvgPaint::Color) || stroke == Some(vn_css::SvgPaint::Color);
    let uses_current = out.to_ascii_lowercase().contains("currentcolor");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    out.hash(&mut h);
    SvgDoc {
        source: h.finish(),
        paint: class.finish(),
        uses_current,
        css_paint_colour,
        bytes: out.into_bytes(),
    }
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

/// Folds one element's own presentation attributes and inline declarations
/// (which win, as CSS beats a presentation attribute) into its context, and
/// records what a shape paints.
fn classify(
    t: &Tree,
    n: &crate::tree::Node,
    tag: &str,
    root: bool,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
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
    // The cascade's paint replaces the root's attribute (`write_element` does
    // the same to the bytes): "painted or not" is all it carries.
    if root {
        let computed = |p: vn_css::SvgPaint| match p {
            vn_css::SvgPaint::None => Spec::Is(Eff::None),
            _ => Spec::Is(Eff::Current),
        };
        if let Some(p) = fill {
            own_fill = computed(p);
        }
        if let Some(p) = stroke {
            own_stroke = computed(p);
        }
    }
    for (k, v) in &n.style {
        look(t.name(*k), v, &mut own_fill, &mut own_stroke, class);
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
}

impl RasterKey {
    /// The same picture but for the colour: a recolour, which is throttled.
    fn recolour_of(&self, other: &RasterKey) -> bool {
        self.color != other.color && RasterKey { color: other.color, ..*self } == *other
    }
}

/// The key for `doc` in a box of `size` CSS pixels at `scale`, with `color` as
/// the element's CSS colour. `None` for an empty box.
pub(crate) fn raster_key(
    doc: &SvgDoc,
    size: gpui::Size<Pixels>,
    scale: f32,
    color: Option<gpui::Hsla>,
) -> Option<RasterKey> {
    let w = (f32::from(size.width) * scale).round();
    let h = (f32::from(size.height) * scale).round();
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let color = if doc.uses_current {
        let c = gpui::Rgba::from(color.unwrap_or_else(gpui::black));
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some([q(c.r), q(c.g), q(c.b), q(c.a)])
    } else {
        None
    };
    Some(RasterKey { source: doc.source, width: w as u32, height: h as u32, scale_milli: (scale * 1000.0).round() as u32, color })
}

/// The document as the colour raster reads it: the root's `width`/`height`
/// are the element's laid-out CSS size (so `viewBox` and
/// `preserveAspectRatio` fit the content into the box, as in a browser), and
/// `currentColor` resolves to `color` through the root's `color` property,
/// unless the root sets a `color` of its own.
pub(crate) fn raster_source(doc: &SvgDoc, css_w: f32, css_h: f32, color: Option<[u8; 4]>) -> Vec<u8> {
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
        if name == "width" || name == "height" {
            continue;
        }
        has_color |= name == "color";
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(value);
        out.push('"');
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

/// Paints `doc` in full colour into `bounds`. Runs inside the element's style
/// paint, so its transform, opacity and clip apply to the image.
fn paint_colour(
    id: u32,
    doc: &SvgDoc,
    bounds: Bounds<Pixels>,
    color: Option<gpui::Hsla>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(key) = raster_key(doc, bounds.size, window.scale_factor(), color) else {
        return;
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
            let src = raster_source(doc, f32::from(bounds.size.width), f32::from(bounds.size.height), key.color);
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
    let _ = window.paint_image(bounds, bounds, gpui::Corners::default(), image, 0, false);
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
}

pub(crate) fn raster(id: u32, doc: Rc<SvgDoc>) -> SvgRaster {
    SvgRaster { interactivity: gpui::Interactivity::new(), id, doc }
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
        let (id, doc) = (self.id, self.doc.clone());
        self.interactivity
            .paint(global_id, inspector_id, bounds, hitbox.as_ref(), window, cx, |style, window, cx| {
                paint_colour(id, &doc, bounds, style.text.color, window, cx);
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

fn hash_subtree(
    t: &Tree,
    id: u32,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fill.map(|p| p as u8).hash(&mut h);
    stroke.map(|p| p as u8).hash(&mut h);
    hash_node(t, id, &mut h);
    h.finish()
}

fn hash_node(t: &Tree, id: u32, h: &mut impl Hasher) {
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
    for &c in &n.children {
        hash_node(t, c, h);
    }
}

// ---------------------------------------------------------------------------
// serialization
// ---------------------------------------------------------------------------

/// SVG's own namespace. usvg refuses a document without it, and an inline
/// `<svg>` in HTML does not have to carry it — the HTML parser supplies it.
const SVG_NS: &str = "http://www.w3.org/2000/svg";

#[allow(clippy::too_many_arguments)]
fn write_element(
    t: &Tree,
    id: u32,
    root: bool,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
    parent: &Inherited,
    class: &mut Class,
    out: &mut String,
) {
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
    let here = classify(t, n, tag, root, fill, stroke, parent, class);
    out.push('<');
    out.push_str(tag);

    let mut wrote_ns = false;
    for (k, v, _) in &n.props {
        let name = t.name(*k);
        if !is_name(name) {
            continue;
        }
        // The cascade beats a presentation attribute (CSS always does), so a
        // `fill` the cascade computed replaces the one the markup carries.
        if root && name.eq_ignore_ascii_case("fill") && fill.is_some() {
            continue;
        }
        if root && name.eq_ignore_ascii_case("stroke") && stroke.is_some() {
            continue;
        }
        if name == "xmlns" {
            wrote_ns = true;
        }
        write_attr(name, v, out);
    }
    if root {
        if let Some(p) = fill {
            write_attr("fill", p.as_attr(), out);
        }
        if let Some(p) = stroke {
            write_attr("stroke", p.as_attr(), out);
        }
        if !wrote_ns {
            write_attr("xmlns", SVG_NS, out);
        }
    }
    // Inline `style` travels on its own channel (`__vn_set_style`), not in
    // `props`, so it has to be rebuilt into the declaration body resvg reads.
    if !n.style.is_empty() {
        let mut body = String::new();
        for (k, v) in &n.style {
            let name = t.name(*k);
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
            write_element(t, c, false, None, None, &here, class, out);
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

    fn serialize(
        t: &Tree,
        id: u32,
        fill: Option<vn_css::SvgPaint>,
        stroke: Option<vn_css::SvgPaint>,
    ) -> String {
        String::from_utf8(build(t, id, fill, stroke).bytes).unwrap()
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
            build(t, id, None, None)
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
    fn the_cascade_paint_on_the_root_is_painted_or_not() {
        let doc = with_tree(|t| {
            let s = el(t, "svg", &[NS, ("viewBox", "0 0 24 24"), ("fill", "#f00")], 0);
            el(t, "path", &[("d", "M0 0h4v4z")], s);
            build(t, s, Some(vn_css::SvgPaint::Color), None)
        });
        // vn-css carries only "painted", so it is written as currentColor —
        // mono — and the renderer says that the colour itself was lost.
        assert_eq!(doc.paint, Paint::Mono);
        assert!(doc.css_paint_colour);
    }

    fn sample_doc(uses_current: bool) -> SvgDoc {
        SvgDoc { bytes: Vec::new(), source: 7, paint: Paint::Colour("t"), uses_current, css_paint_colour: false }
    }

    #[test]
    fn the_raster_key_is_source_device_size_scale_and_the_resolved_colour() {
        let size = gpui::size(gpui::px(24.0), gpui::px(24.0));
        let blue = Some(gpui::Hsla::from(gpui::rgb(0x3b82f6)));
        let red = Some(gpui::Hsla::from(gpui::rgb(0xff0000)));
        let doc = sample_doc(true);
        let a = raster_key(&doc, size, 1.25, blue).unwrap();
        assert_eq!((a.width, a.height, a.scale_milli), (30, 30, 1250));
        assert!(close(a.color.unwrap(), [0x3b, 0x82, 0xf6, 255]), "{:?}", a.color);
        assert_eq!(raster_key(&doc, size, 1.25, blue), Some(a));
        // Another colour is a recolour of the same picture…
        let b = raster_key(&doc, size, 1.25, red).unwrap();
        assert!(a.recolour_of(&b));
        // …another size or scale is another picture, not a recolour.
        let c = raster_key(&doc, gpui::size(gpui::px(48.0), gpui::px(24.0)), 1.25, red).unwrap();
        assert!(!a.recolour_of(&c) && a != c);
        let d = raster_key(&doc, size, 1.5, blue).unwrap();
        assert_eq!(d.width, 36);
        assert!(!a.recolour_of(&d) && a != d);
        // A document that never says currentColor does not depend on colour.
        let fixed = sample_doc(false);
        assert_eq!(raster_key(&fixed, size, 1.0, blue), raster_key(&fixed, size, 1.0, red));
        // An empty box has no raster.
        assert_eq!(raster_key(&doc, gpui::size(gpui::px(0.0), gpui::px(24.0)), 1.0, blue), None);
    }

    #[test]
    fn the_raster_source_takes_the_box_as_its_viewport_and_the_colour_as_currentcolor() {
        let doc = doc_of(|t| {
            let s = el(t, "svg", &[NS, ("width", "24"), ("height", "24"), ("viewBox", "0 0 24 24"), ("fill", "currentColor")], 0);
            el(t, "path", &[("d", "M0 0h4v4z")], s);
            s
        });
        let src = String::from_utf8(raster_source(&doc, 48.0, 32.0, Some([1, 2, 3, 255]))).unwrap();
        assert!(src.contains(r#"width="48" height="32""#), "{src}");
        assert!(!src.contains(r#"width="24""#), "{src}");
        assert!(src.contains(r#"color="rgba(1,2,3,1)""#), "{src}");
        assert!(src.ends_with("</svg>"), "{src}");
        let tree = usvg::Tree::from_data(src.as_bytes(), &usvg::Options::default()).expect("usvg parses it");
        assert_eq!((tree.size().width(), tree.size().height()), (48.0, 32.0));
        // A root that sets its own colour keeps it.
        let own = doc_of(|t| el(t, "svg", &[NS, ("color", "red")], 0));
        let src = String::from_utf8(raster_source(&own, 10.0, 10.0, Some([1, 2, 3, 255]))).unwrap();
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
        let key = raster_key(doc, gpui::size(gpui::px(css.0), gpui::px(css.1)), scale, color).unwrap();
        let src = raster_source(doc, css.0, css.1, key.color);
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
            let doc = serialize(t, svg, None, None);
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
        let doc = with_tree(|t| serialize(t, svg, Some(vn_css::SvgPaint::Current), None));
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
        let a = with_tree(|t| document(t, svg, None, None));
        let b = with_tree(|t| document(t, svg, None, None));
        assert!(Rc::ptr_eq(&a, &b), "an unchanged subtree must not re-serialize");
        with_tree(|t| {
            let d = t.intern("d");
            let child = t.node(svg).unwrap().children[0];
            t.set_prop(child, d, Some("M0 0 1 1".to_string()));
        });
        let c = with_tree(|t| document(t, svg, None, None));
        assert!(!Rc::ptr_eq(&a, &c), "a changed `d` must re-serialize");
        // …and so must a changed computed paint, which is not in the subtree.
        let d = with_tree(|t| document(t, svg, Some(vn_css::SvgPaint::None), None));
        assert!(!Rc::ptr_eq(&c, &d));
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
            serialize(t, svg, None, None)
        });
        assert!(!doc.contains("2bad"), "{doc}");
        assert!(usvg::Tree::from_data(doc.as_bytes(), &usvg::Options::default()).is_ok(), "{doc}");
    }
}
