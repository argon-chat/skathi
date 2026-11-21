//! Dev boot and the file-based HMR transport (no HTTP server or WebSocket: the JS engine has
//! neither, and the host can read files).
//!
//! changes.json is written last by the watcher and is the only file polled: once its mtime moves,
//! every module script and the manifest are on disk. Each changed module script is re-evaluated
//! (redefining its factory and re-registering its CSS), then the raw changes JSON goes to
//! `__SK__.hmrApply`, which decides rerender / reload / css-only per entry.
//!
//! `sk dev` writes [`EFFECTS_CHANGED`] after it recompiled the app's shaders; the new `--shading`
//! effects replace the running ones (`gpui::reload_app_effects`) and every window restyles and redraws.
//!
//! Compiled out under `--features prod`, where the bundle is embedded (see `prod.rs`).

use sk_runtime::JsRuntime;

use crate::json;

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_name_is_the_app_directory_of_the_entry_module() {
        // What names `%LOCALAPPDATA%\<app>\storage` in a dev host. Getting this
        // wrong would let two apps share one localStorage.
        assert_eq!(
            super::app_dir_of("apps/hello/src/main.js").as_deref(),
            Some("hello")
        );
        assert_eq!(
            super::app_dir_of("apps/shadcn/src/main.ts").as_deref(),
            Some("shadcn")
        );
        assert_eq!(
            super::app_dir_of("apps/imgperf/main.js").as_deref(),
            Some("imgperf")
        );
        assert_eq!(
            super::app_dir_of("apps\\hello\\src\\main.js").as_deref(),
            Some("hello")
        );
        assert_eq!(super::app_dir_of("main.js"), None);
        assert_eq!(super::app_dir_of(""), None);
        assert_eq!(super::app_dir_of("src/main.js"), None);
    }
}

const POLL_EVERY_FRAMES: u32 = 30;

/// `{"dir": "<compiled effects>"}`, written by `sk dev` after the effect files.
const EFFECTS_CHANGED: &str = "effects-changed.json";

pub(crate) fn dist(rel: &str) -> std::path::PathBuf {
    crate::slot::path(rel)
}

fn module_path(id: &str) -> std::path::PathBuf {
    dist("modules").join(format!("{}.js", id.replace('/', "__")))
}

fn mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

struct Manifest {
    vendor_rev: f64,
    entry: String,
    app_root: Option<std::path::PathBuf>,
    modules: Vec<String>,
    asset_files: std::collections::HashMap<String, std::path::PathBuf>,
}

fn read_manifest() -> Option<Manifest> {
    let path = dist("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| {
            eprintln!(
                "[dev] read {}: {e} — run `bun build.ts` in packages/cli",
                path.display()
            )
        })
        .ok()?;
    let v = json::parse(&text)?;
    Some(Manifest {
        asset_files: match v.get("assetFiles") {
            Some(json::Json::Obj(files)) => files
                .iter()
                .filter_map(|(key, value)| {
                    Some((key.clone(), std::path::PathBuf::from(value.as_str()?)))
                })
                .collect(),
            _ => Default::default(),
        },
        vendor_rev: v
            .get("vendorRev")
            .and_then(json::Json::as_f64)
            .unwrap_or(0.0),
        entry: v.get("entry").and_then(json::Json::as_str)?.to_string(),
        app_root: v
            .get("appRoot")
            .and_then(json::Json::as_str)
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute()),
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
    static EFFECTS_STAMP: std::cell::Cell<Option<std::time::SystemTime>> =
        const { std::cell::Cell::new(None) };
}

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

/// The base a relative `<img src>` resolves against: the app root the builder recorded, since one
/// dev host runs any app's dist. `SK_APP_ROOT` (read in `lib.rs`) wins.
fn publish_asset_base(app_root: Option<std::path::PathBuf>) {
    if std::env::var_os("SK_APP_ROOT").is_some_and(|v| !v.is_empty()) {
        return;
    }
    match app_root {
        Some(dir) => sk_tree::set_asset_base(dir),
        None => eprintln!(
            "[dev] the dev build names no app root, so the app's own files cannot be served: \
             rebuild it with the current builder, or set SK_APP_ROOT to the app's directory"
        ),
    }
}

/// The `window` block of `apps/<app>/sk.config.json`, which `build.ts` copies verbatim into
/// `dist/manifest.json`. Read before `cx.open_window`, since creation-time gpui options cannot be
/// applied afterwards without a visible flash.
pub fn window_config() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    Some(v.get("window")?.to_json())
}

#[cfg(sk_devtools)]
pub fn devtools_published() -> bool {
    true
}

pub fn launch_config() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    Some(v.get("launch")?.to_json())
}

/// A dev host cannot use its executable's name: one `sk-app.exe` runs every app `SK_DIST` points at,
/// and they must not share a `localStorage`.
pub fn app_name() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    if let Some(n) = v
        .get("name")
        .and_then(json::Json::as_str)
        .filter(|n| !n.is_empty())
    {
        return Some(n.to_string());
    }
    app_dir_of(v.get("entry").and_then(json::Json::as_str)?)
}

pub fn app_version() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let version = json::parse(&text)?.get("version")?.as_str()?.to_string();
    (!version.is_empty()).then_some(version)
}

fn app_dir_of(entry: &str) -> Option<String> {
    let mut segs: Vec<&str> = entry.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    segs.pop()?; // the entry file itself
    if segs.last() == Some(&"src") {
        segs.pop();
    }
    segs.pop().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Same seam as `prod.rs`, with the same key (`sk_payload::content_key`: percent-decoded, `.`/`..`
/// folded, never above the root): a URL that resolves here resolves to the same file once packaged,
/// and one that cannot be packaged cannot be read here either.
///
/// A miss is a 404, with one advisory when a file of that name exists relative to the cwd (a
/// packaged exe has no meaningful cwd; sk-runtime `NO_DISK_ROOT`).
fn install_asset_provider(rt: &JsRuntime) {
    // `boot` runs again on a cold restart, and the provider is process-wide and
    // set-once; re-registering would only print an advisory.
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(base) = sk_tree::asset_base() else {
        return;
    };
    ONCE.call_once(move || {
        rt.set_content_provider(move |raw| {
            let key = sk_payload::content_key(raw)?;
            let path = sk_tree::dev_asset_file(raw)
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
        "[skathi compat] fetch({raw:?}) is a 404: it is not under the app root. A file of that name \
         exists relative to the working directory, which is not a fallback: a packaged exe has none, so \
         neither does dev. Move it under the app (and into sk.config.json \"assets\") to have it served."
    );
}

pub fn boot(rt: &JsRuntime) {
    let Some(manifest) = read_manifest() else {
        return;
    };
    publish_asset_base(manifest.app_root.clone());
    sk_tree::set_dev_asset_files(manifest.asset_files);
    install_asset_provider(rt);
    if !eval_path(rt, &dist("vendor.js"), "sk://vendor.js", "vendor.js") {
        return;
    }
    for id in &manifest.modules {
        eval_path(rt, &module_path(id), &format!("sk://{id}"), "modules");
    }
    let js = format!("__SK__.boot({});", json::quote(&manifest.entry));
    if let Err(e) = crate::perf::boot_eval("boot.js (mount)", || rt.eval(&js, "sk://boot.js")) {
        eprintln!("[dev] boot: {}\n{}", e.header, e.stack);
    }
    VENDOR_REV.with(|v| v.set(manifest.vendor_rev));
    CHANGES_STAMP.with(|s| s.set(mtime(&dist("changes.json"))));
    EFFECTS_STAMP.with(|s| s.set(mtime(&dist(EFFECTS_CHANGED))));
}

pub fn reboot(rt: &JsRuntime) {
    boot(rt);
}

fn cold_restart(rt: &JsRuntime) {
    eprintln!("[dev] vendor.js changed — cold restart (state is lost)");
    if let Err(e) = rt.eval(
        "globalThis.__SK_TEARDOWN__ && globalThis.__SK_TEARDOWN__();",
        "sk://teardown.js",
    ) {
        eprintln!("[dev] teardown: {}\n{}", e.header, e.stack);
    }
    // A new vendor.js is a new runtime that can never reach the old page's
    // captures and AudioContexts: end them, as a reload does.
    crate::reload::end_media(rt, "cold restart");
    crate::js::reload(rt.ctx());
    #[cfg(sk_devtools)]
    crate::devtools::mark_reload("cold restart: vendor.js changed");
    boot(rt);
}

/// Unlike `changes.json`, the message may first appear after boot: an app's first rebuild writes it.
fn poll_effects() {
    let path = dist(EFFECTS_CHANGED);
    let current = mtime(&path);
    if current.is_none() || EFFECTS_STAMP.with(|s| s.replace(current)) == current {
        return;
    }
    let dir = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| json::parse(&text))
        .and_then(|message| {
            message
                .get("dir")
                .and_then(json::Json::as_str)
                .map(str::to_string)
        });
    let Some(dir) = dir else {
        eprintln!(
            "[dev] {} names no effects directory; the app keeps its `--shading` effects",
            path.display()
        );
        return;
    };
    match gpui::AppEffects::from_dir(std::path::Path::new(&dir)) {
        Ok(effects) => {
            gpui::reload_app_effects(effects);
            sk_tree::effects_changed();
        }
        Err(error) => {
            eprintln!("[dev] {error} — the app keeps its previous `--shading` effects")
        }
    }
}

pub fn poll_frame(frame_no: u32) {
    if frame_no % POLL_EVERY_FRAMES != 0 {
        return;
    }
    poll_effects();
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
    let Ok(text) = std::fs::read_to_string(&changes_path) else {
        return;
    };
    let Some(changes) = json::parse(&text) else {
        eprintln!("[dev] changes.json is not valid JSON");
        return;
    };
    let vendor_rev = changes
        .get("vendorRev")
        .and_then(json::Json::as_f64)
        .unwrap_or(0.0);

    if let Some(manifest) = read_manifest() {
        sk_tree::set_dev_asset_files(manifest.asset_files);
    }
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
        for id in &ids {
            // After the first frame, so boot accounting has closed and this
            // label is never charged.
            eval_path(rt, &module_path(id), &format!("sk://{id}"), "hmr");
        }
        if let Err(e) = rt.eval(&format!("__SK__.hmrApply({text});"), "sk://hmr.js") {
            eprintln!("[dev] hmrApply: {}\n{}", e.header, e.stack);
        }
    });
}
