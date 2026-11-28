//! The Launch Handler API (`launchQueue.setConsumer`) and `appLaunch`. This process's own launch is
//! buffered until a consumer is set, and every launch a second process forwards (`singleInstance`)
//! is another. `appLaunch` adds what a browser cannot say: raw argv, cwd, existing-path arguments,
//! the single-instance lock, the `launch` event. Deep links and argv are untrusted input.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value as Json;
use sk_runtime::host;
use sk_runtime::host_fn;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::events;
use sk_runtime::redjs::webapi::util::*;

use super::{fail_value, Errors};
use crate::launch;

struct Params {
    url: Option<String>,
}

#[derive(Default)]
struct State {
    obj: u64,
    argv: u64,
    paths: u64,
    cwd: String,
    url: Option<String>,
    single_instance: bool,
    params_class: usize,
    params_proto: u64,
    s_files: usize,
    consumer: u64,
    queued: Vec<(Option<String>, Json)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static DELIVERING: Cell<bool> = const { Cell::new(false) };
}

static OWN_LAUNCH: AtomicBool = AtomicBool::new(false);

fn class() -> JSClassRef {
    STATE.with(|s| s.borrow().params_class) as JSClassRef
}

unsafe fn params<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Params> {
    unsafe { host_data::<Params>(g, class(), v) }
}

unsafe fn strings(g: *mut c_void, v: &Json) -> u64 {
    unsafe {
        let items: Vec<&str> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .collect();
        host::frozen_strings(g, &items)
    }
}

unsafe fn make_params(g: *mut c_void, url: Option<String>, handles: &Json) -> u64 {
    unsafe {
        let (proto, slot) = STATE.with(|s| {
            let s = s.borrow();
            (s.params_proto, s.s_files)
        });
        let files = new_array(g);
        let mut n = 0;
        for wire in handles
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| w["h"].is_u64())
        {
            set_i(g, files, n, super::fsa::from_wire(g, wire));
            n += 1;
        }
        let object = get(g, host::global(g), "Object");
        let files = call(g, get(g, object, "freeze"), object, &[files]).unwrap_or(files);
        let obj = host_create(g, class(), proto, Params { url });
        glue::rjs_define(g, obj, slot as *mut c_void, files, 0);
        obj
    }
}

unsafe fn deliver(g: *mut c_void, consumer: u64, url: Option<String>, handles: &Json) {
    unsafe {
        let p = make_params(g, url, handles);
        if let Err(e) = call(g, consumer, UNDEF, &[p]) {
            // A consumer that throws must not take the next launch with it.
            let stack = get(g, e, "stack");
            let text = host::string_arg(
                g,
                if is_cell(e) && stack != UNDEF {
                    stack
                } else {
                    e
                },
            )
            .unwrap_or_default();
            let console = get(g, host::global(g), "console");
            let _ = call(
                g,
                get(g, console, "error"),
                console,
                &[js_string(
                    g,
                    &format!("[skathi] launchQueue consumer threw: {text}"),
                )],
            );
        }
    }
}

unsafe fn enqueue(g: *mut c_void, url: Option<String>, handles: Json) {
    unsafe {
        let consumer = STATE.with(|s| s.borrow().consumer);
        if consumer == 0 {
            STATE.with(|s| s.borrow_mut().queued.push((url, handles)));
        } else {
            deliver(g, consumer, url, &handles);
        }
    }
}

host_fn!(fn set_consumer(g, args) {
    unsafe {
        let f = args.arg(0).0;
        if !events::is_callable(g, f) {
            return host::throw(g, "TypeError", "Failed to execute 'setConsumer' on 'LaunchQueue': parameter 1 is not a function.");
        }
        redjs_protect(g, f);
        let old = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().consumer, f));
        if old != 0 {
            redjs_unprotect(g, old);
        }
        if DELIVERING.with(|d| d.replace(true)) {
            return UNDEF;
        }
        loop {
            let next = STATE.with(|s| {
                let mut s = s.borrow_mut();
                (!s.queued.is_empty()).then(|| s.queued.remove(0))
            });
            let Some((url, handles)) = next else { break };
            let consumer = STATE.with(|s| s.borrow().consumer);
            deliver(g, consumer, url, &handles);
        }
        DELIVERING.with(|d| d.set(false));
    }
    UNDEF
});

host_fn!(fn params_target_url(g, args) {
    unsafe {
        match params(g, args.this().0) {
            Some(p) => p.url.as_deref().map_or(NULL, |u| js_string(g, u)),
            None => throw_type_error(g, "Illegal invocation"),
        }
    }
});

host_fn!(fn params_files(g, args) {
    unsafe {
        let this = args.this().0;
        if params(g, this).is_none() {
            return throw_type_error(g, "Illegal invocation");
        }
        get_a(g, this, STATE.with(|s| s.borrow().s_files) as *mut c_void)
    }
});

/// A second launch forwarded here, or the app reopened.
pub(super) unsafe fn on_launch(g: *mut c_void, p: &Json) {
    unsafe {
        let obj = STATE.with(|s| s.borrow().obj);
        if obj == 0 {
            return;
        }
        let url = p["url"].as_str().map(str::to_string);
        let ev = events::new_trusted_event(g, "launch");
        define(g, ev, "argv", strings(g, &p["argv"]), 1);
        define(
            g,
            ev,
            "cwd",
            js_string(g, p["cwd"].as_str().unwrap_or("")),
            1,
        );
        define(
            g,
            ev,
            "url",
            url.as_deref().map_or(NULL, |u| js_string(g, u)),
            1,
        );
        define(g, ev, "paths", strings(g, &p["paths"]), 1);
        define(
            g,
            ev,
            "source",
            js_string(g, p["source"].as_str().unwrap_or("second-instance")),
            1,
        );
        events::dispatch(g, obj, ev);
        enqueue(g, url, p["handles"].clone());
    }
}

host_fn!(fn get_argv(_g, _args) {
    STATE.with(|s| s.borrow().argv)
});

host_fn!(fn get_paths(_g, _args) {
    STATE.with(|s| s.borrow().paths)
});

host_fn!(fn get_cwd(g, _args) {
    unsafe { js_string(g, &STATE.with(|s| s.borrow().cwd.clone())) }
});

host_fn!(fn get_url(g, _args) {
    unsafe { STATE.with(|s| s.borrow().url.clone()).map_or(NULL, |u| js_string(g, &u)) }
});

host_fn!(fn get_single_instance(_g, _args) {
    enc_bool(STATE.with(|s| s.borrow().single_instance))
});

host_fn!(fn register_protocol(g, args) {
    unsafe {
        let scheme = host::string_arg(g, args.arg(0).0).unwrap_or_default();
        match launch::js_register(&scheme) {
            Ok(()) => UNDEF,
            Err(f) => {
                glue::rjs_throw(g, fail_value(g, Errors::Desktop, &f));
                UNDEF
            }
        }
    }
});

host_fn!(fn unregister_protocol(g, args) {
    unsafe { enc_bool(launch::js_unregister(&host::string_arg(g, args.arg(0).0).unwrap_or_default())) }
});

host_fn!(fn is_protocol_registered(g, args) {
    unsafe { enc_bool(launch::js_is_registered(&host::string_arg(g, args.arg(0).0).unwrap_or_default())) }
});

pub(super) unsafe fn install(g: *mut c_void, event_target: u64) {
    unsafe {
        let info: Json = serde_json::from_str(&launch::info_json()).unwrap_or_default();
        let params_class = make_host_class::<Params>("LaunchParams");
        let params_cls = define_class(
            g,
            &ClassSpec {
                name: "LaunchParams",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[],
                getters: &[("targetURL", params_target_url), ("files", params_files)],
                accessors: &[],
            },
        );
        let queue_cls = define_class(
            g,
            &ClassSpec {
                name: "LaunchQueue",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[("setConsumer", 1, set_consumer)],
                getters: &[],
                accessors: &[],
            },
        );
        let queue = new_object(g, queue_cls.proto);
        redjs_protect(g, queue);
        define(g, host::global(g), "launchQueue", queue, 0);

        let obj = host::install_object(g, "__SK_LAUNCH__", event_target);
        let (argv, paths) = (strings(g, &info["argv"]), strings(g, &info["paths"]));
        redjs_protect(g, argv);
        redjs_protect(g, paths);
        STATE.with(|s| {
            *s.borrow_mut() = State {
                obj,
                argv,
                paths,
                cwd: info["cwd"].as_str().unwrap_or("").to_string(),
                url: info["url"].as_str().map(str::to_string),
                single_instance: info["singleInstance"].as_bool().unwrap_or(false),
                params_class: params_class as usize,
                params_proto: params_cls.proto,
                s_files: atom_priv(g, "sk#launchFiles") as usize,
                consumer: 0,
                queued: Vec::new(),
            }
        });
        host::getter(g, obj, "argv", get_argv);
        host::getter(g, obj, "cwd", get_cwd);
        host::getter(g, obj, "url", get_url);
        host::getter(g, obj, "paths", get_paths);
        host::getter(g, obj, "singleInstance", get_single_instance);
        host::method(g, obj, "registerProtocol", 1, register_protocol);
        host::method(g, obj, "unregisterProtocol", 1, unregister_protocol);
        host::method(g, obj, "isProtocolRegistered", 1, is_protocol_registered);

        // Once per process: a reload is not a launch.
        if take_own_launch(&OWN_LAUNCH, crate::args::process_toast_activation()) {
            enqueue(
                g,
                info["url"].as_str().map(str::to_string),
                info["handles"].clone(),
            );
        }
    }
}

fn take_own_launch(seen: &AtomicBool, toast_activation: bool) -> bool {
    !seen.swap(true, Ordering::AcqRel) && !toast_activation
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sk_runtime::JsRuntime;

    #[test]
    fn a_toast_launch_does_not_enqueue_an_ordinary_launch_on_startup_or_reload() {
        let seen = super::AtomicBool::new(false);
        assert!(!super::take_own_launch(&seen, true));
        assert!(!super::take_own_launch(&seen, false));

        let seen = super::AtomicBool::new(false);
        assert!(super::take_own_launch(&seen, false));
        assert!(!super::take_own_launch(&seen, false));
    }

    #[test]
    fn launches_queue_until_a_consumer_and_fire_the_event() {
        let out = super::super::testing::run(
            "globalThis.log = []; const r = [typeof __SK_LAUNCH__.argv.length, Object.isFrozen(__SK_LAUNCH__.paths), __SK_LAUNCH__.singleInstance, \
             launchQueue instanceof LaunchQueue]; \
             try { new LaunchParams(); } catch (e) { r.push(e instanceof TypeError); } \
             try { launchQueue.setConsumer(1); } catch (e) { r.push(e instanceof TypeError); } \
             __SK_LAUNCH__.addEventListener('launch', (e) => log.push('event:' + e.url + ':' + e.source + ':' + e.argv.join(' '))); \
             launchQueue.setConsumer((p) => { log.push('own:' + p.targetURL + ':' + p.files.length + ':' + (p instanceof LaunchParams)); }); \
             return r.join('|');",
        );
        assert_eq!(out, "number|true|false|true|true|true");
        let log = sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| unsafe {
                let g = rt.raw();
                super::on_launch(
                    g,
                    &json!({ "kind": "launch", "source": "open-url", "argv": ["app", "x://y"], "cwd": "/", "url": "x://y", "paths": [], "handles": [] }),
                );
                rt.ctx().eval("launchQueue.setConsumer(() => { throw new Error('boom'); }); launchQueue.setConsumer((p) => log.push('next:' + p.targetURL)); ''").unwrap();
                super::on_launch(
                    g,
                    &json!({ "argv": [], "cwd": "", "url": null, "paths": [] }),
                );
                rt.ctx()
                    .eval("log.join('|')")
                    .and_then(|v| rt.ctx().to_rust_string(v))
                    .unwrap()
            })
        });
        assert_eq!(log, "own:null:0:true|event:x://y:open-url:app x://y|own:x://y:0:true|event:null:second-instance:|next:null");
    }
}
