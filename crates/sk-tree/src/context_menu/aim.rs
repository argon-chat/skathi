use gpui::{point, px, rgba, Bounds, PathBuilder, Pixels, Point, Window};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use crate::windows::Rect;

type Pt = (f32, f32);

#[derive(Clone, Copy)]
pub(super) struct Aim {
    left: f32,
    right: f32,
    y: f32,
    direction: i8,
    peak: f32,
}

impl Aim {
    pub(super) fn on_trigger(previous: Option<Self>, (x, y): Pt) -> Self {
        // A short stretch of the approach survives two vertically aligned samples across a row.
        Self {
            left: previous.map_or(x, |a| a.left.min(x).max(x - 32.)),
            right: previous.map_or(x, |a| a.right.max(x).min(x + 32.)),
            y,
            direction: 0,
            peak: x,
        }
    }

    pub(super) fn advance(&mut self, direction: i8, x: f32) {
        if direction != 0 {
            self.direction = direction;
            self.peak = if direction > 0 {
                self.peak.max(x)
            } else {
                self.peak.min(x)
            };
        }
    }
}

#[derive(Clone, Copy)]
struct Wedge {
    from: Pt,
    edge: f32,
    start: f32,
    top: f32,
    bottom: f32,
}

impl Wedge {
    fn new(a: Aim, (left, top, w, h): Rect, direction: i8) -> Option<Self> {
        if a.direction != 0 && a.direction != direction {
            return None;
        }
        let (x, edge) = if direction > 0 {
            (a.left, left)
        } else {
            (a.right, left + w)
        };
        let sign = f32::from(direction);
        if (edge - x) * sign <= 0. {
            return None;
        }
        // Retreat is measured from the furthest point reached, including several small steps back.
        let start = if direction > 0 {
            x.max(a.peak - 6.)
        } else {
            x.min(a.peak + 6.)
        };
        ((edge - start) * sign >= 0.).then_some(Self {
            from: (x, a.y),
            edge,
            start,
            top: top - 4.,
            bottom: top + h + 4.,
        })
    }

    fn limits(self, x: f32) -> (f32, f32) {
        let (ax, ay) = self.from;
        let t = (x - ax) / (self.edge - ax);
        // The pointer may stray 3 CSS px vertically. For an edge entirely above or below
        // the apex both limits shift in the same direction.
        let upper = ay + t * (self.top - ay) + if self.top > ay { 3. } else { -3. };
        let lower = ay + t * (self.bottom - ay) + if self.bottom < ay { -3. } else { 3. };
        (upper, lower)
    }

    fn contains(self, (x, y): Pt) -> bool {
        if x == self.from.0 || !(self.start.min(self.edge)..=self.start.max(self.edge)).contains(&x)
        {
            return false;
        }
        let (upper, lower) = self.limits(x);
        (upper..=lower).contains(&y)
    }

    fn corners(self) -> [Pt; 4] {
        let (a, b) = self.limits(self.start);
        let (c, d) = self.limits(self.edge);
        [
            (self.start, a),
            (self.edge, c),
            (self.edge, d),
            (self.start, b),
        ]
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Zone {
    grace: Option<Pt>,
    wedges: [Option<Wedge>; 2],
}

const GRACE: f32 = 4.;

impl Zone {
    pub(super) fn new(aim: Option<Aim>, bounds: Option<Rect>) -> Self {
        let Some(a) = aim else { return Self::default() };
        Self {
            grace: (a.direction == 0).then_some((a.peak, a.y)),
            wedges: [1, -1].map(|d| bounds.and_then(|b| Wedge::new(a, b, d))),
        }
    }

    pub(super) fn hit(self, p: Pt) -> Option<i8> {
        if self
            .grace
            .is_some_and(|(x, y)| (p.0 - x).hypot(p.1 - y) <= GRACE)
        {
            return Some(0);
        }
        self.wedges
            .into_iter()
            .flatten()
            .find(|w| w.contains(p))
            .map(|w| if w.edge > w.from.0 { 1 } else { -1 })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Tracking,
    Held,
    Released,
}

struct Trace {
    child: u32,
    zone: Zone,
    state: State,
    points: VecDeque<Pt>,
    last_miss: Option<Pt>,
}

thread_local! {
    static TRACES: RefCell<HashMap<u32, Trace>> = RefCell::new(HashMap::new());
}

pub(super) fn record(owner: u32, child: u32, zone: Zone, state: State, p: Pt) {
    TRACES.with_borrow_mut(|traces| {
        let t = traces.entry(owner).or_insert_with(|| Trace {
            child,
            zone,
            state,
            points: VecDeque::new(),
            last_miss: None,
        });
        if t.child != child || (state == State::Tracking && t.state != State::Tracking) {
            t.points.clear();
        }
        t.child = child;
        t.zone = zone;
        t.state = state;
        if state == State::Released {
            t.last_miss = Some(p);
        }
        if t.points.back() != Some(&p) {
            if t.points.len() == 40 {
                t.points.pop_front();
            }
            t.points.push_back(p);
        }
    });
}

pub(super) fn reposition(owner: u32, child: u32, zone: Zone) {
    TRACES.with_borrow_mut(|traces| {
        if let Some(t) = traces
            .get_mut(&owner)
            .filter(|t| t.child == child && t.state != State::Released)
        {
            t.zone = zone;
        }
    });
}

pub(super) fn clear(root: u32) {
    TRACES.with_borrow_mut(|traces| {
        traces.retain(|owner, t| {
            if *owner == root {
                return false;
            }
            if t.child == root && t.state != State::Released {
                t.zone = Zone::default();
                t.points.clear();
                return t.last_miss.is_some();
            }
            true
        });
    });
}

fn pos((x, y): Pt) -> Point<Pixels> {
    point(px(x), px(y))
}

fn path(
    window: &mut Window,
    points: impl IntoIterator<Item = Pt>,
    closed: bool,
    width: f32,
    color: u32,
) {
    let mut builder = if width == 0. {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(width))
    };
    let mut points = points.into_iter();
    let Some(first) = points.next() else { return };
    builder.move_to(pos(first));
    for p in points {
        builder.line_to(pos(p));
    }
    if closed {
        builder.close();
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, rgba(color));
    }
}

fn dot(window: &mut Window, (x, y): Pt, r: f32, fill: u32, stroke: u32) {
    window.paint_quad(gpui::quad(
        Bounds::new(pos((x - r, y - r)), gpui::size(px(2. * r), px(2. * r))),
        px(r),
        rgba(fill),
        px(1.),
        rgba(stroke),
        gpui::BorderStyle::Solid,
    ));
}

pub(crate) fn paint(owner: u32, window: &mut Window) {
    TRACES.with_borrow(|traces| {
        let Some(t) = traces.get(&owner) else { return };
        let ink = match t.state {
            State::Tracking => 0x007aaeff,
            State::Held => 0x00836aff,
            State::Released => 0xd44133ff,
        };
        for w in t.zone.wedges.into_iter().flatten() {
            let points = w.corners();
            path(window, points, true, 0., ink & 0xffffff00 | 0x34);
            path(window, points, true, 1.5, ink);
        }
        if let Some(p) = t.zone.grace {
            dot(window, p, GRACE, ink & 0xffffff00 | 0x34, ink);
        }
        if t.points.len() > 1 {
            path(window, t.points.iter().copied(), false, 3., 0xffffffdd);
            path(window, t.points.iter().copied(), false, 1.5, 0x263349ff);
        }
        if t.state != State::Released {
            if let Some(&p) = t.points.back() {
                dot(window, p, 3., ink, 0xffffffff);
            }
        }
        if let Some((x, y)) = t.last_miss {
            for (width, color) in [(4., 0xffffffff), (2., 0xd44133ff)] {
                for d in [-4., 4.] {
                    path(
                        window,
                        [(x - 4., y - d), (x + 4., y + d)],
                        false,
                        width,
                        color,
                    );
                }
            }
        }
    });
}
