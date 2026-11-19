use std::ops::Range;
use usvg::roxmltree::{Document, Node};

pub(super) struct Attribute {
    pub offset: usize,
    pub node: u32,
}

pub(super) fn has_dimensions(tag: &str) -> bool {
    matches!(tag, "rect" | "svg" | "image" | "use" | "foreignObject")
}

pub(super) fn is_length(name: &str) -> bool {
    matches!(
        name,
        "x" | "y" | "width" | "height" | "rx" | "ry" | "cx" | "cy" | "r"
    )
}

pub(super) fn skipped_parameter<'a>(
    tag: &str,
    value: impl Fn(&str) -> Option<&'a str>,
) -> Option<&'static str> {
    let required: &[&str] = match tag {
        "polyline" | "polygon" => {
            let points = value("points").unwrap_or_default();
            return (svgtypes::PointsParser::from(points).take(2).count() < 2).then_some("points");
        }
        "rect" => &["width", "height"],
        "circle" => &["r"],
        "ellipse" => &["rx", "ry"],
        _ => return None,
    };
    required.iter().copied().find(|name| {
        let mut v = value(name).filter(|v| *v != "auto");
        if tag == "ellipse" && v.is_none() {
            v = value(if *name == "rx" { "ry" } else { "rx" }).filter(|v| *v != "auto");
        }
        v.is_none_or(|v| {
            if v.contains('(') || v.contains('%') {
                return false;
            }
            sk_css::svg_user_length(v, 0.).is_none_or(|n| n <= 0.)
        })
    })
}

pub(super) fn view_box_size(text: &str) -> Option<[f32; 2]> {
    let view = text.parse::<svgtypes::ViewBox>().ok()?;
    let [w, h] = [view.w as f32, view.h as f32];
    (w.is_finite() && h.is_finite() && w > 0. && h > 0.).then_some([w, h])
}

/// Only computed CSS attributes are expanded. Offsets refer to the immutable document before
/// the root size and optional shadow wrapper are applied.
pub(super) fn materialize(text: &str, offsets: &[Attribute], size: [f32; 2]) -> String {
    let Ok(doc) = Document::parse(text) else {
        return text.to_owned();
    };
    let mut edits = Vec::with_capacity(offsets.len());
    collect(doc.root_element(), size, true, offsets, &mut edits);
    let mut result = String::with_capacity(text.len());
    let mut from = 0;
    for (range, value) in edits {
        result.push_str(&text[from..range.start]);
        result.push_str(&value.to_string());
        from = range.end;
    }
    result.push_str(&text[from..]);
    result
}

fn collect(
    node: Node,
    parent: [f32; 2],
    root: bool,
    offsets: &[Attribute],
    edits: &mut Vec<(Range<usize>, f32)>,
) {
    for attr in node.attributes() {
        let Ok(index) = offsets.binary_search_by_key(&attr.range().start, |a| a.offset) else {
            continue;
        };
        let basis = match attr.name() {
            "x" | "cx" | "width" | "rx" => parent[0],
            "y" | "cy" | "height" | "ry" => parent[1],
            "r" => parent[0].hypot(parent[1]) / std::f32::consts::SQRT_2,
            _ => continue,
        };
        if let Some(value) = sk_css::svg_user_length(attr.value(), basis) {
            let tag = node.tag_name().name();
            if value <= 0.
                && matches!(
                    (tag, attr.name()),
                    ("rect", "width" | "height") | ("circle", "r") | ("ellipse", "rx" | "ry")
                )
            {
                warn_skipped(
                    offsets[index].node,
                    tag,
                    node.attribute("id").unwrap_or_default(),
                    attr.name(),
                );
            }
            edits.push((attr.range_value(), value));
        }
    }
    let size = match node.tag_name().name() {
        "svg" | "symbol" => node
            .attribute("viewBox")
            .and_then(view_box_size)
            .unwrap_or_else(|| {
                if root {
                    return parent;
                }
                let dimension = |name, basis| {
                    node.attribute(name)
                        .and_then(|v| sk_css::svg_user_length(v, basis))
                        .unwrap_or(basis)
                };
                [
                    dimension("width", parent[0]),
                    dimension("height", parent[1]),
                ]
            }),
        _ => parent,
    };
    for child in node.children().filter(Node::is_element) {
        collect(child, size, false, offsets, edits);
    }
}

pub(super) fn warn_skipped(id: u32, tag: &str, label: &str, attr: &str) {
    crate::render::warn_compat_at(
        &format!("svg-shape-{id}-{attr}"),
        || format!("<{tag}> {label} (node {id}): "),
        &format!("resvg skips this shape because `{attr}` is missing or invalid"),
    );
}
