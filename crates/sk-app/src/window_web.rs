//! The main window's standard web surface: `screen`, `matchMedia`, `navigator.windowControlsOverlay`,
//! `screenX`/`screenY`, `moveTo`/`resizeTo` and friends, `focus`/`blur`/`close`/`open`, and
//! `document.title`/`hasFocus()`. Every getter reads the published [`super::Snapshot`].

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::events;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host, host_fn, raw_host_fn, JsRuntime};

use super::{submit, Op, SNAPSHOT};
use crate::json::Json;

/// Live `MediaQueryList`s whose query reads compute pressure; dropped by the GC thread's finalizer.
static PRESSURE_QUERIES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Bumped whenever the environment `matchMedia` evaluates against changes.
    static ENV_GEN: Cell<u64> = const { Cell::new(1) };
    /// Queries with a `change` listener or `onchange`, protected while they have one.
    static LISTENED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static MQL_PROTO: Cell<u64> = const { Cell::new(0) };
    static ONCHANGE: Cell<usize> = const { Cell::new(0) };
    static ONGEOMETRYCHANGE: Cell<usize> = const { Cell::new(0) };
    static WCO: Cell<u64> = const { Cell::new(0) };
    /// `document.title` as script reads it; `None` until script sets it.
    static TITLE: RefCell<Option<String>> = const { RefCell::new(None) };
    static PAGE_HIDDEN: Cell<bool> = const { Cell::new(false) };
    static CHANGE_LISTENERS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

struct Mql {
    query: String,
    matches: bool,
    generation: u64,
    pressure: bool,
    listened: bool,
}

impl Drop for Mql {
    fn drop(&mut self) {
        if self.pressure {
            PRESSURE_QUERIES.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

unsafe fn mql_class() -> JSClassRef {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe { make_host_class::<Mql>("MediaQueryList") as usize })
        as JSClassRef
}

unsafe fn mql<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Mql> {
    unsafe { host_data(g, mql_class(), v) }
}

/// An advisory once per key: on the console and in the host log.
pub(crate) unsafe fn warn(g: *mut c_void, key: &str, msg: &str) {
    if unsafe { host::warn_compat(g, key, msg) } {
        super::advise(msg);
    }
}

unsafe fn report(g: *mut c_void, error: u64) {
    unsafe {
        let report = get(g, host::global(g), "reportError");
        let _ = call(g, report, UNDEF, &[error]);
    }
}

fn snapshot<R>(f: impl FnOnce(&super::Snapshot) -> R) -> R {
    SNAPSHOT.with(|s| f(&s.borrow()))
}

pub(crate) fn note_env_changed() {
    ENV_GEN.with(|g| g.set(g.get() + 1));
}

/// HTML's "strip and collapse ASCII whitespace".
fn normalize_title(text: &str) -> String {
    text.split(['\t', '\n', '\x0c', '\r', ' '])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs the `__SK_WINDOW__.onChange` subscribers: some `appWindow` state may have moved.
pub(crate) fn changed() {
    let listeners = CHANGE_LISTENERS.with(|l| l.borrow().clone());
    if listeners.is_empty() {
        return;
    }
    JsRuntime::with(|rt| unsafe {
        let g = rt.raw();
        for f in &listeners {
            redjs_protect(g, *f);
        }
        for f in &listeners {
            if let Err(e) = call(g, *f, UNDEF, &[]) {
                report(g, e);
            }
        }
        for f in listeners {
            redjs_unprotect(g, f);
        }
    });
}

/// `pagehide`, once per session.
pub(crate) fn hide_page() {
    if PAGE_HIDDEN.with(|h| h.replace(true)) {
        return;
    }
    JsRuntime::with(|rt| unsafe {
        let g = rt.raw();
        let ev = events::new_trusted_event(g, "pagehide");
        events::dispatch(g, glue::rjs_global(g), ev);
    });
}

host_fn!(pub(super) fn on_change(g, args) {
    unsafe {
        let f = args.arg(0).0;
        if !events::is_callable(g, f) {
            return throw_type_error(g, "onChange requires a function");
        }
        redjs_protect(g, f);
        CHANGE_LISTENERS.with(|l| l.borrow_mut().push(f));
        let off = make_fn(g, "", 0, off_change);
        call(g, get(g, off, "bind"), off, &[UNDEF, f]).unwrap_or(UNDEF)
    }
});

host_fn!(fn off_change(g, args) {
    let f = args.arg(0).0;
    let removed = CHANGE_LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        l.iter().position(|&x| x == f).map(|i| l.remove(i)).is_some()
    });
    if removed {
        unsafe { redjs_unprotect(g, f) };
    }
    UNDEF
});

// --- window geometry and focus ---------------------------------------------------------------

host_fn!(fn screen_x(_g, _args) { enc_f64(f64::from(snapshot(|s| s.outer.0))) });
host_fn!(fn screen_y(_g, _args) { enc_f64(f64::from(snapshot(|s| s.outer.1))) });

fn finite(v: u64) -> Option<f32> {
    dec_f64(v).filter(|n| n.is_finite()).map(|n| n as f32)
}

/// `||`-style: a missing or non-number delta is zero.
fn delta(v: u64) -> f32 {
    finite(v).unwrap_or(0.)
}

/// Best-effort and clamped to the display's working area; a refused call is a silent no-op.
pub(crate) fn set_bounds(
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    h: Option<f32>,
    content: bool,
) {
    submit(Op::Bounds {
        x,
        y,
        w,
        h,
        content,
    });
}

host_fn!(fn resize_to(_g, args) {
    set_bounds(None, None, finite(args.arg(0).0), finite(args.arg(1).0), false);
    UNDEF
});

host_fn!(fn resize_by(_g, args) {
    let (_, _, w, h) = snapshot(|s| s.outer);
    set_bounds(None, None, Some(w + delta(args.arg(0).0)), Some(h + delta(args.arg(1).0)), false);
    UNDEF
});

host_fn!(fn move_to(_g, args) {
    set_bounds(finite(args.arg(0).0), finite(args.arg(1).0), None, None, false);
    UNDEF
});

host_fn!(fn move_by(_g, args) {
    let (x, y, _, _) = snapshot(|s| s.outer);
    set_bounds(Some(x + delta(args.arg(0).0)), Some(y + delta(args.arg(1).0)), None, None, false);
    UNDEF
});

pub(crate) fn close_window() {
    hide_page();
    submit(Op::Close);
}

host_fn!(fn close(_g, _args) {
    close_window();
    UNDEF
});

// The host does not steal focus unless configured to, so `focus()` requests attention instead.
host_fn!(fn focus(g, _args) {
    let raise = !crate::quiet::never_focus() && super::config().focus_raise;
    if !raise {
        unsafe {
            warn(
                g,
                "focus",
                "`window.focus()` requested attention instead of raising the window — this host is \
                 configured not to steal focus. Set `window.focus: \"raise\"` in sk.config.json (or \
                 SK_WINDOW_FOCUS=raise) to allow raising.",
            )
        };
    }
    submit(Op::Activate(raise));
    UNDEF
});

host_fn!(fn blur(_g, _args) { UNDEF });

// A second document, which `<SkWindow>` (one tree, two windows) is not.
host_fn!(fn open(_g, _args) { NULL });

host_fn!(fn title_get(g, _args) {
    let title = TITLE.with(|t| t.borrow().clone())
        .unwrap_or_else(|| normalize_title(&snapshot(|s| s.title.clone())));
    unsafe { js_string(g, &title) }
});

host_fn!(fn title_set(g, args) {
    unsafe { set_title(g, args.arg(0).0) };
    UNDEF
});

pub(super) unsafe fn set_title(g: *mut c_void, v: u64) {
    let raw = unsafe { host::string_arg(g, v) }.unwrap_or_else(|| "undefined".into());
    let next = normalize_title(&raw);
    // The raw string goes to the OS.
    submit(Op::Title(raw));
    let moved = TITLE.with(|t| {
        let mut t = t.borrow_mut();
        let before = t
            .clone()
            .unwrap_or_else(|| normalize_title(&snapshot(|s| s.title.clone())));
        *t = Some(next.clone());
        before != next
    });
    if moved {
        changed();
    }
}

/// What `appWindow.title` reads.
pub(crate) fn title() -> String {
    TITLE
        .with(|t| t.borrow().clone())
        .unwrap_or_else(|| normalize_title(&snapshot(|s| s.title.clone())))
}

host_fn!(fn has_focus(_g, _args) { enc_bool(snapshot(|s| s.focused)) });

// --- Screen ----------------------------------------------------------------------------------

host_fn!(fn screen_width(_g, _args) { enc_f64(f64::from(snapshot(|s| s.screen.0))) });
host_fn!(fn screen_height(_g, _args) { enc_f64(f64::from(snapshot(|s| s.screen.1))) });
host_fn!(fn screen_avail_width(_g, _args) { enc_f64(f64::from(snapshot(|s| s.screen.2))) });
host_fn!(fn screen_avail_height(_g, _args) { enc_f64(f64::from(snapshot(|s| s.screen.3))) });
host_fn!(fn screen_avail_left(_g, _args) { enc_f64(f64::from(snapshot(|s| s.screen.4))) });
host_fn!(fn screen_avail_top(_g, _args) { enc_f64(f64::from(snapshot(|s| s.screen.5))) });
host_fn!(fn screen_depth(_g, _args) { enc_i32(24) });
host_fn!(fn screen_extended(_g, _args) { enc_bool(snapshot(|s| s.displays > 1)) });

thread_local! {
    static ORIENTATION: Cell<u64> = const { Cell::new(0) };
}

host_fn!(fn screen_orientation(_g, _args) { ORIENTATION.with(Cell::get) });

host_fn!(fn orientation_type(g, _args) {
    let landscape = snapshot(|s| s.screen.0 >= s.screen.1);
    unsafe { js_string(g, if landscape { "landscape-primary" } else { "portrait-primary" }) }
});

host_fn!(fn orientation_angle(_g, _args) { enc_i32(0) });

host_fn!(fn orientation_lock(g, _args) {
    unsafe { host::rejected(g, "NotSupportedError", "screen.orientation.lock() is not available on this device.") }
});

host_fn!(fn orientation_unlock(_g, _args) { UNDEF });

// --- matchMedia ------------------------------------------------------------------------------

/// `(matches, reads compute pressure)`; a query that does not compile advises once and never matches.
unsafe fn evaluate(g: *mut c_void, query: &str) -> (bool, bool) {
    match sk_css::match_media_with_demand(query, &sk_tree::env()) {
        Ok(result) => result,
        Err(why) => {
            unsafe {
                warn(
                    g,
                    &format!("mq:{query}"),
                    &format!(
                        "matchMedia({}) `{query}` {why}",
                        Json::s(query.to_string()).to_json()
                    ),
                )
            };
            (false, false)
        }
    }
}

/// Re-evaluates when the environment moved since the query last looked; `true` when `matches` flipped.
unsafe fn refresh_query(g: *mut c_void, data: &mut Mql) -> bool {
    let now = ENV_GEN.with(Cell::get);
    if data.generation == now {
        return false;
    }
    data.generation = now;
    let (matches, _) = unsafe { evaluate(g, &data.query) };
    std::mem::replace(&mut data.matches, matches) != matches
}

host_fn!(fn match_media(g, args) {
    unsafe {
        let Ok(text) = to_js_string(g, args.arg(0).0) else { return UNDEF };
        let query = read_string(g, text).unwrap_or_default();
        let (matches, pressure) = evaluate(g, &query);
        if pressure {
            PRESSURE_QUERIES.fetch_add(1, Ordering::Relaxed);
        }
        host_create(
            g,
            mql_class(),
            MQL_PROTO.with(Cell::get),
            Mql { query, matches, generation: ENV_GEN.with(Cell::get), pressure, listened: false },
        )
    }
});

host_fn!(fn mql_media(g, args) {
    unsafe {
        let Some(data) = mql(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
        js_string(g, &data.query)
    }
});

host_fn!(fn mql_matches(g, args) {
    unsafe {
        let Some(data) = mql(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
        refresh_query(g, data);
        enc_bool(data.matches)
    }
});

unsafe fn handler(
    g: *mut c_void,
    this: u64,
    slot: &'static std::thread::LocalKey<Cell<usize>>,
) -> u64 {
    let v = unsafe { get_a(g, this, slot.with(Cell::get) as *mut c_void) };
    if unsafe { events::is_callable(g, v) } {
        v
    } else {
        NULL
    }
}

unsafe fn set_handler(
    g: *mut c_void,
    this: u64,
    slot: &'static std::thread::LocalKey<Cell<usize>>,
    v: u64,
) {
    let v = if unsafe { is_cell(v) && glue::rjs_kind(g, v) >= 8 } {
        v
    } else {
        NULL
    };
    unsafe { glue::rjs_define(g, this, slot.with(Cell::get) as *mut c_void, v, 0) };
}

host_fn!(fn mql_onchange_get(g, args) {
    unsafe {
        if mql(g, args.this().0).is_none() { return throw_type_error(g, "Illegal invocation") }
        handler(g, args.this().0, &ONCHANGE)
    }
});

host_fn!(fn mql_onchange_set(g, args) {
    unsafe {
        let this = args.this().0;
        if mql(g, this).is_none() { return throw_type_error(g, "Illegal invocation") }
        set_handler(g, this, &ONCHANGE, args.arg(0).0);
        retain(g, this);
        UNDEF
    }
});

/// Holds a query while something listens to it, so `matchMedia(q).onchange = f` keeps firing.
unsafe fn retain(g: *mut c_void, this: u64) {
    unsafe {
        let Some(data) = mql(g, this) else { return };
        let listened = events::has_listener(g, this, "change");
        if listened == data.listened {
            return;
        }
        data.listened = listened;
        if listened {
            // Fires on the next change, not on one that happened while nobody listened.
            refresh_query(g, data);
            redjs_protect(g, this);
            LISTENED.with(|l| l.borrow_mut().push(this));
        } else {
            LISTENED.with(|l| l.borrow_mut().retain(|&q| q != this));
            redjs_unprotect(g, this);
        }
    }
}

raw_host_fn!(fn mql_add_listener(g, cf) {
    unsafe {
        let out = events::add_event_listener(g, cf);
        let this = cf_this(cf);
        if read_string(g, cf_arg(cf, 0)).as_deref() == Some("change") {
            retain(g, this);
            let options = cf_arg(cf, 2);
            if is_cell(options) && glue::rjs_kind(g, options) >= 8 {
                let signal = get(g, options, "signal");
                if is_cell(signal) {
                    let again = make_fn(g, "", 0, mql_retain_this);
                    if let Ok(bound) = call(g, get(g, again, "bind"), again, &[this]) {
                        let once = new_object(g, UNDEF);
                        define(g, once, "once", TRUE, 0);
                        let _ = call(g, get(g, signal, "addEventListener"), signal, &[js_string(g, "abort"), bound, once]);
                    }
                }
            }
        }
        out
    }
});

raw_host_fn!(fn mql_remove_listener(g, cf) {
    unsafe {
        let out = events::remove_event_listener(g, cf);
        retain(g, cf_this(cf));
        out
    }
});

host_fn!(fn mql_retain_this(g, args) {
    unsafe { retain(g, args.this().0) };
    UNDEF
});

host_fn!(fn mql_add_legacy(g, args) {
    unsafe {
        let this = args.this().0;
        let add = get(g, this, "addEventListener");
        call(g, add, this, &[js_string(g, "change"), args.arg(0).0]).unwrap_or(UNDEF)
    }
});

host_fn!(fn mql_remove_legacy(g, args) {
    unsafe {
        let this = args.this().0;
        let remove = get(g, this, "removeEventListener");
        call(g, remove, this, &[js_string(g, "change"), args.arg(0).0]).unwrap_or(UNDEF)
    }
});

/// Fires `change` on every listened query whose result moved; after the frame's other window events.
pub(crate) fn dispatch_media_changes() {
    crate::power::set_match_media_demand(PRESSURE_QUERIES.load(Ordering::Relaxed) > 0);
    let listened = LISTENED.with(|l| l.borrow().clone());
    if listened.is_empty() {
        return;
    }
    JsRuntime::with(|rt| unsafe {
        let g = rt.raw();
        for &q in &listened {
            redjs_protect(g, q);
        }
        for &q in &listened {
            let Some(data) = mql(g, q) else { continue };
            if !refresh_query(g, data) {
                continue;
            }
            let (matches, media) = (data.matches, js_string(g, &data.query));
            let init = new_object(g, UNDEF);
            define(g, init, "matches", enc_bool(matches), 0);
            define(g, init, "media", media, 0);
            let ctor = get(g, glue::rjs_global(g), "MediaQueryListEvent");
            let mut ev = UNDEF;
            if glue::rjs_construct(g, ctor, [js_string(g, "change"), init].as_ptr(), 2, &mut ev)
                != 0
            {
                continue;
            }
            events::dispatch(g, q, ev);
            // A `once` listener is gone now.
            retain(g, q);
        }
        for q in listened {
            redjs_unprotect(g, q);
        }
    });
}

// --- WindowControlsOverlay -------------------------------------------------------------------

host_fn!(fn wco_visible(_g, _args) { enc_bool(snapshot(|s| super::titlebar_rect(s).0)) });

host_fn!(fn wco_rect(g, _args) {
    let (_, (x, y, w, h)) = snapshot(super::titlebar_rect);
    unsafe {
        let ctor = get(g, glue::rjs_global(g), "DOMRect");
        let args = [enc_f64(f64::from(x)), enc_f64(f64::from(y)), enc_f64(f64::from(w)), enc_f64(f64::from(h))];
        let mut out = UNDEF;
        if glue::rjs_construct(g, ctor, args.as_ptr(), 4, &mut out) != 0 {
            glue::rjs_throw(g, out);
            return UNDEF;
        }
        out
    }
});

host_fn!(fn wco_ongeometrychange_get(g, args) {
    unsafe { handler(g, args.this().0, &ONGEOMETRYCHANGE) }
});

host_fn!(fn wco_ongeometrychange_set(g, args) {
    unsafe { set_handler(g, args.this().0, &ONGEOMETRYCHANGE, args.arg(0).0) };
    UNDEF
});

/// `geometrychange` on `navigator.windowControlsOverlay`.
pub(crate) fn dispatch_geometry_change() {
    JsRuntime::with(|rt| unsafe {
        let g = rt.raw();
        let wco = WCO.with(Cell::get);
        let ev = events::new_trusted_event(g, "geometrychange");
        events::dispatch(g, wco, ev);
    });
}

// --- install ---------------------------------------------------------------------------------

unsafe fn accessor(g: *mut c_void, obj: u64, name: &str, read: HostFn, write: Option<HostFn>) {
    unsafe {
        let key = atom(g, name);
        let setter = write.map_or(UNDEF, |w| make_fn(g, &format!("set {name}"), 1, w));
        glue::rjs_define_accessor(
            g,
            obj,
            key,
            make_fn(g, &format!("get {name}"), 0, read),
            setter,
            0,
        );
        glue::rjs_atom_free(g, key);
    }
}

unsafe fn class(
    g: *mut c_void,
    name: &str,
    parent: u64,
    methods: &[(&str, u32, HostFn)],
    getters: &[(&str, HostFn)],
    accessors: &[(&str, HostFn, HostFn)],
) -> u64 {
    unsafe {
        define_class(
            g,
            &ClassSpec {
                name,
                arity: 0,
                construct: None,
                parent_proto: parent,
                methods,
                getters,
                accessors,
            },
        )
        .proto
    }
}

pub(crate) fn install(rt: &JsRuntime) {
    let g = rt.raw();
    // SAFETY: the classes and their rooted values belong to the host's live JS realm.
    unsafe {
        let global = glue::rjs_global(g);
        let target = get(g, get(g, global, "EventTarget"), "prototype");
        ONCHANGE.with(|s| s.set(atom_priv(g, "MediaQueryList onchange") as usize));
        ONGEOMETRYCHANGE
            .with(|s| s.set(atom_priv(g, "WindowControlsOverlay ongeometrychange") as usize));

        accessor(g, global, "screenX", screen_x, None);
        accessor(g, global, "screenY", screen_y, None);
        accessor(g, global, "screenLeft", screen_x, None);
        accessor(g, global, "screenTop", screen_y, None);
        for (name, arity, f) in [
            ("resizeTo", 2, resize_to as HostFn),
            ("resizeBy", 2, resize_by),
            ("moveTo", 2, move_to),
            ("moveBy", 2, move_by),
            ("close", 0, close),
            ("focus", 0, focus),
            ("blur", 0, blur),
            ("open", 0, open),
            ("matchMedia", 1, match_media),
        ] {
            define(g, global, name, make_fn(g, name, arity, f), 2);
        }
        let document = get(g, global, "document");
        accessor(g, document, "title", title_get, Some(title_set));
        define(
            g,
            document,
            "hasFocus",
            make_fn(g, "hasFocus", 0, has_focus),
            2,
        );

        let orientation = class(
            g,
            "ScreenOrientation",
            target,
            &[
                ("lock", 1, orientation_lock),
                ("unlock", 0, orientation_unlock),
            ],
            &[("type", orientation_type), ("angle", orientation_angle)],
            &[],
        );
        let orientation = new_object(g, orientation);
        redjs_protect(g, orientation);
        ORIENTATION.with(|o| o.set(orientation));
        let screen = class(
            g,
            "Screen",
            target,
            &[],
            &[
                ("width", screen_width),
                ("height", screen_height),
                ("availWidth", screen_avail_width),
                ("availHeight", screen_avail_height),
                ("availLeft", screen_avail_left),
                ("availTop", screen_avail_top),
                ("colorDepth", screen_depth),
                ("pixelDepth", screen_depth),
                ("isExtended", screen_extended),
                ("orientation", screen_orientation),
            ],
            &[],
        );
        define(g, global, "screen", new_object(g, screen), 2);

        let mql = class(
            g,
            "MediaQueryList",
            target,
            &[
                ("addEventListener", 2, mql_add_listener),
                ("removeEventListener", 2, mql_remove_listener),
                ("addListener", 1, mql_add_legacy),
                ("removeListener", 1, mql_remove_legacy),
            ],
            &[("media", mql_media), ("matches", mql_matches)],
            &[("onchange", mql_onchange_get, mql_onchange_set)],
        );
        MQL_PROTO.with(|p| p.set(mql));

        let wco = class(
            g,
            "WindowControlsOverlay",
            target,
            &[("getTitlebarAreaRect", 0, wco_rect)],
            &[("visible", wco_visible)],
            &[(
                "ongeometrychange",
                wco_ongeometrychange_get,
                wco_ongeometrychange_set,
            )],
        );
        let wco = new_object(g, wco);
        redjs_protect(g, wco);
        WCO.with(|w| w.set(wco));
        // redjs creates `navigator` lazily.
        let mut navigator = get(g, global, "navigator");
        if !is_cell(navigator) {
            navigator = new_object(g, UNDEF);
            define(g, global, "navigator", navigator, 2);
        }
        define(g, navigator, "windowControlsOverlay", wco, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_title;

    #[test]
    fn title_collapses_only_html_ascii_whitespace() {
        for (value, expected) in [
            ("", ""),
            ("\t\n\x0c\r ", ""),
            (" a\tb\nc\x0cd\re  f ", "a b c d e f"),
            (" \u{a0}\t\u{b} ", "\u{a0} \u{b}"),
            ("\u{1f600}\t\u{2028}", "\u{1f600} \u{2028}"),
        ] {
            assert_eq!(normalize_title(value), expected);
        }
    }
}
