//! `location.reload()`: re-run the app the way the dev host's cold restart does.
//!
//! A browser's reload discards the document and its realm. Skathi has one JS realm for the life
//! of the process, so a reload here is the existing restart path, on request.
//!
//! The host fn only sets a flag. The work runs at the start of the next frame ([`pump`]), never
//! inside the call stack that asked, because tearing the app down under its own handler would leave
//! that handler running against freed nodes.

use std::cell::Cell;

use sk_runtime::redjs::api::Value;
#[cfg(not(feature = "no-interpreter"))]
use sk_runtime::redjs::{
    api::{Context, Error},
    glue,
};
use sk_runtime::{host_fn, JsRuntime};

thread_local! {
    static REQUESTED: Cell<bool> = const { Cell::new(false) };
}

host_fn!(fn sk_reload(_g, _args) {
    REQUESTED.with(|r| r.set(true));
    Value::undefined().0
});

pub fn install(rt: &JsRuntime) {
    rt.register("__sk_reload", 0, sk_reload as *const _);
}

pub fn pump() {
    if !REQUESTED.with(|r| r.replace(false)) {
        return;
    }
    #[cfg(feature = "no-interpreter")]
    eprintln!("[sk] location.reload() requires the interpreter to re-run this app; restart the application instead");
    #[cfg(not(feature = "no-interpreter"))]
    JsRuntime::with(run);
}

#[cfg(not(feature = "no-interpreter"))]
fn run(rt: &JsRuntime) {
    eprintln!("[sk] location.reload() — unloading the app and running it again (same JS realm)");
    // `__SK_UNLOAD__` runs the teardown itself, between `pagehide` and the
    // realm hygiene; a runtime without it (older bundle) is only torn down.
    if let Err(error) = unload(rt.ctx()) {
        if let Error::Exception(value) = error {
            let e = rt.describe(value);
            eprintln!("[sk] reload teardown: {}\n{}", e.header, e.stack);
        } else {
            eprintln!("[sk] reload teardown: {error}");
        }
    }
    end_media(rt, "location.reload()");
    crate::pressure_observer::clear(rt);
    crate::js::reload(rt.ctx());
    #[cfg(sk_devtools)]
    crate::devtools::mark_reload("location.reload()");
    crate::boot_mode::reboot(rt);
}

#[cfg(not(feature = "no-interpreter"))]
fn unload(context: &Context) -> Result<(), Error> {
    for name in ["__SK_UNLOAD__", "__SK_TEARDOWN__"] {
        let key = context.atom(name);
        let hook = context.get(context.global_get("globalThis")?, &key)?;
        if unsafe { glue::rjs_to_boolean(context.raw(), hook.0) } != 0 {
            // The old script's property call observes a getter again and passes its receiver as this.
            let receiver = context.global_get("globalThis")?;
            let target = context.get(receiver, &key)?;
            context.call(target, receiver, &[]).map_err(|e| match e {
                Error::BadArg => {
                    Error::Exception(context.error(1, &format!("{name} is not a function")))
                }
                other => other,
            })?;
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "no-interpreter")]
    #[test]
    fn reload_leaves_the_realm_alone_without_an_interpreter() {
        REQUESTED.with(|request| request.set(true));
        pump();
        assert!(!REQUESTED.with(Cell::get));
    }

    #[cfg(not(feature = "no-interpreter"))]
    #[test]
    fn unload_prefers_the_hook_and_preserves_the_global_receiver() {
        sk_runtime::testing::on_js_thread(|| {
            let context = Context::new();
            context
                .eval(
                    r#"
                globalThis.calls = "";
                Object.defineProperty(globalThis, "__SK_UNLOAD__", { configurable: true, get() {
                    calls += "g";
                    return function () { "use strict"; calls += this === globalThis ? "u" : "!"; };
                }});
                globalThis.__SK_TEARDOWN__ = function () { calls += "t"; };
            "#,
                )
                .unwrap();
            unload(&context).unwrap();
            context.eval("delete globalThis.__SK_UNLOAD__").unwrap();
            unload(&context).unwrap();
            assert_eq!(
                context
                    .read_string(context.global_get("calls").unwrap())
                    .unwrap(),
                "ggut"
            );
        });
    }
}

pub fn end_media(rt: &JsRuntime, why: &str) {
    #[cfg(feature = "audio")]
    let (tracks, contexts) = {
        use sk_runtime::redjs::webapi::{audio, mediastream};
        let g = rt.ctx().raw();
        (mediastream::stop_all(g), audio::close_all(g))
    };
    #[cfg(not(feature = "audio"))]
    let (tracks, contexts) = {
        let _ = rt;
        (0usize, 0usize)
    };
    if tracks + contexts > 0 {
        eprintln!(
            "[sk] {why}: the old page's media ended — {tracks} capture track(s) stopped, \
             {contexts} AudioContext(s) closed"
        );
    }
}
