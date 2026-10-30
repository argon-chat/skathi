//! **The per-window state map** — `docs/MULTIWINDOW.md` §5.6, phase 1.
//!
//! One process, one `JsRuntime`, one Vue app, one retained `Tree` — and N gpui
//! windows, each painting one designated node of that tree. Everything in this
//! module is the third of §0's three halves: the state that two windows
//! genuinely *disagree* about.
//!
//! ## What changed, and why it is a map rather than a rename
//!
//! Before phase 1 the renderer held ~15 single-slot `thread_local!`s for facts
//! that are per-window: which node has keyboard focus, where every node was
//! painted, whether this frame declared a drag region, what the `@media`
//! environment is. With one window that is exactly right and free. With two it
//! is a silent aliasing bug in every direction at once — window B focusing an
//! input fires `blur` on window A's, a rectangle recorded in B's client space
//! is returned for a hit test in A, and B's build clears the hitbox list A's
//! fallback titlebar is derived from.
//!
//! So all of them live here, in one [`WindowState`] per `gpui::WindowId`,
//! behind one accessor.
//!
//! ## "here" is passed, never guessed
//!
//! The design sketch called the accessor `here()` and left the window implicit
//! (`with_published`'s publication would supply it). **This implementation
//! takes `&Window` instead**, at every call site, because every path that
//! touches per-window state already has one:
//!
//! * the render walk — `build_root` / `build_subtree` / `build_node` are handed
//!   `&mut Window` by gpui;
//! * element phases — `Element::{request_layout, prepaint, paint}` likewise;
//! * every input listener — `cx.listener(|_, ev, window, cx| …)`;
//! * the frame pump — `frame_tick(window, cx)`, and the pop-out equivalent
//!   [`crate::window_frame_tick`].
//!
//! An implicit "current window" would have been a second source of truth that
//! can be wrong (a `cx.defer` runs with no publication; a host fn called from
//! window A can dispatch into window B). `key(window)` cannot be wrong: it is
//! the identity of the window whose stack frame we are standing in.
//!
//! The two paths that genuinely have no window — `gc_node_state` and
//! `flush_changes`, which run off a JS wave rather than a frame — iterate every
//! window instead ([`each`]). They already iterate every node.
//!
//! ## The invariant every node-keyed map still rests on
//!
//! `Tree::alloc` allocates from one slab, so **node ids are globally unique
//! across windows**, and `__vn_window_open` refuses a second window on a node
//! that already has one (A-M02). Therefore *a node id is painted by exactly one
//! window*, which is what makes [`window_of_node`] a function rather than a
//! relation, and what keeps `INPUT_STATES`, `SCROLL_HANDLES`, `LOTTIE_STATES`,
//! `VIDEO_STATES` and friends correct as plain node-keyed maps (§5.3).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{FocusHandle, RenderImage, Subscription, Window, WindowControlArea};

/// A gpui `WindowId`, flattened to the `u64` the devtools protocol and the
/// pop-out registry already speak. `0` is not a legal gpui window id, so it is
/// free to mean "no window yet" ([`UNBOUND`]).
pub type WinKey = u64;

/// The key of a window that does not exist — used before `set_primary` has run
/// (tests, and the frames between `App::run` and the first `open_window`).
pub const UNBOUND: WinKey = 0;

/// A rectangle in ONE window's client space, CSS px: `(x, y, width, height)`.
pub type Rect = (f32, f32, f32, f32);

/// Everything one OS window knows that another one disagrees about.
///
/// Fields are grouped by the §5 classification they came from, and each carries
/// the reason it could not stay a single slot.
#[derive(Default)]
pub struct WindowState {
    // --- identity ---------------------------------------------------------
    /// The node this window paints as its root: `0` for the primary window
    /// (which paints `Tree::root` and everything under it), the popped-out node
    /// for a pop-out. This is the key the devtools protocol calls a window's
    /// "root", and the one `bounds_in` is addressed by.
    pub painted_root: u32,

    // --- focus (§5.1 hazard, the trio) ------------------------------------
    /// The node that owns keyboard focus IN THIS WINDOW (0 = none).
    ///
    /// Two windows can each have a focused node — that is what focus *is* on a
    /// desktop. One slot made window B focusing an input fire `blur` on window
    /// A's, which is not a rendering bug but a wrong DOM event.
    pub focused_node: u32,
    /// The focused node that was detached from this window's painted tree and
    /// must lose gpui focus (0 = nothing pending). Queued by `gc_node_state`
    /// (which has no `Window`), applied by the next frame tick that does.
    pub pending_blur: u32,
    /// gpui `on_focus` subscriptions, one per focusable node **per window**.
    ///
    /// `watch_focus` subscribed once per node id, against whichever window
    /// built it first; a node painted in a pop-out therefore never got its own
    /// subscription and its `focus`/`blur` events never fired. The handle
    /// itself is App-scoped (`cx.focus_handle()` lives in `App::focus_handles`)
    /// and stays in the shared `FOCUS_HANDLES` map — it is the *subscription*
    /// that is per-window, because `cx.on_focus(handle, window, …)` names one.
    pub focus_subs: HashMap<u32, Subscription>,
    /// This window's "nothing in me is focused any more" observer. Registered
    /// against one window by `cx.on_focus_lost(window, …)`, so a pop-out needs
    /// its own or focus leaving it is invisible.
    pub focus_lost: Option<Subscription>,

    // --- geometry (§5.2) ---------------------------------------------------
    /// Absolute layout bounds, **in this window's client space**, of every node
    /// this window painted in the last frame it drew.
    ///
    /// A rectangle from window B is meaningless in window A: they are different
    /// coordinate spaces that happen to share an origin name. Merging them is
    /// what made a hit test in one window return a node from the other.
    pub bounds: HashMap<u32, Rect>,
    /// The previous frame's [`Self::bounds`] — the element tree for frame N is
    /// BUILT before frame N's prepaint runs, so the only rectangle a builder
    /// can read is last frame's. `<lottie>` and `<video>` need it to rasterise
    /// into a box whose size CSS expresses relatively.
    pub prev_bounds: HashMap<u32, Rect>,

    // --- input (§5.2) ------------------------------------------------------
    /// The one `mousemove` THIS window is allowed to deliver this frame.
    /// Simultaneous moves in two windows used to drop one.
    pub pending_mouse_move: Option<PendingMouseMove>,
    /// Has a node already taken THIS mouse-down? gpui runs
    /// `on_any_mouse_down` on every listening box under the pointer, innermost
    /// first; the browser has one target. Without the claim an outer
    /// `@mousedown` (or a `document` pointerdown listener, which rides the
    /// root's bit) ran once as the inner box's ancestor and again as a target
    /// of its own. Reset in the capture phase of every mouse-down.
    pub down_claimed: bool,
    /// The focus THIS mouse-down will give, once its `mousedown` has been
    /// dispatched and checkpointed (`render::claim_press_focus`). A cancelled
    /// `mousedown` takes it back, as a browser's `preventDefault()` cancels the
    /// press's default focus.
    pub press_focus: Option<(u32, gpui::FocusHandle)>,
    /// The mouse-up waves not yet delivered to JS (`render::UpWave`). One
    /// physical mouse-up is a `mouseup`, a `click` and (right button) a
    /// `contextmenu`, each at its own innermost listening box (the same "one
    /// target" claim as `down_claimed`) — and gpui calls those boxes
    /// innermost-first, and a box's click listener before its own mouse-up
    /// listener. So the listeners only CLAIM, into the wave the build root
    /// opened in the capture phase, and one deferred flush delivers every wave
    /// in DOM order.
    pub up_waves: Vec<crate::render::UpWave>,
    /// The press in progress: its `mousedown` target and button, until its
    /// `mouseup` (`render::Press`).
    pub press: Option<crate::render::Press>,
    /// This window's activation observer — a press the window loses
    /// activation in the middle of is cancelled (`pointercancel`).
    pub activation: Option<Subscription>,
    /// Did a JS `keydown` handler cancel THE keystroke being dispatched? Reset
    /// in the root's capture phase (the first listener every keystroke
    /// meets), set by `bind_events`' keydown listener, and read by the root's
    /// clipboard listener (the last one), because a cancelled Ctrl+C has no
    /// `copy` event (`crate::clipboard`). `keyup` uses it the same way: a
    /// cancelled Menu-key `keyup` has no `contextmenu`.
    pub key_prevented: bool,
    /// The keyboard click the key event being dispatched owes, decided once
    /// every handler of that event has run (`render::resolve_key_click`).
    pub key_click: Option<crate::render::KeyClick>,
    /// The focused box whose Space `keydown` no handler cancelled: its keyup
    /// clicks it, unless the `keyup` is cancelled.
    pub space_armed: Option<u32>,

    // --- chrome (§5.2, §7.6) -----------------------------------------------
    /// Whether the frame this window is currently building painted any
    /// `window-drag: move` region. Drives the fallback titlebar's retirement,
    /// which is a property of one window's caption.
    pub any_drag_region: bool,
    /// Every control hitbox this window's frame emitted, `(node id, area)`.
    pub control_hitboxes: Vec<(u32, WindowControlArea)>,
    /// `titleBarStyle: "default"` — does the OS draw THIS window's caption?
    /// Two windows genuinely differ: the primary may be custom-chrome while a
    /// pop-out asks for a real `WS_CAPTION`, or the other way round.
    pub os_chrome: bool,

    // --- cascade (§6) ------------------------------------------------------
    /// The `@media` environment the subtree this window paints is cascaded
    /// against: its own viewport, its own scale factor, its own colour scheme.
    /// `None` until the host publishes one, in which case the primary's is
    /// used — a pop-out is cascaded correctly from its first *painted* frame,
    /// not from the frame it was requested on.
    pub env: Option<vn_css::Env>,

    // --- window resources (§5.1) -------------------------------------------
    /// Atlas tiles retired by a node THIS window painted, waiting for a frame
    /// that holds this window to hand them back.
    ///
    /// `Window::drop_image` is per-window: the sprite atlas belongs to the
    /// window. `App::drop_image(image, Some(window))` sweeps every *other*
    /// window's atlas too, so the parameter is exactly "the one window gpui
    /// cannot reach from the map because we are standing inside its update" —
    /// which is why the queue has to be keyed by the window that will drain it.
    pub pending_drops: Vec<Arc<RenderImage>>,
    /// Zero-copy `<video>` frames waiting to be blitted into THIS window's
    /// atlas. The decode happens once, in the pump; the blit has to happen in
    /// the window that paints the element, or that window's atlas has no tile
    /// and the element paints transparent (`crates/vn-video/CONTRACT.md` §6).
    pub pending_uploads: Vec<PendingUpload>,
}

/// The one `mousemove` a window is allowed to deliver per frame (browser event
/// coalescing: UAs align `mousemove`/`pointermove` to the frame clock and drop
/// the rest).
pub struct PendingMouseMove {
    pub id: u32,
    pub position: gpui::Point<gpui::Pixels>,
    pub button: f64,
    pub modifiers: gpui::Modifiers,
}

/// A decoded `<video>` frame that still lives in the renderer's GPU memory and
/// has to reach one window's sprite atlas before that window's next draw.
pub struct PendingUpload {
    pub node: u32,
    pub image: Arc<RenderImage>,
    pub gpu: vn_video::GpuFrame,
}

thread_local! {
    /// The map §5.6 asked for. ~15 single-slot thread-locals collapsed into it.
    static WINDOWS: RefCell<HashMap<WinKey, WindowState>> = RefCell::new(HashMap::new());
    /// `painted root node → the window painting it`. The reverse of
    /// [`WindowState::painted_root`], kept because the questions asked most
    /// often go that way: "which window owns this rectangle", "which window's
    /// atlas does this `<video>` belong to", "which `Env` does this subtree
    /// cascade against".
    static ROOTS: RefCell<HashMap<u32, WinKey>> = RefCell::new(HashMap::new());
    /// The window that paints `Tree::root`. Everything not under a popped-out
    /// node belongs to it, and `bounds_in(0, …)` means it.
    static PRIMARY: std::cell::Cell<WinKey> = const { std::cell::Cell::new(UNBOUND) };
}

/// The identity of the window whose stack frame we are standing in.
pub fn key(window: &Window) -> WinKey {
    window.window_handle().window_id().as_u64()
}

/// Declares which window paints `Tree::root`. Called once by the host, from
/// inside the primary window's own builder closure.
pub fn set_primary(k: WinKey) {
    PRIMARY.with(|c| c.set(k));
    of(k, |w| w.painted_root = 0);
}

/// The window that paints `Tree::root`.
pub fn primary() -> WinKey {
    PRIMARY.with(|c| c.get())
}

/// Is this the window that paints `Tree::root`?
pub fn is_primary(k: WinKey) -> bool {
    k == PRIMARY.with(|c| c.get())
}

/// The state of the window we are standing in, creating it on first touch.
///
/// `f` must not re-enter — the map is borrowed for its duration, exactly as
/// every `thread_local!` this replaced was. Anything that dispatches into
/// JavaScript reads or writes a field first and dispatches after.
pub fn here<R>(window: &Window, f: impl FnOnce(&mut WindowState) -> R) -> R {
    of(key(window), f)
}

/// [`here`], addressed by key rather than by the window itself.
pub fn of<R>(k: WinKey, f: impl FnOnce(&mut WindowState) -> R) -> R {
    WINDOWS.with(|m| f(m.borrow_mut().entry(k).or_default()))
}

/// Queues atlas tiles for `Window::drop_image` on the window whose atlas holds
/// them — and **drops them on the floor when that window is gone**.
///
/// [`of`] creates a [`WindowState`] on first touch, which is right for a window
/// that is about to paint and wrong for one that has closed: the queue would
/// sit in a resurrected state nothing will ever drain, and the `Arc`s would
/// outlive the process's need for them. [`forget`] already says what the right
/// answer is — a closed window's sprite atlas went with it, so dropping the
/// `Arc`s IS the cleanup, and handing them to another window's `drop_image`
/// would remove keys a live window may legitimately have re-inserted.
pub fn queue_drops(k: WinKey, images: impl IntoIterator<Item = std::sync::Arc<gpui::RenderImage>>) {
    let live = WINDOWS.with(|m| m.borrow().contains_key(&k));
    if !live {
        return;
    }
    of(k, |w| w.pending_drops.extend(images));
}

/// Reads one window's state without creating it. `None` = no such window has
/// ever painted.
pub fn peek<R>(k: WinKey, f: impl FnOnce(&WindowState) -> R) -> Option<R> {
    WINDOWS.with(|m| m.borrow().get(&k).map(f))
}

/// Every window, primary first. The order matters for [`crate::render::bounds_of`]:
/// "where is this node" prefers the primary window's answer, which is what a
/// single-window app (and every pre-multi-window caller) means.
pub fn keys() -> Vec<WinKey> {
    let primary = primary();
    let mut out: Vec<WinKey> = WINDOWS.with(|m| m.borrow().keys().copied().collect());
    out.sort_unstable_by_key(|&k| (k != primary, k));
    out
}

/// Runs `f` against every window's state, primary first. The escape hatch for
/// the two paths that have no `Window` at all — `gc_node_state` and
/// `flush_changes` — which run off a JS wave rather than off a frame.
pub fn each(mut f: impl FnMut(WinKey, &mut WindowState)) {
    for k in keys() {
        of(k, |w| f(k, w));
    }
}

/// Registers "this window paints this node as its root", called from the build
/// itself so the mapping cannot drift from what is actually on screen.
pub fn set_painted_root(k: WinKey, root: u32) {
    let previous = of(k, |w| std::mem::replace(&mut w.painted_root, root));
    if previous == root {
        return;
    }
    ROOTS.with(|m| {
        let mut m = m.borrow_mut();
        if previous != 0 {
            m.remove(&previous);
        }
        if root != 0 {
            m.insert(root, k);
        }
    });
    // **A subtree that has just changed hands has to be re-cascaded.** Its
    // `@media` conditions, `env(titlebar-area-*)`, `prefers-color-scheme` and
    // `resolution` are answered against the window that PAINTS it (§6), and
    // until this moment it was answering against a different one — or, on the
    // frame a pop-out opens, against the primary window, because nothing had
    // yet said otherwise.
    //
    // Marked here rather than left to the next mutation because there may not
    // BE one: a pop-out that is opened and then left alone would keep the main
    // window's answers forever. Scoped to the subtree, so the document does not
    // pay for a pop-out resizing.
    for id in [previous, root] {
        if id != 0 {
            crate::tree::with_tree(|t| t.mark_restyle_subtree(id));
        }
    }
}

/// The window painting `root` as its root node — `None` when nothing does.
pub fn window_of_root(root: u32) -> Option<WinKey> {
    if root == 0 {
        return Some(primary());
    }
    ROOTS.with(|m| m.borrow().get(&root).copied())
}

/// The painted root a window owns: `0` for the primary window, the popped-out
/// node for a pop-out.
pub fn root_of(k: WinKey) -> u32 {
    peek(k, |w| w.painted_root).unwrap_or(0)
}

/// **The plumbing §6.3 asked for.** Which window paints this node?
///
/// Walks `parent` to the nearest ancestor that is somebody's painted root; if
/// there is none, the node hangs under `Tree::root` and belongs to the primary
/// window. That walk is a handful of hops and is the same one `reaches_root`
/// already performs, with a different predicate.
///
/// This is the function that makes "the tile goes back to the window that made
/// it" and "this subtree cascades against its own viewport" mechanical rather
/// than bookkeeping.
pub fn window_of_node(node: u32) -> WinKey {
    if !any_secondary() {
        return primary();
    }
    crate::tree::read_tree(|t| window_of_node_in(t, node))
}

/// [`window_of_node`] against a tree the caller already holds.
///
/// The cascade needs this one: `Tree::restyle` runs inside `with_tree`'s
/// **mutable** borrow, so a nested `read_tree` there is not a slow path, it is
/// a panic. Anything that already has a `&Tree` calls this; only callers with
/// none (a devtools command, a host fn) go through [`window_of_node`].
pub fn window_of_node_in(t: &crate::tree::Tree, node: u32) -> WinKey {
    let popped: Option<WinKey> = ROOTS.with(|m| {
        let roots = m.borrow();
        if roots.is_empty() {
            return None;
        }
        let mut cursor = node;
        // A tree is a tree, but a corrupt parent chain must not hang a frame.
        for _ in 0..1024 {
            if let Some(&k) = roots.get(&cursor) {
                return Some(k);
            }
            match t.node(cursor) {
                Some(n) if n.parent != 0 => cursor = n.parent,
                _ => return None,
            }
        }
        None
    });
    popped.unwrap_or_else(primary)
}

/// Forgets a window that has closed, returning whatever it still held so the
/// caller can decide what to do with it.
///
/// The atlas tiles are the interesting part: a closed window's sprite atlas is
/// gone with it, so its `pending_drops` must NOT be handed to another window's
/// `drop_image` — that would remove keys a live window may legitimately have
/// re-inserted. Dropping the `Arc`s is the whole cleanup.
pub fn forget(k: WinKey) -> Option<WindowState> {
    let state = WINDOWS.with(|m| m.borrow_mut().remove(&k));
    if let Some(s) = state.as_ref() {
        if s.painted_root != 0 {
            ROOTS.with(|m| m.borrow_mut().remove(&s.painted_root));
        }
    }
    state
}

/// Is any window other than the primary one painting part of this tree?
///
/// The one-line gate that keeps every per-window path free for a single-window
/// app: `ROOTS` is empty until a pop-out actually paints, so the ancestor walk
/// in [`window_of_node`], the per-root `Env` lookup in the cascade and the
/// A-M05 advisory all short-circuit on an atomic-cheap `is_empty`.
pub fn any_secondary() -> bool {
    ROOTS.with(|m| !m.borrow().is_empty())
}

// ---------------------------------------------------------------------------
// the `@media` environment, per window (docs/MULTIWINDOW.md §6)
// ---------------------------------------------------------------------------

/// Publishes one window's `@media` environment. Returns `true` when it changed,
/// which obliges the caller to mark the tree dirty — a resize or a theme flip
/// in EITHER window can turn any `@media` rule on or off for the subtree that
/// window paints.
pub fn set_env(k: WinKey, env: vn_css::Env) -> bool {
    of(k, |w| {
        if w.env == Some(env) {
            return false;
        }
        w.env = Some(env);
        true
    })
}

/// One window's published environment, or `None` before it has painted.
pub fn env_of(k: WinKey) -> Option<vn_css::Env> {
    peek(k, |w| w.env).flatten()
}

/// **The cascade's question**: which `Env` does this subtree answer against?
///
/// `fallback` is the primary window's published environment, which is the
/// answer for everything that is not inside a pop-out — i.e. all of it, in a
/// single-window app, without touching a map.
///
/// A pop-out that has been requested but has not painted yet has no `Env` of
/// its own; it inherits the primary's for those frames rather than cascading
/// against zeros, which is the same rule `INITIAL_VIEWPORT_BASIS` follows.
pub fn env_for_node(t: &crate::tree::Tree, node: u32, fallback: &vn_css::Env) -> vn_css::Env {
    if !any_secondary() {
        return *fallback;
    }
    // `_in`, not the free function: the cascade holds the tree's MUTABLE borrow
    // for the whole pass, so re-entering `read_tree` here would panic with
    // "RefCell already mutably borrowed" the first frame a pop-out is open.
    let k = window_of_node_in(t, node);
    if is_primary(k) {
        return *fallback;
    }
    env_of(k).unwrap_or(*fallback)
}

/// The environment a node introduces **because it is a painted root**, or
/// `None` when it is not one.
///
/// [`env_for_node`] answers "which window is this node in" by walking upwards,
/// which is the right question at a walk ROOT and the wrong one per node — it
/// is O(depth) and the answer only ever changes at one kind of node. This is
/// that node: a cascade walking downwards switches environments here and
/// nowhere else, so a full-document pass costs one map lookup per child and
/// still gives every window's subtree its own `@media` answers.
///
/// The primary window is deliberately absent from `ROOTS` (its painted root is
/// `0`), so `t.root` keeps the fallback and a single-window app never gets
/// here at all — the caller gates on [`any_secondary`].
pub fn env_at_root(node: u32, fallback: &vn_css::Env) -> Option<vn_css::Env> {
    let k = ROOTS.with(|m| m.borrow().get(&node).copied())?;
    Some(env_of(k).unwrap_or(*fallback))
}

// ---------------------------------------------------------------------------
// focus — the one field with enough call sites to deserve helpers
// ---------------------------------------------------------------------------

/// The node that owns keyboard focus in this window.
pub fn focused_node(k: WinKey) -> u32 {
    peek(k, |w| w.focused_node).unwrap_or(0)
}

/// Sets the node that owns keyboard focus in this window.
pub fn set_focused_node(k: WinKey, node: u32) {
    of(k, |w| w.focused_node = node);
}

/// The focus handle a node would use, App-scoped and therefore **shared**.
///
/// Kept out of [`WindowState`] deliberately: `cx.focus_handle()` mints an id in
/// `App::focus_handles`, not in a window, so one handle per node is correct for
/// N windows — and since a node is painted by exactly one window, the handle is
/// only ever *in a focus path* in one of them anyway.
pub type SharedFocusHandles = HashMap<u32, FocusHandle>;

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        WINDOWS.with(|m| m.borrow_mut().clear());
        ROOTS.with(|m| m.borrow_mut().clear());
        PRIMARY.with(|c| c.set(UNBOUND));
    }

    #[test]
    fn the_primary_window_owns_everything_that_is_not_popped_out() {
        reset();
        set_primary(7);
        assert_eq!(primary(), 7);
        assert!(is_primary(7));
        // No pop-out has ever registered, so the ancestor walk short-circuits.
        assert_eq!(window_of_node(42), 7);
        assert_eq!(window_of_root(0), Some(7));
    }

    #[test]
    fn a_painted_root_maps_both_ways_and_is_forgotten_with_its_window() {
        reset();
        set_primary(1);
        set_painted_root(9, 55);
        assert_eq!(window_of_root(55), Some(9));
        assert_eq!(root_of(9), 55);
        forget(9);
        assert_eq!(window_of_root(55), None);
        assert!(peek(9, |_| ()).is_none());
    }

    #[test]
    fn re_rooting_a_window_drops_the_old_reverse_entry() {
        reset();
        set_primary(1);
        set_painted_root(9, 55);
        set_painted_root(9, 56);
        assert_eq!(window_of_root(55), None);
        assert_eq!(window_of_root(56), Some(9));
    }

    #[test]
    fn keys_put_the_primary_first_so_bounds_of_prefers_it() {
        reset();
        set_primary(4);
        of(9, |_| ());
        of(2, |_| ());
        assert_eq!(keys(), vec![4, 2, 9]);
    }
}
