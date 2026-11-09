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
    MouseButton, MouseDownEvent, MouseUpEvent, Overflow, Point, ScrollHandle, ScrollEvent,
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
    /// select() can run in nextTick before the input has its first native build.
    static INPUT_SELECT_ALL: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// Persistent scroll offset per scroll container. gpui keys anonymous
    /// scroll state by `ElementId` too, but a handle we own is what lets us
    /// *read* the offset back out for `e.target.scrollTop`.
    static SCROLL_HANDLES: RefCell<HashMap<u32, ScrollHandle>> = RefCell::new(HashMap::new());
    /// Last offset reported to JS per node, in CSS pixels — a `scroll` event
    /// only fires when the position actually moved.
    static SCROLL_REPORTED: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
    /// One-shot compat advisories already emitted (keyed by message).
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Nodes lowered in the build currently running — the VN_PERF counter. One
    /// `Cell` increment per node, unconditionally: the branch a probe check
    /// would add costs as much as the add.
    static BUILT: Cell<u64> = const { Cell::new(0) };
    /// Element-tree builds since the process started.
    ///
    /// Not a statistic: it is how a DOM read that asks for a forced layout
    /// finds out whether it GOT one. `sync_layout` is a no-op wherever a
    /// second `draw` is refused — which is anywhere inside the window's own
    /// update, i.e. every event handler — and a caller that has to report a
    /// stale answer has no other way to tell (`crate::dom::layout_if_stale`).
    static BUILDS: Cell<u64> = const { Cell::new(0) };
    /// Builds in ANY window, a pop-out's included: when layout can have moved.
    /// It is how the observer step knows a frame laid anything out since its
    /// last run (`crate::observe`). [`BUILDS`] stays the primary window's
    /// alone, because a pop-out's build is not the forced layout a DOM read on
    /// the main window asked for.
    static LAYOUTS: Cell<u64> = const { Cell::new(0) };
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
    /// Per-node "this has looked wrong for N builds running" counters — see
    /// [`settled`]. Empty on every app that has nothing to advise about.
    static SETTLING: RefCell<HashMap<u32, u8>> = RefCell::new(HashMap::new());
}

/// Whether the build asks gpui to tag a hitbox onto every node.
///
/// Off by default and turned on by `vn-app`'s devtools boot, which every host
/// compiles but a `--features no-devtools` one (a `--features prod` host
/// included: the inspector attaches to a host that is already running). A
/// no-devtools binary tags nothing, so gpui inserts hitboxes only where an
/// element is interactive and its per-move hit test iterates exactly what it
/// always did; the other elements get the cheaper PICK records
/// (`crate::pointer`). The cost when it IS on is one `Hitbox` per painted
/// element per frame and the same number of rectangle tests per mouse move;
/// measured on apps/meet-probe (~400 painted nodes) it does not move the frame
/// budget, and on `fixtures/hit-perf` (~3 000 elements) it is within noise of
/// the picks (research/reports/m26/hit-target-1.md §3).
static PICK_HITBOXES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turns [`PICK_HITBOXES`] on. Called once, from the dev host's devtools boot.
pub fn enable_pick_hitboxes() {
    PICK_HITBOXES.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Tells this window whether to give every element a hitbox, and turns its hit
/// targeting on. Called once per build, because the flags live on the `Window`
/// and every window this process paints has to be told.
fn arm_pick_hitboxes(window: &mut Window) {
    if PICK_HITBOXES.load(std::sync::atomic::Ordering::Relaxed) {
        window.set_tag_every_hitbox(true);
    }
    // Every host: a mouse event goes to the topmost element under the
    // pointer, which needs every element to be a box gpui can find there
    // (picks, for the ones without a hitbox) and the target resolved before
    // the listeners run (`crate::pointer`).
    crate::pointer::arm(window);
}

/// The hitbox gpui made for `id` in the frame this window last rendered — the
/// handle [`gpui::Hitbox::is_hovered_at`] takes.
///
/// It is gpui's OWN hitbox for the element, not one of ours beside it, which is
/// the whole point: it carries the CSS `transform` in force where gpui inserted
/// it (from `Style::transform_matrix`, which prepaint and paint share), the
/// content mask of every clipping ancestor, and the place in the frame's hitbox
/// list that a deferred draw gave it. A hitbox recorded from OUTSIDE the
/// element — which is where a wrapper like [`BoundsProbe`] stands — would carry
/// the PARENT's matrix, and so be wrong for exactly the case this exists for.
///
/// The window is the key: hitboxes belong to the frame that inserted them, so a
/// popped-out node answers from the pop-out's own `Window`.
pub fn pick_hitbox(window: &Window, id: u32) -> Option<gpui::Hitbox> {
    window.tagged_hitbox(id).cloned()
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
    warn_compat_at(key, String::new, msg)
}

/// [`warn_compat`], with the node that tripped it named first.
///
/// One advisory covers every element with the same gap, which is the point —
/// but "somewhere on this page" is not a place a developer can go and look. The
/// FIRST offender's id and selector are printed, and `ctl query`/`highlight`
/// take a node id, so the line is a lead rather than a riddle. m19's own report
/// listed "not pinned to a node" against four of these. `at` is only called
/// for the first offender: the checks run on every build of every node.
pub(crate) fn warn_compat_at(key: &str, at: impl FnOnce() -> String, msg: &str) {
    WARNED.with(|w| {
        let mut warned = w.borrow_mut();
        if !warned.contains(key) && warned.insert(key.to_string()) {
            let msg = crate::log::one_line(&format!("{}{msg}", at()));
            eprintln!("[vue-native compat] {msg}");
            crate::log::record("advisory", msg);
        }
    });
}

/// `"node 42 (<div class=\"a b\">): "` — the lead-in [`warn_compat_at`] puts in
/// front of an advisory. At most three classes; the point is to recognise the
/// element, not to reproduce it.
fn node_prefix(t: &Tree, id: u32) -> String {
    let Some(node) = t.node(id) else { return String::new() };
    let tag = t.name(node.tag);
    let classes: Vec<&str> = node.classes.iter().map(String::as_str).take(3).collect();
    if classes.is_empty() {
        format!("node {id} (<{tag}>): ")
    } else {
        format!("node {id} (<{tag} class=\"{}\">): ", classes.join(" "))
    }
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
        // Reuse both generations' allocation: rebuilding a chat frame must
        // not allocate and grow every bounds map from zero again.
        std::mem::swap(&mut w.bounds, &mut w.prev_bounds);
        w.bounds.clear();
        if w.bounds.capacity() > w.prev_bounds.len().saturating_mul(4).max(1024) {
            w.bounds.shrink_to(w.prev_bounds.len().saturating_mul(2));
        }
        std::mem::swap(&mut w.transformed, &mut w.prev_transformed);
        w.transformed.clear();
        if w.transformed.capacity() > w.prev_transformed.len().saturating_mul(4).max(1024) {
            w.transformed.shrink_to(w.prev_transformed.len().saturating_mul(2));
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

/// `matrix` is the node's whole CSS `transform`, its ancestors' composed with
/// its own: where the picture of the box is, which is what
/// `getBoundingClientRect()` reports in a browser. The layout box stays the
/// rectangle offsetWidth, ResizeObserver and hit-testing-by-rect read.
fn record_bounds(
    id: u32,
    bounds: gpui::Bounds<gpui::Pixels>,
    matrix: gpui::TransformationMatrix,
    window: &Window,
) {
    let rect = rect_of(bounds);
    let moved = (!matrix.is_unit()).then(|| rect_of(window.transformed_bounds(matrix, bounds)));
    // Which window's prepaint recursion are we inside? gpui just told us: this
    // is the `&mut Window` it is running the recursion with. The spike had to
    // infer it from a `SubtreeScope` element that bracketed the recursion,
    // because the map was keyed by painted root rather than by window.
    windows::here(window, |w| {
        w.bounds.insert(id, rect);
        match moved {
            Some(t) => w.transformed.insert(id, t),
            // A prepaint can run twice in one frame (gpui's `transact`).
            None => w.transformed.remove(&id),
        };
    });
}

/// Where CSS `transform` put the node, in the window [`bounds_of`] finds it
/// in: `None` when nothing moved it, and its layout box is the answer.
pub fn transformed_of(id: u32) -> Option<Rect> {
    let k = windows::keys()
        .into_iter()
        .find(|&k| windows::peek(k, |w| w.bounds.contains_key(&id)) == Some(true))?;
    windows::peek(k, |w| w.transformed.get(&id).copied()).flatten()
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

/// [`bounds_of`], asked of ONE window: the rectangle **this** window painted
/// the node at, `None` when this window did not paint it at all.
///
/// This is the form a question about a POINT needs. `bounds_of` answers "where
/// is it" by searching every window in turn, which is right for `query`/`tree`
/// and wrong the moment the number is going to be fed back in as a coordinate —
/// two windows' maps hold numbers in two different client spaces.
pub fn bounds_here(window: &Window, id: u32) -> Option<Rect> {
    windows::peek(windows::key(window), |w| w.bounds.get(&id).copied()).flatten()
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
pub(crate) fn prev_size_of(id: u32) -> Option<(f32, f32)> {
    // The window that paints this node is the only one whose previous frame can
    // have a rectangle for it, so this is a direct lookup rather than a search.
    let k = windows::window_of_node(id);
    windows::peek(k, |w| w.prev_bounds.get(&id).copied())
        .flatten()
        .filter(|(_, _, w, h)| *w > 0.0 && *h > 0.0)
        .map(|(_, _, w, h)| (w, h))
}

/// Element-tree builds since the process started — see [`BUILDS`].
pub(crate) fn build_count() -> u64 {
    BUILDS.with(|c| c.get())
}

/// Per-node side state the renderer keeps (devtools `mem`): `(focus handles,
/// text-input states, scroll handles, bounds entries over every window)`.
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

/// Builds in any window since the process started — see [`LAYOUTS`].
pub(crate) fn layout_count() -> u64 {
    LAYOUTS.with(|c| c.get())
}

/// A build, for a test that has no window to build in.
#[cfg(test)]
pub(crate) fn note_build() {
    BUILDS.with(|c| c.set(c.get().wrapping_add(1)));
    LAYOUTS.with(|c| c.set(c.get().wrapping_add(1)));
}

/// The [`ScrollHandle`] a scroll container owns, or `None` for anything the
/// cascade never gave `overflow: scroll|auto`.
///
/// The handle is the scroll position: gpui paints the subtree through it, our
/// wheel router moves it, the overlay scrollbar reads its geometry from it, and
/// [`crate::scroll`] is the JS surface on top of the same object. It outlives
/// the frame (the map here is keyed by node id and swept by
/// [`gc_node_state`]), which is what makes a programmatic write readable back
/// on the next line with no frame in between.
pub(crate) fn scroll_handle_of(id: u32) -> Option<ScrollHandle> {
    SCROLL_HANDLES.with(|m| m.borrow().get(&id).cloned())
}

/// The whole rectangle this node was painted at in the frame BEFORE the one
/// being built — [`prev_size_of`]'s origin included, which is what the cache's
/// "did my parent reflow?" check compares.
pub(crate) fn prev_rect_of(id: u32) -> Option<Rect> {
    let k = windows::window_of_node(id);
    windows::peek(k, |w| w.prev_bounds.get(&id).copied()).flatten()
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
        let _profile_window = vn_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
        let _profile = vn_runtime::redjs::profiling::span("layout", "Layout requests");
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
        // Recorded before the subtree prepaints, which may read it, as where
        // the ancestors' transforms put the box; its own transform is only
        // known once its element has prepainted, and amends it below.
        let ancestors = window.element_transform();
        record_bounds(self.id, bounds, ancestors, window);
        // Every hitbox gpui inserts from here down is tagged with this node id
        // until a descendant's own probe overrides it — which is how a node
        // gets back the hitbox gpui made for IT, under the transform and
        // content mask it made it under (see [`pick_hitbox`]). The previous tag
        // is restored on the way out, so the parent's boxes stay the parent's.
        let _profile_window = self.root.then(|| vn_runtime::redjs::profiling::WindowScope::enter(windows::key(window)));
        if self.root { crate::perf::note_taffy_gap(); }
        let _profile = self.root.then(|| vn_runtime::redjs::profiling::span("paint", "Prepaint native tree"));
        let span = self.root.then(crate::perf::Span::start);
        let (_, own) =
            window.with_hitbox_tag(self.id, |window| self.inner.prepaint(window, cx));
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
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.root {
            self.inner.paint(window, cx);
            return;
        }
        let _profile_window = vn_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
        let _profile = vn_runtime::redjs::profiling::span("paint", "Paint native tree");
        // A text selection being dragged hears every move and the release
        // from here, wherever the pointer is (`crate::selection`).
        crate::selection::paint_listeners(window);
        crate::pointer::capture_listeners(window);
        let span = crate::perf::Span::start();
        self.inner.paint(window, cx);
        crate::perf::note_paint(span.ms());
    }
}

/// Wraps a built element so its rectangle lands in the side map. Text and
/// anchor nodes are skipped — they have no box a selector can ask about.
pub(crate) fn probe(id: u32, inner: AnyElement) -> AnyElement {
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
    INPUT_SELECT_ALL.with(|m| m.borrow_mut().retain(|&id| live(id)));
    SCROLL_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_REPORTED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SETTLING.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    crate::cache::gc(&live);
    crate::cull::gc(&live);
    crate::scroll::gc(&live);
    img_src::gc(live);
    crate::lottie::gc(live);
    crate::video::gc(live);
    sticker::gc(live);
    crate::svg::gc(live);
    // A selection boundary in a node that went away, or past the end of a
    // shortened text (`crate::selection::gc`).
    tree::with_tree(crate::selection::gc);
    // This function runs off a JS WAVE, not off a frame, so it has no `Window`
    // and cannot ask which one it is standing in — one of exactly two such
    // paths (docs/MULTIWINDOW.md §5.6). It iterates every window instead, which
    // is what it already does for every node.
    windows::each(|_, w| {
        w.bounds.retain(|&id, _| live(id));
        w.prev_bounds.retain(|&id, _| live(id));
        w.transformed.retain(|&id, _| live(id));
        w.prev_transformed.retain(|&id, _| live(id));
        w.focus_subs.retain(|&id, _| live(id));
        if w.pointer_capture.is_some_and(|id| !live(id) || !tree::read_tree(|t| reaches_root(t, id))) {
            w.pointer_capture = None;
        }
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
        // The Menu key is VK_APPS, which gpui names "menu"; the DOM's name is
        // "ContextMenu". The function keys are "F1"…"F24" in the DOM, and gpui
        // spells them in lower case.
        "menu" => "ContextMenu".into(),
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            f.to_ascii_uppercase()
        }
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
///
/// A listener's claim says only that somebody listens under the pointer: the
/// move goes to the topmost element there (`crate::pointer::target_at`), which
/// the listening box may be painted under. Resolved here, once a frame, rather
/// than on every gpui move.
///
/// The frame's hover is resolved first (`crate::pointer::resolve_hover`), so
/// its `mouseleave` / `mouseenter` come before the `mousemove`, as in a
/// browser.
pub fn flush_pending_mouse_move(window: &mut Window, cx: &mut App) {
    // The frame's script runs after this: its queries see this window's
    // modality as gpui's `:focus-visible` rung paints it.
    crate::live_state::note_modality(window);
    crate::pointer::resolve_hover(window, cx);
    let Some(p) = windows::here(window, |w| w.pending_mouse_move.take()) else { return };
    let payload = mouse_payload(p.position, p.button, &p.modifiers);
    if p.button < 0.0 && crate::pointer::captured_target(window).is_some() {
        cancel_press(window, cx);
    }
    let target = crate::pointer::captured_target(window).unwrap_or_else(|| crate::pointer::target_at(window, p.position));
    // The pump's, before gpui's draw: a forced layout may draw (`crate::off_input`).
    crate::off_input(|| js_dispatch_app(target, tree::EV_MOUSEMOVE, &payload, window, cx));
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

pub(crate) fn mouse_payload(position: Point<gpui::Pixels>, button: f64, m: &gpui::Modifiers) -> EventPayload<'static> {
    EventPayload {
        a: f64::from(position.x),
        b: f64::from(position.y),
        c: button,
        d: mods_bits(m),
        key: None,
    }
}

/// [`mouse_payload`] for the events whose `detail` is the click count
/// (`mousedown`, `mouseup`, `click`; UI Events §3.5): it rides `d` above the
/// four modifier bits, so the hot path still crosses with four doubles.
fn mouse_payload_n(position: Point<gpui::Pixels>, button: f64, m: &gpui::Modifiers, detail: usize) -> EventPayload<'static> {
    let mut p = mouse_payload(position, button, m);
    p.d += (detail.min(255) as f64) * 16.0;
    p
}

/// A mouse payload kept past its listener (an [`UpWave`] slot). No `key`, so
/// no lifetime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MousePt {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

impl MousePt {
    fn of(p: &EventPayload) -> Self {
        MousePt { a: p.a, b: p.b, c: p.c, d: p.d }
    }
    fn payload(&self) -> EventPayload<'static> {
        EventPayload { a: self.a, b: self.b, c: self.c, d: self.d, key: None }
    }
}

/// One physical mouse-up, as the DOM sees it: a `mouseup`, then a `click`
/// (left button, paired with its DOM press) or an `auxclick` (the middle or
/// right button, the same rule), then — the right button, Chromium
/// on Windows — a `contextmenu`. Each slot is claimed by the first listening
/// box on the target's path, for the target the release resolved to
/// (`crate::pointer`; a click's and an auxclick's is the common ancestor of
/// the press's and the release's). Why they are claimed and delivered later
/// rather than dispatched in place: `WindowState::up_waves`.
#[derive(Default)]
pub struct UpWave {
    /// DOM identity survives a cached-view wrapper being inserted or replaced.
    press: Option<Press>,
    mouseup: Option<(u32, MousePt)>,
    click: Option<(u32, MousePt)>,
    aux: Option<(u32, MousePt)>,
    menu: Option<(u32, MousePt)>,
    scheduled: bool,
    /// The release of a press the host cancelled: nothing is delivered.
    cancelled: bool,
}

impl UpWave {
    fn click_press(&self) -> Option<u32> {
        let p = self.press.as_ref()?;
        (p.clickable && !p.cancelled).then_some(p.target)
    }
}

/// The press in progress in one window (`WindowState::press`).
pub struct Press {
    /// The `mousedown` target — where a `pointercancel` goes.
    target: u32,
    button: f64,
    position: Point<gpui::Pixels>,
    /// A disabled control or a native drag cannot become a click later.
    clickable: bool,
    /// The host gave up on it ([`cancel_press`]): its mouse-up delivers nothing.
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

/// The capture phase of EVERY mouse-up opens its wave, from the build root —
/// the first box painted, so the first capture listener gpui runs — whether
/// the pointer is over it or not ([`bind_root_mouse`]). The release also ends
/// the press in progress, if it is that press's button.
fn open_up_wave(ev: &MouseUpEvent, window: &mut Window, cx: &mut App) {
    let button = button_code(ev.button);
    windows::here(window, |w| {
        let press = Press::take_for_release(&mut w.press, button);
        let cancelled = press.as_ref().is_some_and(|p| p.cancelled);
        // A wave nobody claimed a share of is never flushed: drop it here.
        w.up_waves.retain(|wave| wave.scheduled);
        w.up_waves.push(UpWave { press, cancelled, ..Default::default() });
    });
    // A capture must receive its release even when no hovered element has a
    // mouse-up listener. Claim it from the root before any optional listeners.
    if let Some(target) = crate::pointer::captured_target(window) {
        let payload = mouse_payload_n(ev.position, button, &ev.modifiers, ev.click_count);
        claim_up(window, cx, |w| &mut w.mouseup, target, &payload);
    }
}

/// The root's share of every press: it starts each mouse-down's focus step
/// ([`begin_press_focus`]), and opens each mouse-up's wave. See
/// [`open_up_wave`]; the `_out` variants are what make "every" true when
/// something that blocks the mouse covers the root under the pointer.
fn bind_root_mouse<E: gpui::InteractiveElement>(mut e: E) -> E {
    e = e.capture_any_mouse_down(|_, window, cx| {
        deliver_move_before_button(window, cx);
        // Re-arm only when a DOM bubble listener actually receives this press.
        // Native widgets may consume it before the DOM path is reached.
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

/// A press or a release delivers the `mousemove` the frame has coalesced
/// first, as Chromium does: a discrete input event flushes the continuous ones
/// queued before it. Otherwise a move and the release after it that arrive in
/// one frame were heard release first, and the move after the drag had ended —
/// a drag stopped where the move before the last one left it. Seen in a pop-out
/// (`specs/lightbox.spec.ts`), whose moves wait for its frame tick.
fn deliver_move_before_button(window: &mut Window, cx: &mut App) {
    let Some(p) = windows::here(window, |w| w.pending_mouse_move.take()) else { return };
    let payload = mouse_payload(p.position, p.button, &p.modifiers);
    if p.button < 0.0 && crate::pointer::captured_target(window).is_some() {
        cancel_press(window, cx);
    }
    let target = crate::pointer::captured_target(window).unwrap_or_else(|| crate::pointer::target_at(window, p.position));
    js_dispatch_app(target, tree::EV_MOUSEMOVE, &payload, window, cx);
}

/// Bubble phase: the first (= innermost) box to ask gets the slot, and the
/// wave's first claim schedules the one flush that delivers it.
fn claim_up(
    window: &mut Window,
    cx: &mut App,
    slot: fn(&mut UpWave) -> &mut Option<(u32, MousePt)>,
    id: u32,
    payload: &EventPayload,
) {
    let schedule = windows::here(window, |w| {
        let Some(wave) = w.up_waves.last_mut() else { return false };
        let s = slot(wave);
        if s.is_none() {
            *s = Some((id, MousePt::of(payload)));
        }
        !std::mem::replace(&mut wave.scheduled, true)
    });
    if schedule {
        // Deferred, so no gpui listener is under the waves: a forced layout may
        // draw in them (`crate::off_input`).
        window.defer(cx, |window, cx| crate::off_input(|| flush_up_waves(window, cx)));
    }
}

/// Pair a physical release with its DOM press, independently of gpui's
/// element-state path. Cached subtree wrappers may change that path between
/// down and up; the DOM node IDs and this window's press remain stable.
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
    let Some(target) = crate::pointer::click_target(window, down, fallback) else { return };
    let payload = mouse_payload_n(ev.position, button_code(ev.button), &ev.modifiers, ev.click_count);
    claim_up(window, cx, slot, target, &payload);
}

/// Delivers every claimed wave, in DOM order. Runs at the end of gpui's
/// dispatch of the mouse-up, so every listener has claimed what it will.
fn flush_up_waves(window: &mut Window, cx: &mut App) {
    loop {
        let Some(wave) = windows::here(window, |w| (!w.up_waves.is_empty()).then(|| w.up_waves.remove(0))) else {
            return;
        };
        // A press the host cancelled delivers nothing more: Pointer Events
        // §10, no events for a pointer after its `pointercancel` until its
        // buttons are released.
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
        // Blink's `WebFrameWidgetImpl::HandleMouseUp` finishes the release
        // (`mouseup`, then the click family) before it shows the context menu
        // on Windows, so the right button's `auxclick` precedes `contextmenu`.
        if let Some((id, p)) = wave.aux {
            js_dispatch_app(id, tree::EV_AUXCLICK, &p.payload(), window, cx);
        }
        if let Some((id, p)) = wave.menu {
            js_dispatch_app(id, tree::EV_CONTEXTMENU, &p.payload(), window, cx);
        }
    }
}

/// The host gives up on the press in progress: `pointercancel` at its
/// `mousedown` target, and its release will deliver nothing (see
/// [`flush_up_waves`]). Called when the window loses activation mid-press —
/// the case where the release goes to some other window, or nowhere.
fn cancel_press(window: &mut Window, cx: &mut App) {
    let captured = crate::pointer::captured_target(window);
    let press = windows::here(window, |w| {
        w.active = None;
        match &mut w.press {
            Some(p) if !p.cancelled => {
                p.cancelled = true;
                Some((captured.unwrap_or(p.target), p.position))
            }
            _ => None,
        }
    });
    if let Some((target, position)) = press {
        if tree::read_tree(|t| t.node(target).is_some()) {
            let payload = mouse_payload(position, -1.0, &gpui::Modifiers::default());
            js_dispatch_app(target, tree::EV_POINTERCANCEL, &payload, window, cx);
        }
    }
    windows::here(window, |w| w.pointer_capture = None);
}

/// Subscribes this window's activation observer once ([`cancel_press`]).
fn watch_activation<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let k = windows::key(window);
    if windows::of(k, |w| w.activation.is_some()) {
        return;
    }
    let sub = cx.observe_window_activation(window, |_, window, cx| {
        if !window.is_window_active() {
            cancel_press(window, cx);
        }
    });
    windows::of(k, |w| w.activation = Some(sub));
}

/// The keyboard's `contextmenu`, as Chromium sources it on Windows
/// (`WebFrameWidgetImpl::HandleKeyEvent`): the UNMODIFIED Menu key fires it on
/// its key-UP and Shift+F10 on its key-DOWN, each only when no handler
/// cancelled that key event. Bound on the build root after every key listener
/// it could run behind, so `key_prevented` holds that answer.
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

/// A keyboard `contextmenu` (Chromium's `ShowNonLocatedContextMenu`): at the
/// focused element, positioned at the centre of its box; with nothing focused,
/// at the document, 1 px in from the corner. No button (`button` 0, `buttons`
/// 0), no modifiers.
fn fire_keyboard_menu(root: u32, window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let focused = windows::focused_node(k);
    let target = if focused != 0 && tree::read_tree(|t| t.node(focused).is_some()) { focused } else { root };
    let (x, y) = windows::of(k, |w| w.bounds.get(&target).copied())
        .filter(|_| target != root || focused == root)
        .map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0))
        .unwrap_or((1.0, 1.0));
    let payload = EventPayload { a: f64::from(x), b: f64::from(y), ..Default::default() };
    js_dispatch_app(target, tree::EV_CONTEXTMENU, &payload, window, cx);
}

/// Publish → dispatch → flush; the shape every listener shares. Returns
/// `false` when a JS handler called `preventDefault()`.
pub(crate) fn js_dispatch_app(id: u32, event: u32, payload: &EventPayload, window: &mut Window, cx: &mut App) -> bool {
    js_wave_app(window, cx, || crate::dispatch_event(id, event, payload))
}

/// [`js_dispatch_app`]'s frame around any dispatch: `wave` runs with the
/// commit phase published, and whatever it changed is flushed and repainted.
/// The clipboard events build their DataTransfer inside it (`crate::clipboard`).
pub(crate) fn js_wave_app(window: &mut Window, cx: &mut App, wave: impl FnOnce() -> bool) -> bool {
    // A gpui listener's dispatch (unless the frame tick is the caller): no
    // forced layout may draw under it (`crate::in_input_dispatch`).
    let _input = crate::InputDispatch::enter();
    crate::live_state::note_modality(window);
    let proceed = with_published(window, cx, Phase::Commit, wave);
    settle_owed_later(window, cx);
    if crate::flush_changes() {
        // NOT `window.refresh()`. Refreshing a window sets gpui's `refreshing`
        // flag, which makes the next draw ignore every `Entity::cached` span in
        // it — so a single devtools click or synthesized event re-rendered the
        // WHOLE document however little of it had moved (m23). The fan-out below
        // notifies the hosting view of every window instead, which schedules the
        // same redraw and leaves the untouched subtrees replayable.
        if !repaint_other_windows(cx) {
            // No host registered the fan-out (a bare unit-test harness): fall
            // back to the hammer rather than silently not repainting.
            window.refresh();
        }
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

/// Asks for a frame in every window that paints this tree, the way a JS wave
/// does ([`js_wave_app`]): the cached subtrees nothing touched stay
/// replayable. For a change made from an input listener that no wave carries
/// (the text selection's).
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

/// The `Context<V>` flavour: `cx.notify()` is the cheaper repaint request when
/// we already hold the hosting view.
fn js_dispatch<V: 'static>(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut Context<V>,
) -> bool {
    let _input = crate::InputDispatch::enter();
    crate::live_state::note_modality(window);
    let proceed = with_published(window, &mut **cx, Phase::Commit, || {
        crate::dispatch_event(id, event, payload)
    });
    settle_owed_later(window, &mut **cx);
    if crate::flush_changes() {
        cx.notify();
        repaint_other_windows(cx);
    }
    proceed
}

thread_local! {
    /// [`settle_owed_later`] has a deferred checkpoint queued.
    static SETTLE_QUEUED: Cell<bool> = const { Cell::new(false) };
}

/// Queues the microtask checkpoint a listener's dispatch left owed
/// (`crate::dispatch_event`) for after gpui's dispatch (docs/RUNTIME.md §5.8).
///
/// An `App::defer` runs once the update that called the listener has
/// returned, and with it the frame's listeners. There the checkpoint runs in a
/// publication of its own, off input, so an `onMounted` that the patch mounts
/// can measure, and the forced layout draws. One is queued per gpui dispatch.
/// Any JS that starts before it settles the checkpoint itself
/// (`vn_runtime::bridge::settle_owed_drain`), and then this one finds nothing
/// owed. It publishes the window the listener ran in; if that one has closed,
/// any other; with none left, the next publication settles it.
fn settle_owed_later(window: &Window, cx: &mut App) {
    if !vn_runtime::bridge::drain_owed() || SETTLE_QUEUED.with(|c| c.replace(true)) {
        return;
    }
    let origin = window.window_handle();
    cx.defer(move |cx| {
        SETTLE_QUEUED.with(|c| c.set(false));
        if !vn_runtime::bridge::drain_owed() {
            return;
        }
        // `js_wave_app`'s publication settles it on entry, then flushes and
        // repaints what the patch changed.
        let settle = |window: &mut Window, cx: &mut App| {
            crate::off_input(|| js_wave_app(window, cx, || true));
        };
        if origin.update(cx, |_, window, cx| settle(window, cx)).is_ok() {
            return;
        }
        for other in cx.windows() {
            if other.update(cx, |_, window, cx| settle(window, cx)).is_ok() {
                return;
            }
        }
    });
}

/// A text field's default action must see what its own event's handlers did
/// (a browser runs the checkpoint between the event and the default action),
/// so the checkpoint is settled HERE, under the listener, before the widget
/// edits. Its microtasks then cannot lay out (L-1); the field's other events
/// (`input`, the mouse-up wave) are dispatched off input anyway.
fn settle_before_default(window: &mut Window, cx: &mut App) {
    if vn_runtime::bridge::drain_owed() {
        js_wave_app(window, cx, || true);
    }
}

/// An edit's payload (`tree::EV_BEFOREINPUT`): its `inputType` code in `a`,
/// `isComposing` in `b`.
fn intent_payload(intent: &vn_elements::InputIntent) -> EventPayload<'static> {
    EventPayload {
        a: f64::from(intent.input_type.code()),
        b: if intent.input_type.composing() { 1.0 } else { 0.0 },
        ..Default::default()
    }
}

/// Does anything on `id`'s path listen for `code`: the node, an ancestor, or
/// `document` / `window`, which ride the root's bit?
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

/// [`js_dispatch_app`] for an event with a string beside its payload (an
/// InputEvent's or a CompositionEvent's `data`), passed as the dispatcher's
/// eighth argument; `None` passes nothing, which the runtime reads as `null`.
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
            vn_runtime::JsRuntime::try_with(|rt| {
                let v = rt.ctx().string(s);
                // Held across the dispatch, which runs arbitrary JS.
                rt.ctx().protect(v);
                v
            })
        });
        let proceed = crate::dispatch_event_with(id, event, payload, value);
        if let Some(v) = value {
            vn_runtime::JsRuntime::try_with(|rt| rt.ctx().unprotect(v));
        }
        proceed
    })
}

/// DOM order for a focus move: `blur` on the node that had it, then `focus` on
/// the node that got it. Neither event bubbles (the JS runtime knows). Each
/// carries the other node in `a`, the events' `relatedTarget`.
///
/// While the old node hears `blur`, nothing is focused (the slot is 0, and
/// `document.activeElement` is `body`). A handler that takes the focus then
/// (reka's FocusScope pulls it back into a dialog from `focusout`) abandons
/// this move: the new node hears nothing. This is Chromium's
/// `Document::SetFocusedElement` ("handler shifted focus"); the JS half of the
/// same rule is the runtime's `focusMoved`.
///
/// The window's root is where gpui's focus rests when nothing is focused (see
/// [`ensure_root_focus`]), which is `body` having it: no node of the page.
/// Focus moving there is a blur with no `focus` after it, as in a browser.
fn focus_transition<V: 'static>(new_focus: u32, window: &mut Window, cx: &mut Context<V>) {
    // gpui calls focus listeners from inside `Window::draw`, after its frame
    // swap: a forced layout must not draw under them (`crate::in_input_dispatch`).
    let _in_draw = crate::InputDispatch::inside_gpui();
    focus_transition_app(new_focus, window, cx);
    // A focus move can change what `:focus` / `:focus-visible` paint even when
    // no JS handler ran (an `<input>` folds its focus rungs at build time), so
    // the repaint cannot be left to `flush_changes`.
    cx.notify();
}

/// [`focus_transition`]'s events, for a caller with no view: a press's focus
/// ([`apply_press_focus`]) reports its move here, before the next platform
/// event, rather than from the observer a frame later.
fn focus_transition_app(new_focus: u32, window: &mut Window, cx: &mut App) {
    // Per window (docs/MULTIWINDOW.md §5.2): with one slot, window B focusing
    // an input fired `blur` on window A's — a wrong DOM event, not a wrong
    // pixel. gpui hands the observer the window the focus moved in, so "which
    // window" is not inferred here either.
    let k = windows::key(window);
    let new_focus = if new_focus != 0 && new_focus == window_root(k) { 0 } else { new_focus };
    let old = windows::focused_node(k);
    if old == new_focus {
        return;
    }
    if old != 0 {
        windows::set_focused_node(k, 0);
        let payload = EventPayload { a: new_focus as f64, ..Default::default() };
        js_dispatch_app(old, tree::EV_BLUR, &payload, window, cx);
        // `el.focus()` from a handler set the slot (`js_focus`); `el.blur()`
        // cannot have, since nothing is focused.
        if windows::focused_node(k) != 0 {
            return;
        }
    }
    windows::set_focused_node(k, new_focus);
    if new_focus != 0 {
        crate::selection::focus_moved(new_focus);
        let payload = EventPayload { a: old as f64, ..Default::default() };
        js_dispatch_app(new_focus, tree::EV_FOCUS, &payload, window, cx);
    }
}

/// The root this window paints: where gpui's focus rests when the page has
/// none ([`ensure_root_focus`]).
fn window_root(k: windows::WinKey) -> u32 {
    match windows::root_of(k) {
        0 => tree::read_tree(|t| t.root),
        popped => popped,
    }
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

/// The overlay scrollbar for one `overflow: auto | scroll` box.
///
/// Everything CSS gets to say about it is folded here: `scrollbar-width` picks
/// the track width (or suppresses the painting entirely), `scrollbar-color`
/// replaces the thumb and track colours, `overscroll-behavior` decides whether
/// a wheel this box cannot use reaches its ancestors, and — when the author
/// said nothing — the thumb takes its colour from the element's own text, which
/// is the one value that is guaranteed to read against the background the app
/// painted.
///
/// The element is built even when `scrollbar-width: none` hides it: the
/// property means "do not draw", not "do not scroll", and the wheel routing
/// this element carries is what makes `overscroll-behavior` and the
/// innermost-box-wins rule work at all.
fn scrollbar_for<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    handle: &ScrollHandle,
    window: &Window,
    auto_hide: bool,
    cx: &mut Context<V>,
) -> vn_elements::Scrollbar {
    use vn_css::{OverscrollBehavior, ScrollbarWidth};

    let extras = node.computed.as_ref().map(|cs| &cs.extras);
    let text_color = node
        .computed
        .as_ref()
        .and_then(|cs| cs.color)
        .unwrap_or_else(|| gpui::hsla(0., 0., 0.45, 1.0));
    let mut style = vn_elements::ScrollbarStyle::from_text_color(text_color);
    match extras.and_then(|x| x.scrollbar_width) {
        Some(ScrollbarWidth::Thin) => style = style.thin(),
        Some(ScrollbarWidth::None) => style = style.hidden(),
        Some(ScrollbarWidth::Auto) | None => {}
    }
    if let Some((thumb, track)) = extras.and_then(|x| x.scrollbar_color) {
        style = style.with_colors(thumb, track);
    }
    // The platform's own answer to "does a scrollbar fade away": Windows
    // exposes it as `UISettings::AutoHideScrollBars`, and a user who turned it
    // off wants the bar to stay.
    style = style.always_visible(!auto_hide);

    let chains = |b: Option<OverscrollBehavior>| b.map(|b| b.chains()).unwrap_or(true);
    let overscroll = vn_elements::Overscroll {
        x_chains: chains(extras.and_then(|x| x.overscroll_x)),
        y_chains: chains(extras.and_then(|x| x.overscroll_y)),
    };

    // Which axes this box really scrolls, off the FINAL computed overflow —
    // the same value that made `track_scroll` happen.
    let overflow = node.computed.as_ref().map(|cs| cs.base.overflow.clone()).unwrap_or_default();
    // The root is the viewport and always scrolls both ways, whatever the
    // cascade said — the same rule `build_node` applies to its overflow.
    let scrolls_x = overflow.x == Some(Overflow::Scroll) || id == t.root;
    let scrolls_y = overflow.y == Some(Overflow::Scroll) || id == t.root;

    // The DOM events this element has to fire itself.
    //
    // Two paths reach here that gpui's own listener never covers: a wheel the
    // router TOOK OVER (it stops propagation, so the renderer's own
    // `on_scroll` listeners do not run), and a thumb drag or track click,
    // which moves the offset with no wheel event anywhere in sight — so before
    // this, `@scroll` never fired for a scrollbar drag at all.
    //
    // The `wheel` hook dispatches at the wheel's target, the topmost element
    // under the pointer (`crate::pointer`), and the JS runtime propagates the
    // synthesized event over the mirror tree from there, so the container's
    // and its ancestors' `@wheel` fire, and a descendant's on the way. The
    // divergence, and it is the honest one to name: the hook exists only when
    // the container or an ancestor listens (`wheel_listener_above`), so a
    // `@wheel` found only BELOW a taken-over box does not see the wheel.
    let sb = vn_elements::Scrollbar::new(("vn-scrollbar", id), handle.clone(), style)
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
            // `js_dispatch_app` returns "the app did not call
            // preventDefault()", so the hook's answer is its negation.
            let target = crate::pointer::target_or(window, id);
            let at = window.mouse_position();
            crate::pointer_at_next_dispatch(f64::from(at.x), f64::from(at.y));
            !js_dispatch_app(target, tree::EV_WHEEL, &payload, window, cx)
        })
    } else {
        sb
    };
    let scroll = handle.clone();
    sb.on_scrolled(move |window, cx| report_scroll_later(id, &scroll, window, cx))
}

/// [`report_scroll`] once gpui's dispatch is over, from the window the scroll
/// happened in, off input. In a browser `scroll` is asynchronous: it follows
/// the `wheel` that caused it, and a cancelled `wheel` causes none. Here
/// gpui's own wheel listener moves a box before the box's `wheel` listener
/// runs, and a cancelled `wheel` puts the box back
/// (`vn_elements::undo_wheel_scroll`). Reporting later fires `scroll` after
/// `wheel`, and for a box put back, not at all: the report is deduplicated
/// against the last position. Off input, a handler that measures can lay out.
fn report_scroll_later(id: u32, handle: &ScrollHandle, window: &mut Window, cx: &mut App) {
    let handle = handle.clone();
    window.defer(cx, move |window, cx| {
        crate::off_input(|| report_scroll(id, &handle, window, cx));
    });
}

thread_local! {
    /// The DOM `wheel` of the wheel event being dispatched has been fired.
    /// gpui runs every listening box's wheel listener, innermost first, and the
    /// JS side propagates from the target itself, so without the claim an
    /// outer `@wheel` ran once per listening box under the pointer. Cleared by
    /// an effect at the end of the dispatch.
    static WHEEL_CLAIMED: Cell<bool> = const { Cell::new(false) };
}

/// What a key event owes the focused box, once all of its handlers have run.
#[derive(Clone, Copy, Debug)]
pub enum KeyClick {
    /// A keyboard `click` (`detail` 0, which is how a page tells it from a
    /// mouse click). `armed`: a Space release, which also needs its press to
    /// have been armed. `key` (`"Enter"` / `" "`) rides in the payload's `key`,
    /// which a mouse click leaves empty, so the runtime can tell the two apart.
    Click { id: u32, at: Point<gpui::Pixels>, modifiers: gpui::Modifiers, armed: bool, key: &'static str },
    /// A Space press, which arms the click of its release.
    Arm(u32),
}

/// Does a browser click this element from Enter (`space` false) or Space? Only
/// an element with an activation behaviour for that key does (Chromium's
/// `DefaultEventHandler`s):
/// - a `<button>` and a `<summary>`, for both keys;
/// - an `<a href>` or `<area href>`, for Enter only;
/// - a button-like `<input>` (`button`, `submit`, `reset`, `image`, `color`), for
///   both keys;
/// - a checkbox or a radio, for Space only.
///
/// A `<label>` has none, and a `<div role="button" tabindex="0">` has none: it
/// handles its keys itself, and a click from the host would run its action
/// twice. An `<input type=file>` is clicked by the runtime, from the key events
/// it hears.
fn keyboard_activates(id: u32, space: bool) -> bool {
    tree::read_tree(|t| {
        let Some(node) = t.node(id) else { return false };
        let ty = || t.prop(id, "type").map(|v| v.trim().to_ascii_lowercase()).unwrap_or_default();
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

/// Owes `step` for the key event being dispatched. A browser clicks as the
/// DEFAULT action of the key event, after every handler and only if none
/// cancelled it, and the ancestors' key handlers run after the focused box's
/// listener. So the root's last key listener settles it
/// ([`resolve_key_click`]), and a deferred effect does it for a window whose
/// root has none.
fn owe_key_click(step: KeyClick, window: &mut Window, cx: &mut App) {
    windows::here(window, |w| w.key_click = Some(step));
    window.defer(cx, |window, cx| crate::off_input(|| resolve_key_click(window, cx)));
}

/// Settles the key event's owed [`KeyClick`] against `key_prevented`.
fn resolve_key_click(window: &mut Window, cx: &mut App) {
    let Some(step) = windows::here(window, |w| w.key_click.take()) else { return };
    let prevented = windows::here(window, |w| w.key_prevented);
    match step {
        KeyClick::Arm(id) => {
            if !prevented {
                windows::here(window, |w| w.space_armed = Some(id));
            }
        }
        KeyClick::Click { id, at, modifiers, armed, key } => {
            let go = windows::here(window, |w| !armed || w.space_armed.take() == Some(id)) && !prevented;
            if go {
                let payload = EventPayload { key: Some(key), ..mouse_payload_n(at, 0.0, &modifiers, 0) };
                js_dispatch_app(id, tree::EV_CLICK, &payload, window, cx);
            }
        }
    }
}

/// The root's last key listeners: the keyboard click a key event owes is
/// settled there, after every handler of that event (see [`owe_key_click`]).
fn bind_root_key_clicks<E: gpui::InteractiveElement>(e: E) -> E {
    e.on_key_down(|_: &KeyDownEvent, window, cx| resolve_key_click(window, cx))
        .on_key_up(|_: &KeyUpEvent, window, cx| resolve_key_click(window, cx))
}

/// Claims this wheel event's one DOM `wheel`; `false` if a box nearer the
/// pointer (or the scroll router) already fired it.
fn claim_wheel(cx: &mut App) -> bool {
    if WHEEL_CLAIMED.with(|c| c.replace(true)) {
        return false;
    }
    cx.defer(|_| WHEEL_CLAIMED.with(|c| c.set(false)));
    true
}

/// Does this node, or any ancestor, listen for `wheel`? A DOM `wheel` event
/// synthesized from a scroll container propagates up the mirror tree, so an
/// ancestor's handler is reason enough to pay for the dispatch — and nothing
/// listening anywhere is reason not to.
fn wheel_listener_above(t: &Tree, id: u32) -> bool {
    let mut cur = id;
    loop {
        let Some(node) = t.node(cur) else { return false };
        if node.listeners & (1u64 << tree::EV_WHEEL) != 0 {
            return true;
        }
        if node.parent == 0 {
            return false;
        }
        cur = node.parent;
    }
}

/// The DOM `scroll` event for a node whose offset just moved, deduplicated
/// against the last position seen for it. A box starts at the origin, so a
/// wheel cancelled on a box that never scrolled reports nothing. A move seen
/// while nothing listened is still recorded.
pub(crate) fn report_scroll(id: u32, handle: &ScrollHandle, window: &mut Window, cx: &mut App) {
    // gpui offsets are <= 0 (content shifted up/left); CSS scrollTop/scrollLeft
    // count down/right as positive.
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
        t.node(id).is_some_and(|n| n.listeners & (1u64 << tree::EV_SCROLL) != 0)
    });
    if moved && listens {
        let payload = EventPayload { a: left as f64, b: top as f64, ..Default::default() };
        js_dispatch_app(id, tree::EV_SCROLL, &payload, window, cx);
    }
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

/// HTML's focusable-by-default elements, which take tab index 0 with no
/// `tabindex` attribute:
/// - `<button>`, `<select>`;
/// - `<a href>`, `<area href>`;
/// - the `<summary>` that summarizes its `<details>`;
/// - an editing host (`contenteditable`).
///
/// `<input>` and `<textarea>` are the native widget's business (`build_input`).
/// An actually-disabled control is excluded here too: see
/// [`actually_disabled`].
fn default_tab_index(t: &Tree, id: u32) -> Option<isize> {
    let n = t.node(id)?;
    let focusable = match t.name(n.tag) {
        "button" | "select" => true,
        "a" | "area" => t.prop(id, "href").is_some(),
        "summary" => summarizes_details(t, id),
        _ => false,
    } || t.prop(id, "contenteditable").is_some_and(|v| !v.eq_ignore_ascii_case("false"));
    focusable.then_some(0)
}

/// Is `id` the first `<summary>` child of a `<details>`?
fn summarizes_details(t: &Tree, id: u32) -> bool {
    let Some(parent) = t.node(id).and_then(|n| t.node(n.parent)) else { return false };
    t.name(parent.tag) == "details"
        && parent
            .children
            .iter()
            .copied()
            .find(|&c| t.node(c).is_some_and(|k| k.kind == Kind::Element && t.name(k.tag) == "summary"))
            == Some(id)
}

/// HTML's "actually disabled": a `<button>`, `<input>`, `<select>`,
/// `<textarea>`, `<optgroup>`, `<option>` or `<fieldset>` with `disabled`, or
/// a form control inside a disabled `<fieldset>` but not inside its first
/// `<legend>`. Such an element is not focusable at all, whatever its
/// `tabindex`, and neither a click nor `el.focus()` focuses it.
pub(crate) fn actually_disabled(t: &Tree, id: u32) -> bool {
    let disabled = |id: u32| t.prop(id, "disabled").is_some_and(|v| v != "false");
    let Some(n) = t.node(id) else { return false };
    let tag = t.name(n.tag);
    if !matches!(tag, "button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset") {
        return false;
    }
    if disabled(id) {
        return true;
    }
    if tag == "optgroup" || tag == "option" {
        return false;
    }
    // A disabled fieldset disables its descendants, except those in its first
    // legend.
    let (mut child, mut cur) = (id, n.parent);
    for _ in 0..256 {
        let Some(p) = t.node(cur) else { return false };
        if t.name(p.tag) == "fieldset" && disabled(cur) {
            let first_legend = p
                .children
                .iter()
                .copied()
                .find(|&c| t.node(c).is_some_and(|k| k.kind == Kind::Element && t.name(k.tag) == "legend"));
            return first_legend != Some(child);
        }
        (child, cur) = (cur, p.parent);
    }
    false
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
    let _profile_window = vn_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
    let _profile = vn_runtime::redjs::profiling::span("render", "Build native element tree");
    BUILDS.with(|c| c.set(c.get().wrapping_add(1)));
    LAYOUTS.with(|c| c.set(c.get().wrapping_add(1)));
    // ONE clock read per frame: nodes animating together must sample the same
    // instant or they skew.
    let now = std::time::Instant::now();
    let span = crate::perf::Span::start();
    BUILT.with(|c| c.set(0));
    // One `<label for>` index per frame, built lazily and only if a control
    // needs a name it cannot get any other way — see `crate::aria`.
    crate::aria::reset_frame();
    // …and the scroll-chain registry the wheel router walks (`vn_elements`).
    vn_elements::scrollbar::reset_frame();
    advise_elliptical_corners();
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
    arm_pick_hitboxes(window);
    crate::cache::begin_frame(window, now);
    crate::cull::begin_frame(window, now);
    reset_stacking();
    let el = tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            // The root inherits nothing: `window-drag`'s initial value is
            // `none`, spelled here as "nobody declared anything".
            if let Some(id) = crate::fullscreen::current(t).filter(|id| windows::window_of_node_in(t, *id) == win) {
                build_fullscreen(t, id, t.root, window, cx, now)
            } else {
                build_node(
                    t, t.root, t.root, window, cx, now, None, None, None, None, Stacking::default(),
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
        .id("vn-fullscreen-document")
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
    let _profile_window = vn_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
    let _profile = vn_runtime::redjs::profiling::span("render", "Build secondary window tree");
    LAYOUTS.with(|c| c.set(c.get().wrapping_add(1)));
    let now = std::time::Instant::now();
    // Per WINDOW, and a pop-out is a window of its own.
    arm_pick_hitboxes(window);
    let win = windows::key(window);
    // Registered from the build itself, so "which window paints this node"
    // cannot drift from what is actually on screen — it is what
    // `windows::window_of_node` walks to, and therefore what routes atlas
    // tiles, `@media` environments and hit tests.
    windows::set_painted_root(win, id);
    reset_bounds(win);
    crate::aria::reset_frame();
    // …and the scroll-chain registry the wheel router walks (`vn_elements`).
    vn_elements::scrollbar::reset_frame();
    windows::of(win, |w| {
        w.any_drag_region = false;
        w.control_hitboxes.clear();
    });
    crate::cache::begin_frame(window, now);
    crate::cull::begin_frame(window, now);
    reset_stacking();
    tree::read_tree(|t| {
        if t.node(id).is_none() {
            return Empty.into_any_element();
        }
        // A pop-out keeps its DOM ancestry even though it paints elsewhere.
        let mut element = if let Some(fullscreen) = crate::fullscreen::current(t).filter(|node| windows::window_of_node_in(t, *node) == win) {
            build_fullscreen(t, fullscreen, id, window, cx, now)
        } else {
            build_node(t, id, id, window, cx, now, None, None, None, None, Stacking::default())
        };
        let mut ancestor = t.node(id).map_or(0, |node| node.parent);
        while let Some(node) = t.node(ancestor) {
            if let Some(matrix) = node.computed.as_ref().and_then(|computed| computed.extras.filter_url.as_ref())
                .and_then(|url| crate::filter::resolve(t, url, ancestor)) {
                let mut layer = div().size_full();
                layer.style().vn_color_matrix = Some(matrix);
                element = layer.child(element).into_any_element();
            }
            ancestor = node.parent;
        }
        element
    })
}

/// Everything `build_node` threads DOWN the walk rather than folding into a
/// node's own style: the inherited values CSS has and vn-css does not carry, plus
/// the stacking context the children sort inside.
///
/// It is a struct rather than five arguments because the retained-render cache
/// has to compare it: a cached subtree is lowered from its node id and this, so
/// a change to any of them at an ancestor changes the subtree's pixels without
/// touching a single node inside it (`crate::cache`).
#[derive(Clone, Default)]
pub(crate) struct Inherited {
    pub(crate) drag: Option<WindowDrag>,
    pub(crate) case: Option<vn_css::TextTransform>,
    pub(crate) ws: Option<vn_css::WhiteSpaceMode>,
    pub(crate) align: Option<gpui::TextAlign>,
    pub(crate) stack: Stacking,
}

impl PartialEq for Inherited {
    fn eq(&self, other: &Self) -> bool {
        self.drag == other.drag
            && self.case == other.case
            && self.ws == other.ws
            && self.align == other.align
            // The stacking KEY is left out on purpose. A boundary's subtree
            // holds no deferred draw (`cache::node_blocks`), so no key inside
            // it is ever used — and its ordinals change whenever anything
            // earlier in the document does, which would make every boundary
            // after an insertion stale.
            && self.stack.deferred == other.stack.deferred
    }
}

/// Lowers ONE cache boundary's subtree — `crate::cache::CachedSubtree`'s
/// `render`. It is `build_node` with the inherited context restored from the
/// recording rather than from a live walk, and with the ids it lowers collected
/// so the boundary can carry their rectangles on a replay frame.
pub(crate) fn build_cached<V: 'static>(
    id: u32,
    root: u32,
    ctx: Inherited,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let _profile_window = vn_runtime::redjs::profiling::WindowScope::enter(windows::key(window));
    let _profile = vn_runtime::redjs::profiling::span("render", "Build cached subtree");
    let now = std::time::Instant::now();
    let span = crate::perf::Span::start();
    let before = BUILT.with(|c| c.get());
    // A boundary renders inside gpui's prepaint, after the build that placed
    // it: no context of that build is open any more.
    let open = OPEN_CONTEXTS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let (el, ids) = tree::read_tree(|t| {
        let el =
            build_node(t, id, root, window, cx, now, ctx.drag, ctx.case, ctx.ws, ctx.align, ctx.stack);
        // The WHOLE subtree, not only what this render lowered: a nested
        // boundary's own nodes are recorded here too, because when this one
        // replays nothing else will ever carry their rectangles
        // (`cache::Entry::ids`).
        let mut ids = Vec::new();
        crate::cache::subtree_ids(t, id, &mut ids);
        (el, ids)
    });
    OPEN_CONTEXTS.with(|c| *c.borrow_mut() = open);
    crate::cache::note_rendered(id, ids);
    // The build cost lands in the `build` bucket wherever it is paid, so the
    // VN_PERF partition still adds up: a boundary's render runs inside gpui's
    // prepaint, and without this it would be attributed to prepaint instead.
    crate::perf::note_build_extra(span.ms(), BUILT.with(|c| c.get()) - before);
    el
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
    // The window this build is lowering into. gpui handed it to us; nothing is
    // inferred (docs/MULTIWINDOW.md §5.6).
    let win = windows::key(window);
    // A subtree outside every clip around it paints nothing: it is lowered as
    // a box of its last size instead, ahead of the cache, which would still
    // have to build or replay it (`crate::cull`).
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
    // A cache boundary returns the ENTIRE node — probe, stacking wrapper and
    // all — because its recording already contains them (`crate::cache`). The
    // `active` gate is one `Cell` read, so a build with caching switched off
    // (or refused for the frame) pays nothing per node for it.
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
    // `window-drag` is inherited (the CSSWG resolved that explicitly) and
    // vn-css has no inherited properties at all, so the fold happens here, on
    // the way down. A browser allows the property on a replaced element too,
    // hence `<img>` and `<lottie>` get the same treatment.
    let extras = tree::animated_extras(id, now).unwrap_or_else(|| extras_of(node));
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
    // Where the box paints (`Stacking`): a stacking context's children are
    // built inside it, so their own z-indices sort INSIDE it; a z-auto
    // `absolute` box is deferred in the context it is in. The root of the
    // build IS the root context, and paints first.
    let parent_display = t.node(node.parent).and_then(|p| p.computed.as_ref()).and_then(|cs| cs.base.display);
    let own_position = node.computed.as_ref().and_then(|cs| cs.base.position);
    let effect = node.computed.as_ref().is_some_and(|cs| effect_context(&extras, &cs.base));
    let layer = if id == root {
        Layer::InFlow
    } else {
        layer_of(&extras, own_position, effect, parent_display)
    };
    let Entered { own, inside: stacking, opens, negative } =
        parent_stack.enter(layer, next_stack_ordinal());
    if negative {
        note_negative_z();
    }
    // The root context's children, and a context's, are built with the context
    // open, so a negative z among them is seen (`split_in_flow`).
    let opens = opens || id == root;
    advise_relative_tree_order(&extras, own_position, layer);
    let (drag, control_area) = window_control_of(&extras, parent_drag);
    advise_interactive_in_drag_region(t, node, &extras, drag);
    advise_double_action(t, node, &extras);
    advise_out_of_flow(t, id, node, &extras, windows::key(window));
    // Set once the children are built, if they had to split (`split_in_flow`).
    let split = Cell::new(false);
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
            // Web fonts load lazily: text drawn with a waiting face asks for it.
            crate::fonts::want_for_text(t, node.parent, &collapsed);
            match trim_edges(t, node, id, &collapsed, ws) {
                // A text node that is nothing but the white space at the end
                // of a line renders nothing, exactly as in a browser.
                "" => Empty.into_any_element(),
                // The plain text element unless a selection paints on it or a
                // drag is recording (`crate::selection`).
                text => {
                    let lead = text.as_ptr() as usize - collapsed.as_ptr() as usize;
                    let text = crate::selection::text_element(t, id, &node.text, ws, lead, text, case);
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
        // Metadata content renders NOTHING in a browser — it is not a box, it
        // is instructions. reka-ui injects a `<style>` element at runtime to
        // hide a scroll container's scrollbars, and painting its source as text
        // put a paragraph of CSS at the top of the page.
        //
        // A `<style>` is not silently ignored, though: `tree::sync_runtime_sheets`
        // installs its CSS into the cascade. Rendering nothing and MEANING
        // nothing are two different things, and only the first is the browser's
        // behaviour — which is why the advisory that used to stand here is gone.
        Kind::Element if is_metadata(t.name(node.tag)) => Empty.into_any_element(),
        // `stack` moves the PAINT into gpui's deferred layer for the boxes
        // `Layer` defers (`position: fixed`, a `z-index`, …); the
        // window-control hitbox rides the inner element either way, and the
        // hit test follows the paint (`Stacking`) — which is the order the
        // fork's topmost-first resolution (F-01) already wants.
        Kind::Element if t.name(node.tag) == "img" => stack(
            own.clone(),
            &extras,
            probe(id, build_img(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element if t.name(node.tag) == "lottie" => stack(
            own.clone(),
            &extras,
            probe(id, build_lottie(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element if t.name(node.tag) == "video" => stack(
            own.clone(),
            &extras,
            probe(id, build_video(t, id, node, window, cx, now, control_area)),
        ),
        // vue-native's own: stickers and animated emoji (`sticker` below).
        Kind::Element if t.name(node.tag) == "vn-sticker" => stack(
            own.clone(),
            &extras,
            probe(id, sticker::build(t, id, node, window, cx, now, control_area)),
        ),
        // The bitmap the page drew, painted into the box (`canvas` below).
        Kind::Element if t.name(node.tag) == "canvas" => stack(
            own.clone(),
            &extras,
            probe(id, canvas::build(t, id, node, window, cx, now, control_area)),
        ),
        // Inline `<svg>` is a LEAF: the subtree is serialized into a document
        // resvg rasterises, so its children must not also become taffy nodes
        // (see crate::svg).
        Kind::Element if t.name(node.tag) == "svg" => stack(
            own.clone(),
            &extras,
            probe(id, build_svg(t, id, node, window, cx, now, control_area)),
        ),
        Kind::Element => stack_box(own, &split, id == root, &stacking, &extras, probe_at(id, id == root, {
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
            advise_missing_font();
            // A `url()` layer anywhere in the stack moves the WHOLE stack into
            // a painter child (`crate::bgimage`), so gradients and images keep
            // their CSS order; gpui keeps only the colour.
            let bg_stack = extras.resolve_background_stack();
            let bg_radii = bg_stack.as_ref().map(|_| padding_box_radii(d.style(), window.rem_size()));
            apply_background_layers(d.style(), &extras, bg_stack.is_some());
            apply_background_clip_text(d.style(), &extras, id as u64);
            apply_self_blur(d.style(), t, id, node, &extras);
            apply_drop_shadow(
                d.style(),
                &extras,
                node.computed.as_ref().and_then(|cs| cs.color),
                || node_prefix(t, id),
            );
            apply_mask_ring(d.style(), &extras, bg_stack.is_some(), paints_children(t, node), || {
                node_prefix(t, id)
            });
            apply_inline_row(d.style(), t, node, align);
            apply_inline_item_minimum(d.style(), t, node);
            apply_inline_align(d.style(), t, node, &extras, align);
            apply_button_content_box(d.style(), t, node);
            advise_inline_level(t, id, node, &extras, d.style());
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
                let h = focus_handle_for(id, None, cx);
                d = d.track_focus(&h);
                // Watched with no `tabindex` too: gpui's focus coming back here
                // is `body` getting it, the blur of whatever had it
                // (`focus_transition`).
                watch_focus(id, &h, window, cx);
            } else if id == crate::cache::boundary_root() {
                // A cache boundary is laid out AS A ROOT inside the box its
                // wrapper reserved (`Entity::cached` → `layout_as_root`), and a
                // root box with `width: auto` shrinks to fit its content
                // instead of stretching to its parent — which is not what the
                // node's own place in the flow said. Same `get_or_insert` as
                // the build root above, and for the same reason: fill the box
                // you were given, unless a rule already said otherwise.
                //
                // NOT the viewport treatment, though: no `Overflow::Scroll` and
                // no root focus handle. This node is a node, not a document.
                let style = d.style();
                let full = gpui::Length::Definite(gpui::relative(1.0));
                style.size.width.get_or_insert(full);
                style.size.height.get_or_insert(full);
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
                // Where this container stood when the frame was built, so a
                // `getBoundingClientRect()` taken after a scroll and before the
                // next paint can correct for the difference (`crate::scroll`).
                crate::scroll::note_built(id, h);
            }

            let mask = if id == root { root_mask(t, node) } else { pointer_mask(node) };
            let bit = move |ev: u32| mask & (1u64 << ev) != 0;
            // `<input>` and `<textarea>` are the SAME native widget — only
            // `TextInputOptions::multiline` differs (see `build_input`), so
            // everything the wrapper does for one it does for the other.
            //
            // `<input type=file>` is NOT that widget: it is a button and a
            // status line (`file_input_children`), and the runtime opens the
            // picker from its click. It keeps the ordinary wrapper's focus
            // handle, so Enter and Space reach it as keydown/keyup.
            let is_file_input = crate::aria::is_file_input(t, id);
            let is_range = crate::range::is_range(t, id);
            let is_color = t.name(node.tag) == "input" && t.prop(id, "type").is_some_and(|v| v.trim().eq_ignore_ascii_case("color"));
            let is_input = matches!(t.name(node.tag), "input" | "textarea") && !is_file_input && !is_range && !is_color;
            // Both are tabbable by default in HTML; a plain <div> is not. A
            // disabled file input is not focusable at all.
            let file_tab = is_file_input && !t.prop(id, "disabled").is_some_and(|v| v != "false");
            // So are a <button>, a <select>, a link and the rest of HTML's
            // focusable-by-default list, and an actually-disabled control is
            // not focusable whatever its tabindex (m26 #10: reka's FocusScope
            // could not focus a dialog's first button).
            let disabled = !is_input && actually_disabled(t, id);
            let tab_index = tab_index_of(t, id)
                .or((is_input || file_tab || is_range || is_color).then_some(0))
                .or_else(|| default_tab_index(t, id))
                .filter(|_| !disabled);

            // Focus and tab navigation need a persistent FocusHandle.
            // `<input>` owns its own focus handle inside the native widget —
            // it is decorated through TextInputOptions instead. Only HTML's
            // focusable areas take focus: a `tabindex`, or an element focusable
            // by default. A key or focus LISTENER makes nothing focusable (FO-3,
            // docs/RUNTIME.md §5.2): a `<span @focus>` never takes focus in a
            // browser, and a container's `@keydown` hears its focused
            // descendants' keys by bubbling.
            let focusable = !is_input && !disabled && tab_index.is_some();
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
            d = bind_events(d, id, mask, click_focus, focusable, !is_input, cx);
            // A `<select>` opens its popup as a press's default action
            // (`crate::select`).
            let is_select = t.name(node.tag) == "select";
            if is_select && !disabled {
                d = crate::select::bind_press(d, id);
            }
            if is_range && !disabled && extras.pointer_events_none != Some(true) {
                d = crate::range::bind_press(d, id);
            }
            // Copy / cut / paste outside a text field: the root is on every
            // keystroke's path, and its listener (bound AFTER its own keydown
            // one, so it runs after every keydown handler) is the last stop.
            if id == root {
                d = crate::clipboard::bind_root(d, id);
                // Every mouse-up's wave (`bind_events`), and the keyboard's
                // `contextmenu`: the Menu key and Shift+F10, bound after the
                // clipboard keys so they too run after every key handler.
                d = bind_root_mouse(d);
                // A press that may start a text selection, and Ctrl+A.
                d = crate::selection::bind_root(d);
                d = bind_menu_keys(d, id);
                // A `<select>`'s keys: its popup's before the page's, a closed
                // one's after them.
                d = crate::select::bind_root(d);
                d = crate::range::bind_root(d);
                // …and the keyboard click a key event owes, last of all.
                d = bind_root_key_clicks(d);
            }

            // `scroll` reports the position *after* the move, once the wheel's
            // dispatch is over (`report_scroll_later`): after the `wheel`, and
            // not at all when a cancelled `wheel` put the box back.
            if let Some(scroll) = scroll_handle.clone().filter(|_| bit(tree::EV_SCROLL)) {
                d = d.on_scroll(cx.listener(move |_, _: &ScrollEvent, window, cx| {
                    report_scroll_later(id, &scroll, window, &mut **cx);
                }));
            }

            if is_range {
                d.style().size.width.get_or_insert(gpui::px(129.).into());
                d.style().size.height.get_or_insert(gpui::px(16.).into());
                d.child(crate::range::children(t, id, window)).into_any_element()
            } else if is_color {
                crate::color::defaults(d.style());
                d.child(crate::color::children(t, id, node, window)).into_any_element()
            } else if is_input {
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
                        // A text field shows focus whatever moved it there, a
                        // click included, as in Chromium; anything else only
                        // after the keyboard (`crate::live_state`).
                        if let Some(f) = &cs.focus_visible {
                            if window.last_input_was_keyboard()
                                || crate::live_state::is_text_field(t, id)
                            {
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
                let own_display = node.computed.as_ref().and_then(|cs| cs.base.display);
                let mut kids: Vec<AnyElement> = Vec::new();
                if opens {
                    open_context();
                }
                // FIRST, so it paints under every real child (`crate::bgimage`).
                if let (Some(stack), Some(radii)) = (bg_stack, bg_radii) {
                    kids.push(crate::bgimage::layer_element(id, stack, radii));
                }
                // The background, which stays in the box's own paint.
                let background_kids = kids.len();
                if let Some(marker) = list_marker_for(t, id, node, &extras) {
                    kids.push(marker);
                }
                if let Some(pb) = before {
                    crate::fonts::want_for_pseudo(t, id, &pb);
                    kids.push(pseudo_element(&pb, case, &stacking, own_display));
                }
                if is_file_input {
                    kids.push(file_input_children(t, id, node, case));
                } else if is_select {
                    // The menulist Chromium draws, not the options as boxes.
                    let skin = extras.appearance_none != Some(true);
                    kids.extend(crate::select::closed_children(t, id, skin, case));
                    if let Some(popup) = crate::select::popup_child(t, id, node, case, window) {
                        kids.push(popup);
                    }
                } else if !node.text.is_empty() {
                    // `setElementText` — what Vue calls for an element whose
                    // only child is text, which is most of them. The text IS
                    // the element's whole content, so both its ends are the
                    // ends of a line (see `trim_own_edges`). Collapse first,
                    // then trim, exactly as the text-node arm does.
                    let collapsed =
                        processed_text(&node.text, ws.unwrap_or(vn_css::WhiteSpaceMode::Normal));
                    crate::fonts::want_for_text(t, id, &collapsed);
                    let text = trim_own_edges(t, node, &collapsed, ws);
                    if !text.is_empty() {
                        let lead = text.as_ptr() as usize - collapsed.as_ptr() as usize;
                        kids.push(crate::selection::text_element(t, id, &node.text, ws, lead, text, case));
                    }
                } else {
                    kids.extend(
                        laid_out_children(t, node)
                            .into_iter()
                            .map(|c| {
                                build_node(
                                    t, c, root, window, cx, now, drag, case, ws, align,
                                    stacking.clone(),
                                )
                            }),
                    );
                }
                if let Some(pb) = after {
                    crate::fonts::want_for_pseudo(t, id, &pb);
                    kids.push(pseudo_element(&pb, case, &stacking, own_display));
                }
                // The overlay scrollbar, and the wheel routing that goes with
                // it. LAST child, so its hitboxes sit above the content's and
                // its capture-phase wheel listener is registered after this
                // container's own — which, since capture runs in registration
                // order and a container registers before painting its children,
                // is what makes the INNERMOST scroll box get the wheel first.
                if let Some(h) = &scroll_handle {
                    let auto_hide = cx.should_auto_hide_scrollbars();
                    kids.push(
                        scrollbar_for(t, id, node, h, window, auto_hide, cx).into_any_element(),
                    );
                }
                // A negative z landed in this context: its in-flow content
                // paints apart from its background, after the negative ones.
                if opens && close_context() {
                    split.set(true);
                    kids = split_in_flow(kids, background_kids, &stacking);
                }
                // A cache wrapper owns the resolved outer box. Keep its root
                // from resolving authored percentages and margins a second time.
                crate::cache::normalize_inner_root(id, d.style());
                d.children(kids).into_any_element()
            }
        })),
    }
}

/// Says so when a `font-family` list's FIRST choice is a font the host does not
/// have.
///
/// The substitution itself is correct — vn-css takes the first installed
/// family in the list, CSS's "first available font", as a browser does — but
/// the SILENCE was not, and this is the one gap on apps/meet-probe that changed
/// how much text fitted on a line: the app asks for Inter, ships Inter as a
/// `@font-face` in a stylesheet vue-native never loads (there is no
/// `@font-face` and no `<link rel=stylesheet>`; the sheet is the SFC `<style>`
/// blocks plus Tailwind), and gets the platform UI font, which is wider. The
/// hero subtitle wraps to two lines where a browser fits it on one, and nothing
/// anywhere said why.
///
/// Only a missing FIRST entry is reported: `ui-monospace, SFMono-Regular, …,
/// Consolas` landing on Consolas is exactly what a browser on this machine
/// does. The misses are recorded where the list is mapped
/// (`vn_css::take_font_misses`) and drained here, one-shot per family name, so
/// a page asking for a missing font on 300 nodes reports it once.
///
/// Since m26 (#29) a stylesheet's `@font-face` and a script's `FontFace` are
/// loaded (`crate::fonts`), so a miss is handed to [`crate::fonts::hold_miss`]
/// first: a family a face declares, now or within a short grace (a script
/// that builds its `FontFace` after the first paint), is not reported, and a
/// load that fails says so itself. What is left comes back through
/// [`warn_missing_font`].
fn advise_missing_font() {
    for (family, used) in vn_css::take_font_misses() {
        crate::fonts::hold_miss(family, used);
    }
}

/// The missing-font advisory for one family nothing declared.
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

/// `background-clip: text` retains the CSS brush as a text-only fill.
///
/// The background stack is retained as a brush and painted through descendant
/// glyph alpha. Glyph layout, text selection, decorations and shadows stay in
/// their existing paths; no filled background rectangle is emitted.
fn apply_background_clip_text(style: &mut gpui::StyleRefinement, extras: &vn_css::Extras, id: u64) {
    if let Some(vn_css::TextFill::Color(fill)) = extras.text_fill_color {
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
    style.vn_text_fill = Some(gpui::VnTextFill { group_id: id, backgrounds });
    style.background = Some(gpui::Fill::Color(gpui::transparent_black().into()));
    style.background_layers = Some(Vec::new());
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
///
/// `painted_by_child`: the stack holds a `url()` image, and `crate::bgimage`
/// paints all of it, so gpui must paint none of it.
fn apply_background_layers(
    style: &mut gpui::StyleRefinement,
    extras: &vn_css::Extras,
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

/// The corner radii of an element's PADDING box, which is where
/// `crate::bgimage` paints and what `clip_replaced` clips to.
///
/// A length corner is its border radius less the border it sits in. A
/// relative corner (a percentage, a calc() with one, an `a / b` ellipse) is
/// kept as written and measured against the padding box it is painted on
/// ([`PaddingRadii::resolve`], or gpui's paint for a styled box). CSS measures
/// it against the border box and then takes the border off. For `50%` the two
/// agree exactly, and any other percentage lands within one border width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaddingRadii {
    lengths: gpui::Corners<gpui::AbsoluteLength>,
    relative: gpui::Corners<Option<gpui::VnCornerRadius>>,
}

impl PaddingRadii {
    /// The radii a padding box of `size` paints, CSS's overlap rule included.
    pub(crate) fn resolve(&self, size: gpui::Size<gpui::Pixels>, rem: gpui::Pixels) -> gpui::Corners<gpui::Pixels> {
        gpui::vn_resolve_corner_radii(&self.lengths, &self.relative, size, rem)
    }

    fn is_square(&self) -> bool {
        let zero = gpui::AbsoluteLength::from(gpui::px(0.));
        let l = &self.lengths;
        self.relative == gpui::Corners::default()
            && [l.top_left, l.top_right, l.bottom_right, l.bottom_left].iter().all(|c| *c == zero)
    }

    /// Gives `style` these corners.
    fn apply(&self, style: &mut gpui::StyleRefinement) {
        let c = &mut style.corner_radii;
        (c.top_left, c.top_right) = (Some(self.lengths.top_left), Some(self.lengths.top_right));
        (c.bottom_right, c.bottom_left) =
            (Some(self.lengths.bottom_right), Some(self.lengths.bottom_left));
        let v = &mut style.vn_corner_radii;
        (v.top_left, v.top_right) = (Some(self.relative.top_left), Some(self.relative.top_right));
        (v.bottom_right, v.bottom_left) =
            (Some(self.relative.bottom_right), Some(self.relative.bottom_left));
    }
}

fn padding_box_radii(style: &gpui::StyleRefinement, rem: gpui::Pixels) -> PaddingRadii {
    let radius = |v: Option<gpui::AbsoluteLength>| v.map_or(0.0, |v| f32::from(v.to_pixels(rem)));
    let border = |v: Option<gpui::AbsoluteLength>| v.map_or(0.0, |v| f32::from(v.to_pixels(rem)));
    let (top, right) = (border(style.border_widths.top), border(style.border_widths.right));
    let (bottom, left) = (border(style.border_widths.bottom), border(style.border_widths.left));
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

/// gpui draws corners as circles. When it had to draw one where CSS asks for
/// an ellipse (`50%` on a box that is not square, or an `a / b` radius), it
/// says so, and this passes it on once.
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

/// A replaced element's picture, clipped to the element's rounded corners.
///
/// A `<video>` frame, a `<canvas>` bitmap and a video poster are an `img`
/// child of the element's box, and CSS clips a replaced element's content to
/// its rounded edge. A full-size `overflow: hidden` wrapper with the
/// padding-box radii pushes that rounded content mask. Making the element's
/// own box `overflow: hidden` would also zero its automatic minimum size in a
/// flex row, so the wrapper sits inside it. Without a radius the picture goes
/// in unwrapped. (`<img>` needs none of this: it is the gpui `img` itself, and
/// its sprite takes the radii.)
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
    clip.into_any_element()
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
pub(crate) const INLINE_LEVEL_TAGS: &[&str] = &[
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
/// block-level equivalent (see [`advise_inline_level`]) and a block container
/// lays every child out on its own row. For a container whose
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

// A synthesized inline row uses flex only because taffy has no inline
// formatting context. Its text must not inherit a flex item's automatic
// min-content floor: `overflow-wrap: break-word` deliberately excludes its
// emergency breaks from min-content sizing, so one long word otherwise paints
// outside the paragraph whenever a timestamp or styled span sits beside it.
fn synthetic_inline_item(t: &Tree, node: &crate::tree::Node) -> bool {
    let Some(parent) = t.node(node.parent) else { return false };
    let display = parent.computed.as_ref().and_then(|style| style.base.display);
    inline_row_applies(t, parent, display)
}

fn apply_inline_item_minimum(style: &mut gpui::StyleRefinement, t: &Tree, node: &crate::tree::Node) {
    let declared = node.computed.as_ref().and_then(|computed| computed.base.display);
    if synthetic_inline_item(t, node)
        && !matches!(declared, Some(gpui::Display::Flex) | Some(gpui::Display::Grid))
        && style.size.width.is_none()
    {
        style.min_size.width.get_or_insert(gpui::px(0.0).into());
    }
}

/// Does [`apply_inline_row`] fire for this container? Split out so that
/// [`advise_inline_level`] can ask the question from the CHILD's side — an
/// inline-level box whose parent became a row is laid out the way a browser
/// lays it out, and has nothing to advise about.
fn inline_row_applies(
    t: &Tree,
    node: &crate::tree::Node,
    display: Option<gpui::Display>,
) -> bool {
    // An author-declared display is authority: `display: flex` on a paragraph
    // means flex, and `display: grid` means grid.
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
        let Some(kid) = t.node(child) else { return false };
        match kid.kind {
            Kind::Text => has_text = true,
            Kind::Element => {
                if !INLINE_LEVEL_TAGS.contains(&t.name(kid.tag)) {
                    return false;
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
                    return false;
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
    has_text || has_element
}

/// `text-align` on a block container centres its INLINE-level children.
///
/// taffy has no inline formatting context, so `display: inline-flex` is laid
/// out as `flex` and the box becomes block-level (see [`advise_inline_level`]).
/// Everything about the box is then right except its
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
///
/// **Only in a BLOCK container** (m26). In a flex or grid container an
/// inline-level child is blockified into a flex/grid item, and `text-align`
/// does not place items at all — `justify-content` / `justify-items` do. The
/// auto margins were therefore wrong there, and in flexbox they actively
/// move the item, because auto margins take the free space before
/// `justify-content` sees it. The Argon client's `.icon-box` (`inline-flex`, in a
/// `flex justify-center` div that inherits `text-center`) landed at the right
/// edge. The same holds for a parent that [`apply_inline_row`] turned into a
/// flex row: its own `justify-content` already carries the `text-align`.
fn apply_inline_align(
    style: &mut gpui::StyleRefinement,
    t: &Tree,
    node: &crate::tree::Node,
    extras: &vn_css::Extras,
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

/// HTML's button layout: unless a `<button>` is a flex or grid container, its
/// content sits in an anonymous box that is centred vertically while it fits
/// and starts at the top when it overflows (the HTML standard's Rendering
/// section, "The button element"). There is no UA stylesheet and no anonymous
/// box here, so the
/// button's own container gets `align-content: safe center`, which is the same
/// picture. taffy's block layout honours it, and so does the wrapping flex row
/// [`apply_inline_row`] makes of a button holding text beside an icon. An
/// author's `align-content` wins.
///
/// @argon/ui's radio relies on it: reka's indicator is a zero-height `<span>`
/// and its dot is positioned at `top: 50%` of that span, so only the centring
/// puts the dot in the middle of the circle (m26 client sweep #36).
fn apply_button_content_box(style: &mut gpui::StyleRefinement, t: &Tree, node: &crate::tree::Node) {
    if style.align_content.is_some() || t.name(node.tag) != "button" {
        return;
    }
    let declared = node.computed.as_ref().and_then(|cs| cs.base.display);
    if matches!(declared, None | Some(gpui::Display::Block)) {
        style.align_content = Some(gpui::AlignContent::SafeCenter);
    }
}

/// Is `node` laid out by a flex or grid container: a parent whose display is
/// flex or grid, or one [`apply_inline_row`] turns into a flex row?
fn parent_is_flex_or_grid(t: &Tree, node: &crate::tree::Node) -> bool {
    let Some(parent) = t.node(node.parent) else { return false };
    let display = parent.computed.as_ref().and_then(|cs| cs.base.display);
    match display {
        Some(gpui::Display::Flex) | Some(gpui::Display::Grid) => true,
        _ => inline_row_applies(t, parent, display),
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
    node_id: u32,
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
            // The shadow IS the box here, so its middle stays filled: a CSS
            // outer shadow is cut out of the border box, this one must not be.
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

/// `filter: drop-shadow(<dx> <dy> <blur> <color>)` on a box whose painted
/// output IS its own rectangle: gpui's shadow primitive, exactly.
///
/// CSS's `drop-shadow()` blurs the ALPHA of what the element painted and lays
/// the result underneath it. gpui's `BoxShadow` rasterises a Gaussian-blurred
/// rounded rect with the element's own corner radii — which is precisely that
/// alpha, for an element whose alpha IS an opaque rounded rect. The sigma
/// transfers 1:1 for the same reason `blur()`'s does (both CSS and
/// `shaders.hlsl` define it as a sigma), and `rounded-full` casts a round
/// shadow.
///
/// So the shape test is **an opaque solid background**. Children and text do
/// not disqualify it, unlike [`apply_self_blur`]: they paint ON TOP of an
/// opaque box and cannot change its silhouette. A translucent background does
/// disqualify it — CSS would carry that alpha into the shadow and gpui's
/// shadow is drawn at full strength — and so does no background at all, where
/// the silhouette is the glyphs or the SVG strokes and there is nothing
/// rectangular to blur.
///
/// It APPENDS, so `box-shadow` and `filter: drop-shadow()` on the same element
/// both draw, which is what a browser does with them too.
///
/// One residual, stated because it is invisible: a child painted OUTSIDE the
/// box (an unclipped overflow) is part of the browser's silhouette and not of
/// this one.
fn apply_drop_shadow(
    style: &mut gpui::StyleRefinement,
    extras: &vn_css::Extras,
    inherited_color: Option<gpui::Hsla>,
    at: impl Fn() -> String,
) {
    let Some(shadow) = extras.drop_shadow else { return };
    let opaque = style
        .background
        .as_ref()
        .and_then(|fill| fill.color())
        .and_then(|bg| bg.as_solid())
        .filter(|c| c.a >= 0.99);
    if opaque.is_none() {
        advise_drop_shadow_shape(at);
        return;
    }
    let color = shadow
        .color
        .or_else(|| style.text.color.or(inherited_color))
        .unwrap_or_else(gpui::black);
    style.box_shadow.get_or_insert_with(Vec::new).push(gpui::BoxShadow {
        color,
        offset: gpui::point(gpui::px(shadow.dx), gpui::px(shadow.dy)),
        blur_radius: gpui::px(shadow.blur),
        spread_radius: gpui::px(0.0),
        inset: false,
    });
}

/// CSS `mask` as the gradient-border idiom: the background paints only in the
/// padding and border ring (`gpui::Style::vn_mask_ring`, vn-css's
/// `Extras::mask_ring`). The two cases the ring does not cover say so:
/// - a `url()` background, which `crate::bgimage` paints from a child the
///   ring does not reach;
/// - children, which a browser masks with the element and gpui does not.
fn apply_mask_ring(
    style: &mut gpui::StyleRefinement,
    extras: &vn_css::Extras,
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

/// Does this element paint anything of its own inside it: an element child,
/// or text? Vue's empty anchors and empty text nodes do not count.
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

/// The `drop-shadow()` this engine cannot draw, said once (from
/// [`apply_drop_shadow`]; an inline `<svg>` draws its own, `crate::svg`).
fn advise_drop_shadow_shape(at: impl FnOnce() -> String) {
    warn_compat_at(
        "filter-drop-shadow-shape",
        at,
        "`filter: drop-shadow()` is honoured only on a box with an OPAQUE solid background \
         (v1), and on an inline <svg>, whose shapes resvg shadows. gpui has no \
         post-processing pass over an element's own painted output, so on a box the shadow \
         is drawn as the element's OWN blurred rounded rect. That is exact for an opaque box \
         and impossible for a silhouette made of glyphs, or of a translucent fill whose alpha \
         the shadow would have to carry. On a box, `box-shadow` draws the same picture",
    );
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
    stacking: &Stacking,
    // The ORIGINATING element's display: a generated box is its child, so a
    // flex or grid container makes it an item, which `z-index` applies to.
    parent_display: Option<gpui::Display>,
) -> AnyElement {
    let mut d = div();
    *d.style() = (*pb.style).clone();
    // The gradient-border `mask`, most often written on a `::before`.
    if pb.extras.mask_ring() {
        d.style().vn_mask_ring = Some(true);
    }
    let case = pb.extras.text_transform.or(parent_case);
    let e = if pb.content.is_empty() {
        d.into_any_element()
    } else {
        d.child(transformed_text(&pb.content, case)).into_any_element()
    };
    let effect = effect_context(&pb.extras, &pb.style);
    let layer = layer_of(&pb.extras, pb.style.position, effect, parent_display);
    let entered = stacking.enter(layer, next_stack_ordinal());
    if entered.negative {
        note_negative_z();
    }
    stack(entered.own, &pb.extras, e)
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
pub(crate) fn transformed_text(text: &str, case: Option<vn_css::TextTransform>) -> SharedString {
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
pub(crate) fn processed_text(raw: &str, mode: vn_css::WhiteSpaceMode) -> std::borrow::Cow<'_, str> {
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

pub(crate) fn trim_edges<'a>(
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
pub(crate) fn trim_own_edges<'a>(
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
    // Document order, negative `z-index` included: a negative-z box is a
    // deferred draw that sorts before its context's in-flow content
    // (`Stacking`), so this list, which is also taffy's, never has to move.
    node.children.iter().copied().filter(|&c| generates_a_box(t, c)).collect()
}

/// `display: inline-*` — the ONE shape where mapping an inline-level box onto
/// its block-level equivalent is visible, said once.
///
/// The mapper used to say it for every `inline-flex` in the sheet, which on a
/// Tailwind page is every Button and every Badge. Almost none of them diverge,
/// and three separate rules say why:
///
/// * **CSS blockifies most of them itself** (CSS Display 3 §2.7). A flex item,
///   a grid item and an out-of-flow box all compute their `display` to its
///   block-level equivalent — so `inline-flex` on a flex child IS `flex`, in
///   Chromium as much as here. shadcn's controls live in flex rows; that is the
///   whole reason they always looked right.
/// * **[`apply_inline_row`] recovers the rest of the common case.** A block
///   container whose laid-out children are all inline content becomes a
///   wrapping, baseline-aligned flex row, and a flex item is sized to its
///   content — which is shrink-to-fit, which is the property being missed.
/// * **an explicit width answers it too.** "Fills the available width rather
///   than shrinking to fit" cannot bite a box that was given a width.
///
/// What is left, and what this advises about: an in-flow inline-level box in a
/// block container that did NOT become a row (it has a block-level sibling), with
/// no width of its own. That box takes a line to itself and stretches across it.
fn advise_inline_level(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    extras: &vn_css::Extras,
    style: &gpui::StyleRefinement,
) {
    if extras.inline_level != Some(true) {
        return;
    }
    // Out of flow: blockified by CSS, so there is nothing to report.
    if style.position == Some(gpui::Position::Absolute) {
        return;
    }
    // A definite width takes the whole complaint away.
    if !matches!(style.size.width, None | Some(gpui::Length::Auto)) {
        return;
    }
    let Some(parent) = t.node(node.parent) else { return };
    let parent_display = parent.computed.as_ref().and_then(|cs| cs.base.display);
    // A flex or grid ITEM is blockified by CSS as well.
    if matches!(parent_display, Some(gpui::Display::Flex) | Some(gpui::Display::Grid)) {
        return;
    }
    // …and a block container full of inline content is about to become a row,
    // which sizes this box to its content exactly as a line box would.
    if inline_row_applies(t, parent, parent_display) {
        return;
    }
    warn_compat_at(
        "display-inline-level-stretches",
        || node_prefix(t, id),
        "`display: inline-*` on an in-flow box whose siblings are block-level takes a LINE of          its own and fills the width (v1) — taffy models the inner display type (flex/grid/flow)          but has no line box, so there is nothing for an inline-level box to sit beside. Where          the parent is a flex or grid container, or the box is out of flow, CSS blockifies it          too and nothing diverges; where the parent's children are all inline content, the          renderer makes that parent a wrapping flex row and this does not arise either. Give the          box a width, or put it in a flex row, for the browser's result",
    );
}

/// `position: fixed` and `position: sticky` — the two out-of-flow positions
/// taffy cannot express, advised only where they actually come out different.
///
/// **`fixed`.** taffy has one out-of-flow mode and it resolves against the
/// element's PARENT; CSS's `fixed` resolves against the viewport. Those two
/// agree exactly when the parent IS the viewport — which is the case that
/// matters, because the app root is `document.body` and every overlay library
/// portals its layer there (`<Teleport to="body">`). reka does; so the meet
/// app's dialogs, its popovers and its tooltips are all laid out where a
/// browser lays them out, and were being advised about anyway. What is left is
/// a `fixed` element left inside a smaller ancestor.
///
/// **`sticky`.** It needs the scroll offset of the nearest scroll container at
/// LAYOUT time and taffy takes no such input, so it maps onto `relative`. But
/// a sticky box only ever MOVES relative to its flow position while something
/// above it scrolls: with no scrolling ancestor, `relative` is what a browser
/// paints too. The app root does not scroll (the window is the viewport), so
/// this fires only under a real `overflow: auto` / `scroll` container.
///
/// Both are painted in a deferred layer either way, so they still stack above
/// their siblings; only the box's ORIGIN is in question.
fn advise_out_of_flow(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    extras: &vn_css::Extras,
    win: WinKey,
) {
    // **A-M03** (docs/MULTIWINDOW.md §10), said at the point the design named:
    // where a fixed box is about to be deferred and its nearest painted root is
    // not `t.root`. The behaviour is the BROWSER's — a `position: fixed`
    // element in a second window anchors to that window's viewport, because
    // that is the viewport it is in — but it is still a real change for the
    // same element the moment it is popped out of the document, so it is worth
    // saying once.
    if extras.fixed == Some(true) && !windows::is_primary(win) {
        warn_compat(
            "position-fixed-in-popout",
            "A-M03: `position: fixed` inside a pop-out resolves against THAT window's              viewport, not the main window's — the element anchors to the edges of the              window that paints it. That is what a second browser window does too, and it              is still a behaviour change for an element that was pinned to the document's              viewport a moment ago. See docs/MULTIWINDOW.md §10",
        );
    }
    if extras.fixed == Some(true) && settled(id, !fixed_lands_on_the_viewport(t, node, win)) {
        warn_compat_at(
            "position-fixed-not-portalled",
            || node_prefix(t, id),
            "`position: fixed` is laid out against a box that is NOT the viewport (v1)              — gpui/taffy have no viewport-relative position, so the element resolves              against its PARENT, and this element's parent is a different size from the              window. It is still painted in a deferred layer, so it stacks above its              siblings; portal it to the app root (Vue's `<Teleport to=\"body\">`, which              is what every overlay library already does) and the two agree exactly",
        );
    }
    if extras.sticky == Some(true) && scrolling_ancestor(t, node) {
        warn_compat_at(
            "position-sticky-in-scroller",
            || node_prefix(t, id),
            "`position: sticky` inside a scroll container is laid out as `relative` (v1)              — it needs that container's scroll offset at LAYOUT time and taffy takes no              such input, so the box does not stay put as the content moves under it. A              header that must stay put belongs OUTSIDE the scroll container, as a flex              sibling of it. (Outside one this says nothing: with nothing scrolling above              it, `sticky` and `relative` paint the same box in a browser too.)",
        );
    }
}

/// True once `holds` has been true for this node on [`SETTLED_FRAMES`]
/// consecutive builds; `false` resets the count.
///
/// The layout of the first frames of an app's life is not its layout. A page
/// that says `min-h-screen` is exactly `100vh` tall, and `vh` is flattened at
/// sheet-compile time — so between boot and the recompile that adopts the real
/// client size, a full-bleed wrapper is the size of the WINDOW while the app
/// root is the size of the window minus the chrome. For those two frames the
/// wrapper is not the viewport, and a `position: fixed` child of it really is
/// laid out against the wrong box.
///
/// An advisory fires once for the life of the process and can never be
/// retracted, so it must not be spent on that. Three builds is long past any
/// basis recompile and instant for anything that is really wrong.
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

/// `(the node whose box IS this window's viewport, the key
/// [`painted_bounds_in`] addresses that window by)`.
///
/// They differ for the primary window and only there: its painted root is the
/// sentinel `0` ("the tree root"), while the node whose rectangle is the
/// viewport is `t.root` itself. A pop-out's painted root is both.
fn window_viewport(t: &Tree, win: WinKey) -> (u32, u32) {
    match windows::root_of(win) {
        0 => (t.root, 0),
        popped => (popped, popped),
    }
}

/// Is the box a `position: fixed` element is actually laid out against the
/// viewport?
///
/// taffy resolves an out-of-flow box against its PARENT; CSS resolves `fixed`
/// against the viewport. So the question is not "was it portalled" but "is the
/// parent's box the window's box" — and for the app root it is by definition
/// (the root IS the viewport, `root_bounds`), which is what makes every
/// `<Teleport to="body">` overlay agree exactly.
///
/// For anything else the rectangles are compared, because a full-bleed wrapper
/// (`min-h-screen`, a `fixed inset-0` decoration layer inside it) is laid out
/// against a box that happens to BE the viewport, and advising there would be
/// advising about a difference of zero. Padding and a border on that wrapper
/// are counted, since they inset the containing block without changing the
/// element's own rectangle.
///
/// Rectangles come from the previous frame (`painted_bounds` — a build cannot
/// see this frame's layout), so on the very first frame nothing is known and
/// nothing is said: an advisory fires once for the life of the process, and
/// one it can never retract must not be based on a rectangle that does not
/// exist yet.
fn fixed_lands_on_the_viewport(t: &Tree, node: &crate::tree::Node, win: WinKey) -> bool {
    // **The viewport is the PAINTING window's, not the primary's** (A-M03).
    // A pop-out's painted root is its viewport exactly as `t.root` is the
    // document's, so a fixed box that is a child of it lands on the viewport
    // and nothing is owed — and one inside a smaller wrapper in that window is
    // owed the advisory, measured against the right rectangle. Asking
    // `painted_bounds` (which searches every window, primary first) would have
    // compared a pop-out's box against the MAIN window's root: two coordinate
    // spaces, and an advisory fired at every `fixed` element in every pop-out.
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
    // The containing block is the parent's PADDING box, so a padded or bordered
    // wrapper of exactly viewport size still insets it.
    t.node(node.parent)
        .and_then(|p| p.computed.as_ref())
        .is_none_or(|cs| no_inner_inset(&cs.base))
}

/// Are this style's padding and border widths all provably zero? A percentage
/// padding is not provable without a basis, so it counts as an inset.
fn no_inner_inset(style: &gpui::StyleRefinement) -> bool {
    let zero_definite = |v: &Option<gpui::DefiniteLength>| match v {
        None => true,
        Some(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(p))) => f32::from(*p) == 0.0,
        Some(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Rems(r))) => r.0 == 0.0,
        Some(gpui::DefiniteLength::Fraction(f)) => *f == 0.0,
        Some(gpui::DefiniteLength::Calc { .. }) => false,
    };
    let zero_absolute = |v: &Option<gpui::AbsoluteLength>| match v {
        None => true,
        Some(gpui::AbsoluteLength::Pixels(p)) => f32::from(*p) == 0.0,
        Some(gpui::AbsoluteLength::Rems(r)) => r.0 == 0.0,
    };
    [&style.padding.top, &style.padding.right, &style.padding.bottom, &style.padding.left]
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

/// Does anything between `node` and the root actually scroll? `overflow:
/// scroll`/`auto` is what gives a node a `ScrollHandle`, and the cascade has
/// already folded it into the ancestors' computed styles by the time a child is
/// built.
fn scrolling_ancestor(t: &Tree, node: &crate::tree::Node) -> bool {
    let mut id = node.parent;
    for _ in 0..1024 {
        let Some(n) = t.node(id) else { return false };
        if let Some(cs) = n.computed.as_ref() {
            let o = &cs.base.overflow;
            if o.x == Some(Overflow::Scroll) || o.y == Some(Overflow::Scroll) {
                return true;
            }
        }
        if n.parent == 0 {
            return false;
        }
        id = n.parent;
    }
    false
}

/// HTML metadata content: elements that carry instructions, not a box. A
/// browser gives all of these `display: none` from its UA stylesheet.
pub(crate) fn is_metadata(tag: &str) -> bool {
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
    | (1 << tree::EV_CONTEXTMENU)
    | (1 << tree::EV_AUXCLICK)
    | (1 << tree::EV_MOUSEMOVE)
    | (1 << tree::EV_MOUSEENTER)
    | (1 << tree::EV_MOUSELEAVE)
    | (1 << tree::EV_WHEEL);

/// The listener mask of a BUILD root: the main window's root, or a pop-out's.
///
/// A root is what gpui's dispatch of every press and keystroke in its window
/// passes through last, so it listens for what anything above the event's
/// target does. In the main window that is the root's own listeners, which
/// `document`'s and `window`'s ride. A pop-out's root is a node deep in the
/// same tree (docs/MULTIWINDOW.md §1): its ancestors are in no element gpui
/// lays out for the pop-out, yet in the DOM their listeners — `document`'s
/// among them — hear its events bubble. So it takes their bits as well. Without
/// them a press on an element with no `mousedown` listener of its own never
/// reached a `document` `pointerdown` listener in a pop-out, which is what
/// dismisses a reka-ui menu on a click outside it.
///
/// And `pointer-events: none` never takes a root's away, as
/// [`pointer_mask`] says for the main one: a pop-out's root is its window's
/// `body` (runtime.ts, `windowDocument`), which a modal layer sets to `none`.
fn root_mask(t: &Tree, node: &crate::tree::Node) -> u64 {
    let mut mask = node.listeners;
    let mut cur = node.parent;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        mask |= n.listeners;
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    mask
}

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

/// The style half every element shares: the folded base refinement assigned
/// wholesale, the running transition overlay refined over it, the `:hover` and
/// `group-hover:` rungs when the hover path holds the element or its anchor,
/// then gpui's own active rungs (the FnOnce builders run eagerly — only the
/// resulting refinement is stored).
fn apply_computed_style<E>(
    mut e: E,
    cs: &vn_css::ComputedStates,
    id: u32,
    now: std::time::Instant,
) -> E
where
    E: gpui::Styled + gpui::StatefulInteractiveElement + 'static,
{
    use gpui::Refineable as _;
    *e.style() = (*cs.base).clone();
    if let Some(url) = &cs.extras.filter_url {
        e.style().vn_color_matrix = tree::read_tree(|tree| crate::filter::resolve(tree, url, id));
    }
    if let Some(overlay) = tree::animation_overlay(id, &cs.base, now) {
        e.style().refine(&overlay);
    }
    // The properties the author named in `transition` come OUT of the hover
    // rungs: a rung has no clock and would refine them in whole the instant
    // the element is hovered, leaving the animator nothing to interpolate.
    // `tree::note_hover` feeds them to the animator instead, and its overlay
    // (refined in above) is what paints them. Everything else stays in the
    // rung and still snaps — which is exactly what CSS does with an
    // untransitioned property.
    let split = |rung: &std::rc::Rc<gpui::StyleRefinement>| {
        vn_css::strip_transitioned(rung, &cs.transitions)
            .map(std::rc::Rc::new)
            .unwrap_or_else(|| rung.clone())
    };
    // `:hover` and `.group:hover .this` match the HOVER PATH, the topmost
    // element under the pointer and its ancestors, resolved once a frame
    // (`crate::pointer::resolve_hover`), in gpui's order: the group's rung,
    // then the element's own. Not gpui's `hover` rungs, which follow every
    // hitbox under the pointer: an element under a cover stayed hovered, and
    // an ancestor whose box the pointer left lost its hover while its child
    // kept it. `pointer-events: none` does not keep them off: an element that
    // cannot be hit is still hovered while a descendant that can be is.
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
    // `pointer-events: none` means the node is not interactive at all — so it
    // must not carry gpui's active rungs either, which are exactly what would
    // make gpui insert a hitbox for it. `:active` is gpui's ladder on the
    // press's refined set (`crate::pointer::refine`): the press target's path.
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
/// The focus itself waits until gpui's dispatch is over ([`apply_press_focus`]).
/// A browser focuses as the mousedown's DEFAULT action, after its listeners and
/// their microtask checkpoint. So a `preventDefault()` in any of them cancels it
/// ([`cancel_press_focus`]); Vue's `@mousedown.prevent` on a mention list or a
/// toolbar button is how a composer keeps its caret. The `mousedown` is
/// dispatched by the innermost box on the target's path that LISTENS, which may
/// be an ancestor whose listener runs after this one, so the claim waits for
/// it. Only boxes on that path run at all (`crate::pointer`), so the focus goes
/// to the target's nearest focusable ancestor, as in a browser — never to a
/// field the target is painted over. Only a node HTML lets take focus claims
/// (FO-3, docs/RUNTIME.md §5.2): with none on the path, the press blurs to
/// `body`.
///
/// One log line per real focus move, at `dev` kind — this is the exact failure
/// the meet app's "typing does nothing" turned out to be.
fn claim_press_focus(id: u32, handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    begin_press_focus(window, cx);
    // gpui's own focus-on-press (`track_focus`, the root's included) backs off:
    // the press's focus is decided by the claim. An inner box tracked only for
    // a `:focus-within` rule may already have taken it that way; the step
    // below puts it where the claim says.
    window.prevent_default();
    // Mouse listeners run TOPMOST-FIRST, so the first claim is the nearest
    // focusable ancestor of the target, and the rest are its ancestors.
    windows::here(window, |w| {
        if !w.press_focus.claims.iter().any(|(c, _)| *c == id) {
            w.press_focus.claims.push((id, handle.clone()));
        }
    });
}

/// Starts a mouse-down's focus step, once per press: the build root's capture
/// listener ([`bind_root_mouse`]) calls this first, and a claim does if that
/// one did not run.
fn begin_press_focus(window: &mut Window, cx: &mut App) {
    let schedule = windows::here(window, |w| {
        if w.press_focus.pending {
            return false;
        }
        w.press_focus = windows::PressFocus { pending: true, ..Default::default() };
        true
    });
    if schedule {
        // Deferred: every listener of this mouse-down has claimed, or
        // cancelled, by the time it runs, and no gpui listener is under it.
        window.defer(cx, |window, cx| crate::off_input(|| apply_press_focus(window, cx)));
    }
}

/// A cancelled `mousedown`: the press focuses nothing, neither the box that
/// claimed it nor a focusable ancestor nor gpui's own focus-on-press.
fn cancel_press_focus(window: &mut Window) {
    windows::here(window, |w| w.press_focus.cancelled = true);
    window.prevent_default();
}

/// A mouse-down's default action, when its dispatch is over: the `mousedown`'s
/// microtask checkpoint, then the focus move, then that move's `blur` /
/// `focusout` / `focus` / `focusin`, all before the next platform event, so
/// before `mouseup`, `click` and `contextmenu` (docs/RUNTIME.md §5.22). They
/// used to come from gpui's focus observer on the next draw: after the
/// `contextmenu` of a quick right click, so a menu that mounted from it
/// remembered `body` as the focused element.
///
/// - Claimed: the nearest claiming node still in the document takes the focus.
///   Chromium moves a press's node to its parent when it is removed, so a
///   button the checkpoint removed gives the focus to its focusable ancestor.
/// - Nothing on the path can take focus: the root takes gpui's focus, which is
///   `body` having it, and the old node blurs.
/// - Cancelled, or taken by a native widget before the root heard it (a
///   scrollbar): the focus stays. A text field's editor focuses itself on its
///   own press before any listener runs, and keeps it (L-2, §5.8).
fn apply_press_focus(window: &mut Window, cx: &mut App) {
    let press = windows::here(window, |w| std::mem::take(&mut w.press_focus));
    // The mousedown's checkpoint first, as in a browser (the one
    // `settle_owed_later` queued finds nothing owed after this).
    if vn_runtime::bridge::drain_owed() {
        js_wave_app(window, cx, || true);
    }
    let k = windows::key(window);
    let root = window_root(k);
    let root_handle = || FOCUS_HANDLES.with(|m| m.borrow().get(&root).cloned());
    if !press.cancelled && (!press.claims.is_empty() || press.reached) {
        let in_document = |id: u32| tree::read_tree(|t| t.node(id).is_some() && reaches_painted_root(t, id));
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
        // gpui's own focus-on-press may have moved the focus to an inner box
        // tracked only for a `:focus-within` rule, which HTML never focuses:
        // put it back.
        if now != root && !windows::of(k, |w| w.focus_subs.contains_key(&now)) {
            let slot = windows::focused_node(k);
            let back = if slot != 0 { focus_target_of(slot, cx) } else { root_handle() };
            if let Some(h) = back {
                window.focus(&h, cx);
            }
        }
    }
    sync_focus_events(window, cx);
    // …and a press on a `<select>` opens it, now that it has the focus.
    crate::select::press_settled(press.cancelled, window, cx);
    crate::range::press_settled(press.cancelled, window, cx);
}

/// Reports gpui's focus to JS now, if it moved past what JS was last told:
/// the `blur` / `focus` events the observer would send on the next draw.
fn sync_focus_events(window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let now = match focused_node(window, cx) {
        None => 0,
        Some(id) if id == window_root(k) => 0,
        Some(id) if windows::of(k, |w| w.focus_subs.contains_key(&id)) => id,
        // A box that reports no focus (see above): nothing to say.
        Some(_) => return,
    };
    if now == windows::focused_node(k) {
        return;
    }
    focus_transition_app(now, window, cx);
    // `:focus` / `:focus-visible` paint from the new state.
    if !repaint_other_windows(cx) {
        window.refresh();
    }
}

/// The gpui group name for an element: its node id, which is unique in the
/// document and stable for as long as the node is. Two different `.group`
/// ancestors can therefore never collide, and a nested one shadows its outer
/// namesake for free — `GroupHitboxes` is a stack and `get` returns its top.
fn group_name(id: u32) -> gpui::SharedString {
    format!("vn-g{id}").into()
}

/// Where a box paints: the STACKING CONTEXT it is in, as the key gpui sorts
/// deferred draws by.
///
/// CSS paints a stacking context ATOMICALLY (CSS 2.1 Appendix E). Inside one,
/// in this order: the context's own background and border; its negative
/// `z-index` child contexts, lowest first; its in-flow content; its positioned
/// `z-index: auto` boxes and its `z-index: 0` contexts, in tree order; its
/// positive `z-index` child contexts, lowest first, tree order breaking ties.
/// Each child context is painted the same way, whole, in its one slot, so
/// NOTHING inside an earlier context, its own `z-50` descendants included,
/// paints over a later sibling context of the same z, and nothing inside a
/// `z-10` context paints over a `z-20` sibling whatever its own z is.
///
/// So the key is a PATH: one slot per enclosing context, outermost first, and
/// each slot is `(z, tree-order ordinal)` of that context within its parent.
/// gpui compares keys lexicographically (`gpui::Deferred::with_stack_key`),
/// a key sorts after every key it is a prefix of, and that comparison is the
/// order above, exactly. A z-auto `position: absolute` box opens no context:
/// it is one more slot at z 0 in the context it is in, and its positioned
/// descendants keep sorting in that same context, after it in tree order, as
/// CSS says. The hit test walks the same order (the fork's
/// `Frame::vn_hit_order`), so a click lands on what is painted on top.
///
/// The packed `usize` this replaced kept a context's level in its digits but
/// left equal digits to gpui's prepaint ROUNDS, and a draw nested one round
/// deeper was painted after every draw of the round before. The Argon profile
/// popover is that shape: two portalled `fixed z-50` overlays, the popover
/// first, with `z-index: 0 / 1 / 2` layers inside it. The layers sorted after
/// the menu the popover opens, and painted, and took clicks, over its items.
///
/// **What is a context and what is deferred** is [`Layer`], from [`layer_of`]:
///
/// - **A `z-index` counts only where it applies:** on a positioned box, or on
///   a flex or grid item even when static. `z-10` on a static block in a
///   block parent is inert in a browser and inert here. The Argon client's
///   sign-in `<Input class="z-10">` used to lift over the error badge beside
///   it.
/// - **An integer `z-index` that applies makes a context; `auto` does not.**
///   So do `position: fixed` and `sticky`, and, at z auto, `opacity < 1`, a
///   `transform`, a `filter` (`blur()`, `drop-shadow()`), `backdrop-filter`
///   and `isolation: isolate`. A non-zero z, `fixed`, and a context on an
///   `absolute` box are deferred into their slot.
/// - **A NEGATIVE `z-index` paints between its context's background and its
///   context's in-flow content.** The background and the content are one
///   gpui element, so when a context has a negative-z box in it (seen while
///   its children are built, [`OPEN_CONTEXTS`]), its children are each
///   deferred into the in-flow slot of its key, after the negative ones;
///   layout does not move, because a deferred element keeps its place in
///   taffy. The background stays in the context's own paint. An in-flow box
///   is as good as an out-of-flow one here.
/// - **A context that is not deferred paints in tree order** — `relative` /
///   `sticky` at z 0 or auto, and the effect contexts on boxes that are not
///   `absolute` — instead of after its context's in-flow content (step 8).
///   That is a stated limit, with an advisory for the positioned ones
///   (`position-relative-tree-order`): such boxes wrap whole subtrees (an app
///   shell's root, a card, every `opacity-50` button), and lifting them would
///   move most of a page into the deferred layer for the rare case where a
///   later in-flow sibling overlaps one. It is still a context, and its own
///   z-indexed descendants sort inside it, at its place in step 8. One that
///   has to split for a negative z is deferred to that place whole.
/// - **The effect contexts are read off the declared style**, not the
///   animated one: a transition through `opacity: 0.5` does not open a
///   context mid-flight, and a `transform` that moves nothing (the `none` of
///   gpui's `TransformSpec`) is not one. `mix-blend-mode`, `mask`,
///   `will-change` and `contain` make no context here; none is modelled.
///
/// Deferral nests: every deferred box inside another is one more prepaint
/// round. Past [`MAX_DEFERRED_NESTING`] a box paints in flow, with an
/// advisory; a context stays a context.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stacking {
    /// The context's path, one slot per enclosing context ([`slot`]),
    /// outermost first. Empty = the root context.
    key: std::rc::Rc<[u128]>,
    /// How many deferred boxes enclose this one. Bounds gpui's deferred rounds.
    deferred: u32,
}

/// How one box takes part in painting, by CSS 2.1 Appendix E. See [`Stacking`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    /// Painted where it is, in tree order, in the context it is in.
    InFlow,
    /// Painted where it is, in tree order, but a stacking context: its
    /// descendants' z-indices sort inside it. `relative` / `sticky` at z 0,
    /// `sticky` at z auto, or an effect context (`opacity < 1`, …) on a box
    /// that is not `absolute`.
    InFlowContext,
    /// `position: absolute` with `z-index: auto`: deferred after the in-flow
    /// content of the context it is in (step 8), opening none.
    Positioned,
    /// Deferred, and a context at this z: an integer `z-index` that applies
    /// (`0` only on an `absolute` box), `position: fixed`, or an effect
    /// context on an `absolute` box.
    Context(i32),
}

/// Nested deferred boxes allowed before one paints in flow. gpui runs one
/// prepaint round per nesting level and asserts at 64 (fork item 37); the
/// margin is for the in-flow splits, which [`Stacking::enter`] cannot count
/// because they are decided once the children are built.
const MAX_DEFERRED_NESTING: u32 = 32;

/// Does `z-index` apply to a box? Only to a positioned one (a declared
/// `position` other than `static`, CSS 2.1 §9.9.1) or to a flex or grid item,
/// static or not (css-flexbox §4.3, css-grid §9). `parent_display` is the
/// DECLARED display of the box's parent; a block that `apply_inline_row`
/// lays out as a row is still a block container to CSS.
pub(crate) fn z_applies(extras: &vn_css::Extras, parent_display: Option<gpui::Display>) -> bool {
    extras.positioned == Some(true)
        || matches!(parent_display, Some(gpui::Display::Flex) | Some(gpui::Display::Grid))
}

/// Does a box's declared style make it a stacking context at `z-index:
/// auto`? `opacity < 1`, a `transform` that moves something, a `filter` we
/// model, `backdrop-filter`, `isolation: isolate`, a `clip-path` (CSS
/// Masking 1 §5). (`fixed` and `sticky` are [`layer_of`]'s.)
pub(crate) fn effect_context(extras: &vn_css::Extras, style: &gpui::StyleRefinement) -> bool {
    style.opacity.is_some_and(|o| o < 1.0)
        || !matches!(style.vn_clip_path, None | Some(gpui::VnClipPath::None))
        || style.vn_transform.as_ref().is_some_and(|t| !t.is_identity())
        || style.backdrop_blur.is_some_and(|r| r > gpui::px(0.0))
        || extras.blur.is_some_and(|b| b > 0.0)
        || extras.filter_url.is_some()
        || extras.drop_shadow.is_some()
        || extras.isolate == Some(true)
}

/// The [`Layer`] of a box: its extras, its own `position` as gpui has it
/// (`Absolute` for both `absolute` and `fixed`), whether its style makes it
/// an [`effect_context`], and its parent's display.
pub(crate) fn layer_of(
    extras: &vn_css::Extras,
    position: Option<gpui::Position>,
    effect: bool,
    parent_display: Option<gpui::Display>,
) -> Layer {
    let z = if z_applies(extras, parent_display) { extras.z() } else { None };
    let out_of_flow = position == Some(gpui::Position::Absolute);
    match z {
        Some(z) if z != 0 => Layer::Context(z),
        Some(_) if out_of_flow => Layer::Context(0),
        Some(_) => Layer::InFlowContext,
        None if extras.fixed == Some(true) => Layer::Context(0),
        None if out_of_flow && effect => Layer::Context(0),
        None if out_of_flow => Layer::Positioned,
        None if effect || extras.sticky == Some(true) => Layer::InFlowContext,
        None => Layer::InFlow,
    }
}

/// The stated limit of [`Stacking`]: a `relative` / `sticky` box at z auto or
/// 0 keeps its tree-order paint. Said once.
fn advise_relative_tree_order(
    extras: &vn_css::Extras,
    position: Option<gpui::Position>,
    layer: Layer,
) {
    if !matches!(layer, Layer::InFlow | Layer::InFlowContext)
        || extras.positioned != Some(true)
        || position == Some(gpui::Position::Absolute)
        || extras.z().is_some_and(|z| z != 0)
    {
        return;
    }
    warn_compat(
        "position-relative-tree-order",
        "`position: relative` / `sticky` with `z-index: auto` or `0` paints in TREE ORDER (v1). CSS \
         paints a positioned box after the in-flow content of its stacking context (CSS 2.1 \
         Appendix E, step 8), so a LATER in-flow sibling that overlaps it paints under it in a \
         browser and over it here. `position: absolute` boxes are painted in that layer; \
         `relative` ones are not, because relative wrappers hold whole subtrees and lifting them \
         would move most of a page into the deferred layer. A positive `z-index` lifts the box",
    );
}

/// One slot of a [`Stacking`] key: a box's z, then its tree-order ordinal.
/// The in-flow content of a context ([`IN_FLOW`]) ranks between its negative
/// z and its z 0, which is where Appendix E paints it.
fn slot(rank: u64, ordinal: u64) -> u128 {
    ((rank as u128) << 64) | ordinal as u128
}

/// The rank of a context at `z`: odd, so that [`IN_FLOW`] fits between -1
/// and 0.
fn z_rank(z: i32) -> u64 {
    ((i64::from(z) + (1 << 31)) as u64) * 2 + 1
}

/// The rank of a context's in-flow content, when it has to be painted apart
/// from the context's background (a negative `z-index` in between).
const IN_FLOW: u64 = 1 << 32;

thread_local! {
    /// The tree-order ordinal of the next box to enter a [`Stacking`]. Reset
    /// per build: a key only has to order the draws of the frame it is in.
    static STACK_ORDINAL: Cell<u64> = const { Cell::new(0) };
    /// One entry per stacking context whose children are being built,
    /// innermost last: has a negative-z box landed in it? See [`Stacking`].
    static OPEN_CONTEXTS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// The next tree-order ordinal.
fn next_stack_ordinal() -> u64 {
    STACK_ORDINAL.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    })
}

/// A new build: ordinals start over, and no context is open.
fn reset_stacking() {
    STACK_ORDINAL.with(|c| c.set(0));
    OPEN_CONTEXTS.with(|c| c.borrow_mut().clear());
}

/// What [`Stacking::enter`] decides for one box.
pub(crate) struct Entered {
    /// The box's own deferred-draw key; `None` = it paints where it is.
    pub(crate) own: Option<std::rc::Rc<[u128]>>,
    /// The context the box's children are in.
    pub(crate) inside: Stacking,
    /// The box is a stacking context (`inside` is its own), deferred or not.
    pub(crate) opens: bool,
    /// The box is a deferred NEGATIVE-z context: its context has to paint its
    /// in-flow content apart from its background.
    pub(crate) negative: bool,
}

impl Stacking {
    /// `self`'s key with one more slot.
    fn with_slot(&self, slot: u128) -> std::rc::Rc<[u128]> {
        self.key.iter().copied().chain(std::iter::once(slot)).collect()
    }

    /// The key of this context's in-flow content, painted apart: one wrapper
    /// per child, in tree order.
    fn in_flow_key(&self) -> std::rc::Rc<[u128]> {
        self.with_slot(slot(IN_FLOW, next_stack_ordinal()))
    }

    /// Where a box of `layer`, `ordinal`-th in tree order, paints, and the
    /// context its children see.
    fn enter(&self, layer: Layer, ordinal: u64) -> Entered {
        let (z, defer, opens) = match layer {
            Layer::InFlow => {
                return Entered { own: None, inside: self.clone(), opens: false, negative: false }
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
                    "more than 32 nested out-of-flow layers (`position: absolute` / `fixed` / a \
                     `z-index`, each inside the last) — the deeper ones paint in tree order \
                     instead of above their in-flow siblings (each is one more of gpui's deferred \
                     prepaint rounds). A stacking context stays one",
                );
            }
            room
        };
        let deferred = self.deferred + u32::from(defer);
        let inside = Stacking { key: if opens { key.clone() } else { self.key.clone() }, deferred };
        Entered { own: defer.then_some(key), inside, opens, negative: defer && z < 0 }
    }
}

/// The open context gets a negative-z box: it will paint its in-flow content
/// apart (see [`Stacking`]).
fn note_negative_z() {
    OPEN_CONTEXTS.with(|c| {
        if let Some(top) = c.borrow_mut().last_mut() {
            *top = true;
        }
    });
}

/// A context's children are about to be built.
fn open_context() {
    OPEN_CONTEXTS.with(|c| c.borrow_mut().push(false));
}

/// A context's children are built: did a negative-z box land in it?
fn close_context() -> bool {
    OPEN_CONTEXTS.with(|c| c.borrow_mut().pop()).unwrap_or(false)
}

/// Paints a context's in-flow children APART from its background, after its
/// negative-z boxes: each child deferred into the context's in-flow slot, in
/// tree order. `keep` leading children stay in the context's own paint — the
/// background layer, which is the background.
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

/// Wraps a node that paints out of tree order — see [`Layer`] — in gpui's
/// deferred layer, at its [`Stacking`] key.
///
/// `Deferred` keeps the child's LAYOUT where it is (it returns the child's own
/// `LayoutId`) and only moves the PAINT, and the hit test with it.
fn stack(key: Option<std::rc::Rc<[u128]>>, extras: &vn_css::Extras, el: AnyElement) -> AnyElement {
    match key {
        Some(key) => gpui::deferred(el)
            .with_stack_key(key)
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

/// [`stack`], for an element whose children are built. A context whose
/// children had to `split` (a negative z in it, [`split_in_flow`]) and that is
/// not deferred already is deferred whole, to its own slot (`inside.key`): its
/// background has to go under the negative-z boxes that paint over it. The
/// root of a build never is; it paints first.
fn stack_box(
    own: Option<std::rc::Rc<[u128]>>,
    split: &Cell<bool>,
    is_root: bool,
    inside: &Stacking,
    extras: &vn_css::Extras,
    el: AnyElement,
) -> AnyElement {
    let own = own.or_else(|| (split.get() && !is_root).then(|| inside.key.clone()));
    stack(own, extras, el)
}

/// Claims the key event gpui is dispatching in this window for ONE DOM event:
/// `true` for the first caller, `false` for every later one.
///
/// A browser fires one `keydown` per key, at the focused element, and lets it
/// bubble to `document`. gpui instead hands the key event to EVERY key listener
/// on the focus path, and a text field's editor sees it before all of them
/// (its keystroke interceptor). Each of those used to dispatch a DOM event of
/// its own: a focused box's listener, then a listening ancestor's, then the
/// root's (which stands in for `document`) — so a `document` listener heard
/// one Escape twice or three times, and a field whose Escape handler removed
/// it still had the root deliver the key again, because gpui walks the path it
/// computed before anyone ran. A dialog's field closed two layers on one key.
///
/// The claim is keyed by gpui's per-event number (`Window::vn_key_event_seq`,
/// fork), not by a flag reset in some phase: interceptors run before the
/// capture phase, a cancelled key stops gpui's walk early, and devtools sends
/// several keys inside one update — a number that changes with each event is
/// right in all three.
fn claim_key_event(window: &Window) -> bool {
    let seq = window.vn_key_event_seq();
    windows::here(window, |w| std::mem::replace(&mut w.key_event_claimed, seq) != seq)
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
    cx: &mut Context<V>,
) -> E
where
    E: gpui::StatefulInteractiveElement + 'static,
    V: 'static,
{
    let bit = move |ev: u32| mask & (1u64 << ev) != 0;
    // ONE event per press, dispatched AT the browser's target: the topmost
    // element under the pointer, which `crate::pointer` resolved before any
    // listener ran — and which need not listen at all. The same resolution
    // narrowed gpui's hovered set to that target's path, so the listeners
    // below run only on the target and its ancestors, never on a box the
    // target is painted over. gpui runs them innermost first, in the bubble
    // phase, and the JS side walks the ancestors itself, so the first to run
    // claims the event and the rest stay quiet: without the claim an outer
    // `@click` ran twice for one press, once bubbled from the target and once
    // as a target of its own. (An `<a href>` listens for its own activation,
    // which is how this surfaced for `click`: desktop-a-impl.md. `mousedown`
    // and `mouseup` had the same double delivery until m26.)
    //
    // The three events of a mouse-up (`mouseup`, `click`, `contextmenu`) are
    // claimed into one wave and delivered together, in DOM order, when gpui's
    // dispatch is over (`WindowState::up_waves`): gpui calls a box's click
    // listener BEFORE its own mouse-up listener, and an inner box's listeners
    // before an outer box's, so dispatching in place put `click` before
    // `mouseup` and could put `contextmenu` first of all. The build root opens
    // every wave ([`bind_root_mouse`]).
    if bit(tree::EV_CLICK) {
        e = e.on_click(cx.listener(move |_, ev: &ClickEvent, window, cx| match ev {
            // Physical mouse clicks are paired by DOM identity below. gpui's
            // pending_mouse_down belongs to a cached view's element-state path,
            // which may change during the redraw the press itself requests.
            ClickEvent::Mouse(_) => {}
            // Space on the focused box reaches that box only, and is no part of
            // a mouse-up. gpui clicks on the key's RELEASE, which is
            // Chromium's rule for Space, but before the `keyup` handlers: the
            // click waits for them (`keyboard_click_later`). Enter clicks on
            // its press, below.
            ClickEvent::Keyboard(k) => {
                // Only a box whose Space press armed it (`keyboard_activates`).
                if k.button == gpui::KeyboardButton::Space {
                    let (at, modifiers) = (ev.position(), ev.modifiers());
                    let step = KeyClick::Click { id, at, modifiers, armed: true, key: " " };
                    owe_key_click(step, window, &mut **cx);
                }
            }
            ClickEvent::Touch(_) => {
                let payload = mouse_payload_n(ev.position(), 0.0, &ev.modifiers(), ev.click_count());
                js_dispatch(id, tree::EV_CLICK, &payload, window, cx);
            }
        }));
        e = e.on_mouse_up(MouseButton::Left, move |ev, window, cx| {
            claim_mouse_click(ev, id, window, cx, |w| &mut w.click);
        });
        // Enter clicks the focused box on its PRESS (Chromium dispatches the
        // click from the keypress), after every `keydown` handler and only if
        // none cancelled it. gpui would click on the release, after `keyup`. A
        // held key repeats it, as a repeated keypress does. A Space press
        // only ARMS the click its release makes, and a cancelled one does not.
        // Only an element with an activation behaviour for the key is clicked
        // (`keyboard_activates`).
        if let Some(focus) = click_focus.clone() {
            e = e.on_key_down(cx.listener(move |_, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                if k.modifiers.modified() || !focus.is_focused(window) {
                    return;
                }
                match k.key.as_str() {
                    "enter" if keyboard_activates(id, false) => {
                        // Where gpui's keyboard click says it is: the box's
                        // bottom left.
                        let at = windows::peek(windows::key(window), |w| w.bounds.get(&id).copied())
                            .flatten()
                            .map(|(x, y, _, h)| gpui::point(gpui::px(x), gpui::px(y + h)))
                            .unwrap_or_default();
                        let step = KeyClick::Click { id, at, modifiers: k.modifiers, armed: false, key: "Enter" };
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
        e = e.capture_any_mouse_down(|_, window, _| windows::here(window, |w| w.down_claimed = false));
    }
    if bit(tree::EV_MOUSEDOWN) || bit(tree::EV_CLICK) || bit(tree::EV_AUXCLICK) || focusable {
        let handle = click_focus;
        e = e.on_any_mouse_down(cx.listener(move |_, ev: &MouseDownEvent, window, cx| {
            if windows::here(window, |w| w.press.is_none()) {
                let target = crate::pointer::target_or(window, id);
                let clickable = !cx.has_active_drag() && crate::pointer::click_target(window, target, id).is_some();
                windows::here(window, |w| {
                    w.press = Some(Press {
                        target, button: button_code(ev.button), position: ev.position,
                        clickable, cancelled: false,
                    });
                });
                watch_activation(window, cx);
            }
            if let Some(h) = &handle {
                // Focus follows the press, as in a browser, once the mousedown
                // is over; the focus/blur events come out of the subscription.
                claim_press_focus(id, h, window, &mut **cx);
            }
            if bit(tree::EV_MOUSEDOWN) && !windows::here(window, |w| std::mem::replace(&mut w.down_claimed, true)) {
                let button = button_code(ev.button);
                let target = crate::pointer::target_or(window, id);
                let payload = mouse_payload_n(ev.position, button, &ev.modifiers, ev.click_count);
                if !js_dispatch(target, tree::EV_MOUSEDOWN, &payload, window, cx) {
                    cancel_press_focus(window);
                    // …and a cancelled press starts no text selection.
                    crate::selection::note_press_prevented();
                }
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
                    let payload = mouse_payload_n(ev.position, button_code(ev.button), &ev.modifiers, ev.click_count);
                    let target = crate::pointer::target_or(window, id);
                    claim_up(window, &mut **cx, |w| &mut w.mouseup, target, &payload);
                }),
            );
        }
    }
    if bit(tree::EV_CONTEXTMENU) {
        // Chromium on Windows opens the menu on the right button's RELEASE
        // (`ShowContextMenuOnMouseUp`), after `mouseup`. `button` is 2;
        // `buttons` is 0, because the button is already up; `detail` is 0.
        e = e.on_mouse_up(
            MouseButton::Right,
            cx.listener(move |_, ev: &MouseUpEvent, window, cx| {
                let payload = mouse_payload(ev.position, 2.0, &ev.modifiers);
                let target = crate::pointer::target_or(window, id);
                claim_up(window, &mut **cx, |w| &mut w.menu, target, &payload);
            }),
        );
    }
    // The pointer family (compat roadmap G4). gpui sources "the mouse moved
    // while over me" here, and the JS runtime derives the W3C pointer/mouse
    // set from it. `mouseenter` / `mouseleave` (and so `pointerenter` /
    // `pointerleave`) are not gpui's: they fire from the hover path, resolved
    // once a frame by `crate::pointer::resolve_hover`.
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
    if bit(tree::EV_WHEEL) {
        e = e.on_scroll(cx.listener(move |_, ev: &ScrollEvent, window, cx| {
            // One `wheel` per wheel event, at its target (`crate::pointer`).
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
                // Cancelled: a browser scrolls nothing. gpui's own listener has
                // already moved this box, if it scrolls, and any scroll
                // container inside it: put them back, and keep the outer ones
                // from moving.
                vn_elements::undo_wheel_scroll();
                cx.stop_propagation();
            }
        }));
    }
    // ONE `keydown` / `keyup` per key event, AT the focused element, bubbling
    // from there to `document` and `window` — whichever listening box on the
    // focus path gpui reaches first dispatches it ([`claim_key_event`]), and
    // every other one (an ancestor, the root standing in for `document`) lets
    // it pass, the root included when the first dispatch removed the focused
    // element.
    if bit(tree::EV_KEYDOWN) && keys_here {
        e = e.on_key_down(cx.listener(move |_, ev: &KeyDownEvent, window, cx| {
            if !claim_key_event(window) {
                return;
            }
            let target = focused_node(window, cx).unwrap_or(id);
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            if !js_dispatch(target, tree::EV_KEYDOWN, &payload, window, cx) {
                // A cancelled Ctrl+C has no `copy` event: the root's clipboard
                // listener, which this keystroke reaches last, reads this.
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
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            if !js_dispatch(target, tree::EV_KEYUP, &payload, window, cx) {
                // A cancelled Menu-key keyup has no `contextmenu`
                // (`bind_menu_keys`, the last listener it reaches).
                windows::here(window, |w| w.key_prevented = true);
            }
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

/// The devtools' focus, host-side. Only a focusable area by HTML's rules
/// ([`could_take_focus`]) that has been built has a handle to focus — a plain
/// `<div>` with no `tabindex` is refused, as a browser refuses `el.focus()` on
/// it, whatever listeners or `:focus` rules it carries.
///
/// The `focus`/`blur` events are NOT synthesized here: they come out of the
/// gpui focus observers ([`watch_focus`]), so programmatic focus reports
/// identically to a click or a `Tab` — the invariant the module header states.
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

// ---------------------------------------------------------------------------
// Programmatic focus: `el.focus()` / `el.blur()` from JS (`__vn_focus`)
// ---------------------------------------------------------------------------
//
// A browser's `el.focus()` is SYNCHRONOUS: when it returns,
// `document.activeElement === el` and `blur` / `focusout` / `focus` /
// `focusin` have already run, in that order. gpui's focus observers fire
// later, from the next draw. So for a node that can take focus NOW (it has
// been built and has a handle), the host moves gpui's focus and the window's
// `focused_node` slot together, and tells JS which node lost focus. JS then
// dispatches the four events itself. When the observer fires on the next
// draw, it finds the slot already at the new node (`focus_transition`'s
// `old == new`), so nothing is reported twice.
//
// A node that cannot take focus yet because it has not been built (an
// `<input>` mounted in the same wave, `nextTick(() => input.focus())`) makes
// `__vn_focus` lay out first, where a layout can be forced (docs/RUNTIME.md
// §5.8; `focus_wants_build`); the node then has its handle and the move is
// synchronous. Where it cannot (under a gpui listener), the node is
// queued instead. The queue holds ONE node, because the last `focus()` wins.
// It is applied from the frame tick once the node has a handle, for up to
// `PENDING_FOCUS_FRAMES` frames. Its events then come from the observers,
// exactly like a click's, so they fire a frame late: a named divergence.

thread_local! {
    /// `(node, frames left)`; `(0, 0)` = nothing queued.
    static PENDING_FOCUS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

const PENDING_FOCUS_FRAMES: u32 = 10;

/// What `el.focus()` did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FocusAnswer {
    /// Focus moved now. JS fires `blur`/`focusout` on `old` (0 = none) and
    /// then `focus`/`focusin` on the node.
    Moved { old: u32 },
    /// It already had focus: no events, as in a browser.
    Already,
    /// Queued for the frame tick; the observers report it.
    Pending,
    /// Not focusable, not in the document, or hidden by CSS: a no-op, as in
    /// a browser.
    Refused,
}

/// Is `id` a focusable area by HTML's rules? An `<input>` or `<textarea>`, a
/// `tabindex`, or an element HTML makes focusable by default, and never an
/// actually-disabled control: the renderer's own rules (see `bind_events` and
/// `build_input`). A listener counts for nothing (FO-3), and neither does a
/// `:focus` rule, which gives a node a handle for gpui's style rungs only.
fn could_take_focus(t: &Tree, id: u32) -> bool {
    let Some(n) = t.node(id) else { return false };
    let tag = t.name(n.tag);
    if tag != "input" && tag != "textarea" && actually_disabled(t, id) {
        return false;
    }
    // A `tabindex` that is not an integer is ignored, as the build ignores it
    // (`tab_index_of`).
    let tab_index = t.prop(id, "tabindex").is_some_and(|v| v.trim().parse::<isize>().is_ok());
    tag == "input" || tag == "textarea" || tab_index || default_tab_index(t, id).is_some()
}

/// Is the CSS `visibility` of `id` `visible`? The nearest DECLARED value on it
/// or an ancestor decides (`Extras::visibility_hidden`), since the property
/// inherits, and it is `visible` when nothing declared one. The screen-reader-only idioms paint
/// nothing but are `visible` here, as in a browser. `getComputedStyle` reports
/// this, and `el.focus()` asks it ([`style_refuses_focus`]).
pub(crate) fn css_visible(t: &Tree, id: u32) -> bool {
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        if let Some(hidden) = n.computed.as_ref().and_then(|cs| cs.extras.visibility_hidden) {
            return !hidden;
        }
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    true
}

/// Is `id` in a `display: none` subtree, so not rendered at all?
fn in_display_none(t: &Tree, id: u32) -> bool {
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { return false };
        if n.computed.as_ref().is_some_and(|cs| cs.base.display == Some(gpui::Display::None)) {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        cur = n.parent;
    }
    false
}

/// Does CSS keep `id` from taking focus now? A browser's `el.focus()` flushes
/// style first and is a no-op on an element that is not rendered
/// (`display: none` on it or an ancestor) or whose computed `visibility` is
/// not `visible` (Chromium's `Element::IsFocusableStyle`; HTML's "being
/// rendered"). So a popover that mounts `visibility: hidden` and focuses its
/// item from `onMounted`, before its own reactive "now visible" write is
/// patched, does NOT take the focus. Argon's emoji tone popover does exactly
/// that inside a reka popover, and taking the focus moved it out of reka's
/// dismissable layer, which closed the picker (docs/RUNTIME.md §5.20).
pub(crate) fn style_refuses_focus(id: u32) -> bool {
    tree::flush_pending_style();
    tree::read_tree(|t| t.node(id).is_some() && (in_display_none(t, id) || !css_visible(t, id)))
}

/// Does `el.focus()` on `id` need a build first? It could take focus but has
/// no handle yet, because it has not been built: mounted in this same update,
/// as reka's FocusScope does from `onMounted` when a dialog opens. `__vn_focus`
/// then forces a layout where one is legal (`crate::sync_layout`, docs/RUNTIME.md
/// §5.8), so `focus()` is synchronous, as in a browser: reka's `focusFirst`
/// checks `document.activeElement` straight after each call and moves on to
/// the next candidate when it did not change.
pub(crate) fn focus_wants_build(id: u32) -> bool {
    let built = INPUT_STATES.with(|m| m.borrow().contains_key(&id))
        || FOCUS_HANDLES.with(|m| m.borrow().contains_key(&id));
    !built && tree::read_tree(|t| t.node(id).is_some() && could_take_focus(t, id))
}

/// Queues `id` for the frame tick (the last call wins).
pub(crate) fn queue_focus(id: u32) {
    PENDING_FOCUS.with(|p| p.set((id, PENDING_FOCUS_FRAMES)));
}

/// `el.focus()`, host-side. See the section header.
pub(crate) fn js_focus(id: u32, window: &mut Window, cx: &mut App) -> FocusAnswer {
    let k = windows::key(window);
    let (in_document, could) =
        tree::read_tree(|t| (t.node(id).is_some() && reaches_painted_root(t, id), could_take_focus(t, id)));
    // Not a focusable area: a node with only a `:focus` rule has a handle, but
    // a browser refuses to focus it.
    if !in_document || !could {
        return FocusAnswer::Refused;
    }
    // Another window paints it: that window's tick applies it.
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
    windows::set_focused_node(k, id);
    crate::log::record("dev", format!("[focus] el.focus() focused node {id}"));
    FocusAnswer::Moved { old: if old == id { 0 } else { old } }
}

/// `el.blur()`, host-side: `true` when `id` had focus and lost it (JS fires
/// `blur` / `focusout`). Focus then falls back to the window's root, which
/// reports nothing, like a browser's `body`.
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
    windows::set_focused_node(k, 0);
    true
}

/// Applies a queued `el.focus()` once the node has a handle. Runs from the
/// frame ticks, before `ensure_root_focus`.
pub(crate) fn apply_pending_focus(window: &mut Window, cx: &mut App) {
    let (id, left) = PENDING_FOCUS.with(|p| p.get());
    if id == 0 || windows::window_of_node(id) != windows::key(window) {
        return;
    }
    if !tree::read_tree(|t| t.node(id).is_some() && reaches_painted_root(t, id)) {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        return;
    }
    // Visible when focus() queued it, hidden since: a browser would have
    // focused it then and let it go when it hid, so it does not get it now.
    if style_refuses_focus(id) {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        crate::log::record("dev", format!("[focus] queued el.focus() on node {id} dropped: CSS hid it"));
        return;
    }
    if let Some(handle) = focus_target_of(id, cx) {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        if !handle.is_focused(window) {
            // The observers report it, as they do a click.
            window.focus(&handle, cx);
            crate::log::record("dev", format!("[focus] queued el.focus() focused node {id}"));
        }
        return;
    }
    if left <= 1 {
        PENDING_FOCUS.with(|p| p.set((0, 0)));
        crate::log::record("dev", format!("[focus] queued el.focus() on node {id} dropped: it never got a focus handle"));
    } else {
        PENDING_FOCUS.with(|p| p.set((id, left - 1)));
    }
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

/// The devtools' IME (`crate::devtools::ime`): the two calls the platform's
/// input handler makes on a field, a pre-edit or a commit.
pub(crate) fn ime(id: u32, text: &str, commit: bool, window: &mut Window, cx: &mut App) -> Result<(), String> {
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

/// Select a text field's actual native editing buffer. A field mounted in
/// this JS wave does not have an entity yet; preserve the request until its
/// first build, after its controlled value has been synchronized. Key/input
/// callbacks also defer, because their own TextInputState is already borrowed.
pub(crate) fn select_input_text(id: u32) -> bool {
    let info = tree::read_tree(|t| {
        let node = t.node(id)?;
        let multiline = t.name(node.tag) == "textarea";
        if !multiline && (t.name(node.tag) != "input"
            || matches!(t.prop(id, "type").unwrap_or("text").trim().to_ascii_lowercase().as_str(), "file" | "range" | "color")) {
            return None;
        }
        let value = t.prop_write(id, "value").map(|(v, write)| (v.to_string(), write))
            .or_else(|| multiline.then(|| (node.text.clone(), 0)));
        Some((multiline, value))
    });
    let Some((multiline, value)) = info else { return false };
    if !crate::in_input_dispatch() {
        let state = INPUT_STATES.with(|m| m.borrow().get(&id).cloned());
        if let Some(state) = state {
            let applied = vn_runtime::with_app(|cx| state.update(cx, |s, cx| {
                s.set_multiline(multiline);
                if let Some((value, write)) = &value { s.sync_value(value, *write, cx); }
                s.select_all_text(cx);
            })).is_some();
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

/// The live text of a native `<input>` — the widget's own buffer, not the
/// `value` prop, so it reflects typing that JS has not echoed back yet.
pub(crate) fn input_value(id: u32, cx: &App) -> Option<String> {
    INPUT_STATES.with(|m| m.borrow().get(&id).map(|s| s.read(cx).text().to_string()))
        .or_else(|| tree::read_tree(|t| crate::range::is_range(t, id).then(|| crate::range::value(t, id))))
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
    // The same path `el.scrollBy(...)` takes, and that is the point: a devtools
    // gesture that moved the box differently from the app's own API would be a
    // harness that proves the wrong thing. `crate::scroll` clamps, invalidates
    // the enclosing cache boundaries and queues the DOM `scroll` event for the
    // next frame (where a browser fires it too).
    //
    // It used to end in `window.refresh()`, which was correct and expensive:
    // a frame drawn with gpui's `refreshing` flag up ignores EVERY
    // `Entity::cached` recording in the window, so a spec that scrolled a
    // 1 000-row list re-rendered the whole document on every step. The
    // `Tree::touch` inside `scroll::apply` invalidates exactly the boundaries
    // whose recording has stopped describing the pixels, and `frame_tick`
    // repaints because the queue is not empty.
    if let Some(info) = crate::scroll::by(id, dx, dy) {
        return Ok((info.left, info.top));
    }
    // A `<textarea>` scrolls its own overflow inside the native widget rather
    // than through a gpui `ScrollHandle`, so it has no entry there. Route to
    // the editor instead — otherwise devtools would report "not a scroll
    // container" about the one element on the page that visibly is.
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

/// Synthesizes one input event on `id` through exactly the path a real gpui
/// listener takes: publish `Phase::Commit`, hand the compact payload to the JS
/// dispatcher (which builds the W3C event and runs capture/bubble over the
/// mirror tree), then flush the tree so Vue's wave repaints in this frame.
/// Returns `false` when a handler called `preventDefault()`.
///
/// The devtools call it from the frame pump, so unlike a real listener's
/// dispatch no gpui dispatch is under it, and a forced layout may draw
/// (`crate::off_input`).
pub(crate) fn synth_event(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    crate::off_input(|| js_dispatch_app(id, event, payload, window, cx))
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

    let resolved = img_src::source_for(id, &src);
    // A stale cached decode (a blob: URL of a file picked again after it
    // changed) goes BEFORE gpui is asked for this source.
    img_src::evict_pending(window, cx);
    let Some(source) = resolved else {
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

    // gpui runs this only when the decode for the paint actually failed. The
    // DOM `load` / `error` events are not decided here but by the probe
    // (`img_src::tick`), which also answers images that are never painted.
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
                 text instead (the element's `error` event fired, unless its header read as an \
                 image and only the picture itself is corrupt: then `load` fired, as in a \
                 browser, which also paints what it can)"
            ),
        );
        alt_element(&failed_alt)
    });

    let mask = pointer_mask(node);
    // `<img>` is not focusable in HTML without `tabindex`.
    let tab_index = tab_index_of(t, id);
    // A key or focus listener makes nothing focusable (FO-3).
    let focusable = tab_index.is_some();
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
    // An `<svg>`'s silhouette is its shapes, and resvg draws a
    // `drop-shadow()` around them (`crate::svg`, `Shadow`). What gpui paints
    // for the box itself, a background or a border, is outside the raster
    // and casts none.
    let shadow = node.computed.as_ref().and_then(|cs| cs.extras.drop_shadow).map(|d| crate::svg::ShadowSpec {
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
        warn_compat(
            "svg-css-paint-server",
            "an inline <svg> takes a `fill`/`stroke` from a CSS rule that is a paint server \
             (`url(#…)`), `context-fill`/`context-stroke`, or a colour vn-css cannot resolve; \
             it is drawn in the element's `color` (`currentColor`). Concrete colours and \
             `var()` are drawn as computed, and a `url(#…)` written on the shapes \
             (`fill=\"url(#g)\"`) is drawn as written",
        );
    }

    // No `with_transformation` here any more. `transform` is a paint-level
    // property on the style now (m13 item 2), so `Style::paint` pushes the
    // matrix for the whole element and `Window::paint_svg` composes it into the
    // sprite. Setting it here as well would apply it twice.
    //
    // Mono: gpui's mask, tinted by `color`. Anything else: the full-colour
    // raster (`crate::svg` explains the split).
    // A shadow needs the colour path: the mask has one colour, the icon's.
    match (doc.paint, shadow) {
        (crate::svg::Paint::Mono, None) => {
            crate::svg::forget_raster(id);
            let e = with_control_area(gpui::svg().data(&doc.bytes).id(id as usize), id, control_area, win);
            finish_svg(e, t, id, node, window, cx, now)
        }
        _ => {
            let e = with_control_area(crate::svg::raster(id, doc.clone(), shadow).id(id as usize), id, control_area, win);
            finish_svg(e, t, id, node, window, cx, now)
        }
    }
}

/// Does the cascade give this element a background or a border of its own?
fn paints_own_box(node: &crate::tree::Node) -> bool {
    let Some(cs) = &node.computed else { return false };
    let b = &cs.base.border_widths;
    let wide = |w: Option<gpui::AbsoluteLength>| w.is_some_and(|w| w.to_pixels(gpui::px(16.0)) > gpui::px(0.0));
    let background = cs.base.background.as_ref().is_some_and(|f| {
        f.color().and_then(|c| c.as_solid()).map_or(true, |c| c.a > 0.0)
    });
    background || wide(b.top) || wide(b.right) || wide(b.bottom) || wide(b.left)
}

/// Everything after the choice of element: ARIA, the computed style, the
/// presentational size, focus and events, and the inherited-colour tint.
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
    // Not focusable in HTML without `tabindex` (an inline <svg> is not a
    // control), same as <img>.
    let tab_index = tab_index_of(t, id);
    // A key or focus listener makes nothing focusable (FO-3).
    let focusable = tab_index.is_some();
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
    let win = windows::key(window);
    // Load (once per `src`, off the UI thread) and push this frame's props
    // into the transport.
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

        // The tile the worker's frames are written into (`crate::lottie`), and
        // the box the next one is drawn for. gpui's image element implements
        // `filter: grayscale(1)` natively, but the cascade does not carry
        // `filter` yet — wire it here the day `Extras` grows the property.
        d = d.child(match crate::lottie::paint(id, win, size, window.scale_factor()) {
            Some(image) => gpui::img(gpui::ImageSource::Render(image))
                .object_fit(object_fit.into())
                .size_full()
                .into_any_element(),
            None => div().size_full().into_any_element(),
        });
    }

    let mask = pointer_mask(node);
    // Like `<img>`: not focusable in HTML without `tabindex`.
    let tab_index = tab_index_of(t, id);
    // A key or focus listener makes nothing focusable (FO-3).
    let focusable = tab_index.is_some();
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

// ---------------------------------------------------------------------------
// <canvas>
// ---------------------------------------------------------------------------

/// `<canvas>` lives in its own file, as a child of this module so that it
/// builds with the replaced-element helpers `<img>` and `<video>` use.
#[path = "canvas.rs"]
pub(crate) mod canvas;

// ---------------------------------------------------------------------------
// <vn-sticker>
// ---------------------------------------------------------------------------

/// `<vn-sticker>` lives in its own file, a child of this module for the same
/// reason `<canvas>` is.
#[path = "sticker.rs"]
pub(crate) mod sticker;

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
            let child = clip_replaced(child, d.style(), window.rem_size());
            d = d.child(child);
        }
    } else if let Some(poster) = t.prop(id, "poster").filter(|p| !p.trim().is_empty()) {
        // Before `loadeddata` — and forever, for a `src` that failed — the
        // poster is what a browser paints.
        if let Some(source) = img_src::source_for(id, poster.trim()) {
            let poster = gpui::img(source)
                .object_fit(gpui_object_fit(
                    node.computed
                        .as_ref()
                        .and_then(|cs| cs.extras.object_fit)
                        .unwrap_or(vn_css::ObjectFit::Contain),
                ))
                .size_full()
                .into_any_element();
            let poster = clip_replaced(poster, d.style(), window.rem_size());
            d = d.child(poster);
        }
    }

    let mask = pointer_mask(node);
    // Like `<img>` and `<lottie>`: not focusable in HTML without `tabindex`.
    // (A browser's `<video controls>` IS focusable, but we render no controls —
    // the app's own buttons are the focusable things.)
    let tab_index = tab_index_of(t, id);
    // A key or focus listener makes nothing focusable (FO-3).
    let focusable = tab_index.is_some();
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

/// What an `<input type=file>` paints inside its own box: the "Choose File"
/// button, then the status ("No file chosen", the file's name, "3 files"),
/// in one row whatever the input's own `display`.
///
/// The button's look is Chromium's UA default, approximately (D-F12): a
/// light face, a 1px grey border, small radius, `1px 6px` padding and a 4px
/// gap before the status. `::file-selector-button` rules refine it, which is
/// what Tailwind's `file:` variants and shadcn-vue's `<Input type="file">`
/// write. The strings are Chromium's English ones (`aria::file_input_labels`);
/// the status comes from the runtime as the reserved `:file-label` prop.
fn file_input_children(t: &Tree, id: u32, node: &crate::tree::Node, case: Option<vn_css::TextTransform>) -> AnyElement {
    use gpui::Refineable as _;
    let (button_text, status) = crate::aria::file_input_labels(t, id).unwrap_or(("Choose File", String::new()));
    let mut button = div()
        .flex_none()
        .mr(gpui::px(4.0))
        .px(gpui::px(6.0))
        .py(gpui::px(1.0))
        .rounded(gpui::px(3.0))
        .border_1()
        .border_color(gpui::rgb(0x767676))
        .bg(gpui::rgb(0xefefef))
        .text_color(gpui::black());
    if let Some(r) = node.computed.as_ref().and_then(|cs| cs.file_selector_button.as_ref()) {
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
    state.update(cx, |s, _| s.set_multiline(multiline));
    // Web fonts load lazily: the field's value and placeholder ask for theirs.
    for shown in [t.prop(id, "value"), t.prop(id, "placeholder"), Some(node.text.as_str())].into_iter().flatten() {
        crate::fonts::want_for_text(t, id, shown);
    }

    // Controlled value: the write counter, not the string, is what tells the
    // widget an assignment happened (`v-model` re-forcing "" after the user
    // typed again writes the same string the previous clear did).
    if let Some((value, write)) = t.prop_write(id, "value") {
        let value = value.to_string();
        // A value assigned in the middle of an IME composition ends it: its
        // `compositionend` goes out after this frame.
        state.update(cx, |s, cx| {
            s.sync_value(&value, write, cx);
            s.flush_abandoned_composition(window, cx);
        });
    } else if multiline && !node.text.is_empty() {
        // A `<textarea>`'s initial value is its TEXT CONTENT, not an attribute
        // — `<textarea>hello</textarea>`. Vue writes that through
        // `setElementText`, so it arrives on `Node::text`, and the wrapper's
        // `d.child(node.text)` arm is never reached for a text field. Applied
        // once (write counter 0), so a later `value` assignment still wins.
        let value = node.text.clone();
        state.update(cx, |s, cx| s.sync_value(&value, 0, cx));
    }

    if INPUT_SELECT_ALL.with(|m| m.borrow_mut().remove(&id)) {
        state.update(cx, |s, cx| s.select_all_text(cx));
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
    let masked = !multiline
        && t
            .prop(id, "type")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("password"));
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
        input_config: {
            let mut config = input_configuration(t, id, multiline);
            // A browser offers no corrections or suggestions for a password,
            // whatever the attributes say: they would learn it.
            if masked {
                config.autocorrect = false;
                config.suggestions = false;
            }
            config
        },
        // Read every frame, so a "show password" toggle that switches `type`
        // to `text` unmasks the value on the next frame.
        masked,
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

    // vn-elements reports an edit from a `window.defer`, outside gpui's key
    // dispatch, so a forced layout may draw in the `input` handler. The edit's
    // `inputType`, `isComposing` and `data` ride along (`tree::EV_BEFOREINPUT`).
    let on_change: vn_elements::OnChange = std::rc::Rc::new(move |text, intent, window, cx| {
        let payload = EventPayload { key: Some(text), ..intent_payload(intent) };
        crate::off_input(|| js_dispatch_text(id, tree::EV_INPUT, &payload, intent.data.as_deref(), window, cx));
    });

    // `beforeinput`, from inside the widget's edit and before it changes
    // anything; `false` cancels the edit. Only built when something on the
    // field's path listens (the root's bit stands for `document`/`window`).
    let on_before_input: vn_elements::OnBeforeInput =
        std::rc::Rc::new(move |intent, window: &mut Window, cx: &mut App| {
            if !tree::read_tree(|t| path_listens(t, id, tree::EV_BEFOREINPUT)) {
                return true;
            }
            let payload = intent_payload(intent);
            let proceed = js_dispatch_text(id, tree::EV_BEFOREINPUT, &payload, intent.data.as_deref(), window, cx);
            if proceed {
                // The widget edits next.
                settle_before_default(window, cx);
            }
            proceed
        });

    // The IME composition's events. `End` comes deferred (after the commit's
    // `input`), outside gpui's dispatch, as `input` does.
    let on_composition: vn_elements::OnComposition =
        std::rc::Rc::new(move |phase, data, window: &mut Window, cx: &mut App| {
            let code = match phase {
                vn_elements::CompositionPhase::Start => tree::EV_COMPOSITIONSTART,
                vn_elements::CompositionPhase::Update => tree::EV_COMPOSITIONUPDATE,
                vn_elements::CompositionPhase::End => tree::EV_COMPOSITIONEND,
            };
            if !tree::read_tree(|t| path_listens(t, id, code)) {
                return;
            }
            let payload = EventPayload::default();
            if phase == vn_elements::CompositionPhase::End {
                crate::off_input(|| js_dispatch_text(id, code, &payload, Some(data), window, cx));
            } else {
                js_dispatch_text(id, code, &payload, Some(data), window, cx);
            }
        });

    // Keystrokes are forwarded *before* the widget acts on them, so a JS
    // handler that calls `preventDefault()` cancels the default editing
    // (`dispatch_event` already reports `!defaultPrevented`).
    let on_key_down: Option<vn_elements::OnKeyDown> = bit(tree::EV_KEYDOWN).then(|| {
        std::rc::Rc::new(move |ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App| {
            // The field's interceptor runs before any listener, so it is the
            // one `keydown` of this key event; the root's listener, which
            // gpui reaches afterwards even when this handler removed the
            // field, lets it pass (`claim_key_event`).
            if !claim_key_event(window) {
                return true;
            }
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            let proceed = js_dispatch_app(id, tree::EV_KEYDOWN, &payload, window, cx);
            if !proceed {
                // The root's key listeners (clipboard, Shift+F10) run after
                // the field's and read this.
                windows::here(window, |w| w.key_prevented = true);
            } else {
                // The widget acts on the key next.
                settle_before_default(window, cx);
            }
            proceed
        }) as vn_elements::OnKeyDown
    });
    let on_key_up: Option<vn_elements::OnKeyUp> = bit(tree::EV_KEYUP).then(|| {
        std::rc::Rc::new(move |ev: &KeyUpEvent, window: &mut Window, cx: &mut App| {
            if !claim_key_event(window) {
                return;
            }
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            if !js_dispatch_app(id, tree::EV_KEYUP, &payload, window, cx) {
                windows::here(window, |w| w.key_prevented = true);
            }
        }) as vn_elements::OnKeyUp
    });

    // Copy / cut / paste in the field: the ClipboardEvent first, the widget's
    // own clipboard work only if nothing cancelled it (`crate::clipboard`).
    let on_clipboard: vn_elements::OnClipboard =
        std::rc::Rc::new(move |command, window: &mut Window, cx: &mut App| {
            let proceed = crate::clipboard::fire(id, crate::clipboard::Verb::of_command(command), window, cx);
            if proceed {
                // The widget copies, cuts or pastes next.
                settle_before_default(window, cx);
            }
            proceed
        });

    // What the keydown's or the clipboard event's handlers assigned, for the
    // widget to edit (`settle_before_default` ran their microtasks).
    let controlled_value: vn_elements::ControlledValue = std::rc::Rc::new(move |_| {
        tree::read_tree(|t| t.prop_write(id, "value").map(|(v, write)| (v.to_string(), write)))
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
        Press { target, button, position: Point::default(), clickable: true, cancelled: false }
    }

    #[test]
    fn dom_press_survives_listener_rebuild_until_its_matching_release() {
        // The pressed DOM node stays alive while its GPUI listeners/view
        // wrappers are rebuilt. Pairing belongs to WindowState, not either
        // listener's element state, and an unrelated release cannot consume it.
        let mut state = windows::WindowState::default();
        state.press = Some(dom_press(42, 0.0));
        assert!(Press::take_for_release(&mut state.press, 2.0).is_none());
        assert_eq!(state.press.as_ref().map(|p| p.target), Some(42));
        let wave = UpWave { press: Press::take_for_release(&mut state.press, 0.0), ..Default::default() };
        assert_eq!(wave.click_press(), Some(42));
        assert!(state.press.is_none());
        assert!(Press::take_for_release(&mut state.press, 0.0).is_none(), "one press cannot click twice");
        assert_eq!(UpWave::default().click_press(), None, "a release without a press cannot click");
    }

    #[test]
    fn cancelled_or_ineligible_dom_presses_cannot_click() {
        let mut cancelled = dom_press(42, 0.0);
        cancelled.cancelled = true;
        assert!(!cancelled.is_active());
        assert_eq!(UpWave { press: Some(cancelled), ..Default::default() }.click_press(), None);
        let mut disabled_or_native_drag = dom_press(42, 0.0);
        disabled_or_native_drag.clickable = false;
        assert_eq!(UpWave { press: Some(disabled_or_native_drag), ..Default::default() }.click_press(), None);
    }

    /// A POSITIONED box with this `z-index` (the dialog's `fixed z-50`, its
    /// header's `sticky z-10`): positioned, so the z applies.
    fn z(z_index: i32) -> vn_css::Extras {
        vn_css::Extras { z_index: Some(Some(z_index)), positioned: Some(true), ..Default::default() }
    }

    fn fixed() -> vn_css::Extras {
        vn_css::Extras { fixed: Some(true), positioned: Some(true), ..Default::default() }
    }

    /// The layer of a box in a BLOCK parent; `fixed` is gpui's `Absolute`.
    fn layer(x: &vn_css::Extras) -> Layer {
        let position = (x.fixed == Some(true)).then_some(gpui::Position::Absolute);
        layer_of(x, position, false, None)
    }

    /// One box of the paint-order model: its name stands for its own
    /// background, its children are its content.
    struct B {
        name: &'static str,
        layer: Layer,
        kids: Vec<B>,
    }

    fn b(name: &'static str, layer: Layer, kids: Vec<B>) -> B {
        B { name, layer, kids }
    }

    /// What a box lowers to: painted in its parent's paint, or a deferred draw.
    enum Item {
        Paint(&'static str),
        Inline(Vec<Item>),
        Deferred(std::rc::Rc<[u128]>, Vec<Item>),
    }

    /// `build_node`'s stacking calls, for one box of the model: enter, the
    /// open context, the in-flow split and the lift of a split context.
    fn lower(node: &B, parent: &Stacking, is_root: bool) -> Item {
        let layer = if is_root { Layer::InFlow } else { node.layer };
        let Entered { own, inside, opens, negative } = parent.enter(layer, next_stack_ordinal());
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
            kids = kids.into_iter().map(|k| Item::Deferred(inside.in_flow_key(), vec![k])).collect();
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

    /// The order gpui paints a tree of boxes in: the root pass, then every
    /// deferred draw, whatever round it was prepainted in, by key (a stable
    /// sort, as gpui's), each painting what is not deferred inside it. The
    /// hit test walks the same order backwards (the fork's `vn_hit_order`).
    fn paint_order(root: &B) -> Vec<&'static str> {
        reset_stacking();
        let (mut painted, mut draws) = (Vec::new(), Vec::new());
        flatten(lower(root, &Stacking::default(), true), &mut painted, &mut draws);
        draws.sort_by(|a, b| a.0.cmp(&b.0));
        painted.extend(draws.into_iter().flat_map(|(_, names)| names));
        painted
    }

    /// The Argon profile popover: two portalled `fixed z-50` overlays, the
    /// popover FIRST, with `z-index: 0 / 1 / 2` layers inside it, and the
    /// menu it opens after it. The popover is painted whole, layers included,
    /// before the menu — the layers used to paint (and take clicks) over the
    /// menu's items, because they were deferred one round deeper.
    #[test]
    fn a_context_paints_whole_before_a_later_sibling_of_the_same_z() {
        let tree = b("root", Layer::InFlow, vec![
            b("app", Layer::InFlow, vec![b("page", Layer::InFlow, vec![])]),
            b("popover", layer(&z(50)), vec![b("inner", Layer::InFlow, vec![
                b("card-bg", Layer::Context(0), vec![]),
                b("hero", layer(&z(1)), vec![]),
                b("glass", layer(&z(2)), vec![b("glass-body", Layer::InFlow, vec![])]),
            ])]),
            b("menu", layer(&z(50)), vec![b("item", Layer::InFlow, vec![])]),
        ]);
        assert_eq!(
            paint_order(&tree),
            ["root", "app", "page", "popover", "inner", "card-bg", "hero", "glass", "glass-body", "menu", "item"]
        );
    }

    /// A negative `z-index` paints over its context's own background and
    /// under its context's in-flow content — wherever it is in the tree, and
    /// in flow or not. Lower first; the context's later siblings are not
    /// touched, and neither is the root's background.
    #[test]
    fn a_negative_z_paints_between_its_contexts_background_and_content() {
        let tree = b("root", Layer::InFlow, vec![
            b("card", layer(&z(1)), vec![
                b("text", Layer::InFlow, vec![]),
                b("glow", Layer::Context(-1), vec![]),
                b("box", Layer::InFlow, vec![b("deep", layer(&z(-2)), vec![])]),
            ]),
            b("after", Layer::InFlow, vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "after", "card", "deep", "glow", "text", "box"]);

        // In the ROOT context: over the root's background, under the page.
        let tree = b("root", Layer::InFlow, vec![
            b("page", Layer::InFlow, vec![b("backdrop", layer(&z(-1)), vec![])]),
            b("footer", Layer::InFlow, vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "backdrop", "page", "footer"]);

        // An `isolate` (or `opacity`, …) box that paints in flow keeps a
        // negative z inside it: it is lifted whole to its own slot, and the
        // decoration goes behind its content and no further.
        let tree = b("root", Layer::InFlow, vec![
            b("avatar", Layer::InFlowContext, vec![
                b("face", Layer::InFlow, vec![]),
                b("frame", layer(&z(-1)), vec![]),
            ]),
            b("name", Layer::InFlow, vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "name", "avatar", "frame", "face"]);
    }

    /// Three contexts deep, with z-indices that would interleave if they were
    /// one flat rank: `a1x`'s z 100 stays inside `a` (z 1), so it paints
    /// under `c`, a LATER z-1 sibling of `a`, and under `b` (z 2).
    #[test]
    fn nested_contexts_sort_by_their_path() {
        let tree = b("root", Layer::InFlow, vec![
            b("a", layer(&z(1)), vec![
                b("a1", layer(&z(3)), vec![
                    b("a1x", layer(&z(100)), vec![]),
                    b("a1y", layer(&z(-5)), vec![]),
                    b("a1-text", Layer::InFlow, vec![]),
                ]),
                b("a2", layer(&z(2)), vec![]),
            ]),
            b("b", layer(&z(2)), vec![
                b("b1", layer(&z(-1)), vec![]),
                b("b-text", Layer::InFlow, vec![]),
            ]),
            b("c", layer(&z(1)), vec![]),
        ]);
        assert_eq!(
            paint_order(&tree),
            ["root", "a", "a2", "a1", "a1y", "a1-text", "a1x", "c", "b", "b1", "b-text"]
        );
    }

    /// `opacity < 1` or a `transform` makes a stacking context at z auto. A
    /// z-100 box inside it sorts INSIDE it, so it paints under a later z-10
    /// sibling of the faded box. In flow the box itself keeps its tree-order
    /// paint (the stated limit); on an `absolute` box it is deferred at z 0.
    #[test]
    fn an_effect_makes_a_context_at_z_auto() {
        let style = |f: &dyn Fn(&mut gpui::StyleRefinement)| {
            let mut s = gpui::StyleRefinement::default();
            f(&mut s);
            s
        };
        let none = vn_css::Extras::default();
        assert!(!effect_context(&none, &style(&|_| {})));
        assert!(effect_context(&none, &style(&|s| s.opacity = Some(0.5))));
        assert!(!effect_context(&none, &style(&|s| s.opacity = Some(1.0))));
        let mut moved = gpui::TransformSpec::IDENTITY;
        moved.scale = (0.95, 0.95);
        assert!(effect_context(&none, &style(&|s| s.vn_transform = Some(moved))));
        assert!(!effect_context(&none, &style(&|s| s.vn_transform = Some(gpui::TransformSpec::IDENTITY))));
        assert!(effect_context(&none, &style(&|s| s.backdrop_blur = Some(gpui::px(8.0)))));
        let isolated = vn_css::Extras { isolate: Some(true), ..Default::default() };
        assert!(effect_context(&isolated, &style(&|_| {})));

        let abs = Some(gpui::Position::Absolute);
        assert_eq!(layer_of(&none, None, true, None), Layer::InFlowContext);
        assert_eq!(layer_of(&none, abs, true, None), Layer::Context(0));
        assert_eq!(layer_of(&none, abs, false, None), Layer::Positioned);

        let tree = b("root", Layer::InFlow, vec![
            b("faded", Layer::InFlowContext, vec![b("lifted", layer(&z(100)), vec![])]),
            b("later", layer(&z(10)), vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "faded", "lifted", "later"]);

        let tree = b("root", Layer::InFlow, vec![
            b("moved", layer_of(&none, abs, true, None), vec![b("inner", layer(&z(5)), vec![])]),
            b("badge", Layer::Positioned, vec![]),
            b("later", layer(&z(1)), vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "moved", "inner", "badge", "later"]);
    }

    /// `position: fixed` with no `z-index` is still a context, so a z-indexed
    /// descendant of a portalled overlay sorts inside it rather than against
    /// the document; `z-index: 0` is one too, and `auto` is not.
    #[test]
    fn fixed_and_z_zero_are_contexts_and_auto_is_not() {
        assert_eq!(layer(&fixed()), Layer::Context(0));
        let zero_rel = z(0);
        assert_eq!(layer(&zero_rel), Layer::InFlowContext);
        assert_eq!(layer_of(&zero_rel, Some(gpui::Position::Absolute), false, None), Layer::Context(0));
        let auto = vn_css::Extras { z_index: Some(None), positioned: Some(true), ..Default::default() };
        assert_eq!(layer(&auto), Layer::InFlow);
        let sticky = vn_css::Extras { sticky: Some(true), ..auto.clone() };
        assert_eq!(layer(&sticky), Layer::InFlowContext);

        let tree = b("root", Layer::InFlow, vec![
            b("overlay", layer(&fixed()), vec![b("inside", layer(&z(1)), vec![])]),
            b("document-z1", layer(&z(1)), vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "overlay", "inside", "document-z1"]);
        let tree = b("root", Layer::InFlow, vec![
            b("zero", layer(&zero_rel), vec![b("inside", layer(&z(9)), vec![])]),
            b("auto", layer(&auto), vec![b("escapes", layer(&z(8)), vec![])]),
            b("document-z1", layer(&z(1)), vec![]),
        ]);
        assert_eq!(paint_order(&tree), ["root", "zero", "auto", "inside", "document-z1", "escapes"]);
    }

    /// An element that is not out of flow neither defers nor opens a context,
    /// so an ordinary subtree costs nothing.
    #[test]
    fn an_in_flow_element_changes_nothing() {
        let root = Stacking::default();
        let e = root.enter(layer(&vn_css::Extras::default()), 0);
        assert!(e.own.is_none() && !e.opens);
        assert_eq!(e.inside.key, root.key);
        assert_eq!(e.inside.deferred, root.deferred);
    }

    /// m26 6d, rule 1: `z-index` applies to a positioned box, or to a flex or
    /// grid item even when it is static (CSS 2.1 §9.9.1, css-flexbox §4.3,
    /// css-grid §9). Anywhere else it is inert. The Argon client's
    /// `<Input class="z-10">` is a static block in a block parent, and it used
    /// to lift over the absolute error badge beside it.
    #[test]
    fn z_index_applies_only_to_positioned_boxes_and_flex_or_grid_items() {
        let z10 = |positioned: Option<bool>| vn_css::Extras {
            z_index: Some(Some(10)),
            positioned,
            ..Default::default()
        };
        let block = None;
        let flex = Some(gpui::Display::Flex);
        let grid = Some(gpui::Display::Grid);

        let at = |x: &vn_css::Extras, parent| layer_of(x, None, false, parent);
        assert_eq!(at(&z10(None), block), Layer::InFlow, "static (undeclared) in a block");
        assert_eq!(at(&z10(Some(false)), block), Layer::InFlow, "`position: static`");
        assert_eq!(at(&z10(Some(false)), Some(gpui::Display::Block)), Layer::InFlow);
        assert_eq!(at(&z10(None), flex), Layer::Context(10), "a static flex item");
        assert_eq!(at(&z10(None), grid), Layer::Context(10), "a static grid item");
        assert_eq!(at(&z10(Some(true)), block), Layer::Context(10), "relative in a block");
        assert!(!z_applies(&z10(None), block) && z_applies(&z10(None), flex));

        // An inert z opens no context, so the subtree's own z-indices keep
        // sorting in the outer one.
        let root = Stacking::default();
        let e = root.enter(at(&z10(None), block), 0);
        assert!(e.own.is_none() && !e.opens);
        assert_eq!(e.inside.key, root.key);
    }

    /// m26 6d, rule 2: a `position: absolute` box with `z-index: auto` paints
    /// after ALL the in-flow content of its stacking context (CSS 2.1
    /// Appendix E, step 8), in tree order with the other positioned boxes, and
    /// it opens no context. So the badge (before the `<Input>` in tree order)
    /// paints over it, a z>0 box of the same context still paints above the
    /// badge, and a z>0 DESCENDANT of the badge sorts in the outer context.
    #[test]
    fn an_absolute_z_auto_box_paints_after_in_flow_content_and_opens_no_context() {
        let absolute = vn_css::Extras { positioned: Some(true), ..Default::default() };
        let abs = Some(gpui::Position::Absolute);
        assert_eq!(layer_of(&absolute, abs, false, None), Layer::Positioned);
        assert_eq!(layer_of(&absolute, None, false, None), Layer::InFlow, "relative: the stated limit");

        let tree = b("root", Layer::InFlow, vec![
            b("field", Layer::InFlow, vec![
                b("badge", Layer::Positioned, vec![
                    b("badge-text", Layer::InFlow, vec![]),
                    b("badge-dot", Layer::Positioned, vec![]),
                    b("badge-tip", layer(&z(5)), vec![]),
                ]),
                b("input", Layer::InFlow, vec![]),
            ]),
            b("z1-box", layer(&z(1)), vec![]),
            b("later-abs", Layer::Positioned, vec![]),
        ]);
        assert_eq!(
            paint_order(&tree),
            ["root", "field", "input", "badge", "badge-text", "badge-dot", "later-abs", "z1-box", "badge-tip"]
        );
    }

    /// The cache compares `Inherited`, and must not see a stacking KEY: its
    /// ordinals move whenever anything earlier in the document does.
    #[test]
    fn the_cache_does_not_compare_stacking_keys() {
        let root = Stacking::default();
        let a = Inherited { stack: root.enter(layer(&z(1)), 3).inside, ..Default::default() };
        let b = Inherited { stack: root.enter(layer(&z(1)), 7).inside, ..Default::default() };
        assert_ne!(a.stack.key, b.stack.key);
        assert!(a == b);
    }

    /// Each deferred box inside another costs gpui one more prepaint round.
    /// Past the limit a box paints in flow, and a context stays a context.
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
        assert!(!s.enter(Layer::Context(-1), 99).negative, "painted in flow: nothing to split");
        let e = s.enter(Layer::Context(3), 99);
        assert!(e.own.is_none() && e.opens && e.inside.key.len() == s.key.len() + 1);
    }

    /// The ranks: every negative z, then a context's in-flow content, then z
    /// 0 and up — and the ordinal only breaks a tie of z.
    #[test]
    fn slots_order_z_then_tree_order() {
        assert!(z_rank(i32::MIN) < z_rank(-1));
        assert!(z_rank(-1) < IN_FLOW && IN_FLOW < z_rank(0));
        assert!(z_rank(0) < z_rank(1) && z_rank(1) < z_rank(i32::MAX));
        assert!(slot(z_rank(1), 9) < slot(z_rank(2), 0));
        assert!(slot(z_rank(1), 0) < slot(z_rank(1), 1));
    }

    /// Enter and Space click only what a browser clicks from the keyboard. A
    /// `div role=button tabindex=0` handles its own keys (reka's menu items,
    /// shadcn's toggles), and a click from the host would run its action twice.
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
        // (element, Enter clicks, Space clicks)
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
            (el("div", &[("role", "button"), ("tabindex", "0")]), false, false),
        ];
        for (i, (id, enter, space)) in cases.into_iter().enumerate() {
            assert_eq!(
                (keyboard_activates(id, false), keyboard_activates(id, true)),
                (enter, space),
                "case {i}"
            );
        }
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

    /// m26 #10: HTML's focusable-by-default elements take tab index 0 with no
    /// `tabindex`, and an actually-disabled control is not focusable at all.
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

            // Only the summary that summarizes its details.
            let details = el(t, "details", root, &[]);
            let first = el(t, "summary", details, &[]);
            let second = el(t, "summary", details, &[]);
            assert_eq!(default_tab_index(t, first), Some(0));
            assert_eq!(default_tab_index(t, second), None);

            // Disabled, directly or by a fieldset, except in its first legend.
            let off = el(t, "button", root, &[("disabled", ""), ("tabindex", "0")]);
            let on = el(t, "button", root, &[("disabled", "false")]);
            assert!(actually_disabled(t, off) && !could_take_focus(t, off), "disabled, tabindex or not");
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

    /// FO-3 (docs/RUNTIME.md §5.2): a key or focus listener makes nothing
    /// focusable, as in Chrome 156, where a press on a `<span>` with a focus
    /// listener or a `<div>` with a keydown listener blurs to `body` and
    /// `focus()` on either is refused. A `tabindex`, even -1, does, and one
    /// that is not an integer is ignored.
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

    /// m26 client sweep #36: a `<button>` that is not a flex or grid container
    /// centres its content vertically, safely; one that is keeps its own
    /// alignment, and so does a button whose author wrote `align-content`.
    #[test]
    fn a_block_button_centres_its_content_safely() {
        with_tree(|t| {
            let button = t.intern("button");
            let div = t.intern("div");
            let declared = |t: &mut Tree, tag: u32, display: Option<gpui::Display>| {
                let id = t.create_element(tag);
                let mut base = gpui::StyleRefinement::default();
                base.display = display;
                let mut cs = vn_css::ComputedStates::default();
                cs.base = std::rc::Rc::new(base);
                if let Some(n) = t.node_mut(id) {
                    n.computed = Some(cs);
                }
                id
            };
            let laid = |t: &Tree, id: u32, authored: Option<gpui::AlignContent>| {
                let mut s = gpui::StyleRefinement::default();
                s.align_content = authored;
                apply_button_content_box(&mut s, t, t.node(id).unwrap());
                s.align_content
            };
            let safe = Some(gpui::AlignContent::SafeCenter);

            let bare = declared(t, button, None);
            assert_eq!(laid(t, bare, None), safe, "no display: a block container");
            let block = declared(t, button, Some(gpui::Display::Block));
            assert_eq!(laid(t, block, None), safe, "block, inline-block, inline");
            let flex = declared(t, button, Some(gpui::Display::Flex));
            assert_eq!(laid(t, flex, None), None, "a flex button is a flex container as normal");
            let grid = declared(t, button, Some(gpui::Display::Grid));
            assert_eq!(laid(t, grid, None), None, "and a grid one a grid container");
            let start = Some(gpui::AlignContent::Start);
            assert_eq!(laid(t, bare, start), start, "the author's align-content wins");
            let plain = declared(t, div, None);
            assert_eq!(laid(t, plain, None), None, "only a button");
        });
    }

    /// `text-align: center` centres an inline-level box with auto margins in
    /// a BLOCK container only. In a flex or grid container (declared, or one
    /// `apply_inline_row` made) it is an item, and auto margins would move it:
    /// the Argon client's `.icon-box` landed at x=852 instead of 694.
    #[test]
    fn text_align_adds_auto_margins_in_a_block_parent_only() {
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");
            let extras = vn_css::Extras { inline_level: Some(true), ..Default::default() };
            let centred = |t: &Tree, child: u32| {
                let mut s = gpui::StyleRefinement::default();
                apply_inline_align(&mut s, t, t.node(child).unwrap(), &extras, Some(gpui::TextAlign::Center));
                (s.margin.left, s.margin.right)
            };
            let with_display = |t: &mut Tree, id: u32, display: gpui::Display| {
                let mut base = gpui::StyleRefinement::default();
                base.display = Some(display);
                let mut cs = vn_css::ComputedStates::default();
                cs.base = std::rc::Rc::new(base);
                if let Some(n) = t.node_mut(id) {
                    n.computed = Some(cs);
                }
            };
            let auto = (Some(gpui::Length::Auto), Some(gpui::Length::Auto));

            // A block parent with a block sibling (no inline row): centred.
            let block = t.create_element(div);
            let badge = t.create_element(div);
            let sibling = t.create_element(div);
            t.insert(badge, block, 0);
            t.insert(sibling, block, 1);
            assert_eq!(centred(t, badge), auto);

            // Declared flex, and declared grid: left alone.
            for display in [gpui::Display::Flex, gpui::Display::Grid] {
                let parent = t.create_element(div);
                let item = t.create_element(div);
                t.insert(item, parent, 0);
                with_display(t, parent, display);
                assert_eq!(centred(t, item), (None, None), "{display:?}");
            }

            // A block parent that `apply_inline_row` turns into a flex row
            // (text beside a span): left alone, the row's justify-content
            // already says `center`.
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

    /// The Menu key and the function keys reach JS under the DOM's names.
    #[test]
    fn keys_reach_js_under_their_dom_names() {
        assert_eq!(dom_key("menu"), "ContextMenu");
        assert_eq!(dom_key("f10"), "F10");
        assert_eq!(dom_key("f1"), "F1");
        assert_eq!(dom_key("f24"), "F24");
        // Not a function key: a letter, and a key name that only starts with f.
        assert_eq!(dom_key("f"), "f");
        assert_eq!(dom_key("f25"), "f25");
        assert_eq!(dom_key("fn"), "fn");
    }

    /// `detail` rides `d` above the modifier bits, so the mods still read.
    #[test]
    fn the_click_count_rides_above_the_modifiers() {
        use gpui::px;
        let m = gpui::Modifiers { shift: true, ..Default::default() };
        let p = mouse_payload_n(Point::new(px(1.0), px(2.0)), 0.0, &m, 2);
        assert_eq!(p.d as u32 & 0xf, 2, "shift");
        assert_eq!(p.d as u32 >> 4, 2, "detail");
        assert_eq!(mouse_payload_n(Point::new(px(0.0), px(0.0)), 0.0, &m, 9999).d as u32 >> 4, 255);
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

    // ---- the render-side advisories m22 narrowed (engine-round.md §4.2/§4.3) --

    /// Did this advisory key ever fire on this thread? `warn_compat_at` records
    /// one line per key for the life of the process, and the set it dedupes
    /// against is the thread-local `WARNED` — so a test thread sees exactly its
    /// own, and libtest gives every test its own thread.
    fn spoke(key: &str) -> bool {
        WARNED.with(|w| w.borrow().contains(key))
    }

    fn computed(base: gpui::StyleRefinement, extras: vn_css::Extras) -> vn_css::ComputedStates {
        vn_css::ComputedStates { base: std::rc::Rc::new(base), extras, ..Default::default() }
    }

    fn displayed(d: gpui::Display) -> gpui::StyleRefinement {
        gpui::StyleRefinement { display: Some(d), ..Default::default() }
    }

    fn inline_level() -> vn_css::Extras {
        vn_css::Extras { inline_level: Some(true), ..Default::default() }
    }

    /// `display: inline-*` maps onto its block-level equivalent — taffy models
    /// the inner display type and has no line box — but CSS itself blockifies
    /// most of the cases (CSS Display 3 §2.7: a flex item, a grid item, an
    /// out-of-flow box), and `apply_inline_row` covers the rest by turning a
    /// block container full of inline content into a wrapping flex row, whose
    /// items are sized to their content. What is genuinely left is an in-flow
    /// inline-level box with a block-level sibling and no width of its own: it
    /// takes a line to itself and stretches across it.
    ///
    /// The meet tour tripped this three times and not one of them was real
    /// (engine-round.md §4.3). This pins each silence to its own reason.
    #[test]
    fn advise_inline_level_is_silent_wherever_css_blockifies_too() {
        const KEY: &str = "display-inline-level-stretches";
        with_tree(|t| {
            let div = t.intern("div");
            let span = t.intern("span");
            let auto = gpui::StyleRefinement::default();

            // A flex ITEM: blockified by CSS, so nothing diverges.
            let flex = t.create_element(div);
            t.node_mut(flex).unwrap().computed =
                Some(computed(displayed(gpui::Display::Flex), Default::default()));
            let item = t.create_element(span);
            t.insert(item, flex, 0);
            advise_inline_level(t, item, t.node(item).unwrap(), &inline_level(), &auto);
            assert!(!spoke(KEY), "a flex item is blockified by CSS");

            // A grid item, for the same reason.
            let grid = t.create_element(div);
            t.node_mut(grid).unwrap().computed =
                Some(computed(displayed(gpui::Display::Grid), Default::default()));
            let cell = t.create_element(span);
            t.insert(cell, grid, 0);
            advise_inline_level(t, cell, t.node(cell).unwrap(), &inline_level(), &auto);
            assert!(!spoke(KEY), "a grid item is blockified by CSS");

            // A block container with a block-level sibling in it — the shape
            // that DOES stretch. Every silence below is that same element,
            // saved by one of the other rules.
            let block = t.create_element(div);
            t.node_mut(block).unwrap().computed =
                Some(computed(displayed(gpui::Display::Block), Default::default()));
            let badge = t.create_element(span);
            t.insert(badge, block, 0);
            let sibling = t.create_element(div);
            t.insert(sibling, block, 1);
            t.node_mut(sibling).unwrap().computed =
                Some(computed(displayed(gpui::Display::Block), Default::default()));

            // Out of flow: blockified by CSS as well.
            let absolute = gpui::StyleRefinement {
                position: Some(gpui::Position::Absolute),
                ..Default::default()
            };
            advise_inline_level(t, badge, t.node(badge).unwrap(), &inline_level(), &absolute);
            assert!(!spoke(KEY), "an out-of-flow box is blockified by CSS");

            // An explicit width answers it too: "fills the available width
            // rather than shrinking to fit" cannot bite a box given a width.
            let mut sized = gpui::StyleRefinement::default();
            sized.size.width = Some(gpui::px(120.0).into());
            advise_inline_level(t, badge, t.node(badge).unwrap(), &inline_level(), &sized);
            assert!(!spoke(KEY), "a definite width answers it");

            // …and a block container whose laid-out children are all inline
            // content becomes a wrapping flex row, which shrink-wraps them.
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
            assert!(!spoke(KEY), "the parent becomes an inline row and sizes it to content");

            // The one that is left.
            advise_inline_level(t, badge, t.node(badge).unwrap(), &inline_level(), &auto);
            assert!(spoke(KEY), "an in-flow inline box beside a block sibling really stretches");
        });
    }

    /// `position: fixed` resolves against the viewport in CSS and against the
    /// PARENT in taffy — which agree exactly when the parent's box IS the
    /// window's box. That is true of every `<Teleport to="body">` overlay (the
    /// app root is the viewport by definition, which is where reka portals all
    /// of them) and of a full-bleed wrapper of exactly viewport size, so the
    /// advisory is geometric rather than declarative (engine-round.md §4.2).
    ///
    /// The `settled()` guard on top of it is what keeps a BOOT TRANSIENT from
    /// spending a line that can never be retracted: `100vh` is flattened at
    /// sheet-compile time, so between boot and the recompile that adopts the
    /// real client size a full-bleed wrapper is window-sized while the app root
    /// is window-minus-chrome.
    #[test]
    fn advise_out_of_flow_speaks_only_for_a_fixed_box_in_a_smaller_parent() {
        const KEY: &str = "position-fixed-not-portalled";
        let fixed = || vn_css::Extras { fixed: Some(true), ..Default::default() };
        with_tree(|t| {
            let div = t.intern("div");
            let root = t.create_root();
            // The rectangles the PREVIOUS frame painted — a build cannot see
            // this frame's layout, which is what `painted_bounds` reads.
            let win = windows::primary();
            windows::of(win, |w| {
                w.bounds.insert(root, (0.0, 0.0, 1200.0, 800.0));
            });

            // A child OF THE ROOT is portalled by definition: the root IS the
            // viewport. Four builds, so `settled`'s three cannot be the reason.
            let overlay = t.create_element(div);
            t.insert(overlay, root, 0);
            for _ in 0..4 {
                advise_out_of_flow(t, overlay, t.node(overlay).unwrap(), &fixed(), win);
            }
            assert!(!spoke(KEY), "a Teleport-to-body overlay lands on the viewport");

            // A wrapper that IS viewport-sized (`min-h-screen` with a
            // `fixed inset-0` decoration layer inside it) is laid out against a
            // box of exactly the right size — advising there would be advising
            // about a difference of zero.
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
            assert!(!spoke(KEY), "a full-bleed wrapper of viewport size diverges by nothing");

            // The genuine offender the tour left standing: a ZERO-HEIGHT
            // wrapper. And not on the first two builds — the boot guard.
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

    /// **A-M03: the viewport a `position: fixed` box resolves against is the
    /// PAINTING window's** (docs/MULTIWINDOW.md §10).
    ///
    /// A pop-out's painted root is its viewport exactly as `t.root` is the
    /// document's, so a fixed box that is a child of it anchors to that
    /// window's edges and owes nothing — which is also what a `position: fixed`
    /// element in a second browser window does. Asking the PRIMARY window's
    /// root instead compared two coordinate spaces and fired
    /// `position-fixed-not-portalled` at every `fixed` element in every
    /// pop-out, while the one that genuinely IS in a smaller wrapper in that
    /// window went unmeasured.
    #[test]
    fn a_fixed_box_in_a_popout_resolves_against_that_windows_viewport() {
        const KEY: &str = "position-fixed-not-portalled";
        let fixed = || vn_css::Extras { fixed: Some(true), ..Default::default() };
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
            // The document is 1200x800 in the main window…
            windows::of(windows::primary(), |w| {
                w.bounds.insert(root, (0.0, 0.0, 1200.0, 800.0));
            });
            (panel, toolbar, inner, pinned)
        });
        // …and a 360x280 pop-out paints `panel`, which is therefore ITS viewport
        // and is nothing like the document's box. Outside `with_tree`:
        // `set_painted_root` marks the subtree for a re-cascade and so takes the
        // tree's mutable borrow itself.
        windows::set_painted_root(popout, panel);
        windows::of(popout, |w| {
            w.bounds.insert(panel, (0.0, 0.0, 360.0, 280.0));
            w.bounds.insert(inner, (0.0, 0.0, 360.0, 40.0));
        });
        with_tree(|t| {
            // A `fixed` toolbar that is a child of the pop-out's root lands on
            // that window's viewport. Four builds, so `settled`'s three cannot
            // be the reason nothing was said.
            for _ in 0..4 {
                advise_out_of_flow(t, toolbar, t.node(toolbar).unwrap(), &fixed(), popout);
            }
            assert!(!spoke(KEY), "the pop-out's painted root IS its viewport");

            // …and a wrapper inside that window which is NOT its viewport is
            // advised, measured against the pop-out's rectangle rather than the
            // document's.
            for _ in 0..3 {
                advise_out_of_flow(t, pinned, t.node(pinned).unwrap(), &fixed(), popout);
            }
            assert!(spoke(KEY), "a 40px-tall wrapper in a 280px window is not the viewport");
        });
        windows::forget(popout);
    }

    /// `position: sticky` maps onto `relative` because taffy takes no scroll
    /// offset as a LAYOUT input — but a sticky box only ever MOVES relative to
    /// its flow position while something above it scrolls. With no scrolling
    /// ancestor, `relative` is what a browser paints too, and the app root does
    /// not scroll (the window is the viewport), which is why the meet tour
    /// never reaches this.
    #[test]
    fn advise_sticky_needs_a_real_scroll_container_above_it() {
        const KEY: &str = "position-sticky-in-scroller";
        let sticky = || vn_css::Extras { sticky: Some(true), ..Default::default() };
        with_tree(|t| {
            let div = t.intern("div");
            let root = t.create_root();
            let plain = t.create_element(div);
            t.insert(plain, root, 0);
            let header = t.create_element(div);
            t.insert(header, plain, 0);
            advise_out_of_flow(t, header, t.node(header).unwrap(), &sticky(), windows::primary());
            assert!(!spoke(KEY), "nothing above it scrolls");

            // Now give the ancestor a real `overflow-y: auto`.
            let mut scroller = gpui::StyleRefinement::default();
            scroller.overflow.y = Some(gpui::Overflow::Scroll);
            t.node_mut(plain).unwrap().computed =
                Some(computed(scroller, Default::default()));
            advise_out_of_flow(t, header, t.node(header).unwrap(), &sticky(), windows::primary());
            assert!(spoke(KEY), "inside a scroll container it really does not stay put");
        });
    }
    #[test]
    fn synthetic_inline_rows_allow_text_to_shrink_but_keep_authored_flex_minimums() {
        with_tree(|tree| {
            let div=tree.intern("div");let span=tree.intern("span");
            let parent=tree.create_element(div);
            let text=tree.create_text();tree.set_text(text,"unbrokenword".repeat(60));tree.insert(text,parent,0);
            let timestamp=tree.create_element(span);tree.set_element_text(timestamp,"11:55".into());tree.insert(timestamp,parent,0);
            assert!(synthetic_inline_item(tree,tree.node(text).expect("text")));
            let mut style=gpui::StyleRefinement::default();
            apply_inline_item_minimum(&mut style,tree,tree.node(timestamp).expect("timestamp"));
            assert_eq!(style.min_size.width,Some(gpui::px(0.0).into()));
            style.min_size.width=Some(gpui::px(12.0).into());
            apply_inline_item_minimum(&mut style,tree,tree.node(timestamp).expect("timestamp"));
            assert_eq!(style.min_size.width,Some(gpui::px(12.0).into()),"an authored minimum remains authoritative");
            let parent_node=tree.node_mut(parent).expect("parent");
            let mut computed=vn_css::ComputedStates::default();
            computed.base=std::rc::Rc::new(gpui::StyleRefinement{display:Some(gpui::Display::Flex),..Default::default()});
            parent_node.computed=Some(computed);
            assert!(!synthetic_inline_item(tree,tree.node(text).expect("text")),"a real flex container keeps CSS min-content behavior");
        });
    }

}
