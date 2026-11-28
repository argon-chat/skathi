//! `appWindow` and `globalThis.__SK_WINDOW__`: the window facade's verbs and state over the published
//! [`super::Snapshot`], its events, `defineWindow`, and the close request's `close-requested`/`beforeunload`.

use std::cell::Cell;
use std::ffi::c_void;

use sk_runtime::bridge::{poisoned, settle_owed_drain};
use sk_runtime::redjs::ffi::redjs_protect;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::events;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host, host_fn, with_commit, JsRuntime};

use super::{
    config, controls, display_mode, host_controls, resize_direction, set_backdrop, set_overlay,
    set_theme, snapshot, submit, titlebar_rect, web, Op, OverlayMode, Snapshot, TitleBarStyle,
    SNAPSHOT,
};

thread_local! {
    static APP_WINDOW: Cell<u64> = const { Cell::new(0) };
    /// Sticky activation gates the `beforeunload` veto, as in Chrome: an untouched app cannot refuse to close.
    static STICKY: Cell<bool> = const { Cell::new(false) };
}

fn read<R>(f: impl FnOnce(&Snapshot) -> R) -> R {
    SNAPSHOT.with(|s| f(&s.borrow()))
}

/// Runs the `onChange` subscribers: some `appWindow` state may have moved.
pub(super) fn changed() {
    if !poisoned() {
        web::changed();
    }
}

/// Re-reads the window under a live Commit, since geometry writes lag a frame; outside one the
/// published snapshot stands.
pub(super) fn refresh() {
    let Some(s) = with_commit(|window, cx| snapshot(window, cx)) else {
        return;
    };
    let moved = SNAPSHOT.with(|c| {
        let mut c = c.borrow_mut();
        if *c == s {
            return false;
        }
        *c = s;
        true
    });
    if moved {
        changed();
    }
}

unsafe fn fire(g: *mut c_void, target: u64, kind: &str) {
    unsafe { events::dispatch(g, target, events::new_trusted_event(g, kind)) };
}

unsafe fn fire_custom(g: *mut c_void, target: u64, kind: &str, fullscreen: bool) {
    unsafe {
        let detail = new_object(g, UNDEF);
        define(g, detail, "fullscreen", enc_bool(fullscreen), 0);
        let init = new_object(g, UNDEF);
        define(g, init, "detail", detail, 0);
        let ctor = get(g, glue::rjs_global(g), "CustomEvent");
        let mut ev = UNDEF;
        if glue::rjs_construct(g, ctor, [js_string(g, kind), init].as_ptr(), 2, &mut ev) == 0 {
            events::dispatch(g, target, ev);
        }
    }
}

/// The frame's window events, in `kinds` order.
pub(super) fn dispatch(kinds: &[&str]) {
    let app = APP_WINDOW.with(Cell::get);
    if kinds.is_empty() || app == 0 || poisoned() {
        return;
    }
    JsRuntime::with(|rt| unsafe {
        let g = rt.raw();
        let global = glue::rjs_global(g);
        for &kind in kinds {
            match kind {
                "boot" => {}
                "focus" | "blur" => {
                    fire(g, global, kind);
                    fire(g, app, kind);
                }
                "will-enter-fullscreen" | "will-exit-fullscreen" => {
                    fire_custom(g, app, "fullscreenwillchange", kind.contains("enter"))
                }
                "cancel-enter-fullscreen" | "cancel-exit-fullscreen" => {
                    fire_custom(g, app, "fullscreencancel", kind.contains("enter"))
                }
                "geometrychange" => web::dispatch_geometry_change(),
                _ => fire(g, app, kind),
            }
        }
        rt.ctx().drain_microtasks();
    });
}

/// A user close request (title bar, Alt+F4, taskbar): `close-requested` on `appWindow`, then
/// `beforeunload`; `false` keeps the window. `close()`, a tray quit and the session ending are not asked.
pub(super) fn close_requested() -> bool {
    let app = APP_WINDOW.with(Cell::get);
    // No JS after a caught native panic: nothing can veto a close, so the window's own X still works.
    if app == 0 || poisoned() {
        return true;
    }
    // This comes from gpui's should-close callback, outside any publication.
    settle_owed_drain();
    let close = JsRuntime::with(|rt| unsafe {
        let g = rt.raw();
        // Not gated on sticky activation: an untouched app may still keep its window, e.g. in a tray.
        let request = events::new_trusted_event_with(g, "close-requested", false, true);
        let close = if !events::dispatch(g, app, request) {
            false
        } else {
            let ev = events::new_trusted_event_with(g, "beforeunload", false, true);
            let vetoed = !events::dispatch(g, glue::rjs_global(g), ev);
            if vetoed && STICKY.with(Cell::get) {
                if !events::has_listener(g, app, "close-requested") {
                    web::warn(
                        g,
                        "beforeunload-trap",
                        "`beforeunload` cancelled the close, but Skathi shows no confirmation dialog, so the \
                         close button does nothing. Cancel `close-requested` on `appWindow` instead, ask the \
                         user, and call `appWindow.close()` when they confirm.",
                    );
                }
                false
            } else {
                true
            }
        };
        rt.ctx().drain_microtasks();
        close
    });
    if close {
        web::hide_page();
    }
    close
}

/// A close or quit that asks nothing: the tray's quit item, the embedder, the session ending.
pub(super) fn page_hide() {
    if !poisoned() {
        web::hide_page();
    }
}

// --- verbs -----------------------------------------------------------------------------------

macro_rules! verb {
    ($name:ident, $op:expr_2021) => {
        host_fn!(fn $name(_g, _args) {
            submit($op);
            UNDEF
        });
    };
}

verb!(minimize, Op::Minimize);
verb!(maximize, Op::Maximize);
verb!(unmaximize, Op::Unmaximize);
verb!(toggle_maximize, Op::ToggleMaximize);
verb!(center, Op::Center);
verb!(start_dragging, Op::StartDragging);

host_fn!(fn start_resize_dragging(g, args) {
    let direction = args.arg(0).0;
    let edge = if direction == UNDEF {
        None
    } else {
        match unsafe { read_string(g, direction) }.as_deref().and_then(resize_direction) {
            Some(edge) => Some(edge),
            None => {
                return unsafe {
                    host::throw(
                        g,
                        "TypeError",
                        "Use North, NorthEast, East, SouthEast, South, SouthWest, West or NorthWest \
                         for the resize direction, or no argument for the edge nearest the pointer.",
                    )
                };
            }
        }
    };
    submit(Op::StartResizeDragging(edge));
    UNDEF
});

host_fn!(fn close(_g, _args) {
    web::close_window();
    UNDEF
});

host_fn!(fn set_fullscreen(g, args) {
    unsafe {
        let options = args.arg(1).0;
        let mode = if is_cell(options) { get(g, options, "transition") } else { UNDEF };
        let fade = if mode == UNDEF {
            false
        } else {
            match read_string(g, mode).as_deref() {
                Some("fade") => true,
                Some("system") => false,
                _ => return host::throw(g, "TypeError", "Use fade or system for the fullscreen transition."),
            }
        };
        submit(Op::Fullscreen(host::truthy(g, args.arg(0).0), fade));
    }
    UNDEF
});

macro_rules! query {
    ($name:ident, $field:ident) => {
        host_fn!(fn $name(_g, _args) {
            refresh();
            enc_bool(read(|s| s.$field))
        });
    };
}

query!(is_maximized, maximized);
query!(is_minimized, minimized);
query!(is_fullscreen, fullscreen);
query!(is_focused, focused);
query!(is_visible, visible);

unsafe fn rect(g: *mut c_void, (x, y, w, h): (f32, f32, f32, f32)) -> u64 {
    unsafe {
        let obj = new_object(g, UNDEF);
        for (key, v) in [("x", x), ("y", y), ("width", w), ("height", h)] {
            define(g, obj, key, enc_f64(f64::from(v)), 0);
        }
        obj
    }
}

host_fn!(fn get_bounds(g, _args) { unsafe { rect(g, read(|s| s.outer)) } });
host_fn!(fn get_content_bounds(g, _args) { unsafe { rect(g, read(|s| s.content)) } });

unsafe fn finite(g: *mut c_void, obj: u64, key: &str) -> Option<f32> {
    if !is_cell(obj) {
        return None;
    }
    dec_f64(unsafe { get(g, obj, key) })
        .filter(|n| n.is_finite())
        .map(|n| n as f32)
}

host_fn!(fn set_content_bounds(g, args) {
    let r = args.arg(0).0;
    let [x, y, w, h] = ["x", "y", "width", "height"].map(|k| unsafe { finite(g, r, k) });
    web::set_bounds(x, y, w, h, true);
    UNDEF
});

/// Electron's shape: `false` retires the host-drawn min/max/close cluster, an object installs it (the
/// OS performs the actions; HTMAXBUTTON arms the Windows 11 snap flyout). `titleBarHeight` alone never does.
unsafe fn title_bar_overlay(g: *mut c_void, o: u64) {
    unsafe {
        if o == FALSE || o == NULL {
            set_overlay(None, OverlayMode::Off);
        } else if o == TRUE {
            set_overlay(
                None,
                OverlayMode::On {
                    color: String::new(),
                    symbol_color: String::new(),
                },
            );
        } else if is_cell(o) && !host::is_string(g, o) {
            let colour = |key: &str| {
                let v = get(g, o, key);
                if host::truthy(g, v) {
                    host::string_arg(g, v).unwrap_or_default()
                } else {
                    String::new()
                }
            };
            let (color, symbol_color) = (colour("color"), colour("symbolColor"));
            set_overlay(
                finite(g, o, "height"),
                OverlayMode::On {
                    color,
                    symbol_color,
                },
            );
        } else {
            return;
        }
    }
    refresh();
}

host_fn!(fn set_title_bar_overlay(g, args) {
    unsafe { title_bar_overlay(g, args.arg(0).0) };
    UNDEF
});

// --- state -----------------------------------------------------------------------------------

host_fn!(fn title_get(g, _args) { unsafe { js_string(g, &web::title()) } });

host_fn!(fn title_set(g, args) {
    unsafe { web::set_title(g, args.arg(0).0) };
    UNDEF
});

unsafe fn string_value(g: *mut c_void, v: u64) -> String {
    unsafe { host::string_arg(g, v) }.unwrap_or_else(|| "undefined".into())
}

host_fn!(fn backdrop_get(g, _args) { unsafe { js_string(g, read(|s| s.backdrop.as_str())) } });

host_fn!(fn backdrop_set(g, args) {
    set_backdrop(&unsafe { string_value(g, args.arg(0).0) });
    refresh();
    UNDEF
});

// What the window draws of `backdrop`: `none` while a macOS fullscreen window has no desktop picture.
host_fn!(fn effective_backdrop(g, _args) {
    unsafe { js_string(g, read(|s| s.effective_backdrop.as_str())) }
});

// Electron's `nativeTheme.themeSource`: the windows' dark mode (Mica's tint) and `prefers-color-scheme`.
host_fn!(fn theme_get(g, _args) { unsafe { js_string(g, read(|s| s.theme.as_str())) } });

host_fn!(fn theme_set(g, args) {
    set_theme(&unsafe { string_value(g, args.arg(0).0) });
    refresh();
    UNDEF
});

host_fn!(fn always_on_top_get(_g, _args) { enc_bool(read(|s| s.always_on_top)) });

host_fn!(fn always_on_top_set(g, args) {
    submit(Op::AlwaysOnTop(unsafe { host::truthy(g, args.arg(0).0) }));
    refresh();
    UNDEF
});

host_fn!(fn scale_factor(_g, _args) { enc_f64(f64::from(read(|s| s.scale))) });

host_fn!(fn chrome(g, _args) {
    let s = read(Snapshot::clone);
    let (overlay_visible, area) = titlebar_rect(&s);
    let style = match config().title_bar_style {
        TitleBarStyle::Default => "default",
        TitleBarStyle::Hidden => "hidden",
    };
    unsafe {
        let num = |v: f32| enc_f64(f64::from(v));
        let obj = new_object(g, UNDEF);
        let traffic = match s.traffic_light {
            Some((x, y)) => {
                let p = new_object(g, UNDEF);
                define(g, p, "x", num(x), 0);
                define(g, p, "y", num(y), 0);
                p
            }
            None => NULL,
        };
        define(g, obj, "trafficLightPosition", traffic, 0);
        define(g, obj, "titleBarStyle", js_string(g, style), 0);
        define(g, obj, "displayMode", js_string(g, display_mode(s.fullscreen).as_str()), 0);
        define(g, obj, "titleBarHeight", num(area.3), 0);
        define(g, obj, "fallbackTitleBar", enc_bool(s.fallback_titlebar), 0);
        define(g, obj, "overlayVisible", enc_bool(overlay_visible), 0);
        define(g, obj, "hostControls", enc_bool(host_controls(&s)), 0);
        let client = new_object(g, UNDEF);
        define(g, client, "width", num(s.client.0), 0);
        define(g, client, "height", num(s.client.1), 0);
        define(g, obj, "client", client, 0);
        let inset = new_object(g, UNDEF);
        let (left, top, right, bottom) = s.content_inset;
        for (key, v) in [("left", left), ("top", top), ("right", right), ("bottom", bottom)] {
            define(g, inset, key, num(v), 0);
        }
        define(g, obj, "contentInset", inset, 0);
        obj
    }
});

// --- defineWindow ----------------------------------------------------------------------------

/// The keys applied at run time.
const RUNTIME: [&str; 8] = [
    "title",
    "backdrop",
    "theme",
    "visibleOnAllWorkspaces",
    "opacity",
    "alwaysOnTop",
    "titleBarHeight",
    "titleBarOverlay",
];
/// `sk.config.json` `window` keys read only when the window is created.
const CREATION_ONLY: [&str; 13] = [
    "width",
    "height",
    "minWidth",
    "minHeight",
    "x",
    "y",
    "center",
    "monitor",
    "resizable",
    "minimizable",
    "maximizable",
    "titleBarFallback",
    "focus",
];
/// These need a different window style; changing it mid-session desyncs gpui's border_offset and hit-test flags.
const STYLE_ONLY: [&str; 2] = ["frame", "titleBarStyle"];

unsafe fn assign(g: *mut c_void, obj: u64, key: &str, v: u64) {
    unsafe {
        let reflect = get(g, glue::rjs_global(g), "Reflect");
        let _ = call(
            g,
            get(g, reflect, "set"),
            reflect,
            &[obj, js_string(g, key), v],
        );
    }
}

/// `false` when the value's type was refused.
unsafe fn apply_key(g: *mut c_void, app: u64, key: &str, v: u64) -> bool {
    unsafe {
        match key {
            "title" | "backdrop" | "theme" | "alwaysOnTop" => assign(g, app, key, v),
            "visibleOnAllWorkspaces" if v == TRUE || v == FALSE => assign(g, app, key, v),
            "opacity" if dec_f64(v).is_some() => assign(g, app, key, v),
            "titleBarHeight" => {
                set_overlay(
                    dec_f64(v).filter(|n| n.is_finite()).map(|n| n as f32),
                    OverlayMode::Keep,
                );
                refresh();
            }
            "titleBarOverlay" => title_bar_overlay(g, v),
            _ => return false,
        }
    }
    true
}

host_fn!(fn define_window(g, args) {
    let opts = args.arg(0).0;
    unsafe {
        if !is_cell(opts) || host::is_string(g, opts) {
            return UNDEF;
        }
        let object = get(g, glue::rjs_global(g), "Object");
        let Ok(keys) = call(g, get(g, object, "keys"), object, &[opts]) else { return UNDEF };
        let app = APP_WINDOW.with(Cell::get);
        for i in 0..array_len(g, keys) {
            let Some(key) = read_string(g, get_i(g, keys, i)) else { continue };
            if apply_key(g, app, &key, get(g, opts, &key)) {
                continue;
            }
            if STYLE_ONLY.contains(&key.as_str()) {
                web::warn(
                    g,
                    &format!("style-only:{key}"),
                    &format!(
                        "`{key}` cannot change after the window is created — it needs a different window \
                         style, which would desync the hit test and flicker. Declare it in sk.config.json."
                    ),
                );
            } else if CREATION_ONLY.contains(&key.as_str()) {
                web::warn(
                    g,
                    &format!("creation-only:{key}"),
                    &format!(
                        "`{key}` can only be set when the window is created. Declare it in `sk.config.json` \
                         under `window`."
                    ),
                );
            } else {
                let config_keys = [&CREATION_ONLY[..], &STYLE_ONLY[..]].concat().join(", ");
                web::warn(
                    g,
                    &format!("defineWindow:{key}"),
                    &format!(
                        "unknown `defineWindow` key: {}. It applies {}; {config_keys} belong in \
                         `sk.config.json` under `window`.",
                        crate::json::Json::s(key.clone()).to_json(),
                        RUNTIME.join(", "),
                    ),
                );
            }
        }
    }
    UNDEF
});

// --- install ---------------------------------------------------------------------------------

host_fn!(fn mark_sticky(_g, _args) {
    STICKY.with(|s| s.set(true));
    UNDEF
});

host_fn!(fn js_refresh(_g, _args) {
    refresh();
    UNDEF
});

pub(super) fn install(rt: &JsRuntime) {
    let g = rt.raw();
    let methods: Vec<(&str, u32, HostFn)> = [
        ("minimize", 0, minimize as HostFn),
        ("maximize", 0, maximize),
        ("unmaximize", 0, unmaximize),
        ("toggleMaximize", 0, toggle_maximize),
        ("restore", 0, unmaximize),
        ("close", 0, close),
        ("center", 0, center),
        ("startDragging", 0, start_dragging),
        ("startResizeDragging", 0, start_resize_dragging),
        ("setFullscreen", 2, set_fullscreen),
        ("isMaximized", 0, is_maximized),
        ("isMinimized", 0, is_minimized),
        ("isFullscreen", 0, is_fullscreen),
        ("isFocused", 0, is_focused),
        ("isVisible", 0, is_visible),
        ("getBounds", 0, get_bounds),
        ("getContentBounds", 0, get_content_bounds),
        ("setContentBounds", 1, set_content_bounds),
        ("setTitleBarOverlay", 1, set_title_bar_overlay),
    ]
    .into_iter()
    .chain(controls::METHODS)
    .collect();
    let accessors: Vec<(&str, HostFn, HostFn)> = [
        ("title", title_get as HostFn, title_set as HostFn),
        ("backdrop", backdrop_get, backdrop_set),
        ("theme", theme_get, theme_set),
        ("alwaysOnTop", always_on_top_get, always_on_top_set),
    ]
    .into_iter()
    .chain(controls::ACCESSORS)
    .collect();
    // SAFETY: the class and its rooted values belong to the host's live JS realm.
    unsafe {
        let global = glue::rjs_global(g);
        let target = get(g, get(g, global, "EventTarget"), "prototype");
        let class = define_class(
            g,
            &ClassSpec {
                name: "AppWindow",
                arity: 0,
                construct: None,
                parent_proto: target,
                methods: &methods,
                getters: &[
                    ("effectiveBackdrop", effective_backdrop),
                    ("scaleFactor", scale_factor),
                    ("chrome", chrome),
                ],
                accessors: &accessors,
            },
        );
        let name = atom(g, "AppWindow");
        glue::rjs_delete(g, global, name);
        glue::rjs_atom_free(g, name);
        let app = new_object(g, class.proto);
        redjs_protect(g, app);
        APP_WINDOW.with(|a| a.set(app));

        // gpui input reaches script through the runtime's dispatcher, which calls `__skStickyActivation`;
        // the capture listeners catch what script dispatches itself.
        let sticky = make_fn(g, "__skStickyActivation", 0, mark_sticky);
        define(g, global, "__skStickyActivation", sticky, 2);
        let add = get(g, global, "addEventListener");
        for kind in ["keydown", "mousedown"] {
            let _ = call(g, add, global, &[js_string(g, kind), sticky, TRUE]);
        }

        let host_obj = new_object(g, UNDEF);
        define(g, host_obj, "appWindow", app, 0);
        define(
            g,
            host_obj,
            "defineWindow",
            make_fn(g, "defineWindow", 1, define_window),
            0,
        );
        define(
            g,
            host_obj,
            "onChange",
            make_fn(g, "onChange", 1, web::on_change),
            0,
        );
        define(
            g,
            host_obj,
            "refresh",
            make_fn(g, "refresh", 0, js_refresh),
            0,
        );
        define(g, global, "__SK_WINDOW__", host_obj, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(js: &str) -> String {
        JsRuntime::with(|rt| {
            let v = rt.ctx().eval(js).unwrap_or_else(|_| panic!("{js} threw"));
            rt.ctx().to_rust_string(v).unwrap_or_default()
        })
    }

    #[test]
    fn an_unknown_resize_direction_throws_a_type_error() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                web::install(rt);
                install(rt);
            });
            let threw = |arg: &str| {
                eval(&format!(
                    "(() => {{ try {{ __SK_WINDOW__.appWindow.startResizeDragging({arg}); return 'none'; }}
                       catch (e) {{ return e instanceof TypeError ? 'TypeError' : String(e); }} }})()"
                ))
            };
            for bad in ["'east'", "'right'", "null", "4"] {
                assert_eq!(threw(bad), "TypeError", "{bad}");
            }
            for good in ["'SouthEast'", ""] {
                assert_eq!(threw(good), "none", "{good}");
            }
            super::super::PENDING.with(|p| p.borrow_mut().clear());
        });
    }

    #[test]
    fn close_request_veto_and_pagehide() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                web::install(rt);
                install(rt);
            });
            eval(
                "globalThis.__log = []; const w = __SK_WINDOW__.appWindow;
                 addEventListener('pagehide', () => __log.push('pagehide'));
                 __SK_WINDOW__.onChange(() => __log.push('change'));
                 globalThis.__keep = (e) => { __log.push(`asked:${e.isTrusted}:${e.cancelable}`); e.preventDefault(); };
                 w.addEventListener('close-requested', __keep); 1",
            );
            assert!(
                !close_requested(),
                "a cancelled close-requested keeps the window"
            );

            eval(
                "__SK_WINDOW__.appWindow.removeEventListener('close-requested', __keep);
                 addEventListener('beforeunload', (e) => e.preventDefault()); 1",
            );
            STICKY.with(|s| s.set(false));
            eval("__SK_WINDOW__.appWindow.title = 'A'; __SK_WINDOW__.appWindow.title = ' A '; 1");
            assert!(
                close_requested(),
                "an untouched app's beforeunload veto does not hold"
            );
            page_hide();
            assert_eq!(eval("__log.join()"), "asked:true:true,change,pagehide");

            eval("__skStickyActivation(); 1");
            assert!(
                !close_requested(),
                "after activation beforeunload keeps the window"
            );
        });
    }
}
