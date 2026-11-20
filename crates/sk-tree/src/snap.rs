//! CSS Scroll Snap: where a snapping scroll container comes to rest, and putting it there after each
//! way it scrolls.
//!
//! - A programmatic or keyboard scroll picks its destination here before it moves
//!   ([`crate::scroll`]), and a smooth one glides there on the programmatic curve.
//! - A wheel, touchpad or touch gesture snaps once its events have stopped for [`QUIET`], which
//!   also waits out momentum. A notched wheel, an arrow key and `scrollBy` are directional, as in
//!   browsers: they reach at least the next snap position.
//! - A scrollbar drag or track click snaps on release.
//! - After a layout change a container goes back to the area it was snapped to, or, when
//!   `mandatory`, to the nearest snap position.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{Point, ScrollHandle};
use sk_css::snap::{SnapAlign, SnapType};

use crate::{render, scroll, tree};

const QUIET: Duration = Duration::from_millis(120);
/// `proximity` snaps only within this fraction of the snapport, as Chromium does.
const PROXIMITY: f32 = 1.0 / 3.0;
const EPS: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Intent {
    /// As near the destination as a snap position allows.
    End,
    /// At least one snap position past `from`, toward the destination.
    Toward { from: f32 },
}

/// One axis of the snapport: the scrollport's length, its `scroll-padding`, and the scroll range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Port {
    pub(crate) view: f32,
    pub(crate) pad_start: f32,
    pub(crate) pad_end: f32,
    pub(crate) max: f32,
}

/// One axis of a snap area, in the container's content coordinates, before `scroll-margin`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Area {
    pub(crate) id: u32,
    pub(crate) start: f32,
    pub(crate) end: f32,
    pub(crate) margin_start: f32,
    pub(crate) margin_end: f32,
    pub(crate) align: SnapAlign,
    pub(crate) always: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Candidate {
    pos: f32,
    always: bool,
    id: u32,
}

/// An area longer than the snapport: every position from `lo` to `hi` keeps it covering the
/// snapport, and each is a snap position.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cover {
    lo: f32,
    hi: f32,
    id: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Axis {
    candidates: Vec<Candidate>,
    covers: Vec<Cover>,
    snapport: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pick {
    pub(crate) pos: f32,
    pub(crate) id: u32,
}

pub(crate) fn axis(areas: &[Area], port: Port) -> Axis {
    let len = (port.view - port.pad_start - port.pad_end).max(0.0);
    let clamp = |p: f32| p.clamp(0.0, port.max.max(0.0));
    let mut out = Axis {
        snapport: len,
        ..Axis::default()
    };
    for a in areas {
        let (a0, a1) = (a.start - a.margin_start, a.end + a.margin_end);
        let pos = match a.align {
            SnapAlign::None => continue,
            SnapAlign::Start => a0 - port.pad_start,
            SnapAlign::End => a1 - port.pad_start - len,
            SnapAlign::Center => (a0 + a1) / 2.0 - port.pad_start - len / 2.0,
        };
        out.candidates.push(Candidate {
            pos: clamp(pos),
            always: a.always,
            id: a.id,
        });
        if a1 - a0 > len + EPS {
            let (lo, hi) = (clamp(a0 - port.pad_start), clamp(a1 - port.pad_start - len));
            out.covers.push(Cover { lo, hi, id: a.id });
            for pos in [lo, hi] {
                out.candidates.push(Candidate {
                    pos,
                    always: false,
                    id: a.id,
                });
            }
        }
    }
    out
}

fn direction(delta: f32) -> f32 {
    if delta > EPS {
        1.0
    } else if delta < -EPS {
        -1.0
    } else {
        0.0
    }
}

/// The snap position a scroll aimed at `target` lands on, or `None` to stay at `target`. `origin` is
/// where a user scroll began, for `scroll-snap-stop: always`; `prefer` is an area to land on when it
/// has a snap position.
pub(crate) fn select(
    axis: &Axis,
    target: f32,
    intent: Intent,
    origin: Option<f32>,
    mandatory: bool,
    prefer: Option<u32>,
) -> Option<Pick> {
    let distance = |c: &&Candidate| (c.pos - target).abs();
    if let Some(id) = prefer {
        if let Some(c) = axis
            .candidates
            .iter()
            .filter(|c| c.id == id)
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        {
            return Some(Pick { pos: c.pos, id });
        }
    }
    let barrier = origin.and_then(|o| {
        let d = direction(target - o);
        axis.candidates
            .iter()
            .filter(|c| d != 0.0 && c.always && (c.pos - o) * d > EPS)
            .min_by(|a, b| (a.pos - o).abs().total_cmp(&(b.pos - o).abs()))
            .map(|c| (c.pos, d))
    });
    let inside = |p: f32| barrier.is_none_or(|(b, d)| (p - b) * d <= EPS);
    if inside(target) {
        if let Some(c) = axis
            .covers
            .iter()
            .find(|c| c.lo - EPS <= target && target <= c.hi + EPS)
        {
            return Some(Pick {
                pos: target,
                id: c.id,
            });
        }
    }
    let nearest = |keep: &dyn Fn(&Candidate) -> bool| {
        axis.candidates
            .iter()
            .filter(|c| inside(c.pos) && keep(c))
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
    };
    let best = match intent {
        Intent::Toward { from } if direction(target - from) != 0.0 => {
            let d = direction(target - from);
            nearest(&|c| (c.pos - from) * d > EPS).or_else(|| {
                if mandatory {
                    nearest(&|_| true)
                } else {
                    None
                }
            })
        }
        _ => nearest(&|_| true),
    }?;
    if !mandatory && (best.pos - target).abs() > axis.snapport * PROXIMITY {
        return None;
    }
    Some(Pick {
        pos: best.pos,
        id: best.id,
    })
}

/// A scroll about to happen: where it is aimed and where it starts, in CSS pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Aim {
    pub(crate) to: (f32, f32),
    pub(crate) from: (f32, f32),
    /// Directional (a notched wheel, an arrow key, `scrollBy`) rather than to an end position.
    pub(crate) toward: bool,
    /// A user scroll, which `scroll-snap-stop: always` holds.
    pub(crate) user: bool,
    pub(crate) prefer: Option<u32>,
}

#[derive(Default)]
struct State {
    pending: Option<Pending>,
    held: bool,
    snapped: [Option<u32>; 2],
    feedback_at: Option<((f32, f32), gpui::HapticTarget)>,
    seen: Option<Seen>,
}

#[derive(Clone, Copy)]
struct Pending {
    from: (f32, f32),
    last: Instant,
    toward: bool,
    user: bool,
}

/// What a re-snap depends on: the container's box and range and where its snapped areas sit.
#[derive(Clone, Copy, PartialEq)]
struct Seen {
    rect: (f32, f32, f32, f32),
    max: (f32, f32),
    areas: [Option<(f32, f32, f32, f32)>; 2],
}

thread_local! {
    static STATES: RefCell<HashMap<u32, State>> = RefCell::new(HashMap::new());
}

fn snap_type(id: u32) -> Option<SnapType> {
    tree::read_tree(|t| {
        let cs = t.node(id)?.computed.as_ref()?;
        cs.extras.snap.snap_type.filter(|s| s.snaps())
    })
}

/// A scroll container was built; `snaps` is whether it has a `scroll-snap-type`.
pub(crate) fn note_built(id: u32, snaps: bool) {
    STATES.with(|s| {
        let mut s = s.borrow_mut();
        if snaps {
            s.entry(id).or_default();
        } else if !s.is_empty() {
            s.remove(&id);
        }
    });
}

pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    STATES.with(|s| s.borrow_mut().retain(|&id, _| live(id)));
}

fn css(o: Point<gpui::Pixels>) -> (f32, f32) {
    (-f32::from(o.x), -f32::from(o.y))
}

/// What the user did to container `id`, from its scrollbar element.
pub(crate) fn note_gesture(id: u32, handle: &ScrollHandle, gesture: sk_elements::Gesture) {
    let now = Instant::now();
    STATES.with(|s| {
        let mut s = s.borrow_mut();
        let Some(state) = s.get_mut(&id) else { return };
        state.feedback_at = None;
        match gesture {
            sk_elements::Gesture::Wheel { discrete } => {
                let from = state.pending.map(|p| p.from).unwrap_or_else(|| {
                    scroll::glide_target(id).map_or_else(|| css(handle.offset()), css)
                });
                let toward = discrete || state.pending.is_some_and(|p| p.toward);
                state.pending = Some(Pending {
                    from,
                    last: now,
                    toward,
                    user: true,
                });
                sk_runtime::frames::wake_at(now + QUIET);
            }
            sk_elements::Gesture::Grab => {
                state.held = true;
                state.pending = None;
            }
            sk_elements::Gesture::Release => {
                state.held = false;
                state.pending = Some(Pending {
                    from: css(handle.offset()),
                    last: now.checked_sub(QUIET).unwrap_or(now),
                    toward: false,
                    user: false,
                });
                sk_runtime::frames::wake();
            }
        }
    });
}

/// Where a scroll of `id` aimed at `aim.to` comes to rest; `aim.to` itself when it does not snap.
/// Remembers the areas it lands on, for [`tick`]'s re-snap.
pub(crate) fn settle(
    id: u32,
    handle: &ScrollHandle,
    aim: Aim,
    feedback: Option<gpui::HapticTarget>,
) -> (f32, f32) {
    let Some(kind) = snap_type(id) else {
        return aim.to;
    };
    let Some(axes) = geometry(id, handle) else {
        return aim.to;
    };
    let mut out = aim.to;
    let mut snapped = [None; 2];
    for (i, (ax, on)) in axes.iter().zip([kind.x, kind.y]).enumerate() {
        if !on {
            continue;
        }
        let (to, from) = if i == 0 {
            (aim.to.0, aim.from.0)
        } else {
            (aim.to.1, aim.from.1)
        };
        let intent = if aim.toward {
            Intent::Toward { from }
        } else {
            Intent::End
        };
        let origin = aim.user.then_some(from);
        if let Some(pick) = select(ax, to, intent, origin, kind.mandatory, aim.prefer) {
            snapped[i] = Some(pick.id);
            if i == 0 {
                out.0 = pick.pos;
            } else {
                out.1 = pick.pos;
            }
        }
    }
    STATES.with(|s| {
        if let Some(state) = s.borrow_mut().get_mut(&id) {
            let changed = snapped != state.snapped && snapped.iter().any(Option::is_some);
            for (slot, (pick, on)) in state
                .snapped
                .iter_mut()
                .zip(snapped.into_iter().zip([kind.x, kind.y]))
            {
                if on {
                    *slot = pick;
                }
            }
            state.seen = None;
            state.feedback_at = feedback.filter(|_| changed).map(|target| (out, target));
        }
    });
    out
}

/// The snap areas of container `id` on each axis, in its content coordinates. `None` when it has
/// not been laid out.
fn geometry(id: u32, handle: &ScrollHandle) -> Option<[Axis; 2]> {
    let (padding, areas) = tree::read_tree(|t| {
        let snap = t.node(id)?.computed.as_ref()?.extras.snap.clone();
        let mut areas = Vec::new();
        let mut stack: Vec<u32> = t.node(id)?.children.iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            let Some(node) = t.node(n) else { continue };
            if node.kind != tree::Kind::Element {
                continue;
            }
            if let Some(cs) = node.computed.as_ref() {
                let s = &cs.extras.snap;
                if let Some(align) = s.align.filter(|a| *a != (SnapAlign::None, SnapAlign::None)) {
                    areas.push((n, align, s.stop_always == Some(true), s.margin_px()));
                }
            }
            if render::scroll_handle_of(n).is_none() {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        Some((snap, areas))
    })?;
    let rect = content_rect(id, id, handle)?;
    let max = handle.max_offset();
    let pad = padding.padding_px(rect.2, rect.3);
    let ports = [
        Port {
            view: rect.2,
            pad_start: pad[3],
            pad_end: pad[1],
            max: f32::from(max.x),
        },
        Port {
            view: rect.3,
            pad_start: pad[0],
            pad_end: pad[2],
            max: f32::from(max.y),
        },
    ];
    let mut per_axis: [Vec<Area>; 2] = [Vec::new(), Vec::new()];
    for (n, align, always, margin) in areas {
        let Some((x, y, w, h)) = content_rect(id, n, handle) else {
            continue;
        };
        per_axis[0].push(Area {
            id: n,
            start: x,
            end: x + w,
            margin_start: margin[3],
            margin_end: margin[1],
            align: align.0,
            always,
        });
        per_axis[1].push(Area {
            id: n,
            start: y,
            end: y + h,
            margin_start: margin[0],
            margin_end: margin[2],
            align: align.1,
            always,
        });
    }
    Some([axis(&per_axis[0], ports[0]), axis(&per_axis[1], ports[1])])
}

/// `node`'s rectangle in container `id`'s content coordinates (where it sits with the container
/// scrolled to 0, 0, from the padding box's corner), corrected for scrolls since the last paint as
/// `getBoundingClientRect()` is. For `id` itself, its scrollport.
fn content_rect(id: u32, node: u32, handle: &ScrollHandle) -> Option<(f32, f32, f32, f32)> {
    let (cx, cy, cw, ch) = scroll::scrollport(id)?;
    if node == id {
        return Some((0.0, 0.0, cw, ch));
    }
    let (x, y, w, h) = render::bounds_of(node)?;
    let (csx, csy) = scroll::unpainted_shift(id);
    let (sx, sy) = scroll::unpainted_shift(node);
    let (left, top) = css(handle.offset());
    Some((
        (x + sx) - (cx + csx) + left,
        (y + sy) - (cy + csy) + top,
        w,
        h,
    ))
}

/// Snaps the gestures that have gone quiet and re-snaps containers whose layout changed. True when
/// it moved a box without a glide.
pub(crate) fn tick() -> bool {
    let ids: Vec<u32> = STATES.with(|s| s.borrow().keys().copied().collect());
    if ids.is_empty() {
        return false;
    }
    let now = Instant::now();
    let mut moved = false;
    for id in ids {
        let Some(handle) = render::scroll_handle_of(id) else {
            continue;
        };
        let (held, pending) = STATES.with(|s| {
            s.borrow()
                .get(&id)
                .map_or((true, None), |st| (st.held, st.pending))
        });
        if held {
            continue;
        }
        if let Some(p) = pending {
            let due = p.last + QUIET;
            if now < due {
                sk_runtime::frames::wake_at(due);
                continue;
            }
            STATES.with(|s| {
                if let Some(st) = s.borrow_mut().get_mut(&id) {
                    st.pending = None;
                }
            });
            let to = settle(
                id,
                &handle,
                Aim {
                    to: css(handle.offset()),
                    from: p.from,
                    toward: p.toward,
                    user: p.user,
                    prefer: None,
                },
                Some(gpui::HapticTarget::Mouse),
            );
            scroll::glide_to(id, &handle, to);
        }
        if scroll::glide_target(id).is_none() {
            let destination = STATES.with(|s| {
                s.borrow_mut()
                    .get_mut(&id)
                    .and_then(|st| st.feedback_at.take())
            });
            if let Some((to, target)) = destination {
                let at = css(handle.offset());
                if (to.0 - at.0).abs() <= EPS && (to.1 - at.1).abs() <= EPS {
                    let window = crate::windows::window_of_node(id);
                    crate::windows::of(window, |w| w.haptics.snaps.push((id, target)));
                }
            }
            moved |= resnap(id, &handle);
        }
    }
    moved
}

fn resnap(id: u32, handle: &ScrollHandle) -> bool {
    let Some(kind) = snap_type(id) else {
        return false;
    };
    let Some(rect) = render::bounds_of(id) else {
        return false;
    };
    let snapped = STATES.with(|s| s.borrow().get(&id).map(|st| st.snapped));
    let Some(snapped) = snapped else { return false };
    let max = handle.max_offset();
    let seen = Seen {
        rect,
        max: (f32::from(max.x), f32::from(max.y)),
        areas: snapped.map(|a| a.and_then(|a| content_rect(id, a, handle))),
    };
    let unchanged = STATES.with(|s| {
        let mut s = s.borrow_mut();
        let Some(st) = s.get_mut(&id) else {
            return true;
        };
        let same = st.seen == Some(seen);
        st.seen = Some(seen);
        same
    });
    if unchanged {
        return false;
    }
    let Some(axes) = geometry(id, handle) else {
        return false;
    };
    let now = css(handle.offset());
    let mut to = now;
    let mut picked = snapped;
    for (i, (ax, on)) in axes.iter().zip([kind.x, kind.y]).enumerate() {
        if !on {
            continue;
        }
        let at = if i == 0 { now.0 } else { now.1 };
        let covered = |area: u32| {
            ax.covers
                .iter()
                .any(|c| c.id == area && c.lo - EPS <= at && at <= c.hi + EPS)
        };
        let pick = match snapped[i] {
            Some(area) if ax.candidates.iter().any(|c| c.id == area) => {
                select(ax, at, Intent::End, None, true, Some(area)).filter(|_| !covered(area))
            }
            _ if kind.mandatory => select(ax, at, Intent::End, None, true, None),
            _ => None,
        };
        if let Some(pick) = pick {
            picked[i] = Some(pick.id);
            if i == 0 {
                to.0 = pick.pos;
            } else {
                to.1 = pick.pos;
            }
        }
    }
    STATES.with(|s| {
        if let Some(st) = s.borrow_mut().get_mut(&id) {
            st.snapped = picked;
        }
    });
    if (to.0 - now.0).abs() <= EPS && (to.1 - now.1).abs() <= EPS {
        return false;
    }
    scroll::jump_to(id, handle, to);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: Port = Port {
        view: 400.0,
        pad_start: 0.0,
        pad_end: 0.0,
        max: 1600.0,
    };

    /// Five 400 px slides, each `align` on this axis.
    fn slides(align: SnapAlign, size: f32) -> Vec<Area> {
        (0..5)
            .map(|i| Area {
                id: i + 1,
                start: i as f32 * size,
                end: (i + 1) as f32 * size,
                margin_start: 0.0,
                margin_end: 0.0,
                align,
                always: false,
            })
            .collect()
    }

    fn at(axis: &Axis, target: f32, intent: Intent, mandatory: bool) -> Option<f32> {
        select(axis, target, intent, None, mandatory, None).map(|p| p.pos)
    }

    #[test]
    fn mandatory_always_lands_and_proximity_only_when_near() {
        let ax = axis(&slides(SnapAlign::Start, 400.0), PORT);
        assert_eq!(at(&ax, 130.0, Intent::End, true), Some(0.0));
        assert_eq!(at(&ax, 270.0, Intent::End, true), Some(400.0));
        assert_eq!(at(&ax, 30.0, Intent::End, false), Some(0.0));
        assert_eq!(at(&ax, 200.0, Intent::End, false), None);
    }

    #[test]
    fn a_directional_scroll_reaches_the_next_position_however_short() {
        let ax = axis(&slides(SnapAlign::Start, 400.0), PORT);
        let toward = |from| Intent::Toward { from };
        assert_eq!(at(&ax, 440.0, toward(400.0), true), Some(800.0));
        assert_eq!(at(&ax, 360.0, toward(400.0), true), Some(0.0));
        assert_eq!(at(&ax, 1250.0, toward(400.0), true), Some(1200.0));
        assert_eq!(at(&ax, 1640.0, toward(1600.0), true), Some(1600.0));
        assert_eq!(at(&ax, 440.0, toward(400.0), false), None);
    }

    #[test]
    fn center_alignment_and_the_clamp_to_the_scroll_range() {
        let ax = axis(
            &slides(SnapAlign::Center, 300.0),
            Port {
                max: 1100.0,
                ..PORT
            },
        );
        let pos: Vec<f32> = ax.candidates.iter().map(|c| c.pos).collect();
        assert_eq!(pos, vec![0.0, 250.0, 550.0, 850.0, 1100.0]);
    }

    #[test]
    fn snap_stop_always_cannot_be_passed_by_a_user_scroll() {
        let mut areas = slides(SnapAlign::Start, 400.0);
        areas[1].always = true;
        let ax = axis(&areas, PORT);
        let user = select(&ax, 1500.0, Intent::End, Some(0.0), true, None);
        assert_eq!(user.map(|p| p.pos), Some(400.0));
        let script = select(&ax, 1500.0, Intent::End, None, true, None);
        assert_eq!(script.map(|p| p.pos), Some(1600.0));
        let back = select(&ax, 100.0, Intent::End, Some(1200.0), true, None);
        assert_eq!(back.map(|p| p.pos), Some(400.0));
        let onward = select(&ax, 1500.0, Intent::End, Some(400.0), true, None);
        assert_eq!(onward.map(|p| p.pos), Some(1600.0));
    }

    #[test]
    fn scroll_padding_shrinks_the_snapport_and_scroll_margin_grows_the_area() {
        let mut areas = slides(SnapAlign::Start, 400.0);
        for a in &mut areas {
            a.margin_start = 10.0;
        }
        let port = Port {
            pad_start: 50.0,
            ..PORT
        };
        let ax = axis(&areas, port);
        assert_eq!(at(&ax, 330.0, Intent::End, true), Some(340.0));
        let end = axis(
            &slides(SnapAlign::End, 300.0),
            Port {
                pad_end: 40.0,
                ..PORT
            },
        );
        assert_eq!(at(&end, 250.0, Intent::End, true), Some(240.0));
    }

    #[test]
    fn inside_an_area_longer_than_the_snapport_any_position_holds() {
        let mut areas = slides(SnapAlign::Start, 400.0);
        areas[1].end = 1400.0;
        for a in areas.iter_mut().skip(2) {
            a.start += 600.0;
            a.end += 600.0;
        }
        let ax = axis(
            &areas,
            Port {
                max: 2200.0,
                ..PORT
            },
        );
        assert_eq!(at(&ax, 700.0, Intent::End, true), Some(700.0));
        assert_eq!(
            at(&ax, 760.0, Intent::Toward { from: 720.0 }, true),
            Some(760.0)
        );
        assert_eq!(
            at(&ax, 1040.0, Intent::Toward { from: 1000.0 }, true),
            Some(1400.0)
        );
        assert_eq!(at(&ax, 1500.0, Intent::End, true), Some(1400.0));
    }

    #[test]
    fn both_axes_pick_independently() {
        let ax = axis(&slides(SnapAlign::Start, 400.0), PORT);
        let ay = axis(
            &slides(SnapAlign::Start, 300.0),
            Port {
                max: 1100.0,
                ..PORT
            },
        );
        assert_eq!(at(&ax, 380.0, Intent::End, true), Some(400.0));
        assert_eq!(at(&ay, 380.0, Intent::End, true), Some(300.0));
    }

    #[test]
    fn a_preferred_area_wins_over_a_nearer_one() {
        let ax = axis(&slides(SnapAlign::Start, 400.0), PORT);
        let pick = select(&ax, 790.0, Intent::End, None, true, Some(4));
        assert_eq!(pick, Some(Pick { pos: 1200.0, id: 4 }));
    }
}
