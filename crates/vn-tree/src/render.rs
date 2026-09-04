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
    SharedString, Window, WindowControlArea,
};
use vn_css::{WindowControl, WindowDrag};

use crate::tree::{self, Kind, Tree};
use crate::windows::{self, PendingMouseMove, Rect, WinKey};
use crate::EventPayload;
use crate::img as img_src;
use vn_elements::{text_input, TextInputHandlers, TextInputOptions, TextInputState};
use vn_runtime::{with_published, Phase};

thread_local! {
    /// Persistent per-node focus handles (minting a new handle each render
    /// would blur the node every frame). GC'd against tree liveness.
    ///
    /// **SHARED across windows, on purpose** (docs/MULTIWINDOW.md §5.1):
    /// `cx.focus_handle()` mints its id in `App::focus_handles`, not in a
    /// window, so one handle per node is correct however many windows exist —
    /// and since a node id is painted by exactly one window (A-M02), a handle
    /// can only ever be in one window's focus path anyway. What IS per-window
    /// is the *subscription* to it (`WindowState::focus_subs`), because
    /// `cx.on_focus(handle, window, …)` names a window.
    static FOCUS_HANDLES: RefCell<HashMap<u32, FocusHandle>> = RefCell::new(HashMap::new());
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
    /// Every font family the host can actually resolve. Enumerating them is a
    /// platform call over every installed font, so it happens once — see
    /// [`advise_missing_font`].
    static FONT_NAMES: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
    /// Nodes lowered in the build currently running — the VN_PERF counter. One
    /// `Cell` increment per node, unconditionally: the branch a probe check
    /// would add costs as much as the add.
    static BUILT: Cell<u64> = const { Cell::new(0) };
    /// Nodes that are being painted by SOME OTHER window (docs/MULTIWINDOW.md).
    /// A popped-out node stays in the retained tree — it keeps its id, its
    /// listeners, its computed style and its Vue reactivity — but it generates
    /// no box in the tree it was declared in, exactly like the cache container
    /// half of `<KeepAlive>`. The second window builds it as its own root
    /// through [`build_subtree`].
    ///
    /// SHARED, and correctly so: "somebody else paints this" is a fact about
    /// the ONE tree, asked by whichever window is building. Which window that
    /// somebody is lives in `windows::ROOTS`.
    static POPPED_OUT: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
}

/// Marks (or unmarks) a node as painted by another window. Returns `true` when
/// the flag actually moved, so the caller can skip a pointless repaint.
pub fn set_popped_out(id: u32, on: bool) -> bool {
    POPPED_OUT.with(|s| {
        let mut s = s.borrow_mut();
        if on { s.insert(id) } else { s.remove(&id) }
    })
}

/// Is this node currently painted by another window?
pub fn popped_out(id: u32) -> bool {
    POPPED_OUT.with(|s| s.borrow().contains(&id))
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

/// Called from `build_root` / `build_subtree`, i.e. once per `render()` of ONE
/// window: that window's rectangles from its last draw are about to be replaced
/// by this draw's, and anything it stops painting must stop having bounds in it.
///
/// **Per window, on that window's own draw schedule.** Before phase 1 this
/// rotated one global map from the primary window's build, which wiped the
/// rectangles a pop-out had recorded: a popped-out node read back as "not
/// painted (bounds null)" on every frame the primary drew and the pop-out did
/// not, which is most of them — breaking `getBoundingClientRect()`, every
/// devtools query and therefore every assertion about a pop-out.
fn reset_bounds(win: WinKey) {
    windows::of(win, |w| {
        let last = std::mem::take(&mut w.bounds);
        w.prev_bounds = last;
    });
}

fn record_bounds(id: u32, bounds: gpui::Bounds<gpui::Pixels>, window: &Window) {
    let rect = (
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    );
    // Which window's prepaint recursion are we inside? gpui just told us: this
    // is the `&mut Window` it is running the recursion with. The spike had to
    // infer it from a `SubtreeScope` element that bracketed the recursion,
    // because the map was keyed by painted root rather than by window.
    windows::here(window, |w| w.bounds.insert(id, rect));
}

/// `(x, y, width, height)` in CSS pixels, **relative to the window that painted
/// it**, from the last frame this node was painted in — `None` when it was not
/// painted at all.
///
/// Searches the primary window first and then every pop-out, which is the right
/// answer to "where is it" and the wrong one to "what is at this point" — see
/// [`bounds_in`].
pub fn bounds_of(id: u32) -> Option<Rect> {
    // At most a handful of windows, and `keys()` puts the primary first.
    windows::keys()
        .into_iter()
        .find_map(|k| windows::peek(k, |w| w.bounds.get(&id).copied()).flatten())
}

/// The rectangle ONE window painted this node at.
///
/// [`bounds_of`] answers "where is it", which is the right question for a
/// single-window app and for `query`/`tree`. But a rectangle is only meaningful
/// in the client space of the window that painted it, and two windows' maps
/// hold numbers that are not comparable — so a question about a POINT ("what is
/// under the pointer in window B") has to name the window, or it will happily
/// return a node from the other one whose x/y happen to overlap.
///
/// `root` is the node a window paints as its root (`0` = the primary window),
/// which is how the devtools protocol addresses a window's coordinate space.
pub fn bounds_in(root: u32, id: u32) -> Option<Rect> {
    let k = windows::window_of_root(root)?;
    windows::peek(k, |w| w.bounds.get(&id).copied()).flatten()
}

/// [`painted_bounds`] for one window — what a per-window highlight overlay
/// positions itself from.
pub fn painted_bounds_in(root: u32, id: u32) -> Option<Rect> {
    let k = windows::window_of_root(root)?;
    windows::peek(k, |w| {
        w.bounds.get(&id).copied().or_else(|| w.prev_bounds.get(&id).copied())
    })
    .flatten()
}

/// Where the app root was painted in the window's client area, in CSS px —
/// `None` before the first painted frame.
///
/// This is **the app's viewport**: everything the host draws around it (§4.2
/// mode 2's fallback titlebar, its padding, its status panel) is UA chrome, and
/// a browser's `innerWidth`/`innerHeight` exclude UA chrome by definition. The
/// window host reads it to publish `window.inner*`, the cascade's `@media`
/// viewport and the `contentInset` metric from ONE number, so JS, CSS and
/// `getBoundingClientRect()` cannot disagree about how big the viewport is.
pub fn root_bounds() -> Option<Rect> {
    root_bounds_in(windows::primary())
}

/// [`root_bounds`] for one window: where THAT window's painted root landed in
/// its own client area. This is the number a per-window `@media` viewport and a
/// `VnWindow`'s `@resize` payload are derived from (docs/MULTIWINDOW.md §6).
pub fn root_bounds_in(k: WinKey) -> Option<Rect> {
    let root = match windows::root_of(k) {
        0 => crate::tree::read_tree(|t| t.root),
        popped => popped,
    };
    windows::peek(k, |w| {
        w.bounds.get(&root).copied().or_else(|| w.prev_bounds.get(&root).copied())
    })
    .flatten()
}

/// Where the node is, asked from INSIDE a build.
///
/// A build clears its window's live map before it walks and prepaint refills it
/// afterwards, so during a build the only rectangles that exist are the
/// previous frame's. Anything the host draws BESIDE the app tree in the same
/// render — the devtools highlight overlay is the only such thing — has to read
/// them from here or it would see nothing at all. Exact for an element at rest,
/// one frame behind one that is moving.
pub fn painted_bounds(id: u32) -> Option<Rect> {
    windows::keys().into_iter().find_map(|k| {
        windows::peek(k, |w| {
            w.bounds.get(&id).copied().or_else(|| w.prev_bounds.get(&id).copied())
        })
        .flatten()
    })
}

/// The rectangle this node was painted at in the frame BEFORE the one being
/// built. Only builders need this — `bounds_of` is the answer to "where is it
/// now", which is what devtools and a future `getBoundingClientRect()` want.
fn prev_size_of(id: u32) -> Option<(f32, f32)> {
    // The window that paints this node is the only one whose previous frame can
    // have a rectangle for it, so this is a direct lookup rather than a search.
    let k = windows::window_of_node(id);
    windows::peek(k, |w| w.prev_bounds.get(&id).copied())
        .flatten()
        .filter(|(_, _, w, h)| *w > 0.0 && *h > 0.0)
        .map(|(_, _, w, h)| (w, h))
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
        record_bounds(self.id, bounds, window);
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
    INPUT_STATES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_REPORTED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    img_src::gc(live);
    crate::lottie::gc(live);
    crate::video::gc(live);
    crate::svg::gc(live);
    // This function runs off a JS WAVE, not off a frame, so it has no `Window`
    // and cannot ask which one it is standing in — one of exactly two such
    // paths (docs/MULTIWINDOW.md §5.6). It iterates every window instead, which
    // is what it already does for every node.
    windows::each(|_, w| {
        w.bounds.retain(|&id, _| live(id));
        w.prev_bounds.retain(|&id, _| live(id));
        w.focus_subs.retain(|&id, _| live(id));
    });
    // The browser rule: **detaching the focused element blurs it.** A node can
    // stay allocated and yet stop being part of the document — `<KeepAlive>`
    // moves a deactivated page into a container that is never inserted, and a
    // build walks from its painted root only, so that subtree is not painted.
    // Liveness is not enough: gpui would keep routing keystrokes to a handle
    // whose element is nowhere in the tree, so a deactivated page's `<input>`
    // kept its focus ring AND kept receiving `keydown` while the characters
    // went nowhere (research/reports/m10/keepalive-state.md §4.1).
    //
    // Per window, because focus is: two windows can each have a focused node,
    // and only one of them may have been detached.
    let mut blur: Vec<(WinKey, u32)> = Vec::new();
    windows::each(|k, w| {
        if w.focused_node != 0 && !live(w.focused_node) {
            w.focused_node = 0;
        } else if w.focused_node != 0 {
            blur.push((k, w.focused_node));
        }
    });
    for (k, focused) in blur {
        if !tree::read_tree(|t| reaches_painted_root(t, focused)) {
            windows::of(k, |w| w.pending_blur = focused);
        }
    }
}

/// Does `id` still hang off SOME painted root — the document root, or a node a
/// pop-out window paints?
///
/// A popped-out node is still parented all the way to `t.root` (D-M05: a
/// pop-out is a PAINT-level detachment, not a tree-level one), so
/// [`reaches_root`] already answers `true` for everything inside one. This
/// wrapper exists to say that out loud at the focus call sites, which are the
/// ones where getting it wrong blurs a pop-out's `<input>` every frame.
fn reaches_painted_root(t: &Tree, id: u32) -> bool {
    reaches_root(t, id)
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
/// **Per window**: each window focuses ITS OWN painted root, not the document
/// root. A pop-out whose fallback focus target lived in the primary window
/// would route every keystroke it received to an element it does not paint.
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
    let k = windows::key(window);
    let pending = windows::of(k, |w| std::mem::replace(&mut w.pending_blur, 0));
    // Re-check: focus may legitimately have moved on since the mutation (the
    // same wave that detached the page can focus something on the new one), and
    // a page can be re-attached before this frame runs.
    if pending == 0 || windows::focused_node(k) != pending {
        return;
    }
    if tree::read_tree(|t| reaches_painted_root(t, pending)) {
        return;
    }
    // gpui first — this is what stops keystrokes reaching the detached node.
    // Since zed#63271 `blur` also drops any half-entered key chord and notifies
    // pending-input observers, which is what we want here: the node that owned
    // the chord is gone.
    window.blur(cx);
    windows::set_focused_node(k, 0);
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
/// (The struct itself lives in [`crate::windows`] — it is one field of
/// `WindowState`, because two windows can be moved over at once and one slot
/// dropped whichever move arrived second.)
///
/// Deliver the frame's coalesced `mousemove` (if any) for ONE window. Called by
/// the host's pump BEFORE the JS pump, so everything the handler schedules
/// (Vue's queue, microtasks) drains in the same frame.
pub fn flush_pending_mouse_move(window: &mut Window, cx: &mut App) {
    let Some(p) = windows::here(window, |w| w.pending_mouse_move.take()) else { return };
    let payload = mouse_payload(p.position, p.button, &p.modifiers);
    js_dispatch_app(p.id, tree::EV_MOUSEMOVE, &payload, window, cx);
}

/// gpui's scroll delta as the DOM's `WheelEvent.deltaX` / `deltaY`.
///
/// **The two conventions are opposite, and that is the whole function.** gpui
/// follows the platform: on Windows `WM_MOUSEWHEEL`'s distance is POSITIVE when
/// the wheel turns away from you (a scroll UP), and gpui carries that sign
/// straight through — its own scroll containers add the delta to an offset that
/// is `<= 0` when content has moved up. The DOM specifies the mirror image:
/// `deltaY` is POSITIVE when scrolling DOWN, because it is "how far the content
/// should move", not "which way the wheel turned".
///
/// Until m21 the raw gpui delta was handed to JS unchanged, so every
/// `@wheel` handler in every app saw an inverted `deltaY` — a hand-rolled
/// scroller built on it scrolled backwards, which is exactly how this was found
/// (the devtools console, research/reports/m21/devtools-v2.md). Native
/// `overflow: auto` containers were never affected: gpui scrolls those itself,
/// with its own sign.
fn dom_wheel_delta(delta: Point<gpui::Pixels>) -> (f64, f64) {
    (-f64::from(delta.x), -f64::from(delta.y))
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
        repaint_other_windows(cx);
    }
    proceed
}

thread_local! {
    /// Installed by the host when more than one window can paint this tree
    /// (docs/MULTIWINDOW.md §4.3). See [`repaint_other_windows`].
    static REPAINT_ALL: Cell<Option<fn(&mut App)>> = const { Cell::new(None) };
}

/// Registers "repaint every window that paints part of this tree".
///
/// The frame pump's own notify is not enough, and the reason is exact: a JS
/// wave does not only start in the pump, it also starts inside an **input
/// listener**, and `flush_changes()` consumes the dirty flag there. So when a
/// click in window B mutates a `ref` that window A's text depends on, the pump
/// that runs next finds nothing dirty and never asks window A to redraw — the
/// tree is right, the pixels are a frame behind forever. This hook is what
/// closes that: a wave that lands in ANY window repaints ALL of them.
pub fn set_repaint_all(f: fn(&mut App)) {
    REPAINT_ALL.with(|c| c.set(Some(f)));
}

fn repaint_other_windows(cx: &mut App) {
    if let Some(f) = REPAINT_ALL.with(|c| c.get()) {
        f(cx);
    }
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
        repaint_other_windows(cx);
    }
    proceed
}

/// DOM order for a focus move: `blur` on the node that had it, then `focus` on
/// the node that got it. Neither event bubbles (the JS runtime knows).
fn focus_transition<V: 'static>(new_focus: u32, window: &mut Window, cx: &mut Context<V>) {
    // Per window (docs/MULTIWINDOW.md §5.2): with one slot, window B focusing
    // an input fired `blur` on window A's — a wrong DOM event, not a wrong
    // pixel. gpui hands the observer the window the focus moved in, so "which
    // window" is not inferred here either.
    let k = windows::key(window);
    let old = windows::focused_node(k);
    if old == new_focus {
        return;
    }
    windows::set_focused_node(k, new_focus);
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
    // **Per window.** `cx.on_focus(handle, window, …)` subscribes against ONE
    // window, so a node whose first build happened in the primary window had no
    // subscription in the pop-out that later painted it — and its `focus` /
    // `blur` events simply never fired there (docs/MULTIWINDOW.md §5.1).
    let k = windows::key(window);
    if windows::of(k, |w| w.focus_subs.contains_key(&id)) {
        return;
    }
    let sub = cx.on_focus(handle, window, move |_, window, cx| {
        focus_transition(id, window, cx);
    });
    windows::of(k, |w| w.focus_subs.insert(id, sub));
    // Focus leaving the window's tree entirely (the focused node was removed,
    // or something unfocusable was clicked) has to blur the last node — in the
    // window it left, which is why this observer is per-window too.
    if windows::of(k, |w| w.focus_lost.is_some()) {
        return;
    }
    let sub = cx.on_focus_lost(window, |_, window, cx| {
        focus_transition(0, window, cx);
    });
    windows::of(k, |w| w.focus_lost = Some(sub));
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

// The chrome trio — `any_drag_region`, `control_hitboxes`, `os_chrome` — are
// fields of `WindowState` (docs/MULTIWINDOW.md §5.2, §7.6). Each is a fact
// about ONE window's caption:
//
// * whether the frame this window just built declared a `window-drag: move`
//   region is what retires THAT window's fallback titlebar;
// * the hitbox list describes the chrome of the window that emitted it, and a
//   pop-out's build used to CLEAR the primary window's;
// * `titleBarStyle` genuinely differs between windows — `<VnWindow chrome>`
//   exists to say so, and the spike's pop-out (a real `WS_CAPTION`) already
//   disagreed with a custom-chrome primary window.

/// Declares whether a window has OS-drawn chrome. Set for the primary window by
/// `vn-app`'s `window::init` before its first frame, and for a pop-out by
/// `popout::open_now` from its `chrome` prop — `titleBarStyle` cannot change at
/// runtime (D-25), so this never flips mid-session for a given window.
pub fn set_os_chrome_in(k: WinKey, on: bool) {
    windows::of(k, |w| w.os_chrome = on);
}

/// [`set_os_chrome_in`] for the primary window, which is what a single-window
/// host means and the only thing that existed before phase 1.
pub fn set_os_chrome(on: bool) {
    set_os_chrome_in(windows::primary(), on);
}

/// `true` if the last frame THIS window painted contained a `window-drag: move`
/// region.
pub fn painted_drag_region_in(k: WinKey) -> bool {
    windows::peek(k, |w| w.any_drag_region).unwrap_or(false)
}

/// [`painted_drag_region_in`] for the primary window.
pub fn painted_drag_region() -> bool {
    painted_drag_region_in(windows::primary())
}

/// The control hitboxes the last frame THIS window painted emitted, in paint
/// order.
///
/// Drag regions are the one part of this surface that a synthesised
/// `ctl.ts click` can never exercise: `WM_NCHITTEST` is a real OS message and
/// the content dispatch path never reaches it. This is the eval-reachable debug
/// hook the acceptance transcript needs instead (docs/WINDOWING.md §13).
pub fn control_hitboxes_in(k: WinKey) -> Vec<(u32, WindowControlArea)> {
    windows::peek(k, |w| w.control_hitboxes.clone()).unwrap_or_default()
}

/// [`control_hitboxes_in`] for the primary window.
pub fn control_hitboxes() -> Vec<(u32, WindowControlArea)> {
    control_hitboxes_in(windows::primary())
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
    win: WinKey,
) -> E {
    let Some(area) = area else { return e };
    // Mode 3: the OS owns the caption. A hitbox here would hand `DefWindowProc`
    // an `HTCAPTION`/`HTMAXBUTTON` answer for a piece of the CLIENT area — the
    // content under it would stop receiving clicks and start dragging the
    // window, which is not what "the OS draws the titlebar" means anywhere
    // else. Inert, once, out loud (A-25).
    if windows::peek(win, |w| w.os_chrome).unwrap_or(false) {
        warn_compat(
            "window-drag/os-chrome",
            "`window-drag` / `--window-control` do nothing under `titleBarStyle: \"default\"` — \
             the OS draws the caption and owns its buttons. Drop the custom titlebar under \
             `@media (display-mode: standalone)`, or switch to `titleBarStyle: \"hidden\"` in \
             vn.config.json — see docs/WINDOWING.md §4.2 mode 3.",
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

/// Builds the current tree root (or nothing) for the hosting view `V`.
///
/// This is the PRIMARY window's entry point; [`build_subtree`] is a pop-out's,
/// and the two do the same per-window bookkeeping against their own window.
pub fn build_root<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    // ONE clock read per frame: nodes animating together must sample the same
    // instant or they skew.
    let now = std::time::Instant::now();
    let span = crate::perf::Span::start();
    BUILT.with(|c| c.set(0));
    let win = windows::key(window);
    // The window that paints `Tree::root` is the primary one by definition, and
    // this is the only place that knows it — declared here rather than passed
    // in from the host so the two can never disagree.
    if !windows::is_primary(win) {
        windows::set_primary(win);
    }
    // This draw's rectangles replace this window's last draw's wholesale — see
    // the bounds side map above. `render()` is the only entry point, so this is
    // the only place that can know a new element tree is being built.
    reset_bounds(win);
    windows::of(win, |w| {
        w.any_drag_region = false;
        w.control_hitboxes.clear();
    });
    let el = tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            // The root inherits nothing: `window-drag`'s initial value is
            // `none`, spelled here as "nobody declared anything".
            build_node(
                t, t.root, t.root, window, cx, now, None, None, None, None, Stacking::default(),
            )
        }
    });
    crate::perf::note_build(span.ms(), BUILT.with(|c| c.get()));
    el
}

/// Builds ONE node's subtree as the root of a *second* window's element tree
/// (docs/MULTIWINDOW.md §4). The node keeps its identity in the retained tree:
/// same id, same listeners, same computed style, same Vue reactivity — only the
/// gpui element tree it is lowered into is a different one.
///
/// Phase 1 makes it a peer of [`build_root`] rather than a stripped-down
/// version of it. All three things the spike had to skip are now done, against
/// THIS window's state instead of the primary's:
///
/// * **the bounds rotation** — per window, on this window's own draw schedule,
///   so a pop-out's `getBoundingClientRect()` is never wiped by a frame the
///   primary window drew and this one did not;
/// * **`control_hitboxes` / `any_drag_region`** — accumulated into this
///   window's slots, which is what makes `chrome: "hidden"` and a pop-out's own
///   custom titlebar work (§7.6);
/// * **the root treatment** — `id` is the root OF THIS BUILD, so it gets
///   `build_node`'s 100 %×100 %, `Overflow::Scroll` and `track_focus` (§7.1).
///   The spike's `PopoutShell` wrapped the subtree in `div().size_full()` to
///   fake the first two and could not fake the third at all, which is why a
///   pop-out could not hold keyboard focus.
pub fn build_subtree<V: 'static>(id: u32, window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    let now = std::time::Instant::now();
    let win = windows::key(window);
    // Registered from the build itself, so "which window paints this node"
    // cannot drift from what is actually on screen — it is what
    // `windows::window_of_node` walks to, and therefore what routes atlas
    // tiles, `@media` environments and hit tests.
    windows::set_painted_root(win, id);
    reset_bounds(win);
    windows::of(win, |w| {
        w.any_drag_region = false;
        w.control_hitboxes.clear();
    });
    tree::read_tree(|t| {
        if t.node(id).is_none() {
            return Empty.into_any_element();
        }
        // A popped-out root inherits nothing — it is a root, like `t.root`.
        build_node(t, id, id, window, cx, now, None, None, None, None, Stacking::default())
    })
}

fn build_node<V: 'static>(
    t: &Tree,
    id: u32,
    // `root` is the root of the build this node is part of: `t.root` in the
    // primary window, the popped-out node in a pop-out. Threaded explicitly
    // rather than looked up, because "is this the root" is a property of the
    // BUILD, not of the tree (docs/MULTIWINDOW.md §7.1).
    root: u32,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    parent_drag: Option<WindowDrag>,
    parent_case: Option<vn_css::TextTransform>,
    parent_ws: Option<vn_css::WhiteSpaceMode>,
    parent_align: Option<gpui::TextAlign>,
    parent_stack: Stacking,
) -> AnyElement {
    let Some(node) = t.node(id) else {
        return Empty.into_any_element();
    };
    BUILT.with(|c| c.set(c.get() + 1));
    // The window this build is lowering into. gpui handed it to us; nothing is
    // inferred (docs/MULTIWINDOW.md §5.6).
    let win = windows::key(window);
    // `window-drag` is inherited (the CSSWG resolved that explicitly) and
    // vn-css has no inherited properties at all, so the fold happens here, on
    // the way down. A browser allows the property on a replaced element too,
    // hence `<img>` and `<lottie>` get the same treatment.
    let extras = extras_of(node);
    // `text-transform` INHERITS, and vn-css folds no inherited property — so
    // the effective value is threaded down here, the same way `window-drag` is.
    // A text node has no declarations of its own, so the parent's value is the
    // only one it can have.
    let case = extras.text_transform.or(parent_case);
    // `white-space` inherits too, and a TEXT NODE has no declarations of its
    // own — so the parent's effective value is the only one it can have. Same
    // threading, same reason, as `text-transform` above.
    let ws = extras.white_space.or(parent_ws);
    // `text-align` inherits too, and it has to be threaded for the same reason
    // `text-transform` does — except that this one is needed by LAYOUT, not by
    // painting: `apply_inline_row` turns a block of mixed inline content into a
    // flex row, and a flex row is justified, not text-aligned. See there.
    let align = node
        .computed
        .as_ref()
        .and_then(|cs| cs.base.text.text_align)
        .or(parent_align);
    // `position: fixed` / `z-index` open a stacking context; the children are
    // built with it so their own z-indices sort INSIDE it (see `Stacking`).
    let (z_priority, stacking) = parent_stack.enter(&extras);
    let (drag, control_area) = window_control_of(&extras, parent_drag);
    advise_interactive_in_drag_region(t, node, &extras, drag);
    advise_double_action(t, node, &extras);
    advise_negative_z(t, id, &extras);
    match node.kind {
        // An empty text node takes no space in a browser, but gpui lays a text
        // element out as a full line box — and Vue anchors every Fragment
        // (`v-for`, multi-root components) with `createText("")`, so leaving it
        // in adds a blank line per fragment.
        Kind::Text if node.text.is_empty() => Empty.into_any_element(),
        Kind::Text => {
            // COLLAPSE first, then trim the block's edges: the two halves of
            // CSS's whitespace processing, in the order the spec applies them.
            let collapsed = processed_text(&node.text, ws.unwrap_or(vn_css::WhiteSpaceMode::Normal));
            match trim_edges(t, node, id, &collapsed, ws) {
                // A text node that is nothing but the white space at the end
                // of a line renders nothing, exactly as in a browser.
                "" => Empty.into_any_element(),
                text => transformed_text(text, case).into_any_element(),
            }
        }
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
            z_priority,
            &extras,
            probe(id, build_img(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element if t.name(node.tag) == "lottie" => stack(
            z_priority,
            &extras,
            probe(id, build_lottie(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element if t.name(node.tag) == "video" => stack(
            z_priority,
            &extras,
            probe(id, build_video(t, id, node, window, cx, now, control_area)),
        ),
        // Inline `<svg>` is a LEAF: the subtree is serialized into a document
        // resvg rasterises, so its children must not also become taffy nodes
        // (see crate::svg).
        Kind::Element if t.name(node.tag) == "svg" => stack(
            z_priority,
            &extras,
            probe(id, build_svg(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element => stack(z_priority, &extras, probe_at(id, id == root, {
            // Every mirrored node gets a stable ElementId — hover/active/
            // scroll state in gpui is keyed by it.
            let mut d = with_control_area(div().id(id as usize), id, control_area, win);
            if let Some(cs) = &node.computed {
                d = apply_computed_style(d, cs, id, now);
            }
            // The other half of `group-hover:`: this element is what some
            // descendant's rule watches, so gpui needs a hitbox for it and a
            // named entry on the group stack while its subtree paints. `.group`
            // is what supplies both.
            if t.is_group_anchor(id) {
                d = d.group(group_name(id));
            }
            // `role` / `aria-*` → AccessKit, so this node exists for Narrator
            // and for UI Automation. Every mirrored element already carries the
            // stable `ElementId` gpui keys an accessibility node on, which is
            // the only precondition — see `crate::aria`.
            d = crate::aria::apply(d, t, id, node);
            advise_missing_font(d.style(), cx);
            apply_background_layers(d.style(), &extras);
            apply_background_clip_text(d.style(), &extras);
            apply_self_blur(d.style(), t, node, &extras);
            apply_inline_row(d.style(), t, node, align);
            apply_inline_align(d.style(), &extras, align);
            apply_percentage_flex_basis(d.style(), t, node);
            // "The window IS the root" (see `createApp().mount()`): the root
            // element is vue-native's viewport, so it fills the box the host
            // gives it unless a rule says otherwise. **The root of THIS build**
            // — `t.root` in the primary window, the popped-out node in a
            // pop-out — because a pop-out's window is that subtree's viewport
            // in exactly the same sense (docs/MULTIWINDOW.md §7.1).
            // Without this it sizes to
            // its CONTENT, and then `height: 100%` on its child resolves
            // against an indefinite parent — which is to say `overflow: auto`
            // never scrolls and `position: fixed` (laid out against the root)
            // lands wherever the content happens to end.
            if id == root {
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
            let bit = move |ev: u32| mask & (1u64 << ev) != 0;
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
                    // `:focus-within` is gpui's `.in_focus()` rung — focus
                    // anywhere in this element's focus subtree, which is what
                    // the pseudo-class means. It needs the same tracked handle
                    // the other two do, which `needs_focus_handle` accounts for.
                    if let Some(f) = cs.focus_within.clone() {
                        d = d.in_focus(move |_| (*f).clone());
                    }
                }
            }
            // Only a browser-focusable node takes focus from a press.
            let click_focus = handle.clone().filter(|_| focusable);
            // `<input>` forwards its own keystrokes from inside the widget (it
            // has to run before gpui resolves the editing key bindings), so the
            // wrapper must not also listen — the editor is its descendant and
            // would deliver every key twice.
            let style_hover = wants_style_hover(t, id, node);
            d = bind_events(d, id, mask, click_focus, focusable, !is_input, style_hover, cx);

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
                let inert = extras.pointer_events_none == Some(true);
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
                // the HTML behavior. Not while the field is inert, though: an
                // `opacity-0 pointer-events-none` panel parked over the visible
                // one is the standard tab-switch idiom, and its fields must not
                // take a press aimed at the ones underneath.
                if !inert {
                    d = d.on_any_mouse_down(cx.listener(
                        move |_, _: &MouseDownEvent, window, cx| {
                            claim_press_focus(id, &focus_target, window, &mut **cx);
                        },
                    ));
                }
                d.child(input).into_any_element()
            } else {
                // `::before` / `::after` are children, first and last — that is
                // what CSS says a generated box IS, and it is why they take
                // part in flex/grid layout, `gap` and text flow exactly like a
                // hand-written `<span>` would.
                let (before, after) = match &node.computed {
                    Some(cs) => (cs.before.clone(), cs.after.clone()),
                    None => (None, None),
                };
                let mut kids: Vec<AnyElement> = Vec::new();
                if let Some(marker) = list_marker_for(t, id, node, &extras) {
                    kids.push(marker);
                }
                if let Some(pb) = before {
                    kids.push(pseudo_element(&pb, case, stacking));
                }
                if !node.text.is_empty() {
                    // `setElementText` — what Vue calls for an element whose
                    // only child is text, which is most of them. The text IS
                    // the element's whole content, so both its ends are the
                    // ends of a line (see `trim_own_edges`). Collapse first,
                    // then trim, exactly as the text-node arm does.
                    let collapsed =
                        processed_text(&node.text, ws.unwrap_or(vn_css::WhiteSpaceMode::Normal));
                    let text = trim_own_edges(t, node, &collapsed, ws);
                    if !text.is_empty() {
                        kids.push(transformed_text(text, case).into_any_element());
                    }
                } else {
                    kids.extend(
                        laid_out_children(t, node)
                            .into_iter()
                            .map(|c| {
                                build_node(
                                    t, c, root, window, cx, now, drag, case, ws, align, stacking,
                                )
                            }),
                    );
                }
                if let Some(pb) = after {
                    kids.push(pseudo_element(&pb, case, stacking));
                }
                d.children(kids).into_any_element()
            }
        })),
    }
}

/// Says so when a `font-family` names a font the host does not have.
///
/// The substitution itself is correct and unavoidable — gpui falls back the way
/// a browser does — but the SILENCE was not, and this is the one gap on
/// apps/meet-probe that changed how much text fitted on a line: the app asks
/// for Inter, ships Inter as a `@font-face` in a stylesheet vue-native never
/// loads (there is no `@font-face` and no `<link rel=stylesheet>`; the sheet is
/// the SFC `<style>` blocks plus Tailwind), and gets the platform UI font,
/// which is wider. The hero subtitle wraps to two lines where a browser fits it
/// on one, and nothing anywhere said why.
///
/// The system font list is enumerated once — it is a platform call over every
/// installed family — and the advisory is one-shot per family name, so a page
/// asking for a missing font on 300 nodes reports it once.
fn advise_missing_font(style: &gpui::StyleRefinement, cx: &mut App) {
    let Some(family) = style.text.font_family.as_ref() else {
        return;
    };
    let known = FONT_NAMES.with(|c| {
        let mut cache = c.borrow_mut();
        cache
            .get_or_insert_with(|| cx.text_system().all_font_names().into_iter().collect())
            .contains(family.as_ref())
    });
    if known {
        return;
    }
    warn_compat(
        &format!("missing-font:{family}"),
        &format!(
            "no font installed for `font-family: {family}` — gpui is falling back to the \
             platform default, which has different metrics, so text measures and wraps \
             differently than in a browser. vue-native loads no `@font-face` and no \
             `<link rel=stylesheet>`: the stylesheet is the SFC <style> blocks plus Tailwind, \
             so a webfont the app ships is never fetched. Install the family on the host, or \
             pick one that is"
        ),
    );
}

/// `background-clip: text` — paint the TEXT, not a box.
///
/// The property cannot be implemented as CSS defines it (gpui rasterises a
/// glyph run in one solid colour, so there is no per-glyph mask to clip a
/// gradient to), but it also cannot be *ignored*, because it never appears
/// alone. The Tailwind idiom is three utilities that only make sense together:
///
/// ```text
/// bg-gradient-to-r from-amber-400 via-yellow-500 to-amber-400
/// bg-clip-text text-transparent
/// ```
///
/// Honouring the gradient and the transparent text while dropping the clip —
/// which is what "ignore it" means — paints a filled gradient RECTANGLE with
/// nothing legible on it. That is how apps/meet-probe's tiny β superscript
/// became a solid yellow bar under the header wordmark, and it is strictly
/// worse than either doing it properly or not doing it at all.
///
/// Taking the three as one gesture instead: drop the background, and give the
/// text the colour that background stands for
/// (`vn_css::Extras::background_key_color` — a gradient's midpoint). Flat
/// instead of ramped, which at the size this idiom is used at is a difference
/// nobody can see, and the advisory in `vn_css::mapper` says so out loud.
///
/// The text colour is overwritten rather than defaulted: `text-transparent` is
/// an explicit `color: transparent` and would otherwise win.
fn apply_background_clip_text(style: &mut gpui::StyleRefinement, extras: &vn_css::Extras) {
    if extras.background_clip_text != Some(true) {
        return;
    }
    if let Some(key) = extras.background_key_color {
        style.background = Some(gpui::Fill::Color(gpui::transparent_black().into()));
        style.background_layers = Some(Vec::new());
        style.text.color = Some(key);
    }
}

/// The `background-image` stack, folded together with `background-size`,
/// `background-position` and `background-repeat` and handed to gpui bottom
/// layer first.
///
/// This runs HERE rather than in the mapper because the four properties are
/// independent and Tailwind puts each in a class of its own — the meet app's
/// page grid is two gradients from `bg-[linear-gradient(…),linear-gradient(…)]`
/// and a tile size from `bg-[size:64px_64px]`, which are two different rules
/// that never see each other. The cascade is what brings them together, so the
/// fold belongs after it.
fn apply_background_layers(style: &mut gpui::StyleRefinement, extras: &vn_css::Extras) {
    if extras.background_images.is_none() {
        return;
    }
    let layers = extras.resolve_background_layers();
    if !layers.is_empty() {
        style.background_layers = Some(layers);
    }
}

/// The HTML elements whose UA `display` is inline-level.
///
/// vue-native has no UA stylesheet — a tag carries no default style, everything
/// comes from the author's sheet — so the one place the default matters,
/// [`apply_inline_row`], carries the list itself. It is deliberately the
/// *text-level semantics* set plus the replaced/inline-block elements that
/// appear inside a sentence, and deliberately NOT `div`, `p`, `h1`-`h6`, `ul`,
/// `li`, `section`, … : a block child of a block genuinely does start a new
/// line, and that is the case this must not touch.
const INLINE_LEVEL_TAGS: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "br", "button", "cite", "code", "data", "dfn", "em", "i",
    "img", "input", "kbd", "label", "mark", "output", "q", "rp", "rt", "ruby", "s", "samp",
    "select", "small", "span", "strong", "sub", "sup", "svg", "textarea", "time", "u", "var",
    "wbr",
];

/// Mixed inline content — a text node next to a `<span>` — laid out as one line
/// instead of a stack.
///
/// **This is the closest thing vue-native has to an inline formatting context,
/// and it is worth being precise about what it is not.** taffy models block,
/// flex and grid; it has no line box, so `display: inline` maps to its
/// block-level equivalent (`vn_css::mapper` says so, with an advisory) and a
/// block container lays every child out on its own row. For a container whose
/// children are ALL elements that is usually invisible, because the author
/// wrote `flex` themselves. For a container that mixes a text node with an
/// inline element it is not invisible at all — it is a line break in the middle
/// of a sentence:
///
/// ```text
/// <h1>Argon <span class="text-primary">Meet</span></h1>
/// <span>Argon Meet <sup>β</sup></span>
/// ```
///
/// rendered as "Argon" over "Meet", and as a β on a line of its own under the
/// wordmark. Both are on apps/meet-probe's landing page; both are the first
/// thing anyone notices next to a browser.
///
/// So: when a block container's children are exactly that shape — at least one
/// text node, at least one element, and every element inline-level — it becomes
/// a wrapping, baseline-aligned flex row, which is what an inline formatting
/// context degrades to when the line box is the one thing you do not have.
/// Text that overflows still wraps *inside* its own run rather than across the
/// element boundary, so a long paragraph with a `<strong>` in the middle breaks
/// in the wrong place; that divergence is real, and much smaller than the one
/// it replaces.
///
/// `text-align` has to be re-expressed as `justify-content`, because a flex row
/// is justified and a flex item is sized to its content — without it every
/// centred heading on the page would go flush left. It is the inherited value
/// that matters (`text-center` is almost always on an ancestor, not on the
/// element itself), which is why `build_node` threads it down.
fn apply_inline_row(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
    align: Option<gpui::TextAlign>,
) {
    // An author-declared display is authority: `display: flex` on a paragraph
    // means flex, and `display: grid` means grid.
    if !matches!(style.display, None | Some(gpui::Display::Block)) {
        return;
    }
    let kids = laid_out_children(t, node);
    if kids.len() < 2 {
        return;
    }
    let mut has_text = false;
    let mut has_element = false;
    for &child in &kids {
        let Some(kid) = t.node(child) else { return };
        match kid.kind {
            Kind::Text => has_text = true,
            Kind::Element => {
                if !INLINE_LEVEL_TAGS.contains(&t.name(kid.tag)) {
                    return;
                }
                // …and the author can still opt a `<span>` out of the run
                // with `display: block`.
                //
                // `display: inline` must NOT count as that, and this is the
                // trap: the mapper folds every INLINE outer type onto its
                // block-level equivalent, because taffy models the inner
                // display type only (`Extras::inline_level` is where the
                // outer type survives). So a `<span style="display:inline">`
                // — the strongest possible statement that it belongs on this
                // line — arrives here as `Some(Block)` and used to opt the
                // whole row out, stacking the spans vertically.
                let declared = kid.computed.as_ref().and_then(|cs| cs.base.display);
                let inline_level = kid
                    .computed
                    .as_ref()
                    .is_some_and(|cs| cs.extras.inline_level == Some(true));
                if !inline_level
                    && matches!(declared, Some(gpui::Display::Block) | Some(gpui::Display::Grid))
                {
                    return;
                }
                has_element = true;
            }
            Kind::Anchor => {}
        }
    }
    // At least one child that is inline content. The loop above already
    // refused anything block-level, so reaching here means every laid-out
    // child belongs on the same line.
    //
    // It used to require BOTH a text child and an element child, which left
    // `<div><span>a</span><span>b</span></div>` — no bare text between the
    // spans — as a block, and taffy stacks a block's children VERTICALLY. In
    // a browser those two spans sit side by side; here the second was drawn
    // under the first, which is why a console line built out of several
    // styled runs came out as a column (research/reports/m21/devtools-v2.md).
    // The same was true of two adjacent text nodes.
    if !(has_text || has_element) {
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

/// A percentage `flex-basis` REPLACES the main-axis size — and when it cannot
/// be resolved, the size does not come back.
///
/// CSS Flexbox §9.2.3 step A takes the flex base size from a *definite* used
/// flex basis. A percentage measured against a container with an indefinite
/// main size is not definite, so the item falls through to step E and is sized
/// by its CONTENT. taffy stops one step short — `flex_basis.or(main_size)` —
/// and reads the item's own `height`/`width` instead, which is exactly the
/// property a non-`auto` flex basis is defined to stand in for.
///
/// reka's Select viewport is precisely that shape: reka writes `flex: 1`
/// inline (i.e. `flex-basis: 0%`) and shadcn adds
/// `h-[--reka-select-trigger-height]`, inside reka's own `display: flex;
/// flex-direction: column` listbox whose height is auto. Measured on the same
/// markup in Edge: the viewport is 103px, its content's height. Ours was 40px,
/// the trigger's — so the list showed its group label and clipped every item.
///
/// Dropping the main-axis size whenever a percentage basis is in force is
/// confined to that case: where the percentage DOES resolve, taffy already took
/// the basis and never looked at the size, so this is a no-op there.
fn apply_percentage_flex_basis(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
) {
    if !matches!(
        style.flex_basis,
        Some(gpui::Length::Definite(gpui::DefiniteLength::Fraction(_)))
    ) {
        return;
    }
    // Only a FLEX ITEM has a flex basis; in any other container the property is
    // inert and the size is the size.
    let Some(parent) = t.node(node.parent) else { return };
    let Some(parent_style) = parent.computed.as_ref().map(|cs| &cs.base) else { return };
    if parent_style.display != Some(gpui::Display::Flex) {
        return;
    }
    match parent_style.flex_direction {
        Some(gpui::FlexDirection::Column) | Some(gpui::FlexDirection::ColumnReverse) => {
            style.size.height = None;
        }
        // `None` is CSS's initial `row`.
        _ => style.size.width = None,
    }
}

/// `text-align` on a block container centres its INLINE-level children.
///
/// taffy has no inline formatting context, so `display: inline-flex` is laid
/// out as `flex` and the box becomes block-level (`vn_css::mapper` says so,
/// with an advisory). Everything about the box is then right except its
/// position: a block-level box ignores `text-align`, so a centred column full
/// of centred text puts its one `inline-flex` badge flush against the left
/// edge. On apps/meet-probe that badge is the hero logo, sitting a third of the
/// page away from the "Argon Meet" it is supposed to sit above.
///
/// Auto margins are the block-layout spelling of the same intent, and taffy
/// implements them: a definite-width block child with `margin-left: auto;
/// margin-right: auto` centres in its containing block, which is what
/// `text-align: center` does to an inline-level box. `get_or_insert` keeps an
/// author's own margin (`mx-4`, `ml-auto`) authoritative, and a box that is
/// auto-width fills the line anyway, so there is no free space to distribute
/// and the margins are a no-op — which is also what a browser does.
fn apply_inline_align(
    style: &mut gpui::StyleRefinement,
    extras: &vn_css::Extras,
    align: Option<gpui::TextAlign>,
) {
    if extras.inline_level != Some(true) {
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

/// `filter: blur(<r>)` on a leaf box with a solid background: repaint the box
/// as its own drop shadow.
///
/// gpui has no post-processing pass over an element's painted output, so
/// `filter` in general is still unimplemented (`vn_css::mapper::apply_filter`
/// advises for every other function). This one shape is different, and it is
/// worth the special case twice over:
///
/// * it is the whole visual budget of a modern landing page's background — the
///   ambient "glow blob" (`absolute w-[40rem] h-[40rem] bg-primary/5
///   rounded-full blur-3xl`), which without a blur paints as a hard-edged disc
///   several hundred pixels across. On apps/meet-probe that single missing
///   filter was the loudest difference from the browser by a wide margin: three
///   flat red discs where the browser has a soft vignette;
/// * and it is not an approximation. gpui's drop shadow primitive rasterises a
///   Gaussian-blurred rounded rect with the element's own corner radii, which
///   is *exactly* what blurring a solid rounded rect produces. The radii carry
///   over, so `rounded-full` blurs as a circle.
///
/// Two details that are easy to get wrong:
///
/// * gpui's `blur_radius` is the Gaussian's **sigma** (`shaders.hlsl`:
///   `gaussian(y, shadow.blur_radius)`), and CSS's `blur(<r>)` is *also*
///   defined as a sigma, so the radius transfers 1:1 with no conversion.
/// * a drop shadow is painted with the border box knocked out per CSS, but
///   gpui's shadow shader only masks for `inset` shadows — a drop shadow is a
///   filled blurred rect all the way through. That is a divergence everywhere
///   else; here it is precisely the behaviour we need, and it is why the
///   element's own background has to be cleared rather than left underneath.
///
/// Anything else keeps its sharp edges and gets told why: a blurred box with
/// children would need the subtree rasterised, and a blurred gradient would
/// need the gradient rasterised, and neither is expressible without the
/// post-processing pass that does not exist.
fn apply_self_blur(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
    extras: &vn_css::Extras,
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
        }
        _ => warn_compat(
            "filter-blur-shape",
            "`filter: blur()` is honoured only on a LEAF box with a solid background colour \
             (v1) — gpui has no post-processing pass over an element's own painted output, so \
             the blur is drawn by repainting that box as its own Gaussian drop shadow, which \
             is exact for a solid rounded rect and impossible for anything with children, text \
             or a gradient. This element kept its sharp edges",
        ),
    }
}

/// One `::before` / `::after` generated box.
///
/// A plain `div` carrying the pseudo's folded declarations and its `content`
/// string, built at the position CSS puts it: first or last child of the
/// originating element. It gets no id, no hitbox and no listeners — it is not a
/// node in the retained tree, `querySelector` cannot find it and events do not
/// target it, which is exactly a browser's rule for generated content.
///
/// One deliberate divergence, stated because it is invisible otherwise: gpui's
/// text shaper needs a leaf, so an EMPTY `content: ""` box (the decorative
/// case: a rule, a dot, a chevron drawn entirely out of size + background +
/// border) carries no child at all and is sized purely by its own style.
fn pseudo_element(
    pb: &vn_css::PseudoBox,
    parent_case: Option<vn_css::TextTransform>,
    stacking: Stacking,
) -> AnyElement {
    let mut d = div();
    *d.style() = (*pb.style).clone();
    let case = pb.extras.text_transform.or(parent_case);
    let e = if pb.content.is_empty() {
        d.into_any_element()
    } else {
        d.child(transformed_text(&pb.content, case)).into_any_element()
    };
    stack(stacking.enter(&pb.extras).0, &pb.extras, e)
}

/// The marker box an `<li>` gets from the `list-style-type` in force.
///
/// `list-style-type` INHERITS and vn-css folds no inherited property, so the
/// effective value is the nearest ancestor that declared one — which is what
/// the walk below finds, and which is where an author writes it anyway
/// (`<ul class="list-disc">`). Returning `None` covers all three quiet cases:
/// not a list item, nothing declared anywhere, and an explicit
/// `list-style: none` (the reset every UI framework ships).
///
/// The marker is a real child rather than a painted decoration, for the same
/// reason `::before` is: it then takes part in layout, inherits the item's text
/// style and needs nothing new from gpui. `outside` (the initial value) hangs
/// it into the padding with a negative margin, which is what a browser's marker
/// box does; `inside` leaves it in flow as the item's first inline content.
fn list_marker_for(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    extras: &vn_css::Extras,
) -> Option<AnyElement> {
    if t.name(node.tag) != "li" {
        return None;
    }
    let (marker, inside) = {
        let mut marker = extras.list_marker;
        let mut inside = extras.list_marker_inside;
        let mut cursor = node.parent;
        while marker.is_none() || inside.is_none() {
            let Some(ancestor) = t.node(cursor) else { break };
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
    // The item's ordinal is its position among its ELEMENT siblings, which is
    // the number `counter(list-item)` produces and the number a browser's
    // `decimal` marker shows.
    let ordinal = t
        .node(node.parent)
        .map(|parent| {
            parent
                .children
                .iter()
                .filter(|&&c| t.node(c).is_some_and(|n| n.kind == crate::tree::Kind::Element))
                .position(|&c| c == id)
                .unwrap_or(0)
                + 1
        })
        .unwrap_or(1);
    let text = marker.render(ordinal)?;

    let mut d = div();
    let style = d.style();
    style.flex_shrink = Some(0.0);
    // A browser sizes the marker box to the marker plus a gap; one and a quarter
    // ems is the width every UA sheet lands on for the bullet styles.
    style.size.width = Some(gpui::Length::Definite(gpui::rems(1.25).into()));
    style.text.text_align = Some(gpui::TextAlign::Right);
    style.padding.right = Some(gpui::rems(0.35).into());
    if !inside {
        // `outside`: the marker sits in the item's padding area rather than
        // taking width from the content.
        style.margin.left = Some(gpui::rems(-1.25).into());
    }
    Some(d.child(SharedString::from(text)).into_any_element())
}

/// A text leaf with `text-transform` applied.
///
/// gpui's `TextStyle` has no case field — the shaper is handed a string — so
/// the transform is applied to the string, which is also the only place it
/// could be applied without a second shaping pass.
fn transformed_text(text: &str, case: Option<vn_css::TextTransform>) -> SharedString {
    match case.and_then(|c| c.apply(text)) {
        Some(changed) => SharedString::from(changed),
        None => SharedString::from(text.to_string()),
    }
}

/// White space at the START and END of a line is removed (CSS Text §4.1.1
/// step 4) — the half of white-space processing that is visible in a width.
///
/// Vue's template compiler already condenses runs of white space, so what
/// reaches the tree is a single leading and/or trailing space; what it does NOT
/// do is remove them, because in a browser that is the renderer's job. Ours
/// shaped them, and a space at each end of Inter at 15.75px is 9.3px — which on
/// apps/meet-probe is the difference between the hero subtitle fitting its
/// `max-w-sm` (336px) on one line, where the browser puts it, and wrapping onto
/// two.
///
/// A text node's ends are a line's ends when nothing inline sits beside them,
/// which here means: it is the first (or last) child that generates a box, and
/// its parent is not itself an inline-level box. The second half is what keeps
/// `a<span> </span>b` intact — the span's only child is a space a browser DOES
/// render, because it is in the middle of the line rather than at its end.
///
/// Returned as a slice of the node's own text: nothing is allocated in the
/// overwhelming majority of cases, where there is nothing to trim.
/// CSS whitespace processing, applied to a text node's string.
///
/// ## Why this is here and not in the engine
///
/// gpui shapes whatever string it is handed and breaks a line at every `\n` in
/// it, unconditionally. CSS does not work that way: under `white-space: normal`
/// (the default) and `nowrap`, a run of spaces, tabs and newlines COLLAPSES to
/// a single space, and a source newline is not a line break at all. Only the
/// `pre*` family preserves them.
///
/// Until m21 nothing did this. `trim_edges` below trimmed the white space at
/// the EDGES of a block — which is the other half of the rule and the visible
/// one, so the gap went unnoticed — while interior runs went to gpui untouched.
/// The result: any text node holding a newline rendered as several lines where
/// a browser renders one, inside a box laid out for one. On a fixed-height row
/// that is glyphs drawn on top of each other, which is how it was finally
/// found — every `console.log("%cx", "…multi-line CSS…")` in the devtools
/// console (research/reports/m21/devtools-v2.md).
///
/// ## What each mode does
///
/// * `normal` / `nowrap` — every run of `[ \t\n\r\x0c]` becomes one space.
/// * `pre-line` — runs of spaces and tabs become one space; newlines survive,
///   and white space around a newline is absorbed into it.
/// * `pre` / `pre-wrap` / `break-spaces` — the string is passed through whole.
///
/// Returns a `Cow` so the overwhelmingly common case — text with no run of two
/// collapsible characters in it — keeps borrowing the tree's own string.
fn processed_text(raw: &str, mode: vn_css::WhiteSpaceMode) -> std::borrow::Cow<'_, str> {
    const COLLAPSIBLE: [char; 5] = [' ', '\t', '\n', '\r', '\u{c}'];
    let collapses = mode.collapses_spaces();
    let keeps_newlines = mode.preserves_newlines();
    if !collapses {
        return std::borrow::Cow::Borrowed(raw);
    }
    // What would change? A run of two collapsible characters, or a lone
    // collapsible character that is not already a space (a tab, or — unless
    // they are preserved — a newline).
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
            out.push(if keeps_newlines && saw_newline { '\n' } else { ' ' });
        }
        out.push(c);
    }
    // A trailing run is still a space (or a break): the EDGE trimming that
    // follows is what removes it when the node is last in its block, and a run
    // between two inline siblings has to survive as the word gap it is.
    if let Some(saw_newline) = run {
        out.push(if keeps_newlines && saw_newline { '\n' } else { ' ' });
    }
    std::borrow::Cow::Owned(out)
}

fn trim_edges<'a>(
    t: &Tree,
    node: &crate::tree::Node,
    id: u32,
    text: &'a str,
    mode: Option<vn_css::WhiteSpaceMode>,
) -> &'a str {
    const COLLAPSIBLE: [char; 4] = [' ', '\t', '\n', '\r'];
    // `pre*` preserves the edges too — the trim is the second half of the
    // same collapsing rule, so it applies exactly where the first half does.
    if !mode.unwrap_or(vn_css::WhiteSpaceMode::Normal).collapses_spaces() {
        return text;
    }
    let Some(parent) = t.node(node.parent) else { return text };
    if parent.kind == Kind::Element && INLINE_LEVEL_TAGS.contains(&t.name(parent.tag)) {
        return text;
    }
    let mut boxes = parent.children.iter().copied().filter(|&c| generates_a_box(t, c));
    let first = boxes.next();
    // `.last()` on what remains, so a lone child answers `first == last`.
    let last = boxes.last().or(first);
    let text = if first == Some(id) { text.trim_start_matches(COLLAPSIBLE) } else { text };
    if last == Some(id) { text.trim_end_matches(COLLAPSIBLE) } else { text }
}

/// [`trim_edges`] for the text an ELEMENT carries itself.
///
/// Vue calls `setElementText` for an element whose only child is text — which
/// is most elements, and the shape the hero subtitle takes — so this, not the
/// text-node path, is where the trim usually has to happen. The text is the
/// element's entire content, so both ends are line ends; the exception is an
/// inline-level element sharing a line with something else, where an edge space
/// separates words rather than padding a line.
fn trim_own_edges<'a>(
    t: &Tree,
    node: &crate::tree::Node,
    text: &'a str,
    mode: Option<vn_css::WhiteSpaceMode>,
) -> &'a str {
    const COLLAPSIBLE: [char; 4] = [' ', '\t', '\n', '\r'];
    if !mode.unwrap_or(vn_css::WhiteSpaceMode::Normal).collapses_spaces() {
        return text;
    }
    if INLINE_LEVEL_TAGS.contains(&t.name(node.tag))
        && t.node(node.parent).is_some_and(|parent| {
            parent.children.iter().filter(|&&c| generates_a_box(t, c)).count() > 1
        })
    {
        return text;
    }
    text.trim_matches(COLLAPSIBLE)
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
    // Popped out into another OS window: still in the tree, still reactive,
    // simply not this window's box to lay out (docs/MULTIWINDOW.md §4).
    if popped_out(id) {
        return false;
    }
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
fn laid_out_children(t: &Tree, node: &crate::tree::Node) -> Vec<u32> {
    let mut kids: Vec<u32> = node
        .children
        .iter()
        .copied()
        .filter(|&c| generates_a_box(t, c))
        .collect();
    // Negative `z-index`, for the case where it is expressible.
    //
    // CSS 2.1 Appendix E paints a negative-z child AFTER its parent's own
    // background and border and BEFORE the parent's in-flow content — it goes
    // behind its SIBLINGS, not behind its parent. gpui paints a `Div`'s
    // children in the order they were added, so moving such a child to the
    // front IS that rule, exactly.
    //
    // The catch is that the same order is taffy's, so reordering an IN-FLOW
    // child would move its box. An out-of-flow child (`position: absolute` or
    // `fixed`) is laid out against the container regardless of where it sits
    // among its siblings, so for those — which is every real use of a negative
    // z-index: a decorative layer under the content — the reorder is free.
    // `advise_negative_z` covers the in-flow case, which stays unexpressible.
    //
    // `sort_by_key` is stable, so ties keep document order, which is what CSS
    // says happens within a z-level.
    if kids.iter().any(|&c| paint_first_z(t, c).is_some()) {
        kids.sort_by_key(|&c| paint_first_z(t, c).unwrap_or(0));
    }
    kids
}

/// The negative `z-index` of an out-of-flow child, i.e. one that can be moved
/// to the front of the paint order without moving its box. `None` for
/// everything else, which sorts as 0 and keeps its place.
fn paint_first_z(t: &Tree, id: u32) -> Option<i32> {
    let node = t.node(id)?;
    let computed = node.computed.as_ref()?;
    let z = computed.extras.z_index?;
    if z >= 0 {
        return None;
    }
    let out_of_flow = computed.extras.fixed == Some(true)
        || matches!(computed.base.position, Some(gpui::Position::Absolute));
    out_of_flow.then_some(z)
}

/// A negative `z-index` on an IN-FLOW element — the half that has no lever.
fn advise_negative_z(t: &Tree, id: u32, extras: &vn_css::Extras) {
    if extras.z_index.is_none_or(|z| z >= 0) || paint_first_z(t, id).is_some() {
        return;
    }
    warn_compat(
        "z-index-negative-in-flow",
        "a negative `z-index` on an IN-FLOW element cannot paint below its siblings (v1) —          gpui paints a container's children in the order they were added, which is also the          order taffy lays them out in, so moving one would move its box. It works on an          out-of-flow element (`position: absolute` / `fixed`), which is what a decorative          layer under the content should be; otherwise reorder the elements",
    );
}

/// HTML metadata content: elements that carry instructions, not a box. A
/// browser gives all of these `display: none` from its UA stylesheet.
fn is_metadata(tag: &str) -> bool {
    matches!(tag, "style" | "script" | "template" | "head" | "meta" | "link" | "title" | "base")
}

/// The computed [`vn_css::Extras`] of a node (the default before the first
/// cascade, which is also what an unstyled node computes to).
fn extras_of(node: &crate::tree::Node) -> vn_css::Extras {
    node.computed.as_ref().map(|cs| cs.extras.clone()).unwrap_or_default()
}

/// The bits `pointer-events: none` turns off — the pointer half of the event
/// set. Keyboard and focus listeners are deliberately NOT in it: in a browser
/// `pointer-events: none` makes an element transparent to the POINTER only, and
/// a `tabindex`ed one still focuses and still receives keystrokes.
const POINTER_BITS: u64 = (1 << tree::EV_CLICK)
    | (1 << tree::EV_MOUSEDOWN)
    | (1 << tree::EV_MOUSEUP)
    | (1 << tree::EV_MOUSEMOVE)
    | (1 << tree::EV_MOUSEENTER)
    | (1 << tree::EV_MOUSELEAVE)
    | (1 << tree::EV_WHEEL);

/// The node's listener mask with `pointer-events: none` applied. Together with
/// the hover/active rungs being skipped in [`apply_computed_style`], this is
/// what stops gpui giving the node a hitbox at all.
fn pointer_mask(node: &crate::tree::Node) -> u64 {
    // …except on the ROOT. `document.body` is the root here, and the whole
    // dismissable-layer idiom (reka, radix) sets `body.style.pointerEvents =
    // "none"` while a modal is open — which in a browser costs nothing, because
    // the body still dispatches keydown. Here the root IS the keystroke path
    // (see `build_node`'s root arm), so honouring it would silently kill
    // Escape-to-dismiss for exactly the layer that asked for it.
    if node.parent == 0 {
        return node.listeners;
    }
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

/// Does this node's pointer state have to reach the animator?
///
/// True exactly when [`apply_computed_style`] took something out of its hover
/// rungs — the two MUST agree, or a stripped property would have no one left to
/// paint it. `pointer-events: none` is the case where no rung is installed at
/// all, so there is nothing to strip and nothing to observe.
fn wants_style_hover(t: &Tree, id: u32, node: &crate::tree::Node) -> bool {
    let own = node.computed.as_ref().is_some_and(|cs| {
        cs.extras.pointer_events_none != Some(true) && cs.hover_transitions()
    });
    // …or because this element is the `.group` some DESCENDANT watches. The
    // descendant has no hitbox of the ancestor and gpui gives it no edge of its
    // own, so the anchor's flip is the only signal it will ever get — even
    // though the anchor itself may have neither a hover rung nor a transition.
    // Missing this is what left `group-hover:` elements stuck on their base
    // values once the rung was stripped.
    own || t.has_group_hover_deps(id)
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
    // `pointer-events: none` means the node is not interactive at all — so it
    // must not carry gpui's hover/active rungs either, which are exactly what
    // would make gpui insert a hitbox for it.
    if cs.extras.pointer_events_none != Some(true) {
        // The properties the author named in `transition` come OUT of the
        // hover rungs: gpui's ladder has no clock and would refine them in
        // whole the instant the pointer crosses the hitbox, leaving the
        // animator nothing to interpolate. `tree::note_hover` feeds them to
        // the animator instead, and its overlay (refined in above) is what
        // paints them. Everything else stays in the rung and still snaps —
        // which is exactly what CSS does with an untransitioned property.
        let split = |rung: &std::rc::Rc<gpui::StyleRefinement>| {
            vn_css::strip_transitioned(rung, &cs.transitions)
                .map(std::rc::Rc::new)
                .unwrap_or_else(|| rung.clone())
        };
        if let Some(h) = cs.hover.as_ref().map(split) {
            e = e.hover(move |_| (*h).clone());
        }
        if let Some(a) = cs.active.clone() {
            e = e.active(move |_| (*a).clone());
        }
        // `.group:hover .this` — gpui's own group mechanism, keyed by the
        // ancestor's node id so the name is unique by construction and the
        // `GroupHitboxes` stack resolves to exactly the element the cascade
        // picked. The style is refined at PAINT time from that hitbox: an
        // ancestor's hover flip costs a repaint, never a restyle.
        if let Some((anchor, style)) = cs.group_hover.as_ref().map(|(a, s)| (*a, split(s))) {
            e = e.group_hover(group_name(anchor as u32), move |_| (*style).clone());
        }
        if let Some((anchor, style)) = cs.group_active.clone() {
            e = e.group_active(group_name(anchor as u32), move |_| (*style).clone());
        }
    }
    e
}

/// Focus this node on a press, and CLAIM the press so no ancestor re-focuses
/// itself over the top of it.
///
/// The claim is the whole point. gpui gives every element with a tracked focus
/// handle an automatic mouse-down listener that focuses it and then calls
/// `prevent_default()`, precisely so a focusable ancestor backs off — and the
/// document root is such an ancestor here: it tracks a focus handle so that
/// `document.addEventListener("keydown")` (Escape-to-dismiss, every keyboard
/// shortcut in the ecosystem) has something on the focus path. Our own
/// focus-follows-press listeners did not make the same claim, so a press on an
/// `<input>` focused the editor and then the root's automatic listener focused
/// the root instead: the caret vanished and the keystrokes went to `document`.
///
/// One log line per real focus move, at `dev` kind — this is the exact failure
/// the meet app's "typing does nothing" turned out to be, and it is invisible
/// from JS (`focus` events do not fire while the window is inactive: gpui
/// reports empty focus paths then).
fn claim_press_focus(id: u32, handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    // Somebody nearer the pointer already took it. Mouse listeners run
    // TOPMOST-FIRST, so this is the ancestor half of the same gesture — and
    // without the check the outermost focusable wins every press, which on this
    // tree is always the document root.
    if window.default_prevented() {
        return;
    }
    if !handle.is_focused(window) {
        window.focus(handle, cx);
        crate::log::record("dev", format!("[focus] press focused node {id}"));
    }
    window.prevent_default();
}

/// The gpui group name for an element: its node id, which is unique in the
/// document and stable for as long as the node is. Two different `.group`
/// ancestors can therefore never collide, and a nested one shadows its outer
/// namesake for free — `GroupHitboxes` is a stack and `get` returns its top.
fn group_name(id: u32) -> gpui::SharedString {
    format!("vn-g{id}").into()
}

/// The enclosing STACKING CONTEXTS, packed into the one `usize` gpui sorts
/// deferred draws by.
///
/// A `z-index` is not a document-wide rank: CSS says it orders an element among
/// its siblings *inside its stacking context*, and the whole subtree of a
/// context paints together, between the contexts on either side of it. gpui has
/// one flat priority, so a naive `priority = z` inverts every nested case — and
/// on apps/meet-probe it inverted the loudest one there is. reka's dialog is
/// `fixed z-50`, and the header inside it is `sticky top-0 bg-background z-10`:
/// 10 sorts before 50, so the dialog's own background painted OVER its title
/// bar and the dialog came up as a black slab.
///
/// So the priority is a fixed-width digit string instead — one digit per
/// nesting level, most significant first — which compares exactly the way CSS
/// orders the tree: an element sorts after the context that contains it, and
/// before anything that outranks that context.
#[derive(Clone, Copy, Default)]
struct Stacking {
    prefix: usize,
    depth: u32,
}

/// Bits per level. 0..=16383 covers every `z-index` a real sheet writes (the
/// `z-9999` idiom included) and leaves room for four levels in a `usize`.
const Z_BITS: u32 = 14;
const Z_MAX: i32 = (1 << Z_BITS) - 1;
/// How many nested stacking contexts stay distinguishable. A fifth would not
/// fit; beyond it a descendant simply shares its context's priority, which
/// still keeps it above everything the context outranks.
const Z_LEVELS: u32 = 4;

impl Stacking {
    /// This node's own deferred priority (`None` = paints in flow), and the
    /// context its children see.
    fn enter(self, extras: &vn_css::Extras) -> (Option<usize>, Stacking) {
        let Some(z) = extras.deferred_priority() else {
            return (None, self);
        };
        if self.depth >= Z_LEVELS {
            warn_compat(
                "z-index-nesting-depth",
                "more than four nested stacking contexts (`position: fixed` / `z-index`) — \
                 beyond the fourth a descendant shares its context's paint priority instead of \
                 sorting inside it. It still paints above everything that context outranks",
            );
            return (Some(self.prefix), self);
        }
        let shift = Z_BITS * (Z_LEVELS - 1 - self.depth);
        let priority = self.prefix | ((z.min(Z_MAX as usize)) << shift);
        (Some(priority), Stacking { prefix: priority, depth: self.depth + 1 })
    }
}

/// Wraps a node that has to escape its siblings' paint order — `position:
/// fixed`, or a positive `z-index` — in gpui's deferred layer.
///
/// `Deferred` keeps the child's LAYOUT where it is (it returns the child's own
/// `LayoutId`) and only moves the PAINT to after every ancestor, ordered by
/// priority; gpui sorts those stably, so two elements with the same `z-index`
/// keep document order, exactly as CSS says they should.
fn stack(priority: Option<usize>, extras: &vn_css::Extras, el: AnyElement) -> AnyElement {
    match priority {
        Some(priority) => gpui::deferred(el)
            .with_priority(priority)
            // A `z-index`ed element escapes its siblings' PAINT ORDER, not its
            // ancestors' `overflow` — CSS clips it to the scroll container it is
            // laid out inside. `position: fixed` is the exception CSS makes, and
            // it is also the one shape where our own divergence (laid out
            // against the parent, not the viewport) would turn a clip into a
            // disappearance, so it keeps the escape hatch.
            //
            // Without this the meet app's `<main class="relative z-10">` painted
            // the part of a too-tall page that overflows the viewport OUTSIDE
            // the viewport — over the host's own chrome.
            .clip_to_ancestors(extras.fixed != Some(true))
            .into_any_element(),
        None => el,
    }
}

/// The listener half every element shares. `keys_here` is false for `<input>`,
/// whose editor forwards its own keystrokes from inside the widget.
#[allow(clippy::too_many_arguments)]
fn bind_events<E, V>(
    mut e: E,
    id: u32,
    mask: u64,
    click_focus: Option<FocusHandle>,
    focusable: bool,
    keys_here: bool,
    style_hover: bool,
    cx: &mut Context<V>,
) -> E
where
    E: gpui::StatefulInteractiveElement + 'static,
    V: 'static,
{
    let bit = move |ev: u32| mask & (1u64 << ev) != 0;
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
                claim_press_focus(id, h, window, &mut **cx);
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
        e = e.on_mouse_move(cx.listener(move |_, ev: &gpui::MouseMoveEvent, window, _cx| {
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
        }));
    }
    // One `on_hover` per element — gpui's slot is single (`div.rs`'s
    // `debug_assert!(self.hover_listener.is_none())`), so the JS events and the
    // CSS state transitions have to share it. `style_hover` is set when the
    // cascade gave this node a hover rung AND a `transition` to run it on; see
    // [`tree::note_hover`].
    if style_hover || bit(tree::EV_MOUSEENTER) || bit(tree::EV_MOUSELEAVE) {
        e = e.on_hover(cx.listener(move |_, hovered: &bool, window, cx| {
            if style_hover {
                // Before the JS handler: a `mouseenter` listener that restyles
                // must see the transition already aimed at its target, or its
                // restyle would re-aim it at the OLD fold.
                tree::note_hover(id, *hovered);
            }
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
            let (dx, dy) = dom_wheel_delta(ev.delta.pixel_delta(gpui::px(16.0)));
            let payload = EventPayload {
                a: dx,
                b: dy,
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

/// Which node owns keyboard focus, by asking gpui. See
/// [`crate::devtools::focused_node`] for why this is not simply `FOCUSED_NODE`.
pub(crate) fn focused_node(window: &Window, cx: &App) -> Option<u32> {
    // The editor's handle first: an `<input>`'s focus identity lives inside the
    // native widget, and its wrapper may ALSO have a handle (a `:focus` rule).
    let from_inputs = INPUT_STATES.with(|m| {
        m.borrow()
            .iter()
            .find(|(_, state)| state.read(cx).focus_handle().is_focused(window))
            .map(|(id, _)| *id)
    });
    from_inputs.or_else(|| {
        FOCUS_HANDLES.with(|m| {
            m.borrow().iter().find(|(_, h)| h.is_focused(window)).map(|(id, _)| *id)
        })
    })
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
    let win = windows::key(window);
    let src = t.prop(id, "src").unwrap_or("").to_string();
    let alt = t.prop(id, "alt").unwrap_or("").to_string();

    let Some(source) = img_src::source_for(id, &src) else {
        // Nothing loadable at all (empty/unsupported `src` — already advised).
        // A browser still lays the element out and paints the alt text, so the
        // node keeps its box and its styles.
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
    let bit = |ev: u32| mask & (1u64 << ev) != 0;
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
    let style_hover = wants_style_hover(t, id, node);
    bind_events(e, id, mask, click_focus, focusable, true, style_hover, cx).into_any_element()
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
    let win = windows::key(window);
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

    // No `with_transformation` here any more. `transform` is a paint-level
    // property on the style now (m13 item 2), so `Style::paint` pushes the
    // matrix for the whole element and `Window::paint_svg` composes it into the
    // sprite. Setting it here as well would apply it twice.
    let mut e = with_control_area(gpui::svg().data(&doc).id(id as usize), id, control_area, win);
    e = crate::aria::apply(e, t, id, node);
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
    let style_hover = wants_style_hover(t, id, node);
    let e = bind_events(e, id, mask, click_focus, focusable, true, style_hover, cx);
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
    let win = windows::key(window);
    // Load (once per `src`) and push this frame's props into the transport.
    let intrinsic = crate::lottie::sync(t, id, win);

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
    let bit = |ev: u32| mask & (1u64 << ev) != 0;
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
    let style_hover = wants_style_hover(t, id, node);
    bind_events(d, id, mask, click_focus, focusable, true, style_hover, cx).into_any_element()
}

// ---------------------------------------------------------------------------
// <video>
// ---------------------------------------------------------------------------

fn video_fit(fit: vn_css::ObjectFit) -> vn_video::VideoFit {
    match fit {
        vn_css::ObjectFit::Fill => vn_video::VideoFit::Fill,
        vn_css::ObjectFit::Contain => vn_video::VideoFit::Contain,
        vn_css::ObjectFit::Cover => vn_video::VideoFit::Cover,
        vn_css::ObjectFit::ScaleDown => vn_video::VideoFit::ScaleDown,
        vn_css::ObjectFit::None => vn_video::VideoFit::None,
    }
}

/// `<video src="assets/clip.mp4" autoplay loop muted @timeupdate="…" />` — a
/// replaced element whose content is the frame the pump last pulled out of the
/// OS media pipeline.
///
/// Structurally it is a styled box (so the cascade, `background`, borders,
/// `border-radius`, `overflow: hidden` and the event listeners all work exactly
/// as on a `<div>`) wrapping the crate's `size_full()` image. Keeping it INSIDE
/// the scene graph like this is why we use Media Foundation's frame-server mode
/// rather than handing it a DirectComposition visual of its own: a video on a
/// sibling visual cannot be clipped, rounded, faded or z-interleaved with CSS
/// content.
///
/// The box follows the `<lottie>` / `<img>` rule exactly: CSS `width`/`height`
/// win outright; with neither, the box takes the video's intrinsic size; with
/// one, the other follows from the intrinsic aspect ratio.
///
/// **`poster` is what paints before the first frame** — and it is a real
/// `<img>`, resolved through the same `src` grammar, so a poster and an `<img>`
/// with the same path share gpui's decoded-image cache.
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
    // Load (once per `src`) and push this frame's props into the transport.
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
        // NOT the CSS initial value (`fill`): HTML says a video is "shown
        // centred in the playback area at the largest possible size that fits
        // completely within it, with the aspect ratio preserved".
        .unwrap_or(vn_video::VideoFit::Contain);

    // The box, and with it the size the next tick decodes at.
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

        let (fill_w, fill_h) = match (sized_w, sized_h) {
            (true, true) => (None, None),
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
        painted_box = painted.or(Some((
            known_w.or(fill_w).unwrap_or(iw),
            known_h.or(fill_h).unwrap_or(ih),
        )));
    }

    // Publish the box the next tick decodes at. This is why the present path is
    // affordable at all: a 1080p clip in an 800x450 box reads back 1.4 MB a
    // frame instead of 8.3 (crates/vn-video/src/mf/present.rs).
    let options = vn_video::VideoOptions {
        size: painted_box.unwrap_or((0.0, 0.0)),
        scale: window.scale_factor(),
        object_fit,
    };
    crate::video::note_frame_box(id, &options);

    let has_frame = crate::video::with_state(id, |s| s.frame().is_some()).unwrap_or(false);
    if has_frame {
        if let Some(child) =
            crate::video::with_state(id, |s| vn_video::video(s, &options, window, &mut **cx))
        {
            d = d.child(child);
        }
    } else if let Some(poster) = t.prop(id, "poster").filter(|p| !p.trim().is_empty()) {
        // Before `loadeddata` — and forever, for a `src` that failed — the
        // poster is what a browser paints.
        if let Some(source) = img_src::source_for(id, poster.trim()) {
            d = d.child(
                gpui::img(source)
                    .object_fit(gpui_object_fit(
                        node.computed
                            .as_ref()
                            .and_then(|cs| cs.extras.object_fit)
                            .unwrap_or(vn_css::ObjectFit::Contain),
                    ))
                    .size_full()
                    .into_any_element(),
            );
        }
    }

    let mask = pointer_mask(node);
    let bit = |ev: u32| mask & (1u64 << ev) != 0;
    // Like `<img>` and `<lottie>`: not focusable in HTML without `tabindex`.
    // (A browser's `<video controls>` IS focusable, but we render no controls —
    // the app's own buttons are the focusable things.)
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
    let style_hover = wants_style_hover(t, id, node);
    bind_events(d, id, mask, click_focus, focusable, true, style_hover, cx).into_any_element()
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
    // `::placeholder { color: … }` if the cascade gave the input one, else the
    // faded text colour every browser's UA sheet approximates. The pseudo can
    // also change the placeholder's SIZE and WEIGHT in a browser; the widget
    // paints one run with one font, so only the colour reaches it.
    let placeholder_color = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.placeholder.as_ref())
        .and_then(|ph| ph.text.color)
        .unwrap_or(gpui::Hsla { a: text_color.a * 0.45, ..text_color });
    // `::selection { background: … }`. The field's selection is the ONE
    // selection that exists in vue-native — gpui paints ordinary text and has
    // no selection model for it — so this is where the pseudo-element lands.
    // Its `color` cannot: gpui's selection is a quad painted BEHIND an already
    // shaped run, and re-colouring the glyphs inside the range would mean
    // splitting the run at the selection boundary on every caret move.
    let selection_color = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.selection.as_ref())
        .and_then(|s| s.background.as_ref())
        .and_then(gpui::Fill::color)
        .and_then(|b| b.as_solid())
        .unwrap_or(gpui::Hsla { h: 0.6, s: 0.8, l: 0.6, a: 0.35 });
    let options = TextInputOptions {
        placeholder: t.prop(id, "placeholder").unwrap_or("").to_string().into(),
        text_color,
        placeholder_color,
        selection_color,
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
        // `text-align` is an INHERITED property, so an `<input class="text-center">`
        // carries it on the element itself while an input inside a centred
        // wrapper gets it from the paint-time text style gpui threads down —
        // which the widget cannot see at prepaint, when the offset is computed.
        // Reading the element's own folded value covers the case authors
        // actually write (the OTP / meeting-code field), and the inherited case
        // stays left-aligned, as it was before.
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
            .unwrap_or(vn_elements::DEFAULT_ROWS),
        input_config: input_configuration(t, id, multiline),
        // `pointer-events: none`, inherited (the cascade resolved it before the
        // states were stored). An inert field takes no press at all — see
        // `TextInputOptions::inert`.
        inert: node
            .computed
            .as_ref()
            .is_some_and(|cs| cs.extras.pointer_events_none == Some(true)),
    };

    let mask = pointer_mask(node);
    let bit = move |ev: u32| mask & (1u64 << ev) != 0;

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
    // Edge-triggered: `None` on every frame until gpui has actually queried a
    // configuration this node has not reported yet. See
    // `TextInputState::newly_forwarded_configuration` for why that is the only
    // point in the chain where "gpui took it" is distinguishable from "we built
    // it".
    if let Some((config, queries)) =
        state.update(cx, |s, _| s.newly_forwarded_configuration())
    {
        // One line per distinct CONFIGURATION, not per field. The line proves
        // the chain works, and a form of six identically-configured inputs
        // proves it six times for no extra information — which is noise in a
        // log whose other lines all name a real platform limit.
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

/// The `autocorrect` / `autocapitalize` / `spellcheck` / `enterkeyhint`
/// attributes of one field, mapped onto what gpui forwards to the platform.
///
/// The attributes arrive through the ordinary DOM-compat prop channel
/// (`patchProp` → `__vn_set_prop`), so nothing had to be added on the JS side;
/// `type`/`inputmode` are read for the hints they carry, and a `<textarea>` has
/// no `type` by definition. A value outside HTML's set is advised once, keyed
/// on the value rather than on the node, so a bad attribute in a `v-for` costs
/// one line rather than one per row.
fn input_configuration(t: &Tree, id: u32, multiline: bool) -> gpui::TextInputConfiguration {
    let attributes = vn_elements::InputAttributes {
        autocorrect: t.prop(id, "autocorrect"),
        autocapitalize: t.prop(id, "autocapitalize"),
        spellcheck: t.prop(id, "spellcheck"),
        enterkeyhint: t.prop(id, "enterkeyhint"),
        inputmode: t.prop(id, "inputmode"),
        input_type: (!multiline).then(|| t.prop(id, "type")).flatten(),
    };
    let mapped = vn_elements::configuration(attributes);
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
    use super::*;
    use crate::tree::with_tree;

    fn z(z_index: i32) -> vn_css::Extras {
        vn_css::Extras { z_index: Some(z_index), ..Default::default() }
    }

    fn fixed() -> vn_css::Extras {
        vn_css::Extras { fixed: Some(true), ..Default::default() }
    }

    /// A `z-index` orders an element among its siblings INSIDE its stacking
    /// context — a descendant of a `z-50` context paints above it whatever its
    /// own z is, and below the next context that outranks 50.
    ///
    /// gpui sorts deferred draws by one flat priority, so the context chain is
    /// packed into that integer, most significant level first. Without it
    /// reka's `z-50` dialog painted its own background over its `z-10` sticky
    /// header — 10 sorts before 50 — and the dialog came up as a black slab.
    #[test]
    fn a_nested_stacking_context_sorts_inside_its_parent() {
        let root = Stacking::default();
        let (dialog, inside_dialog) = root.enter(&z(50));
        let (header, _) = inside_dialog.enter(&z(10));
        let (overlay, _) = root.enter(&z(50));
        let (toast, _) = root.enter(&z(60));

        let dialog = dialog.unwrap();
        let header = header.unwrap();
        assert!(header > dialog, "a z-10 header paints above its z-50 dialog");
        assert!(header < toast.unwrap(), "…and below a z-60 sibling of the dialog");
        assert_eq!(overlay.unwrap(), dialog, "same z, same priority: document order decides");
    }

    /// `position: fixed` with no `z-index` still opens a level, so a z-indexed
    /// descendant of a portalled overlay sorts inside it rather than against
    /// the document.
    #[test]
    fn fixed_opens_a_level_even_at_z_zero() {
        let root = Stacking::default();
        let (outer, inside) = root.enter(&fixed());
        let (inner, _) = inside.enter(&z(1));
        assert_eq!(outer, Some(0));
        assert!(inner.unwrap() > 0);
        // …and still below anything the document ranks above the overlay.
        assert!(inner.unwrap() < root.enter(&z(1)).0.unwrap());
    }

    /// An element that is not out of flow neither defers nor opens a level, so
    /// an ordinary subtree costs nothing.
    #[test]
    fn an_in_flow_element_changes_nothing() {
        let root = Stacking::default();
        let (priority, child) = root.enter(&vn_css::Extras::default());
        assert_eq!(priority, None);
        assert_eq!(child.depth, root.depth);
        assert_eq!(child.prefix, root.prefix);
    }

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
            let laid_out: Vec<u32> = laid_out_children(t, node);
            assert_eq!(laid_out, boxes, "only the three elements are flex items");
        });
    }

    /// A negative `z-index` on an OUT-OF-FLOW child paints first, which is
    /// CSS 2.1 Appendix E's rule (after the parent's background, before the
    /// parent's in-flow content) and costs nothing, because taffy lays an
    /// absolutely positioned box out against the container regardless of where
    /// it sits among its siblings.
    ///
    /// An IN-FLOW one keeps its place: moving it would move its box, which is
    /// worse than not honouring the declaration.
    #[test]
    fn a_negative_z_index_paints_first_only_when_it_costs_no_layout() {
        fn styled(z: i32, absolute: bool) -> vn_css::ComputedStates {
            let mut base = gpui::StyleRefinement::default();
            if absolute {
                base.position = Some(gpui::Position::Absolute);
            }
            vn_css::ComputedStates {
                base: std::rc::Rc::new(base),
                extras: vn_css::Extras { z_index: Some(z), ..Default::default() },
                ..Default::default()
            }
        }

        with_tree(|t| {
            let div = t.intern("div");
            let parent = t.create_element(div);
            let plain = t.create_element(div);
            t.insert(plain, parent, 0);
            let in_flow_behind = t.create_element(div);
            t.insert(in_flow_behind, parent, 0);
            let behind = t.create_element(div);
            t.insert(behind, parent, 0);
            let far_behind = t.create_element(div);
            t.insert(far_behind, parent, 0);

            t.node_mut(in_flow_behind).unwrap().computed = Some(styled(-1, false));
            t.node_mut(behind).unwrap().computed = Some(styled(-1, true));
            t.node_mut(far_behind).unwrap().computed = Some(styled(-5, true));

            let node = t.node(parent).unwrap();
            assert_eq!(
                node.children,
                vec![plain, in_flow_behind, behind, far_behind],
                "document order is unchanged in the retained tree"
            );
            assert_eq!(
                laid_out_children(t, node),
                vec![far_behind, behind, plain, in_flow_behind],
                "the two out-of-flow ones move to the front, most negative first; the in-flow                  one keeps its place"
            );
            // …and the in-flow one is the case that gets the advisory.
            assert!(paint_first_z(t, behind).is_some());
            assert!(paint_first_z(t, far_behind).is_some());
            assert!(paint_first_z(t, in_flow_behind).is_none());
            assert!(paint_first_z(t, plain).is_none());
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
                laid_out_children(t, node),
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
                laid_out_children(t, node),
                vec![empty_div, text]
            );
            assert!(!generates_a_box(t, blank));
            // A node that no longer exists never generates a box.
            assert!(!generates_a_box(t, 99_999));
        });
    }

    /// White space at the ends of a LINE is removed (CSS Text §4.1.1). Vue's
    /// compiler condenses `\n      ` to one space but does not remove it, and
    /// two spaces of Inter at 15.75px are 9.3px — which is the difference
    /// between apps/meet-probe's hero subtitle fitting its `max-w-sm` on one
    /// line, where the browser puts it, and wrapping onto two.
    #[test]
    fn inline_children_share_a_line_even_with_no_bare_text() {
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");

            // `<div><span/><span/></div>` — the shape a console line built out
            // of styled runs takes, and the shape that used to stack.
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

            // …and a BLOCK child still opts the whole element out: a `<div>`
            // among the spans is a line of its own in a browser too.
            let mixed = t.create_element(div);
            let inline = t.create_element(span);
            let blocky = t.create_element(div);
            t.insert(inline, mixed, 0);
            t.insert(blocky, mixed, 1);
            let mut style2 = gpui::StyleRefinement::default();
            apply_inline_row(&mut style2, t, t.node(mixed).unwrap(), None);
            assert_eq!(style2.display, None);

            // `display: inline` on a child is the strongest statement that
            // it belongs on this line — but the mapper folds every inline
            // OUTER type onto `Block` (taffy models the inner type only), so
            // without the `inline_level` check it read as an opt-out and
            // stacked the row.
            let declared = t.create_element(div);
            for i in 0..2u32 {
                let s = t.create_element(span);
                t.insert(s, declared, i);
                let mut base = gpui::StyleRefinement::default();
                base.display = Some(gpui::Display::Block);
                let mut cs = vn_css::ComputedStates::default();
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
    fn a_wheel_delta_reaches_js_with_the_DOM_sign() {
        use gpui::px;
        // gpui: positive y = the wheel turned away from you = scrolling UP.
        // DOM:  positive deltaY = scrolling DOWN. They are mirror images, and a
        // handler that adds `deltaY` to an offset has to get the DOM's.
        let (_, up) = dom_wheel_delta(Point::new(px(0.0), px(120.0)));
        assert!(up < 0.0, "a scroll UP must reach JS as a negative deltaY, got {up}");
        let (_, down) = dom_wheel_delta(Point::new(px(0.0), px(-120.0)));
        assert!(down > 0.0, "a scroll DOWN must reach JS as a positive deltaY, got {down}");
        // …and the same for the horizontal axis (shift+wheel).
        let (left, _) = dom_wheel_delta(Point::new(px(80.0), px(0.0)));
        assert!(left < 0.0);
        assert_eq!(dom_wheel_delta(Point::new(px(0.0), px(0.0))), (0.0, 0.0));
    }

    #[test]
    fn interior_white_space_collapses_the_way_css_says() {
        use vn_css::WhiteSpaceMode as WS;
        // The default. A newline is white space, not a line break — this is the
        // one that was missing, and it is what garbled every multi-line
        // console line in the inspector.
        assert_eq!(processed_text("a\nb", WS::Normal), "a b");
        assert_eq!(processed_text("a  \t\n  b", WS::Normal), "a b");
        assert_eq!(processed_text("a\r\nb", WS::Normal), "a b");
        // `nowrap` collapses identically — it only changes whether a LINE may
        // wrap, which is gpui's half of the property.
        assert_eq!(processed_text("a\nb", WS::Nowrap), "a b");
        // `pre-line` keeps the break and collapses the spaces around it.
        assert_eq!(processed_text("a   \n   b", WS::PreLine), "a\nb");
        assert_eq!(processed_text("a  b", WS::PreLine), "a b");
        // The preserving family is passed through whole.
        for mode in [WS::Pre, WS::PreWrap, WS::BreakSpaces] {
            assert_eq!(processed_text("a   \n   b", mode), "a   \n   b");
        }
        // Nothing to do: the string is borrowed rather than rebuilt.
        assert!(matches!(processed_text("a b c", WS::Normal), std::borrow::Cow::Borrowed(_)));
        assert!(matches!(processed_text("plain", WS::Normal), std::borrow::Cow::Borrowed(_)));
        // A leading/trailing run survives collapsing as ONE space — removing it
        // is `trim_edges`' job, and only at the ends of a line.
        assert_eq!(processed_text("  a  ", WS::Normal), " a ");
    }

    #[test]
    fn white_space_at_the_ends_of_a_line_is_removed() {
        with_tree(|t| {
            let p = t.intern("p");
            let span = t.intern("span");

            // `<p>` whose only child is a text node: both ends are line ends.
            let block = t.create_element(p);
            let only = t.create_text();
            t.set_text(only, " Secure video meetings ".to_string());
            t.insert(only, block, 0);
            assert_eq!(
                trim_edges(t, t.node(only).unwrap(), only, &t.node(only).unwrap().text, None),
                "Secure video meetings"
            );

            // `<p>a<span/> b </p>`: the last text node keeps its LEADING space,
            // which is what separates it from the span, and loses its trailing
            // one, which is at the end of the line.
            let mixed = t.create_element(p);
            let head = t.create_text();
            t.set_text(head, " a ".to_string());
            t.insert(head, mixed, 0);
            let inline = t.create_element(span);
            t.insert(inline, mixed, 0);
            let tail = t.create_text();
            t.set_text(tail, " b ".to_string());
            t.insert(tail, mixed, 0);
            assert_eq!(trim_edges(t, t.node(head).unwrap(), head, &t.node(head).unwrap().text, None), "a ");
            assert_eq!(trim_edges(t, t.node(tail).unwrap(), tail, &t.node(tail).unwrap().text, None), " b");

            // `a<span> </span>b`: the span's only child is a space a browser
            // DOES render — it is in the middle of the line, not at its end —
            // so an inline-level parent with siblings suppresses the trim.
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
            assert_eq!(trim_edges(t, t.node(gap).unwrap(), gap, &t.node(gap).unwrap().text, None), " ");
        });
    }

    /// The same rule for the text an ELEMENT carries itself, which is the path
    /// Vue actually takes: `setElementText` for an element whose only child is
    /// text, i.e. most elements — and the hero subtitle.
    #[test]
    fn an_elements_own_text_is_trimmed_at_the_ends_of_its_line() {
        with_tree(|t| {
            let p = t.intern("p");
            let span = t.intern("span");

            let block = t.create_element(p);
            t.set_element_text(block, " Secure video meetings ".to_string());
            assert_eq!(
                trim_own_edges(t, t.node(block).unwrap(), &t.node(block).unwrap().text, None),
                "Secure video meetings"
            );

            // A lone inline element is still the whole line…
            let lone = t.create_element(span);
            t.set_element_text(lone, " x ".to_string());
            assert_eq!(trim_own_edges(t, t.node(lone).unwrap(), &t.node(lone).unwrap().text, None), "x");

            // …but one sharing a line with something else is not.
            let row = t.create_element(p);
            let word = t.create_text();
            t.set_text(word, "a".to_string());
            t.insert(word, row, 0);
            let inline = t.create_element(span);
            t.insert(inline, row, 0);
            t.set_element_text(inline, " b ".to_string());
            assert_eq!(trim_own_edges(t, t.node(inline).unwrap(), &t.node(inline).unwrap().text, None), " b ");
        });
    }

    /// A percentage `flex-basis` stands in for the main-axis size (Flexbox
    /// §9.2.3), and taffy falls back to that size when the percentage cannot be
    /// resolved — which sized reka's Select viewport to the trigger's 40px
    /// instead of to its items.
    #[test]
    fn a_percentage_flex_basis_drops_the_main_axis_size() {
        fn flex(direction: Option<gpui::FlexDirection>) -> vn_css::ComputedStates {
            let mut base = gpui::StyleRefinement::default();
            base.display = Some(gpui::Display::Flex);
            base.flex_direction = direction;
            vn_css::ComputedStates { base: std::rc::Rc::new(base), ..Default::default() }
        }
        fn item() -> gpui::StyleRefinement {
            let mut style = gpui::StyleRefinement::default();
            style.flex_basis = Some(gpui::Length::Definite(gpui::relative(0.0)));
            style.size.width = Some(gpui::Length::Definite(gpui::px(390.0).into()));
            style.size.height = Some(gpui::Length::Definite(gpui::px(40.0).into()));
            style
        }

        with_tree(|t| {
            let div = t.intern("div");
            let column = t.create_element(div);
            t.node_mut(column).unwrap().computed = Some(flex(Some(gpui::FlexDirection::Column)));
            let viewport = t.create_element(div);
            t.insert(viewport, column, 0);

            let mut style = item();
            apply_percentage_flex_basis(&mut style, t, t.node(viewport).unwrap());
            assert_eq!(style.size.height, None, "the main-axis size is the basis's job");
            assert!(style.size.width.is_some(), "the cross axis is untouched");

            // A ROW parent makes width the main axis instead.
            let row = t.create_element(div);
            t.node_mut(row).unwrap().computed = Some(flex(None)); // `row` is the initial value
            let child = t.create_element(div);
            t.insert(child, row, 0);
            let mut style = item();
            apply_percentage_flex_basis(&mut style, t, t.node(child).unwrap());
            assert_eq!(style.size.width, None);
            assert!(style.size.height.is_some());

            // Outside a flex container the property is inert and the size is
            // the size.
            let block = t.create_element(div);
            let inside = t.create_element(div);
            t.insert(inside, block, 0);
            let mut style = item();
            apply_percentage_flex_basis(&mut style, t, t.node(inside).unwrap());
            assert!(style.size.width.is_some() && style.size.height.is_some());

            // …and so is an ABSOLUTE basis, which taffy resolves for itself.
            let absolute = t.create_element(div);
            t.insert(absolute, column, 0);
            let mut style = item();
            style.flex_basis = Some(gpui::Length::Definite(gpui::px(0.0).into()));
            apply_percentage_flex_basis(&mut style, t, t.node(absolute).unwrap());
            assert!(style.size.height.is_some());
        });
    }
}
