//! `appDevtools`: an app switching the devtools off at runtime (`disable()`, or `"devtools": false`),
//! back on, or opening the inspector. Without a [`DevtoolsHost`] (`--features no-devtools`) it answers
//! inertly instead of throwing, so `disable()` in an entry module is harmless.

use std::cell::RefCell;
use std::ffi::c_void;

use redjs::ffi::{redjs_protect, redjs_unprotect};
use redjs::webapi::util::*;

use crate::host::{self, warn_compat};

#[derive(Default)]
pub struct DevtoolsInfo {
    pub enabled: bool,
    /// `SK_DEVTOOLS=1`, which refuses `disable()`.
    pub locked: bool,
    pub port: Option<u16>,
    pub pid: u32,
    /// The `SK_DEVTOOLS_KEY` gate on F12.
    pub key: bool,
    pub inspectors: usize,
    /// The dist slot whose `devtools.json` the host publishes into.
    pub slot: String,
}

pub trait DevtoolsHost {
    fn info(&self) -> DevtoolsInfo;
    /// The state afterwards, or why the change was refused.
    fn set_enabled(&self, on: bool) -> Result<bool, String>;
    fn open(&self) -> Result<bool, String>;
}

thread_local! {
    static HOST: RefCell<Option<Box<dyn DevtoolsHost>>> = const { RefCell::new(None) };
    static WATCHERS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

const NO_DEVTOOLS: &str = "this host was built with `--features no-devtools`";

fn info() -> Option<DevtoolsInfo> {
    HOST.with(|h| h.borrow().as_ref().map(|h| h.info()))
}

fn enabled() -> bool {
    info().is_some_and(|i| i.enabled)
}

unsafe fn notify(g: *mut c_void, on: bool) {
    unsafe {
        let watchers = WATCHERS.with(|w| w.borrow().clone());
        for f in watchers {
            if let Err(e) = call(g, f, UNDEF, &[enc_bool(on)]) {
                let e = host::string_arg(g, e).unwrap_or_default();
                warn_compat(
                    g,
                    "appDevtools.onChange",
                    &format!("a change listener threw and was ignored: {e}"),
                );
            }
        }
    }
}

unsafe fn set(g: *mut c_void, on: bool) -> u64 {
    unsafe {
        let was = enabled();
        let Some(outcome) = HOST.with(|h| h.borrow().as_ref().map(|h| h.set_enabled(on))) else {
            if on {
                warn_compat(
                g,
                "appDevtools",
                &format!("{NO_DEVTOOLS}, so there are no devtools to enable — `appDevtools.enable()` did nothing."),
            );
            }
            return FALSE;
        };
        match outcome {
            Ok(now) => {
                if now != was {
                    notify(g, now);
                }
                enc_bool(now)
            }
            Err(e) => {
                let verb = if on { "enable" } else { "disable" };
                warn_compat(g, "appDevtools", &format!("{verb}() was refused: {e}"));
                enc_bool(was)
            }
        }
    }
}

crate::host_fn!(fn available(_g, _args) {
    enc_bool(HOST.with(|h| h.borrow().is_some()))
});

crate::host_fn!(fn is_enabled(_g, _args) {
    enc_bool(enabled())
});

crate::host_fn!(fn enable(g, _args) {
    unsafe { set(g, true) }
});

crate::host_fn!(fn disable(g, _args) {
    unsafe { set(g, false) }
});

crate::host_fn!(fn open(g, _args) {
    let outcome = HOST.with(|h| h.borrow().as_ref().map(|h| h.open()));
    let refusal = match outcome {
        Some(Ok(opened)) => return enc_bool(opened),
        Some(Err(e)) => format!("open() did not open an inspector: {e}"),
        None => format!("{NO_DEVTOOLS} — `appDevtools.open()` has no inspector to open."),
    };
    unsafe { warn_compat(g, "appDevtools", &refusal) };
    FALSE
});

crate::host_fn!(fn info_js(g, _args) {
    let i = info().unwrap_or_default();
    let info = serde_json::json!({
        "enabled": i.enabled, "locked": i.locked, "port": i.port, "pid": i.pid, "key": i.key,
        "inspectors": i.inspectors, "slot": i.slot,
    });
    unsafe { host::to_js(g, &info) }
});

crate::host_fn!(fn on_change(g, args) {
    unsafe {
        let f = args.arg(0).0;
        if !is_cell(f) || get(g, f, "call") == UNDEF {
            return host::throw(g, "TypeError", "appDevtools.onChange expects a function");
        }
        let fresh = WATCHERS.with(|w| {
            let mut w = w.borrow_mut();
            let fresh = !w.contains(&f);
            if fresh {
                w.push(f);
            }
            fresh
        });
        if fresh {
            redjs_protect(g, f);
        }
        let off = make_fn(g, "off", 0, off);
        call(g, get(g, off, "bind"), off, &[f]).unwrap_or(UNDEF)
    }
});

crate::host_fn!(fn off(g, args) {
    let f = args.this().0;
    let removed = WATCHERS.with(|w| {
        let mut w = w.borrow_mut();
        let before = w.len();
        w.retain(|x| *x != f);
        before != w.len()
    });
    if removed {
        unsafe { redjs_unprotect(g, f) };
    }
    UNDEF
});

/// Installs `__sk_appDevtools` over `host`, or the inert one for `None`.
pub unsafe fn install(g: *mut c_void, host: Option<Box<dyn DevtoolsHost>>) {
    unsafe {
        HOST.with(|h| *h.borrow_mut() = host);
        let obj = host::install_object(g, "__sk_appDevtools", UNDEF);
        host::getter(g, obj, "available", available);
        host::method(g, obj, "isEnabled", 0, is_enabled);
        host::method(g, obj, "enable", 0, enable);
        host::method(g, obj, "disable", 0, disable);
        host::method(g, obj, "open", 0, open);
        host::method(g, obj, "info", 0, info_js);
        host::method(g, obj, "onChange", 1, on_change);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::testing::on_js_thread;
    use crate::JsRuntime;

    #[derive(Default)]
    struct Fake {
        enabled: Cell<bool>,
        locked: Cell<bool>,
        opens: Cell<u32>,
    }

    struct Shared(Rc<Fake>);

    impl DevtoolsHost for Shared {
        fn info(&self) -> DevtoolsInfo {
            let enabled = self.0.enabled.get();
            DevtoolsInfo {
                enabled,
                locked: self.0.locked.get(),
                port: enabled.then_some(49221),
                ..DevtoolsInfo::default()
            }
        }
        fn set_enabled(&self, on: bool) -> Result<bool, String> {
            if !on && self.0.locked.get() {
                return Err("SK_DEVTOOLS=1 holds the devtools on".into());
            }
            self.0.enabled.set(on);
            Ok(on)
        }
        fn open(&self) -> Result<bool, String> {
            if !self.0.enabled.get() {
                return Err("the devtools are disabled".into());
            }
            self.0.opens.set(self.0.opens.get() + 1);
            Ok(true)
        }
    }

    fn eval(src: &str) -> String {
        JsRuntime::with(|rt| {
            let v = rt.ctx().eval(src).expect(src);
            rt.ctx().to_rust_string(v).unwrap_or_default()
        })
    }

    #[test]
    fn changes_reach_listeners_once_and_refusals_do_not() {
        on_js_thread(|| {
            let fake = Rc::new(Fake::default());
            fake.enabled.set(true);
            JsRuntime::with(|rt| unsafe {
                install(rt.raw(), Some(Box::new(Shared(fake.clone()))))
            });
            assert_eq!(
                eval(
                    "globalThis.seen = []; globalThis.dt = __sk_appDevtools; \
                     dt.onChange(() => { throw new Error('boom'); }); \
                     const watch = (on) => seen.push(on); \
                     globalThis.offWatch = dt.onChange(watch); dt.onChange(watch); \
                     [dt.disable(), dt.disable(), dt.info().port, dt.enable(), dt.info().port, seen.join(',')].join('|')"
                ),
                "false|false||true|49221|false,true"
            );
            fake.locked.set(true);
            assert_eq!(
                eval("[dt.disable(), dt.isEnabled(), seen.length].join('|')"),
                "true|true|2"
            );
            fake.locked.set(false);
            assert_eq!(
                eval("[dt.open(), offWatch(), dt.disable(), seen.length, dt.open()].join('|')"),
                "true||false|2|false"
            );
            assert_eq!(fake.opens.get(), 1);
        });
    }

    #[test]
    fn without_a_host_it_answers_inertly() {
        on_js_thread(|| {
            JsRuntime::with(|rt| unsafe { install(rt.raw(), None) });
            assert_eq!(
                eval(
                    "(() => { const dt = __sk_appDevtools; \
                     return [dt.available, dt.isEnabled(), dt.disable(), dt.enable(), dt.open(), dt.info().port, \
                      typeof dt.onChange(() => {})].join('|'); })()"
                ),
                "false|false|false|false|false||function"
            );
        });
    }
}
