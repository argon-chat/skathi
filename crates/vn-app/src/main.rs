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
//! - **prod** (`--features prod`) — [`prod`]: `include_str!`s the single IIFE
//!   from `bun prod.ts` plus the app's assets, so the exe runs standalone with
//!   no dist/ folder, no watcher and no filesystem reads at all.
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
#[cfg(not(feature = "prod"))]
mod devtools; // the automation surface (docs/DEVTOOLS.md) — dev builds only
mod json; // the dev transport, the devtools wire and vn.config.json share one codec
mod perf;
#[cfg(feature = "prod")]
mod prod;
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
const PRELUDE_JS: &str = r#"
    globalThis.__VUE_OPTIONS_API__ = true;
    globalThis.__VUE_PROD_DEVTOOLS__ = false;
    globalThis.__VUE_PROD_HYDRATION_MISMATCH_DETAILS__ = false;
    globalThis.__VN_CSS__ = [];
    // Browser-compat: redjs ships no `history`; vue-router's dev build touches
    // it at startup even with memory history (its isBrowser heuristic sees our
    // window/document). Inert stub, like `location`. TODO: move into redjs
    // webapi as a proper native class.
    globalThis.history = {
        state: null, length: 1, scrollRestoration: "auto",
        pushState() {}, replaceState() {}, go() {}, back() {}, forward() {},
    };
    // (The milestone-1 500ms "tick" status heartbeat used to live here. Every
    // update was a shell notify, i.e. a FULL element-tree rebuild — 40-100ms
    // twice a second on a component-heavy app. Status now only moves on real
    // events.)
"#;

/// The windowing surface's JS half (docs/WINDOWING.md tiers 1 & 2) — its own
/// file because it is real JavaScript, not a four-line stub. Evaluated right
/// after [`PRELUDE_JS`] and before the app bundle, so `document.title`,
/// `screen`, `matchMedia` and `navigator.windowControlsOverlay` are already
/// their real selves when the first component's `<script setup>` runs.
const WINDOW_PRELUDE_JS: &str = include_str!("window.prelude.js");

thread_local! {
    static FRAME_NO: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
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
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{} {}] {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }
    fn flush(&self) {}
}

fn install_logger() {
    let Ok(level) = std::env::var("VN_LOG") else { return };
    let filter = match level.trim().to_ascii_lowercase().as_str() {
        "" | "info" | "1" | "true" => log::LevelFilter::Info,
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        "off" => return,
        other => {
            eprintln!("[vn] VN_LOG={other:?} is not a level — using info");
            log::LevelFilter::Info
        }
    };
    static LOGGER: StderrLogger = StderrLogger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(filter);
    }
}

struct Shell {
    status: String,
    log: Vec<String>,
}

thread_local! {
    static SHELL: RefCell<Option<WeakEntity<Shell>>> = const { RefCell::new(None) };
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
    if vn_runtime::bridge::poisoned() {
        return; // I9: no JS after a caught native panic.
    }
    // The frame's ONE coalesced mousemove (browsers align move events to the
    // frame clock; see PendingMouseMove in vn-tree). Dispatched before the
    // pump block so whatever the handler schedules drains this same frame.
    vn_tree::flush_pending_mouse_move(window, cx);
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
        #[cfg(not(feature = "prod"))]
        devtools::pump();
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
    // `<lottie>` transport is advanced by this frame's wall-clock delta (and
    // its `complete`/`loopComplete`/`enterFrame` events dispatched), and a node
    // that was focused when it got detached loses gpui focus. Before the draw,
    // so the raster a `<lottie>` paints is the one this tick moved to.
    let lottie_moved = vn_tree::frame_tick(window, cx);
    // One notify per JS wave that touched the tree — the draw right after
    // this callback picks it up (same frame). While CSS transitions run or a
    // Lottie plays, keep repainting: `on_next_frame` alone re-arms but does not
    // redraw.
    if vn_tree::flush_changes() || vn_tree::animating() || lottie_moved || vn_tree::lotties_animating()
    {
        SHELL.with(|s| {
            if let Some(shell) = s.borrow().as_ref().and_then(|w| w.upgrade()) {
                shell.update(cx, |_, cx| cx.notify());
            }
        });
    }
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
            .flex()
            .flex_col()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont");
        if !window::fallback_titlebar() {
            // Custom chrome, mode 1: the app owns the WHOLE window — its own
            // titlebar included. No host padding, no host status panel; the
            // shell is a bare mount point.
            return shell.child(vn_tree::build_root(window, cx));
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
                    .child("vue-native — gpui + JSC + Vue"),
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
    }
}

/// The directory a relative `<img src>` resolves against — vue-native's
/// stand-in for the document URL a browser resolves against.
///
/// `VN_APP_ROOT` wins when set; otherwise the dev transport derives it from the
/// manifest entry (`apps/<app>/src/main.js` → `apps/<app>/`) in [`dev::boot`],
/// and the prod build — which reads no files at all — leaves it unset, so
/// relative paths fall back to the working directory with an advisory.
fn publish_app_root() {
    if let Some(dir) = std::env::var_os("VN_APP_ROOT") {
        if !dir.is_empty() {
            vn_tree::set_asset_base(std::path::PathBuf::from(dir));
        }
    }
}

fn main() {
    // Before anything gpui does, so the renderer's boot messages are visible.
    install_logger();
    perf::boot();
    application().run(|cx: &mut App| {
        vn_elements::init(cx); // key bindings for the native <input> widget
        publish_app_root();
        // Before the runtime exists, so the log ring is already recording when
        // the first advisory fires.
        #[cfg(not(feature = "prod"))]
        devtools::start();
        JsRuntime::init();
        JsRuntime::with(|rt| {
            rt.register("__vn_set_status", 1, vn_set_status as *const _);
            rt.register("__vn_log", 1, vn_log as *const _);
            rt.register("__vn_perf_mark", 1, vn_perf_mark as *const _);
            vn_tree::install(rt);
            window::install(rt);
            #[cfg(not(feature = "prod"))]
            devtools::install(rt);
        });

        // The window block of `apps/<app>/vn.config.json`, read BEFORE
        // `open_window` because that is the only tier that can set the
        // creation-time-only gpui options (`window_min_size`, `is_resizable`,
        // `is_minimizable`, `titlebar`, `window_background`) with no visible
        // flash. Environment still wins on top (`VN_WINDOW`, `VN_WINDOW_FOCUS`)
        // — the perf and screenshot harnesses depend on it (D-21).
        let cfg = window::load_config(boot_mode::window_config());
        let bounds = Bounds::centered(None, size(px(cfg.width), px(cfg.height)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: cfg.backdrop.to_gpui(),
                titlebar: Some(TitlebarOptions {
                    title: Some(cfg.title.clone().into()),
                    // `titleBarStyle: "hidden"` — the v1 default and the only
                    // mode gpui can actually produce today (getting real OS
                    // chrome is fork item F-03; asking for it advises A-17 and
                    // lands here).
                    appears_transparent: true,
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
                ..Default::default()
            },
            |window, cx| {
                let shell = cx.new(|_| Shell {
                    status: "booting JS…".into(),
                    log: Vec::new(),
                });
                SHELL.with(|s| *s.borrow_mut() = Some(shell.downgrade()));
                window::init(&cfg, window, cx);
                // `beforeunload`: `on_window_should_close` is a SYNCHRONOUS
                // boolean, which is precisely the web event's shape.
                window.on_window_should_close(cx, |_, _| window::should_close());
                window.on_next_frame(pump_frame);
                shell
            },
        )
        .unwrap();

        // Startup eval: host fns called at top level land in Phase::App.
        with_published_app(cx, || {
            JsRuntime::with(|rt| {
                if let Err(e) = rt.eval(PRELUDE_JS, "vn://prelude.js") {
                    eprintln!("[js boot] {}\n{}", e.header, e.stack);
                }
                if let Err(e) = rt.eval(WINDOW_PRELUDE_JS, "vn://window.js") {
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
                boot_mode::boot(rt);
                // `DOMContentLoaded` + `load`, once, after the app bundle has
                // evaluated — several reused browser components wait on these,
                // and redjs has exported the hook since day one with nothing
                // ever calling it.
                window::content_loaded();
            });
        });
        cx.activate(true);
    });
}
