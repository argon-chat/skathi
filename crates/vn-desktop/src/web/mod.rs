//! The navigator system APIs as native classes: badging, screen wake lock, media session, Web Share and
//! passkeys. A promise for host work is settled from the desktop queue by [`deliver`], before the JS
//! dispatcher sees the token.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use vn_runtime::redjs::ffi::{redjs_promise_create, redjs_protect, redjs_unprotect};
use vn_runtime::redjs::glue;
use vn_runtime::redjs::webapi::domexception;
use vn_runtime::redjs::webapi::util::*;

use crate::Fail;

mod badge;
mod credentials;
mod media_session;
mod share;
mod wake_lock;

/// Turns a settled payload into the value the promise resolves with, or the rejection.
pub(crate) type Convert = unsafe fn(*mut c_void, u32, &serde_json::Value) -> Result<u64, u64>;

struct Pending {
    resolve: u64,
    reject: u64,
    convert: Convert,
}

thread_local! {
    static PENDING: RefCell<HashMap<u32, Pending>> = RefCell::new(HashMap::new());
    static INSTALLED: Cell<usize> = const { Cell::new(0) };
    static HANDLERS: Cell<usize> = const { Cell::new(0) };
}

pub(crate) unsafe fn global(g: *mut c_void) -> u64 {
    glue::rjs_global(g)
}

/// A `TypeError` or a `DOMException` named `name`, as a value.
pub(crate) unsafe fn error(g: *mut c_void, name: &str, msg: &str) -> u64 {
    if name == "TypeError" {
        let ctor = get(g, global(g), "TypeError");
        return call(g, ctor, UNDEF, &[js_string(g, msg)]).unwrap_or(UNDEF);
    }
    domexception::create(g, name, msg)
}

pub(crate) unsafe fn deferred(g: *mut c_void) -> (u64, u64, u64) {
    let (mut resolve, mut reject, mut promise) = (UNDEF, UNDEF, UNDEF);
    redjs_promise_create(g, &mut resolve, &mut reject, &mut promise);
    (promise, resolve, reject)
}

pub(crate) unsafe fn rejected(g: *mut c_void, name: &str, msg: &str) -> u64 {
    rejected_with(g, error(g, name, msg))
}

pub(crate) unsafe fn rejected_with(g: *mut c_void, reason: u64) -> u64 {
    let (p, _, reject) = deferred(g);
    let _ = call(g, reject, UNDEF, &[reason]);
    p
}

/// The promise for host work that settles `token`.
pub(crate) unsafe fn awaiting(g: *mut c_void, started: Result<u32, Fail>, convert: Convert) -> u64 {
    let token = match started {
        Ok(t) => t,
        Err(f) => return rejected(g, f.name, &f.message),
    };
    let (p, resolve, reject) = deferred(g);
    redjs_protect(g, resolve);
    redjs_protect(g, reject);
    PENDING.with(|m| {
        m.borrow_mut().insert(
            token,
            Pending {
                resolve,
                reject,
                convert,
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
        let _ = call(g, p.reject, UNDEF, &[reason]);
        redjs_unprotect(g, p.resolve);
        redjs_unprotect(g, p.reject);
    }
}

/// Whether the settle or host event was this module's.
pub(crate) unsafe fn deliver(g: *mut c_void, token: u32, ok: bool, payload: &str) -> bool {
    let value: serde_json::Value = serde_json::from_str(payload).unwrap_or_default();
    if token == 0 {
        return match value["kind"].as_str() {
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
        (p.convert)(g, token, &value)
    } else {
        let s = |k: &str| value[k].as_str().unwrap_or("").to_string();
        Err(error(
            g,
            value["name"].as_str().unwrap_or("NotAllowedError"),
            &s("message"),
        ))
    };
    let _ = match outcome {
        Ok(v) => call(g, p.resolve, UNDEF, &[v]),
        Err(e) => call(g, p.reject, UNDEF, &[e]),
    };
    redjs_unprotect(g, p.resolve);
    redjs_unprotect(g, p.reject);
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
    HANDLERS.with(|h| h.set(atom_priv(g, "vn#handlers") as usize));
    let nav = navigator_proto(g);
    let event_target = get(g, get(g, global(g), "EventTarget"), "prototype");
    badge::install(g, nav);
    wake_lock::install(g, nav, event_target);
    media_session::install(g, nav);
    share::install(g, nav);
    credentials::install(g, nav);
}

/// A `navigator` getter returning one object for the context.
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
    use vn_runtime::redjs::api::Value;
    use vn_runtime::JsRuntime;

    /// Runs `src` with the classes installed, settles every queued host answer, and returns `globalThis.out`.
    pub fn run(src: &str) -> String {
        vn_runtime::testing::on_js_thread({
            let src = src.to_string();
            move || {
                JsRuntime::with(|rt| unsafe {
                    super::install(rt.raw());
                    rt.ctx().eval(&format!("globalThis.out = undefined; (async () => {{ {src} }})().then(v => {{ out = String(v); }}, e => {{ out = 'threw ' + (e && e.name) + ': ' + (e && e.message); }});")).unwrap();
                    for _ in 0..200 {
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
                    "<timed out>".to_string()
                })
            }
        })
    }

    unsafe fn drain(g: *mut std::ffi::c_void) {
        let tokens: Vec<u32> = super::PENDING.with(|m| m.borrow().keys().copied().collect());
        for (token, (ok, payload)) in crate::take_settled(&tokens) {
            super::deliver(g, token, ok, &payload);
        }
    }
}
