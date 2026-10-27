//! Programmatic scrolling — `el.scrollTop = n`, `scrollTo`, `scrollBy`,
//! `scrollIntoView()`.
//!
//! # What was here before
//!
//! Nothing. `scrollTop` and `scrollLeft` were plain *fields* on the JS mirror
//! node, written only when a `scroll` event was dispatched — so they read `0`
//! for every container the app had not put a `@scroll` listener on, and
//! writing one reached no host function at all. `scrollIntoView()` was an
//! explicit advisory no-op. Every app that restores a scroll position, jumps to
//! an offset or implements "scroll to bottom" silently did nothing
//! (`research/reports/m23/perf-retained.md` §6 found it, and
//! `research/reports/m23/scroll.md` is the round that closed it).
//!
//! # The four decisions
//!
//! **1. The write is synchronous and the read sees it immediately.** A browser
//! answers the NEW value on the next line, and so do we: a `ScrollHandle` is an
//! `Rc<RefCell<…>>`, so [`set`] moves the offset and [`info`] reads it back
//! with no frame in between. The clamp is the same one gpui's own wheel handler
//! applies, so a programmatic scroll can never leave a box somewhere a real
//! wheel could not.
//!
//! **2. A write lays out first, exactly as a read does.** The clamp needs
//! `max_offset`, which only exists once the box has been laid out — and the
//! canonical idiom is `list.append(row); list.scrollTop = list.scrollHeight`,
//! where the append is newer than any frame. So a write (and a `scrollWidth` /
//! `scrollHeight` read) forces the same synchronous layout
//! [`crate::dom::bounds`] forces, for the same reason and through the same
//! hook. Without it "scroll to bottom" would clamp against the height the list
//! had *before* the row was added.
//!
//! **3. The `scroll` event is asynchronous, because it is asynchronous in a
//! browser.** CSSOM-View fires it from "run the scroll steps" in the
//! update-the-rendering loop, never from inside the assignment. So a write
//! queues the node and [`dispatch_pending`] — called once per frame from
//! [`crate::frame_tick`], which is the one place with a `Window` — fires it.
//! That also keeps the host function free of any need for a `&mut Window`,
//! which it could not get: a write from inside a click handler is already
//! inside that window's update.
//!
//! **4. A rectangle read after a scroll is corrected, not re-measured.** The
//! other half of decision 2, and it cannot use the same hook — the wave that
//! scrolls is usually running inside the window's own update, where a second
//! `draw` is refused. So [`unpainted_shift`] subtracts however far a node's
//! scrollable ancestors have moved since the last build, which is exact
//! (scrolling translates the content and does nothing else) and covers the
//! wheel and the scrollbar thumb for free.
//!
//! # What stays advisory
//!
//! `behavior: "smooth"` performs the scroll **instantly** and leaves one
//! advisory naming it. The final position — the thing app logic reads back — is
//! identical; only the interpolation is missing. Three reasons it is not worth
//! faking with the CSS animator:
//!
//! * a scroll offset is not a style property. It lives in gpui's
//!   `ScrollHandle`, outside the cascade the animator interpolates, so it would
//!   need a second animation channel with its own clock;
//! * `crate::animating()` is a **global** gate on the retained-render cache
//!   (`cache::begin_frame`), so a smooth scroll driven by the animator would
//!   switch caching off document-wide for its whole duration — which is to say
//!   it would cost exactly the frames the cache exists to save, on the one
//!   workload (a long list) where it matters most;
//! * a real smooth scroll must be cancelable by user input and must fire
//!   `scrollend`, and neither exists here. An animation that ignores the wheel
//!   mid-flight is a worse divergence than an instant jump that says so.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use gpui::{px, Point, ScrollHandle};

use crate::render;
use crate::tree;

/// Where `scrollIntoView` puts the box inside its scroll container, on one
/// axis. `block` is the vertical one and `inline` the horizontal one — we have
/// one writing mode (see the advisory in `runtime.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Center,
    End,
    Nearest,
}

impl Align {
    /// The wire encoding `__vn_scroll_into_view` takes. Anything else is
    /// `Nearest`, which is the value CSSOM-View gives `inline` by default.
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
    /// `scrollWidth` / `scrollHeight`: the CONTENT box, which is the viewport
    /// plus everything that overflows it. A browser reports the content; the
    /// painted rectangle is the viewport, and reporting that instead is what
    /// made `el.scrollTop = el.scrollHeight` stop a tenth of the way down.
    pub(crate) scroll_width: f32,
    pub(crate) scroll_height: f32,
    /// The content size and the clamp behind this answer are from the LAST
    /// PAINTED frame, because the tree changed in this same JS wave and a
    /// layout could not be forced (see [`crate::dom::layout_if_stale`]: it
    /// can during boot and `onMounted`, and nowhere inside an event
    /// handler). The offset itself is always exact — it is `scrollWidth`,
    /// `scrollHeight` and the clamp that are a frame old. The JS side turns
    /// this into one advisory rather than letting the number be silently
    /// wrong, which is the whole of the compat rule.
    pub(crate) stale: bool,
}

thread_local! {
    /// Nodes whose offset was moved by JS since the last frame, waiting for
    /// their DOM `scroll` event. See the module docs, decision 3.
    static PENDING: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// The CSS position each scroll container was at when the last frame was
    /// BUILT — which is the position every rectangle in the bounds map was
    /// computed from.
    ///
    /// The gap this closes: a rectangle JS reads comes from the last painted
    /// frame, and a scroll MOVES rectangles, so between the assignment and the
    /// next frame every rect inside that container is the position from before
    /// the scroll. A browser has no such gap —
    ///
    /// ```js
    /// row.scrollIntoView();
    /// row.getBoundingClientRect().top;   // the NEW position, same tick
    /// ```
    ///
    /// — and neither can we close it with the `sync_layout` trick
    /// [`crate::dom::bounds`] uses for an unpainted node: a scroll from a click
    /// handler runs INSIDE the window's own update, where a second `draw` is
    /// refused (and rightly so).
    ///
    /// So it is arithmetic instead, and the arithmetic is exact: scrolling
    /// TRANSLATES the content and does nothing else, so a child's rectangle is
    /// its recorded one minus however far its scrollable ancestors have moved
    /// since. It covers the wheel and the scrollbar thumb for free — both move
    /// the offset during paint, i.e. after the build this map was written by.
    static AT_BUILD: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
}

/// Records where a scroll container stood when this frame's build read it.
/// Called from `render::build_node`, once per painted scroll container.
pub(crate) fn note_built(id: u32, handle: &ScrollHandle) {
    let o = handle.offset();
    AT_BUILD.with(|m| m.borrow_mut().insert(id, (-f32::from(o.x), -f32::from(o.y))));
}

/// How far a node's recorded rectangle has to move to describe where the node
/// is NOW — the sum of every scrollable ancestor's un-painted movement.
///
/// `(0, 0)` in the overwhelmingly common case (nothing has scrolled since the
/// last build), and the early return costs one `is_empty`.
pub(crate) fn unpainted_shift(id: u32) -> (f32, f32) {
    if AT_BUILD.with(|m| m.borrow().is_empty()) {
        return (0.0, 0.0);
    }
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    let mut cur = tree::read_tree(|t| t.node(id).map(|n| n.parent).unwrap_or(0));
    // The same depth cap `Tree::touch` has, for the same reason.
    for _ in 0..4096 {
        if cur == 0 {
            break;
        }
        if let Some(handle) = render::scroll_handle_of(cur) {
            let at_build = AT_BUILD.with(|m| m.borrow().get(&cur).copied()).unwrap_or((0.0, 0.0));
            let now = handle.offset();
            // Scrolling DOWN by d moves the content — and the rectangle — UP.
            dx -= (-f32::from(now.x)) - at_build.0;
            dy -= (-f32::from(now.y)) - at_build.1;
        }
        cur = tree::read_tree(|t| t.node(cur).map(|n| n.parent).unwrap_or(0));
    }
    (dx, dy)
}

/// Drops the queue entry for a node that no longer exists — called from
/// `render::gc_node_state` off a JS wave.
pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    PENDING.with(|p| p.borrow_mut().retain(|&id| live(id)));
    AT_BUILD.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// The scroll position and content size of a container, or `None` for a node
/// the cascade never gave `overflow: scroll|auto`.
///
/// A plain `<div>` answering `None` is not an error: a browser reports `0` for
/// `scrollTop` on every non-scrollable element, and that is what the JS side
/// substitutes.
pub(crate) fn info(id: u32) -> Option<Info> {
    // A browser lays out on read, and `scrollTop` / `scrollWidth` are in the
    // set of properties that flush it. It matters most for the read half of
    // `log.append(line); log.scrollTop = log.scrollHeight` — the argument is
    // evaluated FIRST, so if only the write forced a layout the idiom would
    // aim at the content height from before the append and stop a line short.
    // A clean tree costs one flag read here.
    let stale = crate::dom::layout_if_stale(id);
    let handle = render::scroll_handle_of(id)?;
    Some(info_of(&handle, stale))
}

fn info_of(handle: &ScrollHandle, stale: bool) -> Info {
    // gpui's offsets are <= 0 (the content is shifted up/left); CSS counts
    // down/right as positive.
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
///
/// `None` on an axis leaves it where it is, which is what `scrollTo({top})`
/// with no `left` means. Answers the position AFTER the clamp, so the JS getter
/// can report it on the very next line.
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

/// Moves the handle to `want` (gpui sign), clamped to what the box can
/// actually cover, and reports the position afterwards.
fn apply(id: u32, handle: &ScrollHandle, want: Point<gpui::Pixels>, stale: bool) -> Info {
    let before = handle.offset();
    let max = handle.max_offset();
    // The same clamp gpui's own wheel handler applies, and the same one the
    // devtools `scroll` command has always applied: a programmatic scroll
    // cannot leave the box somewhere a real wheel could not.
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
    // The retained-render cache replays a subtree while the number it rendered
    // at still stands, and a scroll offset is not in that key — a wheel gets
    // away with it because gpui's own listener ends in `cx.notify(view)`, and
    // nothing here does. `touch` bumps `sub_rev` on this node and every
    // ancestor, which is exactly the set of boundaries whose recording has
    // just stopped describing the pixels.
    tree::with_tree(|t| t.touch(id));
}

/// A browser lays out on write, and the clamp needs the layout.
///
/// Same hook, same narrow rule as [`crate::dom::bounds`]: only when the tree
/// still holds changes no frame has taken. `sync_layout` flushes those changes
/// itself, so a wave that writes twice forces one layout, not two.
fn layout_then_handle(id: u32) -> Option<(ScrollHandle, bool)> {
    let stale = crate::dom::layout_if_stale(id);
    render::scroll_handle_of(id).map(|h| (h, stale))
}

/// `element.scrollIntoView(...)`.
///
/// Walks the element's scrollable ANCESTORS from the inside out — the root
/// included, because "the window IS the root" gives it `overflow: scroll`
/// unconditionally — and scrolls each so the element lands where `block` /
/// `inline` ask. Every container is clamped on its own, and the element's
/// rectangle is carried down the walk by however far each one actually moved,
/// so an inner box that could only go part of the way leaves the right work for
/// the outer one.
///
/// Off-screen elements work because we lay the whole document out: a row
/// 22 000 px below the fold has an exact rectangle (asserted in
/// `packages/vn-test/specs/retained.spec.ts`), which is the property `gpui::list`
/// virtualization was rejected for taking away.
///
/// `Err` names why nothing happened — the JS side turns it into one advisory.
pub(crate) fn into_view(id: u32, block: Align, inline: Align) -> Result<bool, String> {
    // `true` = the rectangles and clamps below are from the last painted frame
    // (see [`Info::stale`]); the JS side turns it into the same one advisory a
    // stale `scrollHeight` gets.
    let stale = crate::dom::layout_if_stale(id);
    // The same correction `dom::bounds` applies, for the same reason: two
    // `scrollIntoView()` calls in one JS wave must not both aim at the
    // position the first one moved away from.
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
        // A depth cap for the same reason `Tree::touch` has one: a corrupt
        // parent link must not hang the pump.
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
        // Whatever this container actually took, the element moved with the
        // content by the same amount — so the next container up is asked about
        // where the element IS, not where it was.
        x -= f32::from(before.x - after.x);
        y -= f32::from(before.y - after.y);
        scrolled |= before != after;
    }
    let _ = scrolled;
    Ok(stale)
}

/// How far the content must scroll (CSS sign: positive moves the content
/// up/left) to put `[target_start, target_start + target_len)` where `align`
/// asks inside `[view_start, view_start + view_len)`.
///
/// Pure, and the only interesting arithmetic in this module — which is why it
/// is separated from the walk and unit-tested below against CSSOM-View's own
/// wording for `nearest`.
fn align_delta(align: Align, target_start: f32, target_len: f32, view_start: f32, view_len: f32) -> f32 {
    let target_end = target_start + target_len;
    let view_end = view_start + view_len;
    match align {
        Align::Start => target_start - view_start,
        Align::End => target_end - view_end,
        Align::Center => (target_start + target_len / 2.0) - (view_start + view_len / 2.0),
        Align::Nearest => {
            // CSSOM-View: already fully inside → do nothing; covering the
            // whole viewport on both edges → do nothing (scrolling either way
            // would only take some of it off screen); otherwise bring in the
            // edge that is out, and no further.
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

/// Fires the DOM `scroll` event for every container JS moved since the last
/// frame, and says whether anything moved at all.
///
/// Called once per frame from [`crate::frame_tick`] — the one hook that has a
/// `Window` — which is also what makes the event asynchronous the way a
/// browser's is. The `true` is what repaints: a scroll with no `@scroll`
/// listener anywhere still has to be drawn.
pub(crate) fn dispatch_pending(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    let pending: Vec<u32> = PENDING.with(|p| p.borrow_mut().drain().collect());
    if pending.is_empty() {
        return false;
    }
    for id in pending {
        let Some(handle) = render::scroll_handle_of(id) else { continue };
        // Deduplicated against the last position reported for the node, so a
        // write that lands where the box already was fires nothing — which is
        // what a browser does too.
        render::report_scroll(id, &handle, window, cx);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `start` / `center` / `end` are the three unconditional alignments, and
    /// the sign convention is CSS's: a positive delta scrolls the content up
    /// (the element moves toward the top of the viewport).
    #[test]
    fn the_three_fixed_alignments_are_arithmetic() {
        // A 40 px row at y = 500, in a 200 px viewport that starts at y = 100.
        let (t0, tl, v0, vl) = (500.0, 40.0, 100.0, 200.0);
        assert_eq!(align_delta(Align::Start, t0, tl, v0, vl), 400.0);
        assert_eq!(align_delta(Align::End, t0, tl, v0, vl), 240.0);
        assert_eq!(align_delta(Align::Center, t0, tl, v0, vl), 320.0);
    }

    /// `nearest` is the one with a spec algorithm rather than a formula, and
    /// the four outcomes are the four this test names.
    #[test]
    fn nearest_moves_only_the_edge_that_is_out() {
        let (v0, vl) = (100.0, 200.0); // viewport 100..300
        // Fully inside: nothing happens. This is the case that makes
        // `scrollIntoView` idempotent, and the default for the inline axis.
        assert_eq!(align_delta(Align::Nearest, 150.0, 40.0, v0, vl), 0.0);
        // Above the top: bring the top edge to the top edge, no further.
        assert_eq!(align_delta(Align::Nearest, 40.0, 30.0, v0, vl), -60.0);
        // Below the bottom: bring the bottom edge to the bottom edge.
        assert_eq!(align_delta(Align::Nearest, 400.0, 30.0, v0, vl), 130.0);
        // Taller than the viewport and covering it on both sides: a browser
        // does nothing, because every direction takes some of it off screen.
        assert_eq!(align_delta(Align::Nearest, 0.0, 1000.0, v0, vl), 0.0);
    }

    /// An element exactly flush with an edge is inside, not out — otherwise
    /// every `scrollIntoView` on an already-aligned row would jitter by a
    /// pixel of rounding.
    #[test]
    fn nearest_treats_a_flush_edge_as_visible() {
        assert_eq!(align_delta(Align::Nearest, 100.0, 200.0, 100.0, 200.0), 0.0);
        assert_eq!(align_delta(Align::Nearest, 100.0, 50.0, 100.0, 200.0), 0.0);
        assert_eq!(align_delta(Align::Nearest, 250.0, 50.0, 100.0, 200.0), 0.0);
    }

    /// The wire encoding, including the "anything else is `nearest`" fallback
    /// the JS side relies on for an unknown keyword.
    #[test]
    fn the_alignment_codes_are_what_the_runtime_sends() {
        assert_eq!(Align::from_code(0), Align::Start);
        assert_eq!(Align::from_code(1), Align::Center);
        assert_eq!(Align::from_code(2), Align::End);
        assert_eq!(Align::from_code(3), Align::Nearest);
        assert_eq!(Align::from_code(99), Align::Nearest);
    }

    /// A node with no `ScrollHandle` is not an error — a browser reports `0`
    /// for `scrollTop` on every non-scrollable element and silently ignores a
    /// write to it.
    #[test]
    fn a_node_that_does_not_scroll_answers_nothing() {
        assert!(info(0).is_none());
        assert!(set(0, Some(10.0), Some(10.0)).is_none());
        assert!(by(0, 10.0, 10.0).is_none());
    }
}
