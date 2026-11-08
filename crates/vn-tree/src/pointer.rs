//! Which element a mouse event is FOR: the topmost one under the pointer.
//!
//! A browser hit-tests once — the topmost element under the pointer that
//! `pointer-events` lets be hit — and the event goes to it and bubbles through
//! its ancestors. gpui instead hands a mouse event to EVERY hitbox under the
//! pointer, and gpui gives a hitbox only to an element that is interactive.
//! Together that made the rule "the innermost box that LISTENS takes it": a
//! box that listens for nothing let a press fall through to whatever listening
//! box lay under it, a sibling or an unrelated overlay included, and
//! `e.target` was the listening box, never the element pressed (E-2 in
//! docs/RUNTIME.md, now retired). A context menu over its dismissal shield
//! closed on its own items' presses (research/reports/m26/text-selection-1.md
//! step 0); a `@click.self` backdrop closed on a click at the picture in it.
//!
//! The browser's rule, in three parts:
//!
//! * **Every box can be hit.** gpui records a PICK for each element that gets
//!   no hitbox (`Window::set_vn_pick_every_box`, fork item 39): its box,
//!   content mask, transform and node tag, in the tagged list only. A pick is
//!   one small record per element per painted frame, replayed with a cached
//!   view's prepaint, and gpui's per-move hit test never looks at it. (A host
//!   with devtools gives every element a real hitbox anyway, for the
//!   inspector's `pick`; the picks are what a `no-devtools` build has.)
//! * **The target is resolved once per press, release and wheel**, before any
//!   listener runs ([`refine`], the fork's hit refiner): the topmost tagged box
//!   under the pointer ([`gpui::Window::vn_tags_at`], in paint order) whose
//!   element is not `pointer-events: none` and not inside a
//!   `visibility: hidden` subtree. Nothing hittable: the window's root.
//! * **Only the target's path hears it.** The refiner narrows gpui's hovered
//!   set for the event to the hitboxes of the target and its ancestors, and
//!   adds the ancestors' boxes that do not contain the pointer. Everything
//!   downstream keys off that set, so it all follows at once: the listeners
//!   [`crate::render`] binds per element, a text field's own caret placement,
//!   gpui's focus-on-press, the click tracking, the scroll containers a wheel
//!   moves. The renderer then dispatches the DOM event AT the target
//!   ([`target_or`]); a `click` goes to the nearest common ancestor of the
//!   press's target and the release's ([`click_target`]), as in Chromium.
//!
//! `document.elementFromPoint` / `elementsFromPoint` ask the same question
//! from script (`__vn_hit_at`), of the primary window's last painted frame.
//!
//! `mousemove` is not refined (moves are frequent, and resolving one is a walk
//! over every box): its one dispatch per frame resolves its own target
//! ([`target_at`]). **Hover** is resolved the same way, once a frame
//! ([`resolve_hover`]): the path of the topmost element under the pointer is
//! what `:hover` and `group-hover:` match, and the elements joining or leaving
//! it are what `mouseenter` and `mouseleave` fire at. `:active` is gpui's
//! ladder on the press's refined set, so it is the press target's path too.

use gpui::{App, DispatchPhase, MouseMoveEvent, Pixels, Point, VnHitKind, Window};
use vn_runtime::redjs::api::Value;
use vn_runtime::{host_fn, JsRuntime};

use crate::tree::{self, Kind, Tree};
use crate::windows;

/// The pointer events a covered listener would have taken: what the dev log
/// line names.
const PRESS_BITS: u64 = (1 << tree::EV_CLICK)
    | (1 << tree::EV_MOUSEDOWN)
    | (1 << tree::EV_MOUSEUP)
    | (1 << tree::EV_CONTEXTMENU)
    | (1 << tree::EV_AUXCLICK);

/// Turns the targeting on for this window: picks for every element, and the
/// refiner. Called once per build (`render::arm_pick_hitboxes`); both are
/// window flags and cost one read when already set.
pub(crate) fn arm(window: &mut Window) {
    if !window.vn_picks_every_box() {
        window.set_vn_pick_every_box(true);
    }
    if !window.has_vn_hit_refiner() {
        window.set_vn_hit_refiner(Some(std::rc::Rc::new(refine)));
    }
}

/// The node a window paints as its root: the document root in the primary
/// window, the popped-out node in a pop-out.
fn window_root(t: &Tree, window: &Window) -> u32 {
    match windows::root_of(windows::key(window)) {
        0 => t.root,
        popped => popped,
    }
}

/// `id` and its ancestors up to `root`, target first — `None` when `id` is not
/// an element this window can target: gone, not an element, outside `root`'s
/// subtree, `pointer-events: none`, or inside a `visibility: hidden` subtree
/// (gpui paints none of one, so it is not there to be hit).
///
/// `pointer-events` is inherited and the cascade has already folded it, so the
/// element's own computed value is the answer; `auto` under a `none` parent
/// is hittable. The root is the exception `render::pointer_mask` makes too:
/// `body { pointer-events: none }` is how a modal layer says "nothing else",
/// and the root then stands in for the document a browser would target.
fn path_if_hittable(t: &Tree, id: u32, root: u32) -> Option<Vec<u32>> {
    let node = t.node(id)?;
    if node.kind != Kind::Element {
        return None;
    }
    let inert = node.computed.as_ref().is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
    if inert && id != root {
        return None;
    }
    let mut path = Vec::with_capacity(16);
    let mut cur = id;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        if n.computed.as_ref().is_some_and(|cs| cs.base.visibility == Some(gpui::Visibility::Hidden)) {
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

/// What the resolution found: the target, its path (target first, the root
/// last), and the tags under the pointer it came from, topmost first.
struct Resolved {
    target: u32,
    path: Vec<u32>,
    tags: Vec<u32>,
    /// Index in `tags` of the target; `tags.len()` when it is the fallback.
    at: usize,
}

fn resolve(t: &Tree, tags: Vec<u32>, root: u32) -> Resolved {
    for (at, &tag) in tags.iter().enumerate() {
        if let Some(path) = path_if_hittable(t, tag, root) {
            return Resolved { target: tag, path, tags, at };
        }
    }
    let at = tags.len();
    Resolved { target: root, path: vec![root], tags, at }
}

/// The topmost element at `p` that a pointer event there is for, in this
/// window. `None` when nothing but the root is there (or the tree has none).
pub(crate) fn topmost(window: &Window, p: Point<Pixels>) -> Option<u32> {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        tags.into_iter().find(|&tag| path_if_hittable(t, tag, root).is_some())
    })
}

/// [`topmost`], or the window's root: the target of a pointer event at `p`.
pub(crate) fn target_at(window: &Window, p: Point<Pixels>) -> u32 {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        resolve(t, tags, root).target
    })
}

/// The explicit capture target, while it still belongs to this window's tree.
/// CSS hit-testing does not revoke capture; removal or moving to another window does.
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

/// gpui element moves only run while hovered. The OS captures the window,
/// so a raw window listener must collect moves beyond the captured box and
/// beyond the client area as well. Keep normal frame coalescing in both cases.
pub(crate) fn capture_listeners(window: &mut Window) {
    window.on_mouse_event(|ev: &MouseMoveEvent, phase, window, _cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let Some(id) = captured_target(window) else { return };
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

/// The fork's hit refiner: run at the start of every press, release and wheel
/// in a window, before any listener.
fn refine(kind: VnHitKind, p: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let capture = (kind != VnHitKind::Scroll).then(|| captured_target(window)).flatten();
    // Explicit capture owns pointer input before any physical hit target,
    // including an open native <select> popup. Wheel input still hit-tests.
    if capture.is_none() && crate::select::refine(kind, p, window, cx) {
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
    windows::here(window, |w| {
        w.hit = Some(target);
        // `:active` in a script's query: from the press to the release.
        match kind {
            VnHitKind::Down => {
                w.down_hit = Some(target);
                w.active = Some(target);
            }
            VnHitKind::Up => w.active = None,
            VnHitKind::Scroll => {}
        }
    });
    let mut path = resolved.path;
    path.sort_unstable();
    // A wheel scrolls the containers on the path that are UNDER the pointer:
    // one off to the side has nothing of the target in view to scroll.
    window.vn_restrict_mouse_hit(&|tag| path.binary_search(&tag).is_ok(), kind != VnHitKind::Scroll);
}

/// The element the mouse event being dispatched in this window is for, or
/// `fallback` outside such a dispatch (a window the refiner is not armed in).
pub(crate) fn target_or(window: &Window, fallback: u32) -> u32 {
    windows::here(window, |w| w.hit).unwrap_or(fallback)
}

/// Where a `click` goes: the nearest common inclusive ancestor of the
/// original DOM press and the current release. A removed/detached press or
/// an actually-disabled control cannot be activated by the release.
pub(crate) fn click_target(window: &Window, down: u32, fallback: u32) -> Option<u32> {
    let capture = captured_target(window);
    let up = windows::here(window, |w| w.hit).unwrap_or(fallback);
    tree::read_tree(|t| click_target_in_tree(t, down, up, window_root(t, window), capture))
}

fn click_target_in_tree(t: &Tree, down: u32, up: u32, root: u32, capture: Option<u32>) -> Option<u32> {
    // A span inside a disabled button is disabled for activation as well.
    // A fieldset itself must not suppress the enabled first legend.
    let enabled_path = |id| {
        let path = capture_path(t, id, root)?;
        (!path.iter().any(|&id| {
            t.node(id).is_some_and(|n| t.name(n.tag) != "fieldset" && crate::render::actually_disabled(t, id))
        })).then_some(())
    };
    enabled_path(down)?;
    let up = capture.unwrap_or(up);
    enabled_path(up)?;
    capture.or_else(|| common_ancestor(t, down, up))
}

/// The nearest common inclusive ancestor of `a` and `b`.
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

// ---------------------------------------------------------------------------
// `document.elementFromPoint` / `elementsFromPoint` (runtime.ts)
// ---------------------------------------------------------------------------

/// The elements under `p` in `window`, topmost first: every hittable element
/// whose box is there (`all`), or the target alone. Nothing outside the
/// window root's box — the viewport.
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

/// `hit_ids` in the primary window — the one `document`'s viewport is — from
/// wherever the script runs: the window in hand when it is that one, else
/// borrowed through the app.
fn hit_ids_in_primary(p: Point<Pixels>, all: bool) -> Vec<u32> {
    let primary = windows::primary();
    let in_hand = vn_runtime::with_commit(|window, cx| {
        if windows::key(window) == primary {
            Some(hit_ids(window, p, all))
        } else {
            other_window(cx, primary, p, all)
        }
    });
    match in_hand {
        Some(Some(ids)) => ids,
        Some(None) => Vec::new(),
        None => vn_runtime::with_app(|cx| other_window(cx, primary, p, all)).flatten().unwrap_or_default(),
    }
}

fn other_window(cx: &mut App, key: windows::WinKey, p: Point<Pixels>, all: bool) -> Option<Vec<u32>> {
    let handle = cx.windows().into_iter().find(|h| h.window_id().as_u64() == key)?;
    handle.update(cx, |_, window, _| hit_ids(window, p, all)).ok()
}

// `(x, y, all)` → the element ids under the viewport point, topmost first,
// comma-separated; `""` outside the viewport (`hit_ids`).
host_fn!(fn vn_hit_at(_g, args) {
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

// `(node, pointerId, capture)` — the JS mirror validates the active pointer;
// ownership is per native window and is cleared even outside a commit wave
// (unmount and app teardown do not necessarily run in an input callback).
host_fn!(fn vn_pointer_capture(_g, args) {
    let id = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let pointer = args.arg(1).as_number().unwrap_or(0.0);
    if pointer != 1.0 {
        return Value::boolean(false).0;
    }
    let capture = args.arg(2).as_bool().unwrap_or(false);
    if !capture {
        windows::each(|_, w| {
            if w.pointer_capture == Some(id) {
                w.pointer_capture = None;
            }
        });
        return Value::boolean(true).0;
    }
    let k = windows::window_of_node(id);
    // The OS capture is held by the window where the live press began. A
    // script cannot transfer client-space input into another native window.
    if !windows::peek(k, |w| w.press.as_ref().is_some_and(crate::render::Press::is_active)).unwrap_or(false) {
        return Value::boolean(false).0;
    }
    let valid = tree::read_tree(|t| {
        let root = match windows::root_of(k) { 0 => t.root, popped => popped };
        capture_path(t, id, root).is_some()
    });
    if valid {
        windows::each(|_, w| w.pointer_capture = None);
        windows::of(k, |w| w.pointer_capture = Some(id));
    }
    Value::boolean(valid).0
});

/// Registers hit testing and explicit pointer capture.
pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__vn_hit_at", 3, vn_hit_at as *const std::ffi::c_void);
    rt.register("__vn_pointer_capture", 3, vn_pointer_capture as *const std::ffi::c_void);
}

thread_local! {
    /// `(cover, listener)` pairs the dev log has already named.
    static NAMED: std::cell::RefCell<std::collections::HashSet<(u32, u32)>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// A press whose target COVERS an element that listens for presses and is not
/// on its path — a box painted over it that is not one of its ancestors. That
/// is the browser's rule and nothing to correct, but it is new here, and a
/// layout that differs from Chromium's (a transparent box laid out bigger than
/// it should be) takes presses the old rule let through. So the covered
/// listener gets one `dev` log line per (cover, listener) pair: the lead for
/// "my button stopped working".
///
/// Only the elements gpui found under the pointer are looked at, so this is a
/// few node reads per press.
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
        let inert = node.computed.as_ref().is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
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

// ---------------------------------------------------------------------------
// Hover: `:hover`, `group-hover`, `mouseenter` / `mouseleave`
// ---------------------------------------------------------------------------

/// What is hovered in one window (`WindowState::hover`), and what the answer
/// was resolved from.
#[derive(Default)]
pub struct Hover {
    /// The topmost element under the pointer and its ancestors, target first:
    /// the elements `:hover` matches. Empty while the pointer is outside the
    /// window.
    path: Vec<u32>,
    /// The pointer's position, whether it was in the window, and
    /// `render::layout_count()`, at the last resolution.
    from: Option<(Point<Pixels>, bool, u64)>,
    /// When the last resolution ran.
    when: Option<std::time::Instant>,
    /// A pop-out's per-frame tick is armed ([`arm_hover_tick`]).
    ticking: bool,
}

/// How often a page that keeps building frames under a STILL pointer has its
/// hover resolved again: an animation, a scroll, a list filling in. A moving
/// pointer resolves every frame it moves in. Chromium likewise refreshes hover
/// under a still pointer on a timer while a page scrolls, not every frame.
const STILL_POINTER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Resolves what is hovered in this window, once a frame, the way a press is
/// resolved: the topmost element under the pointer that `pointer-events` lets
/// be hit (a pick or a hitbox, [`gpui::Window::vn_tags_at`]), and its
/// ancestors. That path is what `:hover` matches, and the elements that join
/// or leave it are what `mouseenter` and `mouseleave` fire at. gpui's own
/// hover is geometric: every hitbox under the pointer, so an element under a
/// cover that listens for nothing stayed hovered, and an ancestor whose box
/// the pointer had left (a dropdown laid out below its trigger) lost its hover
/// while its child kept it.
///
/// Cost: nothing on a frame in which the pointer did not move and nothing was
/// built; one walk over the frame's tagged boxes on a frame in which it moved;
/// the same walk at most every [`STILL_POINTER_INTERVAL`] while frames are
/// built under a still pointer. The pointer's own moves between frames are
/// never resolved one by one.
///
/// Keyboard input does not hide hover: CSS `:hover` is pointer geometry and
/// nothing else (docs/ARCHITECTURE.md §11 item 12).
pub(crate) fn resolve_hover(window: &mut Window, cx: &mut App) {
    let pos = window.mouse_position();
    // Over an open `<select>` popup nothing of the page is under the pointer:
    // the hover stays as it was, as when the pointer is over another window.
    if crate::select::covers(window, pos) {
        return;
    }
    let inside = window.vn_pointer_inside();
    let built = crate::render::layout_count();
    let now = std::time::Instant::now();
    let due = windows::here(window, |w| match w.hover.from {
        None => true,
        Some((p, i, _)) if p != pos || i != inside => true,
        Some((_, _, b)) if b != built => {
            w.hover.when.is_none_or(|t| now.duration_since(t) >= STILL_POINTER_INTERVAL)
        }
        Some(_) => false,
    });
    if !due {
        return;
    }
    let path = if inside {
        let tags = window.vn_tags_at(pos);
        tree::read_tree(|t| match window_root(t, window) {
            0 => Vec::new(),
            root => resolve(t, tags, root).path,
        })
    } else {
        Vec::new()
    };
    let old = windows::here(window, |w| {
        w.hover.from = Some((pos, inside, built));
        w.hover.when = Some(now);
        (w.hover.path != path).then(|| std::mem::replace(&mut w.hover.path, path.clone()))
    });
    if let Some(old) = old {
        hover_moved(&old, &path, pos, window, cx);
    }
}

/// The elements that leave the hover path, innermost first, and the ones that
/// join it, outermost first: the order `mouseleave` and `mouseenter` fire in
/// (UI Events §4.3.3, as Chromium dispatches them).
fn boundary(old: &[u32], new: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let left = old.iter().copied().filter(|id| !new.contains(id)).collect();
    let entered = new.iter().rev().copied().filter(|id| !old.contains(id)).collect();
    (left, entered)
}

/// The hover path moved from `old` to `new`: the styles first, then the
/// events.
fn hover_moved(old: &[u32], new: &[u32], pos: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let (left, entered) = boundary(old, new);
    // `tree::note_hover` keeps the hovered set the renderer folds `:hover`
    // and `group-hover:` rungs from, and aims the transitions. Before any
    // handler: a `mouseenter` listener that restyles must find the transition
    // already aimed at its target.
    for &id in &left {
        tree::note_hover(id, false);
    }
    for &id in &entered {
        tree::note_hover(id, true);
    }
    // The rungs are folded at build time, so a cached subtree holding one has
    // to render again.
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
    // `mouseenter` and `mouseleave` fire at every element that joins or leaves
    // the path and listens for them, `pointer-events` notwithstanding: it
    // decides what is hit, not who hears about it. An element no longer in
    // the document hears nothing.
    let has = |t: &Tree, id: u32, ev: u32| t.node(id).is_some_and(|n| n.listeners & (1u64 << ev) != 0);
    let (leaves, enters): (Vec<u32>, Vec<u32>) = tree::read_tree(|t| {
        let root = window_root(t, window);
        (
            left.iter().copied().filter(|&id| has(t, id, tree::EV_MOUSELEAVE) && path_to(t, id, root)).collect(),
            entered.iter().copied().filter(|&id| has(t, id, tree::EV_MOUSEENTER)).collect(),
        )
    });
    if leaves.is_empty() && enters.is_empty() {
        return;
    }
    let payload = crate::render::mouse_payload(pos, -1.0, &window.modifiers());
    let events = leaves.into_iter().map(|id| (id, tree::EV_MOUSELEAVE)).chain(enters.into_iter().map(|id| (id, tree::EV_MOUSEENTER)));
    // The pump's, before gpui's draw, as the frame's `mousemove` is
    // (`render::flush_pending_mouse_move`, which runs after this).
    crate::off_input(|| {
        for (id, ev) in events {
            // A handler before this one may have removed it.
            if tree::read_tree(|t| t.node(id).is_some()) {
                crate::render::js_dispatch_app(id, ev, &payload, window, cx);
            }
        }
    });
}

/// Does `id` still hang off `root`?
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

/// A pop-out window has no frame pump (vn-app's pump drives the primary
/// window), so its hover — and its frame's coalesced `mousemove` — are
/// delivered from a per-frame callback of its own, armed here once, from the
/// pop-out's build.
///
/// The `mousemove` used to be left out: a pop-out's move was stored for its
/// window and never delivered, so nothing in a pop-out heard the pointer move
/// — a drag that pans a zoomed picture, a player's controls that come back on
/// a move.
pub(crate) fn arm_hover_tick(window: &mut Window) {
    let k = windows::key(window);
    if windows::is_primary(k) || windows::here(window, |w| std::mem::replace(&mut w.hover.ticking, true)) {
        return;
    }
    window.on_next_frame(hover_tick);
}

fn hover_tick(window: &mut Window, cx: &mut App) {
    // Hover first, then the move, as the pump does for the primary window.
    crate::render::flush_pending_mouse_move(window, cx);
    window.on_next_frame(hover_tick);
}

/// `node 42 (<div id="x" class="a b">)`, for the log line and the advisory.
fn describe(t: &Tree, id: u32) -> String {
    let Some(node) = t.node(id) else { return format!("node {id}") };
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

    /// The cascade's answers, set by hand as the selection tests do:
    /// `pointer-events` already folded down the tree, `visibility` as declared.
    fn set(t: &mut Tree, id: u32, pointer_none: Option<bool>, hidden: bool) {
        let mut cs = vn_css::ComputedStates::default();
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
        let [root, list, a, b, a1] = ids[..] else { unreachable!() };
        tree::read_tree(|t| {
            assert_eq!(common_ancestor(t, a1, a1), Some(a1), "the same element");
            assert_eq!(common_ancestor(t, a1, a), Some(a), "an ancestor of the other");
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
            assert_eq!(click_target_in_tree(t, down, up, root, Some(down)), Some(down));
            assert_eq!(click_target_in_tree(t, down, up, other_window, None), None);
            t.insert(down, other_window, 0);
            assert_eq!(click_target_in_tree(t, down, up, root, None), None, "detached from the original window");
            t.remove(down);
            assert_eq!(click_target_in_tree(t, down, up, root, None), None, "removed press must not retarget to a listener");
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
            assert_eq!(click_target_in_tree(t, child, child, root, None), Some(child));
            let disabled = t.intern("disabled");
            t.set_prop(button, disabled, Some(String::new()));
            assert_eq!(click_target_in_tree(t, child, child, root, None), None);
            assert_eq!(click_target_in_tree(t, root, child, root, None), None, "disabled release target");
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
            assert_eq!(click_target_in_tree(t, enabled, enabled, root, None), Some(enabled));
            assert_eq!(click_target_in_tree(t, disabled_button, disabled_button, root, None), None);
            vec![root, button, child, fieldset, legend, enabled, disabled_button]
        });
    }

    /// `mouseleave` innermost first, then `mouseenter` outermost first; the
    /// shared ancestors hear neither.
    #[test]
    fn the_boundary_between_two_hover_paths() {
        // Paths are target first: a1 → a → list → root, b1 → b → list → root.
        let (left, entered) = boundary(&[5, 3, 2, 1], &[6, 4, 2, 1]);
        assert_eq!(left, vec![5, 3]);
        assert_eq!(entered, vec![4, 6]);
        // Into a child: only the child enters.
        assert_eq!(boundary(&[3, 2, 1], &[5, 3, 2, 1]), (vec![], vec![5]));
        // Out of the window: everything leaves, innermost first.
        assert_eq!(boundary(&[3, 2, 1], &[]), (vec![3, 2, 1], vec![]));
        // Into it: everything enters, the root first.
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
            assert_eq!(capture_path(t, child, root), Some(vec![child, parent, root]));
            assert_eq!(capture_path(t, child, other), None, "capture never crosses windows");
            t.insert(child, other, 0);
            assert_eq!(capture_path(t, child, parent), None, "detached from old painted root");
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
        let [root, decor, inside, hidden, under_hidden, plain] = ids[..] else { unreachable!() };
        tree::read_tree(|t| {
            assert!(path_if_hittable(t, decor, root).is_none(), "pointer-events: none");
            assert_eq!(
                path_if_hittable(t, inside, root),
                Some(vec![inside, decor, root]),
                "auto under none is hit, and its path runs through the none parent"
            );
            assert!(path_if_hittable(t, hidden, root).is_none(), "visibility: hidden");
            assert!(path_if_hittable(t, under_hidden, root).is_none(), "inside a hidden subtree");
            assert_eq!(path_if_hittable(t, plain, root), Some(vec![plain, root]));
            // Topmost first: the decoration and the hidden box are passed over.
            let r = resolve(t, vec![decor, under_hidden, plain, root], root);
            assert_eq!((r.target, r.at), (plain, 2));
            let r = resolve(t, vec![decor], root);
            assert_eq!(r.target, root, "nothing hittable: the root");
            // A node outside the window's root is not this window's.
            assert!(path_if_hittable(t, plain, decor).is_none());
        });
    }
}
