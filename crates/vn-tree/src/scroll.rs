//! Programmatic scrolling — `el.scrollTop = n`, `scrollTo`, `scrollBy`, `scrollIntoView()`.
//!
//! - The write is synchronous and the read sees it at once (a `ScrollHandle` is an `Rc<RefCell<…>>`),
//!   with the same clamp gpui's wheel handler applies.
//! - A write lays out first, as a read does: the clamp needs `max_offset`, and `list.append(row);
//!   list.scrollTop = list.scrollHeight` appends after the last frame.
//! - The `scroll` event is asynchronous, as in a browser: a write queues the node and
//!   [`dispatch_pending`] (from [`crate::frame_tick`], the one place with a `Window`) fires it.
//! - A rectangle read after a scroll is corrected, not re-measured ([`unpainted_shift`]): a second
//!   `draw` is refused inside the window's own update.
//! - `behavior: "smooth"` scrolls instantly with an advisory: an offset is not a style property the
//!   animator can interpolate.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use gpui::{px, Point, ScrollHandle};

use crate::render;
use crate::tree;

/// Where `scrollIntoView` puts the box inside its scroll container, on one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Center,
    End,
    Nearest,
}

impl Align {
    /// The wire encoding `__vn_scroll_into_view` takes.
    pub(crate) fn from_code(code: u32) -> Self {
        match code {
            0 => Align::Start,
            1 => Align::Center,
            2 => Align::End,
            _ => Align::Nearest,
        }
    }
}

/// One scroll container's position and content size, in CSS px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Info {
    pub(crate) left: f32,
    pub(crate) top: f32,
    /// `scrollWidth` / `scrollHeight`: the CONTENT box, which is the viewport plus everything that
    /// overflows it.
    pub(crate) scroll_width: f32,
    pub(crate) scroll_height: f32,
    /// The content size and the clamp behind this answer are from the LAST PAINTED frame, because
    /// the tree changed in this same JS wave and a layout could not be forced (see
    /// [`crate::dom::layout_if_stale`]: it can during boot and `onMounted`, and nowhere inside an
    /// event handler).
    pub(crate) stale: bool,
}

thread_local! {
    static PENDING: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static AT_BUILD: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
}

/// Records where a scroll container stood when this frame's build read it.
pub(crate) fn note_built(id: u32, handle: &ScrollHandle) {
    let o = handle.offset();
    AT_BUILD.with(|m| m.borrow_mut().insert(id, (-f32::from(o.x), -f32::from(o.y))));
}

pub(crate) fn unpainted_shift(id: u32) -> (f32, f32) {
    if AT_BUILD.with(|m| m.borrow().is_empty()) {
        return (0.0, 0.0);
    }
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    let mut cur = tree::read_tree(|t| t.node(id).map(|n| n.parent).unwrap_or(0));
    for _ in 0..4096 {
        if cur == 0 {
            break;
        }
        if let Some(handle) = render::scroll_handle_of(cur) {
            let at_build = AT_BUILD.with(|m| m.borrow().get(&cur).copied()).unwrap_or((0.0, 0.0));
            let now = handle.offset();
            dx -= (-f32::from(now.x)) - at_build.0;
            dy -= (-f32::from(now.y)) - at_build.1;
        }
        cur = tree::read_tree(|t| t.node(cur).map(|n| n.parent).unwrap_or(0));
    }
    (dx, dy)
}

pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    PENDING.with(|p| p.borrow_mut().retain(|&id| live(id)));
    AT_BUILD.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// The scroll position and content size of a container, or `None` for a node the cascade never gave
/// `overflow: scroll|auto`.
pub(crate) fn info(id: u32) -> Option<Info> {
    // A clean tree costs one flag read here.
    let stale = crate::dom::layout_if_stale(id);
    let handle = render::scroll_handle_of(id)?;
    Some(info_of(&handle, stale))
}

fn info_of(handle: &ScrollHandle, stale: bool) -> Info {
    // gpui's offsets are <= 0 (the content is shifted up/left); CSS counts down/right as positive.
    let offset = handle.offset();
    let max = handle.max_offset();
    let viewport = handle.bounds().size;
    Info {
        left: -f32::from(offset.x),
        top: -f32::from(offset.y),
        scroll_width: f32::from(viewport.width + max.x),
        scroll_height: f32::from(viewport.height + max.y),
        stale,
    }
}

/// `el.scrollTop = n` / `el.scrollLeft = n` / `scrollTo({left, top})`.
pub(crate) fn set(id: u32, left: Option<f32>, top: Option<f32>) -> Option<Info> {
    let (handle, stale) = layout_then_handle(id)?;
    let current = handle.offset();
    let want = Point {
        x: left.map_or(current.x, |v| px(-v)),
        y: top.map_or(current.y, |v| px(-v)),
    };
    Some(apply(id, &handle, want, stale))
}

/// `scrollBy({left, top})`, in CSS px (positive = down/right).
pub(crate) fn by(id: u32, dx: f32, dy: f32) -> Option<Info> {
    let (handle, stale) = layout_then_handle(id)?;
    let current = handle.offset();
    let want = Point { x: current.x - px(dx), y: current.y - px(dy) };
    Some(apply(id, &handle, want, stale))
}

fn apply(id: u32, handle: &ScrollHandle, want: Point<gpui::Pixels>, stale: bool) -> Info {
    let before = handle.offset();
    let max = handle.max_offset();
    // The same clamp gpui's own wheel handler applies, and the same one the devtools `scroll`
    // command has always applied: a programmatic scroll cannot leave the box somewhere a real wheel
    // could not.
    let clamped = Point {
        x: want.x.clamp(-max.x, px(0.)),
        y: want.y.clamp(-max.y, px(0.)),
    };
    if clamped != before {
        handle.set_offset(clamped);
        note_moved(id);
    }
    info_of(handle, stale)
}

/// The bookkeeping every offset change JS causes owes the rest of the engine.
fn note_moved(id: u32) {
    PENDING.with(|p| p.borrow_mut().insert(id));
    tree::with_tree(|t| t.touch(id));
}

/// A browser lays out on write, and the clamp needs the layout.
fn layout_then_handle(id: u32) -> Option<(ScrollHandle, bool)> {
    let stale = crate::dom::layout_if_stale(id);
    render::scroll_handle_of(id).map(|h| (h, stale))
}

/// `element.scrollIntoView(...)`.
pub(crate) fn into_view(id: u32, block: Align, inline: Align) -> Result<bool, String> {
    // `true` = the rectangles and clamps below are from the last painted frame (see
    // [`Info::stale`]); the JS side turns it into the same one advisory a stale `scrollHeight`
    // gets.
    let stale = crate::dom::layout_if_stale(id);
    // The same correction `dom::bounds` applies, for the same reason: two `scrollIntoView()` calls
    // in one JS wave must not both aim at the position the first one moved away from.
    let shift = unpainted_shift(id);
    let Some((mut x, mut y, w, h)) = render::bounds_of(id).map(|(x, y, w, h)| (x + shift.0, y + shift.1, w, h)) else {
        return Err(format!(
            "scrollIntoView(): node {id} has no rectangle — it is detached, \
             `display: none`, or parked in a <KeepAlive> cache, and a browser \
             would not scroll to it either"
        ));
    };
    let chain = tree::read_tree(|t| {
        let mut out = Vec::new();
        let mut cur = t.node(id).map(|n| n.parent).unwrap_or(0);
        // A depth cap for the same reason `Tree::touch` has one: a corrupt parent link must not
        // hang the pump.
        for _ in 0..4096 {
            if cur == 0 {
                break;
            }
            out.push(cur);
            cur = t.node(cur).map(|n| n.parent).unwrap_or(0);
        }
        out
    });
    let mut scrolled = false;
    for ancestor in chain {
        let Some(handle) = render::scroll_handle_of(ancestor) else { continue };
        let viewport = handle.bounds();
        if viewport.size.width <= px(0.) && viewport.size.height <= px(0.) {
            continue;
        }
        let dx = align_delta(inline, x, w, f32::from(viewport.origin.x), f32::from(viewport.size.width));
        let dy = align_delta(block, y, h, f32::from(viewport.origin.y), f32::from(viewport.size.height));
        let before = handle.offset();
        let want = Point { x: before.x - px(dx), y: before.y - px(dy) };
        apply(ancestor, &handle, want, stale);
        let after = handle.offset();
        x -= f32::from(before.x - after.x);
        y -= f32::from(before.y - after.y);
        scrolled |= before != after;
    }
    let _ = scrolled;
    Ok(stale)
}

/// How far the content must scroll (CSS sign: positive moves the content up/left) to put
/// `[target_start, target_start + target_len)` where `align` asks inside `[view_start, view_start +
/// view_len)`.
fn align_delta(align: Align, target_start: f32, target_len: f32, view_start: f32, view_len: f32) -> f32 {
    let target_end = target_start + target_len;
    let view_end = view_start + view_len;
    match align {
        Align::Start => target_start - view_start,
        Align::End => target_end - view_end,
        Align::Center => (target_start + target_len / 2.0) - (view_start + view_len / 2.0),
        Align::Nearest => {
            // CSSOM-View: already fully inside → do nothing; covering the whole viewport on both
            // edges → do nothing (scrolling either way would only take some of it off screen);
            // otherwise bring in the edge that is out, and no further.
            if target_start >= view_start && target_end <= view_end {
                0.0
            } else if target_start < view_start && target_end > view_end {
                0.0
            } else if target_start < view_start {
                target_start - view_start
            } else {
                target_end - view_end
            }
        }
    }
}

/// Fires the DOM `scroll` event for every container JS moved since the last frame, and says whether
/// anything moved at all.
pub(crate) fn dispatch_pending(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    let pending: Vec<u32> = PENDING.with(|p| p.borrow_mut().drain().collect());
    if pending.is_empty() {
        return false;
    }
    for id in pending {
        let Some(handle) = render::scroll_handle_of(id) else { continue };
        render::report_scroll(id, &handle, window, cx);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `start` / `center` / `end` are the three unconditional alignments, and the sign convention
    /// is CSS's: a positive delta scrolls the content up (the element moves toward the top of the
    /// viewport).
    #[test]
    fn the_three_fixed_alignments_are_arithmetic() {
        // A 40 px row at y = 500, in a 200 px viewport that starts at y = 100.
        let (t0, tl, v0, vl) = (500.0, 40.0, 100.0, 200.0);
        assert_eq!(align_delta(Align::Start, t0, tl, v0, vl), 400.0);
        assert_eq!(align_delta(Align::End, t0, tl, v0, vl), 240.0);
        assert_eq!(align_delta(Align::Center, t0, tl, v0, vl), 320.0);
    }

    /// `nearest` is the one with a spec algorithm rather than a formula, and the four outcomes are
    /// the four this test names.
    #[test]
    fn nearest_moves_only_the_edge_that_is_out() {
        let (v0, vl) = (100.0, 200.0);
        assert_eq!(align_delta(Align::Nearest, 150.0, 40.0, v0, vl), 0.0);
        assert_eq!(align_delta(Align::Nearest, 40.0, 30.0, v0, vl), -60.0);
        assert_eq!(align_delta(Align::Nearest, 400.0, 30.0, v0, vl), 130.0);
        // Taller than the viewport and covering it on both sides: a browser does nothing, because
        // every direction takes some of it off screen.
        assert_eq!(align_delta(Align::Nearest, 0.0, 1000.0, v0, vl), 0.0);
    }

    /// An element exactly flush with an edge is inside, not out — otherwise every `scrollIntoView`
    /// on an already-aligned row would jitter by a pixel of rounding.
    #[test]
    fn nearest_treats_a_flush_edge_as_visible() {
        assert_eq!(align_delta(Align::Nearest, 100.0, 200.0, 100.0, 200.0), 0.0);
        assert_eq!(align_delta(Align::Nearest, 100.0, 50.0, 100.0, 200.0), 0.0);
        assert_eq!(align_delta(Align::Nearest, 250.0, 50.0, 100.0, 200.0), 0.0);
    }

    /// The wire encoding, including the "anything else is `nearest`" fallback the JS side relies on
    /// for an unknown keyword.
    #[test]
    fn the_alignment_codes_are_what_the_runtime_sends() {
        assert_eq!(Align::from_code(0), Align::Start);
        assert_eq!(Align::from_code(1), Align::Center);
        assert_eq!(Align::from_code(2), Align::End);
        assert_eq!(Align::from_code(3), Align::Nearest);
        assert_eq!(Align::from_code(99), Align::Nearest);
    }

    #[test]
    fn a_node_that_does_not_scroll_answers_nothing() {
        assert!(info(0).is_none());
        assert!(set(0, Some(10.0), Some(10.0)).is_none());
        assert!(by(0, 10.0, 10.0).is_none());
    }
}
