//! Dev boot and the file-based HMR transport (no HTTP server or WebSocket: the JS engine has
//! neither, and the host can read files).
//!
//!   dist/vendor.js          IIFE: @vue/runtime-core + renderer + __VN__ loader
//!   dist/modules/<enc>.js   one classic script per app source file, each
//!                           `__VN__.define("<id>", factory)`; <enc> is the id
//!                           with "/" replaced by "__", plus ".js"
//!   dist/manifest.json      { rev, vendorRev, entry, modules: ["<id>", ...] }
//!   dist/changes.json       { rev, vendorRev, changed: [{ id, kind }] }
//! changes.json is written last by the watcher and is the only file polled: once its mtime moves,
//! every module script and the manifest are on disk. Each changed module script is re-evaluated
//! (redefining its factory and re-registering its CSS), then the raw changes JSON goes to
//! `__VN__.hmrApply`, which decides rerender / reload / css-only per entry.
//!
//! Compiled out under `--features prod`, where the bundle is embedded (see `prod.rs`).

use vn_runtime::JsRuntime;

use crate::json;

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_name_is_the_app_directory_of_the_entry_module() {
        // What names `%LOCALAPPDATA%\<app>\storage` in a dev host. Getting this
        // wrong would let two apps share one localStorage.
        assert_eq!(super::app_dir_of("apps/hello/src/main.js").as_deref(), Some("hello"));
        assert_eq!(super::app_dir_of("apps/shadcn/src/main.ts").as_deref(), Some("shadcn"));
        // No `src/` layer — the entry sits in the app directory itself.
        assert_eq!(super::app_dir_of("apps/imgperf/main.js").as_deref(), Some("imgperf"));
        // Windows separators, because a manifest is written by a builder that
        // ran on Windows.
        assert_eq!(super::app_dir_of("apps\\hello\\src\\main.js").as_deref(), Some("hello"));
        // Nothing to derive: the caller falls through to the executable name.
        assert_eq!(super::app_dir_of("main.js"), None);
        assert_eq!(super::app_dir_of(""), None);
        assert_eq!(super::app_dir_of("src/main.js"), None);
    }
}

/// How often the frame pump calls [`poll`] — every 30th frame, ~2×/s at 60 fps.
const POLL_EVERY_FRAMES: u32 = 30;

/// A file in this host's dist slot. `VN_DIST` points one exe at another app's build output, so
/// several dev hosts can run side by side; the rule itself lives in [`crate::slot`].
pub(crate) fn dist(rel: &str) -> std::path::PathBuf {
    crate::slot::path(rel)
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
    asset_files: std::collections::HashMap<String, std::path::PathBuf>,
}

fn read_manifest() -> Option<Manifest> {
    let path = dist("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| eprintln!("[dev] read {}: {e} — run `bun build.ts` in packages/vue-native", path.display()))
        .ok()?;
    let v = json::parse(&text)?;
    Some(Manifest {
        asset_files: match v.get("assetFiles") {
            Some(json::Json::Obj(files)) => files.iter().filter_map(|(key, value)| {
                Some((key.clone(), std::path::PathBuf::from(value.as_str()?)))
            }).collect(),
            _ => Default::default(),
        },
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
}

/// `label` is what boot-time accounting charges the evaluation to
/// (`crate::perf::boot_eval`); the file read is not part of it.
fn eval_path(rt: &JsRuntime, path: &std::path::Path, url: &str, label: &'static str) -> bool {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[dev] read {}: {e}", path.display());
            return false;
        }
    };
    if let Err(e) = crate::perf::boot_eval(label, || rt.eval(&src, url)) {
        eprintln!("[dev] {url}: {}\n{}", e.header, e.stack);
        return false;
    }
    true
}

/// The base a relative `<img src>` resolves against, which the dev host can only derive from the
/// manifest's entry module: `apps/imgperf/src/main.js` → `apps/imgperf/` (a trailing `src/` is
/// stripped, where a Vite app would keep `index.html`). `VN_APP_ROOT` (read in `lib.rs`) wins.
fn publish_asset_base(entry: &str) {
    if std::env::var_os("VN_APP_ROOT").is_some_and(|v| !v.is_empty()) {
        return;
    }
    const WORKSPACE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let entry_path = std::path::Path::new(WORKSPACE).join(entry);
    let Some(dir) = entry_path.parent() else { return };
    let base = match dir.file_name() {
        Some(name) if name == "src" => dir.parent().unwrap_or(dir),
        _ => dir,
    };
    vn_tree::set_asset_base(base.to_path_buf());
}

/// The `window` block of `apps/<app>/vn.config.json`, which `build.ts` copies verbatim into
/// `dist/manifest.json`. Read before `cx.open_window`, since creation-time gpui options cannot be
/// applied afterwards without a visible flash.
pub fn window_config() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    Some(v.get("window")?.to_json())
}

/// The `launch` block `build.ts` gathers from `vn.config.json`'s top-level `id` / `singleInstance` /
/// `raiseOnSecondInstance` / `protocols`. Read by `main()` before anything else exists.
pub fn launch_config() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    Some(v.get("launch")?.to_json())
}

/// The app's own name, which names its per-app place on disk (`%LOCALAPPDATA%\<app name>\storage`).
///
/// A dev host cannot use its executable's name: one `vn-app.exe` runs every app `VN_DIST` points at,
/// and they must not share a `localStorage`. So it comes from `vn.config.json`'s optional `"name"`,
/// else the app directory of the entry module (as [`publish_asset_base`]). `None` only when there
/// is no manifest, and then vn-storage falls back to the executable name.
pub fn app_name() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    if let Some(n) = v.get("name").and_then(json::Json::as_str).filter(|n| !n.is_empty()) {
        return Some(n.to_string());
    }
    app_dir_of(v.get("entry").and_then(json::Json::as_str)?)
}

/// `apps/hello/src/main.js` → `hello`. Split out so the derivation is testable
/// without a manifest on disk.
fn app_dir_of(entry: &str) -> Option<String> {
    let mut segs: Vec<&str> = entry.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    segs.pop()?; // the entry file itself
    if segs.last() == Some(&"src") {
        segs.pop();
    }
    segs.pop().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Serves `fetch("assets/x.wav")` out of the app directory in a dev build, so it resolves against
/// the same base as `<img src>` (`vn_tree`'s asset base) rather than the process working directory.
///
/// Same seam as `prod.rs`, with the same key (`vn_payload::content_key`: percent-decoded, `.`/`..`
/// folded, never above the root): a URL that resolves here resolves to the same file once packaged,
/// and one that cannot be packaged cannot be read here either.
///
/// A miss is a 404, with one advisory when a file of that name exists relative to the cwd (a
/// packaged exe has no meaningful cwd; vn-runtime `NO_DISK_ROOT`).
fn install_asset_provider(rt: &JsRuntime) {
    // `boot` runs again on a cold restart, and the provider is process-wide and
    // set-once; re-registering would only print an advisory.
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(base) = vn_tree::asset_base() else { return };
    ONCE.call_once(move || {
        rt.set_content_provider(move |raw| {
            let key = vn_payload::content_key(raw)?;
            let path = vn_tree::dev_asset_file(raw)
                .unwrap_or_else(|| base.join(key.replace('/', std::path::MAIN_SEPARATOR_STR)));
            match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(_) => {
                    if std::path::Path::new(raw).is_file() {
                        cwd_advisory(raw);
                    }
                    None
                }
            }
        });
    });
}

/// One line per key, bounded: the miss above was a file relative to the cwd.
fn cwd_advisory(raw: &str) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    let seen = seen.get_or_insert_with(HashSet::new);
    if seen.len() >= 32 || !seen.insert(raw.to_string()) {
        return;
    }
    eprintln!(
        "[vue-native compat] fetch({raw:?}) is a 404: it is not under the app root. A file of that name \
         exists relative to the working directory, which is not a fallback: a packaged exe has none, so \
         neither does dev. Move it under the app (and into vn.config.json \"assets\") to have it served."
    );
}

/// Full boot (also used for a cold restart when vendor.js changed): vendor bundle -> every module
/// script (registration only, nothing executes) -> `__VN__.boot(entry)`, which requires the entry
/// and mounts the app.
pub fn boot(rt: &JsRuntime) {
    let Some(manifest) = read_manifest() else { return };
    publish_asset_base(&manifest.entry);
    vn_tree::set_dev_asset_files(manifest.asset_files);
    // After the base exists, before any app code can `fetch()`.
    install_asset_provider(rt);
    if !eval_path(rt, &dist("vendor.js"), "vn://vendor.js", "vendor.js") {
        return;
    }
    for id in &manifest.modules {
        eval_path(rt, &module_path(id), &format!("vn://{id}"), "modules");
    }
    let js = format!("__VN__.boot({});", json::quote(&manifest.entry));
    if let Err(e) = crate::perf::boot_eval("boot.js (mount)", || rt.eval(&js, "vn://boot.js")) {
        eprintln!("[dev] boot: {}\n{}", e.header, e.stack);
    }
    VENDOR_REV.with(|v| v.set(manifest.vendor_rev));
    CHANGES_STAMP.with(|s| s.set(mtime(&dist("changes.json"))));
}

/// `location.reload()`'s second half (src/reload.rs): the app is already torn
/// down; evaluate `dist/` again, exactly as a cold restart does.
pub fn reboot(rt: &JsRuntime) {
    boot(rt);
}

fn cold_restart(rt: &JsRuntime) {
    eprintln!("[dev] vendor.js changed — cold restart (state is lost)");
    if let Err(e) = rt.eval(
        "globalThis.__VN_TEARDOWN__ && globalThis.__VN_TEARDOWN__();",
        "vn://teardown.js",
    ) {
        eprintln!("[dev] teardown: {}\n{}", e.header, e.stack);
    }
    // A new vendor.js is a new runtime that can never reach the old page's
    // captures and AudioContexts: end them, as a reload does.
    crate::reload::end_media(rt, "cold restart");
    crate::js::reload(rt.ctx());
    // The Network panel's divider between the two runs (devtools/net.rs).
    #[cfg(vn_devtools)]
    crate::devtools::mark_reload("cold restart: vendor.js changed");
    boot(rt);
}

/// Called from the frame pump on every frame; cheap no-op except every
/// [`POLL_EVERY_FRAMES`]th, and even then it returns without touching JS unless
/// `dist/changes.json` moved since we last looked.
pub fn poll_frame(frame_no: u32) {
    if frame_no % POLL_EVERY_FRAMES != 0 {
        return;
    }
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

    // Publish newly imported files before updated modules can request them.
    if let Some(manifest) = read_manifest() { vn_tree::set_dev_asset_files(manifest.asset_files); }
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
            // After the first frame, so boot accounting has closed and this
            // label is never charged.
            eval_path(rt, &module_path(id), &format!("vn://{id}"), "hmr");
        }
        if let Err(e) = rt.eval(&format!("__VN__.hmrApply({text});"), "vn://hmr.js") {
            eprintln!("[dev] hmrApply: {}\n{}", e.header, e.stack);
        }
    });
}
