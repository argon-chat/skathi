//! Lowers the retained tree into gpui elements inside the hosting view's `render()`. gpui's element
//! tree is rebuilt per frame by design, so the whole thing is re-emitted.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, ClickEvent, Context, Empty, FocusHandle, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseUpEvent, Overflow, Point, ScrollEvent, ScrollHandle,
    SharedString, Window, WindowControlArea,
};
use sk_css::{WindowControl, WindowDrag};

use crate::img as img_src;
use crate::tree::{self, Kind, Tree};
use crate::windows::{self, PendingMouseMove, Rect, WinKey};
use crate::EventPayload;
use sk_elements::{text_input, TextInputHandlers, TextInputOptions, TextInputState};
use sk_runtime::{with_published, Phase};

thread_local! {
    /// Persistent per-node focus handles (a fresh handle each render would blur the node every
    /// frame), GC'd against tree liveness. Shared across windows: `cx.focus_handle()` mints its id
    /// in `App`; only the subscription (`WindowState::focus_subs`) is per-window.
    static FOCUS_HANDLES: RefCell<HashMap<u32, FocusHandle>> = RefCell::new(HashMap::new());
    static INPUT_STATES: RefCell<HashMap<u32, gpui::Entity<TextInputState>>> =
        RefCell::new(HashMap::new());
    /// Each `<input>` editor's focus handle as of its last build. Asking where focus is must not
    /// read the editor: a `keydown` handler runs inside its update and the read panics in gpui.
    static INPUT_FOCUS: RefCell<HashMap<u32, FocusHandle>> = RefCell::new(HashMap::new());
    /// select() can run in nextTick before the input has its first native build.
    static INPUT_SELECT_ALL: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// Persistent scroll offset per container; owning the handle lets us read the offset back for
    /// `e.target.scrollTop`.
    static SCROLL_HANDLES: RefCell<HashMap<u32, ScrollHandle>> = RefCell::new(HashMap::new());
    static SCROLL_REPORTED: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static BUILT: Cell<u64> = const { Cell::new(0) };
    /// Element-tree builds since start. A DOM read wanting forced layout learns from this whether
    /// it got one: `sync_layout` is a no-op inside the window's own update, i.e. every event
    /// handler.
    static BUILDS: Cell<u64> = const { Cell::new(0) };
    /// Builds in any window, pop-outs included: when layout can have moved. [`BUILDS`] is
    /// primary-only: a pop-out's build is not the forced layout a DOM read asked for.
    static LAYOUTS: Cell<u64> = const { Cell::new(0) };
    /// Nodes painted by another window: they stay in the retained tree but generate no box where
    /// declared; the other window builds them as its own root via [`build_subtree`].
    static POPPED_OUT: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static SETTLING: RefCell<HashMap<u32, u8>> = RefCell::new(HashMap::new());
}

/// Whether every node gets a hitbox; off unless the devtools boot enables it (otherwise only
/// interactive elements do).
static PICK_HITBOXES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn enable_pick_hitboxes() {
    PICK_HITBOXES.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn arm_pick_hitboxes(window: &mut Window) {
    if PICK_HITBOXES.load(std::sync::atomic::Ordering::Relaxed) {
        window.set_tag_every_hitbox(true);
    }
    // A mouse event goes to the topmost element under the pointer, which needs every element to be
    // a box gpui can find (picks, for ones without a hitbox) and the target resolved before
    // listeners run (`crate::pointer`).
    crate::pointer::arm(window);
}

/// The hitbox gpui made for `id` in this window's last frame, for [`gpui::Hitbox::is_hovered_at`].
/// It is gpui's own, so it carries the CSS `transform` in force where it was inserted. Hitboxes
/// belong to the window that inserted them.
pub fn pick_hitbox(window: &Window, id: u32) -> Option<gpui::Hitbox> {
    window.tagged_hitbox(id).cloned()
}

/// Marks (or unmarks) a node as painted by another window. Returns `true` when the flag moved.
pub fn set_popped_out(id: u32, on: bool) -> bool {
    POPPED_OUT.with(|s| {
        let mut s = s.borrow_mut();
        if on {
            s.insert(id)
        } else {
            s.remove(&id)
        }
    })
}

pub fn popped_out(id: u32) -> bool {
    POPPED_OUT.with(|s| s.borrow().contains(&id))
}

pub(crate) fn warn_compat(key: &str, msg: &str) {
    warn_compat_at(key, String::new, msg)
}

pub(crate) fn warn_compat_at(key: &str, at: impl FnOnce() -> String, msg: &str) {
    WARNED.with(|w| {
        let mut warned = w.borrow_mut();
        if !warned.contains(key) && warned.insert(key.to_string()) {
            let msg = crate::log::one_line(&format!("{}{msg}", at()));
            eprintln!("[skathi compat] {msg}");
            crate::log::record("advisory", msg);
        }
    });
}

fn node_prefix(t: &Tree, id: u32) -> String {
    let Some(node) = t.node(id) else {
        return String::new();
    };
    let tag = t.name(node.tag);
    let classes: Vec<&str> = node.classes.iter().map(String::as_str).take(3).collect();
    if classes.is_empty() {
        format!("node {id} (<{tag}>): ")
    } else {
        format!("node {id} (<{tag} class=\"{}\">): ", classes.join(" "))
    }
}

/// Called once per `render()` of one window: its last-draw rectangles are about to be replaced, and
/// anything it stops painting must stop having bounds. Per window: one global map wiped a pop-out's
/// rectangles on every frame only the primary drew.
fn reset_bounds(win: WinKey) {
    windows::of(win, |w| {
        // Reuse both generations' allocation so a rebuild does not regrow every bounds map.
        std::mem::swap(&mut w.bounds, &mut w.prev_bounds);
        w.bounds.clear();
        if w.bounds.capacity() > w.prev_bounds.len().saturating_mul(4).max(1024) {
            w.bounds.shrink_to(w.prev_bounds.len().saturating_mul(2));
        }
        std::mem::swap(&mut w.transformed, &mut w.prev_transformed);
        w.transformed.clear();
        if w.transformed.capacity() > w.prev_transformed.len().saturating_mul(4).max(1024) {
            w.transformed
                .shrink_to(w.prev_transformed.len().saturating_mul(2));
        }
    });
}

fn rect_of(bounds: gpui::Bounds<gpui::Pixels>) -> Rect {
    (
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

/// `matrix` is the node's whole CSS `transform` (ancestors' composed with its own): where the
/// picture of the box is, which is what `getBoundingClientRect()` reports.
fn record_bounds(
    id: u32,
    bounds: gpui::Bounds<gpui::Pixels>,
    matrix: gpui::TransformationMatrix,
    window: &Window,
) {
    let rect = rect_of(bounds);
    let moved = (!matrix.is_unit()).then(|| rect_of(window.transformed_bounds(matrix, bounds)));
    windows::here(window, |w| {
        w.bounds.insert(id, rect);
        match moved {
            Some(t) => w.transformed.insert(id, t),
            // A prepaint can run twice in one frame (gpui's `transact`).
            None => w.transformed.remove(&id),
        };
    });
}

/// Where CSS `transform` put the node, in the window [`bounds_of`] finds it in; `None` when nothing
/// moved it.
pub fn transformed_of(id: u32) -> Option<Rect> {
    let k = windows::keys()
        .into_iter()
        .find(|&k| windows::peek(k, |w| w.bounds.contains_key(&id)) == Some(true))?;
    windows::peek(k, |w| w.transformed.get(&id).copied()).flatten()
}

/// `(x, y, width, height)` in CSS px relative to the window that painted it, from the last frame it
/// was painted in; `None` if never painted.
pub fn bounds_of(id: u32) -> Option<Rect> {
    windows::keys()
        .into_iter()
        .find_map(|k| windows::peek(k, |w| w.bounds.get(&id).copied()).flatten())
}

/// The rectangle one window painted this node at; a rectangle is meaningful only in its window's
/// client space, so a point query must name the window.
pub fn bounds_in(root: u32, id: u32) -> Option<Rect> {
    let k = windows::window_of_root(root)?;
    windows::peek(k, |w| w.bounds.get(&id).copied()).flatten()
}

pub fn bounds_here(window: &Window, id: u32) -> Option<Rect> {
    windows::peek(windows::key(window), |w| w.bounds.get(&id).copied()).flatten()
}

pub fn painted_bounds_in(root: u32, id: u32) -> Option<Rect> {
    let k = windows::window_of_root(root)?;
    windows::peek(k, |w| {
        w.bounds
            .get(&id)
            .copied()
            .or_else(|| w.prev_bounds.get(&id).copied())
    })
    .flatten()
}

/// Where the app root was painted in the window's client area, CSS px; `None` before the first
/// frame. Host-drawn chrome is excluded, as in `innerWidth`.
pub fn root_bounds() -> Option<Rect> {
    root_bounds_in(windows::primary())
}

/// [`root_bounds`] for one window: the source of a per-window `@media` viewport and a `SkWindow`'s
/// `@resize` payload.
pub fn root_bounds_in(k: WinKey) -> Option<Rect> {
    let root = match windows::root_of(k) {
        0 => crate::tree::read_tree(|t| t.root),
        popped => popped,
    };
    windows::peek(k, |w| {
        w.bounds
            .get(&root)
            .copied()
            .or_else(|| w.prev_bounds.get(&root).copied())
    })
    .flatten()
}

/// Where the node is, asked from inside a build: the window's live map is cleared first, so only
/// the previous frame's rectangles exist.
pub fn painted_bounds(id: u32) -> Option<Rect> {
    windows::keys().into_iter().find_map(|k| {
        windows::peek(k, |w| {
            w.bounds
                .get(&id)
                .copied()
                .or_else(|| w.prev_bounds.get(&id).copied())
        })
        .flatten()
    })
}

#[cfg(any(feature = "lottie", feature = "video"))]
pub(crate) fn prev_size_of(id: u32) -> Option<(f32, f32)> {
    let k = windows::window_of_node(id);
    windows::peek(k, |w| w.prev_bounds.get(&id).copied())
        .flatten()
        .filter(|(_, _, w, h)| *w > 0.0 && *h > 0.0)
        .map(|(_, _, w, h)| (w, h))
}

pub(crate) fn build_count() -> u64 {
    BUILDS.with(|c| c.get())
}

pub(crate) fn side_state_counts() -> (usize, usize, usize, usize) {
    let focus = FOCUS_HANDLES.with(|m| m.borrow().len());
    let inputs = INPUT_STATES.with(|m| m.borrow().len());
    let scrolls = SCROLL_HANDLES.with(|m| m.borrow().len());
    let bounds = windows::keys()
        .into_iter()
        .filter_map(|k| windows::peek(k, |w| w.bounds.len() + w.prev_bounds.len()))
        .sum();
    (focus, inputs, scrolls, bounds)
}

pub(crate) fn layout_count() -> u64 {
    LAYOUTS.with(|c| c.get())
}

#[cfg(test)]
pub(crate) fn note_build() {
    BUILDS.with(|c| c.set(c.get().wrapping_add(1)));
    LAYOUTS.with(|c| c.set(c.get().wrapping_add(1)));
}

/// The [`ScrollHandle`] a scroll container owns; it outlives the frame.
pub(crate) fn scroll_handle_of(id: u32) -> Option<ScrollHandle> {
    SCROLL_HANDLES.with(|m| m.borrow().get(&id).cloned())
}

/// The whole rectangle painted in the previous frame, origin included: what the cache's "did my
/// parent reflow?" check compares.
pub(crate) fn prev_rect_of(id: u32) -> Option<Rect> {
    let k = windows::window_of_node(id);
    windows::peek(k, |w| w.prev_bounds.get(&id).copied()).flatten()
}

/// Transparent wrapper recording the bounds gpui laid its inner element out at; adds no layout.
struct BoundsProbe {
    id: u32,
    /// The root probe brackets the whole recursion in each of gpui's three element phases, so it is
    /// the one place that can time them ([`crate::perf::Phases`]).
    root: bool,
    inner: AnyElement,
}

impl gpui::Element for BoundsProbe {
    type RequestLayoutState = gpui::LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, gpui::LayoutId) {
        if !self.root {
            let layout = self.inner.request_layout(window, cx);
            return (layout, layout);
        }
        let _profile_window =
            sk_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
        let _profile = sk_runtime::redjs::profiling::span("layout", "Layout requests");
        let span = crate::perf::Span::start();
        let layout = self.inner.request_layout(window, cx);
        crate::perf::note_layout_req(span.ms());
        (layout, layout)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        layout: &mut gpui::LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Recorded before the subtree prepaints (which may read it) as where the ancestors'
        // transforms put the box; its own transform is amended below once its element has
        // prepainted.
        let ancestors = window.element_transform();
        record_bounds(self.id, bounds, ancestors, window);
        crate::containers::record_percentage_basis(self.id, || {
            let size = window.vn_percentage_basis(*layout)?;
            Some((f32::from(size.width), f32::from(size.height)))
        });
        // Every hitbox gpui inserts below is tagged with this node id until a descendant's probe
        // overrides it; the previous tag is restored on the way out.
        let _profile_window = self
            .root
            .then(|| sk_runtime::redjs::profiling::WindowScope::enter(windows::key(window)));
        if self.root {
            crate::perf::note_taffy_gap();
        }
        let _profile = self
            .root
            .then(|| sk_runtime::redjs::profiling::span("paint", "Prepaint native tree"));
        let span = self.root.then(crate::perf::Span::start);
        let (_, own) = window.with_hitbox_tag(self.id, |window| self.inner.prepaint(window, cx));
        if let Some(span) = span {
            crate::perf::note_prepaint(span.ms());
        }
        if let Some(matrix) = own {
            record_bounds(self.id, bounds, matrix, window);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut gpui::LayoutId,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.root {
            self.inner.paint(window, cx);
            return;
        }
        let _profile_window =
            sk_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
        let _profile = sk_runtime::redjs::profiling::span("paint", "Paint native tree");
        crate::selection::paint_listeners(window);
        crate::pointer::capture_listeners(window);
        let span = crate::perf::Span::start();
        self.inner.paint(window, cx);
        crate::perf::note_paint(span.ms());
    }
}

/// The probe, shifted by `position: sticky` when the box has a [`crate::sticky::Spec`]: outside the
/// probe, so rectangles and hit tests see where it is painted.
fn placed(
    id: u32,
    sticky: Option<crate::sticky::Spec>,
    root: bool,
    inner: AnyElement,
) -> AnyElement {
    crate::sticky::wrap(id, sticky, probe_at(id, root, inner))
}

pub(crate) fn probe(id: u32, inner: AnyElement) -> AnyElement {
    probe_at(id, false, inner)
}

fn probe_at(id: u32, root: bool, inner: AnyElement) -> AnyElement {
    BoundsProbe { id, root, inner }.into_any_element()
}

pub(crate) fn gc_node_state() {
    let live = |id: u32| tree::read_tree(|t| t.node(id).is_some());
    FOCUS_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    INPUT_STATES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    INPUT_FOCUS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    INPUT_SELECT_ALL.with(|m| m.borrow_mut().retain(|&id| live(id)));
    SCROLL_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_REPORTED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SETTLING.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    crate::cache::gc(&live);
    crate::cull::gc(&live);
    crate::scroll::gc(&live);
    crate::sticky::gc(&live);
    img_src::gc(live);
    crate::lottie::gc(live);
    crate::video::gc(live);
    crate::surface::node::gc(live);
    crate::iframe::gc(live);
    sticker::gc(live);
    crate::svg::gc(live);
    tree::with_tree(crate::selection::gc);
    // This runs off a JS wave, not a frame, so there is no `Window` to ask which one it is in.
    windows::each(|_, w| {
        w.bounds.retain(|&id, _| live(id));
        w.prev_bounds.retain(|&id, _| live(id));
        w.transformed.retain(|&id, _| live(id));
        w.prev_transformed.retain(|&id, _| live(id));
        w.focus_subs.retain(|&id, _| live(id));
        w.haptics.gc(&live);
        if w.pointer_capture
            .is_some_and(|id| !live(id) || !tree::read_tree(|t| reaches_root(t, id)))
        {
            w.pointer_capture = None;
        }
    });
    // Detaching the focused element blurs it, as in a browser. A node can stay allocated yet leave
    // the document (`<KeepAlive>` moves a deactivated page into a container that is never inserted,
    // and a build walks from the painted root only); gpui would keep routing keystrokes to a handle
    // whose element is nowhere. Per window, because focus is.
    let mut blur: Vec<(WinKey, u32)> = Vec::new();
    windows::each(|k, w| {
        if w.focused_node != 0 && !live(w.focused_node) {
            w.focused_node = 0;
        } else if w.focused_node != 0 {
            blur.push((k, w.focused_node));
        }
    });
    for (k, focused) in blur {
        if !tree::read_tree(|t| keeps_focus(t, focused)) {
            windows::of(k, |w| w.pending_blur = focused);
        }
    }
}

/// May `id`, which has focus, keep it? A node that disabled itself, lost its `tabindex` or is
/// `display: none` keeps gpui's focus but is no longer bound to an element, so key events dispatch
/// from the root with no listener; a browser moves focus to the body.
fn keeps_focus(t: &Tree, id: u32) -> bool {
    let shown = |n: &tree::Node| {
        !n.computed
            .as_ref()
            .is_some_and(|c| c.base.display == Some(gpui::Display::None))
    };
    reaches_painted_root(t, id)
        && could_take_focus(t, id)
        && t.node(id).is_some_and(shown)
        && !t.hidden_above(id)
}

fn reaches_painted_root(t: &Tree, id: u32) -> bool {
    reaches_root(t, id)
}

fn reaches_root(t: &Tree, id: u32) -> bool {
    if t.root == 0 {
        return false;
    }
    let mut cursor = id;
    // A corrupt parent chain must not hang the pump.
    for _ in 0..1024 {
        if cursor == t.root {
            return true;
        }
        match t.node(cursor) {
            Some(node) if node.parent != 0 => cursor = node.parent,
            _ => return false,
        }
    }
    false
}

/// Keeps something focused so keystrokes have a dispatch path: gpui has no default focus target,
/// and with nothing focused a key event dispatches from the window's root alone. Per window.
pub(crate) fn ensure_root_focus(window: &mut Window, cx: &mut App) {
    if window.focused(cx).is_some() {
        return;
    }
    let root = match windows::root_of(windows::key(window)) {
        0 => tree::read_tree(|t| t.root),
        popped => popped,
    };
    if root == 0 {
        return;
    }
    let Some(handle) = FOCUS_HANDLES.with(|m| m.borrow().get(&root).cloned()) else {
        return;
    };
    window.focus(&handle, cx);
}

pub(crate) fn dispatch_anim_events(window: &mut Window, cx: &mut App) -> bool {
    let events = tree::take_anim_events();
    if events.is_empty() {
        return false;
    }
    let mut moved = false;
    for event in events {
        let code = match event.kind {
            sk_css::AnimEventKind::TransitionEnd => tree::EV_TRANSITIONEND,
            sk_css::AnimEventKind::AnimationStart => tree::EV_ANIMATIONSTART,
            sk_css::AnimEventKind::AnimationEnd => tree::EV_ANIMATIONEND,
            sk_css::AnimEventKind::AnimationCancel => tree::EV_ANIMATIONCANCEL,
        };
        let listening = tree::read_tree(|t| {
            let mut cursor = event.node;
            for _ in 0..1024 {
                match t.node(cursor) {
                    Some(n) if n.listeners & (1 << code) != 0 => return true,
                    Some(n) if n.parent != 0 => cursor = n.parent,
                    _ => return false,
                }
            }
            false
        });
        if !listening {
            continue;
        }
        let payload = EventPayload {
            a: event.elapsed as f64,
            key: Some(&event.name),
            ..Default::default()
        };
        js_dispatch_app(event.node, code, &payload, window, cx);
        moved = true;
    }
    moved
}

/// Releases focus for a node detached while focused, and tells JS. `blur` is dispatched here, not
/// left to gpui's observer: a node no longer painted may drop out of the focus path without the
/// `Some -> None` edge it watches. `FOCUSED_NODE` is cleared first so the two paths do not both
/// fire.
pub(crate) fn apply_pending_blur(window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let pending = windows::of(k, |w| std::mem::replace(&mut w.pending_blur, 0));
    // Re-check: focus may have moved on since the mutation, and a page can be re-attached before
    // this frame runs.
    if pending == 0 || windows::focused_node(k) != pending {
        return;
    }
    if tree::read_tree(|t| keeps_focus(t, pending)) {
        return;
    }
    window.blur(cx);
    set_focus_slot(k, 0);
    js_dispatch_app(pending, tree::EV_BLUR, &EventPayload::default(), window, cx);
    // `:focus` rungs read the handle live, so repaint without waiting for a JS handler.
    window.refresh();
}

pub(super) fn mods_bits(m: &gpui::Modifiers) -> f64 {
    let mut bits = 0u32;
    if m.control {
        bits |= 1;
    }
    if m.shift {
        bits |= 2;
    }
    if m.alt {
        bits |= 4;
    }
    if m.platform {
        bits |= 8;
    }
    bits as f64
}

fn button_code_opt(b: Option<MouseButton>) -> f64 {
    b.map(button_code).unwrap_or(-1.0)
}

fn button_code(b: MouseButton) -> f64 {
    match b {
        MouseButton::Left => 0.0,
        MouseButton::Middle => 1.0,
        MouseButton::Right => 2.0,
        MouseButton::Navigate(_) => 3.0,
    }
}

/// The one `mousemove` a frame may deliver (UAs align `mousemove` to the frame clock and drop the
/// rest). Also the per-move dedup: gpui runs bubble handlers topmost-first, so the first
/// (innermost) claim wins; otherwise k nested listeners ran k dispatch walks per move. The frame's
/// hover resolves first so `mouseleave`/`mouseenter` precede `mousemove`.
pub fn flush_pending_mouse_move(window: &mut Window, cx: &mut App) {
    crate::pointer_lock::tick(window, cx);
    // The frame's script runs after this: its queries see this window's modality as
    // `:focus-visible` paints it.
    crate::live_state::note_modality(window);
    crate::pointer::resolve_hover(window, cx);
    let Some((p, target)) = take_pending_move(window, cx) else {
        crate::pen::clear_trail(window);
        return;
    };
    let payload = mouse_payload(p.position, p.button, &p.modifiers);
    // The pump's, before gpui's draw: a forced layout may draw (`crate::off_input`).
    crate::off_input(|| js_dispatch_app(target, tree::EV_MOUSEMOVE, &payload, window, cx));
}

/// The coalesced move and its target, its motion armed for the dispatch that follows.
fn take_pending_move(window: &mut Window, cx: &mut App) -> Option<(PendingMouseMove, u32)> {
    let (pending, moved) = windows::here(window, |w| {
        (
            w.pending_mouse_move.take(),
            std::mem::take(&mut w.mouse_movement),
        )
    });
    let p = pending?;
    if p.button < 0.0 && crate::pointer::captured_target(window).is_some() {
        cancel_press(window, cx);
    }
    let target = crate::pointer::routed_target(window)
        .unwrap_or_else(|| crate::pointer::target_at(window, p.position));
    crate::pointer_at_next_dispatch(f64::from(moved.x), f64::from(moved.y));
    Some((p, target))
}

/// gpui's scroll delta as the DOM's `deltaX`/`deltaY`. The conventions are opposite: gpui's follows
/// the platform (positive scrolling up), DOM `deltaY` is positive scrolling down.
fn dom_wheel_delta(delta: Point<gpui::Pixels>) -> (f64, f64) {
    (-f64::from(delta.x), -f64::from(delta.y))
}

pub(crate) fn mouse_payload(
    position: Point<gpui::Pixels>,
    button: f64,
    m: &gpui::Modifiers,
) -> EventPayload<'static> {
    EventPayload {
        a: f64::from(position.x),
        b: f64::from(position.y),
        c: button,
        d: mods_bits(m),
        key: None,
    }
}

/// [`mouse_payload`] for events whose `detail` is the click count: it rides `d` above the four
/// modifier bits so the hot path still crosses with four doubles.
fn mouse_payload_n(
    position: Point<gpui::Pixels>,
    button: f64,
    m: &gpui::Modifiers,
    detail: usize,
) -> EventPayload<'static> {
    let mut p = mouse_payload(position, button, m);
    p.d += (detail.min(255) as f64) * 16.0;
    p
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MousePt {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

impl MousePt {
    fn of(p: &EventPayload) -> Self {
        MousePt {
            a: p.a,
            b: p.b,
            c: p.c,
            d: p.d,
        }
    }
    fn payload(&self) -> EventPayload<'static> {
        EventPayload {
            a: self.a,
            b: self.b,
            c: self.c,
            d: self.d,
            key: None,
        }
    }
}

/// One physical mouse-up as the DOM sees it: `mouseup`, then `click` (left, paired with its DOM
/// press) or `auxclick` (middle/right), then, for the right button (Chromium on Windows),
/// `contextmenu`.
#[derive(Default)]
pub struct UpWave {
    /// DOM identity survives a cached-view wrapper being inserted or replaced.
    press: Option<Press>,
    mouseup: Option<(u32, MousePt)>,
    click: Option<(u32, MousePt)>,
    aux: Option<(u32, MousePt)>,
    menu: Option<(u32, MousePt)>,
    scheduled: bool,
    cancelled: bool,
}

impl UpWave {
    fn click_press(&self) -> Option<u32> {
        let p = self.press.as_ref()?;
        (p.clickable && !p.cancelled).then_some(p.target)
    }
}

pub struct Press {
    target: u32,
    button: f64,
    position: Point<gpui::Pixels>,
    /// A disabled control or a native drag cannot become a click later.
    clickable: bool,
    cancelled: bool,
}

impl Press {
    pub(crate) fn is_active(&self) -> bool {
        !self.cancelled
    }

    fn take_for_release(press: &mut Option<Self>, button: f64) -> Option<Self> {
        if press.as_ref().is_some_and(|p| p.button == button) {
            press.take()
        } else {
            None
        }
    }
}

/// The capture phase of every mouse-up opens its wave, from the build root (the first capture
/// listener gpui runs), whether or not the pointer is over it ([`bind_root_mouse`]).
fn open_up_wave(ev: &MouseUpEvent, window: &mut Window, cx: &mut App) {
    let button = button_code(ev.button);
    windows::here(window, |w| {
        let press = Press::take_for_release(&mut w.press, button);
        let cancelled = press.as_ref().is_some_and(|p| p.cancelled);
        w.up_waves.retain(|wave| wave.scheduled);
        w.up_waves.push(UpWave {
            press,
            cancelled,
            ..Default::default()
        });
    });
    // A capture (or the pointer lock) must receive its release even when no hovered element has a
    // mouse-up listener.
    if let Some(target) = crate::pointer::routed_target(window) {
        let payload = mouse_payload_n(ev.position, button, &ev.modifiers, ev.click_count);
        claim_up(window, cx, |w| &mut w.mouseup, target, &payload);
    }
}

fn bind_root_mouse<E: gpui::InteractiveElement>(mut e: E) -> E {
    e = e.capture_any_mouse_down(|_, window, cx| {
        deliver_move_before_button(window, cx);
        // Re-arm only when a DOM bubble listener actually receives this press.
        windows::here(window, |w| w.press = None);
        begin_press_focus(window, cx)
    });
    e = e.on_any_mouse_down(|_, window, _| windows::here(window, |w| w.press_focus.reached = true));
    e = e.capture_any_mouse_up(|ev, window, cx| {
        deliver_move_before_button(window, cx);
        open_up_wave(ev, window, cx)
    });
    for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
        e = e.on_mouse_up_out(button, |ev, window, cx| {
            deliver_move_before_button(window, cx);
            open_up_wave(ev, window, cx)
        });
    }
    e
}

/// A press or release delivers the coalesced `mousemove` first, as Chromium does; otherwise a move
/// and the release after it in one frame were heard release first.
fn deliver_move_before_button(window: &mut Window, cx: &mut App) {
    let Some((p, target)) = take_pending_move(window, cx) else {
        return;
    };
    let payload = mouse_payload(p.position, p.button, &p.modifiers);
    js_dispatch_app(target, tree::EV_MOUSEMOVE, &payload, window, cx);
}

fn claim_up(
    window: &mut Window,
    cx: &mut App,
    slot: fn(&mut UpWave) -> &mut Option<(u32, MousePt)>,
    id: u32,
    payload: &EventPayload,
) {
    let schedule = windows::here(window, |w| {
        let Some(wave) = w.up_waves.last_mut() else {
            return false;
        };
        let s = slot(wave);
        if s.is_none() {
            *s = Some((id, MousePt::of(payload)));
        }
        !std::mem::replace(&mut wave.scheduled, true)
    });
    if schedule {
        // Deferred, so no gpui listener is under the waves: a forced layout may draw in them
        // (`crate::off_input`).
        window.defer(cx, |window, cx| {
            crate::off_input(|| flush_up_waves(window, cx))
        });
    }
}

fn claim_mouse_click(
    ev: &MouseUpEvent,
    fallback: u32,
    window: &mut Window,
    cx: &mut App,
    slot: fn(&mut UpWave) -> &mut Option<(u32, MousePt)>,
) {
    if cx.has_active_drag() {
        return;
    }
    let down = windows::here(window, |w| w.up_waves.last().and_then(UpWave::click_press));
    let Some(down) = down else { return };
    let Some(target) = crate::pointer::click_target(window, down, fallback) else {
        return;
    };
    let payload = mouse_payload_n(
        ev.position,
        button_code(ev.button),
        &ev.modifiers,
        ev.click_count,
    );
    claim_up(window, cx, slot, target, &payload);
}

fn flush_up_waves(window: &mut Window, cx: &mut App) {
    loop {
        let Some(wave) = windows::here(window, |w| {
            (!w.up_waves.is_empty()).then(|| w.up_waves.remove(0))
        }) else {
            return;
        };
        if wave.cancelled {
            continue;
        }
        if let Some((id, p)) = wave.mouseup {
            js_dispatch_app(id, tree::EV_MOUSEUP, &p.payload(), window, cx);
            windows::here(window, |w| w.pointer_capture = None);
        }
        if let Some((id, p)) = wave.click {
            js_dispatch_app(id, tree::EV_CLICK, &p.payload(), window, cx);
        }
        // Blink finishes the release (`mouseup`, click family) before showing the Windows context
        // menu, so `auxclick` precedes `contextmenu`.
        if let Some((id, p)) = wave.aux {
            js_dispatch_app(id, tree::EV_AUXCLICK, &p.payload(), window, cx);
        }
        if let Some((id, p)) = wave.menu {
            js_dispatch_app(id, tree::EV_CONTEXTMENU, &p.payload(), window, cx);
        }
    }
}

fn cancel_press(window: &mut Window, cx: &mut App) {
    let captured = crate::pointer::captured_target(window);
    crate::live_state::set_active(window, None);
    let press = windows::here(window, |w| match &mut w.press {
        Some(p) if !p.cancelled => {
            p.cancelled = true;
            Some((captured.unwrap_or(p.target), p.position))
        }
        _ => None,
    });
    if let Some((target, position)) = press {
        if tree::read_tree(|t| t.node(target).is_some()) {
            let payload = mouse_payload(position, -1.0, &gpui::Modifiers::default());
            js_dispatch_app(target, tree::EV_POINTERCANCEL, &payload, window, cx);
        }
    }
    windows::here(window, |w| w.pointer_capture = None);
}

fn watch_activation<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let k = windows::key(window);
    if windows::of(k, |w| w.activation.is_some()) {
        return;
    }
    let sub = cx.observe_window_activation(window, |_, window, cx| {
        if !window.is_window_active() {
            crate::haptics::blur(window);
            cancel_press(window, cx);
        }
    });
    windows::of(k, |w| w.activation = Some(sub));
}

/// The keyboard `contextmenu` as Chromium sources it on Windows: the unmodified Menu key fires on
/// key-up, Shift+F10 on key-down, each only if no handler cancelled that key event.
fn bind_menu_keys<E: gpui::InteractiveElement>(e: E, root: u32) -> E {
    e.capture_key_up(|_, window, _| windows::here(window, |w| w.key_prevented = false))
        .on_key_down(move |ev: &KeyDownEvent, window, cx| {
            let m = &ev.keystroke.modifiers;
            let shift_f10 = ev.keystroke.key == "f10" && m.shift && m.number_of_modifiers() == 1;
            if shift_f10 && !windows::here(window, |w| w.key_prevented) {
                fire_keyboard_menu(root, window, cx);
            }
        })
        .on_key_up(move |ev: &KeyUpEvent, window, cx| {
            if ev.keystroke.key == "menu"
                && !ev.keystroke.modifiers.modified()
                && !windows::here(window, |w| std::mem::take(&mut w.key_prevented))
            {
                fire_keyboard_menu(root, window, cx);
            }
        })
}

fn fire_keyboard_menu(root: u32, window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let focused = windows::focused_node(k);
    let target = if focused != 0 && tree::read_tree(|t| t.node(focused).is_some()) {
        focused
    } else {
        root
    };
    let (x, y) = windows::of(k, |w| w.bounds.get(&target).copied())
        .filter(|_| target != root || focused == root)
        .map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0))
        .unwrap_or((1.0, 1.0));
    let payload = EventPayload {
        a: f64::from(x),
        b: f64::from(y),
        ..Default::default()
    };
    js_dispatch_app(target, tree::EV_CONTEXTMENU, &payload, window, cx);
}

/// Publish → dispatch → flush, shared by every listener. Returns `false` when a JS handler called
/// `preventDefault()`.
pub(crate) fn js_dispatch_app(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    crate::pen::arm(window, event);
    let proceed = js_wave_app(window, cx, || crate::haptics::dispatch(id, event, payload));
    crate::haptics::event(id, event, payload, proceed, window, cx);
    proceed
}

pub(crate) fn js_wave_app(window: &mut Window, cx: &mut App, wave: impl FnOnce() -> bool) -> bool {
    // A gpui listener's dispatch: no forced layout may draw under it (`crate::in_input_dispatch`).
    let _input = crate::InputDispatch::enter();
    crate::live_state::note_modality(window);
    let proceed = with_published(window, cx, Phase::Commit, wave);
    settle_owed_later(window, cx);
    if crate::flush_changes() {
        // Not `window.refresh()`: it makes the next draw ignore every `Entity::cached` span; the
        // fan-out notifies each hosting view instead.
        if !repaint_other_windows(cx) {
            // No host registered the fan-out (bare test harness): fall back to `refresh`.
            window.refresh();
        }
    }
    proceed
}

thread_local! {
    static REPAINT_ALL: Cell<Option<fn(&mut App)>> = const { Cell::new(None) };
}

pub fn set_repaint_all(f: fn(&mut App)) {
    REPAINT_ALL.with(|c| c.set(Some(f)));
}

pub(crate) fn request_repaint(window: &mut Window, cx: &mut App) {
    if !repaint_other_windows(cx) {
        window.refresh();
    }
}

fn repaint_other_windows(cx: &mut App) -> bool {
    match REPAINT_ALL.with(|c| c.get()) {
        Some(f) => {
            f(cx);
            true
        }
        None => false,
    }
}

fn js_dispatch<V: 'static>(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut Context<V>,
) -> bool {
    let _input = crate::InputDispatch::enter();
    crate::live_state::note_modality(window);
    crate::pen::arm(window, event);
    let proceed = with_published(window, &mut **cx, Phase::Commit, || {
        crate::haptics::dispatch(id, event, payload)
    });
    crate::haptics::event(id, event, payload, proceed, window, cx);
    settle_owed_later(window, &mut **cx);
    if crate::flush_changes() {
        cx.notify();
        repaint_other_windows(cx);
    }
    proceed
}

thread_local! {
    static SETTLE_QUEUED: Cell<bool> = const { Cell::new(false) };
}

/// Queues the microtask checkpoint a listener's dispatch left owed (`crate::dispatch_event`) for
/// after gpui's dispatch: an `App::defer` runs once the listener's update has returned, in a
/// publication of its own, off input, so an `onMounted` the patch mounts can measure and the forced
/// layout can draw.
fn settle_owed_later(window: &Window, cx: &mut App) {
    if !sk_runtime::bridge::drain_owed() || SETTLE_QUEUED.with(|c| c.replace(true)) {
        return;
    }
    let origin = window.window_handle();
    cx.defer(move |cx| {
        SETTLE_QUEUED.with(|c| c.set(false));
        if !sk_runtime::bridge::drain_owed() {
            return;
        }
        let settle = |window: &mut Window, cx: &mut App| {
            crate::off_input(|| js_wave_app(window, cx, || true));
        };
        if origin
            .update(cx, |_, window, cx| settle(window, cx))
            .is_ok()
        {
            return;
        }
        for other in cx.windows() {
            if other.update(cx, |_, window, cx| settle(window, cx)).is_ok() {
                return;
            }
        }
    });
}

/// A text field's default action must see what its event's handlers did (a browser runs the
/// checkpoint between them), so settle here, under the listener, before the widget edits.
fn settle_before_default(window: &mut Window, cx: &mut App) {
    if sk_runtime::bridge::drain_owed() {
        js_wave_app(window, cx, || true);
    }
}

fn intent_payload(intent: &sk_elements::InputIntent) -> EventPayload<'static> {
    EventPayload {
        a: f64::from(intent.input_type.code()),
        b: if intent.input_type.composing() {
            1.0
        } else {
            0.0
        },
        ..Default::default()
    }
}

fn path_listens(t: &Tree, id: u32, code: u32) -> bool {
    let bit = 1u64 << code;
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { return false };
        if n.listeners & bit != 0 {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        cur = n.parent;
    }
    false
}

/// [`js_dispatch_app`] for an event with a string beside its payload (InputEvent / CompositionEvent
/// `data`), passed as the dispatcher's eighth argument; `None` reads as `null`.
fn js_dispatch_text(
    id: u32,
    event: u32,
    payload: &EventPayload,
    data: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    js_wave_app(window, cx, || {
        let value = data.and_then(|s| {
            sk_runtime::JsRuntime::try_with(|rt| {
                let v = rt.ctx().string(s);
                // Held across the dispatch, which runs arbitrary JS.
                rt.ctx().protect(v);
                v
            })
        });
        let proceed = crate::dispatch_event_with(id, event, payload, value);
        if let Some(v) = value {
            sk_runtime::JsRuntime::try_with(|rt| rt.ctx().unprotect(v));
        }
        proceed
    })
}

/// DOM order for a focus move: `blur` on the old node, then `focus` on the new; neither bubbles.
/// While the old node hears `blur` nothing is focused; a handler that takes focus then abandons
/// this move (Chromium's `SetFocusedElement`). Focus moving to the root (`body`) is a blur with no
/// `focus`.
fn focus_transition<V: 'static>(new_focus: u32, window: &mut Window, cx: &mut Context<V>) {
    // gpui calls focus listeners inside `Window::draw`, after its frame swap: no forced layout may
    // draw under them (`crate::in_input_dispatch`).
    let _in_draw = crate::InputDispatch::inside_gpui();
    focus_transition_app(new_focus, window, cx);
    // A focus move can change what `:focus` / `:focus-visible` paint with no JS handler (an
    // `<input>` folds focus rungs at build time), so the repaint cannot be left to `flush_changes`.
    cx.notify();
}

/// [`focus_transition`]'s events for a caller with no view: a press's focus ([`apply_press_focus`])
/// reports here before the next platform event, not from the observer a frame later.
fn focus_transition_app(new_focus: u32, window: &mut Window, cx: &mut App) {
    // Per window: with one slot, window B focusing an input fired `blur` on window A's. gpui hands
    // the observer the window the focus moved in.
    let k = windows::key(window);
    let new_focus = if new_focus != 0 && new_focus == window_root(k) {
        0
    } else {
        new_focus
    };
    let old = windows::focused_node(k);
    if old == new_focus {
        return;
    }
    if old != 0 {
        set_focus_slot(k, 0);
        let payload = EventPayload {
            a: new_focus as f64,
            ..Default::default()
        };
        js_dispatch_app(old, tree::EV_BLUR, &payload, window, cx);
        // `el.focus()` from a handler set the slot (`js_focus`); `el.blur()` cannot have, since
        // nothing is focused.
        if windows::focused_node(k) != 0 {
            return;
        }
    }
    set_focus_slot(k, new_focus);
    if new_focus != 0 {
        crate::selection::focus_moved(new_focus);
        let payload = EventPayload {
            a: old as f64,
            ..Default::default()
        };
        js_dispatch_app(new_focus, tree::EV_FOCUS, &payload, window, cx);
    }
}

/// Moves the window's focus slot and re-renders the elements whose ancestor-focus rules watch it.
fn set_focus_slot(k: windows::WinKey, id: u32) {
    let old = windows::focused_node(k);
    windows::set_focused_node(k, id);
    if old != id {
        tree::with_tree(|t| t.touch_focus_watchers(old, id));
    }
}

fn window_root(k: windows::WinKey) -> u32 {
    match windows::root_of(k) {
        0 => tree::read_tree(|t| t.root),
        popped => popped,
    }
}

/// Subscribes `id` to its handle's focus changes, once. gpui fires these from the draw that first
/// shows the new focus state, i.e. outside any tree borrow.
fn watch_focus<V: 'static>(
    id: u32,
    handle: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    // Per window: `cx.on_focus` subscribes against one window, so a node first built in the primary
    // had no subscription in the pop-out that later painted it and its `focus`/`blur` never fired
    // there.
    let k = windows::key(window);
    if windows::of(k, |w| w.focus_subs.contains_key(&id)) {
        return;
    }
    let sub = cx.on_focus(handle, window, move |_, window, cx| {
        focus_transition(id, window, cx);
    });
    windows::of(k, |w| w.focus_subs.insert(id, sub));
    // Focus leaving the window's tree entirely (focused node removed, or something unfocusable
    // clicked) must blur the last node, in the window it left.
    if windows::of(k, |w| w.focus_lost.is_some()) {
        return;
    }
    let sub = cx.on_focus_lost(window, |_, window, cx| {
        focus_transition(0, window, cx);
    });
    windows::of(k, |w| w.focus_lost = Some(sub));
}

/// The overlay scrollbar for one `overflow: auto | scroll` box. Built even when
/// `scrollbar-width: none` hides it: that means "do not draw", not "do not scroll", and its wheel
/// routing makes `overscroll-behavior` work.
fn scrollbar_for<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    handle: &ScrollHandle,
    window: &Window,
    auto_hide: bool,
    cx: &mut Context<V>,
) -> sk_elements::Scrollbar {
    use sk_css::{OverscrollBehavior, ScrollbarWidth};

    let extras = node.computed.as_ref().map(|cs| &cs.extras);
    let text_color = node
        .computed
        .as_ref()
        .and_then(|cs| cs.color)
        .unwrap_or_else(|| gpui::hsla(0., 0., 0.45, 1.0));
    let mut style = sk_elements::ScrollbarStyle::from_text_color(text_color);
    match extras.and_then(|x| x.scrollbar_width) {
        Some(ScrollbarWidth::Thin) => style = style.thin(),
        Some(ScrollbarWidth::None) => style = style.hidden(),
        Some(ScrollbarWidth::Auto) | None => {}
    }
    if let Some((thumb, track)) = extras.and_then(|x| x.scrollbar_color) {
        style = style.with_colors(thumb, track);
    }
    // The platform's answer to "does a scrollbar fade away" (Windows
    // `UISettings::AutoHideScrollBars`).
    style = style.always_visible(!auto_hide);

    let chains = |b: Option<OverscrollBehavior>| b.map(|b| b.chains()).unwrap_or(true);
    let overscroll = sk_elements::Overscroll {
        x_chains: chains(extras.and_then(|x| x.overscroll_x)),
        y_chains: chains(extras.and_then(|x| x.overscroll_y)),
    };

    // Axes this box scrolls, off the final computed overflow (what made `track_scroll` happen).
    let overflow = node
        .computed
        .as_ref()
        .map(|cs| cs.base.overflow.clone())
        .unwrap_or_default();
    // The root is the viewport and always scrolls both ways, as in `build_node`.
    let scrolls_x = overflow.x == Some(Overflow::Scroll) || id == t.root;
    let scrolls_y = overflow.y == Some(Overflow::Scroll) || id == t.root;

    // The DOM events this element fires itself, on two paths gpui's own listener never covers: a
    // wheel the router took over (it stops propagation) and a thumb drag or track click. The
    // `wheel` hook exists only when the container or an ancestor listens.
    let sb = sk_elements::Scrollbar::new(("sk-scrollbar", id), handle.clone(), style)
        .axes(scrolls_x, scrolls_y)
        .overscroll(overscroll)
        .line_height(window.line_height());
    let _ = cx;
    let sb = if wheel_listener_above(t, id) {
        sb.on_wheel(move |delta, window, cx| {
            if !claim_wheel(cx) {
                return false;
            }
            let payload = EventPayload {
                a: f32::from(delta.x) as f64,
                b: f32::from(delta.y) as f64,
                c: 0.0,
                d: mods_bits(&window.modifiers()),
                key: None,
            };
            // `js_dispatch_app` returns "the app did not call preventDefault()", so the hook's
            // answer is its negation.
            let target = crate::pointer::target_or(window, id);
            let at = window.mouse_position();
            crate::pointer_at_next_dispatch(f64::from(at.x), f64::from(at.y));
            !js_dispatch_app(target, tree::EV_WHEEL, &payload, window, cx)
        })
    } else {
        sb
    };
    let sb = if snaps(node) {
        let scroll = handle.clone();
        sb.on_gesture(move |g| crate::snap::note_gesture(id, &scroll, g))
    } else {
        sb
    };
    let scroll = handle.clone();
    sb.on_scrolled(move |window, cx| report_scroll_later(id, &scroll, window, cx))
}

fn snaps(node: &crate::tree::Node) -> bool {
    node.computed
        .as_ref()
        .and_then(|cs| cs.extras.snap.snap_type)
        .is_some_and(|s| s.snaps())
}

/// [`report_scroll`] once gpui's dispatch is over, from the window the scroll happened in, off
/// input. In a browser `scroll` follows the `wheel` that caused it and a cancelled `wheel` causes
/// none; gpui moves the box before its `wheel` listener and `undo_wheel_scroll` puts it back, so
/// reporting later keeps that order and the dedup drops a box put back.
fn report_scroll_later(id: u32, handle: &ScrollHandle, window: &mut Window, cx: &mut App) {
    let handle = handle.clone();
    window.defer(cx, move |window, cx| {
        crate::off_input(|| report_scroll(id, &handle, window, cx));
    });
}

thread_local! {
    /// The DOM `wheel` of the event being dispatched has fired. gpui runs every listening box's
    /// wheel listener innermost first and the JS side propagates from the target, so without the
    /// claim an outer `@wheel` ran once per listening box.
    static WHEEL_CLAIMED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug)]
pub enum KeyClick {
    Click {
        id: u32,
        at: Point<gpui::Pixels>,
        modifiers: gpui::Modifiers,
        armed: bool,
        key: &'static str,
    },
    Arm(u32),
}

/// Does a browser click this element from Enter (`space` false) or Space?
/// A `<div role=button tabindex=0>` has no activation behaviour: it handles its keys itself, and a
/// host click would run its action twice.
fn keyboard_activates(id: u32, space: bool) -> bool {
    tree::read_tree(|t| {
        let Some(node) = t.node(id) else { return false };
        let ty = || {
            t.prop(id, "type")
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_default()
        };
        match t.name(node.tag) {
            "button" | "summary" => true,
            "a" | "area" => !space && t.prop(id, "href").is_some(),
            "input" => match ty().as_str() {
                "button" | "submit" | "reset" | "image" | "color" => true,
                "checkbox" | "radio" => space,
                _ => false,
            },
            _ => false,
        }
    })
}

/// A browser clicks as the default action of the key event: after every handler, and only if none
/// cancelled it. So the root's last key listener settles it, and a deferred effect does for a
/// window whose root has none.
fn owe_key_click(step: KeyClick, window: &mut Window, cx: &mut App) {
    windows::here(window, |w| w.key_click = Some(step));
    window.defer(cx, |window, cx| {
        crate::off_input(|| resolve_key_click(window, cx))
    });
}

fn resolve_key_click(window: &mut Window, cx: &mut App) {
    let Some(step) = windows::here(window, |w| w.key_click.take()) else {
        return;
    };
    let prevented = windows::here(window, |w| w.key_prevented);
    match step {
        KeyClick::Arm(id) => {
            if !prevented {
                windows::here(window, |w| w.space_armed = Some(id));
            }
        }
        KeyClick::Click {
            id,
            at,
            modifiers,
            armed,
            key,
        } => {
            let go =
                windows::here(window, |w| !armed || w.space_armed.take() == Some(id)) && !prevented;
            if go {
                let payload = EventPayload {
                    key: Some(key),
                    ..mouse_payload_n(at, 0.0, &modifiers, 0)
                };
                js_dispatch_app(id, tree::EV_CLICK, &payload, window, cx);
            }
        }
    }
}

fn bind_root_key_clicks<E: gpui::InteractiveElement>(e: E) -> E {
    e.on_key_down(|_: &KeyDownEvent, window, cx| resolve_key_click(window, cx))
        .on_key_up(|_: &KeyUpEvent, window, cx| resolve_key_click(window, cx))
}

fn claim_wheel(cx: &mut App) -> bool {
    if WHEEL_CLAIMED.with(|c| c.replace(true)) {
        return false;
    }
    cx.defer(|_| WHEEL_CLAIMED.with(|c| c.set(false)));
    true
}

/// A synthesized `wheel` propagates up the mirror tree, so an ancestor's handler is reason enough
/// to dispatch.
fn wheel_listener_above(t: &Tree, id: u32) -> bool {
    let mut cur = id;
    loop {
        let Some(node) = t.node(cur) else {
            return false;
        };
        if node.listeners & (1u64 << tree::EV_WHEEL) != 0 {
            return true;
        }
        if node.parent == 0 {
            return false;
        }
        cur = node.parent;
    }
}

pub(crate) fn report_scroll(id: u32, handle: &ScrollHandle, window: &mut Window, cx: &mut App) {
    // gpui offsets are <= 0 (content shifted up/left); CSS scrollTop/scrollLeft count down/right as
    // positive.
    let offset = handle.offset();
    let (left, top) = (-f32::from(offset.x), -f32::from(offset.y));
    let moved = SCROLL_REPORTED.with(|m| {
        let mut m = m.borrow_mut();
        if m.get(&id).copied().unwrap_or((0.0, 0.0)) == (left, top) {
            return false;
        }
        m.insert(id, (left, top));
        true
    });
    let listens = tree::read_tree(|t| {
        t.node(id)
            .is_some_and(|n| n.listeners & (1u64 << tree::EV_SCROLL) != 0)
    });
    if moved && listens {
        let payload = EventPayload {
            a: left as f64,
            b: top as f64,
            ..Default::default()
        };
        js_dispatch_app(id, tree::EV_SCROLL, &payload, window, cx);
    }
}

/// The DOM `tabindex` attribute: `>= 0` is a tab stop at that index, `< 0` is focusable but skipped
/// by `Tab`, absent is not focusable at all. gpui reads both values off the focus handle, so they
/// are stamped onto the persistent one (same `FocusId` — focus is never disturbed by this).
fn tab_index_of(t: &Tree, id: u32) -> Option<isize> {
    let raw = t.prop(id, "tabindex")?;
    match raw.trim().parse::<isize>() {
        Ok(n) => Some(n),
        Err(_) => {
            warn_compat(
                "tabindex",
                &format!("tabindex=\"{raw}\" is not an integer; the attribute was ignored"),
            );
            None
        }
    }
}

fn default_tab_index(t: &Tree, id: u32) -> Option<isize> {
    let n = t.node(id)?;
    let focusable = match t.name(n.tag) {
        "button" | "select" | "iframe" => true,
        "a" | "area" => t.prop(id, "href").is_some(),
        "summary" => summarizes_details(t, id),
        _ => false,
    } || t
        .prop(id, "contenteditable")
        .is_some_and(|v| !v.eq_ignore_ascii_case("false"));
    focusable.then_some(0)
}

fn summarizes_details(t: &Tree, id: u32) -> bool {
    let Some(parent) = t.node(id).and_then(|n| t.node(n.parent)) else {
        return false;
    };
    t.name(parent.tag) == "details"
        && parent.children.iter().copied().find(|&c| {
            t.node(c)
                .is_some_and(|k| k.kind == Kind::Element && t.name(k.tag) == "summary")
        }) == Some(id)
}

/// HTML's "actually disabled": a disableable form control with `disabled`, or one inside a disabled
/// `<fieldset>` but not its first `<legend>`.
pub(crate) fn actually_disabled(t: &Tree, id: u32) -> bool {
    let disabled = |id: u32| t.prop(id, "disabled").is_some_and(|v| v != "false");
    let Some(n) = t.node(id) else { return false };
    let tag = t.name(n.tag);
    if !matches!(
        tag,
        "button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset"
    ) {
        return false;
    }
    if disabled(id) {
        return true;
    }
    if tag == "optgroup" || tag == "option" {
        return false;
    }
    let (mut child, mut cur) = (id, n.parent);
    for _ in 0..256 {
        let Some(p) = t.node(cur) else { return false };
        if t.name(p.tag) == "fieldset" && disabled(cur) {
            let first_legend = p.children.iter().copied().find(|&c| {
                t.node(c)
                    .is_some_and(|k| k.kind == Kind::Element && t.name(k.tag) == "legend")
            });
            return first_legend != Some(child);
        }
        (child, cur) = (cur, p.parent);
    }
    false
}

fn focus_handle_for<V: 'static>(
    id: u32,
    tab_index: Option<isize>,
    cx: &mut Context<V>,
) -> FocusHandle {
    FOCUS_HANDLES.with(|m| {
        let mut map = m.borrow_mut();
        let handle = map.entry(id).or_insert_with(|| cx.focus_handle());
        let want_index = tab_index.unwrap_or(0);
        let want_stop = tab_index.is_some_and(|n| n >= 0);
        if handle.tab_index != want_index || handle.tab_stop != want_stop {
            *handle = handle.clone().tab_index(want_index).tab_stop(want_stop);
        }
        handle.clone()
    })
}

pub fn set_os_chrome_in(k: WinKey, on: bool) {
    windows::of(k, |w| w.os_chrome = on);
}

pub fn set_os_chrome(on: bool) {
    set_os_chrome_in(windows::primary(), on);
}

pub fn painted_drag_region_in(k: WinKey) -> bool {
    windows::peek(k, |w| w.any_drag_region).unwrap_or(false)
}

pub fn painted_drag_region() -> bool {
    painted_drag_region_in(windows::primary())
}

pub fn control_hitboxes_in(k: WinKey) -> Vec<(u32, WindowControlArea)> {
    windows::peek(k, |w| w.control_hitboxes.clone()).unwrap_or_default()
}

pub fn control_hitboxes() -> Vec<(u32, WindowControlArea)> {
    control_hitboxes_in(windows::primary())
}

/// The caption button a click on `node` lands on, as the hit test resolves it: the nearest
/// ancestor-or-self declaring `--window-control`, with the window painting it. `None` where the OS
/// draws the caption.
pub fn window_control_at(node: u32) -> Option<(WinKey, WindowControlArea)> {
    let win = windows::window_of_node(node);
    if windows::peek(win, |w| w.os_chrome).unwrap_or(false) {
        return None;
    }
    let area = tree::read_tree(|t| {
        let mut cursor = node;
        for _ in 0..1024 {
            let n = t.node(cursor)?;
            match n.computed.as_ref().and_then(|c| c.extras.window_control) {
                Some(WindowControl::Minimize) => return Some(WindowControlArea::Min),
                Some(WindowControl::Maximize) => return Some(WindowControlArea::Max),
                Some(WindowControl::Close) => return Some(WindowControlArea::Close),
                Some(WindowControl::None) => return None,
                None => {}
            }
            if n.parent == 0 || windows::window_of_root(cursor).is_some() {
                return None;
            }
            cursor = n.parent;
        }
        None
    })?;
    Some((win, area))
}

/// Folds the one inherited property sk-css carries (window control/drag) and decides whether this
/// node needs a gpui window-control hitbox. One is emitted only where the effective value differs
/// from the parent's: the hit test walks outward to the nearest hitbox, so a 30-node titlebar needs
/// two or three.
fn window_control_of(
    extras: &sk_css::Extras,
    parent: Option<WindowDrag>,
) -> (Option<WindowDrag>, Option<WindowControlArea>) {
    let effective = extras.window_drag.or(parent);
    // A caption button wins outright (implicitly `window-drag: none`).
    let area = match extras.window_control {
        Some(WindowControl::Minimize) => Some(WindowControlArea::Min),
        Some(WindowControl::Maximize) => Some(WindowControlArea::Max),
        Some(WindowControl::Close) => Some(WindowControlArea::Close),
        // `--window-control: none` cancels an earlier role but does not by itself carve the element
        // out of an ancestor's drag region; `window-drag: none` does.
        Some(WindowControl::None) | None => match (effective, parent) {
            (Some(WindowDrag::Move), p) if p != Some(WindowDrag::Move) => {
                Some(WindowControlArea::Drag)
            }
            (Some(WindowDrag::None), Some(WindowDrag::Move)) => Some(WindowControlArea::NoDrag),
            _ => None,
        },
    };
    (effective, area)
}

fn advise_interactive_in_drag_region(
    t: &Tree,
    node: &crate::tree::Node,
    extras: &sk_css::Extras,
    effective: Option<WindowDrag>,
) {
    if effective != Some(WindowDrag::Move) {
        return;
    }
    if extras.window_drag == Some(WindowDrag::None) || extras.window_control.is_some() {
        return;
    }
    const POINTER: u64 =
        (1 << tree::EV_CLICK) | (1 << tree::EV_MOUSEDOWN) | (1 << tree::EV_MOUSEUP);
    if node.listeners & POINTER == 0 {
        return;
    }
    let tag = t.name(node.tag);
    warn_compat(
        "window-drag/interactive",
        &format!(
            "<{tag}> has a pointer listener inside a `window-drag: move` region without \
             `window-drag: none`. In a browser and in Electron a drag region swallows ALL \
             pointer events, so this element would be dead there; Skathi still delivers \
             the click. Add `window-drag: none` (or `--window-control: …` for a caption \
             button)."
        ),
    );
}

/// gpui performs min/max/close itself on `WM_NCLBUTTONUP` and also forwards a synthesised click
/// into the tree, so a control with both `--window-control` and a click handler acts twice.
fn advise_double_action(t: &Tree, node: &crate::tree::Node, extras: &sk_css::Extras) {
    let role = match extras.window_control {
        Some(WindowControl::Minimize) => "minimize",
        Some(WindowControl::Maximize) => "maximize",
        Some(WindowControl::Close) => "close",
        _ => return,
    };
    if node.listeners & ((1 << tree::EV_CLICK) | (1 << tree::EV_MOUSEUP)) == 0 {
        return;
    }
    let tag = t.name(node.tag);
    warn_compat(
        "window-control/double-action",
        &format!(
            "<{tag}> declares `--window-control: {role}` AND a click handler: the window will act \
             TWICE — the OS performs the action on the non-client click and the handler runs on \
             the forwarded one. Use one or the other (`--window-control` also gives you Windows \
             11 snap layouts)"
        ),
    );
}

fn with_control_area<E: gpui::InteractiveElement>(
    e: E,
    id: u32,
    area: Option<WindowControlArea>,
    win: WinKey,
) -> E {
    let Some(area) = area else { return e };
    // Mode 3: the OS owns the caption. A hitbox here would make `DefWindowProc` answer `HTCAPTION`
    // for client area, and the content under it would drag the window instead of receiving clicks.
    if windows::peek(win, |w| w.os_chrome).unwrap_or(false) {
        warn_compat(
            "window-drag/os-chrome",
            "`window-drag` / `--window-control` do nothing under `titleBarStyle: \"default\"` — \
             the OS draws the caption and owns its buttons. Drop the custom titlebar under \
             `@media (display-mode: standalone)`, or switch to `titleBarStyle: \"hidden\"` in \
             sk.config.json",
        );
        return e;
    }
    windows::of(win, |w| {
        if area == WindowControlArea::Drag {
            w.any_drag_region = true;
        }
        w.control_hitboxes.push((id, area));
    });
    e.window_control_area(area)
}

pub fn build_root<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    let _profile_window = sk_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
    let _profile = sk_runtime::redjs::profiling::span("render", "Build native element tree");
    BUILDS.with(|c| c.set(c.get().wrapping_add(1)));
    LAYOUTS.with(|c| c.set(c.get().wrapping_add(1)));
    // One clock read per frame: nodes animating together must sample the same instant.
    let now = std::time::Instant::now();
    let span = crate::perf::Span::start();
    BUILT.with(|c| c.set(0));
    crate::aria::reset_frame();
    sk_elements::scrollbar::reset_frame();
    advise_elliptical_corners();
    let win = windows::key(window);
    // The window painting `Tree::root` is the primary by definition; declared here, not passed in
    // by the host, so the two cannot disagree.
    if !windows::is_primary(win) {
        windows::set_primary(win);
    }
    reset_bounds(win);
    crate::sticky::begin_frame(win);
    crate::iframe::drawing(win);
    windows::of(win, |w| {
        w.any_drag_region = false;
        w.control_hitboxes.clear();
    });
    arm_pick_hitboxes(window);
    crate::cache::begin_frame(window, now);
    crate::cull::begin_frame(window, now);
    crate::live_state::begin_draw(focused_node(window, cx), window);
    reset_stacking();
    let el = tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            crate::view_transition::begin_build(t, window);
            // The root inherits nothing: `window-drag`'s initial value is `none`.
            if let Some(id) =
                crate::fullscreen::current(t).filter(|id| windows::window_of_node_in(t, *id) == win)
            {
                build_fullscreen(t, id, t.root, window, cx, now)
            } else {
                build_node(
                    t,
                    t.root,
                    t.root,
                    window,
                    cx,
                    now,
                    None,
                    None,
                    None,
                    None,
                    Stacking::default(),
                )
            }
        }
    });
    crate::perf::note_build(span.ms(), BUILT.with(|c| c.get()));
    el
}

fn build_fullscreen<V: 'static>(
    t: &Tree,
    id: u32,
    document_root: u32,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
) -> AnyElement {
    use gpui::Refineable as _;
    let mut ancestors = Vec::new();
    let mut cursor = if id == document_root {
        0
    } else {
        t.node(id).map_or(0, |node| node.parent)
    };
    while let Some(node) = t.node(cursor) {
        ancestors.push(cursor);
        if cursor == document_root {
            break;
        }
        cursor = node.parent;
    }
    let mut inherited = Inherited::default();
    let mut text = gpui::TextStyleRefinement::default();
    for ancestor in ancestors.into_iter().rev() {
        if let Some(node) = t.node(ancestor) {
            let extras = tree::animated_extras(ancestor, now).unwrap_or_else(|| extras_of(node));
            inherited.case = extras.text_transform.or(inherited.case);
            inherited.ws = extras.white_space.or(inherited.ws);
            if let Some(computed) = &node.computed {
                inherited.align = computed.base.text.text_align.or(inherited.align);
                text.refine(&computed.base.text);
            }
        }
    }
    let handle = focus_handle_for(document_root, None, cx);
    let mut document = div()
        .id("sk-fullscreen-document")
        .on_pinch(crate::pinch::handle)
        .size_full()
        .bg(gpui::black())
        .track_focus(&handle);
    document.style().text = text;
    if id != document_root {
        if let Some(url) = t
            .node(document_root)
            .and_then(|node| node.computed.as_ref())
            .and_then(|computed| computed.extras.filter_url.as_ref())
        {
            document.style().vn_color_matrix = crate::filter::resolve(t, url, document_root);
        }
    }
    if let Some(root) = t.node(document_root) {
        let mask = root_mask(t, root) | (1 << tree::EV_KEYDOWN);
        document = bind_events(document, document_root, mask, None, false, true, cx);
    }
    let element = document
        .child(build_node(
            t,
            id,
            id,
            window,
            cx,
            now,
            None,
            inherited.case,
            inherited.ws,
            inherited.align,
            Stacking::default(),
        ))
        .into_any_element();
    probe_at(document_root, true, element)
}

/// Builds one node's subtree as the root of a second window's element tree, with the same
/// per-window bookkeeping as [`build_root`] and the root treatment (100% size, `Overflow::Scroll`,
/// `track_focus`).
pub fn build_subtree<V: 'static>(id: u32, window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    let _profile_window = sk_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
    let _profile = sk_runtime::redjs::profiling::span("render", "Build secondary window tree");
    LAYOUTS.with(|c| c.set(c.get().wrapping_add(1)));
    let now = std::time::Instant::now();
    arm_pick_hitboxes(window);
    let win = windows::key(window);
    // Registered from the build itself, so "which window paints this node" cannot drift from what
    // is on screen; `windows::window_of_node` routes atlas tiles, `@media` environments and hit
    // tests from it.
    windows::set_painted_root(win, id);
    reset_bounds(win);
    crate::sticky::begin_frame(win);
    crate::iframe::drawing(win);
    crate::aria::reset_frame();
    sk_elements::scrollbar::reset_frame();
    windows::of(win, |w| {
        w.any_drag_region = false;
        w.control_hitboxes.clear();
    });
    crate::cache::begin_frame(window, now);
    crate::cull::begin_frame(window, now);
    crate::live_state::begin_draw(focused_node(window, cx), window);
    reset_stacking();
    tree::read_tree(|t| {
        if t.node(id).is_none() {
            return Empty.into_any_element();
        }
        // A pop-out keeps its DOM ancestry even though it paints elsewhere.
        let mut element = if let Some(fullscreen) =
            crate::fullscreen::current(t).filter(|node| windows::window_of_node_in(t, *node) == win)
        {
            build_fullscreen(t, fullscreen, id, window, cx, now)
        } else {
            build_node(
                t,
                id,
                id,
                window,
                cx,
                now,
                None,
                None,
                None,
                None,
                Stacking::default(),
            )
        };
        let mut ancestor = t.node(id).map_or(0, |node| node.parent);
        while let Some(node) = t.node(ancestor) {
            if let Some(matrix) = node
                .computed
                .as_ref()
                .and_then(|computed| computed.extras.filter_url.as_ref())
                .and_then(|url| crate::filter::resolve(t, url, ancestor))
            {
                let mut layer = div().size_full();
                layer.style().vn_color_matrix = Some(matrix);
                element = layer.child(element).into_any_element();
            }
            ancestor = node.parent;
        }
        element
    })
}

/// Everything `build_node` threads down rather than folding into a node's style. A struct because
/// the retained-render cache compares it: an ancestor change alters a cached subtree's pixels
/// without touching any node inside.
#[derive(Clone, Default)]
pub(crate) struct Inherited {
    pub(crate) drag: Option<WindowDrag>,
    pub(crate) case: Option<sk_css::TextTransform>,
    pub(crate) ws: Option<sk_css::WhiteSpaceMode>,
    pub(crate) align: Option<gpui::TextAlign>,
    pub(crate) stack: Stacking,
}

impl PartialEq for Inherited {
    fn eq(&self, other: &Self) -> bool {
        self.drag == other.drag
            && self.case == other.case
            && self.ws == other.ws
            && self.align == other.align
            // The stacking key is left out on purpose: its ordinals change whenever anything
            // earlier in the document does, which would stale every later boundary.
            && self.stack.deferred == other.stack.deferred
    }
}

pub(crate) fn build_cached<V: 'static>(
    id: u32,
    root: u32,
    ctx: Inherited,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let _profile_window = sk_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
    let _profile = sk_runtime::redjs::profiling::span("render", "Build cached subtree");
    let now = std::time::Instant::now();
    let span = crate::perf::Span::start();
    let before = BUILT.with(|c| c.get());
    // A boundary renders inside gpui's prepaint, after the build that placed it: no context of that
    // build is open.
    let open = OPEN_CONTEXTS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let (el, ids) = tree::read_tree(|t| {
        let el = build_node(
            t, id, root, window, cx, now, ctx.drag, ctx.case, ctx.ws, ctx.align, ctx.stack,
        );
        // The whole subtree: when this boundary replays nothing else carries a nested boundary's
        // rectangles (`cache::Entry::ids`).
        let mut ids = Vec::new();
        crate::cache::subtree_ids(t, id, &mut ids);
        (el, ids)
    });
    OPEN_CONTEXTS.with(|c| *c.borrow_mut() = open);
    crate::cache::note_rendered(id, ids);
    // Build cost lands in the `build` bucket wherever it is paid, so the SK_PERF partition adds up
    // (a boundary renders inside prepaint).
    crate::perf::note_build_extra(span.ms(), BUILT.with(|c| c.get()) - before);
    el
}

fn build_node<V: 'static>(
    t: &Tree,
    id: u32,
    // The root of the build this node is part of: `t.root` in the primary window, the popped-out
    // node in a pop-out.
    root: u32,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    parent_drag: Option<WindowDrag>,
    parent_case: Option<sk_css::TextTransform>,
    parent_ws: Option<sk_css::WhiteSpaceMode>,
    parent_align: Option<gpui::TextAlign>,
    parent_stack: Stacking,
) -> AnyElement {
    let Some(node) = t.node(id) else {
        return Empty.into_any_element();
    };
    let win = windows::key(window);
    // A subtree outside every clip paints nothing: it is lowered as a box of its last size, ahead
    // of the cache (`crate::cull`).
    if !crate::fullscreen::is_active(id) {
        let inherited = || Inherited {
            drag: parent_drag,
            case: parent_case,
            ws: parent_ws,
            align: parent_align,
            stack: parent_stack.clone(),
        };
        if let Some(placeholder) = crate::cull::placeholder(t, id, node, root, inherited, win) {
            return placeholder;
        }
    }
    // A cache boundary returns the entire node (probe, stacking wrapper and all): its recording
    // already contains them (`crate::cache`).
    if crate::cache::active() && !crate::fullscreen::is_active(id) {
        let inherited = Inherited {
            drag: parent_drag,
            case: parent_case,
            ws: parent_ws,
            align: parent_align,
            stack: parent_stack.clone(),
        };
        if let Some(cached) = crate::cache::boundary(t, id, node, root, inherited, win, cx) {
            return cached;
        }
    }
    BUILT.with(|c| c.set(c.get() + 1));
    // `window-drag` is inherited and sk-css folds no inherited properties, so the fold happens
    // here, on the way down.
    let extras = tree::animated_extras(id, now).unwrap_or_else(|| extras_of(node));
    // `text-transform` inherits too: threaded down like `window-drag`.
    let case = extras.text_transform.or(parent_case);
    // `white-space` inherits too, and a text node has no declarations of its own.
    let ws = extras.white_space.or(parent_ws);
    // `text-align` inherits too and layout needs it: `apply_inline_row` turns mixed inline content
    // into a flex row, which is justified, not text-aligned.
    let align = node
        .computed
        .as_ref()
        .and_then(|cs| cs.base.text.text_align)
        .or(parent_align);
    // Where the box paints (`Stacking`): a stacking context's children are built inside it so their
    // z-indices sort inside; a z-auto `absolute` box defers in the context it is in.
    let parent_display = t
        .node(node.parent)
        .and_then(|p| p.computed.as_ref())
        .and_then(|cs| cs.base.display);
    let own_position = node.computed.as_ref().and_then(|cs| cs.base.position);
    let effect = node
        .computed
        .as_ref()
        .is_some_and(|cs| effect_context(&extras, &cs.base));
    let layer = if id == root {
        Layer::InFlow
    } else {
        layer_of(&extras, own_position, effect, parent_display)
    };
    let Entered {
        own,
        inside: stacking,
        opens,
        negative,
    } = parent_stack.enter(layer, next_stack_ordinal());
    if negative {
        note_negative_z();
    }
    // A context's children are built with it open so a negative z among them is seen
    // (`split_in_flow`).
    let opens = opens || id == root;
    let (drag, control_area) = window_control_of(&extras, parent_drag);
    advise_interactive_in_drag_region(t, node, &extras, drag);
    advise_double_action(t, node, &extras);
    advise_out_of_flow(t, id, node, &extras, windows::key(window));
    let sticky = (extras.sticky == Some(true) && id != root)
        .then(|| crate::sticky::spec(t, node, root))
        .flatten();
    let split = Cell::new(false);
    match node.kind {
        // An empty text node takes no space in a browser but gpui lays it out as a full line box,
        // and Vue anchors every Fragment with `createText("")`: leaving it in adds a blank line per
        // fragment.
        Kind::Text if node.text.is_empty() => Empty.into_any_element(),
        Kind::Text => {
            // Collapse first, then trim the block's edges: CSS's whitespace processing order.
            let collapsed =
                processed_text(&node.text, ws.unwrap_or(sk_css::WhiteSpaceMode::Normal));
            crate::fonts::want_for_text(t, node.parent, &collapsed);
            match trim_edges(t, node, id, &collapsed, ws) {
                "" => Empty.into_any_element(),
                // The plain text element unless a selection paints on it or a drag is recording
                // (`crate::selection`).
                text => {
                    let lead = text.as_ptr() as usize - collapsed.as_ptr() as usize;
                    let text =
                        crate::selection::text_element(t, id, &node.text, ws, lead, text, case);
                    if synthetic_inline_item(t, node) {
                        let mut anonymous = div();
                        anonymous.style().min_size.width = Some(gpui::px(0.0).into());
                        anonymous.child(text).into_any_element()
                    } else {
                        text
                    }
                }
            }
        }
        Kind::Anchor => Empty.into_any_element(),
        // Metadata content renders nothing in a browser; painting a runtime `<style>` (reka-ui
        // injects one) as text put a paragraph of CSS at the top of the page.
        Kind::Element if is_metadata(t.name(node.tag)) => Empty.into_any_element(),
        // `stack` moves the paint into gpui's deferred layer for boxes `Layer` defers (`position:
        // fixed`, a `z-index`, …); the window-control hitbox rides the inner element either way,
        // and the hit test follows the paint (`Stacking`).
        Kind::Element if t.name(node.tag) == "img" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_img(t, id, node, window, cx, now, control_area),
            ),
        ),
        #[cfg(feature = "lottie")]
        Kind::Element if t.name(node.tag) == "lottie" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_lottie(t, id, node, window, cx, now, control_area),
            ),
        ),
        #[cfg(feature = "video")]
        Kind::Element if t.name(node.tag) == "video" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_video(t, id, node, window, cx, now, control_area),
            ),
        ),
        #[cfg(not(all(feature = "lottie", feature = "video")))]
        Kind::Element if crate::absent::is_absent(t.name(node.tag)) => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_absent(t, id, node, window, cx, now, control_area),
            ),
        ),
        Kind::Element if t.name(node.tag) == "iframe" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_iframe(t, id, node, window, cx, now, control_area),
            ),
        ),
        #[cfg(feature = "lottie")]
        Kind::Element if t.name(node.tag) == "sk-sticker" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                sticker::build(t, id, node, window, cx, now, control_area),
            ),
        ),
        Kind::Element if t.name(node.tag) == "sk-surface" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_surface(t, id, node, window, cx, now, control_area),
            ),
        ),
        Kind::Element if t.name(node.tag) == "canvas" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                canvas::build(t, id, node, window, cx, now, control_area),
            ),
        ),
        // Inline `<svg>` is a leaf: the subtree is serialized into a document resvg rasterises, so
        // its children must not also become taffy nodes (see crate::svg).
        Kind::Element if t.name(node.tag) == "svg" => stack(
            own.clone(),
            &extras,
            placed(
                id,
                sticky,
                false,
                build_svg(t, id, node, window, cx, now, control_area),
            ),
        ),
        Kind::Element => stack_box(
            own,
            &split,
            id == root,
            &stacking,
            &extras,
            placed(id, sticky, id == root, {
                let mut d = with_control_area(div().id(id as usize), id, control_area, win);
                if let Some(cs) = &node.computed {
                    d = apply_computed_style(d, cs, id, now);
                }
                // The other half of `group-hover:`: some descendant's rule watches this element, so
                // gpui needs a hitbox and a named group entry while its subtree paints.
                if t.is_group_anchor(id) {
                    d = d.group(group_name(id));
                }
                // `role` / `aria-*` → AccessKit, so the node exists for Narrator and UI Automation.
                d = crate::aria::apply(d, t, id, node);
                advise_missing_font();
                // A `url()` layer anywhere in the stack moves the whole stack into a painter child
                // (`crate::bgimage`) so gradients and images keep CSS order; gpui keeps only the
                // colour.
                let bg_stack = extras.resolve_background_stack();
                let bg_radii = bg_stack
                    .as_ref()
                    .map(|_| padding_box_radii(d.style(), window.rem_size()));
                apply_background_layers(d.style(), &extras, bg_stack.is_some());
                apply_background_clip_text(d.style(), &extras, id as u64);
                apply_self_blur(d.style(), t, id, node, &extras);
                apply_drop_shadow(
                    d.style(),
                    &extras,
                    node.computed.as_ref().and_then(|cs| cs.color),
                    || node_prefix(t, id),
                );
                apply_mask_ring(
                    d.style(),
                    &extras,
                    bg_stack.is_some(),
                    paints_children(t, node),
                    || node_prefix(t, id),
                );
                let pseudo = node
                    .computed
                    .as_ref()
                    .is_some_and(|cs| cs.before.is_some() || cs.after.is_some());
                let leaf = bg_stack.is_none()
                    && node.text.is_empty()
                    && !pseudo
                    && !matches!(t.name(node.tag), "input" | "textarea" | "select" | "li")
                    && !paints_children(t, node);
                crate::mask::apply(d.style(), id, &extras, leaf);
                apply_inline_row(d.style(), t, node, align);
                apply_inline_item_minimum(d.style(), t, node);
                apply_inline_align(d.style(), t, node, &extras, align);
                align_control_content(d.style(), t, node);
                advise_inline_level(t, id, node, &extras, d.style());
                // The window is the root (`createApp().mount()`): the root element is the viewport and
                // fills the host's box unless a rule says otherwise. Otherwise `height: 100%` on its
                // child resolves against an indefinite parent, `overflow: auto` never scrolls and
                // `position: fixed` lands wherever the content ends.
                if id == root {
                    let style = d.style();
                    let full = gpui::Length::Definite(gpui::relative(1.0));
                    style.size.width.get_or_insert(full);
                    style.size.height.get_or_insert(full);
                    style.overflow.x.get_or_insert(Overflow::Scroll);
                    style.overflow.y.get_or_insert(Overflow::Scroll);
                    // It also stands in for `document`: gpui dispatches keystrokes down the focus path,
                    // so a listener off that path never fires, which would make every
                    // `document.addEventListener("keydown")` inert. Tracking a focus handle puts the
                    // root on the path; [`ensure_root_focus`] keeps something on it.
                    let h = focus_handle_for(id, None, cx);
                    d = d.track_focus(&h);
                    watch_focus(id, &h, window, cx);
                } else if id == crate::cache::boundary_root() {
                    // A cache boundary is laid out as a root inside the box its wrapper reserved
                    // (`Entity::cached` → `layout_as_root`), and a root with `width: auto` shrinks to
                    // fit instead of stretching to its parent. Not the viewport treatment: no
                    // `Overflow::Scroll`, no root focus handle.
                    let style = d.style();
                    let full = gpui::Length::Definite(gpui::relative(1.0));
                    style.size.width.get_or_insert(full);
                    style.size.height.get_or_insert(full);
                }
                // `overflow: scroll|auto` on either axis, read off the final style so the root's
                // viewport default counts. `track_scroll` supplies an offset that outlives the frame
                // and makes gpui insert the hitbox and wheel listener that move it.
                let scrolls = {
                    let overflow = &d.style().overflow;
                    overflow.x == Some(Overflow::Scroll) || overflow.y == Some(Overflow::Scroll)
                };
                let scroll_handle = scrolls.then(|| {
                    SCROLL_HANDLES.with(|m| m.borrow_mut().entry(id).or_default().clone())
                });
                if let Some(h) = &scroll_handle {
                    d = d.track_scroll(h);
                    // Where this container stood at build, so `getBoundingClientRect()` after a scroll
                    // and before the next paint can correct for the difference (`crate::scroll`).
                    crate::scroll::note_built(id, h);
                    crate::snap::note_built(id, snaps(node));
                }

                let mask = if id == root {
                    d = d.on_pinch(crate::pinch::handle);
                    root_mask(t, node)
                } else {
                    pointer_mask(node)
                };
                let bit = move |ev: u32| mask & (1u64 << ev) != 0;
                // `<input>` and `<textarea>` are the same native widget (only
                // `TextInputOptions::multiline` differs, see `build_input`). `<input type=file>` is
                // not: it keeps the ordinary wrapper's focus handle so Enter and Space reach it as
                // keydown/keyup.
                let is_file_input = crate::aria::is_file_input(t, id);
                let is_range = crate::range::is_range(t, id);
                let is_color = t.name(node.tag) == "input"
                    && t.prop(id, "type")
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("color"));
                let toggle = crate::toggle::kind(t, id);
                let is_input = matches!(t.name(node.tag), "input" | "textarea")
                    && !is_file_input
                    && !is_range
                    && !is_color
                    && toggle.is_none();
                // Both are tabbable by default in HTML; a plain <div> is not. A disabled file input is
                // not focusable at all.
                let file_tab =
                    is_file_input && !t.prop(id, "disabled").is_some_and(|v| v != "false");
                // So are `<button>`, `<select>`, links and the rest of HTML's focusable-by-default
                // list; an actually-disabled control is not focusable whatever its tabindex. A key or
                // focus listener makes nothing focusable (a `<span @focus>` never takes focus; a
                // container's `@keydown` hears descendants by bubbling).
                let disabled = !is_input && actually_disabled(t, id);
                let tab_index = tab_index_of(t, id)
                    .or(
                        (is_input || file_tab || is_range || is_color || toggle.is_some())
                            .then_some(0),
                    )
                    .or_else(|| default_tab_index(t, id))
                    .filter(|_| !disabled);

                let focusable = !is_input && !disabled && tab_index.is_some();
                // gpui's `.focus()` / `.focus_visible()` rungs need a tracked focus handle, so a node
                // with a `:focus` rule needs one even with no listeners or `tabindex`; it must not
                // become click-focusable though.
                let styled_focus = node
                    .computed
                    .as_ref()
                    .is_some_and(|cs| cs.needs_focus_handle());
                let handle = (focusable || (!is_input && styled_focus))
                    .then(|| focus_handle_for(id, tab_index, cx));
                if let Some(h) = &handle {
                    d = d.track_focus(h);
                    if focusable {
                        watch_focus(id, h, window, cx);
                    }
                    if let Some(cs) = &node.computed {
                        if let Some(f) = cs.focus.clone() {
                            d = d.focus(move |_| (*f).clone());
                        }
                        if let Some(f) = cs.focus_visible.clone() {
                            d = d.focus_visible(move |_| (*f).clone());
                        }
                        if let Some(f) = cs.focus_within.clone() {
                            d = d.in_focus(move |_| (*f).clone());
                        }
                    }
                }
                let click_focus = handle.clone().filter(|_| focusable);
                // `<input>` forwards its own keystrokes from inside the widget (before gpui resolves
                // editing key bindings), so the wrapper must not also listen: the editor is its
                // descendant and would deliver every key twice.
                d = bind_events(d, id, mask, click_focus, focusable, !is_input, cx);
                let is_select = t.name(node.tag) == "select";
                if is_select && !disabled {
                    d = crate::select::bind_press(d, id);
                }
                if is_range && !disabled && extras.pointer_events_none != Some(true) {
                    d = crate::range::bind_press(d, id);
                }
                // Copy/cut/paste outside a text field: the root is on every keystroke's path, and its
                // listener (bound after its keydown one, so after every keydown handler) is the last
                // stop.
                if id == root {
                    d = crate::clipboard::bind_root(d, id);
                    // Every mouse-up's wave (`bind_events`) and the keyboard `contextmenu` (Menu key,
                    // Shift+F10), bound after the clipboard keys so they also run after every key
                    // handler.
                    d = bind_root_mouse(d);
                    d = crate::selection::bind_root(d);
                    d = bind_menu_keys(d, id);
                    // A `<select>`'s keys: its popup's before the page's, a closed one's after them.
                    d = crate::select::bind_root(d);
                    d = crate::color_picker::bind_root(d);
                    d = crate::range::bind_root(d);
                    d = crate::toggle::bind_root(d);
                    d = bind_root_key_clicks(d);
                    d = crate::scroll::bind_root(d, id);
                }

                // `scroll` reports the position after the move, once the wheel's dispatch is over
                // (`report_scroll_later`): after the `wheel`, and not at all if a cancelled `wheel` put
                // the box back.
                if let Some(scroll) = scroll_handle.clone().filter(|_| bit(tree::EV_SCROLL)) {
                    d = d.on_scroll(cx.listener(move |_, _: &ScrollEvent, window, cx| {
                        report_scroll_later(id, &scroll, window, &mut **cx);
                    }));
                }

                if is_range {
                    d.style().size.width.get_or_insert(gpui::px(129.).into());
                    d.style().size.height.get_or_insert(gpui::px(16.).into());
                    d.child(crate::range::children(t, id, window))
                        .into_any_element()
                } else if is_color {
                    crate::color::defaults(
                        d.style(),
                        crate::scheme::declared(t, id, window) == Some(true),
                    );
                    d.child(crate::color::children(t, id, node, window))
                        .into_any_element()
                } else if is_input {
                    let inert = extras.pointer_events_none == Some(true);
                    let (input, focus_target) = build_input(t, id, node, tab_index, window, cx);
                    // The editor's own handle carries this node's focus identity (mouse, Tab,
                    // programmatic), so focus/blur events hang off it.
                    watch_focus(id, &focus_target, window, cx);
                    // `:focus` on an `<input>` cannot ride gpui's ladder: tracking the editor's handle
                    // on the wrapper too would enter it into the tab order twice. The rungs are folded
                    // into the wrapper's style instead, read live off that handle: one tab stop.
                    if let Some(cs) = &node.computed {
                        if focus_target.is_focused(window) {
                            use gpui::Refineable as _;
                            if let Some(f) = &cs.focus {
                                d.style().refine(f);
                            }
                            // A text field shows focus whatever moved it there, a click included, as in
                            // Chromium; anything else only after the keyboard (`crate::live_state`).
                            if let Some(f) = &cs.focus_visible {
                                if window.last_input_was_keyboard()
                                    || crate::live_state::is_text_field(t, id)
                                {
                                    d.style().refine(f);
                                }
                            }
                        }
                    }
                    // Clicks on the wrapper's padding/border focus the editor, but not while the field
                    // is inert (an `opacity-0 pointer-events-none` panel parked over the visible one
                    // must not take its press).
                    if !inert {
                        d = d.on_any_mouse_down(cx.listener(
                            move |_, _: &MouseDownEvent, window, cx| {
                                claim_press_focus(id, &focus_target, window, &mut **cx);
                            },
                        ));
                    }
                    d.child(input).into_any_element()
                } else {
                    // `::before` / `::after` are first and last children, so they take part in
                    // flex/grid layout, `gap` and text flow like a hand-written `<span>`.
                    let (before, after) = match &node.computed {
                        Some(cs) => (cs.before.clone(), cs.after.clone()),
                        None => (None, None),
                    };
                    let own_display = node.computed.as_ref().and_then(|cs| cs.base.display);
                    let mut kids: Vec<AnyElement> = Vec::new();
                    if opens {
                        open_context();
                    }
                    // The `bgimage` painter child goes first, so it paints under every real child
                    // (`crate::bgimage`).
                    if let (Some(stack), Some(radii)) = (bg_stack, bg_radii) {
                        kids.push(crate::bgimage::layer_element(id, stack, radii));
                    }
                    let background_kids = kids.len();
                    if let Some(marker) = list_marker_for(t, id, node, &extras) {
                        kids.push(marker);
                    }
                    if let Some(pb) = before {
                        crate::fonts::want_for_pseudo(t, id, &pb);
                        kids.push(pseudo_element(id, &pb, case, &stacking, own_display));
                    }
                    if is_file_input {
                        let dark = crate::scheme::declared(t, id, window) == Some(true);
                        kids.push(file_input_children(t, id, node, case, dark));
                    } else if let Some(k) = toggle {
                        let skin = extras.appearance_none != Some(true);
                        crate::toggle::defaults(d.style(), k, skin);
                        if skin {
                            kids.push(crate::toggle::skin(t, id, k, window));
                        }
                    } else if is_select {
                        let skin = extras.appearance_none != Some(true);
                        kids.extend(crate::select::closed_children(t, id, skin, case));
                        if let Some(popup) =
                            crate::select::popup_child(t, id, node, case, window, cx)
                        {
                            kids.push(popup);
                        }
                    } else if !node.text.is_empty() {
                        // Inline text from `setElementText` (an element whose only child is text).
                        let collapsed = processed_text(
                            &node.text,
                            ws.unwrap_or(sk_css::WhiteSpaceMode::Normal),
                        );
                        crate::fonts::want_for_text(t, id, &collapsed);
                        let text = trim_own_edges(t, node, &collapsed, ws);
                        if !text.is_empty() {
                            let lead = text.as_ptr() as usize - collapsed.as_ptr() as usize;
                            kids.push(crate::selection::text_element(
                                t, id, &node.text, ws, lead, text, case,
                            ));
                        }
                    } else {
                        kids.extend(laid_out_children(t, node).into_iter().map(|c| {
                            build_node(
                                t,
                                c,
                                root,
                                window,
                                cx,
                                now,
                                drag,
                                case,
                                ws,
                                align,
                                stacking.clone(),
                            )
                        }));
                    }
                    if let Some(pb) = after {
                        crate::fonts::want_for_pseudo(t, id, &pb);
                        kids.push(pseudo_element(id, &pb, case, &stacking, own_display));
                    }
                    // The overlay scrollbar and its wheel routing. Last child: its hitboxes sit above
                    // the content's and its capture-phase wheel listener registers after the
                    // container's, so the innermost scroll box gets the wheel first.
                    if let Some(h) = &scroll_handle {
                        let auto_hide = cx.should_auto_hide_scrollbars();
                        kids.push(
                            scrollbar_for(t, id, node, h, window, auto_hide, cx).into_any_element(),
                        );
                    }
                    // A negative z landed in this context: its in-flow content paints apart from its
                    // background, after the negative ones.
                    if opens && close_context() {
                        split.set(true);
                        kids = split_in_flow(kids, background_kids, &stacking);
                    }
                    if id == root {
                        kids.extend(crate::color_picker::layer(t, window, cx));
                    }
                    crate::cache::normalize_inner_root(id, d.style());
                    d.children(kids).into_any_element()
                }
            }),
        ),
    }
}

fn advise_missing_font() {
    for (family, used) in sk_css::take_font_misses() {
        crate::fonts::hold_miss(family, used);
    }
}

pub(crate) fn warn_missing_font(family: &str, used: Option<&str>) {
    let used = match used {
        Some(".SystemUIFont") => "the platform UI font".to_string(),
        Some(u) => format!("`{u}`, the first installed family in the list"),
        None => "the platform default (nothing in the list is installed)".to_string(),
    };
    warn_compat(
        &format!("missing-font:{family}"),
        &format!(
            "no font installed for `font-family: {family}` — using {used}, which has \
             different metrics, so text measures and wraps differently than where \
             `{family}` exists. No `@font-face` in the stylesheet or `FontFace` in \
             `document.fonts` declares it either. Install the family on the host, ship it \
             with an `@font-face` rule, or pick one that is installed"
        ),
    );
}

fn apply_background_clip_text(style: &mut gpui::StyleRefinement, extras: &sk_css::Extras, id: u64) {
    if let Some(sk_css::TextFill::Color(fill)) = extras.text_fill_color {
        // `currentColor` in `text-shadow` is `color`, not the fill that replaces it in gpui.
        if let (Some(color), Some(shadows)) = (style.text.color, style.text.text_shadow.as_mut()) {
            for shadow in shadows {
                shadow.color.get_or_insert(color);
            }
        }
        style.text.color = Some(fill);
    }
    if extras.background_clip_text != Some(true) {
        return;
    }
    let mut backgrounds = Vec::new();
    if let Some(background) = &style.background {
        backgrounds.push(background.color().unwrap_or_default());
    }
    if let Some(layers) = &style.background_layers {
        backgrounds.extend_from_slice(layers);
    }
    style.vn_text_fill = Some(gpui::VnTextFill {
        group_id: id,
        backgrounds,
    });
    style.background = Some(gpui::Fill::Color(gpui::transparent_black().into()));
    style.background_layers = Some(Vec::new());
}

/// The `background-image` stack folded with `background-size`/`position`/`repeat`, handed to gpui
/// bottom layer first. Runs after the cascade because Tailwind puts each in its own class.
/// `painted_by_child`: the stack holds a `url()` image and `crate::bgimage` paints all of it, so
/// gpui paints none.
fn apply_background_layers(
    style: &mut gpui::StyleRefinement,
    extras: &sk_css::Extras,
    painted_by_child: bool,
) {
    if extras.background_images.is_none() {
        return;
    }
    if painted_by_child {
        style.background_layers = Some(Vec::new());
        return;
    }
    let layers = extras.resolve_background_layers();
    if !layers.is_empty() {
        style.background_layers = Some(layers);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaddingRadii {
    lengths: gpui::Corners<gpui::AbsoluteLength>,
    relative: gpui::Corners<Option<gpui::VnCornerRadius>>,
}

impl PaddingRadii {
    pub(crate) fn resolve(
        &self,
        size: gpui::Size<gpui::Pixels>,
        rem: gpui::Pixels,
    ) -> gpui::Corners<gpui::Pixels> {
        gpui::vn_resolve_corner_radii(&self.lengths, &self.relative, size, rem)
    }

    fn is_square(&self) -> bool {
        let zero = gpui::AbsoluteLength::from(gpui::px(0.));
        let l = &self.lengths;
        self.relative == gpui::Corners::default()
            && [l.top_left, l.top_right, l.bottom_right, l.bottom_left]
                .iter()
                .all(|c| *c == zero)
    }

    fn apply(&self, style: &mut gpui::StyleRefinement) {
        let c = &mut style.corner_radii;
        (c.top_left, c.top_right) = (Some(self.lengths.top_left), Some(self.lengths.top_right));
        (c.bottom_right, c.bottom_left) = (
            Some(self.lengths.bottom_right),
            Some(self.lengths.bottom_left),
        );
        let v = &mut style.vn_corner_radii;
        (v.top_left, v.top_right) = (Some(self.relative.top_left), Some(self.relative.top_right));
        (v.bottom_right, v.bottom_left) = (
            Some(self.relative.bottom_right),
            Some(self.relative.bottom_left),
        );
    }
}

fn padding_box_radii(style: &gpui::StyleRefinement, rem: gpui::Pixels) -> PaddingRadii {
    let radius = |v: Option<gpui::AbsoluteLength>| v.map_or(0.0, |v| f32::from(v.to_pixels(rem)));
    let border = |v: Option<gpui::AbsoluteLength>| v.map_or(0.0, |v| f32::from(v.to_pixels(rem)));
    let (top, right) = (
        border(style.border_widths.top),
        border(style.border_widths.right),
    );
    let (bottom, left) = (
        border(style.border_widths.bottom),
        border(style.border_widths.left),
    );
    let inner = |r: f32, a: f32, b: f32| gpui::px((r - a.max(b)).max(0.0)).into();
    let v = &style.vn_corner_radii;
    PaddingRadii {
        lengths: gpui::Corners {
            top_left: inner(radius(style.corner_radii.top_left), top, left),
            top_right: inner(radius(style.corner_radii.top_right), top, right),
            bottom_right: inner(radius(style.corner_radii.bottom_right), bottom, right),
            bottom_left: inner(radius(style.corner_radii.bottom_left), bottom, left),
        },
        relative: gpui::Corners {
            top_left: v.top_left.flatten(),
            top_right: v.top_right.flatten(),
            bottom_right: v.bottom_right.flatten(),
            bottom_left: v.bottom_left.flatten(),
        },
    }
}

/// gpui draws corners as circles.
fn advise_elliptical_corners() {
    if gpui::vn_take_ellipse_drawn_as_circle() {
        warn_compat(
            "elliptical-border-radius",
            "an elliptical corner (`border-radius: 50%` on a box that is not square, or an \
             `a / b` radius) is drawn as a circle of the smaller radius: a pill where a \
             browser draws an ellipse. Square boxes and equal radii are exact",
        );
    }
}

/// A replaced element's picture, clipped to its rounded corners. Wrapped in a full-size `overflow:
/// hidden` box rather than clipping the element, which would zero its automatic minimum size in a
/// flex row.
pub(crate) fn clip_replaced(
    picture: gpui::AnyElement,
    outer: &gpui::StyleRefinement,
    rem: gpui::Pixels,
) -> gpui::AnyElement {
    let radii = padding_box_radii(outer, rem);
    if radii.is_square() {
        return picture;
    }
    let mut clip = gpui::div().size_full().overflow_hidden().child(picture);
    radii.apply(clip.style());
    clip.style().vn_corner_shapes = outer.vn_corner_shapes.clone();
    clip.into_any_element()
}

pub(crate) const INLINE_LEVEL_TAGS: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "br", "button", "cite", "code", "data", "dfn", "em", "i",
    "img", "input", "kbd", "label", "mark", "output", "q", "rp", "rt", "ruby", "s", "samp",
    "select", "small", "span", "strong", "sub", "sup", "svg", "textarea", "time", "u", "var",
    "wbr",
];

/// Mixed inline content (a text node next to a `<span>`) laid out as one line: taffy has no line
/// box, so such a block would break the line mid-sentence. It becomes a wrapping, baseline-aligned
/// flex row, with `text-align` re-expressed as `justify-content` (the inherited value, hence the
/// threading in `build_node`). Text wraps inside its own run, not across element boundaries.
fn apply_inline_row(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
    align: Option<gpui::TextAlign>,
) {
    if !inline_row_applies(t, node, style.display) {
        return;
    }
    style.display = Some(gpui::Display::Flex);
    style.flex_direction = Some(gpui::FlexDirection::Row);
    style.flex_wrap = Some(gpui::FlexWrap::Wrap);
    style.align_items.get_or_insert(gpui::AlignItems::Baseline);
    if style.justify_content.is_none() {
        style.justify_content = match align {
            Some(gpui::TextAlign::Center) => Some(gpui::JustifyContent::Center),
            Some(gpui::TextAlign::Right) => Some(gpui::JustifyContent::End),
            _ => None,
        };
    }
}

// A synthesized inline row uses flex only because taffy has no inline formatting context; its text
// must not inherit a flex item's min-content floor: `overflow-wrap: break-word` excludes emergency
// breaks from min-content, so one long word would paint outside the paragraph.
fn synthetic_inline_item(t: &Tree, node: &crate::tree::Node) -> bool {
    let Some(parent) = t.node(node.parent) else {
        return false;
    };
    let display = parent
        .computed
        .as_ref()
        .and_then(|style| style.base.display);
    inline_row_applies(t, parent, display)
}

fn apply_inline_item_minimum(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
) {
    let declared = node
        .computed
        .as_ref()
        .and_then(|computed| computed.base.display);
    if synthetic_inline_item(t, node)
        && !matches!(
            declared,
            Some(gpui::Display::Flex) | Some(gpui::Display::Grid)
        )
        && style.size.width.is_none()
    {
        style.min_size.width.get_or_insert(gpui::px(0.0).into());
    }
}

fn inline_row_applies(t: &Tree, node: &crate::tree::Node, display: Option<gpui::Display>) -> bool {
    // An author-declared display is authority (`display: flex` on a paragraph means flex).
    if !matches!(display, None | Some(gpui::Display::Block)) {
        return false;
    }
    let kids = laid_out_children(t, node);
    if kids.len() < 2 {
        return false;
    }
    let mut has_text = false;
    let mut has_element = false;
    for &child in &kids {
        let Some(kid) = t.node(child) else {
            return false;
        };
        match kid.kind {
            Kind::Text => has_text = true,
            Kind::Element => {
                if !inline_level_element(t, kid) {
                    return false;
                }
                has_element = true;
            }
            Kind::Anchor => {}
        }
    }
    has_text || has_element
}

fn inline_level_element(t: &Tree, kid: &crate::tree::Node) -> bool {
    if !INLINE_LEVEL_TAGS.contains(&t.name(kid.tag)) {
        return false;
    }
    // The author can opt a `<span>` out with `display: block`, but `display: inline` must not
    // count: the mapper folds every inline outer type onto its block-level equivalent, so it
    // arrives as `Some(Block)` and would opt the whole row out.
    let declared = kid.computed.as_ref().and_then(|cs| cs.base.display);
    let inline_level = kid
        .computed
        .as_ref()
        .is_some_and(|cs| cs.extras.inline_level == Some(true));
    inline_level
        || !matches!(
            declared,
            Some(gpui::Display::Block) | Some(gpui::Display::Grid)
        )
}

const COLLAPSIBLE_WS: [char; 5] = [' ', '\t', '\n', '\r', '\u{c}'];

fn is_blank(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| COLLAPSIBLE_WS.contains(&c))
}

fn white_space_at(t: &Tree, mut id: u32) -> sk_css::WhiteSpaceMode {
    while let Some(n) = t.node(id) {
        if let Some(ws) = n.computed.as_ref().and_then(|cs| cs.extras.white_space) {
            return ws;
        }
        if n.parent == 0 || n.parent == id {
            break;
        }
        id = n.parent;
    }
    sk_css::WhiteSpaceMode::Normal
}

/// A text node of collapsible white space only renders nothing (css-text-3 §4.1.1, CSS 2 §9.2.2.1):
/// in a flex or grid container, beside a block-level box, or at a line's start or end. Between two
/// pieces of inline content it is the word gap.
fn blank_collapses_away(t: &Tree, id: u32, node: &crate::tree::Node) -> bool {
    if !is_blank(&node.text) {
        return false;
    }
    let mode = white_space_at(t, node.parent);
    if !mode.collapses_spaces() || (mode.preserves_newlines() && node.text.contains('\n')) {
        return false;
    }
    let Some(parent) = t.node(node.parent) else {
        return false;
    };
    let display = parent.computed.as_ref().and_then(|cs| cs.base.display);
    if matches!(
        display,
        Some(gpui::Display::Flex) | Some(gpui::Display::Grid)
    ) {
        return true;
    }
    if parent.kind == Kind::Element && INLINE_LEVEL_TAGS.contains(&t.name(parent.tag)) {
        return false;
    }
    let content = |c: u32| {
        !popped_out(c)
            && t.node(c).is_some_and(|n| match n.kind {
                Kind::Anchor => false,
                Kind::Text => !n.text.is_empty() && !is_blank(&n.text),
                Kind::Element => true,
            })
    };
    let mut before = false;
    let mut after = false;
    let mut seen_self = false;
    for &c in &parent.children {
        if c == id {
            seen_self = true;
            continue;
        }
        if !content(c) {
            continue;
        }
        let Some(kid) = t.node(c) else { continue };
        if kid.kind == Kind::Element && !inline_level_element(t, kid) {
            return true;
        }
        if seen_self {
            after = true;
        } else {
            before = true;
        }
    }
    !(before && after)
}

/// `text-align` on a block container centres its inline-level children with auto margins (the
/// block-layout spelling of `text-align: center` for a definite-width box); an author's margin
/// stays authoritative. Only in a block container: in flex or grid, auto margins take free space
/// before `justify-content` sees it.
fn apply_inline_align(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
    extras: &sk_css::Extras,
    align: Option<gpui::TextAlign>,
) {
    if extras.inline_level != Some(true) {
        return;
    }
    if parent_is_flex_or_grid(t, node) {
        return;
    }
    match align {
        Some(gpui::TextAlign::Center) => {
            style.margin.left.get_or_insert(gpui::Length::Auto);
            style.margin.right.get_or_insert(gpui::Length::Auto);
        }
        Some(gpui::TextAlign::Right) => {
            style.margin.left.get_or_insert(gpui::Length::Auto);
        }
        _ => {}
    }
}

/// In block layout, centre the control's label vertically while it fits in the content box.
fn align_control_content(style: &mut gpui::StyleRefinement, t: &Tree, node: &crate::tree::Node) {
    if style.align_content.is_some() || !matches!(t.name(node.tag), "button" | "select") {
        return;
    }
    let declared = node.computed.as_ref().and_then(|cs| cs.base.display);
    if matches!(declared, None | Some(gpui::Display::Block)) {
        style.align_content = Some(gpui::AlignContent::SafeCenter);
    }
}

fn parent_is_flex_or_grid(t: &Tree, node: &crate::tree::Node) -> bool {
    let Some(parent) = t.node(node.parent) else {
        return false;
    };
    let display = parent.computed.as_ref().and_then(|cs| cs.base.display);
    match display {
        Some(gpui::Display::Flex) | Some(gpui::Display::Grid) => true,
        _ => inline_row_applies(t, parent, display),
    }
}

/// `filter: blur(<r>)` on a leaf box with a solid background: repaint the box as its own drop
/// shadow. gpui's `blur_radius` and CSS `blur()` are both a Gaussian sigma (1:1). gpui's shadow
/// shader only masks inset shadows, so a drop shadow fills all the way through, which is why the
/// box's own background is cleared. Boxes with children or gradients keep sharp edges and are
/// advised.
fn apply_self_blur(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node_id: u32,
    node: &crate::tree::Node,
    extras: &sk_css::Extras,
) {
    let Some(radius) = extras.blur.filter(|r| *r > 0.0) else {
        return;
    };
    let is_leaf = node.text.is_empty() && laid_out_children(t, node).is_empty();
    let solid = style
        .background
        .as_ref()
        .and_then(|fill| fill.color())
        .and_then(|bg| bg.as_solid());
    match (is_leaf, solid) {
        (true, Some(color)) if !color.is_transparent() => {
            style.background = Some(gpui::Fill::Color(gpui::transparent_black().into()));
            style.box_shadow = Some(vec![gpui::BoxShadow {
                color,
                offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                blur_radius: gpui::px(radius),
                spread_radius: gpui::px(0.0),
                inset: false,
            }]);
            // The shadow is the box here, so its middle stays filled (a CSS outer shadow is cut out
            // of the border box).
            style.vn_shadow_under_box = Some(true);
        }
        _ => warn_compat_at(
            "filter-blur-shape",
            || node_prefix(t, node_id),
            "`filter: blur()` is honoured only on a LEAF box with a solid background colour \
             (v1) — gpui has no post-processing pass over an element's own painted output, so \
             the blur is drawn by repainting that box as its own Gaussian drop shadow, which \
             is exact for a solid rounded rect and impossible for anything with children, text \
             or a gradient. This element kept its sharp edges",
        ),
    }
}

/// `filter: drop-shadow(...)` on a box whose painted output is its own rectangle: gpui's shadow
/// primitive. Needs an opaque solid background: CSS carries a translucent alpha into the shadow but
/// gpui's is full strength, and with no background the silhouette is glyphs. Children do not
/// disqualify it. It appends, so `box-shadow` also draws.
fn apply_drop_shadow(
    style: &mut gpui::StyleRefinement,
    extras: &sk_css::Extras,
    inherited_color: Option<gpui::Hsla>,
    at: impl Fn() -> String,
) {
    let Some(shadow) = extras.drop_shadow else {
        return;
    };
    if !sk_css::diagnostics::opaque_background(style) {
        advise_drop_shadow_shape(at);
        return;
    }
    let color = shadow
        .color
        .or_else(|| style.text.color.or(inherited_color))
        .unwrap_or_else(gpui::black);
    style
        .box_shadow
        .get_or_insert_with(Vec::new)
        .push(gpui::BoxShadow {
            color,
            offset: gpui::point(gpui::px(shadow.dx), gpui::px(shadow.dy)),
            blur_radius: gpui::px(shadow.blur),
            spread_radius: gpui::px(0.0),
            inset: false,
        });
}

/// CSS `mask` as the gradient-border idiom: the background paints only in the padding and border
/// ring (`gpui::Style::vn_mask_ring`, sk-css's `Extras::mask_ring`).
fn apply_mask_ring(
    style: &mut gpui::StyleRefinement,
    extras: &sk_css::Extras,
    painted_by_child: bool,
    has_children: bool,
    at: impl Fn() -> String,
) {
    if !extras.mask_ring() {
        return;
    }
    if painted_by_child {
        warn_compat_at(
            "mask-ring-url",
            &at,
            "a gradient-border `mask` over a `url()` background is not applied (v1). The \
             image stack paints from a child box the ring does not reach, so the whole box \
             shows",
        );
        return;
    }
    style.vn_mask_ring = Some(true);
    if has_children {
        warn_compat_at(
            "mask-ring-children",
            &at,
            "the children of an element with a gradient-border `mask` are not masked (v1). \
             Its background paints only in the padding and border ring, as in a browser, but \
             its content paints whole, where a browser cuts it to the ring too",
        );
    }
}

fn paints_children(t: &Tree, node: &crate::tree::Node) -> bool {
    fn paints(t: &Tree, id: u32) -> bool {
        let Some(n) = t.node(id) else { return false };
        match n.kind {
            crate::tree::Kind::Element => true,
            crate::tree::Kind::Text => !n.text.is_empty(),
            crate::tree::Kind::Anchor => n.children.iter().any(|&c| paints(t, c)),
        }
    }
    node.children.iter().any(|&c| paints(t, c))
}

fn advise_drop_shadow_shape(at: impl FnOnce() -> String) {
    warn_compat_at(
        "filter-drop-shadow-shape",
        at,
        sk_css::diagnostics::DROP_SHADOW,
    );
}

/// One `::before`/`::after` generated box: a plain `div` with no id, hitbox or listeners, not a
/// retained-tree node. An empty `content: ""` box carries no child because gpui's text shaper needs
/// a leaf.
fn pseudo_element(
    owner: u32,
    pb: &sk_css::PseudoBox,
    parent_case: Option<sk_css::TextTransform>,
    stacking: &Stacking,
    parent_display: Option<gpui::Display>,
) -> AnyElement {
    let mut d = div();
    *d.style() = (*pb.style).clone();
    if pb.extras.mask_ring() {
        d.style().vn_mask_ring = Some(true);
    }
    crate::mask::apply(d.style(), owner, &pb.extras, pb.content.is_empty());
    let case = pb.extras.text_transform.or(parent_case);
    let e = if pb.content.is_empty() {
        d.into_any_element()
    } else {
        d.child(transformed_text(&pb.content, case))
            .into_any_element()
    };
    let effect = effect_context(&pb.extras, &pb.style);
    let layer = layer_of(&pb.extras, pb.style.position, effect, parent_display);
    let entered = stacking.enter(layer, next_stack_ordinal());
    if entered.negative {
        note_negative_z();
    }
    stack(entered.own, &pb.extras, e)
}

/// The marker box an `<li>` gets from the `list-style-type` in force: inherited, so the nearest
/// ancestor that declared one (sk-css folds no inherited property). `None` when not a list item,
/// nothing declared, or `list-style: none`.
fn list_marker_for(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    extras: &sk_css::Extras,
) -> Option<AnyElement> {
    if t.name(node.tag) != "li" {
        return None;
    }
    let (marker, inside) = {
        let mut marker = extras.list_marker;
        let mut inside = extras.list_marker_inside;
        let mut cursor = node.parent;
        while marker.is_none() || inside.is_none() {
            let Some(ancestor) = t.node(cursor) else {
                break;
            };
            if let Some(cs) = &ancestor.computed {
                marker = marker.or(cs.extras.list_marker);
                inside = inside.or(cs.extras.list_marker_inside);
            }
            if ancestor.parent == cursor {
                break;
            }
            cursor = ancestor.parent;
            if cursor == 0 {
                break;
            }
        }
        (marker?, inside.unwrap_or(false))
    };
    let ordinal = t
        .node(node.parent)
        .map(|parent| {
            parent
                .children
                .iter()
                .filter(|&&c| {
                    t.node(c)
                        .is_some_and(|n| n.kind == crate::tree::Kind::Element)
                })
                .position(|&c| c == id)
                .unwrap_or(0)
                + 1
        })
        .unwrap_or(1);
    let text = marker.render(ordinal)?;

    let mut d = div();
    let style = d.style();
    style.flex_shrink = Some(0.0);
    style.size.width = Some(gpui::Length::Definite(gpui::rems(1.25).into()));
    style.text.text_align = Some(gpui::TextAlign::Right);
    style.padding.right = Some(gpui::rems(0.35).into());
    if !inside {
        // `outside`: the marker sits in the item's padding area, not taking content width.
        style.margin.left = Some(gpui::rems(-1.25).into());
    }
    Some(d.child(SharedString::from(text)).into_any_element())
}

/// A text leaf with `text-transform` applied to the string, since gpui's `TextStyle` has no case
/// field.
pub(crate) fn transformed_text(text: &str, case: Option<sk_css::TextTransform>) -> SharedString {
    match case.and_then(|c| c.apply(text)) {
        Some(changed) => SharedString::from(changed),
        None => SharedString::from(text.to_string()),
    }
}

/// CSS whitespace processing for a text node's string. gpui breaks at every newline, but under
/// `normal`/`nowrap` a run of white space collapses to one space and a source newline is no break;
/// `pre-line` keeps newlines, `pre*` passes through. Borrows when nothing changes.
pub(crate) fn processed_text(raw: &str, mode: sk_css::WhiteSpaceMode) -> std::borrow::Cow<'_, str> {
    const COLLAPSIBLE: [char; 5] = [' ', '\t', '\n', '\r', '\u{c}'];
    let collapses = mode.collapses_spaces();
    let keeps_newlines = mode.preserves_newlines();
    if !collapses {
        return std::borrow::Cow::Borrowed(raw);
    }
    let needs_work = {
        let mut prev_ws = false;
        raw.chars().any(|c| {
            let ws = COLLAPSIBLE.contains(&c);
            let doubled = ws && prev_ws;
            prev_ws = ws;
            doubled || (ws && c != ' ' && !(keeps_newlines && c == '\n'))
        })
    };
    if !needs_work {
        return std::borrow::Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut run: Option<bool> = None; // Some(saw_newline_in_this_run)
    for c in raw.chars() {
        if COLLAPSIBLE.contains(&c) {
            let saw_newline = run.unwrap_or(false) || c == '\n';
            run = Some(saw_newline);
            continue;
        }
        if let Some(saw_newline) = run.take() {
            out.push(if keeps_newlines && saw_newline {
                '\n'
            } else {
                ' '
            });
        }
        out.push(c);
    }
    // A trailing run stays a space (or break): edge trimming removes it when the node is last in
    // its block, and a run between inline siblings must survive as the word gap.
    if let Some(saw_newline) = run {
        out.push(if keeps_newlines && saw_newline {
            '\n'
        } else {
            ' '
        });
    }
    std::borrow::Cow::Owned(out)
}

/// Removes white space at the start and end of a line. A text node's ends are a line's ends only
/// when nothing inline sits beside them, so `a<span> </span>b` keeps its space. Allocates nothing
/// when there is nothing to trim.
pub(crate) fn trim_edges<'a>(
    t: &Tree,
    node: &crate::tree::Node,
    id: u32,
    text: &'a str,
    mode: Option<sk_css::WhiteSpaceMode>,
) -> &'a str {
    const COLLAPSIBLE: [char; 4] = [' ', '\t', '\n', '\r'];
    // `pre*` preserves the edges too: the trim is the second half of the collapsing rule.
    if !mode
        .unwrap_or(sk_css::WhiteSpaceMode::Normal)
        .collapses_spaces()
    {
        return text;
    }
    let Some(parent) = t.node(node.parent) else {
        return text;
    };
    if parent.kind == Kind::Element && INLINE_LEVEL_TAGS.contains(&t.name(parent.tag)) {
        return text;
    }
    let mut boxes = parent
        .children
        .iter()
        .copied()
        .filter(|&c| generates_a_box(t, c));
    let first = boxes.next();
    let last = boxes.last().or(first);
    let text = if first == Some(id) {
        text.trim_start_matches(COLLAPSIBLE)
    } else {
        text
    };
    if last == Some(id) {
        text.trim_end_matches(COLLAPSIBLE)
    } else {
        text
    }
}

pub(crate) fn trim_own_edges<'a>(
    t: &Tree,
    node: &crate::tree::Node,
    text: &'a str,
    mode: Option<sk_css::WhiteSpaceMode>,
) -> &'a str {
    const COLLAPSIBLE: [char; 4] = [' ', '\t', '\n', '\r'];
    if !mode
        .unwrap_or(sk_css::WhiteSpaceMode::Normal)
        .collapses_spaces()
    {
        return text;
    }
    if INLINE_LEVEL_TAGS.contains(&t.name(node.tag))
        && t.node(node.parent).is_some_and(|parent| {
            parent
                .children
                .iter()
                .filter(|&&c| generates_a_box(t, c))
                .count()
                > 1
        })
    {
        return text;
    }
    text.trim_matches(COLLAPSIBLE)
}

/// Does this node produce a box the parent's layout makes room for? Vue anchors every Fragment with
/// `createComment()` / `createText("")`; a browser generates no box for them, so lowering them
/// would add flex items and extra `gap`s. They stay real retained-tree nodes; only layout skips
/// them.
fn generates_a_box(t: &Tree, id: u32) -> bool {
    if popped_out(id) {
        return false;
    }
    match t.node(id) {
        None => false,
        Some(n) => match n.kind {
            Kind::Anchor => false,
            Kind::Text => !n.text.is_empty() && !blank_collapses_away(t, id, n),
            Kind::Element => true,
        },
    }
}

fn laid_out_children(t: &Tree, node: &crate::tree::Node) -> Vec<u32> {
    // Document order, negative `z-index` included: a negative-z box is a deferred draw sorted
    // before its context's in-flow content (`Stacking`), so this list, which is also taffy's, never
    // has to move.
    node.children
        .iter()
        .copied()
        .filter(|&c| generates_a_box(t, c))
        .collect()
}

/// `display: inline-*` mapped onto its block-level equivalent, advised only where it is visible: an
/// in-flow inline-level box with no width in a block container that did not become a row (CSS
/// blockifies the rest, [`apply_inline_row`] covers all-inline blocks).
fn advise_inline_level(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    extras: &sk_css::Extras,
    style: &gpui::StyleRefinement,
) {
    if !sk_css::diagnostics::needs_line_box(extras.inline_level, Some(style)) {
        return;
    }
    let Some(parent) = t.node(node.parent) else {
        return;
    };
    let parent_display = parent.computed.as_ref().and_then(|cs| cs.base.display);
    if matches!(
        parent_display,
        Some(gpui::Display::Flex) | Some(gpui::Display::Grid)
    ) {
        return;
    }
    if inline_row_applies(t, parent, parent_display) {
        return;
    }
    warn_compat_at(
        "display-inline-level-stretches",
        || node_prefix(t, id),
        sk_css::diagnostics::INLINE_LEVEL,
    );
}

/// `position: fixed`, which taffy cannot express, advised only where it comes out different: taffy
/// resolves it against the parent, CSS against the viewport; they agree when the parent is the
/// viewport (app root, `<Teleport to="body">`).
fn advise_out_of_flow(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    extras: &sk_css::Extras,
    win: WinKey,
) {
    // The behaviour is the browser's (a fixed element in a second window anchors to that window's
    // viewport), but it is a real change for the same element once popped out, so say so once.
    if extras.fixed == Some(true) && !windows::is_primary(win) {
        warn_compat(
            "position-fixed-in-popout",
            "A-M03: `position: fixed` inside a pop-out resolves against THAT window's viewport, not the main window's — the element anchors to the edges of the window that paints it. That is what a second browser window does too, and it is still a behaviour change for an element that was pinned to the document's viewport a moment ago.",
        );
    }
    if extras.fixed == Some(true) && settled(id, !fixed_lands_on_the_viewport(t, node, win)) {
        warn_compat_at(
            "position-fixed-not-portalled",
            || node_prefix(t, id),
            "`position: fixed` is laid out against a box that is NOT the viewport (v1)              — gpui/taffy have no viewport-relative position, so the element resolves              against its PARENT, and this element's parent is a different size from the              window. It is still painted in a deferred layer, so it stacks above its              siblings; portal it to the app root (Vue's `<Teleport to=\"body\">`, which              is what every overlay library already does) and the two agree exactly",
        );
    }
}

/// True once `holds` has been true for this node on [`SETTLED_FRAMES`] consecutive builds; `false`
/// resets the count. The first frames' layout is not final (`vh` is flattened at sheet-compile
/// time) and an advisory cannot be retracted, so it waits.
fn settled(id: u32, holds: bool) -> bool {
    const SETTLED_FRAMES: u8 = 3;
    SETTLING.with(|m| {
        let mut m = m.borrow_mut();
        if !holds {
            m.remove(&id);
            return false;
        }
        let n = m.entry(id).or_insert(0);
        *n = n.saturating_add(1);
        *n == SETTLED_FRAMES
    })
}

/// `(the node whose box is this window's viewport, the key [`painted_bounds_in`] addresses that
/// window by)`. They differ only for the primary window: its painted root is the sentinel `0` while
/// the viewport node is `t.root`.
fn window_viewport(t: &Tree, win: WinKey) -> (u32, u32) {
    match windows::root_of(win) {
        0 => (t.root, 0),
        popped => (popped, popped),
    }
}

fn fixed_lands_on_the_viewport(t: &Tree, node: &crate::tree::Node, win: WinKey) -> bool {
    // The viewport is the painting window's: `painted_bounds` searches every window and would
    // compare a pop-out's box against the main root, two coordinate spaces.
    let (viewport, painted_root) = window_viewport(t, win);
    if node.parent == viewport {
        return true;
    }
    let (Some(parent), Some(root)) = (
        painted_bounds_in(painted_root, node.parent),
        painted_bounds_in(painted_root, viewport),
    ) else {
        return true;
    };
    let near = |a: f32, b: f32| (a - b).abs() < 0.5;
    if !(near(parent.0, root.0)
        && near(parent.1, root.1)
        && near(parent.2, root.2)
        && near(parent.3, root.3))
    {
        return false;
    }
    // The containing block is the parent's padding box, so a padded or bordered wrapper of exactly
    // viewport size still insets it.
    t.node(node.parent)
        .and_then(|p| p.computed.as_ref())
        .is_none_or(|cs| no_inner_inset(&cs.base))
}

fn no_inner_inset(style: &gpui::StyleRefinement) -> bool {
    let zero_definite = |v: &Option<gpui::DefiniteLength>| match v {
        None => true,
        Some(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(p))) => {
            f32::from(*p) == 0.0
        }
        Some(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Rems(r))) => r.0 == 0.0,
        Some(gpui::DefiniteLength::Fraction(f)) => *f == 0.0,
        Some(gpui::DefiniteLength::Calc { .. }) => false,
    };
    let zero_absolute = |v: &Option<gpui::AbsoluteLength>| match v {
        None => true,
        Some(gpui::AbsoluteLength::Pixels(p)) => f32::from(*p) == 0.0,
        Some(gpui::AbsoluteLength::Rems(r)) => r.0 == 0.0,
    };
    [
        &style.padding.top,
        &style.padding.right,
        &style.padding.bottom,
        &style.padding.left,
    ]
    .into_iter()
    .all(zero_definite)
        && [
            &style.border_widths.top,
            &style.border_widths.right,
            &style.border_widths.bottom,
            &style.border_widths.left,
        ]
        .into_iter()
        .all(zero_absolute)
}

pub(crate) fn is_metadata(tag: &str) -> bool {
    matches!(
        tag,
        "style" | "script" | "template" | "head" | "meta" | "link" | "title" | "base"
    )
}

fn extras_of(node: &crate::tree::Node) -> sk_css::Extras {
    node.computed
        .as_ref()
        .map(|cs| cs.extras.clone())
        .unwrap_or_default()
}

const POINTER_BITS: u64 = (1 << tree::EV_CLICK)
    | (1 << tree::EV_MOUSEDOWN)
    | (1 << tree::EV_MOUSEUP)
    | (1 << tree::EV_CONTEXTMENU)
    | (1 << tree::EV_AUXCLICK)
    | (1 << tree::EV_MOUSEMOVE)
    | (1 << tree::EV_MOUSEENTER)
    | (1 << tree::EV_MOUSELEAVE)
    | (1 << tree::EV_MOUSEOVER)
    | (1 << tree::EV_MOUSEOUT)
    | (1 << tree::EV_WHEEL);

/// The listener mask of a build root (main window's or a pop-out's). A pop-out's root is deep in
/// the same tree and its DOM ancestors lie in no element gpui lays out, yet their (`document`'s)
/// listeners hear its events bubble, so it takes their bits. `pointer-events: none` never takes a
/// root's away: a modal layer sets it on `body`.
fn root_mask(t: &Tree, node: &crate::tree::Node) -> u64 {
    let mut mask = node.listeners | crate::haptics::mask(node);
    let mut cur = node.parent;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        mask |= n.listeners | crate::haptics::mask(n);
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    mask
}

fn pointer_mask(node: &crate::tree::Node) -> u64 {
    let mask = node.listeners | crate::haptics::mask(node);
    // Not on the root: dismissable layers set `body.style.pointerEvents = "none"` while a modal is
    // open, and the root is the keystroke path, so honouring it would kill Escape-to-dismiss.
    if node.parent == 0 {
        return mask;
    }
    let inert = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
    if inert {
        mask & !POINTER_BITS
    } else {
        mask
    }
}

fn apply_computed_style<E>(
    mut e: E,
    cs: &sk_css::ComputedStates,
    id: u32,
    now: std::time::Instant,
) -> E
where
    E: gpui::Styled + gpui::StatefulInteractiveElement + 'static,
{
    use gpui::Refineable as _;
    *e.style() = (*cs.base).clone();
    if let Some(capture) = crate::view_transition::capture_of(id) {
        e.style().vn_capture = Some(capture);
    }
    if let Some(url) = &cs.extras.filter_url {
        e.style().vn_color_matrix = tree::read_tree(|tree| crate::filter::resolve(tree, url, id));
    }
    if let Some(overlay) = tree::animation_overlay(id, &cs.base, now) {
        e.style().refine(&overlay);
    }
    // Properties named in `transition` come out of the hover rungs: a rung has no clock and would
    // refine them whole the instant of hover, leaving the animator nothing to interpolate.
    let split = |rung: &std::rc::Rc<gpui::StyleRefinement>| {
        sk_css::strip_transitioned(rung, &cs.transitions)
            .map(std::rc::Rc::new)
            .unwrap_or_else(|| rung.clone())
    };
    // `:hover` and `.group:hover .this` match the hover path (topmost element under the pointer and
    // its ancestors, resolved once a frame by `crate::pointer::resolve_hover`), in gpui's order:
    // the group's rung, then the element's own.
    if let Some((anchor, style)) = &cs.group_hover {
        if tree::is_hovered(*anchor as u32) {
            e.style().refine(&split(style));
        }
    }
    if let Some(h) = &cs.hover {
        if tree::is_hovered(id) {
            e.style().refine(&split(h));
        }
    }
    if let Some(group) = &cs.group_focus {
        tree::read_tree(|t| {
            group.fold(e.style(), |key, state| {
                crate::live_state::drawn_focus_holds(t, key as u32, state)
            })
        });
    }
    // `pointer-events: none` means not interactive at all, so no gpui active rungs, which would
    // make gpui insert a hitbox.
    if cs.extras.pointer_events_none != Some(true) {
        if let Some(a) = cs.active.clone() {
            e = e.active(move |_| (*a).clone());
        }
        if let Some((anchor, style)) = cs.group_active.clone() {
            e = e.group_active(group_name(anchor as u32), move |_| (*style).clone());
        }
    }
    if crate::fullscreen::is_active(id) {
        crate::fullscreen::apply_style(e.style());
    }
    // Sticky insets are thresholds applied at prepaint (`crate::sticky`), not a relative offset.
    if cs.extras.sticky == Some(true) {
        e.style().inset = Default::default();
    }
    e
}

/// Focus this node on a press and claim the press so no ancestor re-focuses over it. gpui gives
/// every element with a tracked focus handle a mouse-down listener that focuses it and calls
/// `prevent_default()` so focusable ancestors back off; the document root is one, so our listeners
/// must claim too or the root would take focus over an `<input>`. The focus waits until dispatch is
/// over ([`apply_press_focus`]): a browser focuses as the mousedown's default action, which
/// `preventDefault()` cancels ([`cancel_press_focus`]).
fn claim_press_focus(id: u32, handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    begin_press_focus(window, cx);
    window.prevent_default();
    windows::here(window, |w| {
        if !w.press_focus.claims.iter().any(|(c, _)| *c == id) {
            w.press_focus.claims.push((id, handle.clone()));
        }
    });
}

fn begin_press_focus(window: &mut Window, cx: &mut App) {
    let schedule = windows::here(window, |w| {
        if w.press_focus.pending {
            return false;
        }
        w.press_focus = windows::PressFocus {
            pending: true,
            ..Default::default()
        };
        true
    });
    if schedule {
        window.defer(cx, |window, cx| {
            crate::off_input(|| apply_press_focus(window, cx))
        });
    }
}

fn cancel_press_focus(window: &mut Window) {
    windows::here(window, |w| w.press_focus.cancelled = true);
    window.prevent_default();
}

/// A mouse-down's default action once its dispatch is over: the `mousedown`'s microtask checkpoint,
/// then the focus move and its `blur`/`focusout`/`focus`/`focusin`, all before the next platform
/// event (so before `mouseup`, `click`, `contextmenu`); gpui's observer would report it on the next
/// draw, after a quick right click's `contextmenu`. A claimed node still in the document takes
/// focus; with nothing focusable on the path the root takes it (`body`) and the old node blurs; a
/// cancelled press, or one a native widget took, leaves it.
fn apply_press_focus(window: &mut Window, cx: &mut App) {
    let press = windows::here(window, |w| std::mem::take(&mut w.press_focus));
    if sk_runtime::bridge::drain_owed() {
        js_wave_app(window, cx, || true);
    }
    let k = windows::key(window);
    let root = window_root(k);
    let root_handle = || FOCUS_HANDLES.with(|m| m.borrow().get(&root).cloned());
    if !press.cancelled && (!press.claims.is_empty() || press.reached) {
        let in_document =
            |id: u32| tree::read_tree(|t| t.node(id).is_some() && reaches_painted_root(t, id));
        let target = match press.claims.into_iter().find(|(id, _)| in_document(*id)) {
            Some(claim) => Some(claim),
            None => root_handle().map(|h| (root, h)),
        };
        if let Some((id, h)) = target {
            if !h.is_focused(window) {
                window.focus(&h, cx);
                if id != root {
                    crate::log::record("dev", format!("[focus] press focused node {id}"));
                }
            }
        }
    } else if let Some(now) = focused_node(window, cx) {
        // gpui's own focus-on-press may have moved focus to an inner box tracked only for
        // `:focus-within`, which HTML never focuses: put it back.
        if now != root && !windows::of(k, |w| w.focus_subs.contains_key(&now)) {
            let slot = windows::focused_node(k);
            let back = if slot != 0 {
                focus_target_of(slot, cx)
            } else {
                root_handle()
            };
            if let Some(h) = back {
                window.focus(&h, cx);
            }
        }
    }
    sync_focus_events(window, cx);
    crate::select::press_settled(press.cancelled, window, cx);
    crate::range::press_settled(press.cancelled, window, cx);
}

pub(crate) fn sync_focus_events(window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let now = match focused_node(window, cx) {
        None => 0,
        Some(id) if id == window_root(k) => 0,
        Some(id) if windows::of(k, |w| w.focus_subs.contains_key(&id)) => id,
        Some(_) => return,
    };
    if now == windows::focused_node(k) {
        return;
    }
    focus_transition_app(now, window, cx);
    if !repaint_other_windows(cx) {
        window.refresh();
    }
}

fn group_name(id: u32) -> gpui::SharedString {
    format!("sk-g{id}").into()
}

/// Where a box paints: the stacking context it is in, as the key gpui sorts deferred draws by. The
/// key is a path, one `(z, tree-order ordinal)` slot per enclosing context, outermost first; gpui
/// compares lexicographically, which is CSS 2.1 Appendix E's paint order, and the hit test walks
/// the same order. A negative z paints between its context's background and in-flow content, so the
/// context's children are deferred into an in-flow slot ([`IN_FLOW`]). Effect
/// contexts are read off the declared style, not the animated one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stacking {
    key: std::rc::Rc<[u128]>,
    deferred: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    InFlow,
    InFlowContext,
    Positioned,
    Context(i32),
}

/// Nested deferred boxes allowed before one paints in flow. gpui runs one prepaint round per
/// nesting level and asserts at 64 (fork item 37); the margin is for the in-flow splits, which
/// [`Stacking::enter`] cannot count because they are decided once the children are built.
const MAX_DEFERRED_NESTING: u32 = 32;

pub(crate) fn z_applies(extras: &sk_css::Extras, parent_display: Option<gpui::Display>) -> bool {
    extras.positioned == Some(true)
        || matches!(
            parent_display,
            Some(gpui::Display::Flex) | Some(gpui::Display::Grid)
        )
}

pub(crate) fn effect_context(extras: &sk_css::Extras, style: &gpui::StyleRefinement) -> bool {
    style.opacity.is_some_and(|o| o < 1.0)
        || !matches!(style.vn_clip_path, None | Some(gpui::VnClipPath::None))
        || style
            .vn_transform
            .as_ref()
            .is_some_and(|t| !t.is_identity())
        || style.backdrop_blur.is_some_and(|r| r > gpui::px(0.0))
        || extras.blur.is_some_and(|b| b > 0.0)
        || extras.filter_url.is_some()
        || extras.drop_shadow.is_some()
        || extras.isolate == Some(true)
        || reads_content(style)
        || extras.mask.images.as_ref().is_some_and(|l| !l.is_empty())
        || extras.mask.urls.as_ref().is_some_and(|u| !u.is_empty())
}

/// An effect that shades the element's own painting contains its positioned descendants, as a
/// filter does.
fn reads_content(style: &gpui::StyleRefinement) -> bool {
    [style.vn_effect, style.vn_effect_fade.map(|fade| fade.from)]
        .into_iter()
        .flatten()
        .any(|spec| spec.is_some() && spec.flags & gpui::EFFECT_FLAG_NEEDS_CONTENT != 0)
}

pub(crate) fn layer_of(
    extras: &sk_css::Extras,
    position: Option<gpui::Position>,
    effect: bool,
    parent_display: Option<gpui::Display>,
) -> Layer {
    let z = if z_applies(extras, parent_display) {
        extras.z()
    } else {
        None
    };
    let positioned = position == Some(gpui::Position::Absolute) || extras.positioned == Some(true);
    match z {
        Some(z) if z != 0 => Layer::Context(z),
        Some(_) if positioned => Layer::Context(0),
        Some(_) => Layer::InFlowContext,
        None if extras.fixed == Some(true) || extras.sticky == Some(true) => Layer::Context(0),
        None if positioned && effect => Layer::Context(0),
        None if positioned => Layer::Positioned,
        None if effect => Layer::InFlowContext,
        None => Layer::InFlow,
    }
}

fn slot(rank: u64, ordinal: u64) -> u128 {
    ((rank as u128) << 64) | ordinal as u128
}

/// The rank of a context at `z`: odd, so that [`IN_FLOW`] fits between -1 and 0.
fn z_rank(z: i32) -> u64 {
    ((i64::from(z) + (1 << 31)) as u64) * 2 + 1
}

const IN_FLOW: u64 = 1 << 32;

thread_local! {
    /// Tree-order ordinal of the next box to enter a [`Stacking`]; reset per build.
    static STACK_ORDINAL: Cell<u64> = const { Cell::new(0) };
    static OPEN_CONTEXTS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

fn next_stack_ordinal() -> u64 {
    STACK_ORDINAL.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    })
}

fn reset_stacking() {
    STACK_ORDINAL.with(|c| c.set(0));
    OPEN_CONTEXTS.with(|c| c.borrow_mut().clear());
}

pub(crate) struct Entered {
    pub(crate) own: Option<std::rc::Rc<[u128]>>,
    pub(crate) inside: Stacking,
    pub(crate) opens: bool,
    pub(crate) negative: bool,
}

impl Stacking {
    fn with_slot(&self, slot: u128) -> std::rc::Rc<[u128]> {
        self.key
            .iter()
            .copied()
            .chain(std::iter::once(slot))
            .collect()
    }

    fn in_flow_key(&self) -> std::rc::Rc<[u128]> {
        self.with_slot(slot(IN_FLOW, next_stack_ordinal()))
    }

    fn enter(&self, layer: Layer, ordinal: u64) -> Entered {
        let (z, defer, opens) = match layer {
            Layer::InFlow => {
                return Entered {
                    own: None,
                    inside: self.clone(),
                    opens: false,
                    negative: false,
                }
            }
            Layer::InFlowContext => (0, false, true),
            Layer::Positioned => (0, true, false),
            Layer::Context(z) => (z, true, true),
        };
        let key = self.with_slot(slot(z_rank(z), ordinal));
        let defer = defer && {
            let room = self.deferred < MAX_DEFERRED_NESTING;
            if !room {
                warn_compat(
                    "deferred-paint-nesting",
                    "more than 32 nested positioned layers — the deeper ones paint in tree order \
                     instead of above their in-flow siblings (each is one more of gpui's deferred \
                     prepaint rounds). A stacking context stays one",
                );
            }
            room
        };
        let deferred = self.deferred + u32::from(defer);
        let inside = Stacking {
            key: if opens { key.clone() } else { self.key.clone() },
            deferred,
        };
        Entered {
            own: defer.then_some(key),
            inside,
            opens,
            negative: defer && z < 0,
        }
    }
}

fn note_negative_z() {
    OPEN_CONTEXTS.with(|c| {
        if let Some(top) = c.borrow_mut().last_mut() {
            *top = true;
        }
    });
}

fn open_context() {
    OPEN_CONTEXTS.with(|c| c.borrow_mut().push(false));
}

fn close_context() -> bool {
    OPEN_CONTEXTS
        .with(|c| c.borrow_mut().pop())
        .unwrap_or(false)
}

fn split_in_flow(kids: Vec<AnyElement>, keep: usize, inside: &Stacking) -> Vec<AnyElement> {
    kids.into_iter()
        .enumerate()
        .map(|(i, kid)| {
            if i < keep {
                return kid;
            }
            gpui::deferred(kid)
                .with_stack_key(inside.in_flow_key())
                .clip_to_ancestors(true)
                .into_any_element()
        })
        .collect()
}

fn stack(key: Option<std::rc::Rc<[u128]>>, extras: &sk_css::Extras, el: AnyElement) -> AnyElement {
    match key {
        Some(key) => gpui::deferred(el)
            .with_stack_key(key)
            // `position: fixed` keeps the escape hatch: it is laid out against the parent, not the
            // viewport, so clipping it would make it vanish.
            // A `z-index`ed element escapes its siblings' PAINT ORDER, not its ancestors'
            // `overflow` — CSS clips it to the scroll container it is laid out inside.
            .clip_to_ancestors(extras.fixed != Some(true))
            .into_any_element(),
        None => el,
    }
}

fn stack_box(
    own: Option<std::rc::Rc<[u128]>>,
    split: &Cell<bool>,
    is_root: bool,
    inside: &Stacking,
    extras: &sk_css::Extras,
    el: AnyElement,
) -> AnyElement {
    let own = own.or_else(|| (split.get() && !is_root).then(|| inside.key.clone()));
    stack(own, extras, el)
}

/// Claims the key event gpui is dispatching for one DOM event: `true` for the first caller. gpui
/// hands a key event to every key listener on the focus path; each dispatching its own made a
/// `document` listener hear one Escape twice. Keyed by gpui's per-event number because interceptors
/// run before capture, a cancelled key stops the walk early, and devtools sends several keys in one
/// update.
pub(crate) fn claim_key_event(window: &Window) -> bool {
    let seq = window.vn_key_event_seq();
    windows::here(window, |w| {
        std::mem::replace(&mut w.key_event_claimed, seq) != seq
    })
}

#[allow(clippy::too_many_arguments)]
fn bind_events<E, V>(
    mut e: E,
    id: u32,
    mask: u64,
    click_focus: Option<FocusHandle>,
    focusable: bool,
    keys_here: bool,
    cx: &mut Context<V>,
) -> E
where
    E: gpui::StatefulInteractiveElement + 'static,
    V: 'static,
{
    let bit = move |ev: u32| mask & (1u64 << ev) != 0;
    // gpui runs listeners innermost first and the JS side walks the ancestors itself, so the first
    // to run claims the event and the rest stay quiet; otherwise an outer `@click` ran twice. A
    // mouse-up's `mouseup`, `click` and `contextmenu` are claimed into one wave delivered after
    // gpui's dispatch (`WindowState::up_waves`): gpui calls a box's click listener before its
    // mouse-up one, so dispatching in place put `click` before `mouseup`. One event per press,
    // dispatched at the browser's target: the topmost element under the pointer (`crate::pointer`),
    // which need not listen at all.
    if bit(tree::EV_CLICK) {
        e = e.on_click(cx.listener(move |_, ev: &ClickEvent, window, cx| match ev {
            ClickEvent::Mouse(_) => {}
            // Space on the focused box reaches that box only. gpui clicks on the key's release
            // (Chromium's rule for Space) but before the `keyup` handlers, so the click waits for
            // them (`keyboard_click_later`).
            ClickEvent::Keyboard(k) => {
                if k.button == gpui::KeyboardButton::Space {
                    let (at, modifiers) = (ev.position(), ev.modifiers());
                    let step = KeyClick::Click {
                        id,
                        at,
                        modifiers,
                        armed: true,
                        key: " ",
                    };
                    owe_key_click(step, window, &mut **cx);
                }
            }
            ClickEvent::Touch(_) => {
                let payload =
                    mouse_payload_n(ev.position(), 0.0, &ev.modifiers(), ev.click_count());
                js_dispatch(id, tree::EV_CLICK, &payload, window, cx);
            }
        }));
        e = e.on_mouse_up(MouseButton::Left, move |ev, window, cx| {
            claim_mouse_click(ev, id, window, cx, |w| &mut w.click);
        });
        // Enter clicks the focused box on its press (Chromium dispatches from the keypress), after
        // every `keydown` handler and only if none cancelled it; gpui would click on the release,
        // after `keyup`.
        if let Some(focus) = click_focus.clone() {
            e = e.on_key_down(cx.listener(move |_, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                if k.modifiers.modified() || !focus.is_focused(window) {
                    return;
                }
                match k.key.as_str() {
                    "enter" if keyboard_activates(id, false) => {
                        let at =
                            windows::peek(windows::key(window), |w| w.bounds.get(&id).copied())
                                .flatten()
                                .map(|(x, y, _, h)| gpui::point(gpui::px(x), gpui::px(y + h)))
                                .unwrap_or_default();
                        let step = KeyClick::Click {
                            id,
                            at,
                            modifiers: k.modifiers,
                            armed: false,
                            key: "Enter",
                        };
                        owe_key_click(step, window, &mut **cx);
                    }
                    "space" if keyboard_activates(id, true) => {
                        windows::here(window, |w| w.space_armed = None);
                        owe_key_click(KeyClick::Arm(id), window, &mut **cx);
                    }
                    _ => {}
                }
            }));
        }
    }
    if bit(tree::EV_AUXCLICK) {
        for button in [MouseButton::Middle, MouseButton::Right] {
            e = e.on_mouse_up(button, move |ev, window, cx| {
                claim_mouse_click(ev, id, window, cx, |w| &mut w.aux);
            });
        }
    }
    if bit(tree::EV_MOUSEDOWN) {
        e = e.capture_any_mouse_down(|_, window, _| {
            windows::here(window, |w| w.down_claimed = false)
        });
    }
    if bit(tree::EV_MOUSEDOWN) || bit(tree::EV_CLICK) || bit(tree::EV_AUXCLICK) || focusable {
        let handle = click_focus;
        e = e.on_any_mouse_down(cx.listener(move |_, ev: &MouseDownEvent, window, cx| {
            if windows::here(window, |w| w.press.is_none()) {
                let target = crate::pointer::target_or(window, id);
                let clickable = !cx.has_active_drag()
                    && crate::pointer::click_target(window, target, id).is_some();
                windows::here(window, |w| {
                    w.press = Some(Press {
                        target,
                        button: button_code(ev.button),
                        position: ev.position,
                        clickable,
                        cancelled: false,
                    });
                });
                watch_activation(window, cx);
            }
            if let Some(h) = &handle {
                claim_press_focus(id, h, window, &mut **cx);
            }
            if bit(tree::EV_MOUSEDOWN)
                && !windows::here(window, |w| std::mem::replace(&mut w.down_claimed, true))
            {
                let button = button_code(ev.button);
                let target = crate::pointer::target_or(window, id);
                let payload = mouse_payload_n(ev.position, button, &ev.modifiers, ev.click_count);
                if !js_dispatch(target, tree::EV_MOUSEDOWN, &payload, window, cx) {
                    cancel_press_focus(window);
                    crate::selection::note_press_prevented();
                }
            }
        }));
    }
    if bit(tree::EV_MOUSEUP) {
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            e = e.on_mouse_up(
                button,
                cx.listener(move |_, ev: &MouseUpEvent, window, cx| {
                    let payload = mouse_payload_n(
                        ev.position,
                        button_code(ev.button),
                        &ev.modifiers,
                        ev.click_count,
                    );
                    let target = crate::pointer::target_or(window, id);
                    claim_up(window, &mut **cx, |w| &mut w.mouseup, target, &payload);
                }),
            );
        }
    }
    if bit(tree::EV_CONTEXTMENU) {
        // Chromium on Windows opens the menu on the right button's RELEASE
        // (`ShowContextMenuOnMouseUp`), after `mouseup`.
        e = e.on_mouse_up(
            MouseButton::Right,
            cx.listener(move |_, ev: &MouseUpEvent, window, cx| {
                let payload = mouse_payload(ev.position, 2.0, &ev.modifiers);
                let target = crate::pointer::target_or(window, id);
                claim_up(window, &mut **cx, |w| &mut w.menu, target, &payload);
            }),
        );
    }
    if bit(tree::EV_MOUSEMOVE) {
        // No dispatch here: stash and let the pump deliver one mousemove per frame (see
        // PendingMouseMove). An outer listener seeing the same position is the same physical move,
        // which the innermost already claimed.
        e = e.on_mouse_move(
            cx.listener(move |_, ev: &gpui::MouseMoveEvent, window, _cx| {
                windows::here(window, |w| {
                    if let Some(p) = w.pending_mouse_move.as_ref() {
                        if p.position == ev.position {
                            return;
                        }
                    }
                    w.pending_mouse_move = Some(PendingMouseMove {
                        id,
                        position: ev.position,
                        button: button_code_opt(ev.pressed_button),
                        modifiers: ev.modifiers,
                    });
                });
            }),
        );
    }
    if bit(tree::EV_WHEEL) {
        e = e.on_scroll(cx.listener(move |_, ev: &ScrollEvent, window, cx| {
            if !claim_wheel(&mut **cx) {
                return;
            }
            let (dx, dy) = dom_wheel_delta(ev.delta.pixel_delta(gpui::px(16.0)));
            let payload = EventPayload {
                a: dx,
                b: dy,
                c: 0.0,
                d: mods_bits(&ev.modifiers),
                key: None,
            };
            let target = crate::pointer::target_or(window, id);
            crate::pointer_at_next_dispatch(f64::from(ev.position.x), f64::from(ev.position.y));
            if !js_dispatch(target, tree::EV_WHEEL, &payload, window, cx) {
                // Cancelled: a browser scrolls nothing. gpui's own listener already moved this box
                // and any scroll container inside it: put them back and keep the outer ones from
                // moving.
                sk_elements::undo_wheel_scroll();
                cx.stop_propagation();
            }
        }));
    }
    // One `keydown`/`keyup` per key event, at the focused element, bubbling to `document` and
    // `window`: the first listening box on the focus path gpui reaches dispatches it
    // ([`claim_key_event`]) and every other one lets it pass, the root included when the first
    // dispatch removed the focused element.
    if bit(tree::EV_KEYDOWN) && keys_here {
        e = e.on_key_down(cx.listener(move |_, ev: &KeyDownEvent, window, cx| {
            if !claim_key_event(window) {
                return;
            }
            let target = focused_node(window, cx).unwrap_or(id);
            let key = crate::keyboard::Keyboard::new(&ev.keystroke, ev.native.as_ref(), ev.is_held);
            if !key.dispatch(target, tree::EV_KEYDOWN, window, cx) {
                // A cancelled Ctrl+C has no `copy` event: the root's clipboard listener, reached
                // last, reads this.
                windows::here(window, |w| w.key_prevented = true);
            }
        }));
    }
    if bit(tree::EV_KEYUP) && keys_here {
        e = e.on_key_up(cx.listener(move |_, ev: &KeyUpEvent, window, cx| {
            if !claim_key_event(window) {
                return;
            }
            let target = focused_node(window, cx).unwrap_or(id);
            let key = crate::keyboard::Keyboard::new(&ev.keystroke, ev.native.as_ref(), false);
            if !key.dispatch(target, tree::EV_KEYUP, window, cx) {
                // A cancelled Menu-key keyup has no `contextmenu` (`bind_menu_keys`, the last
                // listener reached).
                windows::here(window, |w| w.key_prevented = true);
            }
        }));
    }
    e
}

fn focus_target_of(id: u32, _cx: &App) -> Option<FocusHandle> {
    // Not the editor itself: this runs from `el.focus()` in a keydown handler, inside that editor's
    // update (`INPUT_FOCUS`).
    if let Some(handle) = INPUT_FOCUS.with(|m| m.borrow().get(&id).cloned()) {
        return Some(handle);
    }
    FOCUS_HANDLES.with(|m| m.borrow().get(&id).cloned())
}

/// The devtools' focus, host-side. A browser's `el.focus()` is synchronous but gpui's observers
/// fire on the next draw, so for a node that can take focus now the host moves gpui's focus and the
/// window's `focused_node` slot together and JS dispatches the events; the observer then finds the
/// slot updated. A node not built yet is queued (one slot, last call wins) and its events come a
/// frame late.
pub(crate) fn focus_node(id: u32, window: &mut Window, cx: &mut App) -> Result<(), String> {
    let handle = focus_target_of(id, cx).filter(|_| tree::read_tree(|t| could_take_focus(t, id)));
    let Some(handle) = handle else {
        return Err(format!(
            "node {id} is not focusable — it has no tabindex and is not a form control, a link \
             with href, a summary or contenteditable (a browser would refuse `el.focus()` on it \
             too; a key or focus listener does not make an element focusable)"
        ));
    };
    window.focus(&handle, cx);
    Ok(())
}

thread_local! {
    static PENDING_FOCUS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

const PENDING_FOCUS_FRAMES: u32 = 10;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FocusAnswer {
    /// Focus moved now: JS fires `blur`/`focusout` on `old` (0 = none), then `focus`/`focusin` on
    /// the node.
    Moved {
        old: u32,
    },
    Already,
    Pending,
    /// Not focusable, not in the document, or hidden by CSS: a no-op, as in a browser.
    Refused,
}

fn could_take_focus(t: &Tree, id: u32) -> bool {
    let Some(n) = t.node(id) else { return false };
    let tag = t.name(n.tag);
    if tag != "input" && tag != "textarea" && actually_disabled(t, id) {
        return false;
    }
    // A `tabindex` that is not an integer is ignored, as the build ignores it (`tab_index_of`).
    let tab_index = t
        .prop(id, "tabindex")
        .is_some_and(|v| v.trim().parse::<isize>().is_ok());
    tag == "input" || tag == "textarea" || tab_index || default_tab_index(t, id).is_some()
}

pub(crate) fn css_visible(t: &Tree, id: u32) -> bool {
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        if let Some(hidden) = n
            .computed
            .as_ref()
            .and_then(|cs| cs.extras.visibility_hidden)
        {
            return !hidden;
        }
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    true
}

fn in_display_none(t: &Tree, id: u32) -> bool {
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { return false };
        if n.computed
            .as_ref()
            .is_some_and(|cs| cs.base.display == Some(gpui::Display::None))
        {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        cur = n.parent;
    }
    false
}

/// `el.focus()` is a no-op on an element that is not rendered or whose `visibility` is not
/// `visible` (Chromium's `IsFocusableStyle`): a popover that mounts hidden and focuses an item
/// before its "now visible" write is patched must not take focus.
pub(crate) fn style_refuses_focus(id: u32) -> bool {
    tree::flush_pending_style();
    tree::read_tree(|t| t.node(id).is_some() && (in_display_none(t, id) || !css_visible(t, id)))
}

/// Does `el.focus()` need a build first? `__sk_focus` then forces a layout where legal so `focus()`
/// is synchronous: reka's `focusFirst` checks `document.activeElement` right after each call.
pub(crate) fn focus_wants_build(id: u32) -> bool {
    let built = INPUT_STATES.with(|m| m.borrow().contains_key(&id))
        || FOCUS_HANDLES.with(|m| m.borrow().contains_key(&id));
    !built && tree::read_tree(|t| t.node(id).is_some() && could_take_focus(t, id))
}

pub(crate) fn queue_focus(id: u32) {
    PENDING_FOCUS.with(|p| p.set((id, PENDING_FOCUS_FRAMES)));
}

pub(crate) fn js_focus(id: u32, window: &mut Window, cx: &mut App) -> FocusAnswer {
    let k = windows::key(window);
    let (in_document, could) = tree::read_tree(|t| {
        (
            t.node(id).is_some() && reaches_painted_root(t, id),
            could_take_focus(t, id),
        )
    });
    // Not a focusable area: a node with only a `:focus` rule has a handle, but a browser refuses to
    // focus it.
    if !in_document || !could {
        return FocusAnswer::Refused;
    }
    if windows::window_of_node(id) != k {
        queue_focus(id);
        return FocusAnswer::Pending;
    }
    let Some(handle) = focus_target_of(id, cx) else {
        queue_focus(id);
        return FocusAnswer::Pending;
    };
    PENDING_FOCUS.with(|p| p.set((0, 0)));
    let old = windows::focused_node(k);
    if handle.is_focused(window) && old == id {
        return FocusAnswer::Already;
    }
    window.focus(&handle, cx);
    set_focus_slot(k, id);
    crate::log::record("dev", format!("[focus] el.focus() focused node {id}"));
    FocusAnswer::Moved {
        old: if old == id { 0 } else { old },
    }
}

pub(crate) fn js_blur(id: u32, window: &mut Window, cx: &mut App) -> bool {
    PENDING_FOCUS.with(|p| {
        if p.get().0 == id {
            p.set((0, 0));
        }
    });
    let k = windows::key(window);
    if windows::focused_node(k) != id {
        return false;
    }
    window.blur(cx);
    set_focus_slot(k, 0);
    true
}

/// Applies a queued `el.focus()` once the node has a handle. Runs from the frame ticks, before
/// `ensure_root_focus`.
pub(crate) fn apply_pending_focus(window: &mut Window, cx: &mut App) {
    let (id, left) = PENDING_FOCUS.with(|p| p.get());
    if id == 0 || windows::window_of_node(id) != windows::key(window) {
        return;
    }
    if !tree::read_tree(|t| t.node(id).is_some() && reaches_painted_root(t, id)) {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        return;
    }
    // Visible when `focus()` queued it, hidden since: a browser would have let focus go when it
    // hid.
    if style_refuses_focus(id) {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        crate::log::record(
            "dev",
            format!("[focus] queued el.focus() on node {id} dropped: CSS hid it"),
        );
        return;
    }
    if let Some(handle) = focus_target_of(id, cx) {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        if !handle.is_focused(window) {
            window.focus(&handle, cx);
            crate::log::record(
                "dev",
                format!("[focus] queued el.focus() focused node {id}"),
            );
        }
        return;
    }
    if left <= 1 {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        crate::log::record(
            "dev",
            format!("[focus] queued el.focus() on node {id} dropped: it never got a focus handle"),
        );
    } else {
        PENDING_FOCUS.with(|p| p.set((id, left - 1)));
    }
}

pub(crate) fn is_node_focused(id: u32, window: &Window, cx: &App) -> bool {
    focus_target_of(id, cx).is_some_and(|h| h.is_focused(window))
}

/// Which node owns keyboard focus, by asking gpui. See [`crate::devtools::focused_node`] for why
/// this is not simply `FOCUSED_NODE`.
pub(crate) fn focused_node(window: &Window, _cx: &App) -> Option<u32> {
    // The editor's handle first: an `<input>`'s focus identity lives inside the native widget, and
    // its wrapper may ALSO have a handle (a `:focus` rule).
    let from_inputs = INPUT_FOCUS.with(|m| {
        m.borrow()
            .iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(id, _)| *id)
    });
    from_inputs.or_else(|| {
        FOCUS_HANDLES.with(|m| {
            m.borrow()
                .iter()
                .find(|(_, h)| h.is_focused(window))
                .map(|(id, _)| *id)
        })
    })
}

pub(crate) fn ime(
    id: u32,
    text: &str,
    commit: bool,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), String> {
    use gpui::EntityInputHandler;
    let Some(state) = INPUT_STATES.with(|m| m.borrow().get(&id).cloned()) else {
        return Err(format!("node {id} is not a built <input> or <textarea>"));
    };
    state.update(cx, |s, cx| {
        if commit {
            s.replace_text_in_range(None, text, window, cx);
        } else {
            s.replace_and_mark_text_in_range(None, text, None, window, cx);
        }
    });
    Ok(())
}

/// Selects a text field's native editing buffer. Defers when the field has no entity yet (mounted
/// this wave) or its `TextInputState` is already borrowed (key/input callbacks).
pub(crate) fn select_input_text(id: u32) -> bool {
    let info = tree::read_tree(|t| {
        let node = t.node(id)?;
        let multiline = t.name(node.tag) == "textarea";
        if !multiline
            && (t.name(node.tag) != "input"
                || matches!(
                    t.prop(id, "type")
                        .unwrap_or("text")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "file" | "range" | "color"
                ))
        {
            return None;
        }
        let value = t
            .prop_write(id, "value")
            .map(|(v, write)| (v.to_string(), write))
            .or_else(|| multiline.then(|| (node.text.clone(), 0)));
        Some((multiline, value))
    });
    let Some((multiline, value)) = info else {
        return false;
    };
    if !crate::in_input_dispatch() {
        let state = INPUT_STATES.with(|m| m.borrow().get(&id).cloned());
        if let Some(state) = state {
            let applied = sk_runtime::with_app(|cx| {
                state.update(cx, |s, cx| {
                    s.set_multiline(multiline);
                    if let Some((value, write)) = &value {
                        s.sync_value(value, *write, cx);
                    }
                    s.select_all_text(cx);
                })
            })
            .is_some();
            if applied {
                INPUT_SELECT_ALL.with(|m| m.borrow_mut().remove(&id));
                return true;
            }
        }
    }
    INPUT_SELECT_ALL.with(|m| m.borrow_mut().insert(id));
    tree::with_tree(|t| t.touch(id));
    true
}

/// The live text of a native `<input>`: the widget's buffer, not the `value` prop, so it reflects
/// typing JS has not echoed yet.
pub(crate) fn input_value(id: u32, cx: &App) -> Option<String> {
    INPUT_STATES
        .with(|m| m.borrow().get(&id).map(|s| s.read(cx).text().to_string()))
        .or_else(|| {
            tree::read_tree(|t| crate::range::is_range(t, id).then(|| crate::range::value(t, id)))
        })
}

pub(crate) fn scroll_node(
    id: u32,
    dx: f32,
    dy: f32,
    window: &mut Window,
    cx: &mut App,
) -> Result<(f32, f32), String> {
    if let Some(info) = crate::scroll::by(id, dx, dy, crate::scroll::Behavior::Instant) {
        return Ok((info.left, info.top));
    }
    if let Some(state) = INPUT_STATES.with(|m| m.borrow().get(&id).cloned()) {
        let top = state.update(cx, |s, cx| {
            s.scroll_by(gpui::px(dy), window, cx);
            f32::from(s.scroll_top())
        });
        return Ok((0.0, top));
    }
    Err(format!(
        "node {id} is not a scroll container — nothing in the cascade gave it \
         `overflow: scroll` or `overflow: auto`, so there is no scroll offset to move"
    ))
}

/// Synthesizes one input event on `id` through the path a real gpui listener takes: publish
/// `Phase::Commit`, hand the payload to the JS dispatcher (which runs capture/bubble over the
/// mirror tree), then flush the tree so Vue's wave repaints this frame.
pub(crate) fn synth_event(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    crate::off_input(|| js_dispatch_app(id, event, payload, window, cx))
}

fn alt_element(alt: &str) -> AnyElement {
    if alt.is_empty() {
        Empty.into_any_element()
    } else {
        div()
            .child(SharedString::from(alt.to_string()))
            .into_any_element()
    }
}

/// The `object-position` an inner picture element paints with: the replaced element's own, from
/// its outer box (animation overlay included).
pub(crate) fn object_position_of(outer: &gpui::StyleRefinement) -> gpui::ObjectPosition {
    outer.vn_object_position.unwrap_or_default()
}

fn gpui_object_fit(fit: sk_css::ObjectFit) -> gpui::ObjectFit {
    match fit {
        sk_css::ObjectFit::Fill => gpui::ObjectFit::Fill,
        sk_css::ObjectFit::Contain => gpui::ObjectFit::Contain,
        sk_css::ObjectFit::Cover => gpui::ObjectFit::Cover,
        sk_css::ObjectFit::ScaleDown => gpui::ObjectFit::ScaleDown,
        sk_css::ObjectFit::None => gpui::ObjectFit::None,
    }
}

/// The HTML `width`/`height` attributes are presentational hints: they only fill in a dimension no
/// CSS rule declared.
fn presentational_size(t: &Tree, id: u32, style: &mut gpui::StyleRefinement) {
    fn hint(t: &Tree, id: u32, attr: &str, slot: &mut Option<gpui::Length>) {
        let Some(raw) = t.prop(id, attr) else { return };
        if slot.is_some() {
            return; // CSS wins over a presentational hint.
        }
        match raw.trim().parse::<f32>() {
            Ok(v) if v >= 0.0 => *slot = Some(gpui::Length::Definite(gpui::px(v).into())),
            _ => warn_compat(
                &format!("img-{attr}-attr"),
                &format!(
                    "<img {attr}=\"{raw}\"> is not a non-negative integer; HTML only allows \
                     unitless pixels there — the attribute was ignored (use CSS {attr} instead)"
                ),
            ),
        }
    }
    hint(t, id, "width", &mut style.size.width);
    hint(t, id, "height", &mut style.size.height);
}

fn build_img<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let src = t.prop(id, "src").unwrap_or("").to_string();
    let alt = t.prop(id, "alt").unwrap_or("").to_string();

    let resolved = img_src::source_for(id, &src);
    img_src::evict_pending(window, cx);
    let Some(source) = resolved else {
        let mut d = with_control_area(div().id(id as usize), id, control_area, win);
        d = crate::aria::apply(d, t, id, node);
        if let Some(cs) = &node.computed {
            d = apply_computed_style(d, cs, id, now);
        }
        presentational_size(t, id, d.style());
        return d.child(alt_element(&alt)).into_any_element();
    };

    let mut e = with_control_area(gpui::img(source).id(id as usize), id, control_area, win);
    e = crate::aria::apply(e, t, id, node);
    if let Some(cs) = &node.computed {
        e = apply_computed_style(e, cs, id, now);
    }
    presentational_size(t, id, e.style());

    let fit = node
        .computed
        .as_ref()
        .and_then(|cs| cs.extras.object_fit)
        .map(gpui_object_fit)
        // CSS initial value: gpui's ImageStyle defaults to Contain, which would letterbox every
        // sized `<img>` the web stretches.
        .unwrap_or(gpui::ObjectFit::Fill);
    e = e.object_fit(fit);
    e.style()
        .vn_object_position
        .get_or_insert_with(gpui::ObjectPosition::default);

    let failed_src: String = if src.chars().count() > 64 {
        format!(
            "{}…({} chars)",
            src.chars().take(48).collect::<String>(),
            src.chars().count()
        )
    } else {
        src.clone()
    };
    let failed_alt = alt.clone();
    e = e.with_fallback(move || {
        warn_compat(
            &format!("img-load:{failed_src}"),
            &format!(
                "<img src=\"{failed_src}\"> could not be loaded or decoded — painting the alt \
                 text instead (the element's `error` event fired, unless its header read as an \
                 image and only the picture itself is corrupt: then `load` fired, as in a \
                 browser, which also paints what it can)"
            ),
        );
        alt_element(&failed_alt)
    });

    let mask = pointer_mask(node);
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some();
    let styled_focus = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        e = e.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
        if let Some(cs) = &node.computed {
            if let Some(f) = cs.focus.clone() {
                e = e.focus(move |_| (*f).clone());
            }
            if let Some(f) = cs.focus_visible.clone() {
                e = e.focus_visible(move |_| (*f).clone());
            }
        }
    }
    let click_focus = handle.filter(|_| focusable);
    bind_events(e, id, mask, click_focus, focusable, true, cx).into_any_element()
}

/// A `gpui::Svg` that paints in the inherited text colour: gpui tints an SVG mask with
/// `style.text.color` (default `None`, paints nothing); reading `window.text_style()` at paint time
/// gives the live `currentColor` with no restyle or re-rasterization.
struct SvgTint<E> {
    inner: E,
    inherit: bool,
}

impl<E: gpui::Element + gpui::Styled> gpui::Element for SvgTint<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<gpui::ElementId> {
        self.inner.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.inner
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.inherit {
            // Fills only the base rung, so a `:hover` colour on the svg itself still overrides it
            // in gpui's ladder.
            self.inner.style().text.color = Some(window.text_style().color);
        }
        self.inner.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        )
    }
}

fn build_svg<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    // An `<svg>`'s silhouette is its shapes: resvg draws a `drop-shadow()` around them
    // (`crate::svg`).
    let shadow = node
        .computed
        .as_ref()
        .and_then(|cs| cs.extras.drop_shadow)
        .map(|d| crate::svg::ShadowSpec {
            dx: d.dx,
            dy: d.dy,
            blur: d.blur,
            color: d.color,
        });
    if shadow.is_some() && paints_own_box(node) {
        warn_compat_at(
            "filter-drop-shadow-svg-box",
            || node_prefix(t, id),
            "`filter: drop-shadow()` on an inline <svg> follows its shapes, as in a browser, but \
             the svg's own CSS background and border cast no shadow (v1): they are painted by \
             the box, outside the rasterised document the shadow is drawn in",
        );
    }
    let doc = crate::svg::document(t, id);
    if doc.css_paint_server {
        warn_compat("svg-css-paint-server", sk_css::diagnostics::PAINT_SERVER);
    }

    // No `with_transformation` here: `transform` is a paint-level style property, and setting it
    // here too would apply it twice.
    match (doc.paint, shadow) {
        (crate::svg::Paint::Mono, None) => {
            crate::svg::forget_raster(id);
            let e = with_control_area(
                gpui::svg().data(&doc.bytes).id(id as usize),
                id,
                control_area,
                win,
            );
            finish_svg(e, t, id, node, window, cx, now)
        }
        _ => {
            let e = with_control_area(
                crate::svg::raster(id, doc.clone(), shadow).id(id as usize),
                id,
                control_area,
                win,
            );
            finish_svg(e, t, id, node, window, cx, now)
        }
    }
}

fn paints_own_box(node: &crate::tree::Node) -> bool {
    let Some(cs) = &node.computed else {
        return false;
    };
    let b = &cs.base.border_widths;
    let wide = |w: Option<gpui::AbsoluteLength>| {
        w.is_some_and(|w| w.to_pixels(gpui::px(16.0)) > gpui::px(0.0))
    };
    let background = cs.base.background.as_ref().is_some_and(|f| {
        f.color()
            .and_then(|c| c.as_solid())
            .map_or(true, |c| c.a > 0.0)
    });
    background || wide(b.top) || wide(b.right) || wide(b.bottom) || wide(b.left)
}

fn finish_svg<E, V: 'static>(
    mut e: E,
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
) -> AnyElement
where
    E: gpui::StatefulInteractiveElement + gpui::Styled + gpui::Element + 'static,
{
    e = crate::aria::apply(e, t, id, node);
    let mut inherit = true;
    if let Some(cs) = &node.computed {
        inherit = cs.base.text.color.is_none();
        e = apply_computed_style(e, cs, id, now);
    }
    presentational_size(t, id, e.style());

    let mask = pointer_mask(node);
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some();
    let styled_focus = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        e = e.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
        if let Some(cs) = &node.computed {
            if let Some(f) = cs.focus.clone() {
                e = e.focus(move |_| (*f).clone());
            }
            if let Some(f) = cs.focus_visible.clone() {
                e = e.focus_visible(move |_| (*f).clone());
            }
        }
    }
    let click_focus = handle.filter(|_| focusable);
    let e = bind_events(e, id, mask, click_focus, focusable, true, cx);
    SvgTint { inner: e, inherit }.into_any_element()
}

#[cfg(feature = "lottie")]
fn lottie_fit(fit: sk_css::ObjectFit) -> sk_lottie::LottieFit {
    match fit {
        sk_css::ObjectFit::Fill => sk_lottie::LottieFit::Fill,
        sk_css::ObjectFit::Contain => sk_lottie::LottieFit::Contain,
        sk_css::ObjectFit::Cover => sk_lottie::LottieFit::Cover,
        sk_css::ObjectFit::ScaleDown => sk_lottie::LottieFit::ScaleDown,
        sk_css::ObjectFit::None => sk_lottie::LottieFit::None,
    }
}

#[cfg(any(feature = "lottie", feature = "video"))]
fn definite_px(len: Option<gpui::Length>, rem: gpui::Pixels) -> Option<f32> {
    match len? {
        gpui::Length::Definite(gpui::DefiniteLength::Absolute(a)) => {
            Some(f32::from(a.to_pixels(rem)))
        }
        _ => None,
    }
}

#[cfg(any(feature = "lottie", feature = "video"))]
fn is_sized(len: Option<gpui::Length>) -> bool {
    matches!(len, Some(gpui::Length::Definite(_)))
}

/// A replaced box's ratio: the author's, unless `auto` (zero) or `auto <ratio>` (negative) let the
/// natural one win.
#[cfg(any(feature = "lottie", feature = "video"))]
fn preferred_ratio(declared: Option<f32>, iw: f32, ih: f32) -> f32 {
    declared
        .filter(|r| *r > 0.0 && r.is_finite())
        .unwrap_or(iw / ih)
}

#[cfg(feature = "lottie")]
fn build_lottie<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let intrinsic = crate::lottie::sync(t, id, win, &mut **cx);

    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    presentational_size(t, id, d.style());

    if let Some((iw, ih)) = intrinsic {
        let (iw, ih) = (iw.max(1) as f32, ih.max(1) as f32);
        let rem = window.rem_size();
        let (style_w, style_h) = {
            let size = &d.style().size;
            (size.width, size.height)
        };
        let (sized_w, sized_h) = (is_sized(style_w), is_sized(style_h));
        let painted = prev_size_of(id);
        let known_w = definite_px(style_w, rem).or(painted.filter(|_| sized_w).map(|p| p.0));
        let known_h = definite_px(style_h, rem).or(painted.filter(|_| sized_h).map(|p| p.1));
        let ratio = preferred_ratio(d.style().aspect_ratio, iw, ih);

        let (fill_w, fill_h) = match (sized_w, sized_h) {
            (true, true) => (None, None),
            // One axis given: the other follows the aspect ratio, as for `<img>`.
            (true, false) => (None, known_w.map(|w| w / ratio).or(Some(ih))),
            (false, true) => (known_h.map(|h| h * ratio).or(Some(iw)), None),
            (false, false) => (Some(iw), Some(iw / ratio)),
        };
        if let Some(w) = fill_w {
            d.style().size.width = Some(gpui::Length::Definite(gpui::px(w).into()));
        }
        if let Some(h) = fill_h {
            d.style().size.height = Some(gpui::Length::Definite(gpui::px(h).into()));
        }

        let size = painted.unwrap_or((
            known_w.or(fill_w).unwrap_or(iw),
            known_h.or(fill_h).unwrap_or(ih),
        ));

        let object_fit = node
            .computed
            .as_ref()
            .and_then(|cs| cs.extras.object_fit)
            .map(lottie_fit)
            // `objectfit` is the `<lottie-player>` spelling; CSS beats it, as a presentational hint
            // always loses.
            .or_else(|| match t.prop(id, "objectfit").map(str::trim) {
                Some("fill") => Some(sk_lottie::LottieFit::Fill),
                Some("contain") => Some(sk_lottie::LottieFit::Contain),
                Some("cover") => Some(sk_lottie::LottieFit::Cover),
                Some("scale-down") => Some(sk_lottie::LottieFit::ScaleDown),
                Some("none") => Some(sk_lottie::LottieFit::None),
                Some(other) => {
                    warn_compat(
                        &format!("lottie-objectfit:{other}"),
                        &format!(
                            "<lottie objectfit=\"{other}\"> is not an object-fit keyword — use \
                             fill, contain, cover, scale-down or none"
                        ),
                    );
                    None
                }
                None => None,
            })
            // Not the CSS initial value (`fill`): `<lottie-player>` defaults to
            // `preserveAspectRatio: "xMidYMid meet"` (contain) and this element mirrors it.
            .unwrap_or(sk_lottie::LottieFit::Contain);

        // The tile the worker's frames are written into (`crate::lottie`) and the box the next one
        // is drawn for. TODO: wire `filter: grayscale(1)` (gpui's image element does it natively)
        // once `Extras` carries `filter`.
        let position = object_position_of(d.style());
        d = d.child(
            match crate::lottie::paint(id, win, size, window.scale_factor()) {
                Some(image) => {
                    let mut picture = gpui::img(gpui::ImageSource::Render(image))
                        .object_fit(object_fit.into())
                        .size_full();
                    picture.style().vn_object_position = Some(position);
                    picture.into_any_element()
                }
                None => div().size_full().into_any_element(),
            },
        );
    }

    let mask = pointer_mask(node);
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some();
    let styled_focus = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        d = d.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
    }
    let click_focus = handle.filter(|_| focusable);
    bind_events(d, id, mask, click_focus, focusable, true, cx).into_any_element()
}

#[path = "canvas.rs"]
pub(crate) mod canvas;

#[cfg(feature = "lottie")]
#[path = "sticker.rs"]
pub(crate) mod sticker;
#[cfg(not(feature = "lottie"))]
#[path = "absent/sticker.rs"]
pub(crate) mod sticker;

/// An element whose engine this build left out (`crate::absent`): its CSS box, no content.
#[cfg(not(all(feature = "lottie", feature = "video")))]
fn build_absent<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    presentational_size(t, id, d.style());
    let mask = pointer_mask(node);
    bind_events(d, id, mask, None, false, true, cx).into_any_element()
}

#[cfg(feature = "video")]
fn video_fit(fit: sk_css::ObjectFit) -> sk_video::VideoFit {
    match fit {
        sk_css::ObjectFit::Fill => sk_video::VideoFit::Fill,
        sk_css::ObjectFit::Contain => sk_video::VideoFit::Contain,
        sk_css::ObjectFit::Cover => sk_video::VideoFit::Cover,
        sk_css::ObjectFit::ScaleDown => sk_video::VideoFit::ScaleDown,
        sk_css::ObjectFit::None => sk_video::VideoFit::None,
    }
}

/// `<iframe>`: a replaced box, 300 x 150 unless CSS sizes it. It paints nothing of the document: it
/// reports where it is, and the platform's webview is composited there (`crate::iframe`).
fn build_iframe<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    presentational_size(t, id, d.style());
    let tab_index = tab_index_of(t, id).or_else(|| default_tab_index(t, id));
    let focusable = tab_index.is_some();
    let handle = focusable.then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        d = d.track_focus(h);
        watch_focus(id, h, window, cx);
    }
    let auto = |l: Option<gpui::Length>| matches!(l, None | Some(gpui::Length::Auto));
    if auto(d.style().size.width) {
        d.style().size.width = Some(gpui::px(300.0).into());
    }
    if auto(d.style().size.height) {
        d.style().size.height = Some(gpui::px(150.0).into());
    }
    // The document has no window of its own to take the mouse (it is composited under the app), so
    // the box's listeners carry the pointer to it, only where the app's hit test resolves to this
    // box: under a menu or tooltip the app drew over it, the pointer is the app's.
    {
        use crate::iframe::PointerKind;
        use gpui::MouseButton;
        let held = |b: Option<MouseButton>| match b {
            Some(MouseButton::Left) => 1u8,
            Some(MouseButton::Right) => 2,
            Some(MouseButton::Middle) => 4,
            _ => 0,
        };
        let dom_button = |b: MouseButton| match b {
            MouseButton::Left => Some(0u8),
            MouseButton::Middle => Some(1),
            MouseButton::Right => Some(2),
            _ => None,
        };
        let at = |p: gpui::Point<gpui::Pixels>, window: &Window| {
            let s = window.scale_factor();
            (f32::from(p.x) * s, f32::from(p.y) * s)
        };
        d = d
            .on_mouse_move(move |ev: &gpui::MouseMoveEvent, window, _| {
                let m = ev.modifiers;
                crate::iframe::pointer(
                    id,
                    PointerKind::Move,
                    at(ev.position, window),
                    held(ev.pressed_button),
                    m.shift,
                    m.control,
                );
            })
            .on_any_mouse_down(move |ev: &gpui::MouseDownEvent, window, _| {
                let Some(button) = dom_button(ev.button) else {
                    return;
                };
                let m = ev.modifiers;
                crate::iframe::pointer(
                    id,
                    PointerKind::Down(button),
                    at(ev.position, window),
                    held(Some(ev.button)),
                    m.shift,
                    m.control,
                );
            })
            .on_scroll(move |ev: &gpui::ScrollEvent, window, cx| {
                // The wheel over a document is the document's; the app's scroll container under it
                // does not move too.
                if crate::iframe::has_document(id) {
                    cx.stop_propagation();
                }
                let delta = ev.delta.pixel_delta(window.line_height());
                let s = window.scale_factor();
                let m = ev.modifiers;
                // gpui: positive is away from the user (up); the frame's is down.
                let kind = PointerKind::Wheel(-f32::from(delta.x) * s, -f32::from(delta.y) * s);
                crate::iframe::pointer(id, kind, at(ev.position, window), 0, m.shift, m.control);
            })
            .on_hover(move |hovered: &bool, window, _| {
                if !*hovered {
                    let p = window.mouse_position();
                    crate::iframe::pointer(id, PointerKind::Leave, at(p, window), 0, false, false);
                }
            });
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            d = d.on_mouse_up(button, move |ev: &gpui::MouseUpEvent, window, _| {
                let Some(button) = dom_button(ev.button) else {
                    return;
                };
                let m = ev.modifiers;
                crate::iframe::pointer(
                    id,
                    PointerKind::Up(button),
                    at(ev.position, window),
                    0,
                    m.shift,
                    m.control,
                );
            });
        }
    }
    if let Some(style) = crate::iframe::cursor(id) {
        d = d.cursor(style);
    }
    let opacity = d.style().opacity.unwrap_or(1.0);
    let radii = d.style().corner_radii.clone();
    let reporter = gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let mask = window.content_mask().bounds;
            let rem = window.rem_size();
            let radius =
                |r: Option<gpui::AbsoluteLength>| r.map_or(gpui::px(0.0), |r| r.to_pixels(rem));
            let corners = gpui::Corners {
                top_left: radius(radii.top_left),
                top_right: radius(radii.top_right),
                bottom_right: radius(radii.bottom_right),
                bottom_left: radius(radii.bottom_left),
            };
            let s = window.scale_factor();
            let device_radii = [
                corners.top_left,
                corners.top_right,
                corners.bottom_right,
                corners.bottom_left,
            ]
            .map(|r| f32::from(r) * s);
            let place = crate::iframe::placement(bounds, mask, device_radii, s, opacity);
            crate::iframe::painted_in(id, window);
            crate::iframe::placed(id, place);
            // The document is composited under the app's surface: erase what the app painted in the
            // box so it shows through.
            if crate::iframe::has_document(id) {
                window.vn_paint_punch(bounds, corners);
            }
        },
    )
    .absolute()
    .size_full();
    bind_events(
        d.child(reporter),
        id,
        pointer_mask(node),
        handle,
        focusable,
        true,
        cx,
    )
    .into_any_element()
}

/// `<sk-surface name="…">`: a replaced box, 300 x 150 unless CSS sizes it, showing the frame a native
/// producer presented to the surface of that name (`crate::surface`). `object-fit` defaults to
/// `contain`, as on `<video>`.
fn build_surface<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let name = t.prop(id, "name").unwrap_or("").trim().to_string();
    let image = crate::surface::node::note_build(id, win, &name);

    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    presentational_size(t, id, d.style());
    let auto = |l: Option<gpui::Length>| matches!(l, None | Some(gpui::Length::Auto));
    if auto(d.style().size.width) {
        d.style().size.width = Some(gpui::px(300.0).into());
    }
    if auto(d.style().size.height) {
        d.style().size.height = Some(gpui::px(150.0).into());
    }

    let reporter = gpui::canvas(
        move |bounds, window, _| {
            crate::surface::node::note_painted(
                id,
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                window.scale_factor(),
            );
        },
        |_, (), _, _| {},
    )
    .absolute()
    .size_full();
    if let Some(image) = image {
        let fit = node
            .computed
            .as_ref()
            .and_then(|cs| cs.extras.object_fit)
            .map(gpui_object_fit)
            .unwrap_or(gpui::ObjectFit::Contain);
        let sampling = match node
            .computed
            .as_ref()
            .map(|cs| sk_css::image_rendering(&cs.vars))
        {
            Some("pixelated") => gpui::ImageSampling::Nearest,
            _ => gpui::ImageSampling::SmoothUpscale,
        };
        // Absolute: an in-flow `height: 100%` under an absolutely positioned box resolved to the
        // frame's aspect ratio instead of the box.
        let mut picture = gpui::img(gpui::ImageSource::Render(image))
            .object_fit(fit)
            .image_sampling(sampling)
            .absolute()
            .size_full();
        picture.style().vn_object_position = Some(object_position_of(d.style()));
        let picture = crate::surface::node::HdrPicture(id, picture.into_any_element());
        let radii = padding_box_radii(d.style(), window.rem_size());
        d = if radii.is_square() {
            d.child(picture)
        } else {
            let mut clip = div()
                .absolute()
                .size_full()
                .overflow_hidden()
                .child(picture);
            radii.apply(clip.style());
            d.child(clip)
        };
    }
    d = d.child(reporter);

    let mask = pointer_mask(node);
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some();
    let styled_focus = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        d = d.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
    }
    let click_focus = handle.filter(|_| focusable);
    bind_events(d, id, mask, click_focus, focusable, true, cx).into_any_element()
}

#[cfg(feature = "video")]
/// `<video>`: a replaced element whose content is the frame the pump last pulled from the OS media
/// pipeline. It stays inside the scene graph (Media Foundation frame-server mode, not a
/// DirectComposition visual) because a video on a sibling visual cannot be clipped, rounded, faded
/// or z-interleaved with CSS content. `poster` paints before the first frame.
fn build_video<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let intrinsic = crate::video::sync(t, id, win, window.scale_factor());

    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    presentational_size(t, id, d.style());

    let object_fit = node
        .computed
        .as_ref()
        .and_then(|cs| cs.extras.object_fit)
        .map(video_fit)
        // Not the CSS initial value (`fill`): HTML shows a video centred at the largest size that
        // fits, aspect ratio preserved.
        .unwrap_or(sk_video::VideoFit::Contain);

    let (iw, ih) = intrinsic.unwrap_or((0, 0));
    let mut painted_box = prev_size_of(id);
    if iw > 0 && ih > 0 {
        let (iw, ih) = (iw.max(1) as f32, ih.max(1) as f32);
        let rem = window.rem_size();
        let (style_w, style_h) = {
            let size = &d.style().size;
            (size.width, size.height)
        };
        let (sized_w, sized_h) = (is_sized(style_w), is_sized(style_h));
        let painted = prev_size_of(id);
        let known_w = definite_px(style_w, rem).or(painted.filter(|_| sized_w).map(|p| p.0));
        let known_h = definite_px(style_h, rem).or(painted.filter(|_| sized_h).map(|p| p.1));
        let ratio = preferred_ratio(d.style().aspect_ratio, iw, ih);

        let (fill_w, fill_h) = match (sized_w, sized_h) {
            (true, true) => (None, None),
            (true, false) => (None, known_w.map(|w| w / ratio).or(Some(ih))),
            (false, true) => (known_h.map(|h| h * ratio).or(Some(iw)), None),
            (false, false) => (Some(iw), Some(iw / ratio)),
        };
        if let Some(w) = fill_w {
            d.style().size.width = Some(gpui::Length::Definite(gpui::px(w).into()));
        }
        if let Some(h) = fill_h {
            d.style().size.height = Some(gpui::Length::Definite(gpui::px(h).into()));
        }
        painted_box = painted.or(Some((
            known_w.or(fill_w).unwrap_or(iw),
            known_h.or(fill_h).unwrap_or(ih),
        )));
    }

    let options = sk_video::VideoOptions {
        size: painted_box.unwrap_or((0.0, 0.0)),
        scale: window.scale_factor(),
        object_fit,
        object_position: object_position_of(d.style()),
    };
    crate::video::note_frame_box(id, &options);

    let has_frame = crate::video::with_state(id, |s| s.frame().is_some()).unwrap_or(false);
    if has_frame {
        if let Some(child) =
            crate::video::with_state(id, |s| sk_video::video(s, &options, window, &mut **cx))
        {
            let child = clip_replaced(child, d.style(), window.rem_size());
            d = d.child(child);
        }
    } else if let Some(poster) = t.prop(id, "poster").filter(|p| !p.trim().is_empty()) {
        if let Some(source) = img_src::source_for(id, poster.trim()) {
            let mut poster = gpui::img(source)
                .object_fit(gpui_object_fit(
                    node.computed
                        .as_ref()
                        .and_then(|cs| cs.extras.object_fit)
                        .unwrap_or(sk_css::ObjectFit::Contain),
                ))
                .size_full();
            poster.style().vn_object_position = Some(object_position_of(d.style()));
            let poster = poster.into_any_element();
            let poster = clip_replaced(poster, d.style(), window.rem_size());
            d = d.child(poster);
        }
    }

    let mask = pointer_mask(node);
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some();
    let styled_focus = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        d = d.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
    }
    let click_focus = handle.filter(|_| focusable);
    bind_events(d, id, mask, click_focus, focusable, true, cx).into_any_element()
}

fn file_input_children(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    case: Option<sk_css::TextTransform>,
    dark: bool,
) -> AnyElement {
    use gpui::Refineable as _;
    let (button_text, status) =
        crate::aria::file_input_labels(t, id).unwrap_or(("Choose File", String::new()));
    let (border, face, text) = if dark {
        (0x858585, 0x6b6b6b, gpui::white())
    } else {
        (0x767676, 0xefefef, gpui::black())
    };
    let mut button = div()
        .flex_none()
        .mr(gpui::px(4.0))
        .px(gpui::px(6.0))
        .py(gpui::px(1.0))
        .rounded(gpui::px(3.0))
        .border_1()
        .border_color(gpui::rgb(border))
        .bg(gpui::rgb(face))
        .text_color(text);
    if let Some(r) = node
        .computed
        .as_ref()
        .and_then(|cs| cs.file_selector_button.as_ref())
    {
        button.style().refine(r);
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .child(button.child(transformed_text(button_text, case)))
        .child(div().flex_none().child(transformed_text(&status, case)))
        .into_any_element()
}

/// The native `<input>` / `<textarea>` widget: a persistent `TextInputState` entity per node,
/// controlled through the DOM-compat prop channel, reporting edits as `input` events and keystrokes
/// as cancellable `keydown`/`keyup`.
fn build_input<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    tab_index: Option<isize>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (AnyElement, FocusHandle) {
    let state = INPUT_STATES.with(|m| {
        m.borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.new(|cx| TextInputState::new(window, cx)))
            .clone()
    });

    let multiline = t.name(node.tag) == "textarea";
    state.update(cx, |s, _| s.set_multiline(multiline));
    for shown in [
        t.prop(id, "value"),
        t.prop(id, "placeholder"),
        Some(node.text.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        crate::fonts::want_for_text(t, id, shown);
    }

    // Controlled value: the write counter, not the string, tells the widget an assignment happened
    // (`v-model` re-forcing "" after the user typed again writes the same string the previous clear
    // did).
    if let Some((value, write)) = t.prop_write(id, "value") {
        let value = value.to_string();
        // A value assigned in the middle of an IME composition ends it: its `compositionend` goes
        // out after this frame.
        state.update(cx, |s, cx| {
            s.sync_value(&value, write, cx);
            s.flush_abandoned_composition(window, cx);
        });
    } else if multiline && !node.text.is_empty() {
        let value = node.text.clone();
        state.update(cx, |s, cx| s.sync_value(&value, 0, cx));
    }

    if INPUT_SELECT_ALL.with(|m| m.borrow_mut().remove(&id)) {
        state.update(cx, |s, cx| s.select_all_text(cx));
    }

    let text_style = node.computed.as_ref().map(|cs| cs.base.text.clone());
    // No `color` of its own: the widget inherits one at paint time; this is the fallback for the
    // frame before it paints.
    let inherit_color = text_style.as_ref().and_then(|ts| ts.color).is_none();
    let text_color = text_style
        .as_ref()
        .and_then(|ts| ts.color)
        .unwrap_or_else(gpui::white);
    let font_size = text_style
        .as_ref()
        .and_then(|ts| ts.font_size)
        .map(|al| al.to_pixels(window.rem_size()))
        .unwrap_or(gpui::px(14.0));
    let placeholder_color = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.placeholder.as_ref())
        .and_then(|ph| ph.text.color)
        .unwrap_or(gpui::Hsla {
            a: text_color.a * 0.45,
            ..text_color
        });
    // `::selection { background }`; its `color` cannot land: gpui's selection is a quad behind an
    // already shaped run, and recolouring glyphs would mean splitting the run on every caret move.
    let selection_color = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.selection.as_ref())
        .and_then(|s| s.background.as_ref())
        .and_then(gpui::Fill::color)
        .and_then(|b| b.as_solid())
        .unwrap_or(gpui::Hsla {
            h: 0.6,
            s: 0.8,
            l: 0.6,
            a: 0.35,
        });
    let masked = !multiline
        && t.prop(id, "type")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("password"));
    let options = TextInputOptions {
        placeholder: t.prop(id, "placeholder").unwrap_or("").to_string().into(),
        text_color,
        placeholder_color,
        selection_color,
        caret_color: text_color,
        font_size,
        // `readonly` and `disabled` differ in HTML, but for the editor both mean no typing and no
        // caret (selection and copy still work), so both map onto the widget's one flag.
        disabled: t.prop(id, "disabled").is_some_and(|v| v != "false")
            || t.prop(id, "readonly").is_some_and(|v| v != "false"),
        tab_index,
        multiline,
        // `text-align` is inherited: an `<input class="text-center">` carries it itself, while one
        // inside a centred wrapper gets it from the paint-time text style, which the widget cannot
        // see at prepaint when the offset is computed.
        text_align: node
            .computed
            .as_ref()
            .and_then(|cs| cs.base.text.text_align)
            .unwrap_or(gpui::TextAlign::Left),
        inherit_color,
        rows: t
            .prop(id, "rows")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(sk_elements::DEFAULT_ROWS),
        input_config: {
            let mut config = input_configuration(t, id, multiline);
            // A browser offers no corrections or suggestions for a password: they would learn it.
            if masked {
                config.autocorrect = false;
                config.suggestions = false;
            }
            config
        },
        // Read every frame, so a "show password" toggle that switches `type` to `text` unmasks on
        // the next frame.
        masked,
        // `pointer-events: none`, inherited (resolved before the states were stored). An inert
        // field takes no press at all (`TextInputOptions::inert`).
        inert: node
            .computed
            .as_ref()
            .is_some_and(|cs| cs.extras.pointer_events_none == Some(true)),
    };

    let mask = pointer_mask(node);
    let bit = move |ev: u32| mask & (1u64 << ev) != 0;

    // sk-elements reports an edit from a `window.defer`, outside gpui's key dispatch, so a forced
    // layout may draw in the `input` handler.
    let on_change: sk_elements::OnChange = std::rc::Rc::new(move |text, intent, window, cx| {
        let payload = EventPayload {
            key: Some(text),
            ..intent_payload(intent)
        };
        crate::off_input(|| {
            js_dispatch_text(
                id,
                tree::EV_INPUT,
                &payload,
                intent.data.as_deref(),
                window,
                cx,
            )
        });
    });

    let on_before_input: sk_elements::OnBeforeInput =
        std::rc::Rc::new(move |intent, window: &mut Window, cx: &mut App| {
            if !tree::read_tree(|t| path_listens(t, id, tree::EV_BEFOREINPUT)) {
                return true;
            }
            let payload = intent_payload(intent);
            let proceed = js_dispatch_text(
                id,
                tree::EV_BEFOREINPUT,
                &payload,
                intent.data.as_deref(),
                window,
                cx,
            );
            if proceed {
                settle_before_default(window, cx);
            }
            proceed
        });

    // The IME composition's events; `End` comes deferred, after the commit's `input`, outside
    // gpui's dispatch.
    let on_composition: sk_elements::OnComposition =
        std::rc::Rc::new(move |phase, data, window: &mut Window, cx: &mut App| {
            let code = match phase {
                sk_elements::CompositionPhase::Start => tree::EV_COMPOSITIONSTART,
                sk_elements::CompositionPhase::Update => tree::EV_COMPOSITIONUPDATE,
                sk_elements::CompositionPhase::End => tree::EV_COMPOSITIONEND,
            };
            if !tree::read_tree(|t| path_listens(t, id, code)) {
                return;
            }
            let payload = EventPayload::default();
            if phase == sk_elements::CompositionPhase::End {
                crate::off_input(|| js_dispatch_text(id, code, &payload, Some(data), window, cx));
            } else {
                js_dispatch_text(id, code, &payload, Some(data), window, cx);
            }
        });

    let on_key_down: Option<sk_elements::OnKeyDown> = bit(tree::EV_KEYDOWN).then(|| {
        std::rc::Rc::new(
            move |ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App| {
                // The field's interceptor runs before any listener, so it is the one `keydown` of this
                // key event; the root's listener, which gpui reaches afterwards even when this handler
                // removed the field, lets it pass (`claim_key_event`).
                if !claim_key_event(window) {
                    return true;
                }
                let key =
                    crate::keyboard::Keyboard::new(&ev.keystroke, ev.native.as_ref(), ev.is_held);
                let proceed = key.dispatch(id, tree::EV_KEYDOWN, window, cx);
                if !proceed {
                    // The root's key listeners (clipboard, Shift+F10) run after the field's and read
                    // this.
                    windows::here(window, |w| w.key_prevented = true);
                } else {
                    settle_before_default(window, cx);
                }
                proceed
            },
        ) as sk_elements::OnKeyDown
    });
    let on_key_up: Option<sk_elements::OnKeyUp> = bit(tree::EV_KEYUP).then(|| {
        std::rc::Rc::new(move |ev: &KeyUpEvent, window: &mut Window, cx: &mut App| {
            if !claim_key_event(window) {
                return;
            }
            let key = crate::keyboard::Keyboard::new(&ev.keystroke, ev.native.as_ref(), false);
            if !key.dispatch(id, tree::EV_KEYUP, window, cx) {
                windows::here(window, |w| w.key_prevented = true);
            }
        }) as sk_elements::OnKeyUp
    });

    let on_clipboard: sk_elements::OnClipboard =
        std::rc::Rc::new(move |command, window: &mut Window, cx: &mut App| {
            let proceed =
                crate::clipboard::fire(id, crate::clipboard::Verb::of_command(command), window, cx);
            if proceed {
                settle_before_default(window, cx);
            }
            proceed
        });

    let controlled_value: sk_elements::ControlledValue = std::rc::Rc::new(move |_| {
        tree::read_tree(|t| {
            t.prop_write(id, "value")
                .map(|(v, write)| (v.to_string(), write))
        })
    });

    let handlers = TextInputHandlers {
        on_change: Some(on_change),
        on_before_input: Some(on_before_input),
        on_composition: Some(on_composition),
        on_key_down,
        on_key_up,
        on_clipboard: Some(on_clipboard),
        controlled_value: Some(controlled_value),
    };

    let focus_target = state.read(cx).focus_handle();
    INPUT_FOCUS.with(|m| m.borrow_mut().insert(id, focus_target.clone()));
    // Edge-triggered: only here is "gpui took the configuration" distinguishable from "we built it"
    // (`TextInputState::newly_forwarded_configuration`).
    if let Some((config, queries)) = state.update(cx, |s, _| s.newly_forwarded_configuration()) {
        let line = format!(
            "[input] text input configuration forwarded to gpui (query {queries}): \
             autocorrect={} autocapitalize={:?} suggestions={} input_action={:?}",
            config.autocorrect, config.autocapitalize, config.suggestions, config.input_action,
        );
        thread_local! {
            static SEEN: std::cell::RefCell<std::collections::HashSet<String>> =
                std::cell::RefCell::new(std::collections::HashSet::new());
        }
        if SEEN.with(|s| s.borrow_mut().insert(line.clone())) {
            crate::log::record("dev", &format!("{line} (first: #{id})"));
        }
    }
    (text_input(&state, options, handlers, cx), focus_target)
}

fn input_configuration(t: &Tree, id: u32, multiline: bool) -> gpui::TextInputConfiguration {
    let attributes = sk_elements::InputAttributes {
        autocorrect: t.prop(id, "autocorrect"),
        autocapitalize: t.prop(id, "autocapitalize"),
        spellcheck: t.prop(id, "spellcheck"),
        enterkeyhint: t.prop(id, "enterkeyhint"),
        inputmode: t.prop(id, "inputmode"),
        input_type: (!multiline).then(|| t.prop(id, "type")).flatten(),
    };
    let mapped = sk_elements::configuration(attributes);
    let element = if multiline { "textarea" } else { "input" };
    for (attribute, value) in &mapped.unsupported {
        warn_compat(
            &format!("input-attr:{attribute}={value}"),
            &format!(
                "<{element} {attribute}=\"{value}\"> — not a value HTML defines for \
                 `{attribute}`; the attribute was ignored and the field keeps the platform \
                 default"
            ),
        );
    }
    mapped.config
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_focused_node_that_stopped_being_focusable_does_not_keep_the_focus() {
        crate::tree::with_tree(|t| {
            let root = t.create_root();
            let button = t.intern("button");
            let div = t.intern("div");
            let button = t.create_element(button);
            let plain = t.create_element(div);
            t.insert(button, root, 0);
            t.insert(plain, root, 0);
            assert!(super::keeps_focus(t, button));
            assert!(
                !super::keeps_focus(t, plain),
                "a div was never a focusable area"
            );
            let tabindex = t.intern("tabindex");
            t.set_prop(plain, tabindex, Some("0".to_string()));
            assert!(super::keeps_focus(t, plain));
            t.set_prop(plain, tabindex, None);
            assert!(!super::keeps_focus(t, plain), "its tabindex was removed");
            let disabled = t.intern("disabled");
            t.set_prop(button, disabled, Some(String::new()));
            assert!(!super::keeps_focus(t, button), "it disabled itself");
            t.set_prop(button, disabled, None);
            t.remove(button);
            assert!(!super::keeps_focus(t, button), "it left the document");
        });
    }

    #[test]
    fn select_on_an_unbuilt_text_field_is_preserved_until_build_and_removed_by_gc() {
        let (field, other) = crate::tree::with_tree(|t| {
            let root = t.create_root();
            let textarea = t.intern("textarea");
            let div = t.intern("div");
            let field = t.create_element(textarea);
            let other = t.create_element(div);
            t.insert(field, root, 0);
            t.insert(other, root, 0);
            let value = t.intern("value");
            t.set_prop(field, value, Some("Привет 🙂\nSecond line".to_string()));
            (field, other)
        });
        assert!(super::select_input_text(field));
        assert!(super::INPUT_SELECT_ALL.with(|m| m.borrow().contains(&field)));
        assert!(!super::select_input_text(other));
        assert!(!super::INPUT_SELECT_ALL.with(|m| m.borrow().contains(&other)));
        crate::tree::with_tree(|t| t.remove(field));
        super::gc_node_state();
        assert!(!super::INPUT_SELECT_ALL.with(|m| m.borrow().contains(&field)));
    }

    use super::*;
    use crate::tree::with_tree;

    fn dom_press(target: u32, button: f64) -> Press {
        Press {
            target,
            button,
            position: Point::default(),
            clickable: true,
            cancelled: false,
        }
    }

    #[test]
    fn dom_press_survives_listener_rebuild_until_its_matching_release() {
        let mut state = windows::WindowState::default();
        state.press = Some(dom_press(42, 0.0));
        assert!(Press::take_for_release(&mut state.press, 2.0).is_none());
        assert_eq!(state.press.as_ref().map(|p| p.target), Some(42));
        let wave = UpWave {
            press: Press::take_for_release(&mut state.press, 0.0),
            ..Default::default()
        };
        assert_eq!(wave.click_press(), Some(42));
        assert!(state.press.is_none());
        assert!(
            Press::take_for_release(&mut state.press, 0.0).is_none(),
            "one press cannot click twice"
        );
        assert_eq!(
            UpWave::default().click_press(),
            None,
            "a release without a press cannot click"
        );
    }

    #[test]
    fn cancelled_or_ineligible_dom_presses_cannot_click() {
        let mut cancelled = dom_press(42, 0.0);
        cancelled.cancelled = true;
        assert!(!cancelled.is_active());
        assert_eq!(
            UpWave {
                press: Some(cancelled),
                ..Default::default()
            }
            .click_press(),
            None
        );
        let mut disabled_or_native_drag = dom_press(42, 0.0);
        disabled_or_native_drag.clickable = false;
        assert_eq!(
            UpWave {
                press: Some(disabled_or_native_drag),
                ..Default::default()
            }
            .click_press(),
            None
        );
    }

    fn z(z_index: i32) -> sk_css::Extras {
        sk_css::Extras {
            z_index: Some(Some(z_index)),
            positioned: Some(true),
            ..Default::default()
        }
    }

    fn fixed() -> sk_css::Extras {
        sk_css::Extras {
            fixed: Some(true),
            positioned: Some(true),
            ..Default::default()
        }
    }

    fn layer(x: &sk_css::Extras) -> Layer {
        let position = (x.fixed == Some(true)).then_some(gpui::Position::Absolute);
        layer_of(x, position, false, None)
    }

    struct B {
        name: &'static str,
        layer: Layer,
        kids: Vec<B>,
    }

    fn b(name: &'static str, layer: Layer, kids: Vec<B>) -> B {
        B { name, layer, kids }
    }

    enum Item {
        Paint(&'static str),
        Inline(Vec<Item>),
        Deferred(std::rc::Rc<[u128]>, Vec<Item>),
    }

    fn lower(node: &B, parent: &Stacking, is_root: bool) -> Item {
        let layer = if is_root { Layer::InFlow } else { node.layer };
        let Entered {
            own,
            inside,
            opens,
            negative,
        } = parent.enter(layer, next_stack_ordinal());
        if negative {
            note_negative_z();
        }
        let opens = opens || is_root;
        if opens {
            open_context();
        }
        let mut kids: Vec<Item> = node.kids.iter().map(|k| lower(k, &inside, false)).collect();
        let mut split = false;
        if opens && close_context() {
            split = true;
            kids = kids
                .into_iter()
                .map(|k| Item::Deferred(inside.in_flow_key(), vec![k]))
                .collect();
        }
        let own = own.or_else(|| (split && !is_root).then(|| inside.key.clone()));
        let mut body = vec![Item::Paint(node.name)];
        body.extend(kids);
        match own {
            Some(key) => Item::Deferred(key, body),
            None => Item::Inline(body),
        }
    }

    fn flatten(
        item: Item,
        painted: &mut Vec<&'static str>,
        draws: &mut Vec<(std::rc::Rc<[u128]>, Vec<&'static str>)>,
    ) {
        match item {
            Item::Paint(name) => painted.push(name),
            Item::Inline(items) => items.into_iter().for_each(|i| flatten(i, painted, draws)),
            Item::Deferred(key, items) => {
                let mut own = Vec::new();
                items.into_iter().for_each(|i| flatten(i, &mut own, draws));
                draws.push((key, own));
            }
        }
    }

    fn paint_order(root: &B) -> Vec<&'static str> {
        reset_stacking();
        let (mut painted, mut draws) = (Vec::new(), Vec::new());
        flatten(
            lower(root, &Stacking::default(), true),
            &mut painted,
            &mut draws,
        );
        draws.sort_by(|a, b| a.0.cmp(&b.0));
        painted.extend(draws.into_iter().flat_map(|(_, names)| names));
        painted
    }

    #[test]
    fn a_context_paints_whole_before_a_later_sibling_of_the_same_z() {
        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b("app", Layer::InFlow, vec![b("page", Layer::InFlow, vec![])]),
                b(
                    "popover",
                    layer(&z(50)),
                    vec![b(
                        "inner",
                        Layer::InFlow,
                        vec![
                            b("card-bg", Layer::Context(0), vec![]),
                            b("hero", layer(&z(1)), vec![]),
                            b(
                                "glass",
                                layer(&z(2)),
                                vec![b("glass-body", Layer::InFlow, vec![])],
                            ),
                        ],
                    )],
                ),
                b(
                    "menu",
                    layer(&z(50)),
                    vec![b("item", Layer::InFlow, vec![])],
                ),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            [
                "root",
                "app",
                "page",
                "popover",
                "inner",
                "card-bg",
                "hero",
                "glass",
                "glass-body",
                "menu",
                "item"
            ]
        );
    }

    #[test]
    fn a_negative_z_paints_between_its_contexts_background_and_content() {
        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "card",
                    layer(&z(1)),
                    vec![
                        b("text", Layer::InFlow, vec![]),
                        b("glow", Layer::Context(-1), vec![]),
                        b("box", Layer::InFlow, vec![b("deep", layer(&z(-2)), vec![])]),
                    ],
                ),
                b("after", Layer::InFlow, vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            ["root", "after", "card", "deep", "glow", "text", "box"]
        );

        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "page",
                    Layer::InFlow,
                    vec![b("backdrop", layer(&z(-1)), vec![])],
                ),
                b("footer", Layer::InFlow, vec![]),
            ],
        );
        assert_eq!(paint_order(&tree), ["root", "backdrop", "page", "footer"]);

        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "avatar",
                    Layer::InFlowContext,
                    vec![
                        b("face", Layer::InFlow, vec![]),
                        b("frame", layer(&z(-1)), vec![]),
                    ],
                ),
                b("name", Layer::InFlow, vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            ["root", "name", "avatar", "frame", "face"]
        );
    }

    #[test]
    fn nested_contexts_sort_by_their_path() {
        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "a",
                    layer(&z(1)),
                    vec![
                        b(
                            "a1",
                            layer(&z(3)),
                            vec![
                                b("a1x", layer(&z(100)), vec![]),
                                b("a1y", layer(&z(-5)), vec![]),
                                b("a1-text", Layer::InFlow, vec![]),
                            ],
                        ),
                        b("a2", layer(&z(2)), vec![]),
                    ],
                ),
                b(
                    "b",
                    layer(&z(2)),
                    vec![
                        b("b1", layer(&z(-1)), vec![]),
                        b("b-text", Layer::InFlow, vec![]),
                    ],
                ),
                b("c", layer(&z(1)), vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            ["root", "a", "a2", "a1", "a1y", "a1-text", "a1x", "c", "b", "b1", "b-text"]
        );
    }

    #[test]
    fn an_effect_makes_a_context_at_z_auto() {
        let style = |f: &dyn Fn(&mut gpui::StyleRefinement)| {
            let mut s = gpui::StyleRefinement::default();
            f(&mut s);
            s
        };
        let none = sk_css::Extras::default();
        assert!(!effect_context(&none, &style(&|_| {})));
        assert!(effect_context(&none, &style(&|s| s.opacity = Some(0.5))));
        assert!(!effect_context(&none, &style(&|s| s.opacity = Some(1.0))));
        let mut moved = gpui::TransformSpec::IDENTITY;
        moved.scale = (0.95, 0.95);
        assert!(effect_context(
            &none,
            &style(&|s| s.vn_transform = Some(moved))
        ));
        assert!(!effect_context(
            &none,
            &style(&|s| s.vn_transform = Some(gpui::TransformSpec::IDENTITY))
        ));
        assert!(effect_context(
            &none,
            &style(&|s| s.backdrop_blur = Some(gpui::px(8.0)))
        ));
        let isolated = sk_css::Extras {
            isolate: Some(true),
            ..Default::default()
        };
        assert!(effect_context(&isolated, &style(&|_| {})));

        let abs = Some(gpui::Position::Absolute);
        assert_eq!(layer_of(&none, None, true, None), Layer::InFlowContext);
        assert_eq!(layer_of(&none, abs, true, None), Layer::Context(0));
        assert_eq!(layer_of(&none, abs, false, None), Layer::Positioned);

        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "faded",
                    Layer::InFlowContext,
                    vec![b("lifted", layer(&z(100)), vec![])],
                ),
                b("later", layer(&z(10)), vec![]),
            ],
        );
        assert_eq!(paint_order(&tree), ["root", "faded", "lifted", "later"]);

        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "moved",
                    layer_of(&none, abs, true, None),
                    vec![b("inner", layer(&z(5)), vec![])],
                ),
                b("badge", Layer::Positioned, vec![]),
                b("later", layer(&z(1)), vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            ["root", "moved", "inner", "badge", "later"]
        );
    }

    #[test]
    fn fixed_and_z_zero_are_contexts_and_auto_is_not() {
        assert_eq!(layer(&fixed()), Layer::Context(0));
        let zero_rel = z(0);
        assert_eq!(layer(&zero_rel), Layer::Context(0));
        assert_eq!(
            layer_of(&zero_rel, Some(gpui::Position::Absolute), false, None),
            Layer::Context(0)
        );
        let auto = sk_css::Extras {
            z_index: Some(None),
            positioned: Some(true),
            ..Default::default()
        };
        assert_eq!(layer(&auto), Layer::Positioned);
        let sticky = sk_css::Extras {
            sticky: Some(true),
            ..auto.clone()
        };
        assert_eq!(layer(&sticky), Layer::Context(0));

        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "overlay",
                    layer(&fixed()),
                    vec![b("inside", layer(&z(1)), vec![])],
                ),
                b("document-z1", layer(&z(1)), vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            ["root", "overlay", "inside", "document-z1"]
        );
        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "zero",
                    layer(&zero_rel),
                    vec![b("inside", layer(&z(9)), vec![])],
                ),
                b(
                    "auto",
                    layer(&auto),
                    vec![b("escapes", layer(&z(8)), vec![])],
                ),
                b("document-z1", layer(&z(1)), vec![]),
                b("later-in-flow", Layer::InFlow, vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            [
                "root",
                "later-in-flow",
                "zero",
                "inside",
                "auto",
                "document-z1",
                "escapes"
            ]
        );
    }

    #[test]
    fn z_index_applies_only_to_positioned_boxes_and_flex_or_grid_items() {
        let z10 = |positioned: Option<bool>| sk_css::Extras {
            z_index: Some(Some(10)),
            positioned,
            ..Default::default()
        };
        let block = None;
        let flex = Some(gpui::Display::Flex);
        let grid = Some(gpui::Display::Grid);

        let at = |x: &sk_css::Extras, parent| layer_of(x, None, false, parent);
        assert_eq!(
            at(&z10(None), block),
            Layer::InFlow,
            "static (undeclared) in a block"
        );
        assert_eq!(
            at(&z10(Some(false)), block),
            Layer::InFlow,
            "`position: static`"
        );
        assert_eq!(
            at(&z10(Some(false)), Some(gpui::Display::Block)),
            Layer::InFlow
        );
        assert_eq!(
            at(&z10(None), flex),
            Layer::Context(10),
            "a static flex item"
        );
        assert_eq!(
            at(&z10(None), grid),
            Layer::Context(10),
            "a static grid item"
        );
        assert_eq!(
            at(&z10(Some(true)), block),
            Layer::Context(10),
            "relative in a block"
        );
        assert!(!z_applies(&z10(None), block) && z_applies(&z10(None), flex));

        let root = Stacking::default();
        let e = root.enter(at(&z10(None), block), 0);
        assert!(e.own.is_none() && !e.opens);
        assert_eq!(e.inside.key, root.key);
    }

    #[test]
    fn an_absolute_z_auto_box_paints_after_in_flow_content_and_opens_no_context() {
        let absolute = sk_css::Extras {
            positioned: Some(true),
            ..Default::default()
        };
        let abs = Some(gpui::Position::Absolute);
        assert_eq!(layer_of(&absolute, abs, false, None), Layer::Positioned);
        assert_eq!(
            layer_of(&absolute, None, false, None),
            Layer::Positioned,
            "a relative box with z-index auto uses the same paint layer"
        );

        let tree = b(
            "root",
            Layer::InFlow,
            vec![
                b(
                    "field",
                    Layer::InFlow,
                    vec![
                        b(
                            "badge",
                            Layer::Positioned,
                            vec![
                                b("badge-text", Layer::InFlow, vec![]),
                                b("badge-dot", Layer::Positioned, vec![]),
                                b("badge-tip", layer(&z(5)), vec![]),
                            ],
                        ),
                        b("input", Layer::InFlow, vec![]),
                    ],
                ),
                b("z1-box", layer(&z(1)), vec![]),
                b("later-abs", Layer::Positioned, vec![]),
            ],
        );
        assert_eq!(
            paint_order(&tree),
            [
                "root",
                "field",
                "input",
                "badge",
                "badge-text",
                "badge-dot",
                "later-abs",
                "z1-box",
                "badge-tip"
            ]
        );
    }

    #[test]
    fn the_cache_does_not_compare_stacking_keys() {
        let root = Stacking::default();
        let a = Inherited {
            stack: root.enter(layer(&z(1)), 3).inside,
            ..Default::default()
        };
        let b = Inherited {
            stack: root.enter(layer(&z(1)), 7).inside,
            ..Default::default()
        };
        assert_ne!(a.stack.key, b.stack.key);
        assert!(a == b);
    }

    #[test]
    fn deferred_nesting_stops_below_gpuis_round_limit() {
        let mut s = Stacking::default();
        for level in 0..MAX_DEFERRED_NESTING {
            let e = s.enter(Layer::Positioned, u64::from(level));
            assert!(e.own.is_some(), "level {level} defers");
            s = e.inside;
        }
        assert!(s.enter(Layer::Positioned, 99).own.is_none());
        assert!(s.enter(Layer::Context(-1), 99).own.is_none());
        assert!(
            !s.enter(Layer::Context(-1), 99).negative,
            "painted in flow: nothing to split"
        );
        let e = s.enter(Layer::Context(3), 99);
        assert!(e.own.is_none() && e.opens && e.inside.key.len() == s.key.len() + 1);
    }

    #[test]
    fn slots_order_z_then_tree_order() {
        assert!(z_rank(i32::MIN) < z_rank(-1));
        assert!(z_rank(-1) < IN_FLOW && IN_FLOW < z_rank(0));
        assert!(z_rank(0) < z_rank(1) && z_rank(1) < z_rank(i32::MAX));
        assert!(slot(z_rank(1), 9) < slot(z_rank(2), 0));
        assert!(slot(z_rank(1), 0) < slot(z_rank(1), 1));
    }

    #[test]
    fn only_an_element_with_an_activation_behaviour_clicks_from_a_key() {
        let el = |tag: &str, attrs: &[(&str, &str)]| {
            with_tree(|t| {
                let tag = t.intern(tag);
                let id = t.create_element(tag);
                for (name, value) in attrs {
                    let key = t.intern(name);
                    t.set_prop(id, key, Some((*value).to_string()));
                }
                id
            })
        };
        let cases = [
            (el("button", &[]), true, true),
            (el("button", &[("type", "reset")]), true, true),
            (el("summary", &[]), true, true),
            (el("a", &[("href", "#x")]), true, false),
            (el("a", &[]), false, false),
            (el("area", &[("href", "#x")]), true, false),
            (el("input", &[("type", "submit")]), true, true),
            (el("input", &[("type", " Image ")]), true, true),
            (el("input", &[("type", "color")]), true, true),
            (el("input", &[("type", "checkbox")]), false, true),
            (el("input", &[("type", "radio")]), false, true),
            (el("input", &[("type", "file")]), false, false),
            (el("input", &[]), false, false),
            (el("label", &[]), false, false),
            (
                el("div", &[("role", "button"), ("tabindex", "0")]),
                false,
                false,
            ),
        ];
        for (i, (id, enter, space)) in cases.into_iter().enumerate() {
            assert_eq!(
                (keyboard_activates(id, false), keyboard_activates(id, true)),
                (enter, space),
                "case {i}"
            );
        }
    }

    #[test]
    fn fragment_anchors_are_not_flex_items() {
        with_tree(|t| {
            let div = t.intern("div");
            let row = t.create_element(div);
            let mut boxes = Vec::new();
            for _ in 0..3 {
                let anchor = t.create_anchor();
                t.insert(anchor, row, 0);
                let blank = t.create_text();
                t.insert(blank, row, 0); // createText("") — a Fragment anchor
                let item = t.create_element(div);
                t.insert(item, row, 0);
                boxes.push(item);
            }
            let node = t.node(row).unwrap();
            assert_eq!(node.children.len(), 9, "the retained tree keeps all nine");
            let laid_out: Vec<u32> = laid_out_children(t, node);
            assert_eq!(laid_out, boxes, "only the three elements are flex items");
        });
    }

    #[test]
    fn a_skipped_anchor_is_still_an_insertion_point() {
        with_tree(|t| {
            let div = t.intern("div");
            let parent = t.create_element(div);
            let anchor = t.create_anchor();
            t.insert(anchor, parent, 0);
            let later = t.create_element(div);
            t.insert(later, parent, 0);
            let first = t.create_element(div);
            t.insert(first, parent, anchor);

            let node = t.node(parent).unwrap();
            assert_eq!(node.children, vec![first, anchor, later]);
            assert_eq!(
                laid_out_children(t, node),
                vec![first, later],
                "the anchor still orders the boxes without being one"
            );
        });
    }

    #[test]
    fn an_empty_element_is_still_a_box() {
        with_tree(|t| {
            let div = t.intern("div");
            let parent = t.create_element(div);
            let empty_div = t.create_element(div);
            t.insert(empty_div, parent, 0);
            let text = t.create_text();
            t.set_text(text, "hello".to_string());
            t.insert(text, parent, 0);
            let blank = t.create_text();
            t.insert(blank, parent, 0);

            let node = t.node(parent).unwrap();
            assert_eq!(laid_out_children(t, node), vec![empty_div, text]);
            assert!(!generates_a_box(t, blank));
            assert!(!generates_a_box(t, 99_999));
        });
    }

    #[test]
    fn collapsible_white_space_is_a_box_only_between_inline_content() {
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");
            let text = |t: &mut Tree, s: &str, parent: u32| {
                let n = t.create_text();
                t.set_text(n, s.to_string());
                t.insert(n, parent, 0);
                n
            };
            let element = |t: &mut Tree, tag: u32, parent: u32| {
                let n = t.create_element(tag);
                t.insert(n, parent, 0);
                n
            };
            let set_style = |t: &mut Tree, id: u32, f: &dyn Fn(&mut sk_css::ComputedStates)| {
                let mut cs = sk_css::ComputedStates::default();
                f(&mut cs);
                if let Some(n) = t.node_mut(id) {
                    n.computed = Some(cs);
                }
            };

            // Between blocks, as Svelte keeps it: `<h1/> <button/>` in a block container.
            let blocks = t.create_element(div);
            let h1 = element(t, div, blocks);
            let between = text(t, " ", blocks);
            let after = element(t, div, blocks);
            let edge = text(t, "\n  ", blocks);
            let anchor = t.create_anchor();
            t.insert(anchor, blocks, 0);
            assert_eq!(
                laid_out_children(t, t.node(blocks).unwrap()),
                vec![h1, after]
            );
            assert!(!generates_a_box(t, between) && !generates_a_box(t, edge));

            // The word gap of `<span>a</span> <span>b</span>`, and a line's trailing space.
            let line = t.create_element(div);
            let a = element(t, span, line);
            let gap = text(t, " ", line);
            let b = element(t, span, line);
            let trailing = text(t, " ", line);
            assert_eq!(laid_out_children(t, t.node(line).unwrap()), vec![a, gap, b]);
            assert!(!generates_a_box(t, trailing));
            let mut style = gpui::StyleRefinement::default();
            apply_inline_row(&mut style, t, t.node(line).unwrap(), None);
            assert_eq!(style.display, Some(gpui::Display::Flex));

            // Text beside it counts as inline content, and so does a blank inside an inline element.
            let words = t.create_element(div);
            text(t, "a", words);
            let word_gap = text(t, " ", words);
            element(t, span, words);
            assert!(generates_a_box(t, word_gap));
            let inner = element(t, span, words);
            let inside = text(t, " ", inner);
            assert!(generates_a_box(t, inside));

            // A flex container renders no white-space-only run, even between inline items.
            let flex = t.create_element(div);
            set_style(t, flex, &|cs| {
                let mut base = gpui::StyleRefinement::default();
                base.display = Some(gpui::Display::Flex);
                cs.base = std::rc::Rc::new(base);
            });
            element(t, span, flex);
            let flex_gap = text(t, " ", flex);
            element(t, span, flex);
            assert!(!generates_a_box(t, flex_gap));

            // Preserved white space is content.
            let pre = t.create_element(div);
            set_style(t, pre, &|cs| {
                cs.extras.white_space = Some(sk_css::WhiteSpaceMode::Pre)
            });
            element(t, div, pre);
            let kept = text(t, "  ", pre);
            element(t, div, pre);
            assert!(generates_a_box(t, kept));
            let pre_line = t.create_element(div);
            set_style(t, pre_line, &|cs| {
                cs.extras.white_space = Some(sk_css::WhiteSpaceMode::PreLine)
            });
            let space = text(t, " ", pre_line);
            let newline = text(t, "\n", pre_line);
            assert!(!generates_a_box(t, space));
            assert!(generates_a_box(t, newline));
        });
    }

    #[test]
    fn inline_children_share_a_line_even_with_no_bare_text() {
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");

            let block = t.create_element(div);
            for i in 0..2u32 {
                let s = t.create_element(span);
                let text = t.create_text();
                t.set_text(text, "x".to_string());
                t.insert(text, s, 0);
                t.insert(s, block, i);
            }
            let mut style = gpui::StyleRefinement::default();
            apply_inline_row(&mut style, t, t.node(block).unwrap(), None);
            assert_eq!(style.display, Some(gpui::Display::Flex));
            assert_eq!(style.flex_direction, Some(gpui::FlexDirection::Row));

            let mixed = t.create_element(div);
            let inline = t.create_element(span);
            let blocky = t.create_element(div);
            t.insert(inline, mixed, 0);
            t.insert(blocky, mixed, 1);
            let mut style2 = gpui::StyleRefinement::default();
            apply_inline_row(&mut style2, t, t.node(mixed).unwrap(), None);
            assert_eq!(style2.display, None);

            // `display: inline` on a child folds to `Block` in the mapper (taffy models the inner
            // type only), so without the `inline_level` check it read as an opt-out.
            let declared = t.create_element(div);
            for i in 0..2u32 {
                let s = t.create_element(span);
                t.insert(s, declared, i);
                let mut base = gpui::StyleRefinement::default();
                base.display = Some(gpui::Display::Block);
                let mut cs = sk_css::ComputedStates::default();
                cs.base = std::rc::Rc::new(base);
                cs.extras.inline_level = Some(true);
                if let Some(n) = t.node_mut(s) {
                    n.computed = Some(cs);
                }
            }
            let mut style3 = gpui::StyleRefinement::default();
            apply_inline_row(&mut style3, t, t.node(declared).unwrap(), None);
            assert_eq!(style3.display, Some(gpui::Display::Flex));
        });
    }

    #[test]
    fn html_focusable_by_default_and_disabled_controls() {
        with_tree(|t| {
            let root = t.create_root();
            let el = |t: &mut Tree, tag: &str, parent: u32, props: &[(&str, &str)]| {
                let tag = t.intern(tag);
                let id = t.create_element(tag);
                for (k, v) in props {
                    let key = t.intern(k);
                    t.set_prop(id, key, Some((*v).to_string()));
                }
                t.insert(id, parent, 0);
                id
            };
            let button = el(t, "button", root, &[]);
            let select = el(t, "select", root, &[]);
            let link = el(t, "a", root, &[("href", "#x")]);
            let anchor = el(t, "a", root, &[]);
            let editable = el(t, "div", root, &[("contenteditable", "true")]);
            let not_editable = el(t, "div", root, &[("contenteditable", "false")]);
            let div = el(t, "div", root, &[]);
            for (id, want, what) in [
                (button, Some(0), "button"),
                (select, Some(0), "select"),
                (link, Some(0), "a[href]"),
                (anchor, None, "a without href"),
                (editable, Some(0), "contenteditable"),
                (not_editable, None, "contenteditable=false"),
                (div, None, "div"),
            ] {
                assert_eq!(default_tab_index(t, id), want, "{what}");
                assert_eq!(could_take_focus(t, id), want.is_some(), "{what}");
            }

            let details = el(t, "details", root, &[]);
            let first = el(t, "summary", details, &[]);
            let second = el(t, "summary", details, &[]);
            assert_eq!(default_tab_index(t, first), Some(0));
            assert_eq!(default_tab_index(t, second), None);

            let off = el(t, "button", root, &[("disabled", ""), ("tabindex", "0")]);
            let on = el(t, "button", root, &[("disabled", "false")]);
            assert!(
                actually_disabled(t, off) && !could_take_focus(t, off),
                "disabled, tabindex or not"
            );
            assert!(!actually_disabled(t, on), "Vue's :disabled=\"false\"");
            let fieldset = el(t, "fieldset", root, &[("disabled", "")]);
            let inside = el(t, "button", fieldset, &[]);
            let legend = el(t, "legend", fieldset, &[]);
            let in_legend = el(t, "button", legend, &[]);
            assert!(actually_disabled(t, inside), "inside a disabled fieldset");
            assert!(!actually_disabled(t, in_legend), "inside its first legend");
            assert!(!actually_disabled(t, div), "a div is never disabled");
        });
    }

    #[test]
    fn a_listener_makes_nothing_focusable() {
        with_tree(|t| {
            let root = t.create_root();
            let el = |t: &mut Tree, tag: &str, props: &[(&str, &str)]| {
                let tag = t.intern(tag);
                let id = t.create_element(tag);
                for (k, v) in props {
                    let key = t.intern(k);
                    t.set_prop(id, key, Some((*v).to_string()));
                }
                t.insert(id, root, 0);
                id
            };
            let tag = el(t, "span", &[]);
            t.set_listener(tag, tree::EV_FOCUS, true);
            t.set_listener(tag, tree::EV_BLUR, true);
            let keys = el(t, "div", &[]);
            t.set_listener(keys, tree::EV_KEYDOWN, true);
            t.set_listener(keys, tree::EV_KEYUP, true);
            let role = el(t, "div", &[("role", "button")]);
            t.set_listener(role, tree::EV_KEYDOWN, true);
            let minus = el(t, "div", &[("tabindex", "-1")]);
            let junk = el(t, "div", &[("tabindex", "first")]);
            let label = el(t, "label", &[]);
            for (id, want, what) in [
                (tag, false, "a span with a focus listener"),
                (keys, false, "a div with key listeners"),
                (role, false, "role=button without tabindex"),
                (minus, true, "tabindex=-1"),
                (junk, false, "a tabindex that is not an integer"),
                (label, false, "a label"),
            ] {
                assert_eq!(could_take_focus(t, id), want, "{what}");
            }
        });
    }

    #[test]
    fn block_buttons_and_selects_centre_their_content_safely() {
        with_tree(|t| {
            let div = t.intern("div");
            let declared = |t: &mut Tree, tag: u32, display: Option<gpui::Display>| {
                let id = t.create_element(tag);
                let mut base = gpui::StyleRefinement::default();
                base.display = display;
                let mut cs = sk_css::ComputedStates::default();
                cs.base = std::rc::Rc::new(base);
                if let Some(n) = t.node_mut(id) {
                    n.computed = Some(cs);
                }
                id
            };
            let laid = |t: &Tree, id: u32, authored: Option<gpui::AlignContent>| {
                let mut s = gpui::StyleRefinement::default();
                s.align_content = authored;
                align_control_content(&mut s, t, t.node(id).unwrap());
                s.align_content
            };
            let safe = Some(gpui::AlignContent::SafeCenter);

            for tag in [t.intern("button"), t.intern("select")] {
                let bare = declared(t, tag, None);
                assert_eq!(laid(t, bare, None), safe);
                let block = declared(t, tag, Some(gpui::Display::Block));
                assert_eq!(laid(t, block, None), safe);
                for display in [gpui::Display::Flex, gpui::Display::Grid] {
                    let id = declared(t, tag, Some(display));
                    assert_eq!(laid(t, id, None), None);
                }
                let start = Some(gpui::AlignContent::Start);
                assert_eq!(
                    laid(t, bare, start),
                    start,
                    "the author's align-content wins"
                );
            }
            let plain = declared(t, div, None);
            assert_eq!(laid(t, plain, None), None);
        });
    }

    #[test]
    fn text_align_adds_auto_margins_in_a_block_parent_only() {
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");
            let extras = sk_css::Extras {
                inline_level: Some(true),
                ..Default::default()
            };
            let centred = |t: &Tree, child: u32| {
                let mut s = gpui::StyleRefinement::default();
                apply_inline_align(
                    &mut s,
                    t,
                    t.node(child).unwrap(),
                    &extras,
                    Some(gpui::TextAlign::Center),
                );
                (s.margin.left, s.margin.right)
            };
            let with_display = |t: &mut Tree, id: u32, display: gpui::Display| {
                let mut base = gpui::StyleRefinement::default();
                base.display = Some(display);
                let mut cs = sk_css::ComputedStates::default();
                cs.base = std::rc::Rc::new(base);
                if let Some(n) = t.node_mut(id) {
                    n.computed = Some(cs);
                }
            };
            let auto = (Some(gpui::Length::Auto), Some(gpui::Length::Auto));

            let block = t.create_element(div);
            let badge = t.create_element(div);
            let sibling = t.create_element(div);
            t.insert(badge, block, 0);
            t.insert(sibling, block, 1);
            assert_eq!(centred(t, badge), auto);

            for display in [gpui::Display::Flex, gpui::Display::Grid] {
                let parent = t.create_element(div);
                let item = t.create_element(div);
                t.insert(item, parent, 0);
                with_display(t, parent, display);
                assert_eq!(centred(t, item), (None, None), "{display:?}");
            }

            let row = t.create_element(div);
            let text = t.create_text();
            t.set_text(text, "Argon".to_string());
            let run = t.create_element(span);
            t.insert(text, row, 0);
            t.insert(run, row, 1);
            assert!(inline_row_applies(t, t.node(row).unwrap(), None));
            assert_eq!(centred(t, run), (None, None));
        });
    }

    #[test]
    fn the_click_count_rides_above_the_modifiers() {
        use gpui::px;
        let m = gpui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let p = mouse_payload_n(Point::new(px(1.0), px(2.0)), 0.0, &m, 2);
        assert_eq!(p.d as u32 & 0xf, 2, "shift");
        assert_eq!(p.d as u32 >> 4, 2, "detail");
        assert_eq!(
            mouse_payload_n(Point::new(px(0.0), px(0.0)), 0.0, &m, 9999).d as u32 >> 4,
            255
        );
    }

    #[test]
    fn a_wheel_delta_reaches_js_with_the_DOM_sign() {
        use gpui::px;
        let (_, up) = dom_wheel_delta(Point::new(px(0.0), px(120.0)));
        assert!(
            up < 0.0,
            "a scroll UP must reach JS as a negative deltaY, got {up}"
        );
        let (_, down) = dom_wheel_delta(Point::new(px(0.0), px(-120.0)));
        assert!(
            down > 0.0,
            "a scroll DOWN must reach JS as a positive deltaY, got {down}"
        );
        let (left, _) = dom_wheel_delta(Point::new(px(80.0), px(0.0)));
        assert!(left < 0.0);
        assert_eq!(dom_wheel_delta(Point::new(px(0.0), px(0.0))), (0.0, 0.0));
    }

    #[test]
    fn interior_white_space_collapses_the_way_css_says() {
        use sk_css::WhiteSpaceMode as WS;
        assert_eq!(processed_text("a\nb", WS::Normal), "a b");
        assert_eq!(processed_text("a  \t\n  b", WS::Normal), "a b");
        assert_eq!(processed_text("a\r\nb", WS::Normal), "a b");
        assert_eq!(processed_text("a\nb", WS::Nowrap), "a b");
        assert_eq!(processed_text("a   \n   b", WS::PreLine), "a\nb");
        assert_eq!(processed_text("a  b", WS::PreLine), "a b");
        for mode in [WS::Pre, WS::PreWrap, WS::BreakSpaces] {
            assert_eq!(processed_text("a   \n   b", mode), "a   \n   b");
        }
        assert!(matches!(
            processed_text("a b c", WS::Normal),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(
            processed_text("plain", WS::Normal),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(processed_text("  a  ", WS::Normal), " a ");
    }

    #[test]
    fn white_space_at_the_ends_of_a_line_is_removed() {
        with_tree(|t| {
            let p = t.intern("p");
            let span = t.intern("span");

            let block = t.create_element(p);
            let only = t.create_text();
            t.set_text(only, " Secure video meetings ".to_string());
            t.insert(only, block, 0);
            assert_eq!(
                trim_edges(
                    t,
                    t.node(only).unwrap(),
                    only,
                    &t.node(only).unwrap().text,
                    None
                ),
                "Secure video meetings"
            );

            let mixed = t.create_element(p);
            let head = t.create_text();
            t.set_text(head, " a ".to_string());
            t.insert(head, mixed, 0);
            let inline = t.create_element(span);
            t.insert(inline, mixed, 0);
            let tail = t.create_text();
            t.set_text(tail, " b ".to_string());
            t.insert(tail, mixed, 0);
            assert_eq!(
                trim_edges(
                    t,
                    t.node(head).unwrap(),
                    head,
                    &t.node(head).unwrap().text,
                    None
                ),
                "a "
            );
            assert_eq!(
                trim_edges(
                    t,
                    t.node(tail).unwrap(),
                    tail,
                    &t.node(tail).unwrap().text,
                    None
                ),
                " b"
            );

            // `a<span> </span>b`: the span's only child is a space a browser renders (mid-line), so
            // an inline-level parent with siblings suppresses the trim.
            let line = t.create_element(p);
            let before = t.create_text();
            t.set_text(before, "a".to_string());
            t.insert(before, line, 0);
            let spacer = t.create_element(span);
            t.insert(spacer, line, 0);
            let after = t.create_text();
            t.set_text(after, "b".to_string());
            t.insert(after, line, 0);
            let gap = t.create_text();
            t.set_text(gap, " ".to_string());
            t.insert(gap, spacer, 0);
            assert_eq!(
                trim_edges(
                    t,
                    t.node(gap).unwrap(),
                    gap,
                    &t.node(gap).unwrap().text,
                    None
                ),
                " "
            );
        });
    }

    #[test]
    fn an_elements_own_text_is_trimmed_at_the_ends_of_its_line() {
        with_tree(|t| {
            let p = t.intern("p");
            let span = t.intern("span");

            let block = t.create_element(p);
            t.set_element_text(block, " Secure video meetings ".to_string());
            assert_eq!(
                trim_own_edges(
                    t,
                    t.node(block).unwrap(),
                    &t.node(block).unwrap().text,
                    None
                ),
                "Secure video meetings"
            );

            let lone = t.create_element(span);
            t.set_element_text(lone, " x ".to_string());
            assert_eq!(
                trim_own_edges(t, t.node(lone).unwrap(), &t.node(lone).unwrap().text, None),
                "x"
            );

            let row = t.create_element(p);
            let word = t.create_text();
            t.set_text(word, "a".to_string());
            t.insert(word, row, 0);
            let inline = t.create_element(span);
            t.insert(inline, row, 0);
            t.set_element_text(inline, " b ".to_string());
            assert_eq!(
                trim_own_edges(
                    t,
                    t.node(inline).unwrap(),
                    &t.node(inline).unwrap().text,
                    None
                ),
                " b "
            );
        });
    }

    fn spoke(key: &str) -> bool {
        WARNED.with(|w| w.borrow().contains(key))
    }

    fn computed(base: gpui::StyleRefinement, extras: sk_css::Extras) -> sk_css::ComputedStates {
        sk_css::ComputedStates {
            base: std::rc::Rc::new(base),
            extras,
            ..Default::default()
        }
    }

    fn displayed(d: gpui::Display) -> gpui::StyleRefinement {
        gpui::StyleRefinement {
            display: Some(d),
            ..Default::default()
        }
    }

    fn inline_level() -> sk_css::Extras {
        sk_css::Extras {
            inline_level: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn advise_inline_level_is_silent_wherever_css_blockifies_too() {
        const KEY: &str = "display-inline-level-stretches";
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");
            let auto = gpui::StyleRefinement::default();

            let flex = t.create_element(div);
            t.node_mut(flex).unwrap().computed =
                Some(computed(displayed(gpui::Display::Flex), Default::default()));
            let item = t.create_element(span);
            t.insert(item, flex, 0);
            advise_inline_level(t, item, t.node(item).unwrap(), &inline_level(), &auto);
            assert!(!spoke(KEY), "a flex item is blockified by CSS");

            let grid = t.create_element(div);
            t.node_mut(grid).unwrap().computed =
                Some(computed(displayed(gpui::Display::Grid), Default::default()));
            let cell = t.create_element(span);
            t.insert(cell, grid, 0);
            advise_inline_level(t, cell, t.node(cell).unwrap(), &inline_level(), &auto);
            assert!(!spoke(KEY), "a grid item is blockified by CSS");

            let block = t.create_element(div);
            t.node_mut(block).unwrap().computed = Some(computed(
                displayed(gpui::Display::Block),
                Default::default(),
            ));
            let badge = t.create_element(span);
            t.insert(badge, block, 0);
            let sibling = t.create_element(div);
            t.insert(sibling, block, 1);
            t.node_mut(sibling).unwrap().computed = Some(computed(
                displayed(gpui::Display::Block),
                Default::default(),
            ));

            let absolute = gpui::StyleRefinement {
                position: Some(gpui::Position::Absolute),
                ..Default::default()
            };
            advise_inline_level(t, badge, t.node(badge).unwrap(), &inline_level(), &absolute);
            assert!(!spoke(KEY), "an out-of-flow box is blockified by CSS");

            let mut sized = gpui::StyleRefinement::default();
            sized.size.width = Some(gpui::px(120.0).into());
            advise_inline_level(t, badge, t.node(badge).unwrap(), &inline_level(), &sized);
            assert!(!spoke(KEY), "a definite width answers it");

            let row = t.create_element(div);
            let a = t.create_element(span);
            let b = t.create_element(span);
            t.insert(a, row, 0);
            t.insert(b, row, 1);
            for kid in [a, b] {
                t.node_mut(kid).unwrap().computed =
                    Some(computed(Default::default(), inline_level()));
            }
            advise_inline_level(t, a, t.node(a).unwrap(), &inline_level(), &auto);
            assert!(
                !spoke(KEY),
                "the parent becomes an inline row and sizes it to content"
            );

            advise_inline_level(t, badge, t.node(badge).unwrap(), &inline_level(), &auto);
            assert!(
                spoke(KEY),
                "an in-flow inline box beside a block sibling really stretches"
            );
        });
    }

    #[test]
    fn advise_out_of_flow_speaks_only_for_a_fixed_box_in_a_smaller_parent() {
        const KEY: &str = "position-fixed-not-portalled";
        let fixed = || sk_css::Extras {
            fixed: Some(true),
            ..Default::default()
        };
        with_tree(|t| {
            let div = t.intern("div");
            let root = t.create_root();
            // The previous frame's rectangles: a build cannot see this frame's layout, which
            // `painted_bounds` reads.
            let win = windows::primary();
            windows::of(win, |w| {
                w.bounds.insert(root, (0.0, 0.0, 1200.0, 800.0));
            });

            let overlay = t.create_element(div);
            t.insert(overlay, root, 0);
            for _ in 0..4 {
                advise_out_of_flow(t, overlay, t.node(overlay).unwrap(), &fixed(), win);
            }
            assert!(
                !spoke(KEY),
                "a Teleport-to-body overlay lands on the viewport"
            );

            let wrapper = t.create_element(div);
            t.insert(wrapper, root, 1);
            windows::of(win, |w| {
                w.bounds.insert(wrapper, (0.0, 0.0, 1200.0, 800.0));
            });
            let full_bleed = t.create_element(div);
            t.insert(full_bleed, wrapper, 0);
            for _ in 0..4 {
                advise_out_of_flow(t, full_bleed, t.node(full_bleed).unwrap(), &fixed(), win);
            }
            assert!(
                !spoke(KEY),
                "a full-bleed wrapper of viewport size diverges by nothing"
            );

            // A zero-height wrapper, and not on the first two builds (the boot guard).
            let squashed = t.create_element(div);
            t.insert(squashed, root, 2);
            windows::of(win, |w| {
                w.bounds.insert(squashed, (0.0, 0.0, 1200.0, 0.0));
            });
            let child = t.create_element(div);
            t.insert(child, squashed, 0);
            advise_out_of_flow(t, child, t.node(child).unwrap(), &fixed(), win);
            advise_out_of_flow(t, child, t.node(child).unwrap(), &fixed(), win);
            assert!(!spoke(KEY), "two builds is not settled");
            advise_out_of_flow(t, child, t.node(child).unwrap(), &fixed(), win);
            assert!(spoke(KEY), "three consecutive builds is");
        });
    }

    #[test]
    fn a_fixed_box_in_a_popout_resolves_against_that_windows_viewport() {
        const KEY: &str = "position-fixed-not-portalled";
        let fixed = || sk_css::Extras {
            fixed: Some(true),
            ..Default::default()
        };
        let popout: windows::WinKey = 4242;
        let (panel, toolbar, inner, pinned) = with_tree(|t| {
            let div = t.intern("div");
            let root = t.create_root();
            let panel = t.create_element(div);
            t.insert(panel, root, 0);
            let toolbar = t.create_element(div);
            t.insert(toolbar, panel, 0);
            let inner = t.create_element(div);
            t.insert(inner, panel, 1);
            let pinned = t.create_element(div);
            t.insert(pinned, inner, 0);
            windows::of(windows::primary(), |w| {
                w.bounds.insert(root, (0.0, 0.0, 1200.0, 800.0));
            });
            (panel, toolbar, inner, pinned)
        });
        windows::set_painted_root(popout, panel);
        windows::of(popout, |w| {
            w.bounds.insert(panel, (0.0, 0.0, 360.0, 280.0));
            w.bounds.insert(inner, (0.0, 0.0, 360.0, 40.0));
        });
        with_tree(|t| {
            for _ in 0..4 {
                advise_out_of_flow(t, toolbar, t.node(toolbar).unwrap(), &fixed(), popout);
            }
            assert!(!spoke(KEY), "the pop-out's painted root IS its viewport");

            for _ in 0..3 {
                advise_out_of_flow(t, pinned, t.node(pinned).unwrap(), &fixed(), popout);
            }
            assert!(
                spoke(KEY),
                "a 40px-tall wrapper in a 280px window is not the viewport"
            );
        });
        windows::forget(popout);
    }

    #[test]
    fn synthetic_inline_rows_allow_text_to_shrink_but_keep_authored_flex_minimums() {
        with_tree(|tree| {
            let div = tree.intern("div");
            let span = tree.intern("span");
            let parent = tree.create_element(div);
            let text = tree.create_text();
            tree.set_text(text, "unbrokenword".repeat(60));
            tree.insert(text, parent, 0);
            let timestamp = tree.create_element(span);
            tree.set_element_text(timestamp, "11:55".into());
            tree.insert(timestamp, parent, 0);
            assert!(synthetic_inline_item(tree, tree.node(text).expect("text")));
            let mut style = gpui::StyleRefinement::default();
            apply_inline_item_minimum(&mut style, tree, tree.node(timestamp).expect("timestamp"));
            assert_eq!(style.min_size.width, Some(gpui::px(0.0).into()));
            style.min_size.width = Some(gpui::px(12.0).into());
            apply_inline_item_minimum(&mut style, tree, tree.node(timestamp).expect("timestamp"));
            assert_eq!(
                style.min_size.width,
                Some(gpui::px(12.0).into()),
                "an authored minimum remains authoritative"
            );
            let parent_node = tree.node_mut(parent).expect("parent");
            let mut computed = sk_css::ComputedStates::default();
            computed.base = std::rc::Rc::new(gpui::StyleRefinement {
                display: Some(gpui::Display::Flex),
                ..Default::default()
            });
            parent_node.computed = Some(computed);
            assert!(
                !synthetic_inline_item(tree, tree.node(text).expect("text")),
                "a real flex container keeps CSS min-content behavior"
            );
        });
    }
}
