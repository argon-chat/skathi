//! vue-native host shell: JavaScript (JSC via redjs) inside a gpui window.
//!
//! Two boot modes share everything below the [`boot_mode`] alias, and each owns one file that
//! supplies the JS: [`dev`] reads `packages/vue-native/dist` and polls `changes.json` for HMR;
//! [`prod`] (`--features prod`) runs the VNPK payload stamped into the exe ([`payload`]).
//! Both expose `boot(&JsRuntime)` and `poll_frame(frame_no: u32)`.
//!
//! Two ways to run, both through [`run_app`]: [`run_exe`] (the process is the app) and
//! [`embed`] (another program runs the engine on a thread of its own).

use std::cell::RefCell;

use gpui::{
    App, Bounds, Context, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowControlArea,
    WindowOptions, div, prelude::*, px, rgba, size,
};
use gpui_platform::application;
use vn_runtime::{JsRuntime, Phase, host_fn, with_app, with_published, with_published_app};
use vn_runtime::runtime::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

mod background;
mod idle;
#[cfg(feature = "prod")]
mod bytecode;
#[cfg(not(feature = "prod"))]
mod dev;
// `vn_devtools` is emitted by build.rs: every dev build, plus a release that asked for devtools.
#[cfg(vn_devtools)]
mod devtools;
// `pub` for the exe's `#[global_allocator]` and for embedders, which declare the allocator themselves.
#[cfg(vn_devtools)]
pub mod heapcount;
/// Allocator an embedding executable can install to enable native allocation
/// recording. A build without devtools uses the system allocator directly.
#[cfg(vn_devtools)]
pub use heapcount::Counting as HostAllocator;
#[cfg(not(vn_devtools))]
pub use std::alloc::System as HostAllocator;
mod http_client;
// The unit tests are the library's own binary, and some read the counts.
#[cfg(all(vn_devtools, test))]
#[global_allocator]
static GLOBAL: heapcount::Counting = heapcount::Counting;
mod dialogs;
#[cfg(target_os = "windows")]
#[doc(hidden)] // `preview` is examples/consent_prompt.rs's
pub mod consent_window;
#[cfg(target_os = "windows")]
mod content_dialog;
#[doc(hidden)] // `preview` is examples/crash_dialog.rs's
pub mod crash_window;
pub mod js; // direct JavaScriptCore values and VM-thread dispatch for embedders
pub mod embed; // vue-native as a library: another program runs it on a thread
pub mod tray; // native application tray for Rust embedders and direct JavaScript callbacks
#[cfg(vn_devtools)]
mod inspector;
#[cfg(all(vn_devtools, feature = "native-aot"))]
mod native_aot;
mod json;
#[cfg(feature = "audio")]
mod microphone;
mod mapped_file;
mod owner;
mod payload;
mod occlusion;
mod perf;
mod reclaim;
mod popout;
mod popout_win;
#[cfg(feature = "prod")]
mod prod;
#[cfg(all(feature = "webgpu", target_os = "windows"))]
mod shared_gpu;
mod quiet;
mod reload;
mod shutdown;
mod slot;
mod window;
#[cfg(target_os = "linux")]
mod linux_chrome;

#[cfg(not(feature = "prod"))]
use dev as boot_mode;
#[cfg(feature = "prod")]
use prod as boot_mode;

/// Vue's esm-bundler build reads these three flags off globalThis (bun refuses to inline them).
/// Must run BEFORE the app bundle, since ESM import hoisting would put the bundle's Vue code first.
///
/// Generated from `prelude.ts` and `window.prelude.ts` beside this file (`bun run preludes`) and
/// checked in so `cargo build` needs no JS toolchain.
const PRELUDE_JS: &str = include_str!("prelude.js");

/// The windowing surface's JS half, evaluated right after [`PRELUDE_JS`] and before the app bundle,
/// so `document.title`, `screen`, `matchMedia` and `navigator.windowControlsOverlay` are real by
/// the first component's `<script setup>`.
const WINDOW_PRELUDE_JS: &str = include_str!("window.prelude.js");

thread_local! {
    static FRAME_NO: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static FIRST_WAVE_RELEASED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The first JS wave waits one frame for the first draw, so rAF and `onMounted` timers measure a
/// laid-out tree instead of reading 0. Later waves are never delayed by this.
fn first_wave_waits(window: &mut Window) -> bool {
    if FIRST_WAVE_RELEASED.with(|c| c.get()) {
        return false;
    }
    FIRST_WAVE_RELEASED.with(|c| c.set(true));
    window.refresh();
    true
}

// `VN_LOG` turns on gpui's own log records (adapter choice, device loss, font fallback). It takes a
// bare level or `RUST_LOG`-style per-target directives (`VN_LOG=off,gpui=debug,vn_tree=trace`): a
// bare `debug` is chatty enough to disturb the timing being debugged.
// Records are teed into `vn_tree::log`'s ring, which is thread-local: hence [`OFF_THREAD`] and
// [`drain_log_tee`].
struct StderrLogger {
    /// Longest prefix first, so `gpui::window` beats `gpui`.
    directives: Vec<(String, log::LevelFilter)>,
    default: log::LevelFilter,
}

/// `::`-boundary-aware prefix match: `gpui_windows` must not match a `gpui` directive.
fn target_matches(target: &str, prefix: &str) -> bool {
    target.len() >= prefix.len()
        && target.starts_with(prefix)
        && (target.len() == prefix.len() || target[prefix.len()..].starts_with("::"))
}

impl StderrLogger {
    fn level_for(&self, target: &str) -> log::LevelFilter {
        for (prefix, l) in &self.directives {
            if target_matches(target, prefix) {
                return *l;
            }
        }
        self.default
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
        // stderr first: unbuffered, so a message logged on the way to `abort()` still gets out.
        eprintln!("{line}");
        tee(record.level(), line);
    }
    fn flush(&self) {}
}

/// The thread whose `vn_tree::log` ring `ctl.ts logs` reads.
static MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();
/// Records made on a worker thread, waiting for a frame.
static OFF_THREAD: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());
/// Bounds a wedged subsystem logging in a loop.
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

/// Moves worker-thread records into the main thread's ring, once per frame; `try_lock` so a frame
/// never blocks on a logging thread.
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
    // An unset `VN_LOG` still installs the logger; it passes nothing.
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
    // `VN_LOG=gpui=debug` alone means that target only; an unset `VN_LOG` must stay Off.
    if !saw_bare && directives.is_empty() && !spec.is_empty() {
        default = log::LevelFilter::Info;
    }
    directives.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    let max = directives
        .iter()
        .map(|(_, l)| *l)
        .chain([default])
        .max()
        .unwrap_or(default);
    let logger: &'static StderrLogger =
        Box::leak(Box::new(StderrLogger { directives, default }));
    if log::set_logger(logger).is_ok() {
        // Must be the loosest directive, or records are dropped before `enabled` sees their target.
        log::set_max_level(max);
    }
}

/// Pins the build model: `build.rs` emits `cfg(vn_devtools)` unless `no-devtools` was asked, for
/// every feature combination.
#[cfg(test)]
#[test]
fn devtools_are_in_every_build_that_did_not_opt_out() {
    assert_eq!(cfg!(vn_devtools), !cfg!(feature = "no-devtools"));
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
    fn an_unset_vn_log_passes_nothing() {
        let l = logger(&[], L::Off);
        assert_eq!(l.level_for("parcel_selectors::matching"), L::Off);
        assert_eq!(l.level_for("gpui::platform"), L::Off);
    }

    #[test]
    fn the_longest_matching_directive_wins() {
        let l = logger(&[("gpui", L::Warn), ("gpui::window", L::Debug)], L::Off);
        assert_eq!(l.level_for("gpui::window"), L::Debug);
        assert_eq!(l.level_for("gpui::platform"), L::Warn);
        assert_eq!(l.level_for("vn_tree"), L::Off);
    }

    #[test]
    fn target_boundaries_are_module_separators() {
        assert!(target_matches("gpui::window", "gpui"));
        assert!(target_matches("gpui", "gpui"));
        assert!(!target_matches("gpui_windows::events", "gpui"));
    }
}

struct Shell {
    status: String,
    log: Vec<String>,
    /// The fallback titlebar's text (`window.title`).
    title: String,
}

thread_local! {
    static SHELL: RefCell<Option<WeakEntity<Shell>>> = const { RefCell::new(None) };
    /// The primary window itself, not its root view: the multi-window repaint fan-out runs inside
    /// input listeners, where the root view may already be borrowed.
    static MAIN_WINDOW: std::cell::Cell<Option<gpui::AnyWindowHandle>> =
        const { std::cell::Cell::new(None) };
}

/// The forced synchronous layout a DOM measurement asks for (`vn_tree::set_sync_layout`).
///
/// Draws the primary window once. `with_app` is `None` outside a published phase, including inside
/// a real frame, so this never runs inside a draw.
///
/// When the window is not leased (boot, first mount) it goes through its handle. Every later JS
/// wave runs leased by its own update (frame pump, media tick), where `handle.update` fails, so
/// the draw goes through the window that wave published.
///
/// Never inside a gpui input listener (`vn_tree::in_input_dispatch`): gpui has the frame's
/// listeners taken out, and a draw would swap the frame under them. The read stays stale there.
fn sync_layout() {
    let Some(handle) = MAIN_WINDOW.with(|w| w.get()) else { return };
    // Drawn as a frame draws: with nothing published (`bridge::with_draw`).
    let draw = |window: &mut gpui::Window, cx: &mut gpui::App| vn_runtime::bridge::with_draw(|| window.draw(cx).clear(cx));
    let drew = vn_runtime::bridge::with_app(|cx| handle.update(cx, |_, window, cx| draw(window, cx)).is_ok());
    if drew != Some(false) || vn_tree::in_input_dispatch() {
        return;
    }
    vn_runtime::with_commit(|window, cx| {
        if window.window_handle() == handle {
            draw(window, cx);
        }
    });
}

fn commit_boot_tree(runtime: &JsRuntime) {
    // Vue's mount/load handlers may queue their last DOM writes in a microtask.
    runtime.ctx().drain_microtasks();
    vn_tree::flush_changes();
}

#[cfg(test)]
mod startup_tests {
    #[test]
    fn boot_microtask_styles_are_computed_before_any_frame_pump() {
        vn_runtime::testing::on_js_thread(|| vn_runtime::JsRuntime::with(|runtime| {
            vn_tree::install(runtime);
            runtime.eval(
                "globalThis.__startupRoot = __vn_create_root(); \
                 globalThis.__startupNode = __vn_create_element(__vn_intern('button')); \
                 __vn_insert(__startupNode, __startupRoot, 0); \
                 __vn_set_style(__startupNode, __vn_intern('opacity'), '0.25'); \
                 queueMicrotask(() => __vn_set_style(__startupNode, __vn_intern('opacity'), '0.75'));",
                "vn://startup-regression.js",
            ).expect("queue the mount/load microtask through the native DOM bridge");
            super::commit_boot_tree(runtime);
            let node = runtime.ctx().global_get("__startupNode").unwrap().as_number().unwrap() as u32;
            // Reads existing computed state without forcing the cascade (getComputedStyle would flush it).
            let styles = vn_tree::devtools::computed_styles(node).expect("boot writes must already be cascaded");
            let base = &styles.iter().find(|(name, _)| *name == "base").unwrap().1;
            assert!(base.contains("0.75"), "the first draw must not await pump_frame to cascade boot opacity: {base}");
        }));
    }
}

/// How long a hidden start may go without a tray icon before the window is shown anyway.
const HIDDEN_START_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// Safety net for a hidden start: if no tray icon is up and nothing has shown the window by
/// [`HIDDEN_START_GRACE`], show it so a failed tray cannot strand the app.
fn reveal_if_unreachable(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(HIDDEN_START_GRACE).await;
        let _ = cx.update(|_| {
            if shutdown::is_closing() || window::revealed() || vn_desktop::tray::Tray::app_shown() {
                return;
            }
            let line = format!(
                "[vn] started hidden, and no tray icon is up after {} s: showing the window so the app can be reached",
                HIDDEN_START_GRACE.as_secs()
            );
            eprintln!("{line}");
            vn_tree::log::record("warn", line);
            window::request_activate();
        });
    })
    .detach();
}

#[cfg(target_os = "windows")]
fn reveal_startup_window(cx: &mut App, activate: bool) {
    let Some(handle) = MAIN_WINDOW.with(|window| window.get()) else {
        window::cancel_startup_reveal();
        return;
    };
    cx.spawn(async move |cx| {
        if shutdown::is_closing() || vn_runtime::bridge::poisoned() {
            window::cancel_startup_reveal();
            return;
        }
        let hwnd = cx.update(|cx| handle.update(cx, |_, window, _| window::capture_hwnd(window)));
        let Ok(Some(hwnd)) = hwnd else {
            window::cancel_startup_reveal();
            return;
        };
        // WM_PAINT takes the App itself, so it must run outside any update.
        popout_win::paint_now(hwnd);
        if shutdown::is_closing() || vn_runtime::bridge::poisoned() {
            window::cancel_startup_reveal();
            return;
        }
        let shown = cx.update(|cx| handle.update(cx, |_, window, _| {
            window.show_window(
                activate,
                || !shutdown::is_closing() && !vn_runtime::bridge::poisoned(),
                move |shown| {
                    window::cancel_startup_reveal();
                    if shown {
                        quiet::send_hwnd_to_back(hwnd);
                    }
                    if !shown && !shutdown::is_closing() {
                        eprintln!("[window] the prepared startup window could not be shown");
                    }
                },
            )
        }));
        if !matches!(shown, Ok(true)) {
            window::cancel_startup_reveal();
            eprintln!("[window] failed to reveal the prepared startup window");
        }
    }).detach();
}

/// The primary window's gpui id — what the devtools `windows` command calls
/// `main`, and what `--window` names when F12 was pressed there.
#[cfg(vn_devtools)]
pub(crate) fn main_window_id() -> Option<u64> {
    MAIN_WINDOW.with(|w| w.get()).map(|h| h.window_id().as_u64())
}

/// Repaint the primary window via `notify_shell`, not `Window::refresh`: `refresh` raises gpui's
/// `refreshing` flag, which makes the frame ignore every `Entity::cached` recording and defeats the
/// retained-render cache.
pub(crate) fn refresh_main(cx: &mut App) {
    notify_shell(cx);
}

/// Repaint the primary window. A pop-out opening or closing moves no tree state, so those call this directly.
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
        None => vn_runtime::bridge::throw_error(g, 1, "__vn_*: called outside a published phase"),
    }
}

host_fn!(fn vn_set_status(g, args) {
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    // The CURRENT value, not a stream: a 2 Hz status ticker must not evict the log ring.
    vn_tree::log::set_status(text.clone());
    update_shell(g, move |shell| shell.status = text)
});

// `__vn_perf_mark(label)` labels the profiler's frame-time window; a no-op unless `VN_PERF` is set.
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

/// The per-frame JS pump: runs before the frame's draw, so `cx.notify()` from a host fn lands in the same frame.
fn pump_frame(window: &mut Window, cx: &mut App) {
    #[cfg(vn_devtools)]
    let _allocation_scope = crate::heapcount::recording::scope("runtime", "frame/pump");
    // A cloaked or occluded window is a hidden page: decline the pump (no JS wave, no rAF) and, by
    // not stamping the frame clock, hand the window to the background tick. Not before the first
    // frame, which owns the boot wave.
    if background::pumped_once() && (window::cloaked() || window::occluded()) {
        idle::rearm_paced(window, pump_frame, idle::CLOAKED_INTERVAL);
        return;
    }
    // `on_next_frame` re-arms every frame, so this is the frame clock.
    perf::frame();
    background::note_frame();
    // Before the poison gate: a worker's log is what an agent wants once JS has given up.
    drain_log_tee();
    if vn_runtime::bridge::poisoned() {
        // No JS after a caught native panic: put up the fatal dialog once and do not re-arm.
        crash_window::on_poisoned(cx);
        return;
    }
    if first_wave_waits(window) {
        idle::rearm_unpaced(window, pump_frame);
        return;
    }
    // The frame's one coalesced mousemove, before the pump so what its handler schedules drains this frame.
    vn_tree::flush_pending_mouse_move(window, cx);
    // An inspector whose app has gone closes itself; outside the publication below, which borrows `cx`.
    #[cfg(vn_devtools)]
    inspector::poll_frame(cx);
    js_wave(window, cx, Wave::Frame);
    // After the painted frame, so "did the app declare a drag region" has an answer.
    window::note_painted_frame();
    after_wave(window, cx);
    #[cfg(vn_devtools)]
    if devtools::busy() {
        idle::note_moved();
    }
    idle::rearm(window, pump_frame, window::app_focused(cx) || quiet::never_focus());
    // Closes the `pump` bucket; what follows is gpui's.
    perf::pump_done();
    // ResizeObserver/IntersectionObserver run after layout and before paint: when the frame will
    // draw, this draws it ahead of gpui's (which then only presents). After `pump_done` so that
    // draw stays in gpui's bucket of the frame trace.
    vn_tree::update_layout_observers(window, cx);
    // The background tick measures the stall from here, so a slow frame is not mistaken for a parked one.
    background::note_frame();
}

/// Which clock drives a JS wave.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wave {
    /// The window's own frame; includes animation frames.
    Frame,
    /// The background tick while the window draws no frames: no `requestAnimationFrame`, as in a
    /// hidden page.
    Background,
}

/// One JS wave inside one `Phase::Commit` publication, shared by the frame and the background tick.
pub(crate) fn js_wave(window: &mut Window, cx: &mut App, wave: Wave) {
    let focused = window::app_focused(cx) || quiet::never_focus();
    with_published(window, cx, Phase::Commit, || {
        // Poll dist/changes.json and apply HMR BEFORE the pump, so what Vue schedules while patching
        // drains this frame. Frames only: it counts frames.
        if wave == Wave::Frame {
            let n = FRAME_NO.with(|f| {
                let n = f.get().wrapping_add(1);
                f.set(n);
                n
            });
            boot_mode::poll_frame(n);
        }
        // `location.reload()` asked last frame: tear down and re-run now so the new app's first wave drains this frame.
        reload::pump();
        // One window-state snapshot, then resize, visibility, the JS observer, before `rt.pump` so
        // `resize` handlers' DOM mutations drain this frame and `fullscreenchange` follows the resize.
        vn_runtime::with_commit(|window, cx| window::pump(window, cx));
        // IndexedDB results landing mid-pump get a quarter of THIS display's frame.
        vn_runtime::runtime::set_frame_budget_ms(window::frame_budget_ms());
        // No rAF on the background tick: a hidden page's does not run.
        let flags = match wave {
            Wave::Frame => PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS,
            Wave::Background => PUMP_TIMERS | PUMP_MICROTASKS,
        };
        JsRuntime::with(|rt| {
            let pumped = std::time::Instant::now();
            let stats = rt.pump(flags);
            let elapsed = pumped.elapsed();
            if wave == Wave::Frame {
                #[cfg_attr(not(feature = "audio"), allow(unused_mut))]
                let mut report = idle::Wave::from_stats(&stats, elapsed);
                #[cfg(feature = "audio")]
                {
                    report.audio = vn_runtime::redjs::webapi::audio::busy(rt.raw());
                }
                idle::note_wave(report);
            }
            #[cfg(vn_devtools)]
            devtools::trace::note_js(elapsed.as_secs_f32() * 1000.0);
            for err in rt.drain_errors() {
                eprintln!("[js uncaught] {}\n{}", err.header, err.stack);
                vn_tree::log::record(
                    "error",
                    format!("[js uncaught] {}\n{}", err.header, err.stack),
                );
            }
            // After the wave, so what it just dropped is garbage by now.
            reclaim::after_wave(rt, focused, wave == Wave::Background || window::page_hidden());
        });
        // One queued devtools command per frame, in the same publication, so it never sees a half-applied patch.
        #[cfg(vn_devtools)]
        {
            devtools::pump();
        }
        // The embedder's frame hooks (`HostContext::on_frame`), same publication.
        js::frame();
        // Settled `appPermissions.request()` promises; `change` events are dispatched by redjs inside `rt.pump()`.
        vn_permissions::pump();
        // Settled `appShell.*` promises and unsolicited desktop events.
        vn_desktop::pump();
        // A close the embedder asked for.
        embed::pump();
        // At most one localStorage file write per frame: the frame clock is the coalescing clock.
        #[cfg(feature = "storages")]
        vn_storage::pump();
    });
}

/// What follows a JS wave on both clocks: media tick, tree flush, repaint request, devtools push.
/// On the background tick the repaint request only marks the window dirty.
pub(crate) fn after_wave(window: &mut Window, cx: &mut App) {
    // Media ticks (`<lottie>`, `<video>`) and detached-focus cleanup need a `Window` and run before
    // the draw, so a media element paints this tick's picture. The device check comes first: after a
    // renderer device loss every `<video>` must rebuild before it ticks.
    publish_video_device();
    let media_moved = vn_tree::frame_tick(window, cx);
    // One notify per JS wave that touched the tree. While transitions or a Lottie run keep
    // repainting: `on_next_frame` re-arms but does not redraw.
    let flush = vn_tree::flush_changes_kind();
    let tree_moved = flush != vn_tree::Flush::Unchanged;
    // The push channel's tree revision: lets an inspector refetch because something moved.
    #[cfg(vn_devtools)]
    if tree_moved {
        devtools::ws::note_tree_changed();
    }
    // A playing `<video>` or `<lottie>` is not listed: their ticks ask for a draw only when a present
    // cannot show the frame (`media_moved`).
    // A hidden page paints no animation; the first frame after it is shown resumes them.
    let animating = vn_tree::animating() && !window::page_hidden();
    let moving = animating || media_moved;
    if flush == vn_tree::Flush::DrawnBySync {
        // Counted for the frame trace: a forced layout already drew this tree.
        // With no newer work, the primary-window notification below is skipped.
        vn_tree::perf::note_sync_tail(!moving);
    }
    if flush == vn_tree::Flush::Changed || moving {
        // A synchronous measurement already built the primary window; keep that frame unless newer work remains.
        notify_shell(cx);
    }
    if tree_moved || moving {
        // CSS animations alone are not work: an unfocused window paces them.
        if tree_moved || media_moved {
            idle::note_moved();
        } else {
            idle::note_animating();
        }
        // The synchronous layout draws only the primary window; pop-outs still need this wave.
        popout::notify_all(cx);
    }
    // Push, not poll, after the flush, so a socket is never told about a revision the tree has not reached.
    #[cfg(vn_devtools)]
    devtools::ws::broadcast_frame();
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(vn_devtools)]
        let _allocation_scope = crate::heapcount::recording::scope("render", "frame/build");
        perf::render_begin();
        let el = self.render_body(window, cx);
        perf::render_done();
        el
    }
}

impl Shell {
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        // The built-in titlebar is a fallback: it stays until the app paints a `window-drag: move` region, then retires.
        let shell = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont");
        #[cfg(target_os = "linux")]
        let shell = linux_chrome::wrap(shell, window);
        if !window::fallback_titlebar() {
            // Custom chrome: the app owns the whole window; the shell is a bare mount point.
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
                    // `flex_1().min_h(0)` gives the app root a definite box; an auto-height slot would
                    // break `h-full`, `overflow: auto` and `position: fixed` inside the app.
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

/// The devtools element outline, in window coordinates. It sits in the shell rather than the app's
/// tree so the inspector never appears in the tree it inspects.
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

/// The host-drawn caption-button cluster (`titleBarOverlay`), top-right; the rect apps read back
/// (`getTitlebarAreaRect()`) loses exactly this width.
///
/// `window_control_area` makes `WM_NCHITTEST` answer HTMINBUTTON/HTMAXBUTTON/HTCLOSE, so the OS
/// performs the action and HTMAXBUTTON arms the Windows 11 snap-layouts flyout.
fn window_controls_overlay() -> Option<gpui::Div> {
    let (height, overlay) = window::overlay()?;
    // Segoe Fluent Icons is the Win11 caption glyph font; Win10 has the same code points in Segoe
    // MDL2 Assets. `Font::fallbacks` is DirectWrite's per-code-point chain, so each OS finds the
    // glyph in its own family.
    const MIN: &str = "\u{e921}";
    const MAX: &str = "\u{e922}";
    const CLOSE: &str = "\u{e8bb}";
    let glyph_font = gpui::Font {
        family: "Segoe Fluent Icons".into(),
        fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
            "Segoe MDL2 Assets".to_string(),
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
            // Win11's close button turns #C42B1C on hover; matching the OS matters more than the app palette.
            .child(button("wco-close", CLOSE, WindowControlArea::Close, 0xc42b1cff)),
    )
}

/// The directory a relative `<img src>` resolves against. `VN_APP_ROOT` wins; otherwise [`dev::boot`]
/// derives it from the manifest entry. Prod leaves it unset, and relative paths fall back to the
/// working directory with an advisory.
fn publish_app_root() {
    // A packaged exe: relative URLs resolve out of the payload, like `fetch()`.
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

/// Installs the webview behind `<iframe>`, if this platform has one.
fn install_iframe_backend() {
    #[cfg(feature = "storages")]
    let data = embed::data_dir()
        .map(std::path::Path::to_path_buf)
        .or_else(|| vn_storage::default_folder(boot_mode::app_name().as_deref()).parent().map(std::path::Path::to_path_buf));
    #[cfg(not(feature = "storages"))]
    let data: Option<std::path::PathBuf> = None;
    let user_data = data.unwrap_or_else(|| std::env::temp_dir().join("vue-native")).join("webview");
    let config = vn_webview::Config {
        assets: std::rc::Rc::new(|path| vn_tree::read_app_asset(path)),
        user_data,
        // Matches redjs's `location.origin`.
        page_origin: "app://redjs".into(),
        devtools: !cfg!(feature = "prod"),
    };
    if let Some(backend) = vn_webview::backend(config) {
        vn_tree::iframe::set_backend(backend, std::rc::Rc::new(vn_runtime::frames::wake));
    }
}

/// On quit: every `<iframe>`'s document is closed, so no browser process
/// outlives the app.
fn close_iframes_on_quit(cx: &mut App) {
    std::mem::forget(cx.on_app_quit(|_| {
        vn_tree::iframe::shutdown();
        std::future::ready(())
    }));
}

/// Hands `<video>` the renderer's D3D11 device, and re-hands it whenever the renderer recreates one
/// (device loss after a TDR or driver update): each `<video>` is rebuilt on the new device, keeping
/// its position and transport state. Runs every frame; two atomic loads when nothing changed.
///
/// The pointer is BORROWED from `DirectXDevices`; `vn_video` clones the COM interface if it keeps it.
#[cfg(target_os = "windows")]
fn publish_video_device() {
    use std::cell::Cell;
    #[cfg(feature = "native-dx12")]
    {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            fn provide() -> Result<Option<vn_tree::SharedVideoDevice>, String> {
                gpui_windows::ensure_renderer_d3d11_device()
                    .map(|lease| lease.map(|lease| vn_tree::SharedVideoDevice {
                        device: lease.device,
                        generation: lease.generation,
                        keep_alive: lease.keep_alive,
                    }))
                    .map_err(|error| format!("Could not initialize renderer video bridge: {error:#}"))
            }
            // The renderer's owned lease keeps the raw COM pointer valid.
            if let Err(error) = unsafe { vn_tree::install_video_device_provider(provide) } {
                log::error!("Could not install lazy video device provider: {error}");
            }
        });
    }
    thread_local! {
        /// 0 is never a live generation — `publish_device` pre-increments.
        static PUBLISHED: Cell<u64> = const { Cell::new(0) };
    }
    let (ptr, generation) = gpui_windows::renderer_d3d11_device();
    let previous = PUBLISHED.with(Cell::get);
    if generation == 0 || previous == generation || (ptr.is_null() && previous == 0) {
        return;
    }
    // A new native12 generation has no D3D11 bridge yet: still rebuild old decoders even with a null
    // pointer; their next load asks the lazy provider.
    let first = previous == 0;
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

/// The HTTP client gpui fetches `Resource::Uri` assets through: redjs's network stack, as `fetch`
/// uses ([`http_client`]). Without one, gpui's `NullHttpClient` fails every request.
///
/// `VN_HTTP_CLIENT=reqwest` installs the fork's `ReqwestClient` instead, to bisect image-loading
/// problems without a rebuild.
fn install_http_client(app: gpui::Application) -> gpui::Application {
    if std::env::var("VN_HTTP_CLIENT").is_ok_and(|v| v.eq_ignore_ascii_case("reqwest")) {
        return match reqwest_client::ReqwestClient::user_agent(concat!("vue-native/", env!("CARGO_PKG_VERSION"))) {
            Ok(client) => app.with_http_client(std::sync::Arc::new(client)),
            Err(e) => {
                log::warn!("VN_HTTP_CLIENT=reqwest: the client could not be built ({e}); http(s) images will not load");
                app
            }
        };
    }
    app.with_http_client(std::sync::Arc::new(http_client::RedjsHttpClient::new()))
}

/// The host exe's `main` (src/main.rs): this process IS the app.
pub fn run_exe() {
    // First, so a host whose runner is already gone never opens a window.
    owner::watch();
    // The one parse of the command line; every consumer below reads this, never `std::env::args()`.
    let args = vn_desktop::args::process();
    // `vn build`'s host check: a bare host prints one JSON line and exits before any window or JS
    // exists. A packaged exe's argv belongs to its app.
    if args.host.host_info && !payload::embedded_present() {
        println!("{}", payload::host_info_json());
        return;
    }
    // `--vn-dist <slot>` is `VN_DIST` on the command line (a process the shell starts for a protocol
    // link inherits no environment). Before anything reads the slot.
    if let Some(dist) = args.host.vn_dist.as_deref().filter(|d| !d.is_empty()) {
        std::env::set_var("VN_DIST", dist);
    }
    // Single instance, before the inspector, logger and gpui: a second launch of a `singleInstance`
    // app forwards its argv/url/cwd and exits without creating a window or touching `localStorage`.
    // The inspector (`--devtools`) is never an instance.
    {
        let launch_cfg = if args.host.devtools { None } else { boot_mode::launch_config() };
        // A dev host's slot, for its protocol registrations. The instance key is the app's id and
        // storage folder, not the slot (vn-test and agents' copies opt out with VN_SINGLE_INSTANCE=0).
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
        // A packaged exe re-registers its declared `protocols` per user, idempotently; a dev host only says it did not.
        if !args.host.devtools {
            vn_desktop::launch::register_declared();
        }
    }
    // `--devtools` rewrites the dist slot and preferred-instance env vars, so it runs before anything reads them (via `slot::dir`).
    #[cfg(vn_devtools)]
    inspector::init();
    // A prod build with no app is a bare host: say so and stop.
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
    run_app();
    // gpui has quit: say how the close went and end the process without the C runtime's exit path.
    shutdown::finish();
}

/// An embedded vue-native's thread: the exe's boot without the process-owned parts. The embedder
/// owns the command line and is the instance; the end is reported rather than taken.
pub(crate) fn run_embedded() {
    let args = vn_desktop::args::process();
    // Records the launch `appLaunch` reports.
    if !embed::desktop_launch_prepared() {
        let _ = vn_desktop::launch::start(args, None, boot_mode::app_name(), None);
    }
    run_app();
    shutdown::finish();
}

/// Everything from the logger to gpui's quit, shared by the exe and embedders.
fn run_app() {
    // Before anything gpui does, so the renderer's boot messages are visible.
    install_logger();
    #[cfg(all(feature = "webgpu", target_os = "windows"))]
    shared_gpu::install().expect("installing shared graphics device provider");
    perf::boot();
    // With the Rust heap counted, arm the painted-image registry the `mem` command reports.
    #[cfg(vn_devtools)]
    if heapcount::on() {
        vn_tree::perf::set_live_bytes_probe(|| heapcount::snapshot().map(|s| s.0));
        vn_tree::devtools::set_image_tracking(true);
    }
    // The primary window owns the JS pump; closing it ends the host on every OS (AppKit otherwise keeps a windowless app running).
    let app = install_http_client(application())
        .with_quit_mode(gpui::QuitMode::LastWindowClosed);
    // A hidden primary window comes back on a Dock click as on a tray click; the show is queued, never applied inside AppKit's callback.
    #[cfg(target_os = "macos")]
    app.on_reopen(|_| embed::reopen());
    // `Resource::Embedded(key)` (an `<img>` in a packaged exe) reads from the mapped payload.
    #[cfg(feature = "prod")]
    let app = if payload::get().is_some() { app.with_assets(payload::GpuiAssets) } else { app };
    app.run(|cx: &mut App| {
        // `--devtools` is a third boot mode over this same shell: the app is the inspector, from this executable.
        #[cfg(vn_devtools)]
        let devtools_mode = inspector::is_devtools_host();
        #[cfg(not(vn_devtools))]
        let devtools_mode = false;
        vn_elements::init(cx); // key bindings for the native <input> widget
        publish_app_root();
        // The first quit hook (they run in order of addition): a quit that no accepted close
        // announced, such as the session ending, hides the windows and arms the deadline here.
        std::mem::forget(cx.on_app_quit(|_| {
            window::cancel_startup_reveal();
            shutdown::begin("quit");
            std::future::ready(())
        }));
        std::mem::forget(cx.on_app_quit(|_| {
            JsRuntime::try_with(|runtime| js::shutdown(runtime.ctx()));
            std::future::ready(())
        }));
        close_iframes_on_quit(cx);
        #[cfg(vn_devtools)]
        {
            // F12 in every window of this process (see `inspector::install_hotkey`).
            inspector::install_hotkey(cx);
            // A debugger must not outlive its subject (the child also watches us, for a crash).
            std::mem::forget(cx.on_app_quit(|_| {
                shutdown::step("inspector", inspector::close_children);
                std::future::ready(())
            }));
        }
        // Flush the cookie jar and `localStorage` so a final-frame `Set-Cookie` or `pagehide` `setItem` reaches disk.
        #[cfg(feature = "storages")]
        std::mem::forget(cx.on_app_quit(|_| {
            shutdown::step("storage", || {
                // The last attempt bypasses the automatic retry delay.
                if let Err(e) = vn_storage::flush() {
                    eprintln!("[storage] localStorage: the last change was not written: {e}");
                }
                if let Err(e) = vn_storage::cookies::flush() {
                    eprintln!("[storage] cookies: the last change was not written: {e}");
                }
            });
            // A kill loses only HTTP cache recency; the entry list is rebuilt from the folder at next start.
            shutdown::step("http-cache", vn_runtime::redjs::webapi::http::cache::uninstall);
            std::future::ready(())
        }));
        // Drop uncommitted IndexedDB transactions and close every database on the server thread
        // (checkpoints the WAL for SQLite). Synchronous, since gpui gives quit futures only 200 ms.
        #[cfg(feature = "indexeddb")]
        std::mem::forget(cx.on_app_quit(|_| {
            let t = std::time::Instant::now();
            let done = shutdown::step("indexeddb", || vn_runtime::redjs::ffi::redjs_idb_shutdown(2_000));
            if done == 0 {
                eprintln!(
                    "[vn] IndexedDB did not finish closing within {} ms; the next start recovers from its WAL",
                    t.elapsed().as_millis()
                );
            }
            std::future::ready(())
        }));
        shutdown::install_test_delay(cx);
        // Before the runtime exists so the log ring records the first advisory. Only the recording half; `devtools::boot` decides whether to listen.
        #[cfg(vn_devtools)]
        devtools::arm();

        // The window block of vn.config.json is read before `open_window` (only that tier can set the
        // creation-time-only options without a flash) and before the JS runtime (it carries
        // `"devtools": false`, which must be answered before a port is bound). `VN_WINDOW` and
        // `VN_WINDOW_FOCUS` win over it.
        #[cfg(vn_devtools)]
        let window_json =
            if devtools_mode { inspector::window_config() } else { boot_mode::window_config() };
        #[cfg(not(vn_devtools))]
        let window_json = boot_mode::window_config();
        // The HTTP cache cap rides in the same block; read before it is consumed.
        #[cfg(feature = "storages")]
        let http_cache_mb = http_client::cache_mb(window_json.as_deref());
        let cfg = window::load_config(window_json);

        // The runtime devtools switch (config, then `VN_DEVTOOLS`), before `JsRuntime::init` so the bundle's first line sees the settled state.
        #[cfg(vn_devtools)]
        devtools::boot(window::devtools_config_enabled());

        // `navigator.onLine` from the OS connectivity hint, seeded before `JsRuntime::init` so the first read is the OS's answer and queues no event.
        vn_desktop::network::install();

        #[cfg(all(vn_devtools, not(feature = "native-aot")))]
        if let Some(summary) = devtools_mode.then(|| inspector::engine_without_native(
            std::env::var_os("VN_DEVTOOLS_ENGINE").as_deref(),
            std::env::var_os("VN_DEVTOOLS_NATIVE_IMAGE").is_some(),
        ).err()).flatten() {
            eprintln!("[native-aot] {summary}");
            embed::finished(false, summary);
            cx.quit();
            return;
        }
        #[cfg(all(vn_devtools, feature = "native-aot"))]
        if let Err(error) = native_aot::prepare(devtools_mode) {
            let summary = format!("native inspector preparation failed: {error}");
            eprintln!("[native-aot] {summary}");
            embed::finished(false, summary);
            cx.quit();
            return;
        }
        JsRuntime::init();
        // The permission prompt's window task, before any JS can ask for one.
        #[cfg(target_os = "windows")]
        consent_window::install(cx);
        let setup_result = JsRuntime::with(|rt| -> Result<(), String> {
            rt.register("__vn_set_status", 1, vn_set_status as *const _);
            rt.register("__vn_log", 1, vn_log as *const _);
            rt.register("__vn_perf_mark", 1, vn_perf_mark as *const _);
            reload::install(rt);
            vn_tree::install(rt);
            window::install(rt);
            popout::install(rt);
            #[cfg(feature = "audio")]
            microphone::install();
            // `navigator.clipboard` over gpui's clipboard, the one native <input> uses, so there is one owner of `OpenClipboard`.
            #[cfg(feature = "clipboard")]
            vn_clipboard::install();
            // The backend behind `navigator.permissions` and `__vn_permissions_*`; before any app code.
            vn_permissions::install(rt);
            // `<iframe>`: nothing starts until the first iframe with a document; without a backend every `src` answers `error`.
            if !devtools_mode {
                install_iframe_backend();
            }
            // `__vn_desktop_*` / `__vn_shell_*`: the settle dispatcher and shell behind `<a href>`, `window.open(url)` and `appShell`.
            vn_desktop::set_log_sink(|kind, text| vn_tree::log::record(kind, text));
            vn_desktop::install(rt);
            // A file dialog is owned by the window published for the call that asked.
            vn_desktop::dialog::set_owner_resolver(dialogs::owner);
            // The microphone prompt is vn-desktop's question dialog, owned the same way and named after the window title; on Windows a window of our own.
            vn_permissions::set_asker(dialogs::ask_permission);
            vn_permissions::set_app_name(&cfg.title);
            // The crash dialog's title names the app the same way.
            crash_window::set_app_name(&cfg.title);
            // The backend behind `localStorage` and `__vn_storage_*`. Before any app code: the area loads
            // lazily on first touch, so a late install would hand the app an empty `localStorage`.
            // `boot_mode::app_name` names the default folder in a dev host.
            #[cfg(feature = "storages")]
            if !devtools_mode {
                let encrypted = embed::storage_encrypted();
                if embed::data_dir().is_some() || encrypted.is_some() {
                    let directory = embed::data_dir().map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| vn_storage::default_folder(boot_mode::app_name().as_deref()));
                    vn_storage::set_storage_folder(&directory, encrypted)?;
                    if encrypted == Some(true) {
                        vn_storage::encrypt_local_storage_at(&directory)?;
                    }
                }
            }
            #[cfg(feature = "storages")]
            vn_storage::install(
                rt,
                if devtools_mode { Some("vn-devtools".to_string()) } else { boot_mode::app_name() },
            );
            // The disk HTTP cache under `fetch`, XHR and `<img>`, sealed when storage is. After `install`
            // (which names the app), before any JS; not for the inspector.
            #[cfg(feature = "storages")]
            if !devtools_mode {
                if let Some(directory) = embed::data_dir() {
                    vn_storage::http_cache::install_at(http_cache_mb, directory.join("cache").join("http"));
                } else {
                    vn_storage::http_cache::install(http_cache_mb);
                }
            }
            // The answers to the app's permission prompt live in the storage folder; without storage they last for the run.
            #[cfg(feature = "storages")]
            vn_permissions::set_decisions_file(vn_storage::cookies::folder().map(|f| f.join("permissions.json")));
            // Embedding plugins initialize against the installed storage and cookie jar, before any app JS.
            if !devtools_mode {
                tray::install(rt)?;
                embed::setup(rt)?;
            }
            #[cfg(vn_devtools)]
            devtools::install(rt);
            // `__vn_host_*` for `appHost`; absent in the exe.
            embed::install(rt);
            Ok(())
        });
        if let Err(error) = setup_result {
            let summary = format!("host setup failed: {error}");
            eprintln!("[embed] {summary}");
            embed::finished(false, summary);
            cx.quit();
            return;
        }

        let bounds = Bounds::centered(None, size(px(cfg.width), px(cfg.height)), cx);
        let startup_visible = embed::visible();
        let startup_focus = !devtools_mode && !quiet::never_focus() && embed::activates();
        let defer_reveal = cfg!(target_os = "windows") && startup_visible;
        if defer_reveal {
            window::begin_startup_reveal();
        }
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: cfg.backdrop.to_gpui(),
                titlebar: Some(TitlebarOptions {
                    title: Some(cfg.title.clone().into()),
                    // `appears_transparent` is the chrome-mode switch (`!hide_title_bar` adds `WS_CAPTION`),
                    // hence `titleBarStyle` is creation-time only.
                    appears_transparent: cfg.title_bar_style == window::TitleBarStyle::Hidden,
                    traffic_light_position: window::traffic_light_position(
                        cfg.title_bar_style == window::TitleBarStyle::Hidden,
                        cfg.title_bar_height,
                    ),
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
                // gpui maps this to `SW_SHOWNOACTIVATE`: F12 shows the inspector without taking the caret from the app.
                show: startup_visible && !defer_reveal,
                focus: startup_focus,
                // The app draws its caption and edges; the app id is what the compositor and .desktop file match.
                #[cfg(target_os = "linux")]
                window_decorations: Some(gpui::WindowDecorations::Client),
                #[cfg(target_os = "linux")]
                app_id: embed::desktop_id().or_else(|| vn_desktop::launch::identity().map(|i| i.id.clone())),
                ..Default::default()
            },
            |window, cx| {
                if std::env::var_os("VN_RETAINED_LAYOUT").as_deref() != Some(std::ffi::OsStr::new("0")) {
                    window.vn_enable_retained_layout();
                }
                // Before the first frame is presented, so a test host is never seen in front of the person's windows.
                quiet::send_to_back(window);
                // The renderer's device exists now: the first moment `<video>` can decode onto it instead of a private one.
                publish_video_device();
                let shell = cx.new(|_| Shell {
                    status: "booting JS…".into(),
                    log: Vec::new(),
                    title: cfg.title.clone(),
                });
                SHELL.with(|s| *s.borrow_mut() = Some(shell.downgrade()));
                window::init(&cfg, window, cx);
                // The window a forwarded second launch raises; the pipe thread raises it directly since the foreground grant is short-lived.
                if let Some(hwnd) = window::capture_hwnd(window) {
                    vn_desktop::launch::set_window(hwnd);
                }
                // `beforeunload`: a synchronous boolean, like the web event.
                window.on_window_should_close(cx, |window, cx| {
                    let closing = window::should_close();
                    // Accepted: the windows go now; teardown runs behind them under a deadline.
                    if closing {
                        window::cancel_startup_reveal();
                        shutdown::begin("a close request (WM_CLOSE)");
                    }
                    // The primary window owns the JS pump, so a pop-out left open would be a live window
                    // with no clock. Pop-outs are views onto the app, so they close with it.
                    if closing && popout::count() > 0 {
                        vn_tree::log::record(
                            "window",
                            format!(
                                "[window] the primary window is closing — taking {} pop-out(s) \
                                 with it",
                                popout::count()
                            ),
                        );
                        popout::close_all(cx);
                    }
                    // A file dialog owned by this window outlives a close request by its own 100 ms timer.
                    closing && dialogs::may_close_now(window.window_handle(), cx)
                });
                window.on_next_frame(pump_frame);
                shell
            },
        )
        .map(|handle| MAIN_WINDOW.with(|w| w.set(Some(handle.into()))))
        .unwrap();
        // Timers, sockets and devtools keep running while the window is minimised.
        background::start(cx);
        // Results queued for JS on other threads ask the paced clock for a frame.
        #[cfg(target_os = "windows")]
        vn_runtime::frames::set_waker(gpui_windows::wake_frames);

        // A DOM layout read lays out, as in a browser; before any JS, since the bundle's `onMounted` needs it.
        vn_tree::set_sync_layout(sync_layout);
        // `font-family` resolves to the first installed family, so the mapper needs the host's names (read lazily).
        let text_system = cx.text_system().clone();
        vn_css::set_font_source(move || text_system.all_font_names());
        let metric_system = cx.text_system().clone();
        let metric_text = RefCell::new((0usize, gpui::WindowTextSystem::new(metric_system.clone())));
        vn_css::set_font_metric_source(move |font, size| {
            let mut cache = metric_text.borrow_mut();
            let (queries, text) = &mut *cache;
            // This metric-only window is not frame-pumped: bound its line cache.
            if *queries >= 128 {
                *text = gpui::WindowTextSystem::new(metric_system.clone());
                *queries = 0;
            }
            *queries += 1;
            let run = gpui::TextRun { len: 1, font: font.clone(), color: gpui::black(), ..Default::default() };
            let lines = text.shape_text("0".into(), gpui::px(size), &[run], None, None).ok()?;
            let line = lines.first()?;
            let font_id = line.runs().first()?.font_id;
            Some(vn_css::FontRelativeMetrics {
                zero_advance: f32::from(line.size(gpui::px(size)).width),
                x_height: f32::from(text.x_height(font_id, gpui::px(size))),
            })
        });

        // Startup eval: host fns called at top level land in Phase::App.
        #[cfg(vn_devtools)]
        let mut startup_error: Option<String> = None;
        with_published_app(cx, || {
            JsRuntime::with(|rt| {
                if let Err(e) = perf::boot_eval("preludes", || rt.eval(PRELUDE_JS, "vn://prelude.js")) {
                    eprintln!("[js boot] {}\n{}", e.header, e.stack);
                    #[cfg(all(vn_devtools, feature = "native-aot"))]
                    if vn_runtime::redjs::aot::is_active() {
                        startup_error = Some(format!("native inspector prelude failed: {}", e.header));
                        return;
                    }
                }
                if let Err(e) = perf::boot_eval("preludes", || rt.eval(WINDOW_PRELUDE_JS, "vn://window.js")) {
                    eprintln!("[js boot] {}\n{}", e.header, e.stack);
                    #[cfg(all(vn_devtools, feature = "native-aot"))]
                    if vn_runtime::redjs::aot::is_active() {
                        startup_error = Some(format!("native inspector prelude failed: {}", e.header));
                        return;
                    }
                }
                // `VN_SCENARIO=<name>` pins a harness app to one screen at boot; published as a global (sanitised).
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
                #[cfg(vn_devtools)]
                if devtools_mode {
                    if let Err(error) = inspector::boot(rt) {
                        startup_error = Some(error);
                        return;
                    }
                } else {
                    boot_mode::boot(rt);
                }
                #[cfg(not(vn_devtools))]
                boot_mode::boot(rt);
                // `DOMContentLoaded` + `load`, once, after the app bundle has evaluated.
                window::content_loaded();
                commit_boot_tree(rt);
            });
        });
        #[cfg(vn_devtools)]
        if let Some(error) = startup_error {
            let summary = format!("inspector startup failed: {error}");
            eprintln!("[native-aot] {summary}");
            embed::finished(false, summary);
            window::cancel_startup_reveal();
            cx.quit();
            return;
        }
        // The first frame callback waits for layout before running JS; the initial cascade must not wait too.
        refresh_main(cx);
        #[cfg(target_os = "windows")]
        if defer_reveal {
            reveal_startup_window(cx, startup_focus);
        }
        // A hidden window has no frame callback to own the boot wave: start its background clock after bundle evaluation.
        if !embed::visible() {
            background::note_frame();
            reveal_if_unreachable(cx);
        }
        // An inspector never comes to the front on its own.
        if !devtools_mode && !quiet::never_focus() && embed::activates() {
            cx.activate(true);
        }
        #[cfg(vn_devtools)]
        if devtools_mode {
            inspector::watch_parent();
        }
    });
}
