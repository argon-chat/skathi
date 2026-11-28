//! Web APIs Skathi has no implementation for, defined so code written for a browser keeps running:
//! `alert`, `confirm`, `prompt` and `Notification` advise once and answer as a browser does when the
//! user dismisses or denies. `customElements` has no element defined and refuses `define()`.

use std::cell::Cell;
use std::ffi::c_void;

use redjs::webapi::util::*;

use crate::host::{global, rejected, resolved, throw, warn_compat};

const ALERT: &str = "alert() is not supported in Skathi: no dialog is shown. Show the message in the app's own UI instead.";
const CONFIRM: &str =
    "confirm() is not supported in Skathi: no dialog is shown and it answers false. Ask in the app's own UI instead.";
const PROMPT: &str = "prompt() is not supported in Skathi: no dialog is shown and it answers null. Ask for the text \
                      in the app's own UI instead.";
const NOTIFICATION: &str =
    "Notification is not supported in Skathi: nothing is shown and its permission is always \
                            \"denied\". Show the message in the app's own UI instead.";
const CUSTOM_ELEMENTS: &str = "Skathi has no custom elements (no shadow DOM, nothing can be defined in \
                               customElements): use the component directly instead of defining an element for it.";

thread_local! {
    static NOTIFICATION_PROTO: Cell<u64> = const { Cell::new(0) };
}

crate::host_fn!(fn js_alert(g, _args) {
    unsafe { warn_compat(g, "alert", ALERT) };
    UNDEF
});

crate::host_fn!(fn js_confirm(g, _args) {
    unsafe { warn_compat(g, "confirm", CONFIRM) };
    FALSE
});

crate::host_fn!(fn js_prompt(g, _args) {
    unsafe { warn_compat(g, "prompt", PROMPT) };
    NULL
});

unsafe fn option_string(g: *mut c_void, options: u64, name: &str) -> u64 {
    unsafe {
        let v = if is_cell(options) {
            get(g, options, name)
        } else {
            UNDEF
        };
        match v {
            UNDEF => js_string(g, ""),
            v => to_js_string(g, v).unwrap_or_else(|_| js_string(g, "")),
        }
    }
}

crate::host_fn!(fn notification_construct(g, args) {
    unsafe {
        warn_compat(g, "Notification", NOTIFICATION);
        if args.argc() == 0 {
            return throw_type_error(g, "Failed to construct 'Notification': 1 argument required, but only 0 present.");
        }
        let Ok(title) = to_js_string(g, args.arg(0).0) else { return UNDEF };
        let options = args.arg(1).0;
        let obj = ordinary_create(g, args.this().0, NOTIFICATION_PROTO.with(|p| p.get()));
        define(g, obj, "title", title, 1);
        for name in ["body", "tag", "icon"] {
            define(g, obj, name, option_string(g, options, name), 1);
        }
        let data = if is_cell(options) { get(g, options, "data") } else { UNDEF };
        define(g, obj, "data", if data == UNDEF { NULL } else { data }, 1);
        for handler in ["onclick", "onclose", "onerror", "onshow"] {
            define(g, obj, handler, NULL, 0);
        }
        obj
    }
});

crate::host_fn!(fn noop(_g, _args) {
    UNDEF
});

crate::host_fn!(fn notification_permission(g, _args) {
    unsafe { js_string(g, "denied") }
});

crate::host_fn!(fn notification_request_permission(g, args) {
    unsafe {
        warn_compat(g, "Notification", NOTIFICATION);
        let denied = js_string(g, "denied");
        let callback = args.arg(0).0;
        if is_cell(callback) {
            let _ = call(g, callback, UNDEF, &[denied]);
        }
        resolved(g, denied)
    }
});

crate::host_fn!(fn elements_define(g, _args) {
    unsafe {
        warn_compat(g, "customElements", CUSTOM_ELEMENTS);
        throw(g, "NotSupportedError", &format!("customElements.define(): {CUSTOM_ELEMENTS}"))
    }
});

crate::host_fn!(fn elements_get(_g, _args) {
    UNDEF
});

crate::host_fn!(fn elements_get_name(_g, _args) {
    NULL
});

crate::host_fn!(fn elements_when_defined(g, _args) {
    unsafe { rejected(g, "NotSupportedError", &format!("customElements.whenDefined(): {CUSTOM_ELEMENTS}")) }
});

/// Replaces redjs's own `alert`, `confirm` and `prompt`, which throw without a host dialog handler, and
/// defines `customElements`.
pub(crate) unsafe fn install(g: *mut c_void) {
    unsafe {
        let registry = define_class(
            g,
            &ClassSpec {
                name: "CustomElementRegistry",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[
                    ("define", 2, elements_define),
                    ("get", 1, elements_get),
                    ("getName", 1, elements_get_name),
                    ("upgrade", 1, noop),
                    ("whenDefined", 1, elements_when_defined),
                ],
                getters: &[],
                accessors: &[],
            },
        );
        let elements = new_object(g, registry.proto);
        define(g, global(g), "customElements", elements, 2);

        let global = global(g);
        define(g, global, "alert", make_fn(g, "alert", 0, js_alert), 2);
        define(
            g,
            global,
            "confirm",
            make_fn(g, "confirm", 0, js_confirm),
            2,
        );
        define(g, global, "prompt", make_fn(g, "prompt", 0, js_prompt), 2);

        let event_target = get(g, get(g, global, "EventTarget"), "prototype");
        let class = define_class(
            g,
            &ClassSpec {
                name: "Notification",
                arity: 1,
                construct: Some(notification_construct),
                parent_proto: if is_cell(event_target) {
                    event_target
                } else {
                    0
                },
                methods: &[("close", 0, noop)],
                getters: &[],
                accessors: &[],
            },
        );
        NOTIFICATION_PROTO.with(|p| p.set(class.proto));
        let permission = atom(g, "permission");
        redjs::glue::rjs_define_accessor(
            g,
            class.ctor,
            permission,
            make_fn(g, "get permission", 0, notification_permission),
            UNDEF,
            2,
        );
        redjs::glue::rjs_atom_free(g, permission);
        define(
            g,
            class.ctor,
            "requestPermission",
            make_fn(g, "requestPermission", 0, notification_request_permission),
            2,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::on_js_thread;
    use crate::JsRuntime;

    fn eval(src: &'static str) -> String {
        on_js_thread(move || {
            JsRuntime::with(|rt| {
                let v = rt.ctx().eval(src).expect(src);
                rt.ctx().to_rust_string(v).unwrap_or_default()
            })
        })
    }

    #[test]
    fn dialogs_and_notifications_answer_as_dismissed_and_warn_once() {
        eval(
            "globalThis.__warned = []; globalThis.__warn = console.warn; \
             console.warn = (m) => __warned.push(String(m)); 'ok'",
        );
        assert_eq!(
            eval("[alert('a'), alert('b'), confirm('c'), prompt('d', 'e')].map(String).join('|')"),
            "undefined|undefined|false|null"
        );
        assert_eq!(
            eval(
                "const n = new Notification('Hi', { body: 'there', tag: 't' }); n.close(); \
                 [n.title, n.body, n.tag, n.icon, String(n.data), n instanceof Notification, \
                  n instanceof EventTarget, Notification.permission].join('|')"
            ),
            "Hi|there|t||null|true|true|denied"
        );
        eval(
            "globalThis.__asked = 'pending'; \
             Notification.requestPermission().then((p) => { __asked = p; }); 'ok'",
        );
        on_js_thread(|| JsRuntime::with(|rt| rt.ctx().drain_microtasks()));
        assert_eq!(eval("__asked"), "denied");
        assert_eq!(eval("String(Object.keys(Notification).length)"), "0");
        assert_eq!(
            eval("(() => { try { new Notification(); return 'no throw'; } catch (e) { return e.name; } })()"),
            "TypeError"
        );
        let warned = eval("console.warn = __warn; __warned.join('\\n')");
        let ours = warned
            .lines()
            .filter(|l| l.contains("is not supported in Skathi"));
        assert_eq!(ours.count(), 4, "{warned}");
    }

    #[test]
    fn custom_elements_define_nothing_and_refuse_define() {
        assert_eq!(
            eval("String(customElements.get('sk-surface'))"),
            "undefined"
        );
        assert_eq!(
            eval("(() => { try { customElements.define('x-y', class extends Object {}); return 'no throw'; } catch (e) { return e.name; } })()"),
            "NotSupportedError"
        );
    }
}
