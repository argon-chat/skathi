//! Screen Wake Lock: one system request while any sentinel is active; a hidden page releases them all.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use sk_runtime::host_fn;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::webapi::events;
use sk_runtime::redjs::webapi::util::*;

use super::{handler_get, handler_set, rejected};

struct Sentinel {
    released: bool,
}

thread_local! {
    static CLASS: Cell<usize> = const { Cell::new(0) };
    static PROTO: Cell<u64> = const { Cell::new(0) };
    static WAKE_LOCK: Cell<u64> = const { Cell::new(0) };
    static ACTIVE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

fn class() -> JSClassRef {
    CLASS.with(|c| c.get()) as JSClassRef
}

unsafe fn sentinel<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Sentinel> {
    unsafe { host_data::<Sentinel>(g, class(), v) }
}

unsafe fn page_hidden(g: *mut c_void) -> bool {
    unsafe { get(g, state(g).document, "hidden") == TRUE }
}

unsafe fn release(g: *mut c_void, obj: u64) {
    unsafe {
        let Some(s) = sentinel(g, obj) else { return };
        if s.released {
            return;
        }
        s.released = true;
        let empty = ACTIVE.with(|a| {
            let mut a = a.borrow_mut();
            a.retain(|&o| o != obj);
            a.is_empty()
        });
        if empty {
            crate::power::hold_screen(false);
        }
        events::dispatch(g, obj, events::new_trusted_event(g, "release"));
        redjs_unprotect(g, obj);
    }
}

host_fn!(fn request(g, args) {
    unsafe {
        let v = args.arg(0).0;
        let ty = if v == UNDEF { Some("screen".to_string()) } else { to_js_string(g, v).ok().and_then(|s| read_string(g, s)) };
        if ty.as_deref() != Some("screen") {
            let shown = ty.unwrap_or_default();
            return rejected(g, "TypeError", &format!("Failed to execute 'request' on 'WakeLock': The provided value '{shown}' is not a valid enum value of type WakeLockType."));
        }
        if page_hidden(g) {
            return rejected(g, "NotAllowedError", "The requesting page is not visible");
        }
        if ACTIVE.with(|a| a.borrow().is_empty()) && !crate::power::hold_screen(true) {
            return rejected(g, "NotAllowedError", "The system refused to keep the screen on");
        }
        let obj = host_create(g, class(), PROTO.with(|p| p.get()), Sentinel { released: false });
        redjs_protect(g, obj);
        ACTIVE.with(|a| a.borrow_mut().push(obj));
        resolved_promise(g, obj)
    }
});

host_fn!(fn sentinel_release(g, args) {
    unsafe {
        let this = args.this().0;
        if sentinel(g, this).is_none() {
            return throw_type_error(g, "Illegal invocation");
        }
        release(g, this);
        resolved_promise(g, UNDEF)
    }
});

host_fn!(fn sentinel_released(g, args) {
    unsafe {
        match sentinel(g, args.this().0) {
            Some(s) => enc_bool(s.released),
            None => throw_type_error(g, "Illegal invocation"),
        }
    }
});

host_fn!(fn sentinel_type(g, args) {
    unsafe {
        match sentinel(g, args.this().0) {
            Some(_) => js_string(g, "screen"),
            None => throw_type_error(g, "Illegal invocation"),
        }
    }
});

host_fn!(fn get_onrelease(g, args) {
    unsafe { handler_get(g, args.this().0, "onrelease") }
});

host_fn!(fn set_onrelease(g, args) {
    unsafe { handler_set(g, args.this().0, "onrelease", args.arg(0).0) };
    UNDEF
});

host_fn!(fn on_visibility(g, _args) {
    unsafe {
        if page_hidden(g) {
            for obj in ACTIVE.with(|a| a.borrow().clone()) {
                release(g, obj);
            }
        }
    }
    UNDEF
});

host_fn!(fn navigator_wake_lock(_g, _args) {
    WAKE_LOCK.with(|w| w.get())
});

pub(super) unsafe fn install(g: *mut c_void, nav: u64, event_target: u64) {
    unsafe {
        CLASS.with(|c| c.set(make_host_class::<Sentinel>("WakeLockSentinel") as usize));
        let sentinel = define_class(
            g,
            &ClassSpec {
                name: "WakeLockSentinel",
                arity: 0,
                construct: None,
                parent_proto: event_target,
                methods: &[("release", 0, sentinel_release)],
                getters: &[("released", sentinel_released), ("type", sentinel_type)],
                accessors: &[("onrelease", get_onrelease, set_onrelease)],
            },
        );
        PROTO.with(|p| p.set(sentinel.proto));
        let wake_lock = define_class(
            g,
            &ClassSpec {
                name: "WakeLock",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[("request", 0, request)],
                getters: &[],
                accessors: &[],
            },
        );
        let instance = new_object(g, wake_lock.proto);
        redjs_protect(g, instance);
        WAKE_LOCK.with(|w| w.set(instance));
        super::define_getter(g, nav, "wakeLock", navigator_wake_lock);
        let document = state(g).document;
        let add = get(g, document, "addEventListener");
        let _ = call(
            g,
            add,
            document,
            &[
                js_string(g, "visibilitychange"),
                make_fn(g, "", 0, on_visibility),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sentinels_share_one_request_and_a_hidden_page_releases_them() {
        let out = super::super::testing::run(
            "const log = []; \
             const a = await navigator.wakeLock.request('screen'); const b = await navigator.wakeLock.request(); \
             a.onrelease = (e) => log.push('on:' + e.type); a.addEventListener('release', () => log.push('listener')); \
             log.push(a.type, a.released, a instanceof EventTarget, a instanceof WakeLockSentinel); \
             await a.release(); await a.release(); log.push(a.released); \
             try { await navigator.wakeLock.request('system'); } catch (e) { log.push(e instanceof TypeError); } \
             try { new WakeLockSentinel(); } catch (e) { log.push('ctor:' + (e instanceof TypeError)); } \
             Object.defineProperty(document, 'hidden', { value: true, configurable: true }); \
             document.dispatchEvent(new Event('visibilitychange')); log.push(b.released); \
             try { await navigator.wakeLock.request(); } catch (e) { log.push(e.name); } \
             Object.defineProperty(document, 'hidden', { value: false, configurable: true }); \
             return log.join();",
        );
        assert_eq!(
            out,
            "screen,false,true,true,listener,on:release,true,true,ctor:true,true,NotAllowedError"
        );
        assert!(crate::power::hold_screen(false));
    }
}
