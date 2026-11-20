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
//! - A smooth scroll (`behavior: "smooth"`, or `auto` on a box with `scroll-behavior: smooth`)
//!   glides there over the next frames ([`tick`]); a read sees where it is so far. An instant
//!   scroll of the same box, or the user scrolling it, stops the glide.
//! - A box with `scroll-snap-type` lands on a snap position ([`crate::snap`]).
//! - The keyboard scrolls too ([`bind_root`]).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{px, Pixels, Point, ScrollHandle};

use crate::render;
use crate::tree;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Center,
    End,
    Nearest,
}

/// CSSOM View's `ScrollBehavior`; `Auto` defers to the scrolled box's `scroll-behavior`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Behavior {
    Auto,
    Instant,
    Smooth,
}

impl Behavior {
    fn smooth_for(self, id: u32) -> bool {
        match self {
            Behavior::Instant => false,
            Behavior::Smooth => true,
            Behavior::Auto => tree::read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|cs| cs.extras.scroll_smooth)
            })
            .unwrap_or(false),
        }
    }
}

struct Glide {
    from: Point<Pixels>,
    to: Point<Pixels>,
    start: Instant,
    duration: Duration,
    /// The offset the glide last wrote: anything else means something else scrolled the box.
    last: Point<Pixels>,
}

impl Glide {
    fn new(from: Point<Pixels>, to: Point<Pixels>) -> Self {
        let distance = f32::from((to.x - from.x).abs()).max(f32::from((to.y - from.y).abs()));
        Glide {
            from,
            to,
            start: Instant::now(),
            duration: glide_duration(distance),
            last: from,
        }
    }

    fn at(&self, now: Instant) -> (Point<Pixels>, bool) {
        let t = now.saturating_duration_since(self.start).as_secs_f32()
            / self.duration.as_secs_f32().max(f32::EPSILON);
        if t >= 1.0 {
            return (self.to, true);
        }
        let e = ease_in_out(t);
        let lerp = |a: Pixels, b: Pixels| a + (b - a) * e;
        (
            Point {
                x: lerp(self.from.x, self.to.x),
                y: lerp(self.from.y, self.to.y),
            },
            false,
        )
    }
}

/// Longer for a longer way, as in browsers, within the 150-500 ms they keep a programmatic glide to.
fn glide_duration(distance: f32) -> Duration {
    Duration::from_secs_f32((distance.sqrt() * 15.0).clamp(150.0, 500.0) / 1000.0)
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Info {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) scroll_width: f32,
    pub(crate) scroll_height: f32,
    /// Content size and clamp are from the LAST PAINTED frame: the tree changed in this JS wave and a
    /// layout could not be forced ([`crate::dom::layout_if_stale`]).
    pub(crate) stale: bool,
}

thread_local! {
    static PENDING: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static AT_BUILD: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
    static GLIDES: RefCell<HashMap<u32, Glide>> = RefCell::new(HashMap::new());
}

pub(crate) fn note_built(id: u32, handle: &ScrollHandle) {
    let o = handle.offset();
    AT_BUILD.with(|m| {
        m.borrow_mut()
            .insert(id, (-f32::from(o.x), -f32::from(o.y)))
    });
}

pub(crate) fn unpainted_shift(id: u32) -> (f32, f32) {
    if AT_BUILD.with(|m| m.borrow().is_empty()) {
        return (0.0, 0.0);
    }
    let (dx, dy) = scrolled_since_build(id);
    let (sx, sy) = crate::sticky::unpainted(id, scrolled_since_build);
    (dx + sx, dy + sy)
}

fn scrolled_since_build(id: u32) -> (f32, f32) {
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    let mut cur = tree::read_tree(|t| t.node(id).map(|n| n.parent).unwrap_or(0));
    for _ in 0..4096 {
        if cur == 0 {
            break;
        }
        if let Some(handle) = render::scroll_handle_of(cur) {
            let at_build = AT_BUILD
                .with(|m| m.borrow().get(&cur).copied())
                .unwrap_or((0.0, 0.0));
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
    GLIDES.with(|g| g.borrow_mut().retain(|&id, _| live(id)));
    crate::snap::gc(live);
}

/// `id`'s border widths in px: top, right, bottom, left.
pub(crate) fn border_px(id: u32) -> [f32; 4] {
    let rem = px(sk_css::rem_basis());
    tree::read_tree(|t| {
        let b = &t.node(id)?.computed.as_ref()?.base.border_widths;
        Some(
            [b.top, b.right, b.bottom, b.left]
                .map(|w| w.map_or(0.0, |w| f32::from(w.to_pixels(rem)))),
        )
    })
    .unwrap_or([0.0; 4])
}

/// `id`'s scrollport: its padding box, the rectangle scroll positions are measured from.
pub(crate) fn scrollport(id: u32) -> Option<crate::windows::Rect> {
    let (x, y, w, h) = render::bounds_of(id)?;
    let [t, r, b, l] = border_px(id);
    Some((x + l, y + t, (w - l - r).max(0.0), (h - t - b).max(0.0)))
}

pub(crate) fn info(id: u32) -> Option<Info> {
    let stale = crate::dom::layout_if_stale(id);
    let handle = render::scroll_handle_of(id)?;
    Some(info_of(id, &handle, stale))
}

fn info_of(id: u32, handle: &ScrollHandle, stale: bool) -> Info {
    // gpui's offsets are <= 0 (the content is shifted up/left); CSS counts down/right as positive.
    let offset = handle.offset();
    let max = handle.max_offset();
    let size = handle.bounds().size;
    let [bt, br, bb, bl] = border_px(id);
    Info {
        left: -f32::from(offset.x),
        top: -f32::from(offset.y),
        scroll_width: (f32::from(size.width) - bl - br).max(0.0) + f32::from(max.x),
        scroll_height: (f32::from(size.height) - bt - bb).max(0.0) + f32::from(max.y),
        stale,
    }
}

pub(crate) fn set(
    id: u32,
    left: Option<f32>,
    top: Option<f32>,
    behavior: Behavior,
) -> Option<Info> {
    let (handle, stale) = layout_then_handle(id)?;
    let smooth = behavior.smooth_for(id);
    // An axis a smooth call leaves alone keeps going where a glide was taking it.
    let current = smooth
        .then(|| glide_target(id))
        .flatten()
        .unwrap_or_else(|| handle.offset());
    let want = Point {
        x: left.map_or(current.x, |v| px(-v)),
        y: top.map_or(current.y, |v| px(-v)),
    };
    let want = snapped(id, &handle, want, current, false, None);
    Some(apply(id, &handle, want, stale, smooth).0)
}

pub(crate) fn by(id: u32, dx: f32, dy: f32, behavior: Behavior) -> Option<Info> {
    by_inner(id, dx, dy, behavior, true)
}

/// [`by`] without scroll snapping: a selection drag's autoscroll.
pub(crate) fn by_free(id: u32, dx: f32, dy: f32, behavior: Behavior) -> Option<Info> {
    by_inner(id, dx, dy, behavior, false)
}

fn by_inner(id: u32, dx: f32, dy: f32, behavior: Behavior, snap: bool) -> Option<Info> {
    let (handle, stale) = layout_then_handle(id)?;
    let smooth = behavior.smooth_for(id);
    // A smooth scrollBy during a glide adds to where it is going, as in browsers.
    let current = smooth
        .then(|| glide_target(id))
        .flatten()
        .unwrap_or_else(|| handle.offset());
    let want = Point {
        x: current.x - px(dx),
        y: current.y - px(dy),
    };
    let want = if snap {
        snapped(id, &handle, want, current, true, None)
    } else {
        want
    };
    Some(apply(id, &handle, want, stale, smooth).0)
}

/// `want` moved onto a snap position of `id`, if it snaps (`crate::snap`). Programmatic: no
/// `scroll-snap-stop`.
fn snapped(
    id: u32,
    handle: &ScrollHandle,
    want: Point<Pixels>,
    from: Point<Pixels>,
    toward: bool,
    prefer: Option<u32>,
) -> Point<Pixels> {
    let css = |p: Point<Pixels>| (-f32::from(p.x), -f32::from(p.y));
    let aim = crate::snap::Aim {
        to: css(want),
        from: css(from),
        toward,
        user: false,
        prefer,
    };
    let (x, y) = crate::snap::settle(id, handle, aim, None);
    Point {
        x: px(-x),
        y: px(-y),
    }
}

pub(crate) fn glide_target(id: u32) -> Option<Point<Pixels>> {
    GLIDES.with(|g| g.borrow().get(&id).map(|glide| glide.to))
}

/// A snap's glide to CSS position `to`, on the programmatic curve.
pub(crate) fn glide_to(id: u32, handle: &ScrollHandle, to: (f32, f32)) {
    let want = Point {
        x: px(-to.0),
        y: px(-to.1),
    };
    apply(id, handle, want, false, true);
}

/// A re-snap's move to CSS position `to`, at once.
pub(crate) fn jump_to(id: u32, handle: &ScrollHandle, to: (f32, f32)) {
    let want = Point {
        x: px(-to.0),
        y: px(-to.1),
    };
    apply(id, handle, want, false, false);
}

/// Moves (or starts gliding) to `want`, clamped; answers the position now and the one it goes to.
fn apply(
    id: u32,
    handle: &ScrollHandle,
    want: Point<Pixels>,
    stale: bool,
    smooth: bool,
) -> (Info, Point<Pixels>) {
    let before = handle.offset();
    let max = handle.max_offset();
    // The clamp gpui's wheel handler applies: a programmatic scroll cannot leave the box where a
    // wheel could not.
    let clamped = Point {
        x: want.x.clamp(-max.x, px(0.)),
        y: want.y.clamp(-max.y, px(0.)),
    };
    GLIDES.with(|g| {
        let mut g = g.borrow_mut();
        if smooth && clamped != before {
            g.insert(id, Glide::new(before, clamped));
        } else {
            g.remove(&id);
        }
    });
    if !smooth && clamped != before {
        handle.set_offset(clamped);
        note_moved(id);
    }
    (info_of(id, handle, stale), clamped)
}

/// Advances every glide one frame; true while any is still moving.
pub(crate) fn tick() -> bool {
    if GLIDES.with(|g| g.borrow().is_empty()) {
        return false;
    }
    let now = Instant::now();
    let mut moved = Vec::new();
    GLIDES.with(|g| {
        g.borrow_mut().retain(|&id, glide| {
            let Some(handle) = render::scroll_handle_of(id) else {
                return false;
            };
            if handle.offset() != glide.last {
                return false;
            }
            let (at, done) = glide.at(now);
            let max = handle.max_offset();
            let at = Point {
                x: at.x.clamp(-max.x, px(0.)),
                y: at.y.clamp(-max.y, px(0.)),
            };
            if at != glide.last {
                handle.set_offset(at);
                glide.last = at;
                moved.push(id);
            }
            !done
        })
    });
    for id in &moved {
        note_moved(*id);
    }
    !moved.is_empty() || GLIDES.with(|g| !g.borrow().is_empty())
}

fn note_moved(id: u32) {
    PENDING.with(|p| p.borrow_mut().insert(id));
    tree::with_tree(|t| t.touch(id));
}

fn layout_then_handle(id: u32) -> Option<(ScrollHandle, bool)> {
    let stale = crate::dom::layout_if_stale(id);
    render::scroll_handle_of(id).map(|h| (h, stale))
}

pub(crate) fn into_view(
    id: u32,
    block: Align,
    inline: Align,
    behavior: Behavior,
) -> Result<bool, String> {
    // `true`: the rectangles and clamps are from the last painted frame ([`Info::stale`]); the JS
    // side turns it into the one advisory.
    let stale = crate::dom::layout_if_stale(id);
    // The same correction `dom::bounds` applies, for the same reason: two `scrollIntoView()` calls
    // in one JS wave must not both aim at the position the first one moved away from.
    let shift = unpainted_shift(id);
    let Some((mut x, mut y, w, h)) =
        render::bounds_of(id).map(|(x, y, w, h)| (x + shift.0, y + shift.1, w, h))
    else {
        return Err(format!(
            "scrollIntoView(): node {id} has no rectangle — it is detached, \
             `display: none`, or parked in a <KeepAlive> cache, and a browser \
             would not scroll to it either"
        ));
    };
    let (chain, margin) = tree::read_tree(|t| {
        let mut out = Vec::new();
        let mut cur = t.node(id).map(|n| n.parent).unwrap_or(0);
        // A depth cap for the same reason `Tree::touch` has one: a corrupt parent link must not
        // hang the pump.
        for _ in 0..4096 {
            if cur == 0 {
                break;
            }
            let padding = t
                .node(cur)
                .and_then(|n| n.computed.as_ref())
                .map(|cs| cs.extras.snap.clone())
                .unwrap_or_default();
            out.push((cur, padding));
            cur = t.node(cur).map(|n| n.parent).unwrap_or(0);
        }
        let margin = t
            .node(id)
            .and_then(|n| n.computed.as_ref())
            .map_or([0.0; 4], |cs| cs.extras.snap.margin_px());
        (out, margin)
    });
    // `scroll-margin` grows the target: top, right, bottom, left.
    x -= margin[3];
    y -= margin[0];
    let (w, h) = (w + margin[3] + margin[1], h + margin[0] + margin[2]);
    let mut scrolled = false;
    for (ancestor, snap) in chain {
        let Some(handle) = render::scroll_handle_of(ancestor) else {
            continue;
        };
        let viewport = handle.bounds();
        if viewport.size.width <= px(0.) && viewport.size.height <= px(0.) {
            continue;
        }
        let [bt, br, bb, bl] = border_px(ancestor);
        let (vx, vy) = (
            f32::from(viewport.origin.x) + bl,
            f32::from(viewport.origin.y) + bt,
        );
        let (vw, vh) = (
            (f32::from(viewport.size.width) - bl - br).max(0.0),
            (f32::from(viewport.size.height) - bt - bb).max(0.0),
        );
        // `scroll-padding` shrinks the viewport it aligns to.
        let pad = snap.padding_px(vw, vh);
        let dx = align_delta(inline, x, w, vx + pad[3], (vw - pad[3] - pad[1]).max(0.0));
        let dy = align_delta(block, y, h, vy + pad[0], (vh - pad[0] - pad[2]).max(0.0));
        let before = handle.offset();
        let want = Point {
            x: before.x - px(dx),
            y: before.y - px(dy),
        };
        let want = snapped(ancestor, &handle, want, before, false, Some(id));
        let smooth = behavior.smooth_for(ancestor);
        let (_, after) = apply(ancestor, &handle, want, stale, smooth);
        x -= f32::from(before.x - after.x);
        y -= f32::from(before.y - after.y);
        scrolled |= before != after;
    }
    let _ = scrolled;
    Ok(stale)
}

fn align_delta(
    align: Align,
    target_start: f32,
    target_len: f32,
    view_start: f32,
    view_len: f32,
) -> f32 {
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

/// Chromium's arrow-key step.
const LINE: f32 = 40.0;
/// A page is this much of the scrollport, so the last lines of one stay in view on the next.
const PAGE: f32 = 0.875;

#[derive(Clone, Copy, Debug, PartialEq)]
enum KeyStep {
    Line { vertical: bool, dir: f32 },
    Page(f32),
    Edge(f32),
}

impl KeyStep {
    fn of(key: &str, shift: bool) -> Option<Self> {
        Some(match key {
            "up" => KeyStep::Line {
                vertical: true,
                dir: -1.0,
            },
            "down" => KeyStep::Line {
                vertical: true,
                dir: 1.0,
            },
            "left" => KeyStep::Line {
                vertical: false,
                dir: -1.0,
            },
            "right" => KeyStep::Line {
                vertical: false,
                dir: 1.0,
            },
            "pageup" => KeyStep::Page(-1.0),
            "pagedown" => KeyStep::Page(1.0),
            "space" => KeyStep::Page(if shift { -1.0 } else { 1.0 }),
            "home" => KeyStep::Edge(-1.0),
            "end" => KeyStep::Edge(1.0),
            _ => return None,
        })
    }
}

/// Arrow keys, Page Up/Down, Space, Home and End scroll the focused element's nearest scroll
/// container that can move that way. Bound on the root after every other key listener, so it runs
/// after the app's handlers and not when one cancelled the key.
pub(crate) fn bind_root<E: gpui::InteractiveElement>(e: E, root: u32) -> E {
    e.on_key_down(move |ev: &gpui::KeyDownEvent, window, _| {
        if window.default_prevented() || crate::windows::here(window, |w| w.key_prevented) {
            return;
        }
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return;
        }
        let Some(step) = KeyStep::of(k.key.as_str(), m.shift) else {
            return;
        };
        let focused = crate::windows::focused_node(crate::windows::key(window));
        let chain = tree::read_tree(|t| {
            let start = if t.node(focused).is_some() {
                focused
            } else {
                root
            };
            let tag = t.node(start).map_or("", |n| t.name(n.tag));
            let own_keys = matches!(tag, "input" | "textarea" | "select")
                || k.key == "space" && matches!(tag, "button" | "summary" | "a");
            if own_keys {
                return Vec::new();
            }
            let mut out = Vec::new();
            let mut cur = start;
            for _ in 0..4096 {
                let Some(n) = t.node(cur) else { break };
                out.push(cur);
                if n.parent == 0 {
                    break;
                }
                cur = n.parent;
            }
            out
        });
        for id in chain {
            if key_scroll(id, step) {
                return;
            }
        }
    })
}

/// Scrolls `id` one key step, snapped and gliding; false when it is not a scroll container or
/// cannot move that way.
fn key_scroll(id: u32, step: KeyStep) -> bool {
    let Some(handle) = render::scroll_handle_of(id) else {
        return false;
    };
    let max = handle.max_offset();
    let max = (f32::from(max.x), f32::from(max.y));
    let at = glide_target(id).unwrap_or_else(|| handle.offset());
    let at = (-f32::from(at.x), -f32::from(at.y));
    let view = handle.bounds().size;
    let mut to = at;
    let toward = match step {
        KeyStep::Line { vertical, dir } => {
            if vertical {
                to.1 += dir * LINE;
            } else {
                to.0 += dir * LINE;
            }
            true
        }
        KeyStep::Page(dir) => {
            to.1 += dir * (f32::from(view.height) * PAGE).max(LINE);
            true
        }
        KeyStep::Edge(dir) => {
            to.1 = if dir < 0.0 { 0.0 } else { max.1 };
            false
        }
    };
    let clamped = (to.0.clamp(0.0, max.0), to.1.clamp(0.0, max.1));
    if (clamped.0 - at.0).abs() < 0.5 && (clamped.1 - at.1).abs() < 0.5 {
        return false;
    }
    // Home and End go to the edge on purpose, past any `scroll-snap-stop: always`.
    let aim = crate::snap::Aim {
        to,
        from: at,
        toward,
        user: toward,
        prefer: None,
    };
    let (x, y) = crate::snap::settle(id, &handle, aim, Some(gpui::HapticTarget::Current));
    let smooth = !tree::env().reduce_motion;
    apply(
        id,
        &handle,
        Point {
            x: px(-x),
            y: px(-y),
        },
        false,
        smooth,
    );
    true
}

pub(crate) fn dispatch_pending(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    let pending: Vec<u32> = PENDING.with(|p| p.borrow_mut().drain().collect());
    if pending.is_empty() {
        return false;
    }
    for id in pending {
        let Some(handle) = render::scroll_handle_of(id) else {
            continue;
        };
        render::report_scroll(id, &handle, window, cx);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glide_moves_one_way_and_lands_exactly() {
        let from = Point {
            x: px(0.),
            y: px(-10.),
        };
        let to = Point {
            x: px(0.),
            y: px(-410.),
        };
        let glide = Glide::new(from, to);
        assert_eq!(glide.at(glide.start), (from, false));
        let mut last = from.y;
        let steps = 20;
        for step in 1..steps {
            let (at, done) = glide.at(glide.start + glide.duration * step / steps);
            assert!(!done && at.y < last, "step {step}: {at:?}");
            last = at.y;
        }
        assert_eq!(glide.at(glide.start + glide.duration), (to, true));
        assert!(glide_duration(10.) < glide_duration(400.));
    }

    #[test]
    fn the_three_fixed_alignments_are_arithmetic() {
        let (t0, tl, v0, vl) = (500.0, 40.0, 100.0, 200.0);
        assert_eq!(align_delta(Align::Start, t0, tl, v0, vl), 400.0);
        assert_eq!(align_delta(Align::End, t0, tl, v0, vl), 240.0);
        assert_eq!(align_delta(Align::Center, t0, tl, v0, vl), 320.0);
    }

    #[test]
    fn nearest_moves_only_the_edge_that_is_out() {
        let (v0, vl) = (100.0, 200.0);
        assert_eq!(align_delta(Align::Nearest, 150.0, 40.0, v0, vl), 0.0);
        assert_eq!(align_delta(Align::Nearest, 40.0, 30.0, v0, vl), -60.0);
        assert_eq!(align_delta(Align::Nearest, 400.0, 30.0, v0, vl), 130.0);
        assert_eq!(align_delta(Align::Nearest, 0.0, 1000.0, v0, vl), 0.0);
    }

    /// An element exactly flush with an edge is inside: otherwise `scrollIntoView` on an aligned row jitters.
    #[test]
    fn nearest_treats_a_flush_edge_as_visible() {
        assert_eq!(align_delta(Align::Nearest, 100.0, 200.0, 100.0, 200.0), 0.0);
        assert_eq!(align_delta(Align::Nearest, 100.0, 50.0, 100.0, 200.0), 0.0);
        assert_eq!(align_delta(Align::Nearest, 250.0, 50.0, 100.0, 200.0), 0.0);
    }
}
