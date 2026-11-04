//! `location.reload()`: re-run the app, the way the dev host's cold restart
//! does.
//!
//! A browser's reload discards the document and its realm, then loads the page
//! again. vue-native has one JS realm for the life of the process, so a reload
//! here is the existing restart path, on request:
//!
//! 1. `__VN_UNLOAD__()` (packages/vue-native/src/runtime.ts, "location.
//!    reload"): `pagehide` on `window` (`unload` is not implemented, see
//!    window.prelude.ts), then `__VN_TEARDOWN__()` (the app unmounts and every
//!    node is freed), then the realm hygiene. That clears the timers,
//!    animation frames and idle callbacks the outgoing app scheduled, closes
//!    its WebSockets, and removes its `window`/`document` listeners.
//! 2. The bundle is evaluated again: dev re-reads `dist/` (the same `boot` a
//!    cold restart runs), prod re-evaluates the payload's script.
//!
//! The host fn only sets a flag. The work runs at the start of the next frame
//! ([`pump`]), never inside the call stack that asked for it, because tearing
//! the app down under its own handler would leave that handler running
//! against freed nodes. A browser also navigates asynchronously.
//!
//! What a browser does and this does not, also in the runtime's advisory:
//!
//! - the realm survives. The runtime's unload deletes every configurable
//!   global the old app added and replaces the runtime's own classes and
//!   hooks (1a84b39), so what carries over is only built-ins the old app
//!   MODIFIED, plus globals it made non-configurable (counted in an
//!   advisory);
//! - fetches in flight still settle into the old app's promise chains;
//! - IndexedDB connections stay open until the old code closes them.
//!   (Voice rooms, captures and `AudioContext`s do NOT: [`end_media`] ends
//!   them as a browser's navigation does);
//! - `beforeunload` is not asked (a reload the app requested is not vetoed);
//! - `DOMContentLoaded` / `load` fired once per process and do not fire again
//!   (`document.readyState` stays `"complete"`).

use std::cell::Cell;

use vn_runtime::redjs::api::Value;
use vn_runtime::{host_fn, JsRuntime};

thread_local! {
    static REQUESTED: Cell<bool> = const { Cell::new(false) };
}

// `__vn_reload()`: asks for a reload at the next frame. Idempotent within a
// frame.
host_fn!(fn vn_reload(_g, _args) {
    REQUESTED.with(|r| r.set(true));
    Value::undefined().0
});

pub fn install(rt: &JsRuntime) {
    rt.register("__vn_reload", 0, vn_reload as *const _);
}

/// Runs a requested reload. Called from the frame pump inside its Commit
/// publication, before the JS pump, so the new app's first wave lands in this
/// frame.
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
    // The Network panel's divider between the two runs (devtools/net.rs).
    #[cfg(vn_devtools)]
    crate::devtools::mark_reload("location.reload()");
    crate::boot_mode::reboot(rt);
}

/// What a browser's navigation ends with the old document, after its
/// `pagehide` and teardown have run: its calls (every voice room leaves, and
/// with it its peer connections and the capture feeding them), its captures
/// (every `MediaStreamTrack` stops and an unused microphone is released) and
/// its `AudioContext`s (a looping ringtone stops with them).
///
/// Never blocks the frame: each voice engine leaves on its own thread, within
/// about a second (`vn_voice::close_all`). `why` names the caller in the log.
pub fn end_media(rt: &JsRuntime, why: &str) {
    #[cfg(feature = "voice")]
    let rooms = vn_voice::close_all();
    #[cfg(not(feature = "voice"))]
    let rooms = 0usize;
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
    if rooms + tracks + contexts > 0 {
        eprintln!(
            "[vn] {why}: the old page's media ended — {rooms} voice room(s) leaving, {tracks} capture track(s) \
             stopped, {contexts} AudioContext(s) closed"
        );
    }
}
