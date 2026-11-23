//! Inherited properties the cascade records only where they are declared, read through the
//! ancestors: `interpolate-size`, `dynamic-range-limit`, `line-metrics` and `line-leading`.

use crate::tree::{self, Tree};

fn nearest<T>(t: &Tree, id: u32, get: impl Fn(&sk_css::Extras) -> Option<T>) -> Option<T> {
    let mut at = id;
    while let Some(n) = t.node(at) {
        if let Some(v) = n.computed.as_ref().and_then(|c| get(&c.extras)) {
            return Some(v);
        }
        if n.parent == 0 || n.parent == at {
            return None;
        }
        at = n.parent;
    }
    None
}

pub(crate) fn layout_value<T: Copy + Default>(
    t: &Tree,
    mut id: u32,
    inherited: bool,
    get: impl Fn(&sk_css::Extras) -> Option<sk_css::LayoutValue<T>>,
) -> T {
    while let Some(node) = t.node(id) {
        match node.computed.as_ref().and_then(|style| get(&style.extras)) {
            Some(sk_css::LayoutValue::Value(value)) => return value,
            None if !inherited => break,
            _ => {}
        }
        if node.parent == 0 || node.parent == id {
            break;
        }
        id = node.parent;
    }
    T::default()
}

/// `interpolate-size: allow-keywords` on the element, `own` standing in for its own declaration
/// while its new style is not stored yet.
pub(crate) fn interpolate_keywords(t: &Tree, id: u32, own: Option<bool>) -> bool {
    own.or_else(|| {
        let parent = t.node(id)?.parent;
        (parent != 0 && parent != id)
            .then(|| nearest(t, parent, |x| x.interpolate_size))
            .flatten()
    })
    .unwrap_or(false)
}

/// `id`'s computed `line-metrics` and `line-leading`.
pub(crate) fn line_box(t: &Tree, id: u32) -> gpui::LineBoxRules {
    gpui::LineBoxRules {
        metrics: nearest_text(t, id, |s| s.line_metrics).unwrap_or_default(),
        leading: nearest_text(t, id, |s| s.line_leading).unwrap_or_default(),
    }
}

fn nearest_text<T>(
    t: &Tree,
    id: u32,
    get: impl Fn(&gpui::TextStyleRefinement) -> Option<T>,
) -> Option<T> {
    let mut at = id;
    while let Some(n) = t.node(at) {
        if let Some(v) = n.computed.as_ref().and_then(|c| get(&c.base.text)) {
            return Some(v);
        }
        if n.parent == 0 || n.parent == at {
            return None;
        }
        at = n.parent;
    }
    None
}

pub(crate) fn dynamic_range_limit_in(t: &Tree, id: u32) -> sk_css::DynamicRangeLimit {
    nearest(t, id, |x| x.dynamic_range_limit).unwrap_or_default()
}

/// The computed `dynamic-range-limit` of an element: how bright the HDR content it shows (an
/// `<sk-surface>`, `<video>` or image) may be. An HDR window paints a half-float surface up to
/// it; a producer reads it as [`crate::surface::SurfaceInfo::dynamic_range_limit`].
pub fn dynamic_range_limit(id: u32) -> sk_css::DynamicRangeLimit {
    tree::read_tree(|t| dynamic_range_limit_in(t, id))
}
