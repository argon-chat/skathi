//! **SPIKE** — one Vue subtree, painted by a second OS window
//! (`docs/MULTIWINDOW.md`).
//!
//! The whole idea in one paragraph: there is exactly ONE `JsRuntime`, ONE Vue
//! app and ONE retained tree. `__vn_window_open(nodeId, options)` marks a node
//! *popped out* — it stops generating a box in the primary window's element
//! tree (`vn_tree::set_popped_out`, the `<KeepAlive>`-cache half of the
//! detachment rule) — and opens a second gpui window whose root view lowers
//! **that same node** through `vn_tree::build_subtree`. The node keeps its id,
//! its listeners, its computed style and its reactive bindings, so a click in
//! window B runs the same JS handler, mutates the same `ref`, and both windows
//! repaint from the same wave.
//!
//! What is deliberately NOT here — every item has a design answer in
//! `docs/MULTIWINDOW.md` §7, and none of them is load-bearing for the claim the
//! spike makes:
//!
//! * the `<Teleport to="vn:window">` sugar (that is the TS runtime's job; §3
//!   writes its contract);
//! * a per-window `@media` env — the second window shares the primary's
//!   viewport for cascade purposes (§5.2);
//! * per-window focus, IME, devtools addressing, drag regions (§7).
//!
//! The second window gets **OS chrome** on purpose: `appears_transparent:
//! false` is F-03's real `WS_CAPTION`, so it is movable, resizable and closable
//! with no drag-region plumbing at all — and `CONTROL_HITBOXES` (which describes
//! the PRIMARY window's chrome) stays untouched.

use std::cell::RefCell;

use gpui::{
    App, Bounds, Context, Render, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowId, WindowOptions, point, prelude::*, px, size,
};
use vn_runtime::redjs::api::Value;
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{JsRuntime, host_fn, with_app};

use crate::json::{self, Json};

const UNDEF: u64 = 0xa;

/// The root view of a popped-out window. It holds nothing but the node id: the
/// state lives in the one retained tree, exactly as the primary shell's does.
pub struct PopoutShell {
    node: u32,
}

impl Render for PopoutShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = window.window_handle().window_id().as_u64();
        gpui::div()
            .relative()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont")
            // The SAME node the primary window stopped painting.
            .child(vn_tree::build_subtree(self.node, window, cx))
            // …and the inspector's outline, when it is THIS window's node
            // that is selected. Shell furniture here exactly as it is in the
            // primary window (docs/DEVTOOLS-UI.md §5): an inspector must not
            // appear in the tree it is inspecting, and a pop-out's tree is
            // the same tree.
            .children(popout_inspector_overlay(id, self.node))
    }
}

/// The devtools highlight overlay for a pop-out window. Always `None` in a
/// build with no devtools in it.
fn popout_inspector_overlay(window: u64, root: u32) -> Option<gpui::Div> {
    #[cfg(vn_devtools)]
    {
        crate::devtools::popout_highlight_overlay(window, root)
    }
    #[cfg(not(vn_devtools))]
    {
        let _ = (window, root);
        None
    }
}

struct Popout {
    node: u32,
    id: WindowId,
    /// What the window's caption says — the only human-readable name a window
    /// switcher can show for it. Read back through [`list`], i.e. by the
    /// devtools' `windows` command and nothing else.
    #[cfg_attr(not(vn_devtools), allow(dead_code))]
    title: String,
    handle: WindowHandle<PopoutShell>,
    /// A GC-protected JS function called with `(nodeId)` when the window closes
    /// — the spike's stand-in for the `close` event of §3's contract.
    on_close: u64,
}

thread_local! {
    static POPOUTS: RefCell<Vec<Popout>> = const { RefCell::new(Vec::new()) };
    /// One `App::on_window_closed` subscription for all of them: the callback
    /// is handed the `WindowId`, which is all the addressing we need.
    static CLOSED_SUB: RefCell<Option<Subscription>> = const { RefCell::new(None) };
}

/// Every popped-out window is marked dirty whenever the tree moves. That is
/// what makes "click a button living in window B → BOTH windows' dependent text
/// updates in the same frame group" true: the JS wave already ran
/// (synchronously, inside the click listener), and this is the repaint request
/// that shows its result on the other side.
///
/// **`Window::refresh()` through the window handle, deliberately not
/// `Entity::update` + `cx.notify()`.** This is called from inside an input
/// listener, and a listener IS an `Entity<RootView>::update` — so notifying the
/// root view of the window the event landed in would re-enter that borrow and
/// panic with "cannot update … while it is already being updated". Addressing
/// the *window* instead of its *view* borrows nothing: `update_window` on a
/// window that is already being updated simply fails, and that window was going
/// to be refreshed by the listener anyway.
pub fn notify_all(cx: &mut App) {
    let ids: Vec<WindowId> = POPOUTS.with(|p| p.borrow().iter().map(|w| w.id).collect());
    for id in ids {
        let _ = gpui::AnyWindowHandle::from(WindowHandle::<PopoutShell>::new(id))
            .update(cx, |_, window, _| window.refresh());
    }
}

struct Options {
    title: String,
    x: Option<f32>,
    y: Option<f32>,
    width: f32,
    height: f32,
}

fn parse_options(raw: &str) -> Options {
    let j = json::parse(raw);
    let get = |k: &str| j.as_ref().and_then(|j| j.get(k)).and_then(|v| v.as_f64());
    Options {
        title: j
            .as_ref()
            .and_then(|j| j.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("vue-native")
            .to_string(),
        x: get("x").map(|n| n as f32),
        y: get("y").map(|n| n as f32),
        width: get("width").unwrap_or(420.0) as f32,
        height: get("height").unwrap_or(320.0) as f32,
    }
}

// `__vn_window_open(nodeId, optionsJson, onClose?)` — the spike's whole host
// surface. Returns the new window's id as a number (0 = refused).
//
// The open itself is **deferred**. `App::open_window` builds the root view and
// then draws the window once before returning (`gpui/src/app.rs:1279`), and
// this host fn is called from inside the primary window's own update, with
// `Phase::Commit` published against IT. `cx.defer` runs the open at the end of
// the current effect cycle, i.e. with every entity returned to the app and no
// window published — which is also why the JS caller gets its window id back
// on a later frame rather than synchronously. §3's contract says so.
host_fn!(fn vn_window_open(g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let on_close = args.arg(2);
    if node == 0 || !vn_tree::with_tree(|t| t.node(node).is_some()) {
        crate::window::advise(&format!(
            "__vn_window_open: node {node} is not in the tree — nothing was opened"
        ));
        return UNDEF;
    }
    if popout_of_node(node).is_some() {
        crate::window::advise(&format!(
            "__vn_window_open: node {node} is already painted by another window"
        ));
        return UNDEF;
    }
    let on_close = if on_close.as_number().is_some() || on_close.0 == UNDEF {
        0
    } else {
        unsafe { vn_runtime::redjs::ffi::redjs_protect(g, on_close.0) };
        on_close.0
    };
    let opts = parse_options(&raw);
    // Stop painting it HERE, not in the deferred open: the primary window's
    // very next draw must already be without it, or the subtree would exist in
    // two element trees for one frame (two sets of `ElementId`s over one node —
    // gpui's scroll/hitbox state is keyed by that).
    vn_tree::set_popped_out(node, true);
    with_app(|cx| {
        cx.defer(move |cx| open_now(node, opts, on_close, cx));
    });
    UNDEF
});

// `__vn_window_close(nodeId)` — closes the window painting that node. The
// subtree returns to the primary window through the close observer, so this
// and a user clicking the X take exactly the same path.
host_fn!(fn vn_window_close_popout(_g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let Some(handle) = popout_of_node(node).map(|i| POPOUTS.with(|p| p.borrow()[i].handle)) else {
        return UNDEF;
    };
    with_app(|cx| {
        cx.defer(move |cx| {
            // Untyped, for the reason spelled out in `vn_window_click` below:
            // never take the root entity's borrow when the body can re-enter it.
            let _ = gpui::AnyWindowHandle::from(handle)
                .update(cx, |_, window, _| window.remove_window());
        });
    });
    UNDEF
});

// `__vn_window_click(nodeId, x, y)` — synthesize a REAL platform mouse click
// into the window painting `nodeId`, at that window's client coordinates.
//
// This is the spike's proof surface, and it is deliberately not a shortcut:
// it feeds `gpui::PlatformInput::{MouseMove, MouseDown, MouseUp}` to the
// pop-out's own `Window::dispatch_event` — the *same* entry point the Win32
// message loop uses. So it runs window B's real hit test, finds the real
// element `build_subtree` lowered there, and fires the real `on_click`
// listener `cx.listener` registered against `PopoutShell`. Nothing about the
// dispatch is faked except where the bytes came from.
//
// It exists because the devtools `click` command drives *the published*
// window (`crates/vn-app/src/devtools/mod.rs:752`, `with_commit`), which is
// always the primary one — and because a spec that runs while somebody is
// using the machine may not move the real cursor.
//
// Returns `true` when a window was found and driven.
host_fn!(fn vn_window_click(_g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let x = args.arg(1).as_number().unwrap_or(0.0) as f32;
    let y = args.arg(2).as_number().unwrap_or(0.0) as f32;
    let Some(handle) = popout_of_node(node).map(|i| POPOUTS.with(|p| p.borrow()[i].handle)) else {
        return Value::boolean(false).0;
    };
    let position = gpui::Point::new(px(x), px(y));
    let done = with_app(|cx| {
        // `AnyWindowHandle::update`, NOT `WindowHandle::<PopoutShell>::update`.
        // The typed one wraps the body in `Entity<PopoutShell>::update`, and the
        // click listener `build_subtree` registered is a `cx.listener`, i.e.
        // *another* `Entity<PopoutShell>::update` — so the typed form panics
        // with "cannot update PopoutShell while it is already being updated"
        // the instant the dispatch actually hits a handler. The untyped one
        // hands back the root as an `AnyView` and borrows nothing.
        gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| {
            // A move first: gpui recomputes `mouse_hit_test` from the window's
            // own `mouse_position`, so a press with no preceding move is tested
            // against wherever that window's pointer was last (which, for a
            // window that has never seen the mouse, is the origin).
            window.dispatch_event(
                gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: gpui::Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers: gpui::Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers: gpui::Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
            window.refresh();
        })
        .is_ok()
    });
    Value::boolean(done.unwrap_or(false)).0
});

// `__vn_window_popouts()` — `[{node, windowId}]`, for tests and devtools.
host_fn!(fn vn_window_popouts(_g, _args) {
    let json = POPOUTS.with(|p| {
        Json::arr(
            p.borrow()
                .iter()
                .map(|w| {
                    Json::obj(vec![
                        ("node", Json::n(w.node as f64)),
                        ("windowId", Json::n(w.id.as_u64() as f64)),
                    ])
                })
                .collect(),
        )
        .to_json()
    });
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

fn popout_of_node(node: u32) -> Option<usize> {
    POPOUTS.with(|p| p.borrow().iter().position(|w| w.node == node))
}

// ---------------------------------------------------------------------------
// addressing a pop-out from the devtools (docs/DEVTOOLS.md, the `window` param)
// ---------------------------------------------------------------------------

/// `(window id, the node this window paints, its title)` for every pop-out.
///
/// The devtools' `windows` command puts the primary window at the head of this
/// list; here there is nothing but the pop-outs, because the primary window's
/// identity is `main.rs`'s to report.
///
/// That command is also the only caller, so a `--features no-devtools` build
/// has none. Kept rather than `cfg`'d: the pop-out registry is the spike's own
/// state (docs/MULTIWINDOW.md) and reading it back is part of what the registry
/// IS, whoever happens to be asking this month.
#[cfg_attr(not(vn_devtools), allow(dead_code))]
pub fn list() -> Vec<(u64, u32, String)> {
    POPOUTS.with(|p| {
        p.borrow().iter().map(|w| (w.id.as_u64(), w.node, w.title.clone())).collect()
    })
}

/// Runs `f` against one pop-out window, addressed by its gpui window id.
///
/// `AnyWindowHandle::update`, NOT the typed handle's — for the reason spelled
/// out at `vn_window_click`: the typed form wraps the body in
/// `Entity<PopoutShell>::update`, and a devtools command that dispatches an
/// event would re-enter that borrow the instant it reached a listener.
///
/// Devtools-only, like [`list`] above and for the same reason.
#[cfg_attr(not(vn_devtools), allow(dead_code))]
pub fn with_window<R>(
    id: u64,
    cx: &mut App,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> Option<R> {
    let handle = POPOUTS.with(|p| p.borrow().iter().find(|w| w.id.as_u64() == id).map(|w| w.handle))?;
    gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| f(window, cx)).ok()
}

fn open_now(node: u32, opts: Options, on_close: u64, cx: &mut App) {
    // The node could have been unmounted between the call and this defer — Vue
    // is free to do that, and opening a window onto a dead id would paint an
    // empty rectangle nobody can explain.
    if !vn_tree::with_tree(|t| t.node(node).is_some()) {
        vn_tree::set_popped_out(node, false);
        return;
    }
    let bounds = match (opts.x, opts.y) {
        (Some(x), Some(y)) => Bounds {
            origin: point(px(x), px(y)),
            size: size(px(opts.width), px(opts.height)),
        },
        _ => Bounds::centered(None, size(px(opts.width), px(opts.height)), cx),
    };
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // Inherited from the app's own `vn.config.json` rather than
            // hardcoded: a pop-out of a Mica app that came up opaque white
            // would look like a different application, and the app's stylesheet
            // (written for that backdrop) would be unreadable on it. A
            // per-window `backdrop` option is §3.3's job.
            window_background: crate::window::config().backdrop.to_gpui(),
            titlebar: Some(TitlebarOptions {
                title: Some(opts.title.clone().into()),
                // A REAL OS caption (fork item F-03): movable, resizable,
                // closable, snap-layouts — for free, with no drag region.
                appears_transparent: false,
                ..Default::default()
            }),
            // Same reason the primary window sets it: a window that throttles
            // to 33 ms when unfocused would make "both windows update in the
            // same frame group" false the moment one of them loses focus —
            // which is *always*, since only one can be focused.
            inactive_frame_interval: None,
            ..Default::default()
        },
        |_window, cx| {
            cx.new(|_| PopoutShell { node })
        },
    );
    let Ok(handle) = opened else {
        vn_tree::set_popped_out(node, false);
        crate::window::advise("__vn_window_open: gpui refused to open the window");
        return;
    };
    POPOUTS.with(|p| {
        p.borrow_mut().push(Popout {
            node,
            id: handle.window_id(),
            title: opts.title.clone(),
            handle,
            on_close,
        })
    });
    ensure_close_observer(cx);
    let msg = format!(
        "[window] node {node} popped out into window {:?} — the primary window stopped painting \
         it (docs/MULTIWINDOW.md)",
        handle.window_id()
    );
    eprintln!("{msg}");
    vn_tree::log::record("window", msg);
    // The primary window has to redraw WITHOUT the subtree; nothing in the tree
    // changed, so `flush_changes` would not have asked for it.
    crate::notify_shell(cx);
}

fn ensure_close_observer(cx: &mut App) {
    if CLOSED_SUB.with(|s| s.borrow().is_some()) {
        return;
    }
    let sub = cx.on_window_closed(|cx, id| on_window_closed(id, cx));
    CLOSED_SUB.with(|s| *s.borrow_mut() = Some(sub));
}

/// The X button, `window.close()` from the popped-out window, and
/// `__vn_window_close(node)` all arrive here — one re-attach path, so a subtree
/// can never be left owned by a window that no longer exists.
fn on_window_closed(id: WindowId, cx: &mut App) {
    let Some(i) = POPOUTS.with(|p| p.borrow().iter().position(|w| w.id == id)) else {
        return;
    };
    let popout = POPOUTS.with(|p| p.borrow_mut().remove(i));
    vn_tree::set_popped_out(popout.node, false);
    let msg = format!(
        "[window] the pop-out window for node {} closed — the subtree is back in the primary \
         window",
        popout.node
    );
    eprintln!("{msg}");
    vn_tree::log::record("window", msg);
    // The primary window must repaint WITH the subtree again.
    crate::notify_shell(cx);
    if popout.on_close != 0 {
        JsRuntime::with(|rt| {
            let args = [Value::int32(popout.node as i32)];
            if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
                rt.ctx().call(Value(popout.on_close), Value::undefined(), &args)
            {
                let err = rt.describe(e);
                eprintln!("[window close callback] {}\n{}", err.header, err.stack);
            }
            // Vue schedules its patch flush on microtasks; drain now so the
            // handler's DOM mutations land in this wave, exactly as an input
            // event's do.
            rt.ctx().drain_microtasks();
            // The callback was GC-protected when it was registered and is fired
            // exactly once, so this is where it stops being rooted. Without it
            // a pop-out that is opened and closed repeatedly leaks one function
            // per cycle.
            // SAFETY: `on_close` is a value this module protected in
            // `__vn_window_open` and has not unprotected before.
            unsafe { vn_runtime::redjs::ffi::redjs_unprotect(rt.raw(), popout.on_close) };
        });
    }
}

/// Repaint every window that paints part of the tree. Registered with vn-tree
/// so a JS wave that starts inside an INPUT LISTENER — in any window — reaches
/// all of them, not only the one the event landed in. Without it, a click in
/// window B leaves window A's pixels one wave behind permanently: the listener
/// already consumed the dirty flag, so the pump's own notify never fires.
///
/// **Deferred, and that is not a detail.** gpui takes a window *out of its
/// slot* for the duration of its update (`App::update_window_id`,
/// `gpui/src/app.rs:1874`: `cx.windows.get_mut(id)?.take()?`), so
/// `update_window` on a window that is already being updated **fails**. And the
/// window we most want to repaint is frequently exactly that one: a JS wave
/// dispatched into window B from a host fn called by window A's own pump is
/// nested inside A's update, so refreshing A immediately returns `Err` and is
/// swallowed — the tree is right, A's pixels never move, and nothing says so.
/// `cx.defer` runs the fan-out at the end of the effect cycle, when every
/// window is back in its slot.
fn repaint_all(cx: &mut App) {
    cx.defer(|cx| {
        crate::refresh_main(cx);
        notify_all(cx);
    });
}

pub fn install(rt: &JsRuntime) {
    vn_tree::set_repaint_all(repaint_all);
    rt.register("__vn_window_open", 3, vn_window_open as *const _);
    rt.register("__vn_window_close", 1, vn_window_close_popout as *const _);
    rt.register("__vn_window_popouts", 0, vn_window_popouts as *const _);
    rt.register("__vn_window_click", 3, vn_window_click as *const _);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_a_small_centred_window() {
        let o = parse_options("");
        assert_eq!(o.title, "vue-native");
        assert_eq!((o.width, o.height), (420.0, 320.0));
        assert!(o.x.is_none() && o.y.is_none());
    }

    #[test]
    fn options_are_read_off_the_json() {
        let o = parse_options(r#"{"title":"Palette","x":100,"y":200,"width":300,"height":150}"#);
        assert_eq!(o.title, "Palette");
        assert_eq!(o.x, Some(100.0));
        assert_eq!(o.y, Some(200.0));
        assert_eq!((o.width, o.height), (300.0, 150.0));
    }
}
