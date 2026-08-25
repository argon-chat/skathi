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
    App, Bounds, Context, TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowOptions, div, prelude::*, px, rgba, size,
};
use gpui_platform::application;
use vn_runtime::{JsRuntime, Phase, host_fn, with_app, with_published, with_published_app};
use vn_runtime::runtime::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

#[cfg(not(feature = "prod"))]
mod dev;
#[cfg(not(feature = "prod"))]
mod json; // only the dev transport parses JSON
#[cfg(feature = "prod")]
mod prod;

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
    let ticks = 0;
    setInterval(() => {
        ticks += 1;
        __vn_set_status(`runtime: tick ${ticks}, uptime ${(performance.now() / 1000).toFixed(1)}s`);
    }, 500);
"#;

thread_local! {
    static FRAME_NO: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
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
    update_shell(g, move |shell| shell.status = text)
});

host_fn!(fn vn_log(g, args) {
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
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
    if vn_runtime::bridge::poisoned() {
        return; // I9: no JS after a caught native panic.
    }
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
        JsRuntime::with(|rt| {
            rt.pump(PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS);
            for err in rt.drain_errors() {
                eprintln!("[js uncaught] {}\n{}", err.header, err.stack);
            }
        });
    });
    // One notify per JS wave that touched the tree — the draw right after
    // this callback picks it up (same frame). While CSS transitions run, keep
    // repainting: `on_next_frame` alone re-arms but does not redraw.
    vn_tree::set_reduce_motion(cx.reduce_motion());
    if vn_tree::flush_changes() || vn_tree::animating() {
        SHELL.with(|s| {
            if let Some(shell) = s.borrow().as_ref().and_then(|w| w.upgrade()) {
                shell.update(cx, |_, cx| cx.notify());
            }
        });
    }
    window.on_next_frame(pump_frame);
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(gpui::white())
            .font_family(".SystemUIFont")
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
                    .child(vn_tree::build_root(window, cx))
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

fn main() {
    application().run(|cx: &mut App| {
        vn_elements::init(cx); // key bindings for the native <input> widget
        JsRuntime::init();
        JsRuntime::with(|rt| {
            rt.register("__vn_set_status", 1, vn_set_status as *const _);
            rt.register("__vn_log", 1, vn_log as *const _);
            vn_tree::install(rt);
        });

        let bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                titlebar: Some(TitlebarOptions {
                    title: Some("vue-native".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
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
                boot_mode::boot(rt);
            });
        });
        cx.activate(true);
    });
}
