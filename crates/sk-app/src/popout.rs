//! One Vue subtree painted by a second OS window.
//!
//! One `JsRuntime`, one Vue app, one retained tree: `__sk_window_open` marks a node popped out
//! (it stops generating a box in the primary window) and opens a gpui window whose root view
//! lowers the same node via `sk_tree::build_subtree`.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use gpui::{
    point, prelude::*, px, size, App, Bounds, Context, Render, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowHandle, WindowId, WindowOptions,
};
use sk_runtime::redjs::api::Value;
use sk_runtime::redjs::webapi::util::read_string;
use sk_runtime::{host_fn, with_app, JsRuntime};

use crate::json::{self, Json};
use crate::popout_win;
use crate::window::Backdrop;

#[cfg(not(target_os = "windows"))]
#[path = "popout_posix.rs"]
mod posix;

const UNDEF: u64 = 0xa;

pub struct PopoutShell {
    node: u32,
}

impl Render for PopoutShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = window.window_handle().window_id().as_u64();
        sk_tree::window_frame_tick(window, &mut **cx);
        // Before the env: an env for an unregistered painted root would mark nothing for re-cascade.
        sk_tree::windows::set_painted_root(id, self.node);
        publish_env(id, self.node, window, cx);
        report_metrics(id, self.node, window);
        frame_drawn(id, window, cx);
        let backdrop = POPOUTS
            .with(|p| {
                p.borrow()
                    .iter()
                    .find(|w| w.id.as_u64() == id)
                    .map(|w| w.backdrop)
            })
            .unwrap_or_else(crate::window::backdrop);
        let shell = gpui::div()
            .relative()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont");
        #[cfg(not(target_os = "windows"))]
        let shell = crate::chrome_posix::wrap(shell, window, false);
        let shell =
            match crate::window::opaque_fill(backdrop, crate::window::effective_dark(window)) {
                Some(fill) => shell.bg(fill),
                None => shell,
            };
        shell
            .child(sk_tree::build_subtree(self.node, window, cx))
            .children(popout_inspector_overlay(id, self.node))
    }
}

fn popout_inspector_overlay(window: u64, root: u32) -> Option<gpui::Div> {
    #[cfg(sk_devtools)]
    {
        crate::devtools::popout_highlight_overlay(window, root)
    }
    #[cfg(not(sk_devtools))]
    {
        let _ = (window, root);
        None
    }
}

fn publish_env(id: u64, node: u32, window: &Window, cx: &App) {
    let Some(p) = POPOUTS.with(|p| {
        p.borrow()
            .iter()
            .find(|w| w.id.as_u64() == id)
            .map(|w| (w.os_chrome, w.title_bar_height))
    }) else {
        return;
    };
    // Before the first painted frame the window's client size stands in.
    let viewport = match sk_tree::bounds_in(node, node) {
        Some((_, _, w, h)) if w > 0.0 && h > 0.0 => (w, h),
        _ => {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        }
    };
    let env = crate::window::popout_env(window, cx, viewport, p.0, p.1);
    sk_tree::set_env_for(id, env);
}

struct Popout {
    node: u32,
    id: WindowId,
    title: String,
    handle: WindowHandle<PopoutShell>,
    /// GC-protected JS callback `(nodeId)`; separate because `close` fires after the window is gone.
    on_close: u64,
    /// GC-protected JS callback `(kind, detailJson)` for `open`, `focus`, `blur`, `resize`.
    on_event: u64,
    /// The OS draws the caption (`chrome: "default"`).
    os_chrome: bool,
    title_bar_height: f32,
    backdrop: Backdrop,
    /// Creation-time: changing one only advises.
    resizable: bool,
    min_size: (Option<f32>, Option<f32>),
    /// Last `(width, height)` reported to JS, so `resize` fires on change.
    last_metrics: (f32, f32),
    last_focused: bool,
    fullscreen: bool,
    /// 0 = none.
    hwnd: isize,
    /// A hidden pop-out still paints its subtree and is shown again rather than reopened.
    visible: bool,
    cover: bool,
    taskbar: bool,
    focusable: bool,
    /// Never true for a non-`focusable` window.
    activate: bool,
    always_on_top: bool,
    ignore_mouse_events: bool,
    content_protection: bool,
    /// `EnumDisplayMonitors` index a cover window covers; `None` = the main window's.
    monitor: Option<usize>,
    /// `(x, y, width, height)` in CSS px a window created hidden takes on its first show; `None` once placed and for cover windows.
    first_place: Option<(Option<f32>, Option<f32>, f32, f32)>,
    /// Last declarative geometry; unrelated prop updates preserve user placement.
    requested_bounds: [Option<f32>; 4],
    show_asked: Option<Instant>,
    show_drawn: Option<Instant>,
    last_show_ms: f64,
    /// Bumped by every show and hide, so a deferred tidy-up can tell the window was shown since.
    epoch: u64,
    /// As of its latest frame.
    scale: f32,
    /// Client area in CSS px, as of its latest frame.
    viewport: (f32, f32),
}

thread_local! {
    static POPOUTS: RefCell<Vec<Popout>> = const { RefCell::new(Vec::new()) };
    static CLOSED_SUB: RefCell<Option<Subscription>> = const { RefCell::new(None) };
    static PAINT_ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks every pop-out dirty after a JS wave and paints the visible ones from a task.
///
/// `Window::refresh()` through the handle, not `Entity::update` + `cx.notify()`: this runs inside
/// an input listener (itself an `Entity<RootView>::update`), so notifying would re-enter that
/// borrow and panic. A window already being updated just fails, and its listener refreshes it.
///
/// The paint task exists because a refresh only waits for `WM_PAINT`, which Windows hands out one
/// window at a time; under load a pop-out whose only change was a `<canvas>` went seconds without
/// a frame. One task at a time.
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
            p.borrow()
                .iter()
                .map(|w| w.hwnd)
                .filter(|&hwnd| hwnd != 0 && popout_win::is_visible(hwnd))
                .collect()
        });
        for hwnd in shown {
            popout_win::paint_now(hwnd);
        }
    })
    .detach();
}

/// A pop-out outliving the primary window (which owns the JS pump) would be frozen.
pub fn close_all(cx: &mut App) {
    let handles: Vec<WindowHandle<PopoutShell>> =
        POPOUTS.with(|p| p.borrow().iter().map(|w| w.handle).collect());
    for handle in handles {
        crate::dialogs::remove_window(handle.into(), cx);
    }
}

pub fn count() -> usize {
    POPOUTS.with(|p| p.borrow().len())
}

pub fn ids() -> Vec<sk_tree::WinKey> {
    POPOUTS.with(|p| p.borrow().iter().map(|w| w.id.as_u64()).collect())
}

struct Options {
    bounds: [Option<f32>; 4],
    title: String,
    x: Option<f32>,
    y: Option<f32>,
    width: f32,
    height: f32,
    os_chrome: bool,
    /// `None` = inherit the app's own backdrop.
    backdrop: Option<Backdrop>,
    always_on_top: bool,
    resizable: bool,
    min_width: Option<f32>,
    min_height: Option<f32>,
    title_bar_height: f32,
    /// Defaults to false, unlike gpui's `focus: true`: a palette that grabs focus costs the user
    /// their caret. False maps to `SW_SHOWNOACTIVATE`.
    activate: bool,
    /// Live. `false` creates the window hidden (warm) and hides an open one.
    visible: bool,
    /// Covers the whole monitor, taskbar included, with no frame. Creation-time.
    cover: bool,
    /// `false`: no taskbar button, no Alt-Tab entry. Creation-time.
    taskbar: bool,
    /// Images it shows keep their full decode. Live.
    full_resolution: bool,
    /// Live.
    ignore_mouse_events: bool,
    /// `false`: never activated, no taskbar button, unknown to the shell. Creation-time.
    focusable: bool,
    /// Live.
    content_protection: bool,
    /// The monitor a `cover` window covers, by index. Live.
    monitor: Option<usize>,
}

fn parse_options(raw: &str) -> Options {
    let j = json::parse(raw);
    let num = |k: &str| j.as_ref().and_then(|j| j.get(k)).and_then(|v| v.as_f64());
    let text = |k: &str| {
        j.as_ref()
            .and_then(|j| j.get(k))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let flag = |k: &str| j.as_ref().and_then(|j| j.get(k)).and_then(|v| v.as_bool());
    let backdrop = text("backdrop").and_then(|name| match Backdrop::parse(&name) {
        Some(b) => Some(b),
        None => {
            crate::window::advise(&format!(
                "<SkWindow backdrop=\"{name}\">: unknown backdrop — the pop-out inherits the \
                 app's own"
            ));
            None
        }
    });
    let bounds = ["x", "y", "width", "height"].map(|key| {
        num(key)
            .map(|n| n as f32)
            .filter(|n| n.is_finite() && (matches!(key, "x" | "y") || *n > 0.0))
    });
    Options {
        bounds,
        title: text("title").unwrap_or_else(|| "Skathi".to_string()),
        x: num("x").map(|n| n as f32),
        y: num("y").map(|n| n as f32),
        width: num("width").unwrap_or(420.0) as f32,
        height: num("height").unwrap_or(320.0) as f32,
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
                    "<SkWindow cover=\"{other}\">: unknown cover — expected \"monitor\". The window \
                     takes its `bounds` instead"
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

fn cover_rect(main: Option<isize>, monitor: Option<usize>) -> Option<popout_win::Rect> {
    if let Some(i) = monitor {
        if let Some(r) = popout_win::monitor_rect_at(i) {
            return Some(r);
        }
        crate::window::advise(&format!(
            "<SkWindow :monitor=\"{i}\">: there is no monitor {i} — the window covers the main \
             window's monitor instead"
        ));
    }
    main.and_then(popout_win::monitor_rect)
}

// Deferred: `App::open_window` draws the window before returning, and this runs inside the primary
// window's update. Hence JS learns the window id through the `open` event, not a return value.
host_fn!(fn sk_window_open(g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let on_close = args.arg(2);
    let on_event = args.arg(3);
    if node == 0 || !sk_tree::with_tree(|t| t.node(node).is_some()) {
        crate::window::advise(&format!(
            "__sk_window_open: node {node} is not in the tree — nothing was opened"
        ));
        return UNDEF;
    }
    if popout_of_node(node).is_some() {
        crate::window::advise(&format!(
            "__sk_window_open: node {node} is already painted by another window — the \
             second call was refused. A node id may be painted by exactly one window, because \
             every node-keyed side map (focus, scroll, input, media) would otherwise alias"
        ));
        return UNDEF;
    }
    if let Some(outer) = enclosing_popout(node) {
        crate::window::advise(&format!(
            "<SkWindow> (node {node}) is nested inside another one (node {outer}) — it \
             was flattened to a SIBLING window. Skathi has no owner/child window \
             relationship, and a nested pop-out reads as a modality request."
        ));
    }
    let protect = |v: sk_runtime::redjs::api::Value| -> u64 {
        if v.as_number().is_some() || v.0 == UNDEF || v.0 == 0 {
            0
        } else {
            unsafe { sk_runtime::redjs::ffi::redjs_protect(g, v.0) };
            v.0
        }
    };
    let on_close = protect(on_close);
    let on_event = protect(on_event);
    let opts = parse_options(&raw);
    // Not in the deferred open: the subtree would sit in two element trees for a frame (gpui scroll/hitbox state is keyed by `ElementId`).
    sk_tree::set_popped_out(node, true);
    with_app(|cx| {
        cx.defer(move |cx| open_now(node, opts, on_close, on_event, cx));
    });
    UNDEF
});

host_fn!(fn sk_window_close_popout(_g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let Some(handle) = popout_of_node(node).map(|i| POPOUTS.with(|p| p.borrow()[i].handle)) else {
        return UNDEF;
    };
    with_app(|cx| {
        cx.defer(move |cx| {
            // Untyped, as in `sk_window_click`.
            crate::dialogs::remove_window(handle.into(), cx);
        });
    });
    UNDEF
});

// The live half of `<SkWindow>`'s props. A changed creation-time prop advises and does nothing.
host_fn!(fn sk_window_set_options(g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let Some(i) = popout_of_node(node) else { return UNDEF };
    let (handle, old_title, old_chrome, old_visible, old_cover, old_taskbar, old_focusable) = POPOUTS.with(|p| {
        let p = p.borrow();
        let w = &p[i];
        (w.handle, w.title.clone(), w.os_chrome, w.visible, w.cover, w.taskbar, w.focusable)
    });
    let (old_resizable, old_min_size, old_backdrop, old_title_bar_height) = POPOUTS.with(|p| {
        let w = &p.borrow()[i];
        (w.resizable, w.min_size, w.backdrop, w.title_bar_height)
    });
    let opts = parse_options(&raw);
    if opts.resizable != old_resizable {
        crate::window::advise(
            "<SkWindow resizable>: whether a window can be resized is decided when it is created \
             (a window STYLE and its hit test), so changing it does nothing to a window that is \
             already open — close and re-open it, for example with a `:key` that includes the value",
        );
    }
    if (opts.min_width, opts.min_height) != old_min_size {
        crate::window::advise(
            "<SkWindow minWidth/minHeight>: the minimum size is decided when the window is created, \
             so changing it does nothing to a window that is already open — close and re-open it, \
             for example with a `:key` that includes the value",
        );
    }
    let backdrop = opts
        .backdrop
        .unwrap_or_else(|| crate::window::config().backdrop);
    let backdrop_moved = backdrop != old_backdrop;
    let repaint = backdrop_moved || opts.title_bar_height != old_title_bar_height;
    if opts.focusable != old_focusable {
        crate::window::advise(
            "<SkWindow focusable>: whether a window can be activated is decided when it is created \
             (the taskbar and the shell read its styles when it is first shown), so changing it \
             does nothing to a window that is already open — close and re-open it",
        );
    }
    if opts.os_chrome != old_chrome {
        crate::window::advise(
            "<SkWindow chrome>: the caption mode is decided when the window is created (a real \
             `WS_CAPTION` is a window STYLE), so changing it does nothing to a window that is \
             already open — close and re-open it. Same rule as `titleBarStyle`",
        );
    }
    if opts.cover != old_cover {
        crate::window::advise(
            "<SkWindow cover>: covering the monitor is decided when the window is created (it \
             takes the frame off the window), so changing it does nothing to a window that is \
             already open — close and re-open it",
        );
    }
    if opts.taskbar != old_taskbar {
        crate::window::advise(
            "<SkWindow taskbar>: the taskbar reads a window's owner and styles when it is shown, \
             so this is decided when the window is created and changing it does nothing to a \
             window that is already open — close and re-open it",
        );
    }
    // The next residency sweep reads it.
    sk_tree::windows::set_full_res_images(handle.window_id().as_u64(), opts.full_resolution);
    let title_moved = opts.title != old_title;
    if title_moved {
        POPOUTS.with(|p| p.borrow_mut()[i].title = opts.title.clone());
    }
    // Recorded now so a second change in the same wave compares against it.
    let visible = opts.visible;
    let visible_moved = visible != old_visible;
    if visible_moved {
        POPOUTS.with(|p| p.borrow_mut()[i].visible = visible);
    }
    let always_on_top = opts.always_on_top;
    // Recorded now, applied deferred: a style change re-frames the window, and gpui hears of it through its window proc.
    let (hwnd, click_through, protection, monitor_moved, bounds_moved) = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = &mut p[i];
        w.always_on_top = always_on_top;
        w.backdrop = backdrop;
        w.title_bar_height = opts.title_bar_height;
        w.activate = opts.activate && w.focusable;
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
                if backdrop_moved {
                    window.set_background_appearance(backdrop.to_gpui());
                }
                // Its next frame re-publishes the env (`env(titlebar-area-height)`).
                if repaint {
                    window.refresh();
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
                    cx.spawn(async move |_| {
                        popout_win::set_rect(hwnd, r);
                        popout_win::paint_now(hwnd);
                    })
                    .detach();
                }
            }
            if let Some(bounds) = bounds_request {
                // Outside App/Window borrows: SetWindowPos may synchronously send WM_SIZE/WM_DPICHANGED.
                cx.spawn(async move |_| {
                    if let (Some(window), Some(client)) =
                        (popout_win::window_rect(hwnd), popout_win::client_rect_on_screen(hwnd))
                    {
                        let rect = live_client_rect(window, client, crate::window::primary_scale(), bounds);
                        let _ = popout_win::set_rect(hwnd, rect);
                    }
                }).detach();
            }
            #[cfg(not(target_os = "windows"))]
            posix::apply_live(
                handle,
                posix::Live { always_on_top, click_through, protection, bounds: bounds_request },
                cx,
            );
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

/// A `--window-control` caption button's action on pop-out `key`, as its real press performs it.
pub(crate) fn window_control(key: sk_tree::WinKey, area: gpui::WindowControlArea) -> bool {
    let Some(handle) = POPOUTS.with(|p| {
        p.borrow()
            .iter()
            .find(|w| w.id.as_u64() == key)
            .map(|w| w.handle)
    }) else {
        return false;
    };
    let acts = matches!(
        area,
        gpui::WindowControlArea::Min
            | gpui::WindowControlArea::Max
            | gpui::WindowControlArea::Close
    );
    if acts {
        with_app(|cx| {
            cx.defer(move |cx| {
                if area == gpui::WindowControlArea::Close {
                    crate::dialogs::remove_window(handle.into(), cx);
                    return;
                }
                let _ = gpui::AnyWindowHandle::from(handle).update(cx, |_, window, _| {
                    if area == gpui::WindowControlArea::Min {
                        window.minimize_window();
                    } else if window.is_resizable() {
                        window.vn_set_maximized(!window.vn_is_zoomed());
                    }
                });
            });
        });
    }
    acts
}

/// gpui has no per-window API, so this goes through the cached HWND.
fn set_always_on_top(handle: WindowHandle<PopoutShell>, on: bool) {
    let key = handle.window_id().as_u64();
    let Some(hwnd) = crate::window::hwnd_of(key) else {
        return;
    };
    crate::window::set_always_on_top_hwnd(hwnd, on);
}

// Synthesizes a mouse click through the pop-out's own `Window::dispatch_event` at client
// coordinates, for specs that may not move the real cursor.
host_fn!(fn sk_window_click(_g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let x = args.arg(1).as_number().unwrap_or(0.0) as f32;
    let y = args.arg(2).as_number().unwrap_or(0.0) as f32;
    let Some(handle) = popout_of_node(node).map(|i| POPOUTS.with(|p| p.borrow()[i].handle)) else {
        return Value::boolean(false).0;
    };
    let position = gpui::Point::new(px(x), px(y));
    let done = with_app(|cx| {
        // `AnyWindowHandle::update`, not the typed handle: that one wraps the body in
        // `Entity<PopoutShell>::update`, and the click listener is another, so a hit would panic.
        gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| {
            // gpui hit-tests from the window's own `mouse_position`.
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

host_fn!(fn sk_window_popouts(_g, _args) {
    let json = POPOUTS.with(|p| {
        Json::arr(
            p.borrow()
                .iter()
                .map(|w| {
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

// Maps a node as the primary window painted it into the pop-out's CSS px, relative to its painted
// root, via screen coordinates (holds across monitors and DPIs). A cover window is measured where
// its next show puts it, so the answer is right while hidden.
host_fn!(fn sk_window_map_rect(_g, args) {
    let node = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let src = args.arg(1).as_number().unwrap_or(0.0) as u32;
    let Some(i) = popout_of_node(node) else { return UNDEF };
    let (hwnd, cover, scale, monitor) = POPOUTS.with(|p| {
        let p = p.borrow();
        (p[i].hwnd, p[i].cover, p[i].scale, p[i].monitor)
    });
    let Some(rect) = sk_tree::bounds_in(0, src) else { return UNDEF };
    let Some(main) = crate::window::primary_hwnd() else { return UNDEF };
    let Some(from) = popout_win::client_rect_on_screen(main) else { return UNDEF };
    let main_scale = crate::window::primary_scale();
    let to = if cover {
        // Another monitor uses the scale of its own latest frame.
        let at = monitor.and_then(popout_win::monitor_rect_at);
        let s = if at.is_some() { scale } else { main_scale };
        at.or_else(|| popout_win::monitor_rect(main)).map(|r| (r, s))
    } else {
        popout_win::client_rect_on_screen(hwnd).map(|r| (r, scale))
    };
    let Some((to, to_scale)) = to else { return UNDEF };
    let root = sk_tree::bounds_in(node, node).map_or((0.0, 0.0), |r| (r.0, r.1));
    let (x, y, w, h) =
        popout_win::map_rect(rect, (from[0], from[1]), main_scale, (to[0], to[1]), to_scale);
    let json = Json::arr(vec![Json::n(x - root.0), Json::n(y - root.1), Json::n(w), Json::n(h)]).to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

/// `0` addresses the primary window; any other id must be an existing SkWindow root (a missing
/// owner never falls back to the primary).
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

host_fn!(fn sk_fullscreen_window(_g, args) {
    let owner = args.arg(0).as_number().unwrap_or(0.0) as u32;
    Value::boolean(fullscreen_window(owner, args.arg(1).as_bool())).0
});

fn popout_of_node(node: u32) -> Option<usize> {
    POPOUTS.with(|p| p.borrow().iter().position(|w| w.node == node))
}

fn enclosing_popout(node: u32) -> Option<u32> {
    sk_tree::with_tree(|t| {
        let mut cursor = t.node(node)?.parent;
        for _ in 0..1024 {
            if cursor == 0 {
                return None;
            }
            if sk_tree::popped_out(cursor) {
                return Some(cursor);
            }
            cursor = t.node(cursor)?.parent;
        }
        None
    })
}

/// [`emit`] with the app published, so a handler that changes a prop reaches `__sk_window_set_options`.
fn emit_in(cx: &mut App, on_event: u64, kind: &str, detail: Json) {
    if on_event == 0 {
        return;
    }
    sk_runtime::with_published_app(cx, || emit(on_event, kind, detail));
}

fn emit(on_event: u64, kind: &str, detail: Json) {
    if on_event == 0 {
        return;
    }
    // Settle an input dispatch's owed checkpoint first, as every publication does.
    sk_runtime::bridge::settle_owed_drain();
    JsRuntime::with(|rt| {
        let args = [rt.ctx().string(kind), rt.ctx().string(&detail.to_json())];
        if let Err(sk_runtime::redjs::api::Error::Exception(e)) =
            rt.ctx().call(Value(on_event), Value::undefined(), &args)
        {
            let err = rt.describe(e);
            eprintln!("[SkWindow event] {}\n{}", err.header, err.stack);
        }
        // Vue flushes patches on microtasks; drain so the handler's mutations land in this wave.
        rt.ctx().drain_microtasks();
    });
}

/// `window.innerWidth` stays the primary window's (one JS realm), so a pop-out's own size reaches
/// the app via `<SkWindow @resize>`.
fn report_metrics(id: u64, node: u32, window: &Window) {
    let viewport = match sk_tree::bounds_in(node, node) {
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
        emit(
            on_event,
            if focused { "focus" } else { "blur" },
            Json::obj(vec![]),
        );
    }
}

/// `(window id, painted node, title)` for every pop-out, the primary excluded.
#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn list() -> Vec<(u64, u32, String)> {
    POPOUTS.with(|p| {
        p.borrow()
            .iter()
            .map(|w| (w.id.as_u64(), w.node, w.title.clone()))
            .collect()
    })
}

/// Goes through `AnyWindowHandle::update` for the reason given at `sk_window_click`.
#[cfg_attr(not(sk_devtools), allow(dead_code))]
pub fn with_window<R>(
    id: u64,
    cx: &mut App,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> Option<R> {
    let handle = POPOUTS.with(|p| {
        p.borrow()
            .iter()
            .find(|w| w.id.as_u64() == id)
            .map(|w| w.handle)
    })?;
    gpui::AnyWindowHandle::from(handle)
        .update(cx, |_, window, cx| f(window, cx))
        .ok()
}

fn open_now(node: u32, opts: Options, on_close: u64, on_event: u64, cx: &mut App) {
    // The node may have been unmounted between the call and this defer.
    if !sk_tree::with_tree(|t| t.node(node).is_some()) {
        sk_tree::set_popped_out(node, false);
        return;
    }
    let opened_at = Instant::now();
    let (bounds, display_id) = match (opts.x, opts.y) {
        (Some(x), Some(y)) => placed_bounds(x, y, opts.width, opts.height, cx),
        // Centred on the main window's monitor, as a window created hidden is.
        _ => {
            let main = crate::main_window_display(cx);
            (
                Bounds::centered(main, size(px(opts.width), px(opts.height)), cx),
                main,
            )
        }
    };
    if opts.activate && !opts.focusable {
        crate::window::advise(
            "<SkWindow activate :focusable=\"false\">: a window that cannot be activated does not \
             take the foreground when it is shown either — `activate` is ignored",
        );
    }
    let activate = opts.activate && opts.focusable;
    // Created hidden: its styles change before the first show, and the shell reads them then.
    let hidden_first = opts.cover || !opts.focusable;
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_background: opts
                .backdrop
                .unwrap_or_else(|| crate::window::config().backdrop)
                .to_gpui(),
            titlebar: Some(TitlebarOptions {
                title: Some(opts.title.clone().into()),
                // `"hidden"` hands the surface to the app, which must paint a `window-drag: move` region.
                appears_transparent: !opts.os_chrome,
                traffic_light_position: crate::window::traffic_light_position(
                    !opts.os_chrome,
                    opts.title_bar_height,
                ),
                ..Default::default()
            }),
            is_resizable: opts.resizable,
            focus: activate && !crate::quiet::never_focus(),
            window_min_size: match (opts.min_width, opts.min_height) {
                (None, None) => None,
                (w, h) => Some(size(px(w.unwrap_or(0.0)), px(h.unwrap_or(0.0)))),
            },
            inactive_frame_interval: None,
            show: opts.visible && !hidden_first,
            is_minimizable: !opts.cover,
            display_id,
            app_owns_titlebar_drag: !opts.os_chrome,
            ..Default::default()
        },
        |window, cx| {
            if std::env::var_os("SK_RETAINED_LAYOUT").as_deref() != Some(std::ffi::OsStr::new("0"))
            {
                window.vn_enable_retained_layout();
            }
            crate::quiet::send_to_back(window);
            cx.new(|_| PopoutShell { node })
        },
    );
    let Ok(handle) = opened else {
        sk_tree::set_popped_out(node, false);
        crate::window::advise("__sk_window_open: gpui refused to open the window");
        return;
    };
    let key = handle.window_id().as_u64();
    // Before its first build: `with_control_area` reads it to decide whether `window-drag` is live.
    sk_tree::set_os_chrome_in(key, opts.os_chrome);
    sk_tree::windows::set_full_res_images(key, opts.full_resolution);
    let mut hwnd = 0;
    let mut first_frame = (1.0, (0.0, 0.0));
    let _ = gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| {
        hwnd = crate::window::capture_hwnd(window).unwrap_or(0);
        let client = window.viewport_size();
        first_frame = (
            window.scale_factor(),
            (f32::from(client.width), f32::from(client.height)),
        );
        // The OS keeps an owner disabled under its modal dialog, so the X closes the dialog first.
        window.on_window_should_close(cx, |window, cx| {
            crate::dialogs::may_close_now(window.window_handle(), cx)
        });
    });
    // While still hidden: the taskbar reads styles on show, and a cover window's first frame must already be monitor-sized.
    if hwnd != 0 {
        if !opts.taskbar {
            if let Some(main) = crate::window::primary_hwnd() {
                popout_win::make_owned(hwnd, main);
            }
        }
        if opts.cover {
            popout_win::make_cover(hwnd);
            // A hidden monitor-sized swapchain costs 30 MB at 1440p, 66 MB at 4K; the first show sizes it.
            cx.spawn(async move |_| warm_small(hwnd)).detach();
        }
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
    #[cfg(not(target_os = "windows"))]
    posix::apply_live(
        handle,
        posix::Live {
            always_on_top: opts.always_on_top,
            click_through: opts.ignore_mouse_events.then_some(true),
            protection: opts.content_protection.then_some(true),
            bounds: None,
        },
        cx,
    );
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
            backdrop: opts
                .backdrop
                .unwrap_or_else(|| crate::window::config().backdrop),
            resizable: opts.resizable,
            min_size: (opts.min_width, opts.min_height),
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
            // gpui never applied the bounds of a window created hidden; they apply on its first show (not for cover windows).
            requested_bounds: opts.bounds,
            first_place: ((!opts.visible || hidden_first) && !opts.cover).then_some((
                opts.x,
                opts.y,
                opts.width,
                opts.height,
            )),
            show_asked: None,
            show_drawn: None,
            last_show_ms: 0.0,
            epoch: 0,
            scale: first_frame.0,
            viewport: first_frame.1,
        })
    });
    if opts.always_on_top {
        set_always_on_top(handle, true);
    }
    ensure_close_observer(cx);
    let msg = format!(
        "[window] node {node} popped out into window {:?} — the primary window stopped painting it",
        handle.window_id()
    );
    eprintln!("{msg}");
    sk_tree::log::record("window", msg);
    emit_in(
        cx,
        on_event,
        "open",
        Json::obj(vec![
            ("windowId", Json::n(key as f64)),
            ("node", Json::n(node as f64)),
        ]),
    );
    // Nothing in the tree changed, but the primary window must redraw without the subtree.
    crate::notify_shell(cx);
    // Created hidden (see `hidden_first`); this is its first show.
    if opts.visible && hidden_first {
        show_now(handle.window_id(), cx);
    } else if opts.visible {
        // gpui showed it and drew its first frame inside `open_window`; unless `@open` hid it.
        let id = handle.window_id();
        let still_shown = POPOUTS.with(|p| p.borrow().iter().any(|w| w.id == id && w.visible));
        if still_shown {
            report_show(cx, id, on_event, opened_at.elapsed());
        }
    }
}

/// `x`/`y` are CSS px at the main window's scale, as live `bounds` and a hidden window's first show
/// read them. gpui takes the origin in its target display's own logical px, so the display is the
/// one under the window's centre, named explicitly.
fn placed_bounds(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    cx: &App,
) -> (Bounds<gpui::Pixels>, Option<gpui::DisplayId>) {
    let size = size(px(width), px(height));
    let as_given = (
        Bounds {
            origin: point(px(x), px(y)),
            size,
        },
        None,
    );
    let scale = crate::window::primary_scale();
    let centre = ((x + width / 2.0) * scale, (y + height / 2.0) * scale);
    let Some((id, monitor)) =
        popout_win::monitor_at(centre.0.round() as i32, centre.1.round() as i32)
    else {
        return as_given;
    };
    let id = gpui::DisplayId::new(id);
    let Some(display) = cx.find_display(id) else {
        return as_given;
    };
    let logical_width = f32::from(display.bounds().size.width);
    if logical_width <= 0.0 {
        return as_given;
    }
    let display_scale = (monitor[2] - monitor[0]) as f32 / logical_width;
    (
        Bounds {
            origin: point(px(x * scale / display_scale), px(y * scale / display_scale)),
            size,
        },
        Some(id),
    )
}

/// Edge, physical px, of the hidden window a cover window waits as.
const WARM_SIZE: i32 = 64;
/// How long a hidden cover window keeps its monitor-sized swapchain before going back to [`WARM_SIZE`].
const SHRINK_AFTER: Duration = Duration::from_secs(30);
/// After a hide the window draws once to hand back its atlas tiles.
const TIDY_AFTER: Duration = Duration::from_millis(500);

/// The frame is drawn before the show: a hidden window gets no `WM_PAINT`, so DWM would otherwise
/// show its previous picture until the next vsync. A cover window is placed on its monitor first so
/// that frame is already monitor-sized.
fn show_now(id: WindowId, cx: &mut App) {
    let Some((handle, hwnd, cover, activate, first, monitor, on_top)) = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = p.iter_mut().find(|w| w.id == id)?;
        w.epoch += 1;
        w.show_asked = Some(Instant::now());
        w.show_drawn = None;
        Some((
            w.handle,
            w.hwnd,
            w.cover,
            w.activate,
            w.first_place.take(),
            w.monitor,
            w.always_on_top,
        ))
    }) else {
        return;
    };
    if hwnd == 0 {
        #[cfg(not(target_os = "windows"))]
        posix::show(
            id,
            handle,
            cover,
            activate && !crate::quiet::never_focus(),
            on_top,
            cx,
        );
        return;
    }
    let main = crate::window::primary_hwnd();
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
        // Whatever became topmost while it was hidden would stay above it.
        if on_top {
            crate::window::set_always_on_top_hwnd(hwnd, true);
        }
        cx.update(|cx| shown(id, cx));
    })
    .detach();
}

/// Every resize of a pop-out runs outside any `App` update: gpui hears of a new size through
/// `WM_SIZE` and a callback that needs the app; inside an update that callback is lost and the
/// swapchain follows the new size while layout stays at the old one.
fn warm_small(hwnd: isize) {
    if let Some(r) = popout_win::window_rect(hwnd) {
        popout_win::set_rect(hwnd, [r[0], r[1], r[0] + WARM_SIZE, r[1] + WARM_SIZE]);
    }
}

/// If the first frame was not drawn before the show, [`frame_drawn`] reports it later.
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

fn report_show(cx: &mut App, id: WindowId, on_event: u64, took: Duration) {
    let ms = took.as_secs_f64() * 1000.0;
    let viewport = POPOUTS.with(|p| {
        let mut p = p.borrow_mut();
        let w = p.iter_mut().find(|w| w.id == id)?;
        w.show_asked = None;
        w.last_show_ms = ms;
        Some((w.viewport, w.scale))
    });
    let Some(((width, height), scale)) = viewport else {
        return;
    };
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

/// Called from the pop-out's own render; also records the scale [`sk_window_map_rect`] needs.
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
        // Drawn while hidden (usual): `shown` reports instead.
        #[cfg(target_os = "windows")]
        let visible = popout_win::is_visible(w.hwnd);
        #[cfg(not(target_os = "windows"))]
        let visible = window.vn_is_shown();
        visible.then_some((w.id, w.on_event, now.duration_since(asked)))
    });
    if let Some((wid, on_event, took)) = report {
        cx.defer(move |cx| report_show(cx, wid, on_event, took));
    }
}

/// Activation returns to the main window if this one had it. Unless shown again first, a hidden
/// window gets no frames, so after [`TIDY_AFTER`] it draws once to return its atlas tiles, and
/// after [`SHRINK_AFTER`] a cover window goes back to [`WARM_SIZE`].
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
        #[cfg(not(target_os = "windows"))]
        if posix::hide(id, cx) {
            emit_in(cx, on_event, "hide", Json::obj(vec![]));
        }
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
        cx.background_executor()
            .timer(SHRINK_AFTER - TIDY_AFTER)
            .await;
        if cx.update(|cx| still_hidden(id, epoch, cx)) {
            warm_small(hwnd);
            popout_win::paint_now(hwnd);
            sk_tree::log::record(
                "window",
                format!("[window] the hidden cover window {id:?} went back to its warm size"),
            );
        }
    })
    .detach();
}

/// Whether the hide `epoch` is still the latest event for the window; if so it is made dirty for
/// the caller's draw outside this update.
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
    gpui::AnyWindowHandle::from(handle)
        .update(cx, |_, window, _| window.refresh())
        .is_ok()
}

fn live_client_rect(
    window: popout_win::Rect,
    client: popout_win::Rect,
    scale: f32,
    bounds: [Option<f32>; 4],
) -> popout_win::Rect {
    let border = [
        client[0] - window[0],
        client[1] - window[1],
        window[2] - client[2],
        window[3] - client[3],
    ];
    let x = bounds[0].map_or(client[0], |n| (n * scale).round() as i32);
    let y = bounds[1].map_or(client[1], |n| (n * scale).round() as i32);
    let width = bounds[2].map_or(client[2] - client[0], |n| {
        (n * scale).round().max(1.0) as i32
    });
    let height = bounds[3].map_or(client[3] - client[1], |n| {
        (n * scale).round().max(1.0) as i32
    });
    [
        x - border[0],
        y - border[1],
        x + width + border[2],
        y + height + border[3],
    ]
}

/// First-show rect of a window created hidden without `cover`: client size at `x`/`y` or centred
/// on the main window's monitor, CSS px at the main window's scale.
fn first_rect(
    hwnd: isize,
    main: Option<isize>,
    (x, y, w, h): (Option<f32>, Option<f32>, f32, f32),
) -> Option<popout_win::Rect> {
    let scale = crate::window::primary_scale();
    let (win, client) = (
        popout_win::window_rect(hwnd)?,
        popout_win::client_rect_on_screen(hwnd)?,
    );
    let border = [
        client[0] - win[0],
        client[1] - win[1],
        win[2] - client[2],
        win[3] - client[3],
    ];
    let (cw, ch) = ((w * scale).round() as i32, (h * scale).round() as i32);
    let monitor = main
        .and_then(popout_win::monitor_rect)
        .unwrap_or([0, 0, cw, ch]);
    let left = x.map_or(monitor[0] + (monitor[2] - monitor[0] - cw) / 2, |x| {
        (x * scale).round() as i32
    });
    let top = y.map_or(monitor[1] + (monitor[3] - monitor[1] - ch) / 2, |y| {
        (y * scale).round() as i32
    });
    Some([
        left - border[0],
        top - border[1],
        left + cw + border[2],
        top + ch + border[3],
    ])
}

fn ensure_close_observer(cx: &mut App) {
    if CLOSED_SUB.with(|s| s.borrow().is_some()) {
        return;
    }
    let sub = cx.on_window_closed(|cx, id| on_window_closed(id, cx));
    CLOSED_SUB.with(|s| *s.borrow_mut() = Some(sub));
}

/// The X button, `window.close()` and `__sk_window_close` all arrive here.
fn on_window_closed(id: WindowId, cx: &mut App) {
    let Some(i) = POPOUTS.with(|p| p.borrow().iter().position(|w| w.id == id)) else {
        return;
    };
    let popout = POPOUTS.with(|p| p.borrow_mut().remove(i));
    sk_tree::set_popped_out(popout.node, false);
    // Its atlas is destroyed, so owed tiles must not reach another window's `drop_image`, which could remove keys a live window re-inserted.
    sk_tree::forget_window(id.as_u64());
    crate::window::forget_window(id.as_u64());
    let msg = format!(
        "[window] the pop-out window for node {} closed — the subtree is back in the primary \
         window",
        popout.node
    );
    eprintln!("{msg}");
    sk_tree::log::record("window", msg);
    // Re-cascade: the subtree answered against the pop-out's `@media` env.
    sk_tree::with_tree(|t| t.mark_restyle_subtree(popout.node));
    crate::notify_shell(cx);
    if popout.on_close != 0 {
        sk_runtime::bridge::settle_owed_drain();
        JsRuntime::with(|rt| {
            let args = [Value::int32(popout.node as i32)];
            if let Err(sk_runtime::redjs::api::Error::Exception(e)) =
                rt.ctx()
                    .call(Value(popout.on_close), Value::undefined(), &args)
            {
                let err = rt.describe(e);
                eprintln!("[window close callback] {}\n{}", err.header, err.stack);
            }
            rt.ctx().drain_microtasks();
            // Fired once, so unrooted here; otherwise each open/close cycle leaks a function.
            // SAFETY: `on_close` was protected in `__sk_window_open` and not unprotected before.
            unsafe { sk_runtime::redjs::ffi::redjs_unprotect(rt.raw(), popout.on_close) };
        });
    }
    if popout.on_event != 0 {
        // SAFETY: as `on_close` above: protected once at open, unprotected once here.
        JsRuntime::with(|rt| unsafe {
            sk_runtime::redjs::ffi::redjs_unprotect(rt.raw(), popout.on_event)
        });
    }
}

/// Registered with sk-tree so a JS wave started inside an input listener reaches every window (the
/// listener consumed the dirty flag). Deferred: `update_window` on a window already being updated
/// fails and the refresh is lost; `cx.defer` runs once every window is back.
pub(crate) fn repaint_all(cx: &mut App) {
    cx.defer(|cx| {
        crate::refresh_main(cx);
        notify_all(cx);
    });
}

pub fn install(rt: &JsRuntime) {
    sk_tree::set_repaint_all(repaint_all);
    rt.register(
        "__sk_fullscreen_window",
        2,
        sk_fullscreen_window as *const _,
    );
    rt.register("__sk_window_open", 4, sk_window_open as *const _);
    rt.register("__sk_window_close", 1, sk_window_close_popout as *const _);
    rt.register("__sk_window_popouts", 0, sk_window_popouts as *const _);
    rt.register("__sk_window_click", 3, sk_window_click as *const _);
    rt.register(
        "__sk_window_set_options",
        2,
        sk_window_set_options as *const _,
    );
    rt.register("__sk_window_map_rect", 2, sk_window_map_rect as *const _);
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_partial_bounds_preserve_frame_and_untouched_dimensions() {
        let window = [90, 70, 530, 420];
        let client = [100, 100, 520, 410];
        assert_eq!(
            super::live_client_rect(window, client, 1.5, [Some(200.0), None, Some(300.0), None]),
            [290, 70, 760, 420]
        );
        assert_eq!(
            super::live_client_rect(window, client, 1.0, [None; 4]),
            window
        );
    }

    use super::*;

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
}
