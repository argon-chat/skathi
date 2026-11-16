//! `position: sticky`. The box is laid out in flow with its insets taken off (`apply_computed_style`),
//! then shifted at prepaint from that frame's rectangles: the nearest scroll container's padding box
//! (the build root when none is closer) and the parent's content box, both recorded by their probes
//! before the sticky box prepaints. A scroll therefore moves it without a layout.

use std::cell::RefCell;

use gpui::{
    point, px, AbsoluteLength, AnyElement, App, Bounds, DefiniteLength, Element, ElementId,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Length, Pixels, Window,
};

use crate::idmap::IdMap;
use crate::tree::{Node, Tree};
use crate::windows::{self, Rect, WinKey};

/// Edges in window px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Area {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) right: f32,
    pub(crate) bottom: f32,
}

impl Area {
    fn of(r: Rect) -> Self {
        Area {
            left: r.0,
            top: r.1,
            right: r.0 + r.2,
            bottom: r.1 + r.3,
        }
    }

    fn width(&self) -> f32 {
        self.right - self.left
    }

    fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// `by` is top, right, bottom, left.
    fn deflate(self, by: [f32; 4]) -> Self {
        Area {
            left: self.left + by[3],
            top: self.top + by[0],
            right: self.right - by[1],
            bottom: self.bottom - by[2],
        }
    }

    fn moved(self, (dx, dy): (f32, f32)) -> Self {
        Area {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Insets {
    pub(crate) top: Option<f32>,
    pub(crate) right: Option<f32>,
    pub(crate) bottom: Option<f32>,
    pub(crate) left: Option<f32>,
}

/// The shift that keeps border box `b` at least `insets` inside `port` without leaving `cb` (the
/// containing block's content box less the sticky box's margins). When both sides of an axis apply
/// and conflict, the start side wins.
pub(crate) fn offset(b: Area, cb: Area, port: Area, insets: Insets) -> (f32, f32) {
    let axis = |start: Option<f32>,
                end: Option<f32>,
                (b0, b1): (f32, f32),
                (c0, c1): (f32, f32),
                (p0, p1): (f32, f32)| {
        let mut d = 0.0f32;
        if let Some(end) = end {
            let want = (p1 - end - b1).min(0.0);
            d = want.max((c0 - b0).min(0.0));
        }
        if let Some(start) = start {
            let want = (p0 + start - (b0 + d)).max(0.0);
            d += want.min((c1 - (b1 + d)).max(0.0));
        }
        if d.is_finite() {
            d
        } else {
            0.0
        }
    };
    (
        axis(
            insets.left,
            insets.right,
            (b.left, b.right),
            (cb.left, cb.right),
            (port.left, port.right),
        ),
        axis(
            insets.top,
            insets.bottom,
            (b.top, b.bottom),
            (cb.top, cb.bottom),
            (port.top, port.bottom),
        ),
    )
}

type Sides<T> = [Option<T>; 4];

/// What a sticky box reads at prepaint, gathered while the build has the tree. Sides are top, right,
/// bottom, left.
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    scroller: u32,
    parent: u32,
    inset: Sides<DefiniteLength>,
    margin: Sides<DefiniteLength>,
    parent_padding: Sides<DefiniteLength>,
    parent_border: Sides<AbsoluteLength>,
    port_border: Sides<AbsoluteLength>,
}

/// `None` when no inset is set: the box then stays where flow put it.
pub(crate) fn spec(t: &Tree, node: &Node, root: u32) -> Option<Spec> {
    let cs = node.computed.as_ref()?;
    let definite = |l: Option<Length>| match l {
        Some(Length::Definite(d)) => Some(d),
        _ => None,
    };
    let i = &cs.base.inset;
    let inset = [
        definite(i.top),
        definite(i.right),
        definite(i.bottom),
        definite(i.left),
    ];
    if inset.iter().all(Option::is_none) {
        return None;
    }
    let m = &cs.base.margin;
    let margin = [
        definite(m.top),
        definite(m.right),
        definite(m.bottom),
        definite(m.left),
    ];
    let parent = t.node(node.parent)?.computed.as_ref();
    let scroller = scroller_of(t, node, root);
    let port = t.node(scroller).and_then(|n| n.computed.as_ref());
    let padding = |cs: Option<&sk_css::ComputedStates>| {
        cs.map_or([None; 4], |cs| {
            let p = &cs.base.padding;
            [p.top, p.right, p.bottom, p.left]
        })
    };
    let border = |cs: Option<&sk_css::ComputedStates>| {
        cs.map_or([None; 4], |cs| {
            let b = &cs.base.border_widths;
            [b.top, b.right, b.bottom, b.left]
        })
    };
    Some(Spec {
        scroller,
        parent: node.parent,
        inset,
        margin,
        parent_padding: padding(parent),
        parent_border: border(parent),
        port_border: border(port),
    })
}

/// The nearest ancestor that is a scroll container (`overflow` `hidden`, `scroll` or `auto` on
/// either axis), or the build root, which stands for the viewport.
pub(crate) fn scroller_of(t: &Tree, node: &Node, root: u32) -> u32 {
    let mut cur = node.parent;
    for _ in 0..4096 {
        if cur == root || cur == 0 {
            break;
        }
        let Some(n) = t.node(cur) else { break };
        let scrolls = n.computed.as_ref().is_some_and(|cs| {
            cs.extras
                .overflow_names(&cs.base)
                .iter()
                .any(|k| matches!(*k, "hidden" | "scroll" | "auto"))
        });
        if scrolls {
            return cur;
        }
        cur = n.parent;
    }
    root
}

/// One sticky box as last prepainted: what a rectangle read after a scroll, before the next paint,
/// recomputes the shift from ([`unpainted`]).
struct Stuck {
    win: WinKey,
    scroller: u32,
    b: Area,
    cb: Area,
    port: Area,
    insets: Insets,
    shift: (f32, f32),
}

thread_local! {
    static STUCK: RefCell<IdMap<Stuck>> = RefCell::new(IdMap::default());
}

/// A window's build is about to prepaint every sticky box it still paints.
pub(crate) fn begin_frame(win: WinKey) {
    STUCK.with(|m| m.borrow_mut().retain(|_, s| s.win != win));
}

pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    STUCK.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// How far the sticky boxes at or above `id` moved since they were painted, given `moved(n)`: how
/// far scrolling moved node `n`'s box since then.
pub(crate) fn unpainted(id: u32, moved: impl Fn(u32) -> (f32, f32)) -> (f32, f32) {
    if STUCK.with(|m| m.borrow().is_empty()) {
        return (0.0, 0.0);
    }
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    let mut cur = id;
    for _ in 0..4096 {
        if cur == 0 {
            break;
        }
        let hit = STUCK.with(|m| {
            m.borrow()
                .get(&cur)
                .map(|s| (s.scroller, s.b, s.cb, s.port, s.insets, s.shift))
        });
        if let Some((scroller, b, cb, port, insets, shift)) = hit {
            let along = moved(cur);
            let under = moved(scroller);
            if along != (0.0, 0.0) || under != (0.0, 0.0) {
                let now = offset(b.moved(along), cb.moved(along), port.moved(under), insets);
                dx += now.0 - shift.0;
                dy += now.1 - shift.1;
            }
        }
        cur = crate::tree::read_tree(|t| t.node(cur).map_or(0, |n| n.parent));
    }
    (dx, dy)
}

pub(crate) fn wrap(id: u32, spec: Option<Spec>, inner: AnyElement) -> AnyElement {
    match spec {
        Some(spec) => Sticky { id, spec, inner }.into_any_element(),
        None => inner,
    }
}

struct Sticky {
    id: u32,
    spec: Spec,
    inner: AnyElement,
}

fn resolve(len: Option<DefiniteLength>, base: f32, rem: Pixels) -> f32 {
    len.map_or(0.0, |l| {
        f32::from(l.to_pixels(AbsoluteLength::Pixels(px(base.max(0.0))), rem))
    })
}

fn resolve_all(sides: &Sides<DefiniteLength>, base: f32, rem: Pixels) -> [f32; 4] {
    sides.map(|l| resolve(l, base, rem))
}

fn widths(sides: &Sides<AbsoluteLength>, rem: Pixels) -> [f32; 4] {
    sides.map(|w| w.map_or(0.0, |w| f32::from(w.to_pixels(rem))))
}

impl Sticky {
    fn measure(&self, bounds: Bounds<Pixels>, window: &Window) -> Option<Stuck> {
        let s = &self.spec;
        let rem = window.rem_size();
        let (port, parent) = windows::here(window, |w| {
            (
                w.bounds.get(&s.scroller).copied(),
                w.bounds.get(&s.parent).copied(),
            )
        });
        let port = Area::of(port?).deflate(widths(&s.port_border, rem));
        let cb = if s.parent == s.scroller {
            // The scroll container's own children are held to its scrolled content, not its box.
            let (off, max) = crate::render::scroll_handle_of(s.scroller)
                .map(|h| {
                    let (o, m) = (h.offset(), h.max_offset());
                    (
                        (f32::from(o.x), f32::from(o.y)),
                        (f32::from(m.x).max(0.0), f32::from(m.y).max(0.0)),
                    )
                })
                .unwrap_or_default();
            let content = Area {
                left: port.left,
                top: port.top,
                right: port.right + max.0,
                bottom: port.bottom + max.1,
            }
            .moved(off);
            content.deflate(resolve_all(&s.parent_padding, port.width(), rem))
        } else {
            let p = Area::of(parent?).deflate(widths(&s.parent_border, rem));
            p.deflate(resolve_all(&s.parent_padding, p.width(), rem))
        };
        let cb = cb.deflate(resolve_all(&s.margin, cb.width(), rem));
        let insets = Insets {
            top: s.inset[0].map(|l| resolve(Some(l), port.height(), rem)),
            right: s.inset[1].map(|l| resolve(Some(l), port.width(), rem)),
            bottom: s.inset[2].map(|l| resolve(Some(l), port.height(), rem)),
            left: s.inset[3].map(|l| resolve(Some(l), port.width(), rem)),
        };
        let b = Area {
            left: f32::from(bounds.origin.x),
            top: f32::from(bounds.origin.y),
            right: f32::from(bounds.origin.x + bounds.size.width),
            bottom: f32::from(bounds.origin.y + bounds.size.height),
        };
        Some(Stuck {
            win: windows::key(window),
            scroller: s.scroller,
            b,
            cb,
            port,
            insets,
            shift: offset(b, cb, port, insets),
        })
    }
}

impl Element for Sticky {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let shift = match self.measure(bounds, window) {
            Some(stuck) => {
                let shift = stuck.shift;
                STUCK.with(|m| m.borrow_mut().insert(self.id, stuck));
                shift
            }
            None => {
                STUCK.with(|m| m.borrow_mut().remove(&self.id));
                (0.0, 0.0)
            }
        };
        window.with_element_offset(point(px(shift.0), px(shift.1)), |window| {
            self.inner.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::with_tree;

    fn area(left: f32, top: f32, width: f32, height: f32) -> Area {
        Area::of((left, top, width, height))
    }

    fn top(v: f32) -> Insets {
        Insets {
            top: Some(v),
            ..Default::default()
        }
    }

    const PORT: Area = Area {
        left: 0.0,
        top: 100.0,
        right: 400.0,
        bottom: 500.0,
    };

    #[test]
    fn below_the_threshold_it_stays_where_flow_put_it() {
        let header = area(0.0, 150.0, 400.0, 30.0);
        let section = area(0.0, 150.0, 400.0, 600.0);
        assert_eq!(offset(header, section, PORT, top(0.0)), (0.0, 0.0));
        assert_eq!(offset(header, section, PORT, top(50.0)), (0.0, 0.0));
        assert_eq!(offset(header, section, PORT, top(60.0)), (0.0, 10.0));
    }

    #[test]
    fn scrolled_past_the_threshold_it_sticks_to_the_scrollport() {
        let header = area(0.0, -200.0, 400.0, 30.0);
        let section = area(0.0, -200.0, 400.0, 1000.0);
        assert_eq!(offset(header, section, PORT, top(0.0)), (0.0, 300.0));
        assert_eq!(offset(header, section, PORT, top(8.0)), (0.0, 308.0));
    }

    #[test]
    fn the_next_section_pushes_it_out_with_its_own() {
        let header = area(0.0, -500.0, 400.0, 30.0);
        let section = area(0.0, -500.0, 400.0, 620.0);
        // The section ends at 120: the header's bottom edge rides it, 20px into the scrollport.
        assert_eq!(offset(header, section, PORT, top(0.0)), (0.0, 590.0));
        let gone = area(0.0, -500.0, 400.0, 30.0);
        assert_eq!(offset(gone, gone, PORT, top(0.0)), (0.0, 0.0));
    }

    #[test]
    fn bottom_holds_it_up_from_the_bottom_edge() {
        let footer = area(0.0, 900.0, 400.0, 40.0);
        let section = area(0.0, 0.0, 400.0, 940.0);
        let bottom = Insets {
            bottom: Some(10.0),
            ..Default::default()
        };
        assert_eq!(offset(footer, section, PORT, bottom), (0.0, -450.0));
        // The section's top edge is as high as it can go.
        let short = area(0.0, 600.0, 400.0, 340.0);
        assert_eq!(offset(footer, short, PORT, bottom), (0.0, -300.0));
    }

    #[test]
    fn both_axes_hold_a_corner_cell() {
        let corner = area(-300.0, -50.0, 80.0, 30.0);
        let row = area(-300.0, -50.0, 2000.0, 3000.0);
        let both = Insets {
            top: Some(0.0),
            left: Some(0.0),
            ..Default::default()
        };
        assert_eq!(offset(corner, row, PORT, both), (300.0, 150.0));
        let column = Insets {
            left: Some(0.0),
            ..Default::default()
        };
        assert_eq!(offset(corner, row, PORT, column), (300.0, 0.0));
    }

    #[test]
    fn the_start_edge_wins_when_both_apply() {
        let tall = area(0.0, 50.0, 400.0, 600.0);
        let cb = area(0.0, 0.0, 400.0, 2000.0);
        let both = Insets {
            top: Some(0.0),
            bottom: Some(0.0),
            ..Default::default()
        };
        // Bottom lifts it 50 (as far as the containing block allows), top puts its top edge back
        // on the scrollport's.
        assert_eq!(offset(tall, cb, PORT, both), (0.0, 50.0));
    }

    fn scroll_box(overflow: gpui::Overflow) -> sk_css::ComputedStates {
        let mut base = gpui::StyleRefinement::default();
        base.overflow.y = Some(overflow);
        sk_css::ComputedStates {
            base: std::rc::Rc::new(base),
            ..Default::default()
        }
    }

    #[test]
    fn the_nearest_scroll_container_is_the_reference() {
        with_tree(|t| {
            let div = t.intern("div");
            let root = t.create_root();
            let outer = t.create_element(div);
            let inner = t.create_element(div);
            let plain = t.create_element(div);
            let header = t.create_element(div);
            t.insert(outer, root, 0);
            t.insert(inner, outer, 0);
            t.insert(plain, inner, 0);
            t.insert(header, plain, 0);
            let at = |t: &Tree| scroller_of(t, t.node(header).unwrap(), root);
            assert_eq!(at(t), root, "nothing scrolls: the viewport");
            t.node_mut(outer).unwrap().computed = Some(scroll_box(gpui::Overflow::Scroll));
            assert_eq!(at(t), outer);
            t.node_mut(inner).unwrap().computed = Some(scroll_box(gpui::Overflow::Hidden));
            assert_eq!(at(t), inner, "`overflow: hidden` is a scroll container too");
            t.node_mut(inner).unwrap().computed = Some(scroll_box(gpui::Overflow::Visible));
            assert_eq!(at(t), outer);
        });
    }

    #[test]
    fn a_read_after_a_scroll_moves_only_the_part_the_threshold_lets_through() {
        let win = windows::UNBOUND;
        let b = area(0.0, 200.0, 400.0, 30.0);
        let cb = area(0.0, 200.0, 400.0, 1000.0);
        STUCK.with(|m| {
            m.borrow_mut().insert(
                7,
                Stuck {
                    win,
                    scroller: 3,
                    b,
                    cb,
                    port: PORT,
                    insets: top(0.0),
                    shift: (0.0, 0.0),
                },
            )
        });
        // Scrolled by 250: flow moves it to -50, the threshold holds it at 100.
        let moved = |n: u32| if n == 7 { (0.0, -250.0) } else { (0.0, 0.0) };
        assert_eq!(unpainted(7, moved), (0.0, 150.0));
        STUCK.with(|m| m.borrow_mut().clear());
    }
}
