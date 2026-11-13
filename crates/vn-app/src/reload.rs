//! `location.reload()`: re-run the app the way the dev host's cold restart does.
//!
//! A browser's reload discards the document and its realm. vue-native has one JS realm for the life
//! of the process, so a reload here is the existing restart path, on request.
//!
//! The host fn only sets a flag. The work runs at the start of the next frame ([`pump`]), never
//! inside the call stack that asked, because tearing the app down under its own handler would leave
//! that handler running against freed nodes.

use std::cell::Cell;

use vn_runtime::redjs::api::Value;
use vn_runtime::{host_fn, JsRuntime};

thread_local! {
    static REQUESTED: Cell<bool> = const { Cell::new(false) };
}

host_fn!(fn vn_reload(_g, _args) {
    REQUESTED.with(|r| r.set(true));
    Value::undefined().0
});

pub fn install(rt: &JsRuntime) {
    rt.register("__vn_reload", 0, vn_reload as *const _);
}

pub fn pump() {
    if !REQUESTED.with(|r| r.replace(false)) {
        return;
    }
    JsRuntime::with(run);
}

fn run(rt: &JsRuntime) {
    eprintln!("[vn] location.reload() — unloading the app and running it again (same JS realm)");
    // `__VN_UNLOAD__` runs the teardown itself, between `pagehide` and the
    // realm hygiene; a runtime without it (older bundle) is only torn down.
    if let Err(e) = rt.eval(
        "if (globalThis.__VN_UNLOAD__) globalThis.__VN_UNLOAD__();\
         else if (globalThis.__VN_TEARDOWN__) globalThis.__VN_TEARDOWN__();",
        "vn://reload.js",
    ) {
        eprintln!("[vn] reload teardown: {}\n{}", e.header, e.stack);
    }
    end_media(rt, "location.reload()");
    crate::js::reload(rt.ctx());
    #[cfg(vn_devtools)]
    crate::devtools::mark_reload("location.reload()");
    crate::boot_mode::reboot(rt);
}

pub fn end_media(rt: &JsRuntime, why: &str) {
    #[cfg(feature = "audio")]
    let (tracks, contexts) = {
        use vn_runtime::redjs::webapi::{audio, mediastream};
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
            "[vn] {why}: the old page's media ended — {tracks} capture track(s) stopped, \
             {contexts} AudioContext(s) closed"
        );
    }
}
