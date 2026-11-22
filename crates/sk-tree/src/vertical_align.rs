//! `vertical-align` from the cascade to the layout: an editor's atoms, and the items of a line of
//! mixed inline content, which Skathi lays out as a baseline-aligned flex row.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{px, Pixels, Window};
use sk_elements::vertical_align::{Alignment, LineText, VerticalAlign};

use crate::tree::{Node, Tree};

thread_local! {
    /// The alignment of each line item being built, picked up where its style is set.
    static PENDING: RefCell<HashMap<u32, Alignment>> = RefCell::new(HashMap::new());
}

/// Node `n`'s computed `vertical-align`, a percentage against its own line height when it sets one.
pub(crate) fn of(n: &Node, rem: Pixels) -> VerticalAlign {
    let Some(cs) = n.computed.as_ref() else {
        return VerticalAlign::default();
    };
    use sk_css::VerticalAlign as V;
    match cs.extras.vertical_align.unwrap_or_default() {
        V::Baseline => VerticalAlign::Baseline,
        V::Sub => VerticalAlign::Sub,
        V::Super => VerticalAlign::Super,
        V::TextTop => VerticalAlign::TextTop,
        V::TextBottom => VerticalAlign::TextBottom,
        V::Middle => VerticalAlign::Middle,
        V::Top => VerticalAlign::Top,
        V::Bottom => VerticalAlign::Bottom,
        V::Shift { fraction, px: by } => VerticalAlign::Shift {
            fraction,
            px: px(by),
            line_height: cs
                .base
                .text
                .line_height
                .map(|lh| lh.to_pixels(px(cs.font_size).into(), rem)),
        },
    }
}

/// The line height `id`'s text has: the nearest declared `line-height`, a number or percentage
/// of `id`'s own font size as CSS inherits it, else the window's.
fn line_height(t: &Tree, id: u32, font_size: Pixels, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let mut at = id;
    while let Some(n) = t.node(at) {
        if let Some(lh) = n.computed.as_ref().and_then(|cs| cs.base.text.line_height) {
            return match lh {
                gpui::DefiniteLength::Absolute(lh) => lh.to_pixels(rem),
                relative => relative.to_pixels(font_size.into(), rem),
            };
        }
        if n.parent == 0 || n.parent == at {
            break;
        }
        at = n.parent;
    }
    window
        .text_style()
        .line_height
        .to_pixels(font_size.into(), rem)
}

/// Records where item `id` of a line goes, when its `vertical-align` moves it: the line is its
/// parent's, so its text is the parent's text.
pub(crate) fn note_line_item(t: &Tree, id: u32, node: &Node, window: &Window) {
    let align = of(node, window.rem_size());
    if align.is_baseline() {
        PENDING.with(|p| p.borrow_mut().remove(&id));
        return;
    }
    let Some(parent) = t.node(node.parent) else {
        return;
    };
    let Some(cs) = parent.computed.as_ref() else {
        return;
    };
    let font_size = px(cs.font_size);
    let line = LineText::measure(
        &cs.computed_font,
        font_size,
        line_height(t, node.parent, font_size, window),
        window,
    );
    PENDING.with(|p| p.borrow_mut().insert(id, align.alignment(&line)));
}

/// Applies a recorded line item's alignment to its style.
pub(crate) fn apply(style: &mut gpui::StyleRefinement, id: u32) {
    let Some(alignment) = PENDING.with(|p| p.borrow_mut().remove(&id)) else {
        return;
    };
    match alignment {
        Alignment::Baseline(shift) => style.vn_baseline_shift = Some(shift),
        Alignment::Top => {
            style.align_self = Some(gpui::AlignSelf::FlexStart);
        }
        Alignment::Bottom => {
            style.align_self = Some(gpui::AlignSelf::FlexEnd);
        }
    }
}
