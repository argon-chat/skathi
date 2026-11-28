//! Building blocks for the native objects the host crates install: promises settled later from Rust,
//! errors, JSON values as JS values, accessors on plain objects.

use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;

use redjs::ffi::{redjs_promise_create, redjs_protect, redjs_unprotect};
use redjs::glue;
use redjs::webapi::domexception;
use redjs::webapi::util::*;

pub unsafe fn global(g: *mut c_void) -> u64 {
    unsafe { glue::rjs_global(g) }
}

/// A `TypeError` (or `RangeError`) or a `DOMException` named `name`, as a value.
pub unsafe fn error(g: *mut c_void, name: &str, msg: &str) -> u64 {
    unsafe {
        if matches!(name, "TypeError" | "RangeError" | "Error") {
            let ctor = get(g, global(g), name);
            return call(g, ctor, UNDEF, &[js_string(g, msg)]).unwrap_or(UNDEF);
        }
        domexception::create(g, name, msg)
    }
}

/// Throws `error(name, msg)`; the host function returns what this returns.
pub unsafe fn throw(g: *mut c_void, name: &str, msg: &str) -> u64 {
    unsafe {
        glue::rjs_throw(g, error(g, name, msg));
        UNDEF
    }
}

/// The resolve and reject of a promise kept alive until one of them runs.
pub struct Settler {
    resolve: u64,
    reject: u64,
}

impl Settler {
    pub unsafe fn resolve(self, g: *mut c_void, v: u64) {
        unsafe {
            let _ = call(g, self.resolve, UNDEF, &[v]);
            self.release(g);
        }
    }

    pub unsafe fn reject(self, g: *mut c_void, e: u64) {
        unsafe {
            let _ = call(g, self.reject, UNDEF, &[e]);
            self.release(g);
        }
    }

    pub unsafe fn settle(self, g: *mut c_void, outcome: Result<u64, u64>) {
        unsafe {
            match outcome {
                Ok(v) => self.resolve(g, v),
                Err(e) => self.reject(g, e),
            }
        }
    }

    /// Lets the promise stay pending forever, releasing what kept it settleable.
    pub unsafe fn abandon(self, g: *mut c_void) {
        unsafe {
            self.release(g);
        }
    }

    unsafe fn release(self, g: *mut c_void) {
        unsafe {
            redjs_unprotect(g, self.resolve);
            redjs_unprotect(g, self.reject);
        }
    }
}

/// A pending promise and what settles it.
pub unsafe fn deferred(g: *mut c_void) -> (u64, Settler) {
    unsafe {
        let (mut resolve, mut reject, mut promise) = (UNDEF, UNDEF, UNDEF);
        redjs_promise_create(g, &mut resolve, &mut reject, &mut promise);
        redjs_protect(g, resolve);
        redjs_protect(g, reject);
        (promise, Settler { resolve, reject })
    }
}

pub unsafe fn rejected_with(g: *mut c_void, reason: u64) -> u64 {
    unsafe {
        let (p, s) = deferred(g);
        s.reject(g, reason);
        p
    }
}

pub unsafe fn rejected(g: *mut c_void, name: &str, msg: &str) -> u64 {
    unsafe { rejected_with(g, error(g, name, msg)) }
}

pub unsafe fn resolved(g: *mut c_void, v: u64) -> u64 {
    unsafe { resolved_promise(g, v) }
}

pub unsafe fn to_js(g: *mut c_void, v: &serde_json::Value) -> u64 {
    unsafe {
        use serde_json::Value as J;
        match v {
            J::Null => NULL,
            J::Bool(b) => enc_bool(*b),
            J::Number(n) => enc_f64(n.as_f64().unwrap_or(0.0)),
            J::String(s) => js_string(g, s),
            J::Array(a) => {
                let arr = new_array(g);
                for (i, x) in a.iter().enumerate() {
                    set_i(g, arr, i as u32, to_js(g, x));
                }
                arr
            }
            J::Object(m) => {
                let obj = new_object(g, UNDEF);
                for (k, x) in m {
                    define(g, obj, k, to_js(g, x), 0);
                }
                obj
            }
        }
    }
}

/// `JSON.stringify(v)`, or `None` when it throws or gives `undefined`.
pub unsafe fn stringify(g: *mut c_void, v: u64) -> Option<String> {
    unsafe {
        let json = get(g, global(g), "JSON");
        let s = call(g, get(g, json, "stringify"), json, &[v]).ok()?;
        if s == UNDEF {
            return None;
        }
        read_string(g, s)
    }
}

/// A frozen array of strings.
pub unsafe fn frozen_strings(g: *mut c_void, items: &[impl AsRef<str>]) -> u64 {
    unsafe {
        let arr = new_array(g);
        for (i, s) in items.iter().enumerate() {
            set_i(g, arr, i as u32, js_string(g, s.as_ref()));
        }
        let object = get(g, global(g), "Object");
        call(g, get(g, object, "freeze"), object, &[arr]).unwrap_or(arr)
    }
}

pub unsafe fn is_string(g: *mut c_void, v: u64) -> bool {
    unsafe {
        const KIND_STRING: u32 = 5;
        glue::rjs_kind(g, v) == KIND_STRING
    }
}

pub unsafe fn truthy(g: *mut c_void, v: u64) -> bool {
    unsafe { call(g, get(g, global(g), "Boolean"), UNDEF, &[v]) == Ok(TRUE) }
}

pub unsafe fn string_arg(g: *mut c_void, v: u64) -> Option<String> {
    unsafe {
        if v == UNDEF {
            return None;
        }
        to_js_string(g, v).ok().and_then(|s| read_string(g, s))
    }
}

pub unsafe fn method(g: *mut c_void, obj: u64, name: &str, arity: u32, f: HostFn) {
    unsafe {
        define(g, obj, name, make_fn(g, name, arity, f), 2);
    }
}

pub unsafe fn getter(g: *mut c_void, obj: u64, name: &str, f: HostFn) {
    unsafe {
        let a = atom(g, name);
        glue::rjs_define_accessor(
            g,
            obj,
            a,
            make_fn(g, &format!("get {name}"), 0, f),
            UNDEF,
            0,
        );
        glue::rjs_atom_free(g, a);
    }
}

/// An object for a host API, held for the context and reachable as the hidden global `name`.
pub unsafe fn install_object(g: *mut c_void, name: &str, proto: u64) -> u64 {
    unsafe {
        let obj = new_object(g, proto);
        redjs_protect(g, obj);
        define(g, global(g), name, obj, 2);
        obj
    }
}

thread_local! {
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// `console.warn` once per `key` for this thread; false when `key` was already said.
pub unsafe fn warn_compat(g: *mut c_void, key: &str, msg: &str) -> bool {
    unsafe { warn_once(g, key.to_string(), &format!("[skathi compat] {msg}")) }
}

unsafe fn warn_once(g: *mut c_void, key: String, line: &str) -> bool {
    unsafe {
        if !WARNED.with(|w| w.borrow_mut().insert(key)) {
            return false;
        }
        let console = get(g, global(g), "console");
        if is_cell(console) {
            let _ = call(g, get(g, console, "warn"), console, &[js_string(g, line)]);
        }
        true
    }
}

pub fn has_warned(key: &str) -> bool {
    WARNED.with(|w| w.borrow().contains(key))
}

crate::host_fn!(fn js_advise(g, args) {
    let key = unsafe { string_arg(g, args.arg(0).0) }.unwrap_or_default();
    let msg = unsafe { string_arg(g, args.arg(1).0) }.unwrap_or_default();
    enc_bool(unsafe { warn_compat(g, &key, &msg) })
});

crate::host_fn!(fn js_advised(g, args) {
    let key = unsafe { string_arg(g, args.arg(0).0) }.unwrap_or_default();
    enc_bool(has_warned(&key))
});

crate::host_fn!(fn js_perf_advise(g, args) {
    let code = match unsafe { string_arg(g, args.arg(0).0) }.as_deref() {
        Some("wasted-render") => "wasted-render",
        Some("long-list") => "long-list",
        Some("host-writes") => "host-writes",
        _ => return UNDEF,
    };
    let key = unsafe { string_arg(g, args.arg(1).0) }.unwrap_or_default();
    let msg = unsafe { string_arg(g, args.arg(2).0) }.unwrap_or_default();
    redjs::profiling::hint(code, &msg);
    enc_bool(unsafe { warn_once(g, format!("perf:{code}:{key}"), &format!("[skathi perf] {msg}")) })
});

/// `__sk_advise(key, message)` and `__sk_advised(key)`: the runtime's one-shot advisories share the
/// host's set. `__sk_perf_advise(code, key, message)`: a dev-build performance advisory, a hint on the
/// recorded frame each time and a console line once per key.
pub(crate) unsafe fn install(g: *mut c_void) {
    unsafe {
        define(
            g,
            global(g),
            "__sk_perf_advise",
            make_fn(g, "__sk_perf_advise", 3, js_perf_advise),
            2,
        );
        define(
            g,
            global(g),
            "__sk_advise",
            make_fn(g, "__sk_advise", 2, js_advise),
            2,
        );
        define(
            g,
            global(g),
            "__sk_advised",
            make_fn(g, "__sk_advised", 1, js_advised),
            2,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::JsRuntime;

    #[test]
    fn the_runtime_and_the_host_share_one_advisory_set() {
        crate::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                let read = |src: &str| {
                    rt.ctx()
                        .to_rust_string(rt.ctx().eval(src).unwrap())
                        .unwrap()
                };
                assert!(unsafe { super::warn_compat(rt.raw(), "host-first", "said by the host") });
                assert_eq!(
                    read("[__sk_advised('host-first'), __sk_advise('host-first', 'x'), __sk_advise('js-first', 'y'), __sk_advise('js-first', 'y')].join()"),
                    "true,false,true,false"
                );
                assert!(super::has_warned("js-first"));
            });
        });
    }
}
