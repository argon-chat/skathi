//! vue-native host shell, milestone 1: JavaScript (JSC via redjs) alive inside
//! a Mica gpui window. The JS below drives the UI through two `host_fn!`s;
//! the pump runs as a self-rescheduling `on_next_frame` callback — before the
//! frame's draw, so a JS mutation lands in the same frame.

use std::cell::RefCell;

use gpui::{
    App, Bounds, Context, TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowOptions, div, prelude::*, px, rgba, size,
};
use gpui_platform::application;
use vn_runtime::{JsRuntime, Phase, host_fn, with_app, with_published, with_published_app};
use vn_runtime::runtime::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

const DEMO_JS: &str = r#"
    __vn_log(`typeof fetch=${typeof fetch}, EventTarget=${typeof EventTarget}, ` +
             `structuredClone=${typeof structuredClone}, URL=${typeof URL}`);
    Promise.resolve("microtask drained on the frame pump").then(__vn_log);
    requestAnimationFrame(t => __vn_log(`first rAF at t=${t.toFixed(1)}ms`));
    setTimeout(async () => {
        const v = await new Promise(r => setTimeout(() => r(42), 300));
        __vn_log(`setTimeout -> await chain resolved: ${v}`);
    }, 100);
    let ticks = 0;
    setInterval(() => {
        ticks += 1;
        __vn_set_status(`JS is alive — tick ${ticks}, uptime ${(performance.now() / 1000).toFixed(1)}s`);
    }, 500);
"#;

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
        JsRuntime::with(|rt| {
            rt.pump(PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS);
            for err in rt.drain_errors() {
                eprintln!("[js uncaught] {}\n{}", err.header, err.stack);
            }
        });
    });
    window.on_next_frame(pump_frame);
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
                div().flex_1().p(px(24.)).flex().flex_col().gap(px(16.)).child(
                    div()
                        .p(px(16.))
                        .bg(rgba(0xffffff14))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgba(0xffffff2e))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(div().text_size(px(15.)).child(self.status.clone()))
                        .children(self.log.iter().map(|line| {
                            div()
                                .text_size(px(12.))
                                .text_color(rgba(0xffffffb0))
                                .child(line.clone())
                        })),
                ),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        JsRuntime::init();
        JsRuntime::with(|rt| {
            rt.register("__vn_set_status", 1, vn_set_status as *const _);
            rt.register("__vn_log", 1, vn_log as *const _);
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
                if let Err(e) = rt.eval(DEMO_JS, "vn://demo.js") {
                    eprintln!("[js boot] {}\n{}", e.header, e.stack);
                }
            });
        });
        cx.activate(true);
    });
}
