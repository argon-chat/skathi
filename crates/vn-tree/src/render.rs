//! Lowers the retained tree into gpui elements. Runs inside the hosting
//! view's `render()` each frame — gpui's element tree is rebuilt per frame by
//! design, so we simply re-emit the whole thing; per-component memoization
//! (`Entity::cached`) is a later optimization.
//!
//! Input events: gpui listeners are attached ONLY for events the JS side
//! registered (the listener bitmask — empty handlers cost a hitbox each).
//! Each listener publishes `Phase::Commit`, dispatches one compact payload
//! into JS (the runtime synthesizes the W3C event and runs capture/bubble
//! propagation over the mirror tree), then flushes tree changes so Vue's
//! reactive wave repaints in the same frame.
//!
//! Three things need state that outlives the per-frame rebuild, kept in side
//! maps keyed by node id and GC'd in [`gc_node_state`]:
//! * focus handles — a fresh one each frame would blur the node every frame,
//!   and `tabindex` is stamped onto the handle (gpui reads the tab order from
//!   there at paint time);
//! * `gpui::ScrollHandle`s — `overflow: scroll` in the computed style is what
//!   makes gpui lay out and clip a scroll box, but a handle we own is what lets
//!   us read the offset back for `e.target.scrollTop`;
//! * `Entity<TextInputState>` per `<input>`.
//!
//! `focus`/`blur` are not synthesized from mouse-down: they come from gpui
//! focus observers, so `Tab` and programmatic focus report identically.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, ClickEvent, Context, Empty, FocusHandle, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseUpEvent, Overflow, Point, ScrollHandle, ScrollWheelEvent,
    SharedString, Subscription, Window, WindowControlArea,
};
use vn_css::{WindowControl, WindowDrag};

use crate::tree::{self, Kind, Tree};
use crate::EventPayload;
use crate::img as img_src;
use vn_elements::{text_input, TextInputHandlers, TextInputOptions, TextInputState};
use vn_runtime::{with_published, Phase};

thread_local! {
    /// Persistent per-node focus handles (minting a new handle each render
    /// would blur the node every frame). GC'd against tree liveness.
    static FOCUS_HANDLES: RefCell<HashMap<u32, FocusHandle>> = RefCell::new(HashMap::new());
    /// The node that owns keyboard focus (0 = none) — drives focus/blur events.
    static FOCUSED_NODE: Cell<u32> = const { Cell::new(0) };
    /// gpui `on_focus` subscriptions, one per focusable node. Focus can move by
    /// mouse, by `Tab`, or programmatically; observing the handle catches all
    /// three with one code path.
    static FOCUS_SUBS: RefCell<HashMap<u32, Subscription>> = RefCell::new(HashMap::new());
    /// The single "nothing in the window is focused any more" observer.
    static FOCUS_LOST_SUB: RefCell<Option<Subscription>> = const { RefCell::new(None) };
    /// Persistent editor state per `<input>` node (content, selection, IME,
    /// undo, echo ring live across the per-frame element rebuild).
    static INPUT_STATES: RefCell<HashMap<u32, gpui::Entity<TextInputState>>> =
        RefCell::new(HashMap::new());
    /// Persistent scroll offset per scroll container. gpui keys anonymous
    /// scroll state by `ElementId` too, but a handle we own is what lets us
    /// *read* the offset back out for `e.target.scrollTop`.
    static SCROLL_HANDLES: RefCell<HashMap<u32, ScrollHandle>> = RefCell::new(HashMap::new());
    /// Last offset reported to JS per node, in CSS pixels — a `scroll` event
    /// only fires when the position actually moved.
    static SCROLL_REPORTED: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
    /// One-shot compat advisories already emitted (keyed by message).
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Absolute (window-space) layout bounds of every node that was actually
    /// PAINTED in the last frame gpui drew — see [`record_bounds`].
    static BOUNDS: RefCell<HashMap<u32, (f32, f32, f32, f32)>> = RefCell::new(HashMap::new());
    /// The previous frame's [`BOUNDS`], kept because the element tree for frame
    /// N is BUILT before frame N's prepaint runs — so the only rectangle a
    /// builder can read is last frame's. `<lottie>` needs it to rasterise into
    /// a box whose size CSS expresses relatively (`width: 100%`, flex).
    static PREV_BOUNDS: RefCell<HashMap<u32, (f32, f32, f32, f32)>> = RefCell::new(HashMap::new());
    /// The focused node that was detached from the painted tree and must lose
    /// gpui focus (0 = nothing pending). Set from `gc_node_state` (which has no
    /// `Window`), applied by [`apply_pending_blur`] on the next frame that does.
    static PENDING_BLUR: Cell<u32> = const { Cell::new(0) };
    /// Nodes lowered in the build currently running — the VN_PERF counter. One
    /// `Cell` increment per node, unconditionally: the branch a probe check
    /// would add costs as much as the add.
    static BUILT: Cell<u64> = const { Cell::new(0) };
}

/// Browser-compat advisory: say what was dropped and what to do instead, once.
pub(crate) fn warn_compat(key: &str, msg: &str) {
    WARNED.with(|w| {
        if w.borrow_mut().insert(key.to_string()) {
            eprintln!("[vue-native compat] {msg}");
            crate::log::record("advisory", msg);
        }
    });
}

// ---------------------------------------------------------------------------
// the bounds side map
// ---------------------------------------------------------------------------
//
// COMPAT ROADMAP G3 (research/reports/m10/shadcn.md): this map is deliberately
// the backing store a future `getBoundingClientRect()` will read. A browser
// answers that question off the last completed layout; gpui has no retained
// layout tree to ask after the fact, so the only place the number exists is the
// bounds gpui hands each element during prepaint. `BoundsProbe` below captures
// it there for every mirrored node — one entry per node per drawn frame, no
// extra taffy node, no extra hitbox.
//
// A node that was not painted (detached, or living in a <KeepAlive> cache
// container that is never inserted) simply never records, so it reads back as
// `None` — which is also what a browser reports for a detached element
// (an all-zero rect). Devtools surfaces that as `bounds: null` rather than
// inventing zeros.

/// Called from `build_root`, i.e. once per `render()`: last frame's rectangles
/// are about to be replaced by this frame's, and anything that stops being
/// painted must stop having bounds.
fn reset_bounds() {
    let last = BOUNDS.with(|m| std::mem::take(&mut *m.borrow_mut()));
    PREV_BOUNDS.with(|m| *m.borrow_mut() = last);
}

fn record_bounds(id: u32, bounds: gpui::Bounds<gpui::Pixels>) {
    BOUNDS.with(|m| {
        m.borrow_mut().insert(
            id,
            (
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            ),
        );
    });
}

/// `(x, y, width, height)` in CSS pixels, window-relative, from the last frame
/// this node was painted in — `None` when it was not painted at all.
pub fn bounds_of(id: u32) -> Option<(f32, f32, f32, f32)> {
    BOUNDS.with(|m| m.borrow().get(&id).copied())
}

/// The rectangle this node was painted at in the frame BEFORE the one being
/// built. Only builders need this — `bounds_of` is the answer to "where is it
/// now", which is what devtools and a future `getBoundingClientRect()` want.
fn prev_size_of(id: u32) -> Option<(f32, f32)> {
    PREV_BOUNDS.with(|m| {
        m.borrow()
            .get(&id)
            .copied()
            .filter(|(_, _, w, h)| *w > 0.0 && *h > 0.0)
            .map(|(_, _, w, h)| (w, h))
    })
}

/// A transparent wrapper that records the bounds gpui laid its inner element
/// out at. It contributes NO layout of its own: `request_layout` returns the
/// inner element's own `LayoutId`, so taffy sees exactly the tree it saw
/// before and `bounds` here is the inner element's own rectangle.
struct BoundsProbe {
    id: u32,
    /// The root node's probe brackets the WHOLE recursion in each of gpui's
    /// three element phases, which makes it the one place that can time them
    /// (see [`crate::perf::Phases`]). False everywhere else — the timing hooks
    /// are behind this flag AND behind `VN_PERF`.
    root: bool,
    inner: AnyElement,
}

impl gpui::IntoElement for BoundsProbe {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for BoundsProbe {
    type RequestLayoutState = ();
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
    ) -> (gpui::LayoutId, ()) {
        if !self.root {
            return (self.inner.request_layout(window, cx), ());
        }
        let span = crate::perf::Span::start();
        let layout = self.inner.request_layout(window, cx);
        crate::perf::note_layout_req(span.ms());
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        record_bounds(self.id, bounds);
        if !self.root {
            self.inner.prepaint(window, cx);
            return;
        }
        crate::perf::note_taffy_gap();
        let span = crate::perf::Span::start();
        self.inner.prepaint(window, cx);
        crate::perf::note_prepaint(span.ms());
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.root {
            self.inner.paint(window, cx);
            return;
        }
        let span = crate::perf::Span::start();
        self.inner.paint(window, cx);
        crate::perf::note_paint(span.ms());
    }
}

/// Wraps a built element so its rectangle lands in the side map. Text and
/// anchor nodes are skipped — they have no box a selector can ask about.
fn probe(id: u32, inner: AnyElement) -> AnyElement {
    probe_at(id, false, inner)
}

fn probe_at(id: u32, root: bool, inner: AnyElement) -> AnyElement {
    BoundsProbe { id, root, inner }.into_any_element()
}

/// Drops per-node state for nodes that no longer exist. Called from
/// `flush_changes` after mutations.
pub(crate) fn gc_node_state() {
    let live = |id: u32| tree::read_tree(|t| t.node(id).is_some());
    FOCUS_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    FOCUS_SUBS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    INPUT_STATES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_REPORTED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    BOUNDS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    PREV_BOUNDS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    img_src::gc(live);
    crate::lottie::gc(live);
    crate::svg::gc(live);
    let focused = FOCUSED_NODE.with(|f| f.get());
    if focused == 0 {
        return;
    }
    if !live(focused) {
        FOCUSED_NODE.with(|f| f.set(0));
        return;
    }
    // The browser rule: **detaching the focused element blurs it.** A node can
    // stay allocated and yet stop being part of the document — `<KeepAlive>`
    // moves a deactivated page into a container that is never inserted, and
    // `build_root` walks from `t.root` only, so that subtree is not painted.
    // Liveness (above) is not enough: gpui would keep routing keystrokes to a
    // handle whose element is nowhere in the tree, so a deactivated page's
    // `<input>` kept its focus ring AND kept receiving `keydown` while the
    // characters went nowhere (research/reports/m10/keepalive-state.md §4.1).
    if !tree::read_tree(|t| reaches_root(t, focused)) {
        PENDING_BLUR.with(|p| p.set(focused));
    }
}

/// Does `id` still hang off the painted root? Walks `Node::parent`, which is a
/// handful of hops, and only ever runs when something is focused and the tree
/// just changed.
fn reaches_root(t: &Tree, id: u32) -> bool {
    if t.root == 0 {
        return false;
    }
    let mut cursor = id;
    // The tree is a tree, but a corrupt parent chain must not hang the pump.
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

/// Releases focus for a node that was detached while focused, and tells JS.
/// Runs from the frame pump, which is the first place after the mutation that
/// holds a `Window`.
///
/// The `blur` event is dispatched here rather than left to gpui's focus-lost
/// observer, because that observer compares the *drawn* focus paths — and a
/// node that is no longer painted may already have dropped out of the path, so
/// the `Some → None` edge it watches for is not guaranteed to happen. Clearing
/// `FOCUSED_NODE` first keeps the two paths from both firing: `focus_transition`
/// returns early when the focus is already where it is being moved to.
/// Keeps SOMETHING focused, so keystrokes have a dispatch path to travel.
///
/// A browser's document always has a focus target: with nothing else focused,
/// `document.body` has it, and that is why a `keydown` listener on `document`
/// fires for every keystroke. gpui has no such default — with nothing focused,
/// a key event's dispatch path is the window's root node alone and NO element
/// listener runs. So the app root (which is vue-native's `document`) takes focus
/// whenever nothing else has it.
///
/// Anything the app focuses for real — an `<input>`, a `tabindex`ed node — takes
/// over immediately and stays, because the root is its ANCESTOR in the dispatch
/// tree and so keeps receiving the keystrokes anyway.
pub(crate) fn ensure_root_focus(window: &mut Window, cx: &mut App) {
    if window.focused(cx).is_some() {
        return;
    }
    let root = tree::read_tree(|t| t.root);
    if root == 0 {
        return;
    }
    let Some(handle) = FOCUS_HANDLES.with(|m| m.borrow().get(&root).cloned()) else {
        return;
    };
    window.focus(&handle, cx);
}

/// Dispatches the `transitionend` / `animation*` events vn-css's animator
/// queued, once per frame.
///
/// It has to happen here rather than inside the animator because dispatching
/// into JS needs a `Window` (the runtime's capture/bubble walk can run app code
/// that mutates the tree, which then has to repaint), and the animator runs from
/// the restyle pass and from the paint clock, neither of which has one.
///
/// Returns `true` when a handler changed something and the window must redraw.
pub(crate) fn dispatch_anim_events(window: &mut Window, cx: &mut App) -> bool {
    let events = tree::take_anim_events();
    if events.is_empty() {
        return false;
    }
    let mut moved = false;
    for event in events {
        let code = match event.kind {
            vn_css::AnimEventKind::TransitionEnd => tree::EV_TRANSITIONEND,
            vn_css::AnimEventKind::AnimationStart => tree::EV_ANIMATIONSTART,
            vn_css::AnimEventKind::AnimationEnd => tree::EV_ANIMATIONEND,
            vn_css::AnimEventKind::AnimationCancel => tree::EV_ANIMATIONCANCEL,
        };
        // No listener anywhere on the path means nothing to deliver to. The
        // bit is the node's own; the runtime's walk handles ancestors, so this
        // is only a cheap "nobody on this element" filter — the full check is
        // the listener mask the JS side maintains for the whole path.
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

pub(crate) fn apply_pending_blur(window: &mut Window, cx: &mut App) {
    let pending = PENDING_BLUR.with(|p| p.replace(0));
    // Re-check: focus may legitimately have moved on since the mutation (the
    // same wave that detached the page can focus something on the new one), and
    // a page can be re-attached before this frame runs.
    if pending == 0 || FOCUSED_NODE.with(|f| f.get()) != pending {
        return;
    }
    if tree::read_tree(|t| reaches_root(t, pending)) {
        return;
    }
    // gpui first — this is what stops keystrokes reaching the detached node.
    window.blur();
    FOCUSED_NODE.with(|f| f.set(0));
    js_dispatch_app(pending, tree::EV_BLUR, &EventPayload::default(), window, cx);
    // `:focus` rungs read the handle live, so the repaint cannot wait for a
    // JS handler to have changed something.
    window.refresh();
}

fn mods_bits(m: &gpui::Modifiers) -> f64 {
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

/// `MouseEvent.button` for an event that carries no press: the DOM says `-1`
/// for `mousemove`/`mouseenter` when no button is down.
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

/// gpui keystroke names → DOM `KeyboardEvent.key` values.
fn dom_key(k: &str) -> String {
    match k {
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "tab" => "Tab".into(),
        "space" => " ".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "insert" => "Insert".into(),
        other => other.into(),
    }
}

/// The one `mousemove` a frame is allowed to deliver (browser event
/// coalescing: UAs align `mousemove`/`pointermove` to the frame clock and drop
/// the rest). Written by the per-node `on_mouse_move` handlers, drained by
/// [`flush_pending_mouse_move`] from the frame pump.
///
/// Doubles as the per-move dedup: gpui runs bubble-phase move handlers
/// topmost-first, so for one physical move every listening ANCESTOR fires
/// after the innermost node. The first claim (= innermost, the browser's
/// target) wins; the ancestors see their own position already stored and drop
/// out — without this, one move dispatched k full capture/bubble walks for k
/// nested listeners, and each JS mutation forced a full-tree rebuild. That is
/// the exact shape that took shadcn to 10 fps under the cursor.
struct PendingMouseMove {
    id: u32,
    position: Point<gpui::Pixels>,
    button: f64,
    modifiers: gpui::Modifiers,
}

thread_local! {
    static PENDING_MOUSE_MOVE: std::cell::RefCell<Option<PendingMouseMove>> =
        const { std::cell::RefCell::new(None) };
}

/// Deliver the frame's coalesced `mousemove` (if any). Called by the host's
/// pump BEFORE the JS pump, so everything the handler schedules (Vue's queue,
/// microtasks) drains in the same frame.
pub fn flush_pending_mouse_move(window: &mut Window, cx: &mut App) {
    let Some(p) = PENDING_MOUSE_MOVE.with(|c| c.borrow_mut().take()) else { return };
    let payload = mouse_payload(p.position, p.button, &p.modifiers);
    js_dispatch_app(p.id, tree::EV_MOUSEMOVE, &payload, window, cx);
}

fn mouse_payload(position: Point<gpui::Pixels>, button: f64, m: &gpui::Modifiers) -> EventPayload<'static> {
    EventPayload {
        a: f64::from(position.x),
        b: f64::from(position.y),
        c: button,
        d: mods_bits(m),
        key: None,
    }
}

/// Publish → dispatch → flush; the shape every listener shares. Returns
/// `false` when a JS handler called `preventDefault()`.
pub(crate) fn js_dispatch_app(id: u32, event: u32, payload: &EventPayload, window: &mut Window, cx: &mut App) -> bool {
    let proceed = with_published(window, cx, Phase::Commit, || {
        crate::dispatch_event(id, event, payload)
    });
    if crate::flush_changes() {
        window.refresh();
    }
    proceed
}

/// The `Context<V>` flavour: `cx.notify()` is the cheaper repaint request when
/// we already hold the hosting view.
fn js_dispatch<V: 'static>(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut Context<V>,
) -> bool {
    let proceed = with_published(window, &mut **cx, Phase::Commit, || {
        crate::dispatch_event(id, event, payload)
    });
    if crate::flush_changes() {
        cx.notify();
    }
    proceed
}

/// DOM order for a focus move: `blur` on the node that had it, then `focus` on
/// the node that got it. Neither event bubbles (the JS runtime knows).
fn focus_transition<V: 'static>(new_focus: u32, window: &mut Window, cx: &mut Context<V>) {
    let old = FOCUSED_NODE.with(|f| f.get());
    if old == new_focus {
        return;
    }
    FOCUSED_NODE.with(|f| f.set(new_focus));
    if old != 0 {
        js_dispatch(old, tree::EV_BLUR, &EventPayload::default(), window, cx);
    }
    if new_focus != 0 {
        js_dispatch(new_focus, tree::EV_FOCUS, &EventPayload::default(), window, cx);
    }
    // A focus move can change what `:focus` / `:focus-visible` paint even when
    // no JS handler ran (an `<input>` folds its focus rungs at build time), so
    // the repaint cannot be left to `flush_changes`.
    cx.notify();
}

/// Subscribes `id` to its handle's focus changes, once. gpui fires these from
/// the draw that first shows the new focus state, i.e. outside any tree borrow.
fn watch_focus<V: 'static>(id: u32, handle: &FocusHandle, window: &mut Window, cx: &mut Context<V>) {
    if FOCUS_SUBS.with(|m| m.borrow().contains_key(&id)) {
        return;
    }
    let sub = cx.on_focus(handle, window, move |_, window, cx| {
        focus_transition(id, window, cx);
    });
    FOCUS_SUBS.with(|m| m.borrow_mut().insert(id, sub));
    // Focus leaving the window's tree entirely (the focused node was removed,
    // or something unfocusable was clicked) has to blur the last node.
    if FOCUS_LOST_SUB.with(|s| s.borrow().is_some()) {
        return;
    }
    let sub = cx.on_focus_lost(window, |_, window, cx| {
        focus_transition(0, window, cx);
    });
    FOCUS_LOST_SUB.with(|s| *s.borrow_mut() = Some(sub));
}

/// The DOM `tabindex` attribute: `>= 0` is a tab stop at that index, `< 0` is
/// focusable but skipped by `Tab`, absent is not focusable at all. gpui reads
/// both values off the focus handle, so they are stamped onto the persistent
/// one (same `FocusId` — focus is never disturbed by this).
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

fn focus_handle_for<V: 'static>(id: u32, tab_index: Option<isize>, cx: &mut Context<V>) -> FocusHandle {
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

// ---------------------------------------------------------------------------
// window-drag / --window-control  (docs/WINDOWING.md §2.1, §2.2, §5.1)
// ---------------------------------------------------------------------------

thread_local! {
    /// Whether the frame currently being built painted ANY `window-drag: move`
    /// region. The host reads it to retire its fallback titlebar the first
    /// time the app declares a drag region of its own (§4.2 mode 2) — never
    /// silently removing a behaviour an app with no config still relies on.
    static ANY_DRAG_REGION: Cell<bool> = const { Cell::new(false) };
    /// Every control hitbox this frame emitted, `(node id, area)`.
    ///
    /// Drag regions are the one part of this surface that a synthesised
    /// `ctl.ts click` can never exercise: `WM_NCHITTEST` is a real OS message
    /// and the content dispatch path never reaches it. This is the
    /// eval-reachable debug hook the acceptance transcript needs instead
    /// (docs/WINDOWING.md §13 "Acceptance").
    static CONTROL_HITBOXES: RefCell<Vec<(u32, WindowControlArea)>> = const { RefCell::new(Vec::new()) };
}

/// `true` if the last painted frame contained a `window-drag: move` region.
pub fn painted_drag_region() -> bool {
    ANY_DRAG_REGION.with(|c| c.get())
}

/// The control hitboxes the last painted frame emitted, in paint order.
pub fn control_hitboxes() -> Vec<(u32, WindowControlArea)> {
    CONTROL_HITBOXES.with(|v| v.borrow().clone())
}

/// Folds the one INHERITED property vn-css carries and decides whether this
/// node needs a gpui window-control hitbox.
///
/// Returns `(effective window-drag for this node and its subtree, the hitbox to
/// emit)`.
///
/// A hitbox is emitted only where the effective value DIFFERS from the parent's
/// (D-15). Under the topmost-first resolution this fork's gpui now uses
/// (fork item F-01), the hit test walks outward until it finds a hitbox, and
/// the nearest ancestor that *changed* the value is exactly the one carrying
/// the right answer — so a 30-node titlebar produces two or three hitboxes
/// instead of thirty.
fn window_control_of(
    extras: &vn_css::Extras,
    parent: Option<WindowDrag>,
) -> (Option<WindowDrag>, Option<WindowControlArea>) {
    let effective = extras.window_drag.or(parent);
    // A caption button wins outright: it is implicitly `window-drag: none` for
    // hit-test purposes, so declaring both is redundant but harmless.
    let area = match extras.window_control {
        Some(WindowControl::Minimize) => Some(WindowControlArea::Min),
        Some(WindowControl::Maximize) => Some(WindowControlArea::Max),
        Some(WindowControl::Close) => Some(WindowControlArea::Close),
        // `--window-control: none` cancels a role an earlier rule set; it does
        // NOT by itself carve the element out of an ancestor's drag region —
        // that is what `window-drag: none` is for.
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

/// The compat-first law running in the UNUSUAL direction: we warn because we
/// are *more* permissive than the platform we are compatible with.
///
/// In a browser (and in Electron) a drag region swallows every pointer event,
/// so an interactive element inside one emits no clicks and no hover unless it
/// declares `window-drag: none`. gpui instead FORWARDS non-client mouse events
/// into the content tree (`handle_nc_mouse_down_msg` and friends), so the same
/// button keeps working here — and silently breaks the day the component is
/// dropped into Electron or a PWA. Dev builds say so once (A-13).
fn advise_interactive_in_drag_region(
    t: &Tree,
    node: &crate::tree::Node,
    extras: &vn_css::Extras,
    effective: Option<WindowDrag>,
) {
    if effective != Some(WindowDrag::Move) {
        return;
    }
    // Declaring either opt-out is the fix; a caption button is opted out by
    // definition (§2.2).
    if extras.window_drag == Some(WindowDrag::None) || extras.window_control.is_some() {
        return;
    }
    const POINTER: u32 = (1 << tree::EV_CLICK) | (1 << tree::EV_MOUSEDOWN) | (1 << tree::EV_MOUSEUP);
    if node.listeners & POINTER == 0 {
        return;
    }
    let tag = t.name(node.tag);
    warn_compat(
        "window-drag/interactive",
        &format!(
            "<{tag}> has a pointer listener inside a `window-drag: move` region without \
             `window-drag: none`. In a browser and in Electron a drag region swallows ALL \
             pointer events, so this element would be dead there; vue-native still delivers \
             the click. Add `window-drag: none` (or `--window-control: …` for a caption \
             button) — see docs/WINDOWING.md §2.1."
        ),
    );
}

/// The double-action hazard (A-14).
///
/// gpui performs the min/max/close action itself on the `WM_NCLBUTTONUP` pair
/// AND forwards a synthesised MouseDown/MouseUp into the tree, which the
/// renderer turns into a `click`. So an element carrying both
/// `--window-control: maximize` and `@click="appWindow.toggleMaximize()"`
/// toggles TWICE. Pick one — say which, rather than silently choosing a winner.
fn advise_double_action(t: &Tree, node: &crate::tree::Node, extras: &vn_css::Extras) {
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
             11 snap layouts) — see docs/WINDOWING.md §2.2."
        ),
    );
}

/// Records the emitted hitbox (for [`control_hitboxes`] / the fallback-titlebar
/// signal) and attaches it to the element.
fn with_control_area<E: gpui::InteractiveElement>(
    e: E,
    id: u32,
    area: Option<WindowControlArea>,
) -> E {
    let Some(area) = area else { return e };
    if area == WindowControlArea::Drag {
        ANY_DRAG_REGION.with(|c| c.set(true));
    }
    CONTROL_HITBOXES.with(|v| v.borrow_mut().push((id, area)));
    e.window_control_area(area)
}

/// Builds the current tree root (or nothing) for the hosting view `V`.
pub fn build_root<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    // ONE clock read per frame: nodes animating together must sample the same
    // instant or they skew.
    let now = std::time::Instant::now();
    let span = crate::perf::Span::start();
    BUILT.with(|c| c.set(0));
    // This frame's rectangles replace last frame's wholesale — see the bounds
    // side map above. `render()` is the only entry point, so this is the only
    // place that can know a new element tree is being built.
    reset_bounds();
    ANY_DRAG_REGION.with(|c| c.set(false));
    CONTROL_HITBOXES.with(|v| v.borrow_mut().clear());
    let el = tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            // The root inherits nothing: `window-drag`'s initial value is
            // `none`, spelled here as "nobody declared anything".
            build_node(t, t.root, window, cx, now, None)
        }
    });
    crate::perf::note_build(span.ms(), BUILT.with(|c| c.get()));
    el
}

fn build_node<V: 'static>(
    t: &Tree,
    id: u32,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    parent_drag: Option<WindowDrag>,
) -> AnyElement {
    let Some(node) = t.node(id) else {
        return Empty.into_any_element();
    };
    BUILT.with(|c| c.set(c.get() + 1));
    // `window-drag` is inherited (the CSSWG resolved that explicitly) and
    // vn-css has no inherited properties at all, so the fold happens here, on
    // the way down. A browser allows the property on a replaced element too,
    // hence `<img>` and `<lottie>` get the same treatment.
    let extras = extras_of(node);
    let (drag, control_area) = window_control_of(&extras, parent_drag);
    advise_interactive_in_drag_region(t, node, &extras, drag);
    advise_double_action(t, node, &extras);
    advise_untransformable(t, node, &extras);
    match node.kind {
        // An empty text node takes no space in a browser, but gpui lays a text
        // element out as a full line box — and Vue anchors every Fragment
        // (`v-for`, multi-root components) with `createText("")`, so leaving it
        // in adds a blank line per fragment.
        Kind::Text if node.text.is_empty() => Empty.into_any_element(),
        Kind::Text => SharedString::from(node.text.clone()).into_any_element(),
        Kind::Anchor => Empty.into_any_element(),
        // Metadata content renders NOTHING in a browser — it is not a box, it
        // is instructions. reka-ui injects a `<style>` element at runtime to
        // hide a scroll container's scrollbars, and painting its source as text
        // put a paragraph of CSS at the top of the page.
        Kind::Element if is_metadata(t.name(node.tag)) => {
            if t.name(node.tag) == "style" {
                warn_compat(
                    "runtime-style-element",
                    "a <style> element in the tree renders nothing (as in a browser) but its CSS \
                     is NOT installed either — vue-native's stylesheet is set once per build from \
                     the SFC <style> blocks, so a stylesheet injected at runtime has no effect",
                );
            }
            Empty.into_any_element()
        }
        // `stack` moves the PAINT into gpui's deferred layer for `position:
        // fixed` / positive `z-index`; the window-control hitbox rides the
        // inner element either way, and a deferred paint puts it *above* the
        // content it escaped — which is the order the fork's topmost-first
        // resolution (F-01) already wants.
        Kind::Element if t.name(node.tag) == "img" => stack(
            &extras,
            probe(id, build_img(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element if t.name(node.tag) == "lottie" => stack(
            &extras,
            probe(id, build_lottie(t, id, node, window, cx, now, control_area)),
        ),
        // Inline `<svg>` is a LEAF: the subtree is serialized into a document
        // resvg rasterises, so its children must not also become taffy nodes
        // (see crate::svg).
        Kind::Element if t.name(node.tag) == "svg" => stack(
            &extras,
            probe(id, build_svg(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element => stack(&extras, probe_at(id, id == t.root, {
            // Every mirrored node gets a stable ElementId — hover/active/
            // scroll state in gpui is keyed by it.
            let mut d = with_control_area(div().id(id as usize), id, control_area);
            if let Some(cs) = &node.computed {
                d = apply_computed_style(d, cs, id, now);
            }
            // "The window IS the root" (see `createApp().mount()`): the root
            // element is vue-native's viewport, so it fills the box the host
            // gives it unless a rule says otherwise. Without this it sizes to
            // its CONTENT, and then `height: 100%` on its child resolves
            // against an indefinite parent — which is to say `overflow: auto`
            // never scrolls and `position: fixed` (laid out against the root)
            // lands wherever the content happens to end.
            if id == t.root {
                let style = d.style();
                let full = gpui::Length::Definite(gpui::relative(1.0));
                style.size.width.get_or_insert(full);
                style.size.height.get_or_insert(full);
                // A viewport SCROLLS its overflow — that is the one behaviour
                // that makes "the root is the viewport" hold for content taller
                // than the window. Without it the overflow simply paints
                // outside the box, over whatever the host draws below.
                style.overflow.x.get_or_insert(Overflow::Scroll);
                style.overflow.y.get_or_insert(Overflow::Scroll);
                // …and it is also the node that stands in for `document`. gpui
                // dispatches a keystroke down the FOCUS path, so a listener on
                // an element that is not on that path never fires — which would
                // make every `document.addEventListener("keydown")` inert, and
                // with it every Escape-to-dismiss in the ecosystem. Tracking a
                // focus handle here puts the root on the path;
                // [`ensure_root_focus`] is what keeps something on it when the
                // app has focused nothing of its own (the browser's "the body
                // has focus by default").
                d = d.track_focus(&focus_handle_for(id, None, cx));
            }
            // `overflow: scroll|auto` on either axis. The style itself is all
            // gpui's layout/clipping needs (it came in through the wholesale
            // assignment above, so calling `.overflow_y_scroll()` here would
            // only re-set what is already set); what makes the box actually
            // *scroll* is a scroll offset to paint through — `track_scroll`
            // supplies one that outlives the frame, and its presence is also
            // what makes gpui insert the hitbox and the wheel listener that
            // move it. Read off the FINAL style, so the root's viewport-scroll
            // default counts too.
            let scrolls = {
                let overflow = &d.style().overflow;
                overflow.x == Some(Overflow::Scroll) || overflow.y == Some(Overflow::Scroll)
            };
            let scroll_handle = scrolls.then(|| {
                SCROLL_HANDLES.with(|m| m.borrow_mut().entry(id).or_default().clone())
            });
            if let Some(h) = &scroll_handle {
                d = d.track_scroll(h);
            }

            let mask = pointer_mask(node);
            let bit = move |ev: u32| mask & (1 << ev) != 0;
            // `<input>` and `<textarea>` are the SAME native widget — only
            // `TextInputOptions::multiline` differs (see `build_input`), so
            // everything the wrapper does for one it does for the other.
            let is_input = matches!(t.name(node.tag), "input" | "textarea");
            // Both are tabbable by default in HTML; a plain <div> is not.
            let tab_index = tab_index_of(t, id).or(is_input.then_some(0));

            // Keyboard/focus events and tab navigation need a persistent
            // FocusHandle. `<input>` owns its own focus handle inside the
            // native widget — it is decorated through TextInputOptions instead.
            let focusable = !is_input
                && (tab_index.is_some()
                    || bit(tree::EV_KEYDOWN)
                    || bit(tree::EV_KEYUP)
                    || bit(tree::EV_FOCUS)
                    || bit(tree::EV_BLUR));
            // gpui's `.focus()` / `.focus_visible()` rungs are keyed on a
            // TRACKED focus handle, so a node with a `:focus` rule needs one
            // even with no listeners and no `tabindex`. It must not become
            // click-focusable though: in a browser a plain <div> carrying a
            // `:focus` rule still needs `tabindex` before it can ever focus.
            let styled_focus = node.computed.as_ref().is_some_and(|cs| cs.needs_focus_handle());
            let handle =
                (focusable || (!is_input && styled_focus)).then(|| focus_handle_for(id, tab_index, cx));
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
                }
            }
            // Only a browser-focusable node takes focus from a press.
            let click_focus = handle.clone().filter(|_| focusable);
            // `<input>` forwards its own keystrokes from inside the widget (it
            // has to run before gpui resolves the editing key bindings), so the
            // wrapper must not also listen — the editor is its descendant and
            // would deliver every key twice.
            d = bind_events(d, id, mask, click_focus, focusable, !is_input, cx);

            // `scroll` reports the position *after* the move. gpui installs its
            // own wheel listener that advances the offset during
            // `Interactivity::paint`, i.e. after ours; mouse listeners bubble
            // in reverse registration order, so gpui's runs first and the
            // handle already holds the new offset when this one reads it.
            if let Some(scroll) = scroll_handle.clone().filter(|_| bit(tree::EV_SCROLL)) {
                d = d.on_scroll_wheel(cx.listener(move |_, _: &ScrollWheelEvent, window, cx| {
                    // gpui offsets are <= 0 (content shifted up/left); CSS
                    // scrollTop/scrollLeft count down/right as positive.
                    let offset = scroll.offset();
                    let (left, top) = (-f32::from(offset.x), -f32::from(offset.y));
                    let moved = SCROLL_REPORTED.with(|m| {
                        let mut m = m.borrow_mut();
                        if m.get(&id) == Some(&(left, top)) {
                            return false;
                        }
                        m.insert(id, (left, top));
                        true
                    });
                    if moved {
                        let payload = EventPayload {
                            a: left as f64,
                            b: top as f64,
                            ..Default::default()
                        };
                        js_dispatch(id, tree::EV_SCROLL, &payload, window, cx);
                    }
                }));
            }

            if is_input {
                let (input, focus_target) = build_input(t, id, node, tab_index, window, cx);
                // The editor's own handle carries this node's focus identity
                // (mouse, Tab and programmatic focus all land on it), so the
                // focus/blur events hang off it.
                watch_focus(id, &focus_target, window, cx);
                // `:focus` on an `<input>` cannot ride gpui's ladder: the
                // focus identity lives on the editor's handle INSIDE the
                // widget, and tracking that same handle on the wrapper as well
                // would enter it into the tab order twice (Tab would stop here
                // on two consecutive presses). The rungs are folded into the
                // wrapper's style instead, read live off that handle — same
                // pixels, one tab stop.
                if let Some(cs) = &node.computed {
                    if focus_target.is_focused(window) {
                        use gpui::Refineable as _;
                        if let Some(f) = &cs.focus {
                            d.style().refine(f);
                        }
                        if let Some(f) = &cs.focus_visible {
                            if window.last_input_was_keyboard() {
                                d.style().refine(f);
                            }
                        }
                    }
                }
                // Clicks on the wrapper's padding/border focus the editor —
                // the HTML behavior.
                d = d.on_any_mouse_down(cx.listener(move |_, _: &MouseDownEvent, window, cx| {
                    window.focus(&focus_target, cx);
                }));
                d.child(input).into_any_element()
            } else if !node.text.is_empty() {
                d.child(node.text.clone()).into_any_element()
            } else {
                d.children(
                    laid_out_children(t, node)
                        .map(|c| build_node(t, c, window, cx, now, drag)),
                )
                .into_any_element()
            }
        })),
    }
}

/// Does this node produce a box the parent's layout has to make room for?
///
/// **Fragment anchors and empty text nodes do not, and that is a compat fix.**
/// Vue anchors every `Fragment` — every `v-for`, every multi-root component,
/// every `<Teleport>` — with `createComment()` or `createText("")`, and reka-ui
/// components are almost all multi-root, so on `apps/shadcn` **325 of 548 tree
/// nodes** are one of the two. In a browser a comment node and an empty text
/// node are not boxes at all: they generate no CSS box, they are not flex
/// items, and `gap` does not put a gap around them. Lowering them into
/// `gpui::Empty` made every one of them a REAL taffy node AND a REAL flex item,
/// so a `flex gap-2` container with three components in it laid out with five
/// gaps instead of two.
///
/// Skipping them here fixes that spacing *and* cuts taffy's input by ~60 %
/// (research/reports/m11/perf-restyle-rebuild.md §7 measured it and deliberately
/// left it for a compat round, because it is visible).
///
/// The anchors keep their protocol role in full: they are still real nodes in
/// the retained tree, still returned by `nextSibling`, still valid `anchor`
/// arguments to `insert`. Only the LAYOUT lowering skips them — which is
/// exactly the browser's split between the node tree and the box tree.
///
/// An empty `<div>` is NOT in this set: a browser lays that out (it is an
/// element, it generates a box), and so do we.
fn generates_a_box(t: &Tree, id: u32) -> bool {
    match t.node(id) {
        None => false,
        Some(n) => match n.kind {
            Kind::Anchor => false,
            Kind::Text => !n.text.is_empty(),
            Kind::Element => true,
        },
    }
}

/// The children of `node` that become layout nodes, in document order.
fn laid_out_children<'t>(
    t: &'t Tree,
    node: &'t crate::tree::Node,
) -> impl Iterator<Item = u32> + 't {
    node.children
        .iter()
        .copied()
        .filter(move |&c| generates_a_box(t, c))
}

/// HTML metadata content: elements that carry instructions, not a box. A
/// browser gives all of these `display: none` from its UA stylesheet.
fn is_metadata(tag: &str) -> bool {
    matches!(tag, "style" | "script" | "template" | "head" | "meta" | "link" | "title" | "base")
}

/// The computed [`vn_css::Extras`] of a node (the default before the first
/// cascade, which is also what an unstyled node computes to).
fn extras_of(node: &crate::tree::Node) -> vn_css::Extras {
    node.computed.as_ref().map(|cs| cs.extras).unwrap_or_default()
}

/// The bits `pointer-events: none` turns off — the pointer half of the event
/// set. Keyboard and focus listeners are deliberately NOT in it: in a browser
/// `pointer-events: none` makes an element transparent to the POINTER only, and
/// a `tabindex`ed one still focuses and still receives keystrokes.
const POINTER_BITS: u32 = (1 << tree::EV_CLICK)
    | (1 << tree::EV_MOUSEDOWN)
    | (1 << tree::EV_MOUSEUP)
    | (1 << tree::EV_MOUSEMOVE)
    | (1 << tree::EV_MOUSEENTER)
    | (1 << tree::EV_MOUSELEAVE)
    | (1 << tree::EV_WHEEL);

/// The node's listener mask with `pointer-events: none` applied. Together with
/// the hover/active rungs being skipped in [`apply_computed_style`], this is
/// what stops gpui giving the node a hitbox at all.
fn pointer_mask(node: &crate::tree::Node) -> u32 {
    let inert = node
        .computed
        .as_ref()
        .is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
    if inert {
        node.listeners & !POINTER_BITS
    } else {
        node.listeners
    }
}

/// The style half every element shares: the folded base refinement assigned
/// wholesale, the running transition overlay refined over it, the `translate()`
/// offset, then gpui's own hover/active ladder rungs (the FnOnce builders run
/// eagerly — only the resulting refinement is stored).
fn apply_computed_style<E>(
    mut e: E,
    cs: &vn_css::ComputedStates,
    id: u32,
    now: std::time::Instant,
) -> E
where
    E: gpui::Styled + gpui::StatefulInteractiveElement + 'static,
{
    *e.style() = (*cs.base).clone();
    if let Some(overlay) = tree::animation_overlay(id, &cs.base, now) {
        use gpui::Refineable as _;
        e.style().refine(&overlay);
    }
    apply_translate(e.style(), id, cs);
    // `pointer-events: none` means the node is not interactive at all — so it
    // must not carry gpui's hover/active rungs either, which are exactly what
    // would make gpui insert a hitbox for it.
    if cs.extras.pointer_events_none != Some(true) {
        if let Some(h) = cs.hover.clone() {
            e = e.hover(move |_| (*h).clone());
        }
        if let Some(a) = cs.active.clone() {
            e = e.active(move |_| (*a).clone());
        }
    }
    e
}

/// `transform: translate(…)` as a LAYOUT offset (compat roadmap G3).
///
/// gpui applies no matrix to a laid out box, so the only way to move one is to
/// move it in layout — margins are the additive channel that works for both an
/// in-flow box and an absolutely positioned one (taffy adds the margin to the
/// inset-resolved origin).
///
/// A PERCENTAGE translate resolves against the element's own size, which is not
/// known until it has been laid out — so it is resolved against the rectangle
/// the node was painted at in the previous frame, exactly like `<lottie>`'s
/// intrinsic sizing. A box that has never been painted translates by 0 on its
/// first frame and lands correctly on the second; since a translate does not
/// change the box's SIZE, that converges rather than oscillates.
fn apply_translate(style: &mut gpui::StyleRefinement, id: u32, cs: &vn_css::ComputedStates) {
    let Some((tx, ty)) = cs.extras.translate else {
        return;
    };
    let (w, h) = prev_size_of(id).unwrap_or((0.0, 0.0));
    let (dx, dy) = (tx.to_px(w), ty.to_px(h));
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    fn shift(slot: &mut Option<gpui::Length>, delta: f32, id: u32, side: &str) {
        match slot {
            None => *slot = Some(gpui::Length::Definite(gpui::px(delta).into())),
            Some(gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(p),
            ))) => {
                *slot = Some(gpui::Length::Definite(gpui::px(f32::from(*p) + delta).into()));
            }
            // `margin: auto` (centring) or a percentage margin cannot absorb a
            // pixel offset. Rather than silently pick one, say which node.
            _ => warn_compat(
                "translate-margin",
                &format!(
                    "`transform: translate()` is applied as a margin offset, and node {id} already \
                     has a non-pixel `margin-{side}` (auto or a percentage) — the translate was \
                     dropped on that axis"
                ),
            ),
        }
    }
    if dx != 0.0 {
        shift(&mut style.margin.left, dx, id, "left");
    }
    if dy != 0.0 {
        shift(&mut style.margin.top, dy, id, "top");
    }
}

/// Wraps a node that has to escape its siblings' paint order — `position:
/// fixed`, or a positive `z-index` — in gpui's deferred layer.
///
/// `Deferred` keeps the child's LAYOUT where it is (it returns the child's own
/// `LayoutId`) and only moves the PAINT to after every ancestor, ordered by
/// priority; gpui sorts those stably, so two elements with the same `z-index`
/// keep document order, exactly as CSS says they should.
fn stack(extras: &vn_css::Extras, el: AnyElement) -> AnyElement {
    match extras.deferred_priority() {
        Some(priority) => gpui::deferred(el).with_priority(priority).into_any_element(),
        None => el,
    }
}

/// The listener half every element shares. `keys_here` is false for `<input>`,
/// whose editor forwards its own keystrokes from inside the widget.
#[allow(clippy::too_many_arguments)]
fn bind_events<E, V>(
    mut e: E,
    id: u32,
    mask: u32,
    click_focus: Option<FocusHandle>,
    focusable: bool,
    keys_here: bool,
    cx: &mut Context<V>,
) -> E
where
    E: gpui::StatefulInteractiveElement + 'static,
    V: 'static,
{
    let bit = move |ev: u32| mask & (1 << ev) != 0;
    if bit(tree::EV_CLICK) {
        e = e.on_click(cx.listener(move |_, ev: &ClickEvent, window, cx| {
            let payload = mouse_payload(ev.position(), 0.0, &ev.modifiers());
            js_dispatch(id, tree::EV_CLICK, &payload, window, cx);
        }));
    }
    if bit(tree::EV_MOUSEDOWN) || focusable {
        let handle = click_focus;
        e = e.on_any_mouse_down(cx.listener(move |_, ev: &MouseDownEvent, window, cx| {
            if let Some(h) = &handle {
                // Focus follows the press, as in a browser; the focus/blur
                // events come out of the subscription.
                window.focus(h, cx);
            }
            if bit(tree::EV_MOUSEDOWN) {
                let payload = mouse_payload(ev.position, button_code(ev.button), &ev.modifiers);
                js_dispatch(id, tree::EV_MOUSEDOWN, &payload, window, cx);
            }
        }));
    }
    if bit(tree::EV_MOUSEUP) {
        // gpui's API is per-button here (no any-button variant on the element
        // traits) — attach one listener per button.
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            e = e.on_mouse_up(
                button,
                cx.listener(move |_, ev: &MouseUpEvent, window, cx| {
                    let payload = mouse_payload(ev.position, button_code(ev.button), &ev.modifiers);
                    js_dispatch(id, tree::EV_MOUSEUP, &payload, window, cx);
                }),
            );
        }
    }
    // The pointer family (compat roadmap G4). gpui sources exactly two things
    // here — "the mouse moved while over me" and "the mouse entered/left me" —
    // and the JS runtime derives the whole W3C pointer/mouse set from them.
    //
    // `on_hover` is what makes `mouseenter`/`mouseleave` (and therefore
    // `pointerenter`/`pointerleave`) correct rather than approximated: gpui
    // tracks the hovered hitbox itself and reports both edges, so the
    // non-bubbling "entered this element" semantics come for free instead of
    // being reconstructed from move coordinates.
    if bit(tree::EV_MOUSEMOVE) {
        // No dispatch here — stash and let the pump deliver ONE mousemove per
        // frame (see PendingMouseMove). Same position = same physical move
        // seen by an outer listener: the innermost already claimed it.
        e = e.on_mouse_move(cx.listener(move |_, ev: &gpui::MouseMoveEvent, _window, _cx| {
            PENDING_MOUSE_MOVE.with(|c| {
                let mut slot = c.borrow_mut();
                if let Some(p) = slot.as_ref() {
                    if p.position == ev.position {
                        return;
                    }
                }
                *slot = Some(PendingMouseMove {
                    id,
                    position: ev.position,
                    button: button_code_opt(ev.pressed_button),
                    modifiers: ev.modifiers,
                });
            });
        }));
    }
    if bit(tree::EV_MOUSEENTER) || bit(tree::EV_MOUSELEAVE) {
        e = e.on_hover(cx.listener(move |_, hovered: &bool, window, cx| {
            let event = if *hovered { tree::EV_MOUSEENTER } else { tree::EV_MOUSELEAVE };
            if !bit(event) {
                return;
            }
            let position = window.mouse_position();
            let payload = mouse_payload(position, -1.0, &window.modifiers());
            js_dispatch(id, event, &payload, window, cx);
        }));
    }
    if bit(tree::EV_WHEEL) {
        e = e.on_scroll_wheel(cx.listener(move |_, ev: &ScrollWheelEvent, window, cx| {
            let delta = ev.delta.pixel_delta(gpui::px(16.0));
            let payload = EventPayload {
                a: f64::from(delta.x),
                b: f64::from(delta.y),
                c: 0.0,
                d: mods_bits(&ev.modifiers),
                key: None,
            };
            js_dispatch(id, tree::EV_WHEEL, &payload, window, cx);
        }));
    }
    if bit(tree::EV_KEYDOWN) && keys_here {
        e = e.on_key_down(cx.listener(move |_, ev: &KeyDownEvent, window, cx| {
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            js_dispatch(id, tree::EV_KEYDOWN, &payload, window, cx);
        }));
    }
    if bit(tree::EV_KEYUP) && keys_here {
        e = e.on_key_up(cx.listener(move |_, ev: &KeyUpEvent, window, cx| {
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            js_dispatch(id, tree::EV_KEYUP, &payload, window, cx);
        }));
    }
    e
}

// ---------------------------------------------------------------------------
// programmatic driving (vn-devtools; also the `el.focus()` groundwork)
// ---------------------------------------------------------------------------

/// The focus handle that IS this node's focus identity, if it has one.
///
/// `<input>` is the interesting case: its identity lives on the editor's own
/// handle inside the native widget (see [`build_input`]), not on the wrapper —
/// so focusing the wrapper's handle would focus nothing a keystroke can reach.
fn focus_target_of(id: u32, cx: &App) -> Option<FocusHandle> {
    if let Some(state) = INPUT_STATES.with(|m| m.borrow().get(&id).cloned()) {
        return Some(state.read(cx).focus_handle());
    }
    FOCUS_HANDLES.with(|m| m.borrow().get(&id).cloned())
}

/// `el.focus()`, host-side. Only a node the renderer already decided is
/// focusable has a handle — a plain `<div>` with no `tabindex`, no key/focus
/// listener and no `:focus` rule has none, which is exactly the set a browser
/// refuses to focus too.
///
/// The `focus`/`blur` events are NOT synthesized here: they come out of the
/// gpui focus observers ([`watch_focus`]), so programmatic focus reports
/// identically to a click or a `Tab` — the invariant the module header states.
pub(crate) fn focus_node(id: u32, window: &mut Window, cx: &mut App) -> Result<(), String> {
    let Some(handle) = focus_target_of(id, cx) else {
        return Err(format!(
            "node {id} is not focusable — it has no tabindex, no key/focus listener and no \
             `:focus` rule, so the renderer never gave it a focus handle (a browser would refuse \
             `el.focus()` on it for the same reason)"
        ));
    };
    window.focus(&handle, cx);
    Ok(())
}

/// True while `id` owns keyboard focus. Used to wait out the frame a
/// programmatic focus needs before the native input handler is installed.
pub(crate) fn is_node_focused(id: u32, window: &Window, cx: &App) -> bool {
    focus_target_of(id, cx).is_some_and(|h| h.is_focused(window))
}

/// The live text of a native `<input>` — the widget's own buffer, not the
/// `value` prop, so it reflects typing that JS has not echoed back yet.
pub(crate) fn input_value(id: u32, cx: &App) -> Option<String> {
    INPUT_STATES.with(|m| m.borrow().get(&id).map(|s| s.read(cx).text().to_string()))
}

/// Drives a scroll container's own [`ScrollHandle`] by `(dx, dy)` CSS pixels
/// and fires the same `scroll` event the wheel listener fires (position after
/// the move, CSS sign convention). Returns the new `(scrollLeft, scrollTop)`.
///
/// Only a node the cascade gave `overflow: scroll|auto` has a handle; anything
/// else is a real error rather than a silent no-op.
pub(crate) fn scroll_node(
    id: u32,
    dx: f32,
    dy: f32,
    window: &mut Window,
    cx: &mut App,
) -> Result<(f32, f32), String> {
    let Some(handle) = SCROLL_HANDLES.with(|m| m.borrow().get(&id).cloned()) else {
        // A `<textarea>` scrolls its own overflow inside the native widget
        // rather than through a gpui `ScrollHandle`, so it has no entry here.
        // Route to the editor instead — otherwise devtools would report "not a
        // scroll container" about the one element on the page that visibly is.
        if let Some(state) = INPUT_STATES.with(|m| m.borrow().get(&id).cloned()) {
            let top = state.update(cx, |s, cx| {
                s.scroll_by(gpui::px(dy), window, cx);
                f32::from(s.scroll_top())
            });
            return Ok((0.0, top));
        }
        return Err(format!(
            "node {id} is not a scroll container — nothing in the cascade gave it \
             `overflow: scroll` or `overflow: auto`, so there is no scroll offset to move"
        ));
    };
    // gpui offsets are <= 0 (content shifted up/left); CSS scrollTop/scrollLeft
    // count down/right as positive, and so does the `dy` a caller passes.
    let offset = handle.offset();
    let max = handle.max_offset();
    let want = gpui::point(offset.x - gpui::px(dx), offset.y - gpui::px(dy));
    // The same clamp gpui's own wheel handler applies, so a programmatic scroll
    // cannot leave the box somewhere a real wheel could not.
    let clamped = gpui::point(
        want.x.clamp(-max.x, gpui::px(0.)),
        want.y.clamp(-max.y, gpui::px(0.)),
    );
    handle.set_offset(clamped);
    let (left, top) = (-f32::from(clamped.x), -f32::from(clamped.y));
    let moved = SCROLL_REPORTED.with(|m| {
        let mut m = m.borrow_mut();
        if m.get(&id) == Some(&(left, top)) {
            return false;
        }
        m.insert(id, (left, top));
        true
    });
    if moved {
        let payload = EventPayload { a: left as f64, b: top as f64, ..Default::default() };
        js_dispatch_app(id, tree::EV_SCROLL, &payload, window, cx);
    }
    window.refresh();
    Ok((left, top))
}

/// Synthesizes one input event on `id` through exactly the path a real gpui
/// listener takes: publish `Phase::Commit`, hand the compact payload to the JS
/// dispatcher (which builds the W3C event and runs capture/bubble over the
/// mirror tree), then flush the tree so Vue's wave repaints in this frame.
/// Returns `false` when a handler called `preventDefault()`.
pub(crate) fn synth_event(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    js_dispatch_app(id, event, payload, window, cx)
}

// ---------------------------------------------------------------------------
// <img>
// ---------------------------------------------------------------------------

/// What a browser paints when an image is broken: the `alt` text. An empty
/// `alt` collapses to nothing, exactly as `alt=""` (the "decorative image"
/// spelling) does in HTML.
fn alt_element(alt: &str) -> AnyElement {
    if alt.is_empty() {
        Empty.into_any_element()
    } else {
        div()
            .child(SharedString::from(alt.to_string()))
            .into_any_element()
    }
}

fn gpui_object_fit(fit: vn_css::ObjectFit) -> gpui::ObjectFit {
    match fit {
        vn_css::ObjectFit::Fill => gpui::ObjectFit::Fill,
        vn_css::ObjectFit::Contain => gpui::ObjectFit::Contain,
        vn_css::ObjectFit::Cover => gpui::ObjectFit::Cover,
        vn_css::ObjectFit::ScaleDown => gpui::ObjectFit::ScaleDown,
        vn_css::ObjectFit::None => gpui::ObjectFit::None,
    }
}

/// The HTML `width` / `height` **attributes** — presentational hints, so they
/// only fill in a dimension no CSS rule declared (in a browser they sit in the
/// author-level-beating "presentational hints" origin, i.e. below every real
/// declaration). Values are unitless CSS pixels.
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

/// `<img>` — a REPLACED element, so it is a gpui `img()` rather than a `div`
/// with a background.
///
/// * `src` resolution and its compat advisories live in [`crate::img`].
/// * Sizing follows the browser: with no CSS size the intrinsic (natural) pixel
///   size is used, and one CSS dimension plus `auto` derives the other from the
///   intrinsic aspect ratio — gpui's `Img::request_layout` implements exactly
///   that, so nothing is needed here beyond letting the refinement through.
/// * `object-fit` comes off the cascade (`ComputedStates::extras`); the initial
///   value is the CSS one, `fill`, NOT gpui's `Contain` default.
/// * A source that fails to load renders `alt`, never a panic, and leaves one
///   advisory naming the `src` that was tried.
fn build_img<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let src = t.prop(id, "src").unwrap_or("").to_string();
    let alt = t.prop(id, "alt").unwrap_or("").to_string();

    let Some(source) = img_src::source_for(id, &src) else {
        // Nothing loadable at all (empty/unsupported `src` — already advised).
        // A browser still lays the element out and paints the alt text, so the
        // node keeps its box and its styles.
        let mut d = with_control_area(div().id(id as usize), id, control_area);
        if let Some(cs) = &node.computed {
            d = apply_computed_style(d, cs, id, now);
        }
        presentational_size(t, id, d.style());
        return d.child(alt_element(&alt)).into_any_element();
    };

    let mut e = with_control_area(gpui::img(source).id(id as usize), id, control_area);
    if let Some(cs) = &node.computed {
        e = apply_computed_style(e, cs, id, now);
    }
    presentational_size(t, id, e.style());

    let fit = node
        .computed
        .as_ref()
        .and_then(|cs| cs.extras.object_fit)
        .map(gpui_object_fit)
        // CSS initial value. gpui's ImageStyle defaults to Contain, which would
        // silently letterbox every sized <img> the web would have stretched.
        .unwrap_or(gpui::ObjectFit::Fill);
    e = e.object_fit(fit);

    // gpui runs this only when the load actually failed, which makes it the
    // one place that knows a `src` is broken — there is no decode callback to
    // hang a DOM `error` event on (see research/reports/m10/images.md).
    // A data: URL is its own content — printing all of it buries the log.
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
                 text instead (a browser would also fire an `error` event; vue-native does not \
                 yet)"
            ),
        );
        alt_element(&failed_alt)
    });

    let mask = pointer_mask(node);
    let bit = |ev: u32| mask & (1 << ev) != 0;
    // `<img>` is not focusable in HTML without `tabindex`.
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some()
        || bit(tree::EV_KEYDOWN)
        || bit(tree::EV_KEYUP)
        || bit(tree::EV_FOCUS)
        || bit(tree::EV_BLUR);
    let styled_focus = node.computed.as_ref().is_some_and(|cs| cs.needs_focus_handle());
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

// ---------------------------------------------------------------------------
// inline <svg>
// ---------------------------------------------------------------------------

/// A `gpui::Svg` that paints in the **inherited** text colour.
///
/// gpui renders an SVG as a monochrome alpha mask tinted by the element's own
/// `style.text.color`, and `Style::default()` leaves that `None` — an SVG with
/// no colour of its own paints nothing at all. In CSS, though, `color` is
/// inherited, and `stroke="currentColor"` (what every icon set ships) means
/// exactly "whatever `color` computed to here".
///
/// vn-css folds no inherited properties, but gpui itself carries a text style
/// down the PAINT walk (`Div::paint` wraps its children in
/// `window.with_text_style(style.text_style())` using the style it computed,
/// hover rungs and all). Reading `window.text_style()` at paint time therefore
/// gives the browser's answer, live: a `:hover` on an ancestor that changes
/// `text-*` re-tints the icon on the frame the hover starts, with no restyle,
/// no re-serialization and no re-rasterization — the sprite in the atlas is
/// colour-independent.
///
/// The element's OWN `color` still wins when it declares one, because that
/// arrives through `apply_computed_style` and is only defaulted here.
struct SvgTint<E> {
    inner: E,
    /// False when the cascade already gave this node a `color` of its own.
    inherit: bool,
}

impl<E: gpui::Element + gpui::Styled> gpui::IntoElement for SvgTint<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
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
            // Only fills the BASE rung, so a `:hover` colour declared on the
            // svg itself still overrides it inside gpui's own ladder.
            self.inner.style().text.color = Some(window.text_style().color);
        }
        self.inner
            .paint(id, inspector_id, bounds, request_layout, prepaint, window, cx)
    }
}

/// `rotate()` / `scale()` as a sprite transformation.
///
/// This is the ONE place in the framework where a real CSS transform matrix
/// applies: gpui's `MonochromeSprite` carries a `TransformationMatrix`, and an
/// SVG leaf is a monochrome sprite. A laid out box is not, which is why
/// [`advise_untransformable`] exists.
///
/// `Transformation::into_matrix` rotates and scales about the element's own
/// centre, which is CSS's initial `transform-origin: 50% 50%`.
fn svg_transformation(
    id: u32,
    extras: &vn_css::Extras,
    now: std::time::Instant,
) -> Option<gpui::Transformation> {
    // The animated value when a `@keyframes` is driving it (`animate-spin`),
    // the element's own otherwise.
    let (rotate, scale) = tree::animation_transform(id, extras, now);
    if rotate.is_none() && scale.is_none() {
        return None;
    }
    let mut t = gpui::Transformation::default();
    if let Some(deg) = rotate {
        t = t.with_rotation(gpui::radians(deg.to_radians()));
    }
    if let Some((sx, sy)) = scale {
        t = t.with_scaling(gpui::size(sx, sy));
    }
    Some(t)
}

/// A `rotate()`/`scale()` on anything that is not an `<svg>` leaf.
fn advise_untransformable(t: &Tree, node: &crate::tree::Node, extras: &vn_css::Extras) {
    if extras.rotate.is_none() && extras.scale.is_none() {
        return;
    }
    if t.name(node.tag) == "svg" {
        return;
    }
    warn_compat(
        "transform-rotate-scale",
        "`transform: rotate()/scale()` applies only to an inline <svg> (v1) — gpui carries a \
         transformation matrix on a SPRITE (which is what an <svg> leaf paints as) and none at \
         all on a laid out box, so the declaration was dropped on this element. `translate()` \
         works everywhere; `animate-spin` on an icon works",
    );
}

/// Inline `<svg>` — a replaced element whose content is the serialized subtree.
///
/// * The children are NOT lowered (`crate::svg` explains why).
/// * `width`/`height` attributes are presentational hints exactly as on `<img>`,
///   so a lucide icon with no CSS size still lays out at its natural 24×24.
/// * A document usvg cannot parse paints nothing and leaves one advisory naming
///   the node, rather than a silent hole where an icon should be.
fn build_svg<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let extras = extras_of(node);
    let doc = crate::svg::document(t, id, extras.fill, extras.stroke);
    if crate::svg::last_had_explicit_color() {
        warn_compat(
            "svg-multicolor",
            "an inline <svg> declares a concrete `fill`/`stroke` colour — gpui rasterises an SVG \
             into a single-colour alpha MASK and tints the whole sprite with the element's \
             computed `color`, so every painted part of the icon comes out that one colour. \
             Monochrome icon sets (`stroke=\"currentColor\"`) are unaffected",
        );
    }

    let mut leaf = gpui::svg().data(&doc);
    if let Some(transformation) = svg_transformation(id, &extras, now) {
        leaf = leaf.with_transformation(transformation);
    }
    let mut e = with_control_area(leaf.id(id as usize), id, control_area);
    let mut inherit = true;
    if let Some(cs) = &node.computed {
        inherit = cs.base.text.color.is_none();
        e = apply_computed_style(e, cs, id, now);
    }
    presentational_size(t, id, e.style());

    let mask = pointer_mask(node);
    let bit = |ev: u32| mask & (1 << ev) != 0;
    // Not focusable in HTML without `tabindex` (an inline <svg> is not a
    // control), same as <img>.
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some()
        || bit(tree::EV_KEYDOWN)
        || bit(tree::EV_KEYUP)
        || bit(tree::EV_FOCUS)
        || bit(tree::EV_BLUR);
    let styled_focus = node.computed.as_ref().is_some_and(|cs| cs.needs_focus_handle());
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

// ---------------------------------------------------------------------------
// <lottie>
// ---------------------------------------------------------------------------

fn lottie_fit(fit: vn_css::ObjectFit) -> vn_lottie::LottieFit {
    match fit {
        vn_css::ObjectFit::Fill => vn_lottie::LottieFit::Fill,
        vn_css::ObjectFit::Contain => vn_lottie::LottieFit::Contain,
        vn_css::ObjectFit::Cover => vn_lottie::LottieFit::Cover,
        vn_css::ObjectFit::ScaleDown => vn_lottie::LottieFit::ScaleDown,
        vn_css::ObjectFit::None => vn_lottie::LottieFit::None,
    }
}

/// One CSS length in pixels, when it can be known without layout. `50%` and
/// `auto` cannot be — the caller falls back to the rectangle the node was
/// painted at last frame.
fn definite_px(len: Option<gpui::Length>, rem: gpui::Pixels) -> Option<f32> {
    match len? {
        gpui::Length::Definite(gpui::DefiniteLength::Absolute(a)) => {
            Some(f32::from(a.to_pixels(rem)))
        }
        _ => None,
    }
}

/// True when the style declares this dimension at all (`auto` is "it doesn't").
fn is_sized(len: Option<gpui::Length>) -> bool {
    matches!(len, Some(gpui::Length::Definite(_)))
}

/// `<lottie src="sticker.tgs" autoplay loop speed="1.5" @complete="…" />` — a
/// replaced element whose content is a rasterised Lottie frame.
///
/// Structurally it is a styled box (so the cascade, `background`, borders and
/// the event listeners all work exactly as on a `<div>`) wrapping the crate's
/// `size_full()` image. The transport, the load and the events live in
/// [`crate::lottie`]; the only thing decided here is the BOX, per CONTRACT.md §6:
///
/// * CSS `width`/`height` win outright;
/// * with neither, the box takes the animation's intrinsic canvas size — the
///   `<img>` rule;
/// * with one, the other follows from the intrinsic aspect ratio.
///
/// The raster is produced at the size the node was actually painted at last
/// frame when that is known, so a relatively sized box (`width: 100%`, a flex
/// child) rasterises at its real pixel size instead of a guess. On the first
/// frame — and only then — the computed pair above is used instead; the crate
/// re-rasterises on any size change, so the picture is correct from frame two
/// with no layout pass of our own.
fn build_lottie<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    // Load (once per `src`) and push this frame's props into the transport.
    let intrinsic = crate::lottie::sync(t, id);

    let mut d = with_control_area(div().id(id as usize), id, control_area);
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
        // A relatively sized axis is only knowable from the last paint.
        let known_w = definite_px(style_w, rem).or(painted.filter(|_| sized_w).map(|p| p.0));
        let known_h = definite_px(style_h, rem).or(painted.filter(|_| sized_h).map(|p| p.1));

        let (fill_w, fill_h) = match (sized_w, sized_h) {
            (true, true) => (None, None),
            // One axis given: the other comes off the intrinsic aspect ratio,
            // exactly as it does for an `<img>` with one CSS dimension.
            (true, false) => (None, known_w.map(|w| w * ih / iw).or(Some(ih))),
            (false, true) => (known_h.map(|h| h * iw / ih).or(Some(iw)), None),
            (false, false) => (Some(iw), Some(ih)),
        };
        if let Some(w) = fill_w {
            d.style().size.width = Some(gpui::Length::Definite(gpui::px(w).into()));
        }
        if let Some(h) = fill_h {
            d.style().size.height = Some(gpui::Length::Definite(gpui::px(h).into()));
        }

        // What to rasterise into. Last frame's rectangle is the truth when we
        // have it; otherwise the box just computed.
        let size = painted.unwrap_or((
            known_w.or(fill_w).unwrap_or(iw),
            known_h.or(fill_h).unwrap_or(ih),
        ));

        let object_fit = node
            .computed
            .as_ref()
            .and_then(|cs| cs.extras.object_fit)
            .map(lottie_fit)
            // The `objectfit` attribute is the `<lottie-player>` spelling; CSS
            // beats it, the way a presentational hint always loses.
            .or_else(|| match t.prop(id, "objectfit").map(str::trim) {
                Some("fill") => Some(vn_lottie::LottieFit::Fill),
                Some("contain") => Some(vn_lottie::LottieFit::Contain),
                Some("cover") => Some(vn_lottie::LottieFit::Cover),
                Some("scale-down") => Some(vn_lottie::LottieFit::ScaleDown),
                Some("none") => Some(vn_lottie::LottieFit::None),
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
            // NOT the CSS initial value (`fill`): `<lottie-player>`'s default is
            // `preserveAspectRatio: "xMidYMid meet"`, i.e. contain, and this
            // element mirrors that component. CONTRACT.md §6.
            .unwrap_or(vn_lottie::LottieFit::Contain);

        let options = vn_lottie::LottieOptions {
            size,
            scale: window.scale_factor(),
            object_fit,
            // gpui's image element implements `filter: grayscale(1)` natively,
            // but the cascade does not carry `filter` yet — wire it here the
            // day `Extras` grows the property.
            grayscale: false,
        };
        if let Some(child) =
            crate::lottie::with_state(id, |s| vn_lottie::lottie(s, &options, window, &mut **cx))
        {
            d = d.child(child);
        }
    }

    let mask = pointer_mask(node);
    let bit = |ev: u32| mask & (1 << ev) != 0;
    // Like `<img>`: not focusable in HTML without `tabindex`.
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some()
        || bit(tree::EV_KEYDOWN)
        || bit(tree::EV_KEYUP)
        || bit(tree::EV_FOCUS)
        || bit(tree::EV_BLUR);
    let styled_focus = node.computed.as_ref().is_some_and(|cs| cs.needs_focus_handle());
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

/// The native `<input>` / `<textarea>` widget: persistent `TextInputState`
/// entity per node, controlled through the DOM-compat prop channel (`value`,
/// `placeholder`, `disabled`, `readonly`, `rows`, `tabindex`), styled from the
/// node's computed text style, reporting edits as `input` events (with
/// `e.target.value` populated on the JS side) and keystrokes as cancellable
/// `keydown`/`keyup`.
///
/// `<textarea>` is the same widget with `multiline` set: the content wraps and
/// keeps its line breaks, Return inserts one instead of being dropped, and the
/// box scrolls vertically. Everything else — IME, selection, clipboard, undo,
/// the `v-model` echo ring, the KeepAlive-surviving state map — is shared, and
/// that is the point: a second editor would have been a second set of bugs.
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

    // Controlled value: the write counter, not the string, is what tells the
    // widget an assignment happened (`v-model` re-forcing "" after the user
    // typed again writes the same string the previous clear did).
    if let Some((value, write)) = t.prop_write(id, "value") {
        let value = value.to_string();
        state.update(cx, |s, cx| s.sync_value(&value, write, cx));
    } else if multiline && !node.text.is_empty() {
        // A `<textarea>`'s initial value is its TEXT CONTENT, not an attribute
        // — `<textarea>hello</textarea>`. Vue writes that through
        // `setElementText`, so it arrives on `Node::text`, and the wrapper's
        // `d.child(node.text)` arm is never reached for a text field. Applied
        // once (write counter 0), so a later `value` assignment still wins.
        let value = node.text.clone();
        state.update(cx, |s, cx| s.sync_value(&value, 0, cx));
    }

    let text_style = node.computed.as_ref().map(|cs| cs.base.text.clone());
    // No `color` of its own means the widget inherits one at paint time; the
    // value here is only the fallback for the frame before it paints.
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
    let placeholder_color = gpui::Hsla { a: text_color.a * 0.45, ..text_color };
    let options = TextInputOptions {
        placeholder: t.prop(id, "placeholder").unwrap_or("").to_string().into(),
        text_color,
        placeholder_color,
        selection_color: gpui::Hsla { h: 0.6, s: 0.8, l: 0.6, a: 0.35 },
        caret_color: text_color,
        font_size,
        // `readonly` and `disabled` differ in HTML (a disabled control is not
        // submitted and not focusable); for the EDITOR they mean the same
        // thing — no typing, no caret, selection and copy still work — so both
        // map onto the widget's one flag.
        disabled: t.prop(id, "disabled").is_some_and(|v| v != "false")
            || t.prop(id, "readonly").is_some_and(|v| v != "false"),
        tab_index,
        multiline,
        inherit_color,
        rows: t
            .prop(id, "rows")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(vn_elements::DEFAULT_ROWS),
    };

    let mask = pointer_mask(node);
    let bit = move |ev: u32| mask & (1 << ev) != 0;

    let on_change: vn_elements::OnChange = std::rc::Rc::new(move |text, window, cx| {
        js_dispatch_app(
            id,
            tree::EV_INPUT,
            &EventPayload { key: Some(text), ..Default::default() },
            window,
            cx,
        );
    });

    // Keystrokes are forwarded *before* the widget acts on them, so a JS
    // handler that calls `preventDefault()` cancels the default editing
    // (`dispatch_event` already reports `!defaultPrevented`).
    let on_key_down: Option<vn_elements::OnKeyDown> = bit(tree::EV_KEYDOWN).then(|| {
        std::rc::Rc::new(move |ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App| {
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            js_dispatch_app(id, tree::EV_KEYDOWN, &payload, window, cx)
        }) as vn_elements::OnKeyDown
    });
    let on_key_up: Option<vn_elements::OnKeyUp> = bit(tree::EV_KEYUP).then(|| {
        std::rc::Rc::new(move |ev: &KeyUpEvent, window: &mut Window, cx: &mut App| {
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            js_dispatch_app(id, tree::EV_KEYUP, &payload, window, cx);
        }) as vn_elements::OnKeyUp
    });

    let handlers = TextInputHandlers {
        on_change: Some(on_change),
        on_key_down,
        on_key_up,
    };

    let focus_target = state.read(cx).focus_handle();
    (text_input(&state, options, handlers, cx), focus_target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::with_tree;

    /// `<div class="flex gap-2">` with three components in it. Vue anchors each
    /// one, so the child list is element/anchor/element/anchor/element plus the
    /// empty text nodes a Fragment leaves behind.
    ///
    /// A browser puts TWO gaps in that row, because a comment and an empty text
    /// node generate no box. Emitting a `gpui::Empty` for each of them made
    /// them flex items and the row laid out with five.
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
            let laid_out: Vec<u32> = laid_out_children(t, node).collect();
            assert_eq!(laid_out, boxes, "only the three elements are flex items");
        });
    }

    /// The anchors keep their protocol role: still real nodes, still valid
    /// insertion points. Skipping them is a LAYOUT decision, not a tree one.
    #[test]
    fn a_skipped_anchor_is_still_an_insertion_point() {
        with_tree(|t| {
            let div = t.intern("div");
            let parent = t.create_element(div);
            let anchor = t.create_anchor();
            t.insert(anchor, parent, 0);
            let later = t.create_element(div);
            t.insert(later, parent, 0);
            // Vue's `insert(child, parent, anchor)` — before the comment.
            let first = t.create_element(div);
            t.insert(first, parent, anchor);

            let node = t.node(parent).unwrap();
            assert_eq!(node.children, vec![first, anchor, later]);
            assert_eq!(
                laid_out_children(t, node).collect::<Vec<_>>(),
                vec![first, later],
                "the anchor still orders the boxes without being one"
            );
        });
    }

    /// The boundary cases an over-eager filter would get wrong: an EMPTY
    /// element is a box (a browser lays `<div></div>` out), and a text node
    /// with content is one too (an anonymous flex item).
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
            assert_eq!(
                laid_out_children(t, node).collect::<Vec<_>>(),
                vec![empty_div, text]
            );
            assert!(!generates_a_box(t, blank));
            // A node that no longer exists never generates a box.
            assert!(!generates_a_box(t, 99_999));
        });
    }
}
