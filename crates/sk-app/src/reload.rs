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
    JsRuntime::with(run);
}

fn run(rt: &JsRuntime) {
    eprintln!("[sk] location.reload() — unloading the app and running it again (same JS realm)");
    // `__SK_UNLOAD__` runs the teardown itself, between `pagehide` and the
    // realm hygiene; a runtime without it (older bundle) is only torn down.
    if let Err(e) = rt.eval(
        "if (globalThis.__SK_UNLOAD__) globalThis.__SK_UNLOAD__();\
         else if (globalThis.__SK_TEARDOWN__) globalThis.__SK_TEARDOWN__();",
        "sk://reload.js",
    ) {
        eprintln!("[sk] reload teardown: {}\n{}", e.header, e.stack);
    }
    end_media(rt, "location.reload()");
    crate::js::reload(rt.ctx());
    #[cfg(sk_devtools)]
    crate::devtools::mark_reload("location.reload()");
    crate::boot_mode::reboot(rt);
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
