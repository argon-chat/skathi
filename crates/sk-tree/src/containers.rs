//! Container queries across layout. A cascade answers `@container` and `cq*` units from the sizes
//! the last layout gave each query container, and every container it asked is watched. After a
//! layout, a watched container whose content box moved on an asked axis restyles its subtree, which
//! is laid out again in the same frame, until nothing moves ([`settle`]).
//!
//! Nothing runs while no container is watched, and a frame whose layout moved no watched container
//! compares a few rectangles.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use gpui::{App, Window};
use sk_css::container::{BLOCK, INLINE};

use crate::{render, tree, windows};

/// Layouts one frame may spend on container queries before it gives up on a cycle (a container
/// whose size follows what its own queries change).
const MAX_PASSES: u32 = 8;

struct Watch {
    axes: u8,
    answered: Option<(f32, f32)>,
}

#[derive(Default)]
struct PercentWatch {
    axes: u8,
    answered: Option<(f32, f32)>,
    measured: Option<(f32, f32)>,
    svg_viewport: Option<u32>,
}

/// What the content box is inset by: padding and border px, plus percentages of the parent's width.
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct Insets {
    px: (f32, f32),
    percent: (f32, f32),
    parent: u32,
}

thread_local! {
    static WATCHED: RefCell<HashMap<u32, Watch>> = RefCell::new(HashMap::new());
    static PERCENTAGES: RefCell<HashMap<u32, PercentWatch>> = RefCell::new(HashMap::new());
    static INSETS: RefCell<HashMap<u32, Insets>> = RefCell::new(HashMap::new());
    /// Elements first cascaded since the frame began: a later pass in the same frame is still
    /// their first style, so it starts no transition from the pass before.
    static BORN: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static CHECKED_AT: Cell<u64> = const { Cell::new(u64::MAX) };
    static PASSES: Cell<u64> = const { Cell::new(0) };
    static RESTYLED: Cell<u64> = const { Cell::new(0) };
    static WARNED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn install() {
    sk_css::container::set_size_source(answer);
}

fn answer(id: u32, axes: u8) -> Option<(f32, f32)> {
    if axes & sk_css::container::PERCENT_BASIS != 0 {
        return PERCENTAGES.with(|w| {
            let mut w = w.borrow_mut();
            let watch = w.entry(id).or_default();
            watch.axes |= axes;
            watch.answered = watch.measured;
            watch.measured
        });
    }
    let size = content_box(id);
    WATCHED.with(|w| {
        let mut w = w.borrow_mut();
        let watch = w.entry(id).or_insert(Watch {
            axes: 0,
            answered: size,
        });
        watch.axes |= axes;
        watch.answered = size;
    });
    size
}

fn content_box(id: u32) -> Option<(f32, f32)> {
    content_box_in(&windows::keys(), id)
}

fn content_box_in(keys: &[windows::WinKey], id: u32) -> Option<(f32, f32)> {
    let bounds = |id: u32| {
        keys.iter()
            .find_map(|&k| windows::peek(k, |w| w.bounds.get(&id).copied()).flatten())
    };
    let (_, _, w, h) = bounds(id)?;
    let insets = INSETS
        .with(|i| i.borrow().get(&id).copied())
        .unwrap_or_default();
    let basis = if insets.percent == (0.0, 0.0) {
        0.0
    } else {
        bounds(insets.parent).map_or(0.0, |(_, _, pw, _)| pw)
    };
    Some((
        (w - insets.px.0 - insets.percent.0 * basis).max(0.0),
        (h - insets.px.1 - insets.percent.1 * basis).max(0.0),
    ))
}

/// Whether a cascade can ask about a container at all.
pub(crate) fn watching() -> bool {
    WATCHED.with(|w| !w.borrow().is_empty()) || PERCENTAGES.with(|w| !w.borrow().is_empty())
}

pub(crate) fn record_percentage_basis(id: u32, measure: impl FnOnce() -> Option<(f32, f32)>) {
    PERCENTAGES.with(|w| {
        if let Some(watch) = w.borrow_mut().get_mut(&id) {
            watch.measured = measure();
        }
    });
}

pub(crate) fn note_percentage_style(t: &tree::Tree, id: u32, reads_basis: bool) {
    PERCENTAGES.with(|w| {
        let mut w = w.borrow_mut();
        if !reads_basis {
            w.remove(&id);
        } else if let Some(watch) = w.get_mut(&id) {
            watch.svg_viewport = crate::svg::sizing_viewport(t, id);
        }
    });
}

pub(crate) fn forget_all() {
    WATCHED.with(|w| w.borrow_mut().clear());
    PERCENTAGES.with(|w| w.borrow_mut().clear());
}

pub(crate) fn note_committed(id: u32, parent: u32, first: bool, cs: &sk_css::ComputedStates) {
    if first {
        BORN.with(|b| b.borrow_mut().insert(id));
    }
    let insets = (cs.extras.container_type.unwrap_or(0) != 0).then(|| insets_of(&cs.base, parent));
    INSETS.with(|i| {
        let mut i = i.borrow_mut();
        if let Some(insets) = insets {
            i.insert(id, insets);
        } else {
            i.remove(&id);
        }
    });
    if insets.is_none() {
        WATCHED.with(|w| w.borrow_mut().remove(&id));
    }
}

/// A pass after the first in this frame must not transition an element born in it.
pub(crate) fn born_this_frame(id: u32) -> bool {
    BORN.with(|b| !b.borrow().is_empty() && b.borrow().contains(&id))
}

fn insets_of(base: &gpui::StyleRefinement, parent: u32) -> Insets {
    let rem = sk_css::rem_basis();
    let mut insets = Insets {
        parent,
        ..Insets::default()
    };
    let mut add = |length: Option<gpui::DefiniteLength>, horizontal: bool| {
        let (px, percent) = match length {
            Some(gpui::DefiniteLength::Absolute(a)) => (f32::from(a.to_pixels(gpui::px(rem))), 0.0),
            Some(gpui::DefiniteLength::Fraction(f)) => (0.0, f),
            Some(gpui::DefiniteLength::Calc { fraction, pixels }) => (f32::from(pixels), fraction),
            None => (0.0, 0.0),
        };
        if horizontal {
            insets.px.0 += px;
            insets.percent.0 += percent;
        } else {
            insets.px.1 += px;
            insets.percent.1 += percent;
        }
    };
    let p = &base.padding;
    add(p.left, true);
    add(p.right, true);
    add(p.top, false);
    add(p.bottom, false);
    let b = &base.border_widths;
    for (side, horizontal) in [
        (b.left, true),
        (b.right, true),
        (b.top, false),
        (b.bottom, false),
    ] {
        add(side.map(gpui::DefiniteLength::Absolute), horizontal);
    }
    insets
}

fn moved(before: Option<(f32, f32)>, now: Option<(f32, f32)>, axes: u8) -> bool {
    match (before, now) {
        (Some((w0, h0)), Some((w1, h1))) => {
            (axes & INLINE != 0 && (w0 - w1).abs() > 0.01)
                || (axes & BLOCK != 0 && (h0 - h1).abs() > 0.01)
        }
        (None, None) => false,
        _ => true,
    }
}

/// Marks the subtree of every watched container the last layout moved; `false` when none moved.
/// A moved container stops being watched until a cascade asks about it again.
pub(crate) fn invalidate_moved() -> bool {
    let keys = windows::keys();
    let mut dirty: Vec<u32> = WATCHED.with(|w| {
        let mut out = Vec::new();
        w.borrow_mut().retain(|&id, watch| {
            let shift = moved(watch.answered, content_box_in(&keys, id), watch.axes);
            if shift {
                out.push(id);
            }
            !shift
        });
        out
    });
    PERCENTAGES.with(|w| {
        for (&id, watch) in w.borrow_mut().iter_mut() {
            if let Some(viewport) = watch.svg_viewport {
                watch.measured = tree::read_tree(|t| crate::svg::viewport_user_size(t, viewport));
            }
            if moved(watch.answered, watch.measured, watch.axes) {
                dirty.push(id);
                watch.answered = watch.measured;
            }
        }
    });
    if dirty.is_empty() {
        return false;
    }
    tree::with_tree(|t| {
        for &id in &dirty {
            if t.node(id).is_some() {
                t.mark_restyle_subtree(id);
            }
        }
    });
    RESTYLED.with(|c| c.set(c.get() + dirty.len() as u64));
    true
}

/// Restyles and lays out again (`draw`) until no watched container moves, after a layout.
pub(crate) fn settle(mut draw: impl FnMut()) {
    if !watching() {
        return;
    }
    for _ in 0..MAX_PASSES {
        if !invalidate_moved() {
            CHECKED_AT.with(|c| c.set(render::layout_count()));
            return;
        }
        crate::flush_changes();
        draw();
        PASSES.with(|c| c.set(c.get() + 1));
    }
    CHECKED_AT.with(|c| c.set(render::layout_count()));
    if !WARNED.with(|w| w.replace(true)) {
        crate::render::warn_compat(
            "container-query-cycle",
            &format!(
                "container queries did not settle within {MAX_PASSES} layouts in one frame: a \
                 query container's size depends on what its own `@container` rules or `cq*` \
                 units change. Size the container on the queried axis independently of its \
                 content (an explicit size, or a flex/grid track that does not depend on it)"
            ),
        );
    }
}

/// The frame's container step, after the pump and before gpui draws: lays the window out now when
/// it will draw anyway, then settles. Nothing runs while no container is watched.
pub fn settle_frame(window: &mut Window, cx: &mut App) {
    if watching() {
        if window.vn_is_dirty() {
            window.draw(cx).clear(cx);
        }
        if CHECKED_AT.with(Cell::get) != render::layout_count() {
            settle(|| {
                window.draw(cx).clear(cx);
            });
        }
    }
    end_frame();
}

fn end_frame() {
    BORN.with(|b| b.borrow_mut().clear());
}

/// Layouts re-run for container queries, and containers that moved, since start.
pub fn counts() -> (u64, u64) {
    (PASSES.with(Cell::get), RESTYLED.with(Cell::get))
}

pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    INSETS.with(|i| i.borrow_mut().retain(|&id, _| live(id)));
    WATCHED.with(|w| w.borrow_mut().retain(|&id, _| live(id)));
    PERCENTAGES.with(|w| w.borrow_mut().retain(|&id, _| live(id)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{animation_overlay, read_tree, restyle_all, set_stylesheet, with_tree};
    use crate::windows;

    const SHEET: &str = ".c { container-type: inline-size; padding: 10px } \
        @container (min-width: 300px) { .leaf { opacity: 0.5 } } \
        .leaf { width: 50cqw; transition: opacity 1s linear }";

    fn lay_out(id: u32, w: f32, h: f32) {
        windows::of(windows::primary(), |s| {
            s.bounds.insert(id, (0.0, 0.0, w, h));
        });
        render::note_build();
    }

    fn style(id: u32) -> (Option<f32>, Option<f32>) {
        read_tree(|t| {
            let cs = t.node(id).and_then(|n| n.computed.as_ref()).unwrap();
            let width = match cs.base.size.width {
                Some(gpui::Length::Definite(gpui::DefiniteLength::Absolute(a))) => {
                    Some(f32::from(a.to_pixels(gpui::px(16.0))))
                }
                _ => None,
            };
            (cs.base.opacity, width)
        })
    }

    /// root > container > leaf, and a bystander beside the container.
    fn card() -> (u32, u32, u32) {
        set_stylesheet(SHEET.into());
        let ids = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let c = t.create_element(div);
            let leaf = t.create_element(div);
            let bystander = t.create_element(div);
            t.set_class(c, "c");
            t.set_class(leaf, "leaf");
            t.insert(c, root, 0);
            t.insert(leaf, c, 0);
            t.insert(bystander, root, 1);
            (c, leaf, bystander)
        });
        restyle_all();
        ids
    }

    #[test]
    fn a_layout_that_moves_the_container_restyles_its_subtree_until_stable() {
        let (c, leaf, bystander) = card();
        assert_eq!(style(leaf).0, None, "before any layout the size is unknown");
        assert!(watching());

        lay_out(c, 420.0, 100.0);
        assert!(invalidate_moved());
        let bystander_rev = read_tree(|t| t.sub_rev(bystander));
        restyle_all();
        assert_eq!(style(leaf), (Some(0.5), Some(200.0)), "a 400px content box");
        assert_eq!(
            read_tree(|t| t.sub_rev(bystander)),
            bystander_rev,
            "outside the container"
        );
        assert!(
            !invalidate_moved(),
            "the layout it answered from is the current one"
        );

        lay_out(c, 250.0, 100.0);
        assert!(invalidate_moved());
        restyle_all();
        assert_eq!(style(leaf), (None, Some(115.0)));
    }

    #[test]
    fn percentage_comparisons_follow_both_axes_of_an_uncontained_parent() {
        let (c, leaf, _) = card();
        set_stylesheet(".c { padding: 10px; --cap: min(200px, 100%) } .leaf { width: var(--cap); height: clamp(12px, 50%, 60px) }".into());
        restyle_all();
        for (w, h, expected) in [
            (260., 180., [200., 60.]),
            (160., 40., [140., 12.]),
            (160., 100., [140., 40.]),
        ] {
            lay_out(c, w, h);
            record_percentage_basis(leaf, || Some((w - 20., h - 20.)));
            invalidate_moved();
            restyle_all();
            let cs = read_tree(|t| t.node(leaf).unwrap().computed.clone().unwrap());
            for (value, basis, expected) in [
                (cs.base.size.width, w - 20., expected[0]),
                (cs.base.size.height, h - 20., expected[1]),
            ] {
                let Some(gpui::Length::Definite(v)) = value else {
                    panic!("expected a used length")
                };
                assert_eq!(
                    v.to_pixels(gpui::px(basis).into(), gpui::px(16.)),
                    gpui::px(expected)
                );
            }
        }
        with_tree(|t| t.set_class(leaf, ""));
        restyle_all();
        assert!(!watching());
    }

    #[test]
    fn svg_shapes_use_the_nearest_viewport_even_without_layout_nodes() {
        set_stylesheet(".shape { width: min(90px, 100%) }".into());
        let (outer, inner, shape) = with_tree(|t| {
            let root = t.create_root();
            let svg = t.intern("svg");
            let g = t.intern("g");
            let rect = t.intern("rect");
            let width = t.intern("width");
            let outer = t.create_element(svg);
            let inner = t.create_element(svg);
            let group = t.create_element(g);
            let shape = t.create_element(rect);
            t.insert(outer, root, 0);
            t.insert(inner, outer, 0);
            t.insert(group, inner, 0);
            t.insert(shape, group, 0);
            t.set_prop(inner, width, Some("50%".into()));
            t.set_class(shape, "shape");
            (outer, inner, shape)
        });
        restyle_all();
        lay_out(outer, 160., 40.);
        assert!(invalidate_moved());
        restyle_all();
        read_tree(|t| {
            assert_eq!(crate::svg::sizing_viewport(t, shape), Some(inner));
            assert_eq!(crate::svg::viewport_user_size(t, inner), Some((80., 40.)));
            let cs = t.node(shape).unwrap().computed.as_ref().unwrap();
            let Some(gpui::Length::Definite(width)) = cs.base.size.width else {
                panic!("expected the shape's computed width");
            };
            assert_eq!(
                width.to_pixels(gpui::px(80.).into(), gpui::px(16.)),
                gpui::px(80.)
            );
        });
        with_tree(|t| {
            let vb = t.intern("viewBox");
            t.set_prop(inner, vb, Some("-4 0 120 10".into()));
        });
        assert!(invalidate_moved());
        restyle_all();
        assert_eq!(style(shape).1, Some(90.));
    }

    #[test]
    fn a_frame_that_moves_no_container_restyles_nothing() {
        let (c, leaf, _) = card();
        lay_out(c, 420.0, 100.0);
        invalidate_moved();
        restyle_all();
        with_tree(|t| t.take_dirty());
        let rev = read_tree(|t| t.sub_rev(leaf));

        lay_out(c, 420.0, 100.0);
        assert!(!invalidate_moved());
        lay_out(c, 420.0, 900.0);
        assert!(!invalidate_moved(), "only the inline axis was asked about");
        assert!(!read_tree(|t| t.is_dirty()));
        assert_eq!(read_tree(|t| t.sub_rev(leaf)), rev);
    }

    #[test]
    fn a_first_style_settled_by_layout_does_not_transition_but_a_resize_does() {
        let (c, leaf, _) = card();
        lay_out(c, 420.0, 100.0);
        invalidate_moved();
        restyle_all();
        let now = std::time::Instant::now();
        let base = read_tree(|t| t.node(leaf).and_then(|n| n.computed.clone()).unwrap().base);
        assert!(
            animation_overlay(leaf, &base, now).is_none(),
            "the element's first frame: no before-change style"
        );

        end_frame();
        lay_out(c, 250.0, 100.0);
        invalidate_moved();
        restyle_all();
        let base = read_tree(|t| t.node(leaf).and_then(|n| n.computed.clone()).unwrap().base);
        assert!(
            animation_overlay(leaf, &base, std::time::Instant::now()).is_some(),
            "a container resize is a style change like any other"
        );
    }

    #[test]
    fn nested_named_containers_each_answer_their_own_queries() {
        set_stylesheet(
            ".outer { container: page / inline-size } .inner { container: card / inline-size } \
             @container page (min-width: 600px) { .leaf { opacity: 0.25 } } \
             @container card (min-width: 200px) { .leaf { flex-grow: 2 } }"
                .into(),
        );
        let (outer, inner, leaf) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let outer = t.create_element(div);
            let inner = t.create_element(div);
            let leaf = t.create_element(div);
            t.set_class(outer, "outer");
            t.set_class(inner, "inner");
            t.set_class(leaf, "leaf");
            t.insert(outer, root, 0);
            t.insert(inner, outer, 0);
            t.insert(leaf, inner, 0);
            (outer, inner, leaf)
        });
        restyle_all();
        lay_out(outer, 800.0, 100.0);
        lay_out(inner, 150.0, 100.0);
        assert!(invalidate_moved());
        restyle_all();
        let grow = || {
            read_tree(|t| {
                t.node(leaf)
                    .and_then(|n| n.computed.as_ref())
                    .unwrap()
                    .base
                    .flex_grow
            })
        };
        assert_eq!(style(leaf).0, Some(0.25));
        assert_eq!(grow(), None);

        lay_out(inner, 250.0, 100.0);
        assert!(invalidate_moved());
        restyle_all();
        assert_eq!((style(leaf).0, grow()), (Some(0.25), Some(2.0)));
    }

    #[test]
    fn settling_stops_at_the_cap_on_a_cycle() {
        let (c, _, _) = card();
        let mut width = 300.0;
        let mut draws = 0;
        lay_out(c, width, 100.0);
        settle(|| {
            draws += 1;
            width += 10.0;
            lay_out(c, width, 100.0);
        });
        assert_eq!(draws, MAX_PASSES);
    }
}
