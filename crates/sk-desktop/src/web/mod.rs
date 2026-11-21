//! The JS surface of sk-desktop as native objects: `appShell`, `appLaunch` with `launchQueue`,
//! `appCapture`, and on Windows and macOS badging, screen wake lock, media session, Web Share and
//! passkeys. A promise for host work is settled from the desktop queue by [`deliver`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use sk_runtime::host::{self, Settler};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use crate::Fail;

mod app_capture;
mod app_launch;
mod app_shell;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod badge;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod credentials;
mod entries;
mod fsa;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod media_session;
mod network;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod share;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod wake_lock;

pub(crate) use sk_runtime::host::{error, global, rejected, rejected_with};

/// Turns a settled payload (and the value the host made with it, or `undefined`) into the value the promise resolves with, or the rejection.
pub(crate) type Convert = unsafe fn(*mut c_void, u32, &serde_json::Value, u64) -> Result<u64, u64>;

/// How a host failure reaches JS.
#[derive(Clone, Copy)]
pub(crate) enum Errors {
    /// A `DOMException` named by the failure, or a `TypeError`.
    Dom,
    /// An `Error` (or `TypeError`) whose `name` is the failure's, with Node-style `code` and the raw `osError`.
    Desktop,
}

struct Pending {
    settler: Settler,
    convert: Convert,
    errors: Errors,
}

thread_local! {
    static PENDING: RefCell<HashMap<u32, Pending>> = RefCell::new(HashMap::new());
    static INSTALLED: Cell<usize> = const { Cell::new(0) };
    static HANDLERS: Cell<usize> = const { Cell::new(0) };
}

unsafe fn failure(
    g: *mut c_void,
    errors: Errors,
    name: &str,
    message: &str,
    code: Option<&str>,
    os_error: Option<f64>,
) -> u64 {
    match errors {
        Errors::Dom => error(g, name, message),
        Errors::Desktop => {
            let ctor = get(
                g,
                global(g),
                if name == "TypeError" {
                    "TypeError"
                } else {
                    "Error"
                },
            );
            let e = call(g, ctor, UNDEF, &[js_string(g, message)]).unwrap_or(UNDEF);
            if name != "TypeError" {
                define(g, e, "name", js_string(g, name), 2);
            }
            if let Some(c) = code {
                define(g, e, "code", js_string(g, c), 0);
            }
            if let Some(n) = os_error {
                define(g, e, "osError", enc_f64(n), 0);
            }
            e
        }
    }
}

pub(crate) unsafe fn fail_value(g: *mut c_void, errors: Errors, f: &Fail) -> u64 {
    failure(
        g,
        errors,
        f.name,
        &f.message,
        f.code,
        f.os_error.map(f64::from),
    )
}

/// The promise for host work that settles `token`.
pub(crate) unsafe fn awaiting(g: *mut c_void, started: Result<u32, Fail>, convert: Convert) -> u64 {
    awaiting_with(g, started, convert, Errors::Dom)
}

pub(crate) unsafe fn awaiting_with(
    g: *mut c_void,
    started: Result<u32, Fail>,
    convert: Convert,
    errors: Errors,
) -> u64 {
    let token = match started {
        Ok(t) => t,
        Err(f) => return rejected_with(g, fail_value(g, errors, &f)),
    };
    let (p, settler) = host::deferred(g);
    PENDING.with(|m| {
        m.borrow_mut().insert(
            token,
            Pending {
                settler,
                convert,
                errors,
            },
        )
    });
    p
}

pub(crate) fn is_pending(token: u32) -> bool {
    PENDING.with(|m| m.borrow().contains_key(&token))
}

/// Rejects `token`'s promise now; the host's own answer is dropped when it comes.
pub(crate) unsafe fn abandon(g: *mut c_void, token: u32, reason: u64) {
    if let Some(p) = PENDING.with(|m| m.borrow_mut().remove(&token)) {
        p.settler.reject(g, reason);
    }
}

/// Whether the settle or host event was this module's.
pub(crate) unsafe fn deliver(
    g: *mut c_void,
    token: u32,
    ok: bool,
    payload: &str,
    make: Option<crate::Maker>,
) -> bool {
    let value: serde_json::Value = serde_json::from_str(payload).unwrap_or_default();
    if token == 0 {
        return match value["kind"].as_str() {
            Some("launch") => {
                app_launch::on_launch(g, &value);
                true
            }
            Some("network") => {
                network::on_event(g, &value);
                true
            }
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            Some("mediasession") => {
                media_session::on_action(g, &value);
                true
            }
            _ => false,
        };
    }
    let Some(p) = PENDING.with(|m| m.borrow_mut().remove(&token)) else {
        return false;
    };
    let outcome = if ok {
        let made = make.map_or(UNDEF, |m| m(g));
        (p.convert)(g, token, &value, made)
    } else {
        let name = value["name"].as_str().unwrap_or("Error");
        let message = value["message"]
            .as_str()
            .or(value["error"].as_str())
            .unwrap_or(name);
        Err(failure(
            g,
            p.errors,
            name,
            message,
            value["code"].as_str(),
            value["osError"].as_f64(),
        ))
    };
    p.settler.settle(g, outcome);
    true
}

/// The `on<event>` handler attributes of objects made here.
pub(crate) unsafe fn handler_get(g: *mut c_void, this: u64, name: &str) -> u64 {
    let bag = get_a(g, this, HANDLERS.with(|h| h.get()) as *mut c_void);
    let v = if is_cell(bag) {
        get(g, bag, name)
    } else {
        NULL
    };
    if is_cell(v) {
        v
    } else {
        NULL
    }
}

pub(crate) unsafe fn handler_set(g: *mut c_void, this: u64, name: &str, v: u64) {
    if !is_cell(this) {
        return;
    }
    let slot = HANDLERS.with(|h| h.get()) as *mut c_void;
    let mut bag = get_a(g, this, slot);
    if !is_cell(bag) {
        bag = new_object(g, UNDEF);
        glue::rjs_define(g, this, slot, bag, 0);
    }
    define(g, bag, name, if is_cell(v) { v } else { NULL }, 0);
}

/// `navigator`'s prototype, where the members go as in a browser.
#[cfg(any(target_os = "windows", target_os = "macos"))]
unsafe fn navigator_proto(g: *mut c_void) -> u64 {
    let ctor = get(g, global(g), "Navigator");
    let proto = if is_cell(ctor) {
        get(g, ctor, "prototype")
    } else {
        UNDEF
    };
    if is_cell(proto) {
        proto
    } else {
        get(g, global(g), "navigator")
    }
}

pub(crate) unsafe fn install(g: *mut c_void) {
    if INSTALLED.with(|i| i.replace(g as usize)) == g as usize {
        return;
    }
    HANDLERS.with(|h| h.set(atom_priv(g, "sk#handlers") as usize));
    let event_target = get(g, get(g, global(g), "EventTarget"), "prototype");
    app_shell::install(g);
    app_launch::install(g, event_target);
    app_capture::install(g);
    network::install(g, event_target);
    fsa::install(g);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let nav = navigator_proto(g);
        badge::install(g, nav);
        wake_lock::install(g, nav, event_target);
        media_session::install(g, nav);
        share::install(g, nav);
        credentials::install(g, nav);
    }
}

/// A getter returning one object for the context.
pub(crate) unsafe fn define_getter(g: *mut c_void, proto: u64, name: &str, f: HostFn) {
    let object = get(g, global(g), "Object");
    let desc = new_object(g, UNDEF);
    define(g, desc, "get", make_fn(g, name, 0, f), 0);
    define(g, desc, "enumerable", TRUE, 0);
    define(g, desc, "configurable", TRUE, 0);
    let _ = call(
        g,
        get(g, object, "defineProperty"),
        object,
        &[proto, js_string(g, name), desc],
    );
}

#[cfg(test)]
pub(crate) mod testing {
    use sk_runtime::redjs::api::Value;
    use sk_runtime::JsRuntime;

    /// Runs `src` with the objects installed, settles every queued host answer, and returns `globalThis.out`.
    pub fn run(src: &str) -> String {
        run_with(src, |_| {})
    }

    pub fn run_with(
        src: &str,
        setup: impl FnOnce(*mut std::ffi::c_void) + Send + 'static,
    ) -> String {
        sk_runtime::testing::on_js_thread({
            let src = src.to_string();
            move || {
                JsRuntime::with(|rt| unsafe {
                    super::install(rt.raw());
                    setup(rt.raw());
                    rt.ctx().eval(&format!("globalThis.out = undefined; (async () => {{ {src} }})().then(v => {{ out = String(v); }}, e => {{ out = 'threw ' + (e && e.name) + ': ' + (e && e.message); }});")).unwrap();
                    for _ in 0..400 {
                        rt.ctx().drain_microtasks();
                        drain(rt.raw());
                        rt.ctx().drain_microtasks();
                        let out = rt
                            .ctx()
                            .eval("String(globalThis.out)")
                            .and_then(|v: Value| rt.ctx().to_rust_string(v))
                            .unwrap_or_default();
                        if out != "undefined" {
                            return out;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    let at = rt
                        .ctx()
                        .eval("String(globalThis.at)")
                        .and_then(|v: Value| rt.ctx().to_rust_string(v))
                        .unwrap_or_default();
                    format!("<timed out at {at}>")
                })
            }
        })
    }

    unsafe fn drain(g: *mut std::ffi::c_void) {
        let tokens: Vec<u32> = super::PENDING.with(|m| m.borrow().keys().copied().collect());
        for (token, ok, payload, make) in crate::take_settled_with_makers(&tokens) {
            super::deliver(g, token, ok, &payload, make);
        }
    }
}
