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

/// Vue's esm-bundler build reads these three flags off globalThis (bun refuses
/// to inline them — must run BEFORE the app bundle, since ESM import hoisting
/// would put the bundle's Vue code first otherwise).
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

// ---------------------------------------------------------------------------
// Dev loop: file-based HMR transport (no HTTP server, no WebSocket — the JS
// engine has neither, and the host can read files anyway).
//
//   dist/vendor.js          IIFE: @vue/runtime-core + renderer + __VN__ loader
//   dist/modules/<enc>.js   one classic script per app source file, each
//                           `__VN__.define("<id>", factory)`; <enc> is the id
//                           with "/" replaced by "__", plus ".js"
//   dist/manifest.json      { rev, vendorRev, entry, modules: ["<id>", ...] }
//   dist/changes.json       { rev, vendorRev, changed: [{ id, kind }] }
//
// changes.json is written LAST by the watcher and is the only file we poll:
// once its mtime moves, every module script and the manifest are already on
// disk. We re-eval each changed module script (which redefines its factory and
// re-registers its CSS) and then hand the raw changes JSON to
// `__VN__.hmrApply`, which decides rerender / reload / css-only per entry.
// ---------------------------------------------------------------------------

mod json;

const DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/vue-native/dist");

fn dist(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(DIST).join(rel)
}

/// `apps/hello/src/App.vue` -> `dist/modules/apps__hello__src__App.vue.js`
fn module_path(id: &str) -> std::path::PathBuf {
    dist("modules").join(format!("{}.js", id.replace('/', "__")))
}

fn mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

struct Manifest {
    vendor_rev: f64,
    entry: String,
    modules: Vec<String>,
}

fn read_manifest() -> Option<Manifest> {
    let path = dist("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| eprintln!("[dev] read {}: {e} — run `bun build.ts` in packages/vue-native", path.display()))
        .ok()?;
    let v = json::parse(&text)?;
    Some(Manifest {
        vendor_rev: v.get("vendorRev").and_then(json::Json::as_f64).unwrap_or(0.0),
        entry: v.get("entry").and_then(json::Json::as_str)?.to_string(),
        modules: v
            .get("modules")
            .and_then(json::Json::as_arr)?
            .iter()
            .filter_map(|m| m.as_str().map(str::to_string))
            .collect(),
    })
}

thread_local! {
    static CHANGES_STAMP: std::cell::Cell<Option<std::time::SystemTime>> =
        const { std::cell::Cell::new(None) };
    static VENDOR_REV: std::cell::Cell<f64> = const { std::cell::Cell::new(-1.0) };
    static FRAME_NO: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn eval_path(rt: &JsRuntime, path: &std::path::Path, url: &str) -> bool {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[dev] read {}: {e}", path.display());
            return false;
        }
    };
    if let Err(e) = rt.eval(&src, url) {
        eprintln!("[dev] {url}: {}\n{}", e.header, e.stack);
        return false;
    }
    true
}

/// Full boot (also used for a cold restart when vendor.js changed): vendor
/// bundle -> every module script (registration only, nothing executes) ->
/// `__VN__.boot(entry)`, which requires the entry and mounts the app.
fn boot_app(rt: &JsRuntime) {
    let Some(manifest) = read_manifest() else { return };
    if !eval_path(rt, &dist("vendor.js"), "vn://vendor.js") {
        return;
    }
    for id in &manifest.modules {
        eval_path(rt, &module_path(id), &format!("vn://{id}"));
    }
    let js = format!("__VN__.boot({});", json::quote(&manifest.entry));
    if let Err(e) = rt.eval(&js, "vn://boot.js") {
        eprintln!("[dev] boot: {}\n{}", e.header, e.stack);
    }
    VENDOR_REV.with(|v| v.set(manifest.vendor_rev));
    CHANGES_STAMP.with(|s| s.set(mtime(&dist("changes.json"))));
}

fn cold_restart(rt: &JsRuntime) {
    eprintln!("[dev] vendor.js changed — cold restart (state is lost)");
    if let Err(e) = rt.eval(
        "globalThis.__VN_TEARDOWN__ && globalThis.__VN_TEARDOWN__();",
        "vn://teardown.js",
    ) {
        eprintln!("[dev] teardown: {}\n{}", e.header, e.stack);
    }
    boot_app(rt);
}

/// Polled from the frame pump. Returns without touching JS unless
/// `dist/changes.json` moved since we last looked.
fn poll_dev_changes() {
    let changes_path = dist("changes.json");
    let current = mtime(&changes_path);
    let moved = CHANGES_STAMP.with(|s| {
        let prev = s.get();
        if prev == current {
            return false;
        }
        s.set(current);
        prev.is_some() && current.is_some()
    });
    if !moved {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&changes_path) else { return };
    let Some(changes) = json::parse(&text) else {
        eprintln!("[dev] changes.json is not valid JSON");
        return;
    };
    let vendor_rev = changes.get("vendorRev").and_then(json::Json::as_f64).unwrap_or(0.0);

    JsRuntime::with(|rt| {
        if VENDOR_REV.with(|v| v.get()) != vendor_rev {
            cold_restart(rt);
            return;
        }
        let ids: Vec<String> = changes
            .get("changed")
            .and_then(json::Json::as_arr)
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.get("id").and_then(json::Json::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        // Redefine first (each script re-registers its factory + CSS), then let
        // the loader decide what to tell __VUE_HMR_RUNTIME__ about each id.
        for id in &ids {
            eval_path(rt, &module_path(id), &format!("vn://{id}"));
        }
        if let Err(e) = rt.eval(&format!("__VN__.hmrApply({text});"), "vn://hmr.js") {
            eprintln!("[dev] hmrApply: {}\n{}", e.header, e.stack);
        }
    });
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
        // Dev loop: poll dist/changes.json (~2×/s at 60 fps) and apply HMR
        // BEFORE the pump, so everything Vue schedules while patching (its
        // post-flush queue, the reload path's queueJob) drains in this frame.
        let n = FRAME_NO.with(|f| {
            let n = f.get().wrapping_add(1);
            f.set(n);
            n
        });
        if n % 30 == 0 {
            poll_dev_changes();
        }
        JsRuntime::with(|rt| {
            rt.pump(PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS);
            for err in rt.drain_errors() {
                eprintln!("[js uncaught] {}\n{}", err.header, err.stack);
            }
        });
    });
    // One notify per JS wave that touched the tree — the draw right after
    // this callback picks it up (same frame).
    if vn_tree::flush_changes() {
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
                boot_app(rt);
            });
        });
        cx.activate(true);
    });
}
