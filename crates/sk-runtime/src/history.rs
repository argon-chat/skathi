//! `History` and `window.history`: a window holds one document and has no address bar, so there is one
//! entry. `pushState` / `replaceState` keep the state they are given (`history.state` reads it back) and
//! advise once that the URL goes nowhere; `back`, `forward` and `go` have nothing to traverse.

use std::cell::Cell;
use std::ffi::c_void;

use redjs::ffi::{redjs_protect, redjs_unprotect};
use redjs::webapi::util::*;

use crate::host::{global, warn_compat};

const URL_ADVICE: &str = "history.pushState()/replaceState() keep the state but not the URL: a Skathi window \
                          has no address bar and one document, so location does not change. Route with \
                          vue-router's createMemoryHistory() (or keep the route in app state)";
const TRAVERSE_ADVICE: &str =
    "history.back()/forward()/go() do nothing: a Skathi window keeps no session history. \
                               Route with vue-router's createMemoryHistory(), whose back() works";

thread_local! {
    static STATE: Cell<u64> = const { Cell::new(NULL) };
    static LENGTH: Cell<u32> = const { Cell::new(1) };
    static MANUAL: Cell<bool> = const { Cell::new(false) };
    static PROTO: Cell<u64> = const { Cell::new(0) };
}

unsafe fn checked(g: *mut c_void, this: u64) -> bool {
    unsafe { is_cell(this) && proto_chain_has(g, this, PROTO.get()) }
}

unsafe fn keep(g: *mut c_void, args: &redjs::api::HostArgs, push: bool) -> u64 {
    unsafe {
        if !checked(g, args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
        let data = args.arg(0).0;
        let clone = get(g, global(g), "structuredClone");
        let stored = if is_cell(clone) {
            match call(g, clone, UNDEF, &[data]) {
                Ok(v) => v,
                Err(e) => {
                    redjs::glue::rjs_throw(g, e);
                    return UNDEF;
                }
            }
        } else {
            data
        };
        let url = args.arg(2).0;
        if url != UNDEF && url != NULL {
            warn_compat(g, "history-url", URL_ADVICE);
        }
        if is_cell(stored) {
            redjs_protect(g, stored);
        }
        let old = STATE.replace(stored);
        if is_cell(old) {
            redjs_unprotect(g, old);
        }
        if push {
            LENGTH.set(LENGTH.get().saturating_add(1));
        }
        UNDEF
    }
}

crate::host_fn!(fn push_state(g, args) {
    unsafe { keep(g, &args, true) }
});

crate::host_fn!(fn replace_state(g, args) {
    unsafe { keep(g, &args, false) }
});

crate::host_fn!(fn traverse(g, args) {
    unsafe {
        if !checked(g, args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
        warn_compat(g, "history-traverse", TRAVERSE_ADVICE);
    }
    UNDEF
});

crate::host_fn!(fn state_get(g, args) {
    unsafe {
        if !checked(g, args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
    }
    STATE.get()
});

crate::host_fn!(fn length_get(g, args) {
    unsafe {
        if !checked(g, args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
    }
    enc_f64(f64::from(LENGTH.get()))
});

crate::host_fn!(fn scroll_restoration_get(g, args) {
    unsafe {
        if !checked(g, args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
        js_string(g, if MANUAL.get() { "manual" } else { "auto" })
    }
});

crate::host_fn!(fn scroll_restoration_set(g, args) {
    unsafe {
        if !checked(g, args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
        match read_string(g, args.arg(0).0).as_deref() {
            Some("manual") => MANUAL.set(true),
            Some("auto") => MANUAL.set(false),
            _ => {}
        }
    }
    UNDEF
});

pub(crate) unsafe fn install(g: *mut c_void) {
    unsafe {
        let class = define_class(
            g,
            &ClassSpec {
                name: "History",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[
                    ("pushState", 2, push_state),
                    ("replaceState", 2, replace_state),
                    ("back", 0, traverse),
                    ("forward", 0, traverse),
                    ("go", 0, traverse),
                ],
                getters: &[("state", state_get), ("length", length_get)],
                accessors: &[(
                    "scrollRestoration",
                    scroll_restoration_get,
                    scroll_restoration_set,
                )],
            },
        );
        redjs_protect(g, class.proto);
        PROTO.set(class.proto);
        let history = new_object(g, class.proto);
        redjs_protect(g, history);
        define(g, global(g), "history", history, 2);
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
    fn history_keeps_the_pushed_state_in_one_entry() {
        assert_eq!(
            eval(
                "[history instanceof History, history.state, history.length, history.scrollRestoration, \
                 Object.keys(History.prototype).length].join('|')"
            ),
            "true||1|auto|0"
        );
        assert_eq!(
            eval(
                "const pushed = { page: 2 }; history.pushState(pushed, ''); \
                 [history.state.page, history.state === pushed, history.length].join('|')"
            ),
            "2|false|2"
        );
        assert_eq!(
            eval(
                "history.replaceState(null, ''); history.scrollRestoration = 'manual'; history.scrollRestoration = 'x'; \
                 history.back(); [history.state, history.length, history.scrollRestoration].join('|')"
            ),
            "|2|manual"
        );
        assert_eq!(
            eval("(() => { try { history.pushState(() => {}, ''); } catch (e) { return e.name; } })()"),
            "DataCloneError"
        );
        assert_eq!(
            eval("(() => { try { History.prototype.back.call({}); } catch (e) { return e.name; } })()"),
            "TypeError"
        );
    }
}
