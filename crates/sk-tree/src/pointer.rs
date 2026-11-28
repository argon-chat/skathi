//! Which element a mouse event is FOR: the topmost one under the pointer.
//!
//! A browser hit-tests once; gpui hands the event to EVERY hitbox under the pointer, and only
//! interactive elements have one, so a box that listened for nothing let a press fall through to
//! whatever listened beneath it (a menu over its dismissal shield closed on its own items). Here:
//!
//! - gpui records a pick for every element without a hitbox (`Window::set_vn_pick_every_box`);
//! - the target is resolved once per press, release and wheel, before any listener runs ([`refine`]):
//!   the topmost tagged box whose element is not `pointer-events: none` or under `visibility: hidden`;
//! - only the target's path hears it (the refiner narrows gpui's hovered set), and a `click` goes to
//!   the nearest common ancestor of press and release targets ([`click_target`]).
//!
//! `mousemove` is not refined (too frequent): its one dispatch per frame resolves its own target
//! ([`target_at`]); hover is resolved the same way once a frame ([`resolve_hover`]).

use gpui::{App, DispatchPhase, MouseMoveEvent, Pixels, Point, VnHitKind, Window};
use sk_runtime::redjs::api::Value;
use sk_runtime::{host_fn, JsRuntime};

use crate::tree::{self, Kind, Tree};
use crate::windows;

const PRESS_BITS: u64 = (1 << tree::EV_CLICK)
    | (1 << tree::EV_MOUSEDOWN)
    | (1 << tree::EV_MOUSEUP)
    | (1 << tree::EV_CONTEXTMENU)
    | (1 << tree::EV_AUXCLICK);

/// Turns the targeting on for this window (picks for every element, and the refiner); called once per build.
pub(crate) fn arm(window: &mut Window) {
    if !window.vn_picks_every_box() {
        window.set_vn_pick_every_box(true);
        window.set_external_drop_handler(crate::file_drop::handle);
    }
    if !window.has_vn_hit_refiner() {
        window.set_vn_hit_refiner(Some(std::rc::Rc::new(refine)));
    }
    if !window.has_vn_touch_handler() {
        window.set_vn_touch_handler(Some(std::rc::Rc::new(crate::touch::handle)));
    }
}

fn window_root(t: &Tree, window: &Window) -> u32 {
    match windows::root_of(windows::key(window)) {
        0 => t.root,
        popped => popped,
    }
}

/// `id` and its ancestors up to `root`, target first; `None` when `id` cannot be targeted in this
/// window (gone, outside `root`, `pointer-events: none`, or inside `visibility: hidden`, which gpui
/// paints none of).
fn path_if_hittable(t: &Tree, id: u32, root: u32) -> Option<Vec<u32>> {
    let node = t.node(id)?;
    if node.kind != Kind::Element {
        return None;
    }
    let inert = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
    if inert && id != root {
        return None;
    }
    let mut path = Vec::with_capacity(16);
    let mut cur = id;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        if n.computed
            .as_ref()
            .is_some_and(|cs| cs.base.visibility == Some(gpui::Visibility::Hidden))
        {
            return None;
        }
        path.push(cur);
        if cur == root {
            return Some(path);
        }
        if n.parent == 0 {
            return None;
        }
        cur = n.parent;
    }
    None
}

struct Resolved {
    target: u32,
    path: Vec<u32>,
    tags: Vec<u32>,
    at: usize,
}

fn resolve(t: &Tree, tags: Vec<u32>, root: u32) -> Resolved {
    for (at, &tag) in tags.iter().enumerate() {
        if let Some(path) = path_if_hittable(t, tag, root) {
            return Resolved {
                target: tag,
                path,
                tags,
                at,
            };
        }
    }
    let at = tags.len();
    Resolved {
        target: root,
        path: vec![root],
        tags,
        at,
    }
}

pub(crate) fn topmost(window: &Window, p: Point<Pixels>) -> Option<u32> {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        tags.into_iter()
            .find(|&tag| path_if_hittable(t, tag, root).is_some())
    })
}

pub(crate) fn target_at(window: &Window, p: Point<Pixels>) -> u32 {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        resolve(t, tags, root).target
    })
}

/// The explicit capture target, while it still belongs to this window's tree. CSS hit-testing does
/// not revoke capture; removal or moving to another window does.
pub(crate) fn captured_target(window: &Window) -> Option<u32> {
    let id = windows::here(window, |w| w.pointer_capture)?;
    let valid = tree::read_tree(|t| {
        t.node(id).is_some_and(|n| n.kind == Kind::Element)
            && windows::window_of_node_in(t, id) == windows::key(window)
            && path_to(t, id, window_root(t, window))
    });
    if !valid {
        windows::here(window, |w| w.pointer_capture = None);
    }
    valid.then_some(id)
}

fn capture_path(t: &Tree, id: u32, root: u32) -> Option<Vec<u32>> {
    if t.node(id)?.kind != Kind::Element {
        return None;
    }
    let mut path = Vec::with_capacity(16);
    let mut cur = id;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        path.push(cur);
        if cur == root {
            return Some(path);
        }
        cur = n.parent;
    }
    None
}

/// The capture or the pointer lock's element: where mouse events go wherever the pointer is.
pub(crate) fn routed_target(window: &Window) -> Option<u32> {
    crate::pointer_lock::target_in(window).or_else(|| captured_target(window))
}

/// gpui element moves only run while hovered. The OS captures the window, so a raw window listener
/// must collect moves beyond the captured box and beyond the client area as well. Every move's
/// motion adds up here until a `mousemove` reports it.
pub(crate) fn capture_listeners(window: &mut Window) {
    window.on_mouse_event(|ev: &gpui::MouseDownEvent, phase, window, _| {
        if phase == DispatchPhase::Capture {
            crate::touch::note_mouse_event(window);
            if ev.button == gpui::MouseButton::Left {
                crate::pen::mouse_button(window, window.vn_pen().is_none());
            }
            crate::pointer_lock::note_gesture();
            crate::pen::end_stroke(window);
        }
    });
    window.on_mouse_event(|ev: &gpui::MouseUpEvent, phase, window, _| {
        if phase == DispatchPhase::Capture {
            crate::touch::note_mouse_event(window);
            if ev.button == gpui::MouseButton::Left {
                crate::pen::mouse_button(window, false);
            }
            crate::pen::end_stroke(window);
        }
    });
    window.on_mouse_event(|ev: &gpui::MouseExitEvent, phase, window, _| {
        if phase == DispatchPhase::Capture {
            if ev.pressed_button != Some(gpui::MouseButton::Left) {
                crate::pen::mouse_button(window, false);
            }
            crate::pen::end_stroke(window);
        }
    });
    window.on_mouse_event(|ev: &gpui::MousePressureEvent, phase, window, _| {
        if phase != DispatchPhase::Capture || !crate::pen::note_pressure(window, ev) {
            return;
        }
        let id = routed_target(window).unwrap_or_else(|| target_at(window, ev.position));
        windows::here(window, |w| {
            w.pending_mouse_move = Some(windows::PendingMouseMove {
                id,
                position: ev.position,
                button: 0.0,
                modifiers: ev.modifiers,
            })
        });
        sk_runtime::frames::wake();
        window.refresh();
    });
    window.on_mouse_event(|ev: &MouseMoveEvent, phase, window, _cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        crate::touch::note_mouse_event(window);
        let moved = window.vn_mouse_movement();
        windows::here(window, |w| w.mouse_movement = w.mouse_movement + moved);
        crate::pen::note_move(window, ev.pressed_button);
        let Some(id) = routed_target(window) else {
            return;
        };
        windows::here(window, |w| {
            w.pending_mouse_move = Some(windows::PendingMouseMove {
                id,
                position: ev.position,
                button: match ev.pressed_button {
                    Some(gpui::MouseButton::Left) => 0.0,
                    Some(gpui::MouseButton::Middle) => 1.0,
                    Some(gpui::MouseButton::Right) => 2.0,
                    Some(gpui::MouseButton::Navigate(_)) => 3.0,
                    None => -1.0,
                },
                modifiers: ev.modifiers,
            });
        });
    });
}

fn refine(kind: VnHitKind, p: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let capture = crate::pointer_lock::target_in(window).or_else(|| {
        (kind != VnHitKind::Scroll)
            .then(|| captured_target(window))
            .flatten()
    });
    if capture.is_none()
        && (crate::select::refine(kind, p, window, cx)
            || crate::color_picker::refine(kind, p, window, cx))
    {
        return;
    }
    let tags = window.vn_tags_at(p);
    let resolved = tree::read_tree(|t| {
        let root = window_root(t, window);
        let mut r = resolve(t, tags, root);
        if let Some(id) = capture {
            if let Some(path) = capture_path(t, id, root) {
                r.target = id;
                r.path = path;
            }
        }
        if kind == VnHitKind::Down {
            note_covered(t, &r, p);
        }
        r
    });
    let target = resolved.target;
    match kind {
        VnHitKind::Down => crate::live_state::set_active(window, Some(target)),
        VnHitKind::Up => crate::live_state::set_active(window, None),
        VnHitKind::Scroll => {}
    }
    windows::here(window, |w| {
        w.hit = Some(target);
        if kind == VnHitKind::Down {
            w.down_hit = Some(target);
        }
    });
    let mut path = resolved.path;
    path.sort_unstable();
    window.vn_restrict_mouse_hit(
        &|tag| path.binary_search(&tag).is_ok(),
        kind != VnHitKind::Scroll,
    );
}

pub(crate) fn target_or(window: &Window, fallback: u32) -> u32 {
    windows::here(window, |w| w.hit).unwrap_or(fallback)
}

/// Where a `click` goes: the nearest common inclusive ancestor of the original DOM press and the
/// current release. A removed/detached press or an actually-disabled control cannot be activated by
/// the release.
pub(crate) fn click_target(window: &Window, down: u32, fallback: u32) -> Option<u32> {
    let capture = routed_target(window);
    let up = windows::here(window, |w| w.hit).unwrap_or(fallback);
    tree::read_tree(|t| click_target_in_tree(t, down, up, window_root(t, window), capture))
}

fn click_target_in_tree(
    t: &Tree,
    down: u32,
    up: u32,
    root: u32,
    capture: Option<u32>,
) -> Option<u32> {
    enabled_path(t, down, root)?;
    let up = capture.unwrap_or(up);
    enabled_path(t, up, root)?;
    capture.or_else(|| common_ancestor(t, down, up))
}

pub(crate) fn enabled_path(t: &Tree, id: u32, root: u32) -> Option<()> {
    let path = capture_path(t, id, root)?;
    // A fieldset itself must not suppress the enabled first legend.
    (!path.iter().any(|&id| {
        t.node(id)
            .is_some_and(|n| t.name(n.tag) != "fieldset" && crate::render::actually_disabled(t, id))
    }))
    .then_some(())
}

fn common_ancestor(t: &Tree, a: u32, b: u32) -> Option<u32> {
    let mut seen = Vec::with_capacity(16);
    let mut cur = a;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        seen.push(cur);
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    let mut cur = b;
    for _ in 0..4096 {
        if seen.contains(&cur) {
            return Some(cur);
        }
        let n = t.node(cur)?;
        if n.parent == 0 {
            return None;
        }
        cur = n.parent;
    }
    None
}

fn hit_ids(window: &Window, p: Point<Pixels>, all: bool) -> Vec<u32> {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        let inside = windows::peek(windows::key(window), |w| w.bounds.get(&root).copied())
            .flatten()
            .is_some_and(|(x, y, w, h)| {
                let (px, py) = (f32::from(p.x), f32::from(p.y));
                px >= x && px < x + w && py >= y && py < y + h
            });
        if !inside {
            return Vec::new();
        }
        if !all {
            return vec![resolve(t, tags, root).target];
        }
        let mut out: Vec<u32> = Vec::new();
        for tag in tags {
            if !out.contains(&tag) && path_if_hittable(t, tag, root).is_some() {
                out.push(tag);
            }
        }
        if !out.contains(&root) {
            out.push(root);
        }
        out
    })
}

fn hit_ids_in_primary(p: Point<Pixels>, all: bool) -> Vec<u32> {
    let primary = windows::primary();
    let in_hand = sk_runtime::with_commit(|window, cx| {
        if windows::key(window) == primary {
            Some(hit_ids(window, p, all))
        } else {
            other_window(cx, primary, p, all)
        }
    });
    match in_hand {
        Some(Some(ids)) => ids,
        Some(None) => Vec::new(),
        None => sk_runtime::with_app(|cx| other_window(cx, primary, p, all))
            .flatten()
            .unwrap_or_default(),
    }
}

fn other_window(
    cx: &mut App,
    key: windows::WinKey,
    p: Point<Pixels>,
    all: bool,
) -> Option<Vec<u32>> {
    let handle = cx
        .windows()
        .into_iter()
        .find(|h| h.window_id().as_u64() == key)?;
    handle
        .update(cx, |_, window, _| hit_ids(window, p, all))
        .ok()
}

host_fn!(fn sk_hit_at(_g, args) {
    let n = |v: Value| v.as_number().unwrap_or(f64::NAN);
    let (x, y) = (n(args.arg(0)), n(args.arg(1)));
    let all = n(args.arg(2)) == 1.0;
    let ids = if x.is_finite() && y.is_finite() {
        hit_ids_in_primary(gpui::point(gpui::px(x as f32), gpui::px(y as f32)), all)
    } else {
        Vec::new()
    };
    let s = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    JsRuntime::with(|rt| rt.ctx().string(&s).0)
});

/// The runtime's `pointerId`s: the mouse, and any pen.
const MOUSE_ID: f64 = 1.0;
const PEN_ID: f64 = 2.0;

// `(node, pointerId, capture)` — the JS mirror validates the active pointer; ownership is per
// native window and is cleared even outside a commit wave (unmount and app teardown do not
// necessarily run in an input callback).
host_fn!(fn sk_pointer_capture(_g, args) {
    let id = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let pointer = args.arg(1).as_number().unwrap_or(0.0);
    let capture = args.arg(2).as_bool().unwrap_or(false);
    let done = if pointer == MOUSE_ID || pointer == PEN_ID {
        capture_mouse(id, capture)
    } else if pointer.fract() == 0.0 && pointer > 0.0 && pointer <= f64::from(u32::MAX) {
        crate::touch::capture(id, pointer as u32, capture)
    } else {
        false
    };
    Value::boolean(done).0
});

/// Captures the pointer gpui's mouse events stand for (the mouse, a pen, the primary finger) to
/// `id`, or releases it.
pub(crate) fn capture_mouse(id: u32, capture: bool) -> bool {
    if !capture {
        windows::each(|_, w| {
            if w.pointer_capture == Some(id) {
                w.pointer_capture = None;
            }
        });
        return true;
    }
    let k = windows::window_of_node(id);
    // A script cannot transfer client-space input into another native window.
    if !windows::peek(k, |w| {
        w.press
            .as_ref()
            .is_some_and(crate::render::Press::is_active)
    })
    .unwrap_or(false)
    {
        return false;
    }
    let valid = capturable_in(k, id);
    if valid {
        windows::each(|_, w| w.pointer_capture = None);
        windows::of(k, |w| w.pointer_capture = Some(id));
    }
    valid
}

/// Can `id` hold a capture in window `k`: an element painted there.
pub(crate) fn capturable_in(k: windows::WinKey, id: u32) -> bool {
    tree::read_tree(|t| {
        let root = match windows::root_of(k) {
            0 => t.root,
            popped => popped,
        };
        capture_path(t, id, root).is_some()
    })
}

pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__sk_hit_at", 3, sk_hit_at as *const std::ffi::c_void);
    rt.register(
        "__sk_pointer_capture",
        3,
        sk_pointer_capture as *const std::ffi::c_void,
    );
}

thread_local! {
    static NAMED: std::cell::RefCell<std::collections::HashSet<(u32, u32)>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

fn note_covered(t: &Tree, r: &Resolved, p: Point<Pixels>) {
    let mut listener = None;
    for &tag in r.tags.iter().skip(r.at + 1) {
        if r.path.contains(&tag) {
            continue;
        }
        let Some(node) = t.node(tag) else { continue };
        if node.kind != Kind::Element {
            continue;
        }
        let inert = node
            .computed
            .as_ref()
            .is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
        if !inert && node.listeners & PRESS_BITS != 0 {
            listener = Some(tag);
            break;
        }
    }
    let Some(l) = listener else { return };
    let fresh = NAMED.with(|n| {
        let mut n = n.borrow_mut();
        if n.len() > 256 {
            n.clear();
        }
        n.insert((r.target, l))
    });
    if fresh {
        crate::log::record(
            "dev",
            format!(
                "[hit] a press at ({:.0}, {:.0}) is for {}, painted over {} — which listens for it and, \
                 as in a browser, does not get it (the cover is not one of its descendants)",
                f32::from(p.x),
                f32::from(p.y),
                describe(t, r.target),
                describe(t, l),
            ),
        );
    }
}

#[derive(Default)]
pub struct Hover {
    path: Vec<u32>,
    from: Option<(Point<Pixels>, bool, u64)>,
    when: Option<std::time::Instant>,
    ticking: bool,
}

/// How often a page that keeps building frames under a still pointer has its hover resolved again.
const STILL_POINTER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

pub(crate) fn resolve_hover(window: &mut Window, cx: &mut App) {
    let pos = window.mouse_position();
    if crate::select::covers(window, pos) || crate::color_picker::covers(window, pos) {
        return;
    }
    let inside = window.vn_pointer_inside();
    let built = crate::render::layout_count();
    let now = std::time::Instant::now();
    let due = windows::here(window, |w| match w.hover.from {
        None => true,
        Some((p, i, _)) if p != pos || i != inside => true,
        Some((_, _, b)) if b != built => w
            .hover
            .when
            .is_none_or(|t| now.duration_since(t) >= STILL_POINTER_INTERVAL),
        Some(_) => false,
    });
    if !due {
        return;
    }
    let path = if inside {
        path_at(window, pos)
    } else {
        Vec::new()
    };
    let old = windows::here(window, |w| {
        w.hover.from = Some((pos, inside, built));
        w.hover.when = Some(now);
        (w.hover.path != path).then(|| std::mem::replace(&mut w.hover.path, path.clone()))
    });
    if let Some(old) = old {
        hover_moved(&old, &path, pos, -1.0, true, window, cx);
    }
}

/// The target at `p` and its ancestors up to the window's root, target first.
pub(crate) fn path_at(window: &Window, p: Point<Pixels>) -> Vec<u32> {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| match window_root(t, window) {
        0 => Vec::new(),
        root => resolve(t, tags, root).path,
    })
}

/// `id` and its ancestors up to the window's root, target first; empty when it is not in it.
pub(crate) fn path_of(window: &Window, id: u32) -> Vec<u32> {
    tree::read_tree(|t| capture_path(t, id, window_root(t, window))).unwrap_or_default()
}

/// The hover moves to `pos` now, not at the next frame: a pointer that cannot hover (a finger)
/// gets its over and enter events before its press.
pub(crate) fn hover_now(window: &mut Window, cx: &mut App, pos: Point<Pixels>) {
    let path = path_at(window, pos);
    let built = crate::render::layout_count();
    let old = windows::here(window, |w| {
        w.hover.from = Some((pos, true, built));
        w.hover.when = Some(std::time::Instant::now());
        (w.hover.path != path).then(|| std::mem::replace(&mut w.hover.path, path.clone()))
    });
    if let Some(old) = old {
        hover_moved(&old, &path, pos, -1.0, true, window, cx);
    }
}

/// The boundary events of a pointer that is not the mouse's (a second finger): `old` to `new`
/// without touching `:hover`, which follows the mouse and the primary pointer.
pub(crate) fn boundary_events(
    old: &[u32],
    new: &[u32],
    pos: Point<Pixels>,
    button: f64,
    window: &mut Window,
    cx: &mut App,
) {
    if old != new {
        hover_moved(old, new, pos, button, false, window, cx);
    }
}

fn boundary(old: &[u32], new: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let left = old.iter().copied().filter(|id| !new.contains(id)).collect();
    let entered = new
        .iter()
        .rev()
        .copied()
        .filter(|id| !old.contains(id))
        .collect();
    (left, entered)
}

fn hover_moved(
    old: &[u32],
    new: &[u32],
    pos: Point<Pixels>,
    button: f64,
    styles: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let (left, entered) = boundary(old, new);
    if styles {
        // Before any handler: a `mouseenter` listener that restyles must find the transition
        // already aimed at its target.
        for &id in &left {
            tree::note_hover(id, false);
        }
        for &id in &entered {
            tree::note_hover(id, true);
        }
        let restyled = tree::with_tree(|t| {
            let mut any = false;
            for &id in left.iter().chain(&entered) {
                any |= t.touch_hover_styled(id);
            }
            any
        });
        if restyled {
            crate::render::request_repaint(window, cx);
        }
    }
    // `mouseenter` and `mouseleave` fire at every element that joins or leaves the path and listens
    // for them, `pointer-events` notwithstanding: it decides what is hit, not who hears about it.
    let has = |t: &Tree, id: u32, ev: u32| {
        t.node(id)
            .is_some_and(|n| (n.listeners | crate::haptics::mask(n)) & (1u64 << ev) != 0)
    };
    // `mouseover`/`mouseout` bubble, so anything on the path may be the one listening.
    let (from, to) = (old.first().copied(), new.first().copied());
    let (leaves, enters, out_at, over_at) = tree::read_tree(|t| {
        let root = window_root(t, window);
        let heard = |path: &[u32], ev: u32| path.iter().any(|&id| has(t, id, ev));
        let leaves: Vec<u32> = left
            .iter()
            .copied()
            .filter(|&id| has(t, id, tree::EV_MOUSELEAVE) && path_to(t, id, root))
            .collect();
        let enters: Vec<u32> = entered
            .iter()
            .copied()
            .filter(|&id| has(t, id, tree::EV_MOUSEENTER))
            .collect();
        let moved = from != to;
        let out_at =
            from.filter(|&id| moved && heard(old, tree::EV_MOUSEOUT) && path_to(t, id, root));
        let over_at = to.filter(|_| moved && heard(new, tree::EV_MOUSEOVER));
        (leaves, enters, out_at, over_at)
    });
    if leaves.is_empty() && enters.is_empty() && out_at.is_none() && over_at.is_none() {
        return;
    }
    let mods = window.modifiers();
    let payload = crate::render::mouse_payload(pos, button, &mods);
    let related = |id: Option<u32>| id.map(|id| id.to_string()).unwrap_or_default();
    let (to_key, from_key) = (related(to), related(from));
    let with_key = |key| crate::EventPayload {
        key: Some(key),
        ..crate::render::mouse_payload(pos, button, &mods)
    };
    let (out_payload, over_payload) = (with_key(to_key.as_str()), with_key(from_key.as_str()));
    // Pointer Events order: out, leaves, over, enters.
    let events = out_at
        .map(|id| (id, tree::EV_MOUSEOUT, &out_payload))
        .into_iter()
        .chain(
            leaves
                .into_iter()
                .map(|id| (id, tree::EV_MOUSELEAVE, &payload)),
        )
        .chain(over_at.map(|id| (id, tree::EV_MOUSEOVER, &over_payload)))
        .chain(
            enters
                .into_iter()
                .map(|id| (id, tree::EV_MOUSEENTER, &payload)),
        );
    crate::off_input(|| {
        for (id, ev, payload) in events {
            if tree::read_tree(|t| t.node(id).is_some()) {
                crate::render::js_dispatch_app(id, ev, payload, window, cx);
            }
        }
    });
}

fn path_to(t: &Tree, id: u32, root: u32) -> bool {
    let mut cur = id;
    for _ in 0..4096 {
        if cur == root {
            return true;
        }
        match t.node(cur) {
            Some(n) if n.parent != 0 => cur = n.parent,
            _ => return false,
        }
    }
    false
}

/// A pop-out has no frame pump (sk-app's drives the primary window), so its hover and coalesced
/// `mousemove` are delivered from a per-frame callback armed here once, from its build.
pub(crate) fn arm_hover_tick(window: &mut Window) {
    let k = windows::key(window);
    if windows::is_primary(k)
        || windows::here(window, |w| std::mem::replace(&mut w.hover.ticking, true))
    {
        return;
    }
    window.on_next_frame(hover_tick);
}

fn hover_tick(window: &mut Window, cx: &mut App) {
    crate::render::flush_pending_mouse_move(window, cx);
    window.on_next_frame(hover_tick);
}

fn describe(t: &Tree, id: u32) -> String {
    let Some(node) = t.node(id) else {
        return format!("node {id}");
    };
    let mut out = format!("node {id} (<{}", t.name(node.tag));
    if let Some(el_id) = t.prop(id, "id").filter(|v| !v.is_empty()) {
        out.push_str(&format!(" id=\"{el_id}\""));
    }
    let classes: Vec<&str> = node.classes.iter().map(String::as_str).take(3).collect();
    if !classes.is_empty() {
        out.push_str(&format!(" class=\"{}\"", classes.join(" ")));
    }
    out.push_str(">)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_of(build: impl FnOnce(&mut Tree) -> Vec<u32>) -> Vec<u32> {
        tree::with_tree(build)
    }

    fn set(t: &mut Tree, id: u32, pointer_none: Option<bool>, hidden: bool) {
        let mut cs = sk_css::ComputedStates::default();
        cs.extras.pointer_events_none = pointer_none;
        if hidden {
            let mut base = gpui::StyleRefinement::default();
            base.visibility = Some(gpui::Visibility::Hidden);
            cs.base = std::rc::Rc::new(base);
        }
        t.node_mut(id).unwrap().computed = Some(cs);
    }

    #[test]
    fn the_common_ancestor_of_a_press_and_its_release() {
        let ids = tree_of(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let list = t.create_element(div);
            let a = t.create_element(div);
            let b = t.create_element(div);
            let a1 = t.create_element(div);
            t.insert(list, root, 0);
            t.insert(a, list, 0);
            t.insert(b, list, 1);
            t.insert(a1, a, 0);
            vec![root, list, a, b, a1]
        });
        let [root, list, a, b, a1] = ids[..] else {
            unreachable!()
        };
        tree::read_tree(|t| {
            assert_eq!(common_ancestor(t, a1, a1), Some(a1), "the same element");
            assert_eq!(
                common_ancestor(t, a1, a),
                Some(a),
                "an ancestor of the other"
            );
            assert_eq!(common_ancestor(t, a1, b), Some(list), "two branches");
            assert_eq!(common_ancestor(t, root, b), Some(root));
        });
    }

    #[test]
    fn dom_click_pairing_rejects_removed_or_other_window_presses_and_keeps_capture() {
        tree_of(|t| {
            let root = t.create_root();
            let other_window = t.create_root();
            let div = t.intern("div");
            let parent = t.create_element(div);
            let down = t.create_element(div);
            let up = t.create_element(div);
            t.insert(parent, root, 0);
            t.insert(down, parent, 0);
            t.insert(up, parent, 1);
            assert_eq!(click_target_in_tree(t, down, up, root, None), Some(parent));
            assert_eq!(
                click_target_in_tree(t, down, up, root, Some(down)),
                Some(down)
            );
            assert_eq!(click_target_in_tree(t, down, up, other_window, None), None);
            t.insert(down, other_window, 0);
            assert_eq!(
                click_target_in_tree(t, down, up, root, None),
                None,
                "detached from the original window"
            );
            t.remove(down);
            assert_eq!(
                click_target_in_tree(t, down, up, root, None),
                None,
                "removed press must not retarget to a listener"
            );
            vec![root, other_window, parent, up]
        });
    }

    #[test]
    fn disabled_dom_clicks_include_button_children_but_preserve_the_first_legend() {
        tree_of(|t| {
            let root = t.create_root();
            let button_tag = t.intern("button");
            let span_tag = t.intern("span");
            let fieldset_tag = t.intern("fieldset");
            let legend_tag = t.intern("legend");
            let button = t.create_element(button_tag);
            let child = t.create_element(span_tag);
            t.insert(button, root, 0);
            t.insert(child, button, 0);
            assert_eq!(
                click_target_in_tree(t, child, child, root, None),
                Some(child)
            );
            let disabled = t.intern("disabled");
            t.set_prop(button, disabled, Some(String::new()));
            assert_eq!(click_target_in_tree(t, child, child, root, None), None);
            assert_eq!(
                click_target_in_tree(t, root, child, root, None),
                None,
                "disabled release target"
            );
            t.set_prop(button, disabled, None);
            let fieldset = t.create_element(fieldset_tag);
            let legend = t.create_element(legend_tag);
            let enabled = t.create_element(button_tag);
            let disabled_button = t.create_element(button_tag);
            t.insert(fieldset, root, 1);
            t.insert(legend, fieldset, 0);
            t.insert(enabled, legend, 0);
            t.insert(disabled_button, fieldset, 1);
            t.set_prop(fieldset, disabled, Some(String::new()));
            assert_eq!(
                click_target_in_tree(t, enabled, enabled, root, None),
                Some(enabled)
            );
            assert_eq!(
                click_target_in_tree(t, disabled_button, disabled_button, root, None),
                None
            );
            vec![
                root,
                button,
                child,
                fieldset,
                legend,
                enabled,
                disabled_button,
            ]
        });
    }

    #[test]
    fn the_boundary_between_two_hover_paths() {
        let (left, entered) = boundary(&[5, 3, 2, 1], &[6, 4, 2, 1]);
        assert_eq!(left, vec![5, 3]);
        assert_eq!(entered, vec![4, 6]);
        assert_eq!(boundary(&[3, 2, 1], &[5, 3, 2, 1]), (vec![], vec![5]));
        assert_eq!(boundary(&[3, 2, 1], &[]), (vec![3, 2, 1], vec![]));
        assert_eq!(boundary(&[], &[3, 2, 1]), (vec![], vec![1, 2, 3]));
    }

    #[test]
    fn capture_keeps_its_path_outside_hit_testing_but_not_after_removal() {
        tree_of(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let parent = t.create_element(div);
            let child = t.create_element(div);
            let other = t.create_element(div);
            t.insert(parent, root, 0);
            t.insert(child, parent, 0);
            t.insert(other, root, 1);
            set(t, parent, Some(true), true);
            assert!(path_if_hittable(t, child, root).is_none());
            assert_eq!(
                capture_path(t, child, root),
                Some(vec![child, parent, root])
            );
            assert_eq!(
                capture_path(t, child, other),
                None,
                "capture never crosses windows"
            );
            t.insert(child, other, 0);
            assert_eq!(
                capture_path(t, child, parent),
                None,
                "detached from old painted root"
            );
            vec![root, parent, child, other]
        });
    }

    #[test]
    fn pointer_events_none_and_hidden_subtrees_are_not_targets() {
        let ids = tree_of(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let decor = t.create_element(div);
            let inside = t.create_element(div);
            let hidden = t.create_element(div);
            let under_hidden = t.create_element(div);
            let plain = t.create_element(div);
            t.insert(decor, root, 0);
            t.insert(inside, decor, 0);
            t.insert(hidden, root, 1);
            t.insert(under_hidden, hidden, 0);
            t.insert(plain, root, 2);
            set(t, decor, Some(true), false);
            set(t, inside, Some(false), false);
            set(t, hidden, None, true);
            set(t, under_hidden, None, false);
            vec![root, decor, inside, hidden, under_hidden, plain]
        });
        let [root, decor, inside, hidden, under_hidden, plain] = ids[..] else {
            unreachable!()
        };
        tree::read_tree(|t| {
            assert!(
                path_if_hittable(t, decor, root).is_none(),
                "pointer-events: none"
            );
            assert_eq!(
                path_if_hittable(t, inside, root),
                Some(vec![inside, decor, root]),
                "auto under none is hit, and its path runs through the none parent"
            );
            assert!(
                path_if_hittable(t, hidden, root).is_none(),
                "visibility: hidden"
            );
            assert!(
                path_if_hittable(t, under_hidden, root).is_none(),
                "inside a hidden subtree"
            );
            assert_eq!(path_if_hittable(t, plain, root), Some(vec![plain, root]));
            let r = resolve(t, vec![decor, under_hidden, plain, root], root);
            assert_eq!((r.target, r.at), (plain, 2));
            let r = resolve(t, vec![decor], root);
            assert_eq!(r.target, root, "nothing hittable: the root");
            assert!(path_if_hittable(t, plain, decor).is_none());
        });
    }
}
