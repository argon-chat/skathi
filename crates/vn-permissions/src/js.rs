//! `appPermissions`: `navigator.permissions` plus a standalone `request` and the fourth state,
//! `unsupported`. `change` is re-broadcast from the standard `PermissionStatus` objects.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use vn_runtime::host::{self, Settler};
use vn_runtime::host_fn;
use vn_runtime::redjs::webapi::permissions::{self as api, Capability, PermissionState, RequestOutcome};
use vn_runtime::redjs::webapi::util::*;
use vn_runtime::redjs::webapi::events;

thread_local! {
    static OBJ: Cell<u64> = const { Cell::new(0) };
    static PENDING: RefCell<HashMap<u32, Vec<Settler>>> = RefCell::new(HashMap::new());
}

fn platform_name() -> &'static str {
    api::host().map(|h| h.platform()).unwrap_or("this platform")
}

unsafe fn advise_unsupported(g: *mut c_void, name: &str) {
    host::warn_compat(
        g,
        &format!("appPermissions:{name}"),
        &format!(
            "the '{name}' permission has no equivalent on {} — appPermissions.query('{name}') is 'unsupported' and request() is a no-op.",
            platform_name()
        ),
    );
}

unsafe fn state_str(g: *mut c_void, cap: Capability) -> u64 {
    js_string(g, super::state_of(cap).as_str())
}

host_fn!(fn platform(g, _args) {
    unsafe { js_string(g, platform_name()) }
});

host_fn!(fn names(g, _args) {
    unsafe {
        let arr = new_array(g);
        for (i, c) in Capability::ALL.iter().enumerate() {
            set_i(g, arr, i as u32, js_string(g, c.web_name()));
        }
        arr
    }
});

host_fn!(fn query(g, args) {
    unsafe {
        let name = host::string_arg(g, args.arg(0).0).unwrap_or_default();
        let Some(cap) = Capability::from_web_name(&name) else {
            return host::throw(g, "TypeError", &format!("'{name}' is not a permission name"));
        };
        let state = super::state_of(cap);
        if state == PermissionState::Unsupported {
            advise_unsupported(g, &name);
        }
        js_string(g, state.as_str())
    }
});

unsafe fn row(g: *mut c_void, cap: Capability) -> u64 {
    let (state, can_request, by_system) = super::describe(cap);
    let o = new_object(g, UNDEF);
    define(g, o, "name", js_string(g, cap.web_name()), 0);
    define(g, o, "state", js_string(g, state.as_str()), 0);
    define(g, o, "canRequest", enc_bool(can_request), 0);
    define(g, o, "deniedBySystem", enc_bool(by_system), 0);
    o
}

host_fn!(fn list(g, _args) {
    unsafe {
        let arr = new_array(g);
        for (i, cap) in Capability::ALL.iter().copied().enumerate() {
            if super::state_of(cap) == PermissionState::Unsupported {
                advise_unsupported(g, cap.web_name());
            }
            set_i(g, arr, i as u32, row(g, cap));
        }
        arr
    }
});

/// Asks for `name` and resolves with a fresh read of the state, never optimistically.
pub unsafe fn request(g: *mut c_void, name: &str) -> u64 {
    let Some(cap) = Capability::from_web_name(name) else {
        return host::rejected(g, "TypeError", &format!("{name} is not a permission name"));
    };
    let outcome = match api::host() {
        None => RequestOutcome::Settled(PermissionState::Unsupported),
        Some(h) => h.request(cap),
    };
    match outcome {
        RequestOutcome::Settled(state) => {
            if state == PermissionState::Unsupported {
                advise_unsupported(g, name);
            }
            host::resolved(g, js_string(g, state.as_str()))
        }
        RequestOutcome::Pending(token) => {
            let (p, s) = host::deferred(g);
            PENDING.with(|m| m.borrow_mut().entry(token).or_default().push(s));
            p
        }
    }
}

host_fn!(fn request_fn(g, args) {
    unsafe {
        let name = host::string_arg(g, args.arg(0).0).unwrap_or_default();
        request(g, &name)
    }
});

host_fn!(fn on_status_change(g, args) {
    unsafe {
        let obj = OBJ.with(|o| o.get());
        let Some(cap) = read_string(g, args.arg(0).0).as_deref().and_then(Capability::from_web_name) else { return UNDEF };
        let ev = events::new_trusted_event(g, "change");
        define(g, ev, "name", js_string(g, cap.web_name()), 1);
        define(g, ev, "state", state_str(g, cap), 1);
        events::dispatch(g, obj, ev);
    }
    UNDEF
});

/// Resolves the requests that settled; true when any JS ran.
pub(crate) unsafe fn settle(g: *mut c_void, settled: Vec<(u32, PermissionState)>) -> bool {
    let mut ran = false;
    for (token, state) in settled {
        let Some(waiters) = PENDING.with(|m| m.borrow_mut().remove(&token)) else { continue };
        for s in waiters {
            s.resolve(g, js_string(g, state.as_str()));
            ran = true;
        }
    }
    ran
}

pub(crate) fn installed() -> bool {
    OBJ.with(|o| o.get()) != 0
}

pub(crate) unsafe fn install(g: *mut c_void) {
    let event_target = get(g, get(g, host::global(g), "EventTarget"), "prototype");
    let obj = host::install_object(g, "__VN_PERMISSIONS__", event_target);
    OBJ.with(|o| o.set(obj));
    host::getter(g, obj, "platform", platform);
    host::getter(g, obj, "names", names);
    host::method(g, obj, "query", 1, query);
    host::method(g, obj, "list", 0, list);
    host::method(g, obj, "request", 1, request_fn);
    // One `change` stream from the standard objects: a second native subscription would race for the same queue.
    let listener = make_fn(g, "", 1, on_status_change);
    for cap in Capability::ALL.iter().copied() {
        let Some(status) = api::status_object(g, cap) else { continue };
        let Ok(bound) = call(g, get(g, listener, "bind"), listener, &[UNDEF, js_string(g, cap.web_name())]) else { continue };
        let _ = call(g, get(g, status, "addEventListener"), status, &[js_string(g, "change"), bound]);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use vn_runtime::redjs::api::Value;
    use vn_runtime::JsRuntime;

    use super::*;
    use crate::stub::StubHost;

    fn stub() -> &'static StubHost {
        static H: OnceLock<StubHost> = OnceLock::new();
        H.get_or_init(|| {
            StubHost::scripted(
                "TestOS",
                &[
                    (Capability::Microphone, PermissionState::Granted),
                    (Capability::Camera, PermissionState::Denied),
                    (Capability::DisplayCapture, PermissionState::Prompt),
                ],
            )
            .granting(PermissionState::Granted)
        })
    }

    fn eval(rt: &JsRuntime, src: &str) -> String {
        rt.ctx().eval(src).and_then(|v: Value| rt.ctx().to_rust_string(v)).unwrap_or_else(|_| "<threw>".into())
    }

    #[test]
    fn query_list_request_and_change() {
        vn_runtime::testing::on_js_thread(|| {
            api::set_host(stub());
            JsRuntime::with(|rt| unsafe {
                install(rt.raw());
                let sync = eval(
                    rt,
                    "const p = __VN_PERMISSIONS__; const r = [p.platform, p.names.join('+'), p.query('microphone'), p.query('camera'), \
                     p.query('display-capture'), p.query('geolocation')]; \
                     try { p.query('mic'); } catch (e) { r.push(e instanceof TypeError); } \
                     const rows = p.list(); r.push(rows.length, JSON.stringify(rows[1]), rows[6].canRequest, p instanceof EventTarget); r.join('|')",
                );
                assert_eq!(
                    sync,
                    "TestOS|microphone+camera+display-capture+notifications+clipboard-read+clipboard-write+geolocation|granted|denied|prompt|unsupported|true|7|\
                     {\"name\":\"camera\",\"state\":\"denied\",\"canRequest\":true,\"deniedBySystem\":true}|false|true"
                );
                eval(
                    rt,
                    "globalThis.seen = []; __VN_PERMISSIONS__.addEventListener('change', (e) => seen.push(e.name + ':' + e.state)); \
                     __VN_PERMISSIONS__.request('camera').then((s) => seen.push('req:' + s)); \
                     __VN_PERMISSIONS__.request('microphone').then((s) => seen.push('now:' + s)); \
                     __VN_PERMISSIONS__.request('nope').catch((e) => seen.push(e.name)); ''",
                );
                rt.ctx().drain_microtasks();
                assert_eq!(eval(rt, "seen.join()"), "now:granted,TypeError");
                stub().set(Capability::Microphone, PermissionState::Denied);
                api::pump(rt.raw());
                settle(rt.raw(), api::take_settled());
                rt.ctx().drain_microtasks();
                assert_eq!(eval(rt, "seen.join()"), "now:granted,TypeError,camera:granted,microphone:denied,req:granted");
            });
        });
    }
}
