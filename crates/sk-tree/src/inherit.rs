//! Inherited properties the cascade records only where they are declared, read through the
//! ancestors: `interpolate-size` and `dynamic-range-limit`.

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

pub(crate) fn dynamic_range_limit_in(t: &Tree, id: u32) -> sk_css::DynamicRangeLimit {
    nearest(t, id, |x| x.dynamic_range_limit).unwrap_or_default()
}

/// The computed `dynamic-range-limit` of an element: how bright the HDR content it shows (an
/// `<sk-surface>`, `<video>` or image) may be. Windows draw SDR today, so only a surface's
/// producer acts on it ([`crate::surface::SurfaceInfo::dynamic_range_limit`]).
pub fn dynamic_range_limit(id: u32) -> sk_css::DynamicRangeLimit {
    tree::read_tree(|t| dynamic_range_limit_in(t, id))
}
