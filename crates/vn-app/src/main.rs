//! vue-native host shell, milestone 1: JavaScript (JSC via redjs) alive inside
//! a Mica gpui window. The JS below drives the UI through two `host_fn!`s;
//! the pump runs as a self-rescheduling `on_next_frame` callback — before the
//! frame's draw, so a JS mutation lands in the same frame.
//!
//! ## Two boot modes, one shell
//!
//! Everything below the [`boot_mode`] alias is identical in both builds; only
//! *where the JS comes from* differs, and each mode owns one file:
//!
//! - **dev** (default) — [`dev`]: reads `packages/vue-native/dist` off disk and
//!   polls `changes.json` for HMR. Needs the `bun build.ts --watch` loop.
//! - **prod** (`--features prod`) — [`prod`]: a BARE HOST. The app — the single
//!   IIFE from `bun prod.ts`, its window config, its identity and its assets —
//!   is the VNPK payload `vn build` stamps into a copy of this exe
//!   ([`payload`], docs/PACKAGING.md), so the exe runs standalone with no dist/
//!   folder, no watcher, and no Rust toolchain on the app author's side.
//!
//! Both expose exactly `boot(&JsRuntime)` and `poll_frame(frame_no: u32)`.

use std::cell::RefCell;

use gpui::{
    App, Bounds, Context, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowControlArea,
    WindowOptions, div, prelude::*, px, rgba, size,
};
use gpui_platform::application;
use vn_runtime::{JsRuntime, Phase, host_fn, with_app, with_published, with_published_app};
use vn_runtime::runtime::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

#[cfg(not(feature = "prod"))]
mod dev;
// The automation surface (docs/DEVTOOLS.md) and the inspector it feeds
// (docs/DEVTOOLS-UI.md). `vn_devtools` is emitted by build.rs for
// `not(prod) || feature devtools` — i.e. every dev build, plus a release that
// asked for its own inspector by name.
#[cfg(vn_devtools)]
mod devtools;
#[cfg(vn_devtools)]
mod inspector; // the EMBEDDED inspector: F12, `--devtools` mode, the child registry
mod json; // the dev transport, the devtools wire and vn.config.json share one codec
mod owner; // VN_OWNER_PID: a test host dies with the runner that launched it
mod payload; // the app stamped into a packaged exe (VNPK), and `--vn-host-info`
mod perf;
mod popout; // SPIKE: a Vue subtree painted by a second OS window (docs/MULTIWINDOW.md)
#[cfg(feature = "prod")]
mod prod;
mod slot; // which dist slot this process is — the one thing dev, prod and devtools all ask
mod window; // windowing & chrome (docs/WINDOWING.md)

#[cfg(not(feature = "prod"))]
use dev as boot_mode;
#[cfg(feature = "prod")]
use prod as boot_mode;

/// Vue's esm-bundler build reads these three flags off globalThis (bun refuses
/// to inline them — must run BEFORE the app bundle, since ESM import hoisting
/// would put the bundle's Vue code first otherwise). Identical in both modes:
/// the dev/prod split is in the *bundle's* `define` block (build.ts vs
/// prod.ts), not here.
///
/// Both preludes are AUTHORED IN TYPESCRIPT — `prelude.ts` and
/// `window.prelude.ts` beside this file — and the `.js` these two consts read
/// is a checked-in artifact of the generator named in its header. Checked in
/// because `cargo build` must not need a JS toolchain: an `include_str!` of a
/// missing file is a compile error in a Rust crate, which is the worst possible
/// place to discover bun is not installed. `packages/vue-native/test
/// /prelude-generated.test.ts` is what keeps the two halves in step.
const PRELUDE_JS: &str = include_str!("prelude.js");

/// The windowing surface's JS half (docs/WINDOWING.md tiers 1 & 2) — the big
/// one, ~560 lines of real API surface rather than a handful of flags.
/// Evaluated right after [`PRELUDE_JS`] and before the app bundle, so
/// `document.title`, `screen`, `matchMedia` and
/// `navigator.windowControlsOverlay` are already their real selves when the
/// first component's `<script setup>` runs.
const WINDOW_PRELUDE_JS: &str = include_str!("window.prelude.js");

thread_local! {
    static FRAME_NO: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Has the first JS wave already been let through? See [`first_wave_waits`].
    static FIRST_WAVE_RELEASED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// **The first JS wave waits for the first draw.**
///
/// `pump_frame` runs before the draw of the frame it belongs to, which is the
/// browser's own order: a `requestAnimationFrame` callback at frame N reads the
/// geometry frame N-1 laid out. The exception is the FIRST one. At that point
/// the app has mounted (boot ran the bundle, Vue built the whole tree) and
/// nothing has been laid out at all — so every `rAF`, every `setTimeout(0)` and
/// every timer scheduled from `onMounted` measured a tree with no rectangles in
/// it and got `0` where a browser, which lays out before it fires the first
/// animation frame, gets the real number.
///
/// So the first wave — and only the first — is deferred by exactly one frame.
/// `window.refresh()` guarantees that frame happens; from the next one on the
/// gate is open forever, so no later wave can ever be delayed by this.
fn first_wave_waits(window: &mut Window) -> bool {
    if FIRST_WAVE_RELEASED.with(|c| c.get()) {
        return false;
    }
    FIRST_WAVE_RELEASED.with(|c| c.set(true));
    window.refresh();
    true
}

// ---------------------------------------------------------------------------
// `VN_LOG` — gpui's own log records, on demand
// ---------------------------------------------------------------------------
//
// gpui reports things only it knows through the `log` facade: which DXGI
// adapter the renderer chose, the Direct3D feature level, device-lost
// recovery, DirectWrite fallbacks. Nothing in this process had ever installed a
// sink, so all of it was dropped on the floor — which is how "gpui might be
// rendering on a virtual display adapter" stayed a suspicion instead of a fact
// (research/reports/m11/video.md §5, R1).
//
// Off unless `VN_LOG` is set, because these are not app-developer messages: the
// app-facing channels are the compat advisories and the devtools log ring.
// `VN_LOG=info` (or `error`/`warn`/`debug`/`trace`) writes them to stderr.
//
// `VN_LOG` takes either a bare level (`VN_LOG=debug`) or `RUST_LOG`-shaped
// per-target directives (`VN_LOG=off,livekit=debug,libwebrtc=debug`). The
// second form is not a convenience: at a bare `debug` this process emits
// thousands of `parcel_selectors::matching` lines per restyle, so the one
// setting that makes a chatty subsystem readable also makes the host slow
// enough to move the timing of whatever is being debugged. A directive list
// keeps the cost where the interest is.
//
// ---------------------------------------------------------------------------
// THE FLOOR — why a logger is installed even when `VN_LOG` is unset
// ---------------------------------------------------------------------------
//
// libwebrtc reports its own **fatal** conditions (`RTC_CHECK`, `RTC_DCHECK`)
// through the same sink it uses for ordinary logging, and then calls `abort()`.
// Until m14 this process installed no `log` sink unless `VN_LOG` was set, so an
// `RTC_CHECK` failure inside the prebuilt `webrtc.lib` wrote its diagnosis into
// a sink that did not exist and the host died with no message at all
// (research/reports/m13/voice-flake.md §5 — the shape `voice-impl.md` reported
// as "an abort, no panic message, nothing in the log ring").
//
// So [`FLOOR`] is applied unconditionally and is a **floor, not a default**:
// `level_for` takes the MAX of the matching directive and the matching floor
// entry. `log::set_max_level` stays at Warn when `VN_LOG` is unset, so every
// `debug!`/`trace!` call site in the process still short-circuits inside the
// macro, and a run that never constructs a `Room` prints nothing at all.
//
// It is not silent when voice IS running: a full two-window scenario with a
// server restart produces ~530 warn-or-worse records, 312 of them one benign
// libwebrtc line (`webrtc_video_engine.cc: Factory produced duplicate codecs`)
// that no configuration of ours affects. An operator has to be able to turn
// that down, so the floor is escapable — but only so far. Naming a floored
// target in `VN_LOG` explicitly (`VN_LOG=libwebrtc=error`) drops the floor to
// [`HARD_FLOOR`], and nothing drops it below that. `VN_LOG=libwebrtc=off` still
// prints errors, because an abort must not be silenceable by accident.
//
// The message is teed into `vn_tree::log`'s ring as well as stderr, so
// `ctl.ts logs` sees it too. That ring is thread-local and everything libwebrtc
// logs arrives on a webrtc thread, hence [`OFF_THREAD`] and [`drain_log_tee`].
const FLOOR: &[(&str, log::LevelFilter)] = &[
    // Everything the voice stack can die inside. `libwebrtc` is the one that
    // matters — see `vn_voice::rtclog`, which is what gives these records a
    // severity at all.
    ("libwebrtc", log::LevelFilter::Warn),
    ("webrtc_sys", log::LevelFilter::Warn),
    ("livekit", log::LevelFilter::Warn),
    ("livekit_api", log::LevelFilter::Warn),
    ("livekit_protocol", log::LevelFilter::Warn),
    ("vn_voice", log::LevelFilter::Warn),
];

/// The loosest level any [`FLOOR`] entry asks for — the global gate can never
/// be tighter than this, or the facade drops the record before `enabled` runs.
const FLOOR_MAX: log::LevelFilter = log::LevelFilter::Warn;

/// What a floored target is held to even when `VN_LOG` names it explicitly.
/// This is the line that makes "an abort never dies into a void" a property
/// rather than a default.
const HARD_FLOOR: log::LevelFilter = log::LevelFilter::Error;

struct StderrLogger {
    /// `(target prefix, level)`, longest prefix first, so `livekit::room` beats
    /// `livekit`.
    directives: Vec<(String, log::LevelFilter)>,
    default: log::LevelFilter,
}

/// `::`-boundary-aware prefix match: `livekit_api` must not match a `livekit`
/// directive, but `livekit::room` must.
fn target_matches(target: &str, prefix: &str) -> bool {
    target.len() >= prefix.len()
        && target.starts_with(prefix)
        && (target.len() == prefix.len() || target[prefix.len()..].starts_with("::"))
}

impl StderrLogger {
    fn level_for(&self, target: &str) -> log::LevelFilter {
        let mut level = self.default;
        let mut named = false;
        for (prefix, l) in &self.directives {
            if target_matches(target, prefix) {
                level = *l;
                // The operator asked about THIS target by name, which is what
                // buys them the reduced floor below.
                named = true;
                break; // longest prefix first, so the first hit is the best one
            }
        }
        // The floor is a MAX, not a fallback: whatever `VN_LOG` says, a floored
        // target keeps at least this much.
        for (prefix, l) in FLOOR {
            if target_matches(target, prefix) {
                return level.max(if named { HARD_FLOOR } else { *l });
            }
        }
        level
    }
}

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level_for(metadata.target())
    }
    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("[{} {}] {}", record.level(), record.target(), record.args());
        // stderr first: it is unbuffered, so a message logged on the way to
        // `abort()` is already out of the process before the ring is touched.
        eprintln!("{line}");
        tee(record.level(), line);
    }
    fn flush(&self) {}
}

// ---------------------------------------------------------------------------
// teeing into the devtools log ring, from any thread
// ---------------------------------------------------------------------------

/// The thread `install_logger` ran on — the only one whose `vn_tree::log` ring
/// `ctl.ts logs` reads.
static MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();
/// Records made on a webrtc/livekit worker thread, waiting for a frame.
static OFF_THREAD: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());
/// A wedged subsystem logging in a loop must not grow the process; the drain
/// runs every frame, so this is never reached in practice.
const OFF_THREAD_CAP: usize = 512;

fn kind_of(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "error",
        log::Level::Warn => "warn",
        _ => "log",
    }
}

fn tee(level: log::Level, line: String) {
    if !vn_tree::log::is_enabled() {
        return;
    }
    let kind = kind_of(level);
    if MAIN_THREAD.get() == Some(&std::thread::current().id()) {
        vn_tree::log::record(kind, line);
        return;
    }
    if let Ok(mut queue) = OFF_THREAD.lock() {
        if queue.len() >= OFF_THREAD_CAP {
            queue.remove(0); // drop the OLDEST: a fatal is always the newest
        }
        queue.push((kind, line));
    }
}

/// Moves whatever the worker threads logged into the main thread's ring.
/// Called once per frame; `try_lock` because a frame must never block on a
/// logging thread.
fn drain_log_tee() {
    if !vn_tree::log::is_enabled() {
        return;
    }
    let Ok(mut queue) = OFF_THREAD.try_lock() else { return };
    for (kind, line) in queue.drain(..) {
        vn_tree::log::record(kind, line);
    }
}

fn parse_level(word: &str) -> Option<log::LevelFilter> {
    match word.trim().to_ascii_lowercase().as_str() {
        "" | "info" | "1" | "true" => Some(log::LevelFilter::Info),
        "off" | "0" | "false" => Some(log::LevelFilter::Off),
        "error" => Some(log::LevelFilter::Error),
        "warn" => Some(log::LevelFilter::Warn),
        "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace),
        _ => None,
    }
}

fn install_logger() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
    // NOT `else { return }` any more: an unset `VN_LOG` still installs the
    // logger, it just has nothing but the FLOOR in it.
    let spec = std::env::var("VN_LOG").unwrap_or_default();
    let mut default = log::LevelFilter::Off;
    let mut directives: Vec<(String, log::LevelFilter)> = Vec::new();
    let mut saw_bare = false;
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((target, level)) => match parse_level(level) {
                Some(l) => directives.push((target.trim().to_string(), l)),
                None => eprintln!("[vn] VN_LOG: {level:?} is not a level — ignoring {part:?}"),
            },
            None => match parse_level(part) {
                Some(l) => {
                    default = l;
                    saw_bare = true;
                }
                None => eprintln!("[vn] VN_LOG={part:?} is not a level — using info"),
            },
        }
    }
    // `VN_LOG=livekit=debug` alone means "that target, nothing else" rather
    // than "that target plus everything at info". An UNSET `VN_LOG` is a third
    // case and must stay `Off` + FLOOR, so this only fires when the variable
    // was actually present.
    if !saw_bare && directives.is_empty() && !spec.is_empty() {
        default = log::LevelFilter::Info;
    }
    // Longest prefix wins, so a specific directive is never shadowed by a
    // shorter one that happens to be listed first.
    directives.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    let max = directives
        .iter()
        .map(|(_, l)| *l)
        .chain([default, FLOOR_MAX])
        .max()
        .unwrap_or(FLOOR_MAX);
    let logger: &'static StderrLogger =
        Box::leak(Box::new(StderrLogger { directives, default }));
    if log::set_logger(logger).is_ok() {
        // The facade's global gate must be the *loosest* of the directives, or
        // records are dropped before `enabled` ever sees their target.
        log::set_max_level(max);
    }
}

/// The build model, pinned from inside whatever build is running.
///
/// `build.rs` emits `cfg(vn_devtools)` for every build except one that asked
/// for `no-devtools`, and this holds for EVERY feature combination — which is
/// what makes it the cheap proof that `--features prod` now carries the
/// devtools. Run it under the combination you care about:
///
/// ```sh
/// cargo test -p vn-app                                   # dev host: has them
/// cargo test -p vn-app --features prod                   # shipped app: has them
/// cargo test -p vn-app --features prod,no-devtools       # the opt-out: none
/// ```
///
/// The inversion itself is docs/ARCHITECTURE.md §11 decision 12; the measured
/// binaries are research/reports/m22/devtools-release.md.
#[cfg(test)]
#[test]
fn devtools_are_in_every_build_that_did_not_opt_out() {
    assert_eq!(cfg!(vn_devtools), !cfg!(feature = "no-devtools"));
    // …and `prod` no longer has anything to do with it. This line is the one
    // that would have failed before m22.
    if cfg!(feature = "prod") && !cfg!(feature = "no-devtools") {
        assert!(cfg!(vn_devtools), "a shipped build must carry its own devtools");
    }
}

#[cfg(test)]
mod logger_tests {
    use super::*;
    use log::LevelFilter as L;

    fn logger(spec: &[(&str, L)], default: L) -> StderrLogger {
        let mut directives: Vec<(String, L)> =
            spec.iter().map(|(t, l)| ((*t).to_string(), *l)).collect();
        directives.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        StderrLogger { directives, default }
    }

    #[test]
    fn an_unset_vn_log_still_logs_the_voice_stack_at_warn() {
        let l = logger(&[], L::Off);
        assert_eq!(l.level_for("libwebrtc"), L::Warn);
        assert_eq!(l.level_for("livekit::rtc_engine"), L::Warn);
        assert_eq!(l.level_for("vn_voice::engine"), L::Warn);
        // …and nothing else. This is what keeps the floor free.
        assert_eq!(l.level_for("parcel_selectors::matching"), L::Off);
        assert_eq!(l.level_for("gpui::platform"), L::Off);
    }

    #[test]
    fn a_directive_can_turn_a_floored_target_up() {
        let l = logger(&[("libwebrtc", L::Debug)], L::Off);
        assert_eq!(l.level_for("libwebrtc"), L::Debug);
    }

    #[test]
    fn naming_a_floored_target_lowers_the_floor_to_error_and_no_further() {
        for asked in [L::Error, L::Warn, L::Off] {
            let l = logger(&[("libwebrtc", asked)], L::Off);
            let got = l.level_for("libwebrtc");
            assert!(got >= L::Error, "asked {asked:?}, got {got:?}");
        }
        // Explicitly asking for `error` gets exactly error — the escape hatch
        // that makes 300 benign warnings per call suppressible.
        assert_eq!(logger(&[("libwebrtc", L::Error)], L::Off).level_for("libwebrtc"), L::Error);
        // NOT naming it leaves the full warn floor in place.
        assert_eq!(logger(&[("gpui", L::Error)], L::Off).level_for("libwebrtc"), L::Warn);
    }

    #[test]
    fn a_bare_level_never_drops_a_floored_target_below_warn() {
        // `VN_LOG=off` is the case the m13 hunt would have hit.
        assert_eq!(logger(&[], L::Off).level_for("libwebrtc"), L::Warn);
        assert_eq!(logger(&[], L::Error).level_for("libwebrtc"), L::Warn);
        assert_eq!(logger(&[], L::Trace).level_for("libwebrtc"), L::Trace);
    }

    #[test]
    fn target_boundaries_are_module_separators() {
        // The trap `livekit_api` sets for a `livekit` directive.
        assert!(target_matches("livekit::room", "livekit"));
        assert!(target_matches("livekit", "livekit"));
        assert!(!target_matches("livekit_api::signal_client", "livekit"));
        // …and it has its own floor entry, so it is covered anyway.
        assert_eq!(logger(&[], L::Off).level_for("livekit_api::signal_client"), L::Warn);
    }
}

struct Shell {
    status: String,
    log: Vec<String>,
    /// The fallback titlebar's text: the window's own title (`vn.config.json`
    /// `window.title`), so a packaged app that never paints its own chrome
    /// does not show the engine's name in its titlebar.
    title: String,
}

thread_local! {
    static SHELL: RefCell<Option<WeakEntity<Shell>>> = const { RefCell::new(None) };
    /// The primary window itself, as opposed to its root view. Needed because
    /// the multi-window repaint fan-out runs from inside an input listener,
    /// where the root view's entity may already be borrowed — see
    /// [`refresh_main`].
    static MAIN_WINDOW: std::cell::Cell<Option<gpui::AnyWindowHandle>> =
        const { std::cell::Cell::new(None) };
}

/// **The forced synchronous layout** a DOM measurement asks for.
///
/// Installed into `vn_tree` at startup (`set_sync_layout`), and called from
/// `dom::bounds` when JS measures a node that the host has never painted while
/// the tree still holds changes — the `onMounted` / `nextTick` case, where a
/// browser lays out on read and this engine used to answer `0`. See
/// `vn_tree::sync_layout` for the whole argument.
///
/// One draw of the primary window, which is what gpui's own effect flush does
/// for a dirty window (`App::flush_effects` → `window.draw(cx).clear(cx)`).
/// `with_app` is `None` outside a published phase — including inside a real
/// frame, where nothing is published — so this can never run inside a draw;
/// `vn_tree` guards the re-entrant case on its side as well.
fn sync_layout() {
    let Some(handle) = MAIN_WINDOW.with(|w| w.get()) else { return };
    vn_runtime::bridge::with_app(|cx| {
        let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
    });
}

/// Marks the primary window dirty *without* borrowing its root view.
///
/// `notify_shell` goes through `Entity<Shell>::update`, which is correct from
/// the pump but re-enters and panics when called from inside a `cx.listener`
/// on that same view. Addressing the window borrows nothing, and
/// `update_window` on a window that is mid-update simply fails — which is the
/// right answer, since that window is being refreshed by the listener anyway.
/// The primary window's gpui id — what the devtools `windows` command calls
/// `main`, and what `--window` names when F12 was pressed there.
#[cfg(vn_devtools)]
pub(crate) fn main_window_id() -> Option<u64> {
    MAIN_WINDOW.with(|w| w.get()).map(|h| h.window_id().as_u64())
}

/// Repaint the primary window.
///
/// `notify_shell`, not `Window::refresh` — the two schedule the same redraw,
/// but `refresh` also raises gpui's `refreshing` flag, and a frame drawn with
/// that flag up ignores every `Entity::cached` recording in the window. Marking
/// the hosting view dirty is what the pump does for a JS wave, and it is what
/// leaves the retained-render cache (`vn_tree::cache`) able to replay the
/// subtrees the wave did not touch.
pub(crate) fn refresh_main(cx: &mut App) {
    notify_shell(cx);
}

/// Repaint the primary window. The popped-out windows are notified next to it
/// wherever the tree moved (`popout::notify_all`), but a pop-out OPENING or
/// CLOSING moves no tree state at all — it only changes which window owns a
/// subtree — so those two call this directly.
pub(crate) fn notify_shell(cx: &mut App) {
    SHELL.with(|s| {
        if let Some(shell) = s.borrow().as_ref().and_then(|w| w.upgrade()) {
            shell.update(cx, |_, cx| cx.notify());
        }
    });
}

fn update_shell(g: *mut core::ffi::c_void, f: impl FnOnce(&mut Shell)) -> u64 {
    let ran = with_app(|cx| {
        SHELL.with(|s| {
            if let Some(shell) = s.borrow().as_ref().and_then(|w| w.upgrade()) {
                shell.update(cx, |shell, cx| {
                    f(shell);
                    cx.notify();
                });
            }
        })
    });
    match ran {
        Some(()) => vn_runtime::redjs::api::Value::undefined().0,
        // Wrong phase: this host fn is only legal while gpui is published.
        None => vn_runtime::bridge::throw_error(g, 1, "__vn_*: called outside a published phase"),
    }
}

host_fn!(fn vn_set_status(g, args) {
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    // A status line is the CURRENT value, not a stream — devtools keeps it in
    // its own slot so a 2 Hz status ticker cannot evict the log ring.
    vn_tree::log::set_status(text.clone());
    update_shell(g, move |shell| shell.status = text)
});

// `__vn_perf_mark("<label>")` — labels the frame-time window the profiler is
// accumulating (see `perf`). A no-op unless `VN_PERF` is set, so a perf harness
// app can call it unconditionally.
host_fn!(fn vn_perf_mark(g, args) {
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    perf::mark(&text);
    vn_runtime::redjs::api::Value::undefined().0
});

host_fn!(fn vn_log(g, args) {
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    vn_tree::log::record("log", text.clone());
    update_shell(g, move |shell| {
        shell.log.push(text);
        if shell.log.len() > 8 {
            shell.log.remove(0);
        }
    })
});

/// The per-frame JS pump: runs before this frame's draw (`on_next_frame`
/// callbacks drain first), so `cx.notify()` from a host fn is picked up by the
/// very same frame.
fn pump_frame(window: &mut Window, cx: &mut App) {
    // One `Instant` read per frame when VN_PERF is on, an atomic load when it
    // is off. `on_next_frame` re-arms every frame, so this IS the frame clock.
    perf::frame();
    // Before the poison gate: what a libwebrtc/livekit worker logged is exactly
    // what an agent wants to read when the JS side has already given up.
    drain_log_tee();
    if vn_runtime::bridge::poisoned() {
        return; // I9: no JS after a caught native panic.
    }
    // One frame, once, so the first wave measures a tree that has been laid out.
    if first_wave_waits(window) {
        window.on_next_frame(pump_frame);
        return;
    }
    // The frame's ONE coalesced mousemove (browsers align move events to the
    // frame clock; see PendingMouseMove in vn-tree). Dispatched before the
    // pump block so whatever the handler schedules drains this same frame.
    vn_tree::flush_pending_mouse_move(window, cx);
    // An inspector whose app has gone closes itself. OUTSIDE the publication
    // below: `cx` is borrowed by it, and a quit has nothing to do with JS.
    #[cfg(vn_devtools)]
    inspector::poll_frame(cx);
    with_published(window, cx, Phase::Commit, || {
        // Dev loop: poll dist/changes.json and apply HMR BEFORE the pump, so
        // everything Vue schedules while patching (its post-flush queue, the
        // reload path's queueJob) drains in this frame. In prod this is an
        // empty inline fn.
        let n = FRAME_NO.with(|f| {
            let n = f.get().wrapping_add(1);
            f.set(n);
            n
        });
        boot_mode::poll_frame(n);
        // The window-state funnel (docs/WINDOWING.md §5.3): ONE snapshot, then
        // resize → visibility → the JS observer, in that order, all inside the
        // same JS wave the pump below runs in. Before `rt.pump` so a `resize`
        // handler's DOM mutations drain in this frame, and so
        // `fullscreenchange` fires after the viewport has finished resizing —
        // which the Fullscreen spec requires.
        vn_runtime::with_commit(|window, cx| window::pump(window, cx));
        JsRuntime::with(|rt| {
            rt.pump(PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS);
            for err in rt.drain_errors() {
                eprintln!("[js uncaught] {}\n{}", err.header, err.stack);
                vn_tree::log::record(
                    "error",
                    format!("[js uncaught] {}\n{}", err.header, err.stack),
                );
            }
        });
        // One queued devtools command per frame, in the SAME publication the
        // pump just used: a command can therefore never observe a half-applied
        // patch, and anything it schedules drains with the next frame's wave.
        #[cfg(vn_devtools)]
        {
            devtools::pump();
        }
        // Voice events, drained bounded, in this SAME publication — and NOT
        // dev-gated, because voice is an application capability rather than an
        // automation surface. A room event therefore can never observe a
        // half-applied patch, and whatever its JS handler schedules (a Vue
        // patch off `RoomEvent.ParticipantConnected`, say) drains in this
        // frame's wave. `on_next_frame` re-arms unconditionally, so a queued
        // event is at most ONE frame late — 6 ms at 165 Hz — and no new wake
        // seam was needed. See crates/vn-voice/CONTRACT.md §3.
        vn_voice::pump();
        // Settled `appPermissions.request()` promises, same publication, same
        // reasoning. The `change` half is not here: `PermissionStatus` events
        // are dispatched from inside `rt.pump()` above, by redjs, because that
        // is where the status objects live.
        vn_permissions::pump();
        // Settled `appShell.*` promises and unsolicited desktop events (a
        // second instance's `launch`), same publication, same reasoning: the
        // `.then()` and the Vue patch it triggers land in this frame.
        vn_desktop::pump();
        // At most ONE localStorage file write per frame, whatever the app did
        // to the area during this wave. Inside the publication like everything
        // else here, though nothing in it touches gpui — it is here because the
        // frame clock is the coalescing clock (docs/STORAGE.md §6).
        #[cfg(feature = "storages")]
        vn_storage::pump();
    });
    // The `@media` environment (viewport, colour scheme, reduced motion, DPI
    // scale, chrome mode) is published inside `window::pump` above, next to the
    // snapshot the same numbers come from — one source, so JS and CSS can never
    // disagree about the viewport.
    //
    // The fallback titlebar retires here rather than in `render`: this runs
    // after the frame that painted, so "did the app declare a drag region" has
    // an answer (docs/WINDOWING.md §4.2 mode 2).
    window::note_painted_frame();
    // Everything that needs a `Window` and could not run from a host fn: the
    // `<lottie>` transport is advanced by this frame's wall-clock delta (and its
    // `complete`/`loopComplete`/`enterFrame` events dispatched), every `<video>`
    // pulls the frame the OS media pipeline has ready (and its HTML media events
    // are dispatched), and a node that was focused when it got detached loses
    // gpui focus. Before the draw, so the picture a media element paints is the
    // one this tick produced.
    // Two atomic loads: has the renderer recreated its D3D11 device since the
    // last frame? If so, every `<video>` is decoding onto a dead one and has to
    // be rebuilt before it is ticked.
    publish_video_device();
    let media_moved = vn_tree::frame_tick(window, cx);
    // One notify per JS wave that touched the tree — the draw right after
    // this callback picks it up (same frame). While CSS transitions run or a
    // Lottie plays, keep repainting: `on_next_frame` alone re-arms but does not
    // redraw.
    let tree_moved = vn_tree::flush_changes();
    // The push channel's tree revision. A number on a socket, not a tree: it
    // lets an attached inspector refetch because something MOVED rather than
    // because 2.5 s went by (docs/DEVTOOLS.md, the push channel).
    #[cfg(vn_devtools)]
    if tree_moved {
        devtools::ws::note_tree_changed();
    }
    if tree_moved
        || vn_tree::animating()
        || media_moved
        || vn_tree::lotties_animating()
        || vn_tree::videos_playing()
    {
        notify_shell(cx);
        // …and every window painting a popped-out subtree, in the SAME frame
        // group. This is the line that makes the spike's claim true: one JS
        // wave, N windows repainting off it (docs/MULTIWINDOW.md §6).
        popout::notify_all(cx);
    }
    // Push, not poll: whatever moved this frame — log lines, the tree
    // revision just computed above, a frame-time sample — goes out to every
    // attached inspector here, AFTER the flush, so a socket is never told
    // about a revision the tree has not reached.
    #[cfg(vn_devtools)]
    devtools::ws::broadcast_frame();
    window.on_next_frame(pump_frame);
    // Closes the `pump` bucket: everything after this is gpui's — its decision
    // to draw, the draw itself, the present, and the wait for the next vsync.
    perf::pump_done();
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        perf::render_begin();
        let el = self.render_body(window, cx);
        perf::render_done();
        el
    }
}

impl Shell {
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        // The built-in titlebar is a FALLBACK, not furniture (D-23): an app
        // that declares no drag region would otherwise be unmovable, so it
        // stays until the app paints a `window-drag: move` region of its own —
        // and then retires, with one `[window]` log line. Never silently
        // removing a behaviour an app relied on.
        let shell = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont");
        if !window::fallback_titlebar() {
            // Custom chrome, mode 1: the app owns the WHOLE window — its own
            // titlebar included. No host padding, no host status panel; the
            // shell is a bare mount point.
            return shell
                .child(vn_tree::build_root(window, cx))
                .children(window_controls_overlay())
                .children(inspector_overlay());
        }
        shell
            .child(
                div()
                    .id("titlebar")
                    .window_control_area(WindowControlArea::Drag)
                    .h(px(36.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    // The Vue-driven subtree, straight from the retained tree.
                    //
                    // `flex_1().min_h(0)` is what gives the app root a DEFINITE
                    // box: vue-native's model is "the window is the root", so
                    // the root element sizes itself to 100% of whatever it is
                    // handed (see `vn_tree`'s `build_node`). An auto-height slot
                    // would make `h-full` and `overflow: auto` in the app
                    // resolve against an indefinite parent — i.e. never scroll —
                    // and would put a `position: fixed` overlay wherever the
                    // content happens to end rather than over the viewport.
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .child(vn_tree::build_root(window, cx)),
                    )
                    .child(
                        div()
                            .p(px(12.))
                            .bg(rgba(0xffffff0d))
                            .rounded(px(12.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgba(0xffffff90))
                                    .child(self.status.clone()),
                            )
                            .children(self.log.iter().map(|line| {
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgba(0xffffff70))
                                    .child(line.clone())
                            })),
                    ),
            )
            .children(window_controls_overlay())
            .children(inspector_overlay())
    }
}

/// The devtools inspector's element outline, drawn over everything the app
/// painted (docs/DEVTOOLS-UI.md §5). Always `None` in a prod build — and in a
/// dev build until something sends a `highlight` command.
///
/// It sits in the SHELL rather than in the app's own tree on purpose: the
/// rectangle is in window coordinates (which is what `bounds` reports), the
/// shell is the element at the window origin, and an overlay inside the app's
/// tree would be a node the app can see — an inspector must not appear in the
/// tree it is inspecting.
fn inspector_overlay() -> Option<gpui::Div> {
    #[cfg(vn_devtools)]
    {
        devtools::highlight_overlay()
    }
    #[cfg(not(vn_devtools))]
    {
        None
    }
}

/// The host-drawn caption-button cluster — `titleBarOverlay` (docs/WINDOWING.md
/// §2.3, §4.3).
///
/// A custom-chrome window has no `WS_CAPTION`, so there is no OS cluster to
/// host: on Windows the only way to get caption buttons in a frameless window
/// is to draw them, which is exactly what Electron and Chromium do for their
/// own overlay. What makes these OS-*behaved* rather than three hand-rolled
/// buttons is `window_control_area`: `WM_NCHITTEST` answers `HTMINBUTTON` /
/// `HTMAXBUTTON` / `HTCLOSE`, so the OS performs the action, and `HTMAXBUTTON`
/// is what arms the **Windows 11 snap-layouts flyout** — which no click handler
/// can produce.
///
/// Positioned absolutely at the top-RIGHT because that is where Windows puts
/// them; the rect an app reads back (`getTitlebarAreaRect()`,
/// `env(titlebar-area-width)`) loses exactly this width, so app content is
/// never underneath it.
fn window_controls_overlay() -> Option<gpui::Div> {
    let (height, overlay) = window::overlay()?;
    // Segoe Fluent Icons is the Win11 caption glyph font and these three are the
    // OS's own glyphs — chevron-free, hairline, correctly optically centred.
    //
    // It is NOT present on Windows 10, where the same private-use code points
    // live in Segoe MDL2 Assets, so naming only the Win11 font would have shown
    // three tofu boxes there. `Font::fallbacks` is DirectWrite's own fallback
    // chain (`IDWriteTextFormat::SetFontFallback`), so the lookup is per
    // code point and per machine rather than a compile-time guess: Win11 finds
    // the glyph in the first family, Win10 in the second, and Segoe UI Symbol
    // is the last stop before DirectWrite's system fallback.
    const MIN: &str = "\u{e921}";
    const MAX: &str = "\u{e922}";
    const CLOSE: &str = "\u{e8bb}";
    let glyph_font = gpui::Font {
        family: "Segoe Fluent Icons".into(),
        fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
            "Segoe MDL2 Assets".to_string(), // Windows 10 — the same code points
            "Segoe UI Symbol".to_string(),
        ])),
        features: Default::default(),
        weight: Default::default(),
        style: Default::default(),
    };
    let symbol = overlay
        .symbol_color
        .map(rgba)
        .unwrap_or_else(|| rgba(0xffffffe0));
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea, hover: u32| {
        div()
            .id(id)
            .w(px(window::OVERLAY_BUTTON_WIDTH))
            .h(px(height))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.))
            .text_color(symbol)
            .font(glyph_font.clone())
            .window_control_area(area)
            .hover(|s| s.bg(rgba(hover)))
            .child(glyph)
    };
    let mut strip = div()
        .absolute()
        .top_0()
        .right_0()
        .h(px(height))
        .flex()
        .flex_row();
    if let Some(bg) = overlay.color {
        strip = strip.bg(rgba(bg));
    }
    Some(
        strip
            .child(button("wco-min", MIN, WindowControlArea::Min, 0xffffff1a))
            .child(button("wco-max", MAX, WindowControlArea::Max, 0xffffff1a))
            // Win11's close button turns #C42B1C on hover; every other one gets
            // a neutral wash. Matching the OS matters more here than matching
            // the app's palette — this IS the window's chrome.
            .child(button("wco-close", CLOSE, WindowControlArea::Close, 0xc42b1cff)),
    )
}

/// The directory a relative `<img src>` resolves against — vue-native's
/// stand-in for the document URL a browser resolves against.
///
/// `VN_APP_ROOT` wins when set; otherwise the dev transport derives it from the
/// manifest entry (`apps/<app>/src/main.js` → `apps/<app>/`) in [`dev::boot`],
/// and the prod build — which reads no files at all — leaves it unset, so
/// relative paths fall back to the working directory with an advisory.
fn publish_app_root() {
    // A packaged exe: relative URLs resolve out of the payload, the same store
    // `fetch()` reads — the package is the document (docs/PACKAGING.md).
    #[cfg(feature = "prod")]
    if prod::publish_asset_store() {
        return;
    }
    if let Some(dir) = std::env::var_os("VN_APP_ROOT") {
        if !dir.is_empty() {
            vn_tree::set_asset_base(std::path::PathBuf::from(dir));
        }
    }
}

/// Hands `<video>` the renderer's D3D11 device — and re-hands it every time
/// the renderer recreates one.
///
/// The pointer is BORROWED from `DirectXDevices`, which lives as long as the
/// renderer does; `vn_video` clones the COM interface if it keeps it.
///
/// **This runs once per frame, and the generation is the whole point.** A
/// device-lost recovery (`DirectXDevices::new` after a TDR, a driver update or
/// a GPU reset) builds a *new* `ID3D11Device` and bumps the generation. Every
/// Media Foundation engine we created is still decoding onto the dead one, and
/// every texture it hands us belongs to a device the atlas now refuses to copy
/// from — which is exactly what the atlas's device-identity check catches. So a
/// generation change republishes the device and forces each `<video>` to
/// rebuild on it, keeping its position and transport state.
///
/// Two atomic loads and a compare on the frames where nothing happened.
#[cfg(target_os = "windows")]
fn publish_video_device() {
    use std::cell::Cell;
    thread_local! {
        /// 0 is never a live generation — `publish_device` pre-increments.
        static PUBLISHED: Cell<u64> = const { Cell::new(0) };
    }
    let (ptr, generation) = gpui_windows::renderer_d3d11_device();
    if ptr.is_null() || PUBLISHED.with(Cell::get) == generation {
        return;
    }
    let first = PUBLISHED.with(Cell::get) == 0;
    PUBLISHED.with(|p| p.set(generation));
    // SAFETY: the pointer comes straight from the live `DirectXDevices`.
    unsafe { vn_tree::set_video_device(ptr, generation) };
    if first {
        return;
    }
    let rebuilt = vn_tree::video_device_changed();
    let msg = format!(
        "[video] the renderer recreated its D3D11 device (generation {generation}) — rebuilt \
         {rebuilt} <video> element(s) on it, resuming where they were"
    );
    eprintln!("{msg}");
    vn_tree::log::record("window", msg);
}

#[cfg(not(target_os = "windows"))]
fn publish_video_device() {}

/// The HTTP client gpui fetches `Resource::Uri` assets through.
///
/// Without one, `App::http_client()` is gpui's `NullHttpClient`, which fails
/// every request with "No HttpClient available" a frame after the element asked
/// — the silent failure the m10 images report chose an advisory over rather
/// than shipping.
///
/// The user agent is the app's, not a browser's. Nothing here pretends to be
/// Chrome: a server that serves us different bytes because of the UA string is
/// a server whose answer we would rather see honestly.
///
/// A failure to build one is not fatal — it leaves gpui's null client in place,
/// which is exactly the behaviour every build had before this. It is logged, so
/// "my images do not load" has an answer in `VN_LOG=warn`.
fn install_http_client(app: gpui::Application) -> gpui::Application {
    match reqwest_client::ReqwestClient::user_agent(concat!(
        "vue-native/",
        env!("CARGO_PKG_VERSION")
    )) {
        Ok(client) => app.with_http_client(std::sync::Arc::new(client)),
        Err(e) => {
            log::warn!(
                "could not build an HTTP client ({e}); http(s) <img> and <video> sources will \
                 fail to load. Local paths and data: URIs are unaffected."
            );
            app
        }
    }
}

fn main() {
    // First, so a host whose runner is already gone never opens a window.
    owner::watch();
    // The ONE parse of this process's command line (docs/LAUNCH.md §2). Every
    // consumer below — the host check, the inspector mode, the protocol
    // handler, `appLaunch.argv` — reads this, never `std::env::args()`.
    let args = vn_desktop::args::process();
    // `vn build`'s host check: a BARE host (nothing stamped into it) prints
    // what it is — mode, vue-native version, variant — as one JSON line and
    // exits before any window, logger or JS exists. A packaged exe never takes
    // this branch; its argv belongs to its app.
    if args.host.host_info && !payload::embedded_present() {
        println!("{}", payload::host_info_json());
        return;
    }
    // `--vn-dist <slot>` is `VN_DIST` spelled on the command line — what a
    // protocol registration made by a DEV host writes, because a process the
    // shell starts for a `myapp://` link inherits no environment of ours.
    // Before anything reads the slot (the logger and the profiler do, through
    // `slot::dir`).
    if let Some(dist) = args.host.vn_dist.as_deref().filter(|d| !d.is_empty()) {
        std::env::set_var("VN_DIST", dist);
    }
    // Single instance (docs/LAUNCH.md §3), BEFORE the inspector, the logger and
    // gpui: a second launch of a `singleInstance` app forwards its argv / url /
    // cwd to the running one and exits here, having created no window and
    // touched no `localStorage`. Every other process records its launch for
    // `appLaunch` and carries on. The inspector (`--devtools`) is never an
    // instance of anything and reads no app config.
    {
        let launch_cfg = if args.host.devtools { None } else { boot_mode::launch_config() };
        // A DEV host runs many apps out of one exe, so the slot is part of the
        // instance key (parallel e2e slots of one app must not collide); a
        // packaged exe IS its app.
        #[cfg(not(feature = "prod"))]
        let dev_slot = Some(slot::dir());
        #[cfg(feature = "prod")]
        let dev_slot = None;
        let build_name = if args.host.devtools { None } else { boot_mode::app_name() };
        if let vn_desktop::launch::Outcome::Exit(code) =
            vn_desktop::launch::start(args, launch_cfg.as_deref(), build_name, dev_slot)
        {
            std::process::exit(code);
        }
    }
    // `--devtools` rewrites two environment variables (the dist slot this
    // process publishes into, and which instance it prefers), so it has to run
    // before anything reads either — which the logger and the profiler both
    // do indirectly through `slot::dir`.
    #[cfg(vn_devtools)]
    inspector::init();
    // A prod build with no app is a bare host: it says so and stops, rather
    // than opening an empty window. The inspector mode needs no app.
    #[cfg(feature = "prod")]
    {
        #[cfg(vn_devtools)]
        let inspecting = inspector::is_devtools_host();
        #[cfg(not(vn_devtools))]
        let inspecting = false;
        if !inspecting {
            prod::require_payload();
        }
    }
    // Before anything gpui does, so the renderer's boot messages are visible.
    install_logger();
    perf::boot();
    let app = install_http_client(application());
    // gpui reads `Resource::Embedded(key)` — what an `<img>` resolves to in a
    // packaged exe — through this, straight out of the mapped payload.
    #[cfg(feature = "prod")]
    let app = if payload::get().is_some() { app.with_assets(payload::GpuiAssets) } else { app };
    app.run(|cx: &mut App| {
        // `--devtools` is a THIRD boot mode over this same shell: the app is
        // the inspector, and it comes out of this executable rather than off
        // disk. Everything below — the window, the pump, the preludes — is
        // identical, which is the point of keeping it a mode rather than a
        // second binary.
        #[cfg(vn_devtools)]
        let devtools_mode = inspector::is_devtools_host();
        #[cfg(not(vn_devtools))]
        let devtools_mode = false;
        vn_elements::init(cx); // key bindings for the native <input> widget
        publish_app_root();
        #[cfg(vn_devtools)]
        {
            // F12, in EVERY window of this process, ahead of every element's
            // own key listener. See `inspector::install_hotkey`.
            inspector::install_hotkey(cx);
            // A debugger that outlives its subject is a window nobody can
            // explain. (The child watches us too, for the case where this
            // never runs — a crash.)
            std::mem::forget(cx.on_app_quit(|_| {
                inspector::close_children();
                std::future::ready(())
            }));
        }
        // Before the runtime exists, so the log ring is already recording when
        // the first advisory fires — and before the window config is read, for
        // the same reason. This is only the RECORDING half; whether a listener
        // is started at all is `devtools::boot` below, which needs the config.
        #[cfg(vn_devtools)]
        devtools::arm();

        // The window block of `apps/<app>/vn.config.json`, read BEFORE
        // `open_window` because that is the only tier that can set the
        // creation-time-only gpui options (`window_min_size`, `is_resizable`,
        // `is_minimizable`, `titlebar`, `window_background`) with no visible
        // flash. Environment still wins on top (`VN_WINDOW`, `VN_WINDOW_FOCUS`)
        // — the perf and screenshot harnesses depend on it (D-21).
        //
        // It is read here, ahead of the JS runtime, because it also carries
        // `"devtools": false` — and that has to be answered before a port is
        // bound rather than by closing one a moment after opening it.
        #[cfg(vn_devtools)]
        let window_json =
            if devtools_mode { inspector::window_config() } else { boot_mode::window_config() };
        #[cfg(not(vn_devtools))]
        let window_json = boot_mode::window_config();
        let cfg = window::load_config(window_json);

        // The runtime switch (docs/DEVTOOLS.md "Turning them off"): the config's
        // answer, the `VN_DEVTOOLS` override on top of it, and the listener if
        // the result is on. Before `JsRuntime::init`, so the app bundle's very
        // first line already sees the settled state.
        #[cfg(vn_devtools)]
        devtools::boot(window::devtools_config_enabled());

        JsRuntime::init();
        JsRuntime::with(|rt| {
            rt.register("__vn_set_status", 1, vn_set_status as *const _);
            rt.register("__vn_log", 1, vn_log as *const _);
            rt.register("__vn_perf_mark", 1, vn_perf_mark as *const _);
            vn_tree::install(rt);
            window::install(rt);
            // SPIKE: `__vn_window_open` / `_close` / `_popouts` / `_click` —
            // one Vue subtree painted by a second OS window (docs/MULTIWINDOW.md).
            popout::install(rt);
            // `__vn_voice_*` — the five host fns behind "livekit-client".
            // Registered in every build; the JS shim is pre-seeded into the
            // vendor bundle either way, and an app that never constructs a
            // Room costs one idle registration.
            vn_voice::install(rt);
            // `navigator.clipboard` over gpui's clipboard — the same one the
            // native <input> uses for Ctrl+C, so there is never a second owner
            // of `OpenClipboard`. No host fns and no pump: the clipboard is
            // synchronous, so the promise is settled before `readText()`
            // returns.
            #[cfg(feature = "clipboard")]
            vn_clipboard::install();
            // Installs the platform backend behind redjs's
            // `navigator.permissions` AND registers `__vn_permissions_*` for
            // the `appPermissions` extension. Must happen before any app code
            // runs; the hook is read per call, so nothing else depends on the
            // order within this block. See docs/PERMISSIONS.md.
            vn_permissions::install(rt);
            // `__vn_desktop_*` / `__vn_shell_*`: the settle dispatcher and the
            // shell behind `<a href>`, `window.open(url)` and `appShell`
            // (docs/SHELL.md). The shell thread's log lines ("recorded" under
            // VN_SHELL=record) reach the ring through `pump()`, on this thread.
            vn_desktop::set_log_sink(|kind, text| vn_tree::log::record(kind, text));
            vn_desktop::install(rt);
            // The backend behind redjs's `localStorage`, plus
            // `__vn_storage_*` for the `appStorage` extension. Must happen
            // before any app code runs: the area is loaded lazily on first
            // touch, so a host that installed late would hand the app an empty
            // `localStorage` and only then start honouring the disk.
            // `boot_mode::app_name` is what names the default folder in a dev
            // host, where one exe runs many apps. See docs/STORAGE.md.
            #[cfg(feature = "storages")]
            vn_storage::install(
                rt,
                if devtools_mode { Some("vn-devtools".to_string()) } else { boot_mode::app_name() },
            );
            #[cfg(vn_devtools)]
            devtools::install(rt);
            // `__vn_dev_voice_token` — the LiveKit dev token minter. Dev only,
            // on purpose: an API secret must not exist in a shipped binary.
            #[cfg(not(feature = "prod"))]
            dev::install(rt);
        });

        let bounds = Bounds::centered(None, size(px(cfg.width), px(cfg.height)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: cfg.backdrop.to_gpui(),
                titlebar: Some(TitlebarOptions {
                    title: Some(cfg.title.clone().into()),
                    // `appears_transparent` IS the chrome-mode switch: gpui maps
                    // it to `hide_title_bar`, and fork item F-03 makes
                    // `!hide_title_bar` add `WS_CAPTION` — a real OS caption,
                    // not the captionless thick frame it used to give.
                    // `titleBarStyle` is creation-time only for exactly this
                    // reason (D-25).
                    appears_transparent: cfg.title_bar_style == window::TitleBarStyle::Hidden,
                    ..Default::default()
                }),
                is_resizable: cfg.resizable,
                is_minimizable: cfg.minimizable,
                window_min_size: cfg
                    .min_width
                    .zip(cfg.min_height)
                    .map(|(w, h)| size(px(w), px(h))),
                // A permanently armed pump must not opt the window into the
                // 33 ms unfocused throttle (JS timers would coarsen with it).
                inactive_frame_interval: None,
                // THE line that makes F12 not interrupt you: gpui maps this to
                // `SW_SHOWNOACTIVATE`, so the inspector appears beside the app
                // without taking the caret out of whatever you were typing.
                // An ordinary app is unaffected (`focus: true` is gpui's own
                // default and what every other host here has always used).
                focus: !devtools_mode,
                ..Default::default()
            },
            |window, cx| {
                // The renderer's device now exists (the window created it), so
                // this is the first moment `<video>` can be told to decode onto
                // it rather than onto a second, private one. Before it, and on
                // a platform with no such accessor, the Media Foundation
                // backend falls back — it never fails for want of this.
                publish_video_device();
                let shell = cx.new(|_| Shell {
                    status: "booting JS…".into(),
                    log: Vec::new(),
                    title: cfg.title.clone(),
                });
                SHELL.with(|s| *s.borrow_mut() = Some(shell.downgrade()));
                window::init(&cfg, window, cx);
                // The window a forwarded second launch brings to the front
                // (`raiseOnSecondInstance`, docs/LAUNCH.md §3). The pipe thread
                // raises it directly: the foreground grant the second process
                // passed on is short-lived, and a JS-initiated focus could not
                // use it.
                if let Some(hwnd) = window::capture_hwnd(window) {
                    vn_desktop::launch::set_window(hwnd);
                }
                // `beforeunload`: `on_window_should_close` is a SYNCHRONOUS
                // boolean, which is precisely the web event's shape.
                window.on_window_should_close(cx, |_, cx| {
                    let closing = window::should_close();
                    // **D-M16 / docs/MULTIWINDOW.md §7.8.** The primary window
                    // owns the JS pump (D-M07 option 1), so closing it while a
                    // pop-out is open would leave that pop-out painted and
                    // frozen — a live window with no clock, no timers and no
                    // event loop. A pop-out is a VIEW ONTO the app rather than
                    // a peer, so it goes with it, which is also what a
                    // one-main-window Electron app does.
                    if closing && popout::count() > 0 {
                        vn_tree::log::record(
                            "window",
                            format!(
                                "[window] the primary window is closing — taking {} pop-out(s) \
                                 with it (docs/MULTIWINDOW.md §7.8)",
                                popout::count()
                            ),
                        );
                        popout::close_all(cx);
                    }
                    closing
                });
                window.on_next_frame(pump_frame);
                shell
            },
        )
        .map(|handle| MAIN_WINDOW.with(|w| w.set(Some(handle.into()))))
        .unwrap();

        // A DOM layout read lays out, the way a browser's does. Installed
        // before any JS runs, because the very first thing that needs it is the
        // app bundle's own `onMounted`.
        vn_tree::set_sync_layout(sync_layout);
        // `font-family` resolves to the first INSTALLED family in its list, so
        // the mapper needs the host's family names — read lazily, the first
        // time a sheet maps a `font-family`, and before any JS has run.
        let text_system = cx.text_system().clone();
        vn_css::set_font_source(move || text_system.all_font_names());

        // Startup eval: host fns called at top level land in Phase::App.
        with_published_app(cx, || {
            JsRuntime::with(|rt| {
                if let Err(e) = perf::boot_eval("preludes", || rt.eval(PRELUDE_JS, "vn://prelude.js")) {
                    eprintln!("[js boot] {}\n{}", e.header, e.stack);
                }
                if let Err(e) = perf::boot_eval("preludes", || rt.eval(WINDOW_PRELUDE_JS, "vn://window.js")) {
                    eprintln!("[js boot] {}\n{}", e.header, e.stack);
                }
                // `VN_SCENARIO=<name>` — a harness app can pin itself to one
                // screen at boot so a measurement is one process, one scenario,
                // no input. Published as a plain global; an app that does not
                // look for it never notices. (JSON-quoted: the value is
                // whatever the shell handed us.)
                let scenario = std::env::var("VN_SCENARIO").unwrap_or_default();
                if !scenario.is_empty() {
                    let safe: String = scenario
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                        .collect();
                    let js = format!("globalThis.__VN_SCENARIO__ = \"{safe}\";");
                    if let Err(e) = rt.eval(&js, "vn://scenario.js") {
                        eprintln!("[js boot] {}\n{}", e.header, e.stack);
                    }
                }
                // `VN_VOICE_IDENTITY=<name>` — which participant THIS process is
                // in a voice room. The two-instance demo runs the same app in
                // two VN_DIST slots against one SFU, and without a per-process
                // identity they would collide on it. Same publishing shape and
                // the same filter as VN_SCENARIO above; an app that does not
                // look for it never notices. (docs/VOICE.md §3.1)
                let identity = std::env::var("VN_VOICE_IDENTITY").unwrap_or_default();
                if !identity.is_empty() {
                    let safe: String = identity
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                        .collect();
                    let js = format!("globalThis.__VN_VOICE_IDENTITY__ = \"{safe}\";");
                    if let Err(e) = rt.eval(&js, "vn://voice-identity.js") {
                        eprintln!("[js boot] {}\n{}", e.header, e.stack);
                    }
                }
                #[cfg(vn_devtools)]
                if devtools_mode {
                    inspector::boot(rt);
                } else {
                    boot_mode::boot(rt);
                }
                #[cfg(not(vn_devtools))]
                boot_mode::boot(rt);
                // `DOMContentLoaded` + `load`, once, after the app bundle has
                // evaluated — several reused browser components wait on these,
                // and redjs has exported the hook since day one with nothing
                // ever calling it.
                window::content_loaded();
            });
        });
        // An inspector never comes to the front on its own — see
        // `inspector::install_hotkey`. Everything else does, exactly as before.
        if !devtools_mode {
            cx.activate(true);
        }
        #[cfg(vn_devtools)]
        if devtools_mode {
            inspector::watch_parent();
        }
    });
}
