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
//!   whatever rectangle gpui ends up laying the leaf out at, at 2× for
//!   smoothing, so the icon is crisp at every size and every scale factor.
//! * **colour** — gpui renders an SVG as a single-colour **alpha mask** tinted
//!   by `style.text.color` (`MonochromeSprite`). That is a perfect fit for the
//!   `stroke="currentColor"` icon sets everybody actually ships, and it means
//!   the serialized bytes are colour-INDEPENDENT: a `:hover` that changes
//!   `color` re-tints an already-rasterised sprite instead of invalidating it.
//!   See `render::SvgTint` for where the inherited colour is read.
//!
//! `fill` / `stroke` from the cascade (Tailwind's `fill-current`, `fill-none`)
//! do reach the document, because "painted or not" is the one thing about a
//! paint that the mask can express — that is what makes the shadcn radio dot,
//! a `<circle class="fill-current">`, visible at all.
//!
//! ## Re-serialization
//!
//! Keyed on a hash of everything that can change the bytes: every tag, every
//! attribute name and value, every inline-style declaration, every text node in
//! the subtree, plus the two computed paints. The hash walk allocates nothing;
//! the serialization behind it runs only when the hash moves. The colour is
//! deliberately not in the key (see above).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::tree::{Kind, Tree};

thread_local! {
    /// node id → (subtree hash, serialized document). One entry per live
    /// `<svg>` element; GC'd with the rest of the per-node state.
    static CACHE: RefCell<HashMap<u32, (u64, Rc<Vec<u8>>)>> = RefCell::new(HashMap::new());
}

/// Drops cached documents for nodes that no longer exist.
pub(crate) fn gc(alive: impl Fn(u32) -> bool) {
    CACHE.with(|m| m.borrow_mut().retain(|&id, _| alive(id)));
}

/// The SVG document for this `<svg>` node, re-serialized only if the subtree or
/// the computed paints moved since the last call.
pub(crate) fn document(
    t: &Tree,
    id: u32,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
) -> Rc<Vec<u8>> {
    let key = hash_subtree(t, id, fill, stroke);
    CACHE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((cached, bytes)) = m.get(&id) {
            if *cached == key {
                return bytes.clone();
            }
        }
        let bytes = Rc::new(serialize(t, id, fill, stroke).into_bytes());
        m.insert(id, (key, bytes.clone()));
        bytes
    })
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

thread_local! {
    /// Did any element in the document just serialized ask for a concrete
    /// colour? Reported once by the renderer, because gpui flattens the whole
    /// document to one tint.
    static SAW_EXPLICIT_COLOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True when the LAST serialization saw a paint gpui cannot honour (a colour
/// other than `currentColor`, or a gradient/pattern `url()`).
pub(crate) fn last_had_explicit_color() -> bool {
    SAW_EXPLICIT_COLOR.with(|c| c.get())
}

fn serialize(
    t: &Tree,
    id: u32,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
) -> String {
    SAW_EXPLICIT_COLOR.with(|c| c.set(false));
    let mut out = String::with_capacity(256);
    write_element(t, id, true, fill, stroke, &mut out);
    out
}

fn write_element(
    t: &Tree,
    id: u32,
    root: bool,
    fill: Option<vn_css::SvgPaint>,
    stroke: Option<vn_css::SvgPaint>,
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
        if name.eq_ignore_ascii_case("fill") || name.eq_ignore_ascii_case("stroke") {
            note_paint(v);
        }
        write_attr(name, v, out);
    }
    if root {
        if let Some(p) = fill {
            if p == vn_css::SvgPaint::Color {
                SAW_EXPLICIT_COLOR.with(|c| c.set(true));
            }
            write_attr("fill", p.as_attr(), out);
        }
        if let Some(p) = stroke {
            if p == vn_css::SvgPaint::Color {
                SAW_EXPLICIT_COLOR.with(|c| c.set(true));
            }
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
            if name.eq_ignore_ascii_case("fill") || name.eq_ignore_ascii_case("stroke") {
                note_paint(v);
            }
        }
        write_attr("style", &body, out);
    }
    out.push('>');

    if !n.text.is_empty() {
        escape_text(&n.text, out);
    } else {
        for &c in &n.children {
            write_element(t, c, false, None, None, out);
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// A paint value that is neither `none` nor `currentColor` is a concrete colour
/// gpui will flatten into the element's `color`.
fn note_paint(value: &str) {
    let v = value.trim();
    let explicit = !(v.is_empty()
        || v.eq_ignore_ascii_case("none")
        || v.eq_ignore_ascii_case("currentcolor")
        || v.eq_ignore_ascii_case("inherit"));
    if explicit {
        SAW_EXPLICIT_COLOR.with(|c| c.set(true));
    }
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
