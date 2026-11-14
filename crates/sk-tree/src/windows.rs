//! Per-window state: one process, one `JsRuntime` and one `Tree`, but N gpui windows each painting one
//! node of it. State the windows disagree about (focus, bounds, drag regions, `@media`) lives in one
//! [`WindowState`] per `gpui::WindowId`; single-slot thread-locals would alias across windows.
//! "Here" is passed, never guessed.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::idmap::IdMap;
use std::sync::Arc;

use gpui::{FocusHandle, RenderImage, Subscription, Window, WindowControlArea};

/// A gpui `WindowId` flattened to the `u64` the devtools protocol speaks; `0` is not a legal id, so
/// it means "no window yet" ([`UNBOUND`]).
pub type WinKey = u64;

pub const UNBOUND: WinKey = 0;

/// A rectangle in ONE window's client space, CSS px: `(x, y, width, height)`.
pub type Rect = (f32, f32, f32, f32);

#[derive(Default)]
pub struct PressFocus {
    pub pending: bool,
    pub claims: Vec<(u32, gpui::FocusHandle)>,
    pub cancelled: bool,
    /// The press reached the root's bubble listener. A native widget that took it (a scrollbar stops
    /// propagation) moves no focus unless something focusable on the path claimed it first.
    pub reached: bool,
}

#[derive(Default)]
pub struct WindowState {
    /// The node this window paints as its root: `0` for the primary window, the popped-out node for a pop-out.
    pub painted_root: u32,

    pub focused_node: u32,
    /// Queued by `gc_node_state` (which has no `Window`), applied by the next frame tick that does.
    pub pending_blur: u32,
    pub focus_subs: HashMap<u32, Subscription>,
    /// Registered per window (`cx.on_focus_lost`), so a pop-out needs its own or focus leaving it is invisible.
    pub focus_lost: Option<Subscription>,

    pub bounds: IdMap<Rect>,
    /// The previous frame's [`Self::bounds`]: the element tree for frame N is built before N's
    /// prepaint, so a builder can only read last frame's (`<lottie>`, `<video>` size relatively).
    pub prev_bounds: IdMap<Rect>,
    pub transformed: IdMap<Rect>,
    pub prev_transformed: IdMap<Rect>,

    pub pending_mouse_move: Option<PendingMouseMove>,
    pub pointer_capture: Option<u32>,
    /// Has a node already taken THIS mouse-down? gpui runs `on_any_mouse_down` on every listening box
    /// under the pointer; the browser has one target, so without the claim an outer listener ran twice.
    pub down_claimed: bool,
    pub press_focus: PressFocus,
    /// Mouse-up waves not yet delivered (`render::UpWave`): listeners only claim, one deferred flush
    /// delivers in DOM order.
    pub up_waves: Vec<crate::render::UpWave>,
    /// The press in progress (`render::Press`), kept outside gpui element state so cached/direct
    /// render switches cannot lose it.
    pub press: Option<crate::render::Press>,
    pub hit: Option<u32>,
    pub down_hit: Option<u32>,
    pub hover: crate::pointer::Hover,
    pub active: Option<u32>,
    pub keyboard_input: bool,
    pub activation: Option<Subscription>,
    pub key_prevented: bool,
    pub key_event_claimed: u64,
    pub key_click: Option<crate::render::KeyClick>,
    pub space_armed: Option<u32>,

    pub any_drag_region: bool,
    pub control_hitboxes: Vec<(u32, WindowControlArea)>,
    pub os_chrome: bool,

    pub full_res_images: bool,

    pub env: Option<sk_css::Env>,

    pub pending_drops: Vec<Arc<RenderImage>>,
    pub pending_uploads: Vec<PendingUpload>,
}

pub struct PendingMouseMove {
    pub id: u32,
    pub position: gpui::Point<gpui::Pixels>,
    pub button: f64,
    pub modifiers: gpui::Modifiers,
}

/// A decoded `<video>` frame still in GPU memory that must reach one window's atlas before its next
/// draw. The texture is HELD: the window may not draw for a while.
pub struct PendingUpload {
    pub node: u32,
    pub image: Arc<RenderImage>,
    pub gpu: sk_video::HeldGpuFrame,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<WinKey, WindowState>> = RefCell::new(HashMap::new());
    static ROOTS: RefCell<HashMap<u32, WinKey>> = RefCell::new(HashMap::new());
    static PRIMARY: std::cell::Cell<WinKey> = const { std::cell::Cell::new(UNBOUND) };
}

pub fn key(window: &Window) -> WinKey {
    window.window_handle().window_id().as_u64()
}

pub fn set_primary(k: WinKey) {
    PRIMARY.with(|c| c.set(k));
    of(k, |w| w.painted_root = 0);
}

pub fn primary() -> WinKey {
    PRIMARY.with(|c| c.get())
}

pub fn is_primary(k: WinKey) -> bool {
    k == PRIMARY.with(|c| c.get())
}

/// The state of the window we are standing in, creating it on first touch. `f` must not re-enter: the
/// map is borrowed for its duration, so read or write a field first and dispatch to JS after.
pub fn here<R>(window: &Window, f: impl FnOnce(&mut WindowState) -> R) -> R {
    of(key(window), f)
}

pub fn of<R>(k: WinKey, f: impl FnOnce(&mut WindowState) -> R) -> R {
    WINDOWS.with(|m| f(m.borrow_mut().entry(k).or_default()))
}

pub fn queue_drops(k: WinKey, images: impl IntoIterator<Item = std::sync::Arc<gpui::RenderImage>>) {
    let live = WINDOWS.with(|m| m.borrow().contains_key(&k));
    if !live {
        return;
    }
    of(k, |w| w.pending_drops.extend(images));
}

pub fn peek<R>(k: WinKey, f: impl FnOnce(&WindowState) -> R) -> Option<R> {
    WINDOWS.with(|m| m.borrow().get(&k).map(f))
}

pub fn keys() -> Vec<WinKey> {
    let primary = primary();
    let mut out: Vec<WinKey> = WINDOWS.with(|m| m.borrow().keys().copied().collect());
    out.sort_unstable_by_key(|&k| (k != primary, k));
    out
}

pub fn each(mut f: impl FnMut(WinKey, &mut WindowState)) {
    for k in keys() {
        of(k, |w| f(k, w));
    }
}

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

pub fn window_of_root(root: u32) -> Option<WinKey> {
    if root == 0 {
        return Some(primary());
    }
    ROOTS.with(|m| m.borrow().get(&root).copied())
}

pub fn root_of(k: WinKey) -> u32 {
    peek(k, |w| w.painted_root).unwrap_or(0)
}

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

pub fn forget(k: WinKey) -> Option<WindowState> {
    let state = WINDOWS.with(|m| m.borrow_mut().remove(&k));
    if let Some(s) = state.as_ref() {
        if s.painted_root != 0 {
            ROOTS.with(|m| m.borrow_mut().remove(&s.painted_root));
        }
    }
    state
}

pub fn any_secondary() -> bool {
    ROOTS.with(|m| !m.borrow().is_empty())
}

pub fn set_env(k: WinKey, env: sk_css::Env) -> bool {
    of(k, |w| {
        if w.env == Some(env) {
            return false;
        }
        w.env = Some(env);
        true
    })
}

pub fn env_of(k: WinKey) -> Option<sk_css::Env> {
    peek(k, |w| w.env).flatten()
}

pub fn env_for_node(t: &crate::tree::Tree, node: u32, fallback: &sk_css::Env) -> sk_css::Env {
    if !any_secondary() {
        return *fallback;
    }
    // `_in`, not the free function: the cascade holds the tree's mutable borrow, so `read_tree` here
    // would panic.
    let k = window_of_node_in(t, node);
    if is_primary(k) {
        return *fallback;
    }
    env_of(k).unwrap_or(*fallback)
}

pub fn env_at_root(node: u32, fallback: &sk_css::Env) -> Option<sk_css::Env> {
    let k = ROOTS.with(|m| m.borrow().get(&node).copied())?;
    Some(env_of(k).unwrap_or(*fallback))
}

pub fn focused_node(k: WinKey) -> u32 {
    peek(k, |w| w.focused_node).unwrap_or(0)
}

pub fn set_focused_node(k: WinKey, node: u32) {
    of(k, |w| w.focused_node = node);
}

pub fn set_full_res_images(k: WinKey, on: bool) {
    of(k, |w| w.full_res_images = on);
}

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
