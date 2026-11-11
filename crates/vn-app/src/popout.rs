//! **Multi-window, phase 1** — one Vue subtree, painted by a second OS window
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
//! ## What phase 1 added over the spike
//!
//! * the `<VnWindow>` component's whole prop set — `title`, `bounds`,
//!   `chrome`, `backdrop`, `alwaysOnTop`, `resizable`, `minWidth`/`minHeight` —
//!   with the creation-time-only ones advising rather than silently doing
//!   nothing (D-25/D-30's rule, §3.3);
//! * a push channel back to JS: `open` / `close` / `focus` / `blur` / `resize`,
//!   which is what makes a pop-out's own metrics knowable in a one-realm world
//!   (§7.7 — `window.innerWidth` is still the PRIMARY window's, A-M04);
//! * the per-window state map behind it (`vn_tree::windows`), so this window's
//!   focus, bounds, drag regions, atlas tiles and `@media` environment are its
//!   own (§5);
//! * `<video>` and `<lottie>` in a pop-out, which the spike refused: the tile's
//!   creating window is now recorded, so the atlas it goes back to is the one
//!   that holds it (A-M06 retired);
//! * closing the primary window closes every pop-out first (D-M16, §7.8).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, Render, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowId, WindowOptions, point, prelude::*, px, size,
};
use vn_runtime::redjs::api::Value;
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{JsRuntime, host_fn, with_app};

use crate::json::{self, Json};
use crate::popout_win;
use crate::window::Backdrop;

const UNDEF: u64 = 0xa;

/// The root view of a popped-out window. It holds nothing but the node id: the
/// state lives in the one retained tree, exactly as the primary shell's does.
pub struct PopoutShell {
    node: u32,
}

impl Render for PopoutShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = window.window_handle().window_id().as_u64();
        // **This window's own per-frame work**, which the pump physically
        // cannot do for it: the atlas tiles it owes go back to ITS atlas, the
        // zero-copy `<video>` frames decoded for it are blitted into ITS
        // atlas, and its focus bookkeeping runs against ITS focus path
        // (docs/MULTIWINDOW.md §5.1). The JS-realm half of `frame_tick` — the
        // media clocks, the animation clock, the timers — deliberately stays
        // once per FRAME GROUP in the pump (D-M06).
        vn_tree::window_frame_tick(window, &mut **cx);
        // BEFORE the env: publishing an environment for a window whose painted
        // root is not registered yet would mark nothing for a re-cascade, and
        // the subtree would keep answering `@media` against the main window
        // until something else happened to dirty it. `build_subtree` declares
        // the same thing from the other side, idempotently.
        vn_tree::windows::set_painted_root(id, self.node);
        publish_env(id, self.node, window, cx);
        report_metrics(id, self.node, window);
        frame_drawn(id, window, cx);
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

/// Publishes this window's `@media` environment (docs/MULTIWINDOW.md §6).
///
/// From the pop-out's own render, because that is where its viewport, its DPI
/// and its appearance are — the pump runs with the primary window and has none
/// of them. A change marks only the subtree this window paints for a re-cascade,
/// so a pop-out resizing costs a scoped restyle rather than a document one.
fn publish_env(id: u64, node: u32, window: &Window, cx: &App) {
    let Some(p) = POPOUTS.with(|p| p.borrow().iter().find(|w| w.id.as_u64() == id).map(|w| (w.os_chrome, w.title_bar_height))) else {
        return;
    };
    // The painted root's box, not the client area — the same definition
    // `inner_width` uses for the primary window, so an app's `@media` and its
    // `getBoundingClientRect()` cannot disagree about how wide the viewport is.
    // Before the first painted frame there is no box; the primary window's env
    // stands in for those frames rather than a cascade against zeros.
    let viewport = match vn_tree::bounds_in(node, node) {
        Some((_, _, w, h)) if w > 0.0 && h > 0.0 => (w, h),
        _ => {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        }
    };
    let env = crate::window::popout_env(window, cx, viewport, p.0, p.1);
    vn_tree::set_env_for(id, env);
}

struct Popout {
    node: u32,
    id: WindowId,
    /// What the window's caption says — the only human-readable name a window
    /// switcher can show for it. Read back through [`list`], i.e. by the
    /// devtools' `windows` command and nothing else.
    title: String,
    handle: WindowHandle<PopoutShell>,
    /// A GC-protected JS function called with `(nodeId)` when the window closes
    /// — kept as its own slot because `close` is the one event that fires after
    /// the window is already gone.
    on_close: u64,
    /// A GC-protected JS function called with `(kind, detailJson)` for every
    /// other window event this pop-out reports: `open`, `focus`, `blur`,
    /// `resize`. `<VnWindow>` turns them into Vue emits.
    on_event: u64,
    /// Does the OS draw this window's caption? `chrome: "default"` (the
    /// spike's only mode) versus `"hidden"`, which hands the whole surface to
    /// the app and makes its own `window-drag: move` region load-bearing.
    os_chrome: bool,
    /// The caption strip height this window reports through
    /// `env(titlebar-area-height)` in custom-chrome mode.
    title_bar_height: f32,
    /// Last `(width, height)` reported to JS, so `resize` fires on a change
    /// rather than every frame.
    last_metrics: (f32, f32),
    /// Last focus state reported to JS, for the `focus` / `blur` edges.
    last_focused: bool,
    /// The latest native fullscreen state; cover windows remain monitor-sized.
    fullscreen: bool,
    /// The window's HWND, captured when it opened (0 = none).
    hwnd: isize,
    /// Shown, as far as `visible` is concerned (§7.9). A hidden pop-out still
    /// paints its subtree — nobody sees it — and is shown again rather than
    /// opened again.
    visible: bool,
    /// `cover: "monitor"`: every show covers the whole monitor the main
    /// window is on.
    cover: bool,
    /// `taskbar` as it was created — read back only to advise on a change.
    taskbar: bool,
    /// `focusable` as it was created — read back only to advise on a change.
    focusable: bool,
    /// Does a show take the foreground? Never for a window that is not
    /// `focusable`.
    activate: bool,
    /// `alwaysOnTop`, as of the latest options: every show re-asserts it.
    always_on_top: bool,
    /// `ignoreMouseEvents` and `contentProtection`, as of the latest options.
    ignore_mouse_events: bool,
    content_protection: bool,
    /// `monitor`: which monitor a cover window covers, in `EnumDisplayMonitors`
    /// order. `None` = the main window's.
    monitor: Option<usize>,
    /// The size and place a window created hidden takes on its first show:
    /// `(x, y, width, height)`, CSS px. `None` once placed, and for a cover
    /// window, which the monitor places.
    first_place: Option<(Option<f32>, Option<f32>, f32, f32)>,
    /// Last declarative geometry; unrelated prop updates preserve user placement.
    requested_bounds: [Option<f32>; 4],
    /// When the show in progress was asked for, until its first frame.
    show_asked: Option<Instant>,
    /// The first frame of the show in progress was drawn: when.
    show_drawn: Option<Instant>,
    /// Milliseconds from the latest show request to its first drawn frame.
    last_show_ms: f64,
    /// Bumped by every show and hide, so a deferred tidy-up after a hide can
    /// tell that the window has been shown since.
    epoch: u64,
    /// Device px per CSS px, as of its latest frame.
    scale: f32,
    /// The client area in CSS px, as of its latest frame.
    viewport: (f32, f32),
}

thread_local! {
    static POPOUTS: RefCell<Vec<Popout>> = const { RefCell::new(Vec::new()) };
    /// One `App::on_window_closed` subscription for all of them: the callback
    /// is handed a `WindowId`, which is all the addressing we need.
    static CLOSED_SUB: RefCell<Option<Subscription>> = const { RefCell::new(None) };
    /// [`notify_all`]'s paint task is queued and has not run yet.
    static PAINT_ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
///
/// **…and painted from a task, not left to `WM_PAINT`.** A refresh only marks
/// the window dirty; its frame waits for a `WM_PAINT`, which Windows hands out
/// only once no posted message is queued, and then one window at a time. While
/// the app is busy (a screen share: the primary repainting its preview, the
/// voice engine posting events) the primary's paint is the one handed out, and
/// a pop-out whose only change is a `<canvas>` (the stream-drawing overlay)
/// went seconds without a frame. A posted task is served first, and its
/// [`popout_win::paint_now`] draws the pop-out there and then — or nothing,
/// when it is not dirty. One task at a time, however many waves ask.
pub fn notify_all(cx: &mut App) {
    let ids: Vec<WindowId> = POPOUTS.with(|p| p.borrow().iter().map(|w| w.id).collect());
    if ids.is_empty() {
        return;
    }
    for id in ids {
        let _ = gpui::AnyWindowHandle::from(WindowHandle::<PopoutShell>::new(id))
            .update(cx, |_, window, _| window.refresh());
    }
    if PAINT_ASKED.with(|asked| asked.replace(true)) {
        return;
    }
    cx.spawn(async move |_| {
        PAINT_ASKED.with(|asked| asked.set(false));
        let shown: Vec<isize> = POPOUTS.with(|p| {
            p.borrow().iter().map(|w| w.hwnd).filter(|&hwnd| hwnd != 0 && popout_win::is_visible(hwnd)).collect()
        });
        for hwnd in shown {
            popout_win::paint_now(hwnd);
        }
    })
    .detach();
}

/// Closes every pop-out. **D-M16 / §7.8**: the primary window owns the JS pump,
/// so closing it while a pop-out is open would leave that pop-out painted and
/// frozen — a window with no clock, no timers and no event loop. A pop-out is a
/// *view onto* the app rather than a peer, which is also what a one-main-window
/// Electron app does.
pub fn close_all(cx: &mut App) {
    let handles: Vec<WindowHandle<PopoutShell>> =
        POPOUTS.with(|p| p.borrow().iter().map(|w| w.handle).collect());
    for handle in handles {
        // After its file dialog, if it has one open (dialogs.rs).
        crate::dialogs::remove_window(handle.into(), cx);
    }
}

/// How many pop-outs are open. The primary window's close path reads it to
/// decide whether it has any to take down with it.
pub fn count() -> usize {
    POPOUTS.with(|p| p.borrow().len())
}

/// The open pop-outs' window keys (`vn_tree::windows::key`), which address
/// their HWNDs (`window::hwnd_of`).
pub fn ids() -> Vec<vn_tree::WinKey> {
    POPOUTS.with(|p| p.borrow().iter().map(|w| w.id.as_u64()).collect())
}

// ---------------------------------------------------------------------------
// options — `<VnWindow>`'s props, as the host sees them
// ---------------------------------------------------------------------------

struct Options {
    bounds: [Option<f32>; 4],
    title: String,
    x: Option<f32>,
    y: Option<f32>,
    width: f32,
    height: f32,
    /// `chrome: "default"` = a real OS caption; `"hidden"` = the app draws one.
    os_chrome: bool,
    /// `None` = inherit the app's own `vn.config.json` backdrop.
    backdrop: Option<Backdrop>,
    always_on_top: bool,
    resizable: bool,
    min_width: Option<f32>,
    min_height: Option<f32>,
    title_bar_height: f32,
    /// Does the new window take the keyboard when it appears?
    ///
    /// **Default `false`, which is a deliberate departure from gpui's
    /// `WindowOptions::focus: true`.** A tool palette that grabs focus every
    /// time it opens is a bug in every desktop app: the user was typing in the
    /// document, asked for a palette, and would lose the caret. `focus: false`
    /// becomes `SW_SHOWNOACTIVATE`, so the window appears, paints and is
    /// clickable without an activation. An app that genuinely wants the second
    /// window in front — a detached editor, say — passes `activate`.
    activate: bool,
    /// `visible` — LIVE. `false` creates the window hidden (warm: the next
    /// show is a show, not an open) and hides an open one (§7.9).
    visible: bool,
    /// `cover: "monitor"` — the window covers the whole monitor the main
    /// window is on, taskbar included, with no frame. Creation-time.
    cover: bool,
    /// `taskbar: false` — the main window owns it: no taskbar button and no
    /// second Alt-Tab entry. Creation-time.
    taskbar: bool,
    /// `fullResolution` — the images it shows keep their full decode while it
    /// shows them (a media viewer zooms to 1:1). Live.
    full_resolution: bool,
    /// `ignoreMouseEvents` — the mouse goes through the window to whatever is
    /// under it (§7.11). Live.
    ignore_mouse_events: bool,
    /// `focusable: false` — never activated, no taskbar button, unknown to
    /// the shell (§7.11). Creation-time.
    focusable: bool,
    /// `contentProtection` — screen capture leaves the window out. Live.
    content_protection: bool,
    /// `monitor` — the monitor a `cover` window covers, by index. Live.
    monitor: Option<usize>,
}

fn parse_options(raw: &str) -> Options {
    let j = json::parse(raw);
    let num = |k: &str| j.as_ref().and_then(|j| j.get(k)).and_then(|v| v.as_f64());
    let text = |k: &str| {
        j.as_ref().and_then(|j| j.get(k)).and_then(|v| v.as_str()).map(str::to_string)
    };
    let flag = |k: &str| j.as_ref().and_then(|j| j.get(k)).and_then(|v| v.as_bool());
    let backdrop = text("backdrop").and_then(|name| match Backdrop::parse(&name) {
        Some(b) => Some(b),
        None => {
            crate::window::advise(&format!(
                "<VnWindow backdrop=\"{name}\">: unknown backdrop — the pop-out inherits the \
                 app's own (docs/WINDOWING.md §4.4 lists the names)"
            ));
            None
        }
    });
    let bounds = ["x", "y", "width", "height"].map(|key| {
        num(key).map(|n| n as f32).filter(|n| n.is_finite()
            && (matches!(key, "x" | "y") || *n > 0.0))
    });
    Options {
        bounds,
        title: text("title").unwrap_or_else(|| "vue-native".to_string()),
        x: num("x").map(|n| n as f32),
        y: num("y").map(|n| n as f32),
        width: num("width").unwrap_or(420.0) as f32,
        height: num("height").unwrap_or(320.0) as f32,
        // `chrome` is spelled the way `<VnWindow>` spells it, which is the way
        // `vn.config.json`'s `titleBarStyle` spells it minus the `Style`.
        os_chrome: !matches!(text("chrome").as_deref(), Some("hidden")),
        backdrop,
        always_on_top: flag("alwaysOnTop").unwrap_or(false),
        resizable: flag("resizable").unwrap_or(true),
        min_width: num("minWidth").map(|n| n as f32),
        min_height: num("minHeight").map(|n| n as f32),
        title_bar_height: num("titleBarHeight").unwrap_or(32.0) as f32,
        activate: flag("activate").unwrap_or(false),
        visible: flag("visible").unwrap_or(true),
        cover: match text("cover").as_deref() {
            None | Some("") | Some("none") => false,
            Some("monitor") => true,
            Some(other) => {
                crate::window::advise(&format!(
                    "<VnWindow cover=\"{other}\">: unknown cover — expected \"monitor\". The window \
                     takes its `bounds` instead (docs/MULTIWINDOW.md §7.9)"
                ));
                false
            }
        },
        taskbar: flag("taskbar").unwrap_or(true),
        full_resolution: flag("fullResolution").unwrap_or(false),
        ignore_mouse_events: flag("ignoreMouseEvents").unwrap_or(false),
        focusable: flag("focusable").unwrap_or(true),
        content_protection: flag("contentProtection").unwrap_or(false),
        monitor: num("monitor").filter(|n| *n >= 0.0).map(|n| n as usize),
    }
}

/// The rect a cover window takes on its show: monitor `monitor` when there is
/// one by that index (advised when there is not), else the main window's.
fn cover_rect(main: Option<isize>, monitor: Option<usize>) -> Option<popout_win::Rect> {
    if let Some(i) = monitor {
        if let Some(r) = popout_win::monitor_rect_at(i) {
            return Some(r);
        }
        crate::window::advise(&format!(
            "<VnWindow :monitor=\"{i}\">: there is no monitor {i} — the window covers the main \
             window's monitor instead (docs/MULTIWINDOW.md §7.11)"
        ));
    }
    main.and_then(popout_win::monitor_rect)
}

// ---------------------------------------------------------------------------
// the host surface
// ---------------------------------------------------------------------------

// `__vn_window_open(nodeId, optionsJson, onClose?, onEvent?)`.
//
// The open itself is **deferred**. `App::open_window` builds the root view and
// then draws the window once before returning (`gpui/src/app.rs:1279`), and
// this host fn is called from inside the primary window's own update, with
// `Phase::Commit` published against IT. `cx.defer` runs the open at the end of
// the current effect cycle, i.e. with every entity returned to the app and no
// window published — which is also why the JS caller learns the window id
// through the `open` event rather than as a return value. §3.2's contract says
// so, and `<VnWindow>` is written to it.
host_fn!(fn vn_window_open(g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let on_close = args.arg(2);
    let on_event = args.arg(3);
    if node == 0 || !vn_tree::with_tree(|t| t.node(node).is_some()) {
        crate::window::advise(&format!(
            "__vn_window_open: node {node} is not in the tree — nothing was opened"
        ));
        return UNDEF;
    }
    if popout_of_node(node).is_some() {
        crate::window::advise(&format!(
            "A-M02: __vn_window_open: node {node} is already painted by another window — the \
             second call was refused. A node id may be painted by exactly one window, because \
             every node-keyed side map (focus, scroll, input, media) would otherwise alias \
             (docs/MULTIWINDOW.md §5)"
        ));
        return UNDEF;
    }
    // A-M01: a `<VnWindow>` inside a `<VnWindow>`. gpui has no owner/child
    // window relationship we honour, and a nested pop-out reads as a modality
    // request — which is a Win32 `EnableWindow` question, not a rendering one
    // (§7.6). Flattened to a sibling window, out loud.
    if let Some(outer) = enclosing_popout(node) {
        crate::window::advise(&format!(
            "A-M01: <VnWindow> (node {node}) is nested inside another one (node {outer}) — it \
             was flattened to a SIBLING window. vue-native has no owner/child window \
             relationship, and a nested pop-out reads as a modality request. See \
             docs/MULTIWINDOW.md §7.6"
        ));
    }
    let protect = |v: vn_runtime::redjs::api::Value| -> u64 {
        if v.as_number().is_some() || v.0 == UNDEF || v.0 == 0 {
            0
        } else {
            unsafe { vn_runtime::redjs::ffi::redjs_protect(g, v.0) };
            v.0
        }
    };
    let on_close = protect(on_close);
    let on_event = protect(on_event);
    let opts = parse_options(&raw);
    // Stop painting it HERE, not in the deferred open: the primary window's
    // very next draw must already be without it, or the subtree would exist in
    // two element trees for one frame (two sets of `ElementId`s over one node —
    // gpui's scroll/hitbox state is keyed by that).
    vn_tree::set_popped_out(node, true);
    with_app(|cx| {
        cx.defer(move |cx| open_now(node, opts, on_close, on_event, cx));
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
            // A file dialog it owns closes first (dialogs.rs).
            crate::dialogs::remove_window(handle.into(), cx);
        });
    });
    UNDEF
});

// `__vn_window_set_options(nodeId, optionsJson)` — the LIVE half of
// `<VnWindow>`'s props.
//
// Four of them move at runtime (`title`, `alwaysOnTop`, `visible`,
// `fullResolution`); the rest are creation-time-only in gpui and therefore
// here too. **A creation-time prop
// that changed says so and does nothing**, which is D-25/D-30's rule and the
// opposite of what a silent no-op teaches an author (docs/WINDOWING.md §3.3).
host_fn!(fn vn_window_set_options(g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let Some(i) = popout_of_node(node) else { return UNDEF };
    let (handle, old_title, old_chrome, old_visible, old_cover, old_taskbar, old_focusable) = POPOUTS.with(|p| {
        let p = p.borrow();
        let w = &p[i];
        (w.handle, w.title.clone(), w.os_chrome, w.visible, w.cover, w.taskbar, w.focusable)
    });
    let opts = parse_options(&raw);
    if opts.focusable != old_focusable {
        crate::window::advise(
            "<VnWindow focusable>: whether a window can be activated is decided when it is created \
             (the taskbar and the shell read its styles when it is first shown), so changing it \
             does nothing to a window that is already open — close and re-open it \
             (docs/MULTIWINDOW.md §7.11)",
        );
    }
    if opts.os_chrome != old_chrome {
        crate::window::advise(
            "<VnWindow chrome>: the caption mode is decided when the window is created (a real \
             `WS_CAPTION` is a window STYLE), so changing it does nothing to a window that is \
             already open — close and re-open it. Same rule as `titleBarStyle` \
             (docs/WINDOWING.md D-25)",
        );
    }
    if opts.cover != old_cover {
        crate::window::advise(
            "<VnWindow cover>: covering the monitor is decided when the window is created (it \
             takes the frame off the window), so changing it does nothing to a window that is \
             already open — close and re-open it (docs/MULTIWINDOW.md §7.9)",
        );
    }
    if opts.taskbar != old_taskbar {
        crate::window::advise(
            "<VnWindow taskbar>: the taskbar reads a window's owner and styles when it is shown, \
             so this is decided when the window is created and changing it does nothing to a \
             window that is already open — close and re-open it (docs/MULTIWINDOW.md §7.9)",
        );
    }
    // Live, and nothing to schedule: the next residency sweep reads it.
    vn_tree::windows::set_full_res_images(handle.window_id().as_u64(), opts.full_resolution);
    let title_moved = opts.title != old_title;
    if title_moved {
        POPOUTS.with(|p| p.borrow_mut()[i].title = opts.title.clone());
    }
    // `visible` is recorded now, so a second change in the same wave compares
    // against it; the show or hide itself is deferred with the rest.
    let visible = opts.visible;
    let visible_moved = visible != old_visible;
    if visible_moved {
        POPOUTS.with(|p| p.borrow_mut()[i].visible = visible);
    }
    let always_on_top = opts.always_on_top;
    // §7.11's live three, recorded now (a second change in the same wave
    // compares against them) and applied with the rest, out of this update:
    // a style change re-frames the window, and gpui hears of that through its
    // window proc.
    let (hwnd, click_through, protection, monitor_moved, bounds_moved) = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = &mut p[i];
        w.always_on_top = always_on_top;
        let click_through = (w.ignore_mouse_events != opts.ignore_mouse_events).then_some(opts.ignore_mouse_events);
        w.ignore_mouse_events = opts.ignore_mouse_events;
        let protection = (w.content_protection != opts.content_protection).then_some(opts.content_protection);
        w.content_protection = opts.content_protection;
        let monitor_moved = w.monitor != opts.monitor;
        w.monitor = opts.monitor;
        let bounds_moved = w.requested_bounds != opts.bounds && !w.cover;
        w.requested_bounds = opts.bounds;
        if bounds_moved {
            if let Some((x, y, width, height)) = w.first_place.as_mut() {
                *x = opts.bounds[0].or(*x);
                *y = opts.bounds[1].or(*y);
                *width = opts.bounds[2].unwrap_or(*width);
                *height = opts.bounds[3].unwrap_or(*height);
            }
        }
        (w.hwnd, click_through, protection, monitor_moved, bounds_moved)
    });
    // A shown cover window that is asked for another monitor moves there now
    // (a game moved monitors); a hidden one takes it on its next show.
    let replace = monitor_moved && old_cover && old_visible && visible && hwnd != 0;
    let monitor = opts.monitor;
    let bounds_request = bounds_moved.then_some(opts.bounds);
    let id = handle.window_id();
    with_app(|cx| {
        cx.defer(move |cx| {
            let _ = gpui::AnyWindowHandle::from(handle).update(cx, |_, window, _| {
                if title_moved {
                    window.set_window_title(&opts.title);
                }
            });
            set_always_on_top(handle, always_on_top);
            if let Some(on) = click_through {
                popout_win::set_click_through(hwnd, on);
            }
            if let Some(on) = protection {
                popout_win::set_content_protection(hwnd, on);
            }
            if replace {
                if let Some(r) = cover_rect(crate::window::primary_hwnd(), monitor) {
                    // Outside any update, as every resize here is (`warm_small`).
                    cx.spawn(async move |_| {
                        popout_win::set_rect(hwnd, r);
                        popout_win::paint_now(hwnd);
                    })
                    .detach();
                }
            }
            if let Some(bounds) = bounds_request {
                // SetWindowPos may synchronously send WM_SIZE/WM_DPICHANGED.
                // Run outside App/Window borrows, on the existing HWND, without activation.
                cx.spawn(async move |_| {
                    if let (Some(window), Some(client)) =
                        (popout_win::window_rect(hwnd), popout_win::client_rect_on_screen(hwnd))
                    {
                        let rect = live_client_rect(window, client, crate::window::primary_scale(), bounds);
                        let _ = popout_win::set_rect(hwnd, rect);
                    }
                }).detach();
            }
            if visible_moved {
                if visible {
                    show_now(id, cx);
                } else {
                    hide_now(id, cx);
                }
            }
        });
    });
    UNDEF
});

/// Win32 `HWND_TOPMOST` for one pop-out. gpui has no per-window API for it, so
/// this goes through the HWND the window host cached when that window painted —
/// which is exactly why that cache had to grow a window key (§5.1).
fn set_always_on_top(handle: WindowHandle<PopoutShell>, on: bool) {
    let key = handle.window_id().as_u64();
    let Some(hwnd) = crate::window::hwnd_of(key) else { return };
    crate::window::set_always_on_top_hwnd(hwnd, on);
}

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
// It exists because a spec that runs while somebody is using the machine may
// not move the real cursor. (Since devtools v2, `ctl.ts --window <id> mouse`
// reaches the same path through the command queue.)
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

// `__vn_window_popouts()` — `[{node, windowId, title, chrome, width, height}]`,
// for tests and devtools.
host_fn!(fn vn_window_popouts(_g, _args) {
    let json = POPOUTS.with(|p| {
        Json::arr(
            p.borrow()
                .iter()
                .map(|w| {
                    // The window as Win32 has it, for the specs of §7.9: what
                    // DWM shows, where, and who owns it.
                    let rect = popout_win::window_rect(w.hwnd).map_or(Json::Null, |r| {
                        Json::arr(r.iter().map(|v| Json::n(*v as f64)).collect())
                    });
                    let (owner, appwindow, toolwindow, popup, thickframe, non_rude) =
                        popout_win::describe(w.hwnd);
                    let (layered, transparent, no_activate) = popout_win::describe_input(w.hwnd);
                    Json::obj(vec![
                        ("node", Json::n(w.node as f64)),
                        ("windowId", Json::n(w.id.as_u64() as f64)),
                        ("title", Json::s(w.title.clone())),
                        ("chrome", Json::s(if w.os_chrome { "default" } else { "hidden" })),
                        ("width", Json::n(w.last_metrics.0)),
                        ("height", Json::n(w.last_metrics.1)),
                        ("focused", Json::Bool(w.last_focused)),
                        ("visible", Json::Bool(w.visible)),
                        ("shown", Json::Bool(popout_win::is_visible(w.hwnd))),
                        ("foreground", Json::Bool(popout_win::is_foreground(w.hwnd))),
                        ("cover", Json::Bool(w.cover)),
                        ("rect", rect),
                        ("hwnd", Json::n(w.hwnd as f64)),
                        ("ownedByMain", Json::Bool(owner != 0 && Some(owner) == crate::window::primary_hwnd())),
                        ("appWindow", Json::Bool(appwindow)),
                        ("toolWindow", Json::Bool(toolwindow)),
                        ("popup", Json::Bool(popup)),
                        ("thickFrame", Json::Bool(thickframe)),
                        ("nonRude", Json::Bool(non_rude)),
                        // §7.11: the overlay's half, as Win32 has it.
                        ("focusable", Json::Bool(w.focusable)),
                        ("ignoreMouseEvents", Json::Bool(w.ignore_mouse_events)),
                        ("layered", Json::Bool(layered)),
                        ("transparent", Json::Bool(transparent)),
                        ("noActivate", Json::Bool(no_activate)),
                        ("displayAffinity", Json::n(popout_win::display_affinity(w.hwnd) as f64)),
                        ("monitor", w.monitor.map_or(Json::Null, |m| Json::n(m as f64))),
                        ("lastShowMs", Json::n(w.last_show_ms)),
                        (
                            "mainMonitor",
                            crate::window::primary_hwnd().and_then(popout_win::monitor_rect).map_or(
                                Json::Null,
                                |r| Json::arr(r.iter().map(|v| Json::n(*v as f64)).collect()),
                            ),
                        ),
                    ])
                })
                .collect(),
        )
        .to_json()
    });
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

// `__vn_window_map_rect(popoutNode, nodeId)` — where `nodeId`, as the PRIMARY
// window painted it, falls inside the pop-out painting `popoutNode`: a
// `[x, y, width, height]` JSON string in that pop-out's CSS px, relative to its
// painted root (the space an absolutely positioned child of the root lives
// in), or `undefined` when either was not painted. It goes through the screen,
// so it holds across monitors and DPIs — what a viewer's zoom out of the
// thumbnail needs (§7.9).
//
// A cover window is measured where its next show puts it, the main window's
// monitor at the main window's scale, so the answer is already right while it
// is hidden and still warm-small.
host_fn!(fn vn_window_map_rect(_g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let src = args.arg(1).as_number().unwrap_or(0.0) as u32;
    let Some(i) = popout_of_node(node) else { return UNDEF };
    let (hwnd, cover, scale, monitor) = POPOUTS.with(|p| {
        let p = p.borrow();
        (p[i].hwnd, p[i].cover, p[i].scale, p[i].monitor)
    });
    let Some(rect) = vn_tree::bounds_in(0, src) else { return UNDEF };
    let Some(main) = crate::window::primary_hwnd() else { return UNDEF };
    let Some(from) = popout_win::client_rect_on_screen(main) else { return UNDEF };
    let main_scale = crate::window::primary_scale();
    let to = if cover {
        // On another monitor than the main window's, at the scale of its own
        // latest frame — the main window's says nothing about that monitor.
        let at = monitor.and_then(popout_win::monitor_rect_at);
        let s = if at.is_some() { scale } else { main_scale };
        at.or_else(|| popout_win::monitor_rect(main)).map(|r| (r, s))
    } else {
        popout_win::client_rect_on_screen(hwnd).map(|r| (r, scale))
    };
    let Some((to, to_scale)) = to else { return UNDEF };
    let root = vn_tree::bounds_in(node, node).map_or((0.0, 0.0), |r| (r.0, r.1));
    let (x, y, w, h) =
        popout_win::map_rect(rect, (from[0], from[1]), main_scale, (to[0], to[1]), to_scale);
    let json = Json::arr(vec![Json::n(x - root.0), Json::n(y - root.1), Json::n(w), Json::n(h)]).to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

/// Query or change the OS window owning an element-fullscreen session.
/// `0` addresses the primary window; every other id must be an existing
/// VnWindow root. Missing/closed owners never fall back to the primary.
fn fullscreen_window(owner: u32, on: Option<bool>) -> bool {
    if owner == 0 {
        return crate::window::element_fullscreen(on);
    }
    let Some((handle, cover, current)) = POPOUTS.with(|popouts| {
        popouts
            .borrow()
            .iter()
            .find(|window| window.node == owner)
            .map(|window| {
                (
                    window.handle,
                    window.cover,
                    window.cover || window.fullscreen,
                )
            })
    }) else {
        return false;
    };
    if let Some(on) = on {
        with_app(|cx| {
            cx.defer(move |cx| {
                let _ = gpui::AnyWindowHandle::from(handle).update(cx, |_, window, _| {
                    // A cover window already owns the monitor. Keep its
                    // placement and ownership unchanged on enter and exit.
                    if !cover && window.is_fullscreen() != on {
                        window.toggle_fullscreen();
                    }
                    window.refresh();
                    POPOUTS.with(|popouts| {
                        if let Some(state) = popouts
                            .borrow_mut()
                            .iter_mut()
                            .find(|state| state.id == handle.window_id())
                        {
                            state.fullscreen = window.is_fullscreen();
                        }
                    });
                });
            });
        });
    }
    current
}

host_fn!(fn vn_fullscreen_window(_g, args) {
    let owner = args.arg(0).as_number().unwrap_or(0.0) as u32;
    Value::boolean(fullscreen_window(owner, args.arg(1).as_bool())).0
});

fn popout_of_node(node: u32) -> Option<usize> {
    POPOUTS.with(|p| p.borrow().iter().position(|w| w.node == node))
}

/// The nearest ancestor of `node` that is itself popped out — A-M01's test.
fn enclosing_popout(node: u32) -> Option<u32> {
    vn_tree::with_tree(|t| {
        let mut cursor = t.node(node)?.parent;
        for _ in 0..1024 {
            if cursor == 0 {
                return None;
            }
            if vn_tree::popped_out(cursor) {
                return Some(cursor);
            }
            cursor = t.node(cursor)?.parent;
        }
        None
    })
}

// ---------------------------------------------------------------------------
// the push channel back to JS
// ---------------------------------------------------------------------------

/// [`emit`] from the top of an effect cycle (a `defer`, a timer), with the app
/// published to the handler: a handler that answers by changing a prop —
/// `visible` again, say — reaches `__vn_window_set_options`, which schedules
/// through the app and would otherwise be dropped.
fn emit_in(cx: &mut App, on_event: u64, kind: &str, detail: Json) {
    if on_event == 0 {
        return;
    }
    vn_runtime::with_published_app(cx, || emit(on_event, kind, detail));
}

/// Fires one `<VnWindow>` event. `detail` is a JSON object the component turns
/// into the emit's payload.
fn emit(on_event: u64, kind: &str, detail: Json) {
    if on_event == 0 {
        return;
    }
    // Outside a publication: settle an input dispatch's owed checkpoint first,
    // as every publication does (vn_runtime::bridge::owe_drain).
    vn_runtime::bridge::settle_owed_drain();
    JsRuntime::with(|rt| {
        let args = [rt.ctx().string(kind), rt.ctx().string(&detail.to_json())];
        if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
            rt.ctx().call(Value(on_event), Value::undefined(), &args)
        {
            let err = rt.describe(e);
            eprintln!("[VnWindow event] {}\n{}", err.header, err.stack);
        }
        // Vue schedules its patch flush on microtasks; drain now so the
        // handler's mutations land in this wave, exactly as an input event's do.
        rt.ctx().drain_microtasks();
    });
}

/// `resize` / `focus` / `blur`, from the pop-out's own render.
///
/// **This is the honest answer to "what are a pop-out's metrics?"** in a
/// one-realm world (§7.7): `window.innerWidth` is a JS global and there is one
/// JS realm, so it keeps meaning the PRIMARY window's viewport (A-M04). A
/// pop-out's own size is pushed to the component that asked for the window,
/// which is where an app can actually use it — `<VnWindow @resize>` and the
/// `width`/`height` it exposes.
fn report_metrics(id: u64, node: u32, window: &Window) {
    let viewport = match vn_tree::bounds_in(node, node) {
        Some((_, _, w, h)) if w > 0.0 && h > 0.0 => (w, h),
        _ => return,
    };
    let focused = window.is_window_active();
    let (on_event, resized, focus_moved) = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let Some(w) = p.iter_mut().find(|w| w.id.as_u64() == id) else {
            return (0, false, false);
        };
        let resized = w.last_metrics != viewport;
        let focus_moved = w.last_focused != focused;
        w.last_metrics = viewport;
        w.last_focused = focused;
        w.fullscreen = window.is_fullscreen();
        (w.on_event, resized, focus_moved)
    });
    if on_event == 0 {
        return;
    }
    if resized {
        emit(
            on_event,
            "resize",
            Json::obj(vec![
                ("width", Json::n(viewport.0)),
                ("height", Json::n(viewport.1)),
                ("scale", Json::n(window.scale_factor())),
            ]),
        );
    }
    if focus_moved {
        emit(on_event, if focused { "focus" } else { "blur" }, Json::obj(vec![]));
    }
}

// ---------------------------------------------------------------------------
// addressing a pop-out from the devtools (docs/DEVTOOLS.md, the `window` param)
// ---------------------------------------------------------------------------

/// `(window id, the node this window paints, its title)` for every pop-out.
///
/// The devtools' `windows` command puts the primary window at the head of this
/// list; here there is nothing but the pop-outs, because the primary window's
/// identity is `lib.rs`'s to report.
///
/// That command is also the only caller, so a `--features no-devtools` build
/// has none. Kept rather than `cfg`'d: the pop-out registry is this module's
/// own state (docs/MULTIWINDOW.md) and reading it back is part of what the
/// registry IS, whoever happens to be asking this month.
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

// ---------------------------------------------------------------------------
// opening and closing
// ---------------------------------------------------------------------------

fn open_now(node: u32, opts: Options, on_close: u64, on_event: u64, cx: &mut App) {
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
    if opts.activate && !opts.focusable {
        crate::window::advise(
            "<VnWindow activate :focusable=\"false\">: a window that cannot be activated does not \
             take the foreground when it is shown either — `activate` is ignored \
             (docs/MULTIWINDOW.md §7.11)",
        );
    }
    let activate = opts.activate && opts.focusable;
    // Created hidden, as a cover window is, and for the same reason: its
    // styles change first, and the shell reads them when it is shown. A
    // window shown as an app window and only then made a tool window has
    // already told the shell it exists (§7.11).
    let hidden_first = opts.cover || !opts.focusable;
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // `backdrop` defaults to the app's own `vn.config.json` rather than
            // to a hardcoded value: a pop-out of a Mica app that came up opaque
            // white would look like a different application, and the app's
            // stylesheet (written for that backdrop) would be unreadable on it.
            window_background: opts
                .backdrop
                .unwrap_or_else(|| crate::window::config().backdrop)
                .to_gpui(),
            titlebar: Some(TitlebarOptions {
                title: Some(opts.title.clone().into()),
                // `chrome: "default"` is a REAL OS caption (fork item F-03):
                // movable, resizable, closable, snap-layouts — for free, with
                // no drag region. `chrome: "hidden"` hands the whole surface to
                // the app, which then has to paint its own `window-drag: move`
                // region or the window is unmovable — exactly the tier-3
                // contract `titleBarStyle: "hidden"` has in the primary window.
                appears_transparent: !opts.os_chrome,
                traffic_light_position: crate::window::traffic_light_position(
                    !opts.os_chrome,
                    opts.title_bar_height,
                ),
                ..Default::default()
            }),
            is_resizable: opts.resizable,
            // See `Options::activate`: a pop-out appears without stealing the
            // keyboard unless the app asks for it.
            focus: activate && !crate::quiet::never_focus(),
            window_min_size: match (opts.min_width, opts.min_height) {
                (None, None) => None,
                (w, h) => Some(size(px(w.unwrap_or(0.0)), px(h.unwrap_or(0.0)))),
            },
            // Same reason the primary window sets it: a window that throttles
            // to 33 ms when unfocused would make "both windows update in the
            // same frame group" false the moment one of them loses focus —
            // which is *always*, since only one can be focused.
            inactive_frame_interval: None,
            // A cover window is always created hidden, even a `visible` one:
            // its styles change and it is placed on the monitor first, so its
            // first visible frame already covers it (§7.9). So is a window
            // that is not `focusable` (§7.11).
            show: opts.visible && !hidden_first,
            is_minimizable: !opts.cover,
            ..Default::default()
        },
        |window, cx| {
            // Keep the generic GPUI default; enable retention only for VN windows.
            // The override allows equivalent workloads to compare both paths.
            if std::env::var_os("VN_RETAINED_LAYOUT").as_deref() != Some(std::ffi::OsStr::new("0")) {
                window.vn_enable_retained_layout();
            }
            crate::quiet::send_to_back(window);
            cx.new(|_| PopoutShell { node })
        },
    );
    let Ok(handle) = opened else {
        vn_tree::set_popped_out(node, false);
        crate::window::advise("__vn_window_open: gpui refused to open the window");
        return;
    };
    let key = handle.window_id().as_u64();
    // This window's chrome mode, before its first build: `with_control_area`
    // reads it to decide whether a `window-drag` region is live or inert, and
    // the first frame is already a frame (docs/MULTIWINDOW.md §7.6).
    vn_tree::set_os_chrome_in(key, opts.os_chrome);
    vn_tree::windows::set_full_res_images(key, opts.full_resolution);
    // The HWND has to exist before `alwaysOnTop` can be applied to it, and
    // `open_window` has already drawn the window once by now — so the handle is
    // live and `capture_hwnd` succeeds.
    let mut hwnd = 0;
    let _ = gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| {
        hwnd = crate::window::capture_hwnd(window).unwrap_or(0);
        // Its own X while it owns a file dialog: the dialog closes first
        // (dialogs.rs). The OS keeps an owner disabled under its modal
        // dialog, so this is the taskbar's "Close window", not a click.
        window.on_window_should_close(cx, |window, cx| crate::dialogs::may_close_now(window.window_handle(), cx));
    });
    // §7.9, while the window is still hidden: the taskbar reads a window's
    // styles when it is shown, and a cover window's first frame must already
    // be the monitor's size.
    if hwnd != 0 {
        if !opts.taskbar {
            if let Some(main) = crate::window::primary_hwnd() {
                popout_win::make_owned(hwnd, main);
            }
        }
        if opts.cover {
            popout_win::make_cover(hwnd);
            // WARM SMALL. A hidden window keeps its swapchain, and one the
            // monitor's size is 30 MB at 1440p and 66 MB at 4K, in shared
            // memory on an integrated GPU. The first show sizes it. Out of
            // this update, as every resize here is: see `resize_outside`.
            cx.spawn(async move |_| warm_small(hwnd)).detach();
        }
        // §7.11, while it is still hidden too.
        if !opts.focusable {
            popout_win::make_unfocusable(hwnd);
        }
        if opts.ignore_mouse_events {
            popout_win::set_click_through(hwnd, true);
        }
        if opts.content_protection {
            popout_win::set_content_protection(hwnd, true);
        }
    }
    let title = opts.title.clone();
    POPOUTS.with(|p| {
        p.borrow_mut().push(Popout {
            node,
            id: handle.window_id(),
            title: title.clone(),
            handle,
            on_close,
            on_event,
            os_chrome: opts.os_chrome,
            title_bar_height: opts.title_bar_height,
            last_metrics: (0.0, 0.0),
            last_focused: false,
            fullscreen: false,
            hwnd,
            visible: opts.visible,
            cover: opts.cover,
            taskbar: opts.taskbar,
            focusable: opts.focusable,
            activate,
            always_on_top: opts.always_on_top,
            ignore_mouse_events: opts.ignore_mouse_events,
            content_protection: opts.content_protection,
            monitor: opts.monitor,
            // Every window created hidden takes its `bounds` on its first
            // show — gpui never applied them — except a cover window, which
            // the monitor places.
            requested_bounds: opts.bounds,
            first_place: ((!opts.visible || hidden_first) && !opts.cover)
                .then_some((opts.x, opts.y, opts.width, opts.height)),
            show_asked: None,
            show_drawn: None,
            last_show_ms: 0.0,
            epoch: 0,
            scale: 1.0,
            viewport: (0.0, 0.0),
        })
    });
    if opts.always_on_top {
        set_always_on_top(handle, true);
    }
    ensure_close_observer(cx);
    let msg = format!(
        "[window] node {node} popped out into window {:?} — the primary window stopped painting \
         it (docs/MULTIWINDOW.md)",
        handle.window_id()
    );
    eprintln!("{msg}");
    vn_tree::log::record("window", msg);
    emit_in(
        cx,
        on_event,
        "open",
        Json::obj(vec![("windowId", Json::n(key as f64)), ("node", Json::n(node as f64))]),
    );
    // The primary window has to redraw WITHOUT the subtree; nothing in the tree
    // changed, so `flush_changes` would not have asked for it.
    crate::notify_shell(cx);
    // A cover window asked for `visible` was created hidden all the same, as
    // was one that is not `focusable`; this is its first show.
    if opts.visible && hidden_first {
        show_now(handle.window_id(), cx);
    }
}

/// The edge, physical px, of the hidden window a cover window waits as.
const WARM_SIZE: i32 = 64;
/// How long a hidden cover window keeps the monitor-sized swapchain of its
/// last show before it goes back to [`WARM_SIZE`].
const SHRINK_AFTER: Duration = Duration::from_secs(30);
/// How long after a hide the hidden window draws once, to hand back the atlas
/// tiles of what it showed (§7.9).
const TIDY_AFTER: Duration = Duration::from_millis(500);

/// Shows a pop-out (`visible` became `true`), §7.9.
///
/// The frame is drawn BEFORE the window is shown: a hidden window gets no
/// `WM_PAINT`, so whatever it presented last — the previous picture of a
/// viewer — would otherwise be what DWM shows until the next vsync. A cover
/// window is placed on the main window's monitor first, so that frame is
/// already the monitor's size.
fn show_now(id: WindowId, cx: &mut App) {
    let Some((handle, hwnd, cover, activate, first, monitor, on_top)) = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = p.iter_mut().find(|w| w.id == id)?;
        w.epoch += 1;
        w.show_asked = Some(Instant::now());
        w.show_drawn = None;
        Some((w.handle, w.hwnd, w.cover, w.activate, w.first_place.take(), w.monitor, w.always_on_top))
    }) else {
        return;
    };
    if hwnd == 0 {
        return;
    }
    let main = crate::window::primary_hwnd();
    // Where it goes is worked out here and the move is made outside this
    // update, with the draw (see [`warm_small`]).
    let target = if cover {
        cover_rect(main, monitor).filter(|r| popout_win::window_rect(hwnd) != Some(*r))
    } else {
        first.and_then(|f| first_rect(hwnd, main, f))
    };
    let _ = gpui::AnyWindowHandle::from(handle).update(cx, |_, window, _| window.refresh());
    let activate = activate && !crate::quiet::never_focus();
    // Outside this update: the draw takes the app.
    cx.spawn(async move |cx| {
        if let Some(r) = target {
            popout_win::set_rect(hwnd, r);
            // A frame drawn while it was being placed is not the show's.
            cx.update(|_| {
                POPOUTS.with(|p| {
                    if let Some(w) = p.borrow_mut().iter_mut().find(|w| w.id == id) {
                        w.show_drawn = None;
                    }
                })
            });
        }
        popout_win::paint_now(hwnd);
        popout_win::show(hwnd, activate);
        // Back on top of the topmost band: whatever became topmost while it
        // was hidden — the taskbar, another overlay — would otherwise stay
        // above it. Argon's native overlay re-asserts it on every show too.
        if on_top {
            crate::window::set_always_on_top_hwnd(hwnd, true);
        }
        cx.update(|cx| shown(id, cx));
    })
    .detach();
}

/// Back to [`WARM_SIZE`], where it is.
///
/// **Every resize of a pop-out in this module runs outside any `App` update**,
/// from a spawned task. gpui hears of a window's new size through `WM_SIZE`
/// and a callback that needs the app; inside an update that callback is lost,
/// and the swapchain follows the new size while the window goes on laying out
/// at the old one.
fn warm_small(hwnd: isize) {
    if let Some(r) = popout_win::window_rect(hwnd) {
        popout_win::set_rect(hwnd, [r[0], r[1], r[0] + WARM_SIZE, r[1] + WARM_SIZE]);
    }
}

/// The window is on screen. Its first frame was drawn before the show, or the
/// next vsync draws it and [`frame_drawn`] reports then.
fn shown(id: WindowId, cx: &mut App) {
    let Some((on_event, asked, drawn)) = POPOUTS.with(|p| {
        let p = p.borrow();
        let w = p.iter().find(|w| w.id == id)?;
        Some((w.on_event, w.show_asked, w.show_drawn))
    }) else {
        return;
    };
    if let (Some(asked), Some(_)) = (asked, drawn) {
        report_show(cx, id, on_event, Instant::now().duration_since(asked));
    }
}

/// Fires `show` once per show, with how long it took to the first frame.
fn report_show(cx: &mut App, id: WindowId, on_event: u64, took: Duration) {
    let ms = took.as_secs_f64() * 1000.0;
    let viewport = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = p.iter_mut().find(|w| w.id == id)?;
        w.show_asked = None;
        w.last_show_ms = ms;
        Some((w.viewport, w.scale))
    });
    let Some(((width, height), scale)) = viewport else { return };
    emit_in(
        cx,
        on_event,
        "show",
        Json::obj(vec![
            ("firstFrameMs", Json::n(ms)),
            ("width", Json::n(width)),
            ("height", Json::n(height)),
            ("scale", Json::n(scale)),
        ]),
    );
}

/// From the pop-out's own render: the first frame of a show, and the scale
/// [`vn_window_map_rect`] needs.
fn frame_drawn(id: u64, window: &Window, cx: &mut App) {
    let report = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = p.iter_mut().find(|w| w.id.as_u64() == id)?;
        w.scale = window.scale_factor();
        let size = window.viewport_size();
        w.viewport = (f32::from(size.width), f32::from(size.height));
        let asked = w.show_asked?;
        if w.show_drawn.is_some() {
            return None;
        }
        let now = Instant::now();
        w.show_drawn = Some(now);
        // Drawn while still hidden (the usual case): `shown` reports once
        // the window is on screen. Drawn after it (the paint was skipped):
        // this frame is the first one anybody sees.
        popout_win::is_visible(w.hwnd).then_some((w.id, w.on_event, now.duration_since(asked)))
    });
    if let Some((wid, on_event, took)) = report {
        cx.defer(move |cx| report_show(cx, wid, on_event, took));
    }
}

/// Hides a pop-out (`visible` became `false`), §7.9. The activation goes back
/// to the main window when this one had it.
///
/// What the hidden window still holds is let go of on a timer, unless it is
/// shown again first: after [`TIDY_AFTER`] it draws once (its subtree is
/// empty by then), which hands back the atlas tiles of what it showed — a
/// hidden window gets no frames, and tiles are returned by a frame; after
/// [`SHRINK_AFTER`] a cover window goes back to [`WARM_SIZE`], and with it
/// its monitor-sized swapchain.
fn hide_now(id: WindowId, cx: &mut App) {
    let Some((hwnd, epoch, on_event, cover)) = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = p.iter_mut().find(|w| w.id == id)?;
        w.epoch += 1;
        w.show_asked = None;
        Some((w.hwnd, w.epoch, w.on_event, w.cover))
    }) else {
        return;
    };
    if hwnd == 0 {
        return;
    }
    let return_to = crate::window::primary_hwnd().filter(|_| !crate::quiet::never_focus());
    popout_win::hide(hwnd, return_to);
    emit_in(cx, on_event, "hide", Json::obj(vec![]));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(TIDY_AFTER).await;
        if cx.update(|cx| still_hidden(id, epoch, cx)) {
            popout_win::paint_now(hwnd);
        }
        if !cover {
            return;
        }
        cx.background_executor().timer(SHRINK_AFTER - TIDY_AFTER).await;
        if cx.update(|cx| still_hidden(id, epoch, cx)) {
            warm_small(hwnd);
            popout_win::paint_now(hwnd);
            vn_tree::log::record(
                "window",
                format!("[window] the hidden cover window {id:?} went back to its warm size"),
            );
        }
    })
    .detach();
}

/// Is the hide `epoch` still the latest thing that happened to the window?
/// If so it is made dirty, for a resize and a draw the caller makes outside
/// this update.
fn still_hidden(id: WindowId, epoch: u64, cx: &mut App) -> bool {
    let Some((handle, hwnd)) = POPOUTS.with(|p| {
        let p = p.borrow();
        let w = p.iter().find(|w| w.id == id)?;
        (w.epoch == epoch && !w.visible).then_some((w.handle, w.hwnd))
    }) else {
        return false;
    };
    if popout_win::is_visible(hwnd) {
        return false;
    }
    gpui::AnyWindowHandle::from(handle).update(cx, |_, window, _| window.refresh()).is_ok()
}

/// The first show of a window created hidden without `cover`: gpui never
/// applied its placement (it would have, showing it), so the window takes its
/// `bounds` now — the client size asked for, at `x`/`y` or centred on the main
/// window's monitor, CSS px at the main window's scale. The window rect that
/// makes, for the caller to move it to.
/// Merge partial CSS-pixel bounds into the current client rect while preserving
/// the native frame. The coordinate space matches initial VnWindow placement.
fn live_client_rect(window: popout_win::Rect, client: popout_win::Rect, scale: f32,
    bounds: [Option<f32>; 4]) -> popout_win::Rect
{
    let border = [client[0] - window[0], client[1] - window[1], window[2] - client[2], window[3] - client[3]];
    let x = bounds[0].map_or(client[0], |n| (n * scale).round() as i32);
    let y = bounds[1].map_or(client[1], |n| (n * scale).round() as i32);
    let width = bounds[2].map_or(client[2] - client[0], |n| (n * scale).round().max(1.0) as i32);
    let height = bounds[3].map_or(client[3] - client[1], |n| (n * scale).round().max(1.0) as i32);
    [x - border[0], y - border[1], x + width + border[2], y + height + border[3]]
}

fn first_rect(
    hwnd: isize,
    main: Option<isize>,
    (x, y, w, h): (Option<f32>, Option<f32>, f32, f32),
) -> Option<popout_win::Rect> {
    let scale = crate::window::primary_scale();
    let (win, client) = (popout_win::window_rect(hwnd)?, popout_win::client_rect_on_screen(hwnd)?);
    // The frame around the client area, as this window's styles draw it.
    let border = [client[0] - win[0], client[1] - win[1], win[2] - client[2], win[3] - client[3]];
    let (cw, ch) = ((w * scale).round() as i32, (h * scale).round() as i32);
    let monitor = main.and_then(popout_win::monitor_rect).unwrap_or([0, 0, cw, ch]);
    let left = x.map_or(monitor[0] + (monitor[2] - monitor[0] - cw) / 2, |x| (x * scale).round() as i32);
    let top = y.map_or(monitor[1] + (monitor[3] - monitor[1] - ch) / 2, |y| (y * scale).round() as i32);
    Some([left - border[0], top - border[1], left + cw + border[2], top + ch + border[3]])
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
    // Everything this window held goes with it: its bounds map (in a coordinate
    // space that no longer exists), its focus subscriptions, its `@media`
    // environment, and the atlas tiles it still owed — whose atlas has been
    // destroyed, so handing them to another window's `drop_image` would remove
    // keys a LIVE window may have re-inserted (docs/MULTIWINDOW.md §5.1).
    vn_tree::forget_window(id.as_u64());
    crate::window::forget_window(id.as_u64());
    let msg = format!(
        "[window] the pop-out window for node {} closed — the subtree is back in the primary \
         window",
        popout.node
    );
    eprintln!("{msg}");
    vn_tree::log::record("window", msg);
    // The primary window must repaint WITH the subtree again, and re-cascade it:
    // it was answering against the pop-out's `@media` environment and is about
    // to be laid out in this one.
    vn_tree::with_tree(|t| t.mark_restyle_subtree(popout.node));
    crate::notify_shell(cx);
    if popout.on_close != 0 {
        vn_runtime::bridge::settle_owed_drain();
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
    if popout.on_event != 0 {
        // SAFETY: same contract as `on_close` above — protected once at open,
        // unprotected once here, and the window it belonged to is gone.
        JsRuntime::with(|rt| unsafe {
            vn_runtime::redjs::ffi::redjs_unprotect(rt.raw(), popout.on_event)
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
pub(crate) fn repaint_all(cx: &mut App) {
    cx.defer(|cx| {
        crate::refresh_main(cx);
        notify_all(cx);
    });
}

pub fn install(rt: &JsRuntime) {
    vn_tree::set_repaint_all(repaint_all);
    rt.register("__vn_fullscreen_window", 2, vn_fullscreen_window as *const _);
    rt.register("__vn_window_open", 4, vn_window_open as *const _);
    rt.register("__vn_window_close", 1, vn_window_close_popout as *const _);
    rt.register("__vn_window_popouts", 0, vn_window_popouts as *const _);
    rt.register("__vn_window_click", 3, vn_window_click as *const _);
    rt.register("__vn_window_set_options", 2, vn_window_set_options as *const _);
    rt.register("__vn_window_map_rect", 2, vn_window_map_rect as *const _);
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_partial_bounds_preserve_frame_and_untouched_dimensions() {
        let window = [90, 70, 530, 420];
        let client = [100, 100, 520, 410];
        assert_eq!(super::live_client_rect(window, client, 1.5, [Some(200.0), None, Some(300.0), None]),
            [290, 70, 760, 420]);
        assert_eq!(super::live_client_rect(window, client, 1.0, [None; 4]), window);
    }

    use super::*;

    #[test]
    fn options_default_to_a_small_centred_window_with_os_chrome() {
        let o = parse_options("");
        assert_eq!(o.title, "vue-native");
        assert_eq!((o.width, o.height), (420.0, 320.0));
        assert!(o.x.is_none() && o.y.is_none());
        // A pop-out with no `chrome` prop gets a REAL caption: movable,
        // resizable and closable with no drag region at all (D-M10).
        assert!(o.os_chrome);
        assert!(o.resizable);
        assert!(!o.always_on_top);
        assert!(o.backdrop.is_none());
        // The departure from gpui's default, asserted so it cannot drift back.
        assert!(!o.activate);
        // §7.9's options are all off: an ordinary pop-out is shown, framed,
        // on the taskbar, and its images follow the residency rules.
        assert!(o.visible);
        assert!(!o.cover);
        assert!(o.taskbar);
        assert!(!o.full_resolution);
        // §7.11's too: an ordinary pop-out takes the mouse, can be activated,
        // is captured, and covers nothing.
        assert!(!o.ignore_mouse_events);
        assert!(o.focusable);
        assert!(!o.content_protection);
        assert!(o.monitor.is_none());
    }

    #[test]
    fn an_overlay_is_click_through_unfocusable_and_left_out_of_capture() {
        let o = parse_options(
            r#"{"ignoreMouseEvents":true,"focusable":false,"contentProtection":true,
                "cover":"monitor","monitor":1,"alwaysOnTop":true}"#,
        );
        assert!(o.ignore_mouse_events);
        assert!(!o.focusable);
        assert!(o.content_protection);
        assert!(o.cover);
        assert_eq!(o.monitor, Some(1));
        assert!(o.always_on_top);
        // A negative index is no index: the main window's monitor.
        assert!(parse_options(r#"{"monitor":-1}"#).monitor.is_none());
    }

    #[test]
    fn a_viewer_is_a_hidden_owned_cover_window_at_full_resolution() {
        let o = parse_options(
            r#"{"visible":false,"cover":"monitor","taskbar":false,"fullResolution":true,"activate":true}"#,
        );
        assert!(!o.visible);
        assert!(o.cover);
        assert!(!o.taskbar);
        assert!(o.full_resolution);
        assert!(o.activate);
    }

    #[test]
    fn an_unknown_cover_is_no_cover() {
        // Advised; the window takes its `bounds` instead of covering anything.
        assert!(!parse_options(r#"{"cover":"screen"}"#).cover);
        assert!(!parse_options(r#"{"cover":"none"}"#).cover);
        assert!(!parse_options(r#"{"cover":""}"#).cover);
    }

    #[test]
    fn options_are_read_off_the_json() {
        let o = parse_options(
            r#"{"title":"Palette","x":100,"y":200,"width":300,"height":150,
                "chrome":"hidden","alwaysOnTop":true,"resizable":false,
                "minWidth":120,"minHeight":90,"backdrop":"acrylic"}"#,
        );
        assert_eq!(o.title, "Palette");
        assert_eq!(o.x, Some(100.0));
        assert_eq!(o.y, Some(200.0));
        assert_eq!((o.width, o.height), (300.0, 150.0));
        assert!(!o.os_chrome);
        assert!(o.always_on_top);
        assert!(!o.resizable);
        assert_eq!(o.min_width, Some(120.0));
        assert_eq!(o.min_height, Some(90.0));
        assert_eq!(o.backdrop, Some(Backdrop::Acrylic));
    }

    #[test]
    fn an_unknown_chrome_value_is_the_default_one_rather_than_nothing() {
        // `chrome` is a two-value enum; anything that is not the string
        // "hidden" means the OS draws the caption, which is the mode that
        // cannot leave a window unmovable.
        assert!(parse_options(r#"{"chrome":"wat"}"#).os_chrome);
        assert!(parse_options(r#"{"chrome":"default"}"#).os_chrome);
    }
}
