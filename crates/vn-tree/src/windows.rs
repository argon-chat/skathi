//! Per-window state: one process, one `JsRuntime` and one `Tree`, but N gpui windows each painting one
//! node of it. State two windows disagree about (focus, painted bounds, drag regions, `@media`
//! environment, ...) lives in one [`WindowState`] per `gpui::WindowId`, behind one accessor; as
//! single-slot thread-locals it would alias across windows (window B focusing an input blurring A's).
//! "Here" is passed, never guessed.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::idmap::IdMap;
use std::sync::Arc;

use gpui::{FocusHandle, RenderImage, Subscription, Window, WindowControlArea};

/// A gpui `WindowId`, flattened to the `u64` the devtools protocol and the pop-out registry already
/// speak. `0` is not a legal gpui window id, so it is free to mean "no window yet" ([`UNBOUND`]).
pub type WinKey = u64;

/// The key of a window that does not exist — used before `set_primary` has run (tests, and the
/// frames between `App::run` and the first `open_window`).
pub const UNBOUND: WinKey = 0;

/// A rectangle in ONE window's client space, CSS px: `(x, y, width, height)`.
pub type Rect = (f32, f32, f32, f32);

/// One mouse-down's focus, its `mousedown`'s default action (`render::claim_press_focus`,
/// `render::apply_press_focus`).
#[derive(Default)]
pub struct PressFocus {
    /// The step that applies it is scheduled for the end of this dispatch.
    pub pending: bool,
    /// The nodes on the target's path that HTML lets take focus, nearest first, with their handles.
    pub claims: Vec<(u32, gpui::FocusHandle)>,
    pub cancelled: bool,
    /// The press reached the root's bubble listener. A native widget that took it (a scrollbar's
    /// thumb or track stops its propagation) moves no focus unless something focusable on the path
    /// claimed it first.
    pub reached: bool,
}

/// Everything one OS window knows that another one disagrees about.
#[derive(Default)]
pub struct WindowState {
    /// The node this window paints as its root: `0` for the primary window (which paints
    /// `Tree::root` and everything under it), the popped-out node for a pop-out.
    pub painted_root: u32,

    /// The node that owns keyboard focus IN THIS WINDOW (0 = none).
    pub focused_node: u32,
    /// Queued by `gc_node_state` (which has no `Window`), applied by the next frame tick that does.
    pub pending_blur: u32,
    /// gpui `on_focus` subscriptions, one per focusable node per window.
    pub focus_subs: HashMap<u32, Subscription>,
    /// This window's "nothing in me is focused any more" observer. Registered against one window by
    /// `cx.on_focus_lost(window, …)`, so a pop-out needs its own or focus leaving it is invisible.
    pub focus_lost: Option<Subscription>,

    /// Absolute layout bounds, in this window's client space, of every node this window painted in
    /// the last frame it drew.
    pub bounds: IdMap<Rect>,
    /// The previous frame's [`Self::bounds`] — the element tree for frame N is BUILT before frame
    /// N's prepaint runs, so the only rectangle a builder can read is last frame's. `<lottie>` and
    /// `<video>` need it to rasterise into a box whose size CSS expresses relatively.
    pub prev_bounds: IdMap<Rect>,
    /// Where CSS `transform` put a node, for the nodes it moved: the bounding box of its border box
    /// under its own transform and its ancestors'.
    pub transformed: IdMap<Rect>,
    /// The previous frame's [`Self::transformed`], rotated with [`Self::prev_bounds`].
    pub prev_transformed: IdMap<Rect>,

    /// The one `mousemove` THIS window is allowed to deliver this frame.
    pub pending_mouse_move: Option<PendingMouseMove>,
    /// Explicit DOM pointer capture: events stay with this node outside its hitbox.
    pub pointer_capture: Option<u32>,
    /// Has a node already taken THIS mouse-down? gpui runs `on_any_mouse_down` on every listening
    /// box under the pointer, innermost first; the browser has one target. Without the claim an
    /// outer `@mousedown` (or a `document` pointerdown listener, which rides the root's bit) ran
    /// once as the inner box's ancestor and again as a target of its own.
    pub down_claimed: bool,
    /// The focus THIS mouse-down will give, once its `mousedown` has been dispatched and
    /// checkpointed (`render::apply_press_focus`).
    pub press_focus: PressFocus,
    /// The mouse-up waves not yet delivered to JS (`render::UpWave`). So the listeners only CLAIM,
    /// into the wave the build root opened in the capture phase, and one deferred flush delivers
    /// every wave in DOM order.
    pub up_waves: Vec<crate::render::UpWave>,
    /// The press in progress: its DOM target and button, including click-only nodes. Kept outside
    /// gpui element state so cached/direct render switches between down and up cannot lose it
    /// (`render::Press`).
    pub press: Option<crate::render::Press>,
    /// The element the press, release or wheel being dispatched is for — the topmost one under the
    /// pointer (`crate::pointer`), resolved before any listener ran.
    pub hit: Option<u32>,
    /// [`Self::hit`] of the last press: with the release's, where its `click` goes
    /// (`pointer::click_target`).
    pub down_hit: Option<u32>,
    /// What is hovered in this window, resolved once a frame (`pointer::resolve_hover`).
    pub hover: crate::pointer::Hover,
    /// The target of the mouse press in progress in this window, from the press to the release,
    /// whatever listens: it and its ancestors are what `:active` matches in a script's query
    /// (`crate::live_state`).
    pub active: Option<u32>,
    /// What `:focus-visible` reads in a script's query, as the painter reads
    /// `Window::last_input_was_keyboard`.
    pub keyboard_input: bool,
    /// This window's activation observer — a press the window loses activation in the middle of is
    /// cancelled (`pointercancel`).
    pub activation: Option<Subscription>,
    /// Did a JS `keydown` handler cancel THE keystroke being dispatched?
    pub key_prevented: bool,
    /// The key event (gpui's `Window::vn_key_event_seq`) whose ONE DOM `keydown` / `keyup` has been
    /// dispatched (`render::claim_key_event`).
    pub key_event_claimed: u64,
    /// The keyboard click the key event being dispatched owes, decided once every handler of that
    /// event has run (`render::resolve_key_click`).
    pub key_click: Option<crate::render::KeyClick>,
    /// The focused box whose Space `keydown` no handler cancelled: its keyup clicks it, unless the
    /// `keyup` is cancelled.
    pub space_armed: Option<u32>,

    /// Whether the frame this window is currently building painted any `window-drag: move` region.
    pub any_drag_region: bool,
    /// Every control hitbox this window's frame emitted, `(node id, area)`.
    pub control_hitboxes: Vec<(u32, WindowControlArea)>,
    /// `titleBarStyle: "default"` — does the OS draw THIS window's caption? Two windows genuinely
    /// differ: the primary may be custom-chrome while a pop-out asks for a real `WS_CAPTION`, or
    /// the other way round.
    pub os_chrome: bool,

    /// Do the images this window shows keep their full decode while it shows them? A media viewer
    /// zooms to 1:1 at any moment, so the residency sweep must not have a smaller copy stand in for
    /// what it shows (`crate::img`, "residency").
    pub full_res_images: bool,

    /// The `@media` environment the subtree this window paints is cascaded against: its own
    /// viewport, its own scale factor, its own colour scheme.
    pub env: Option<vn_css::Env>,

    /// Atlas tiles retired by a node THIS window painted, waiting for a frame that holds this
    /// window to hand them back.
    pub pending_drops: Vec<Arc<RenderImage>>,
    /// Zero-copy `<video>` frames waiting to be blitted into THIS window's atlas.
    pub pending_uploads: Vec<PendingUpload>,
}

/// The one `mousemove` a window is allowed to deliver per frame (browser event coalescing: UAs
/// align `mousemove`/`pointermove` to the frame clock and drop the rest).
pub struct PendingMouseMove {
    pub id: u32,
    pub position: gpui::Point<gpui::Pixels>,
    pub button: f64,
    pub modifiers: gpui::Modifiers,
}

/// A decoded `<video>` frame that still lives in the renderer's GPU memory and has to reach one
/// window's sprite atlas before that window's next draw. The texture is HELD, not borrowed: the
/// window may not draw for a while, and the element that made the frame can be gone by then.
pub struct PendingUpload {
    pub node: u32,
    pub image: Arc<RenderImage>,
    pub gpu: vn_video::HeldGpuFrame,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<WinKey, WindowState>> = RefCell::new(HashMap::new());
    /// `painted root node → the window painting it`.
    static ROOTS: RefCell<HashMap<u32, WinKey>> = RefCell::new(HashMap::new());
    /// The window that paints `Tree::root`.
    static PRIMARY: std::cell::Cell<WinKey> = const { std::cell::Cell::new(UNBOUND) };
}

/// The identity of the window whose stack frame we are standing in.
pub fn key(window: &Window) -> WinKey {
    window.window_handle().window_id().as_u64()
}

/// Declares which window paints `Tree::root`.
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

/// The state of the window we are standing in, creating it on first touch. `f` must not re-enter: the
/// map is borrowed for its duration, so read or write a field first and dispatch to JS after.
pub fn here<R>(window: &Window, f: impl FnOnce(&mut WindowState) -> R) -> R {
    of(key(window), f)
}

/// [`here`], addressed by key rather than by the window itself.
pub fn of<R>(k: WinKey, f: impl FnOnce(&mut WindowState) -> R) -> R {
    WINDOWS.with(|m| f(m.borrow_mut().entry(k).or_default()))
}

/// Queues atlas tiles for `Window::drop_image` on the window whose atlas holds them — and drops
/// them on the floor when that window is gone.
pub fn queue_drops(k: WinKey, images: impl IntoIterator<Item = std::sync::Arc<gpui::RenderImage>>) {
    let live = WINDOWS.with(|m| m.borrow().contains_key(&k));
    if !live {
        return;
    }
    of(k, |w| w.pending_drops.extend(images));
}

/// Reads one window's state without creating it.
pub fn peek<R>(k: WinKey, f: impl FnOnce(&WindowState) -> R) -> Option<R> {
    WINDOWS.with(|m| m.borrow().get(&k).map(f))
}

/// Every window, primary first.
pub fn keys() -> Vec<WinKey> {
    let primary = primary();
    let mut out: Vec<WinKey> = WINDOWS.with(|m| m.borrow().keys().copied().collect());
    out.sort_unstable_by_key(|&k| (k != primary, k));
    out
}

/// Runs `f` against every window's state, primary first.
pub fn each(mut f: impl FnMut(WinKey, &mut WindowState)) {
    for k in keys() {
        of(k, |w| f(k, w));
    }
}

/// Registers "this window paints this node as its root", called from the build itself so the
/// mapping cannot drift from what is actually on screen.
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
    // Marked here rather than left to the next mutation because there may not BE one: a pop-out
    // that is opened and then left alone would keep the main window's answers forever.
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

/// The painted root a window owns: `0` for the primary window, the popped-out node for a pop-out.
pub fn root_of(k: WinKey) -> u32 {
    peek(k, |w| w.painted_root).unwrap_or(0)
}

/// Which window paints this node?
pub fn window_of_node(node: u32) -> WinKey {
    if !any_secondary() {
        return primary();
    }
    crate::tree::read_tree(|t| window_of_node_in(t, node))
}

/// [`window_of_node`] against a tree the caller already holds. The cascade needs this one:
/// `Tree::restyle` runs inside `with_tree`'s mutable borrow, so a nested `read_tree` would panic.
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

/// Forgets a window that has closed, returning whatever it still held so the caller can decide what
/// to do with it.
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
pub fn any_secondary() -> bool {
    ROOTS.with(|m| !m.borrow().is_empty())
}


/// Publishes one window's `@media` environment.
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

/// The cascade's question: which `Env` does this subtree answer against?
pub fn env_for_node(t: &crate::tree::Tree, node: u32, fallback: &vn_css::Env) -> vn_css::Env {
    if !any_secondary() {
        return *fallback;
    }
    // `_in`, not the free function: the cascade holds the tree's MUTABLE borrow for the whole pass,
    // so re-entering `read_tree` here would panic with "RefCell already mutably borrowed" the first
    // frame a pop-out is open.
    let k = window_of_node_in(t, node);
    if is_primary(k) {
        return *fallback;
    }
    env_of(k).unwrap_or(*fallback)
}

/// The environment a node introduces because it is a painted root, or `None` when it is not one.
pub fn env_at_root(node: u32, fallback: &vn_css::Env) -> Option<vn_css::Env> {
    let k = ROOTS.with(|m| m.borrow().get(&node).copied())?;
    Some(env_of(k).unwrap_or(*fallback))
}


/// The node that owns keyboard focus in this window.
pub fn focused_node(k: WinKey) -> u32 {
    peek(k, |w| w.focused_node).unwrap_or(0)
}

/// Sets the node that owns keyboard focus in this window.
pub fn set_focused_node(k: WinKey, node: u32) {
    of(k, |w| w.focused_node = node);
}

/// Marks a window as one whose images keep their full decode while shown
/// ([`WindowState::full_res_images`]).
pub fn set_full_res_images(k: WinKey, on: bool) {
    of(k, |w| w.full_res_images = on);
}

/// Is any window keeping its images' full decodes?
pub(crate) fn any_full_res() -> bool {
    WINDOWS.with(|m| m.borrow().values().any(|w| w.full_res_images))
}

/// The focus handle a node would use, App-scoped and therefore shared.
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
