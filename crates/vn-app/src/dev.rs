//! Dev boot + the file-based HMR transport (no HTTP server, no WebSocket — the
//! JS engine has neither, and the host can read files anyway).
//!
//!   dist/vendor.js          IIFE: @vue/runtime-core + renderer + __VN__ loader
//!   dist/modules/<enc>.js   one classic script per app source file, each
//!                           `__VN__.define("<id>", factory)`; <enc> is the id
//!                           with "/" replaced by "__", plus ".js"
//!   dist/manifest.json      { rev, vendorRev, entry, modules: ["<id>", ...] }
//!   dist/changes.json       { rev, vendorRev, changed: [{ id, kind }] }
//!
//! changes.json is written LAST by the watcher and is the only file we poll:
//! once its mtime moves, every module script and the manifest are already on
//! disk. We re-eval each changed module script (which redefines its factory and
//! re-registers its CSS) and then hand the raw changes JSON to
//! `__VN__.hmrApply`, which decides rerender / reload / css-only per entry.
//!
//! Every path here is compiled out under `--features prod`, where the bundle
//! is embedded in the exe and nothing polls the filesystem (see `prod.rs`).

use vn_runtime::JsRuntime;

use crate::json;

// ---------------------------------------------------------------------------
// the dev voice token (docs/VOICE.md §2.6, D-15)
// ---------------------------------------------------------------------------
//
// An API secret in a client binary is the classic LiveKit footgun, so the
// minter lives HERE — in the dev half, compiled out of `--features prod`
// entirely — and never in `vn-voice`. `apps/hello`'s Voice page calls
// `__vn_dev_voice_token(identity, room)`, gets a JWT, and passes it to
// `room.connect()` exactly as a browser app would pass one fetched from its
// backend. In a prod build the host fn does not exist and the app falls back to
// `TokenSource.endpoint`.
//
// The key/secret pair matches `tools/livekit/livekit-dev.yaml`. The secret is
// >= 32 characters because livekit-server refuses to start otherwise ("secret
// is too short"), and it is a well-known dev value on a LOOPBACK-ONLY server —
// there is nothing here to leak.

/// Where the loopback dev server listens. Handed to JS alongside the token so
/// the app never hard-codes a URL.
const DEV_URL: &str = "ws://127.0.0.1:7880";
const DEV_API_KEY: &str = "devkey";
const DEV_API_SECRET: &str = "secret-secret-secret-secret-1234";

// `__vn_dev_voice_token(identity, room)` -> {"serverUrl":…,"participantToken":…}
//
// The shape `TokenSource.endpoint()` resolves to, so an app can swap between
// this and a real backend by changing one line.
vn_runtime::host_fn!(fn vn_dev_voice_token(g, args) {
    use livekit_api::access_token::{AccessToken, VideoGrants};

    let identity = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    let room = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(1).0) }
        .unwrap_or_default();
    if identity.is_empty() || room.is_empty() {
        return vn_runtime::bridge::throw_error(
            g,
            1,
            "__vn_dev_voice_token(identity, room): both arguments are required",
        );
    }

    // `VideoGrants` defaults can_publish / can_subscribe / can_publish_data to
    // TRUE, so naming the room and `room_join` is the whole grant.
    let grants = VideoGrants { room_join: true, room: room.clone(), ..Default::default() };
    let token = AccessToken::with_api_key(DEV_API_KEY, DEV_API_SECRET)
        .with_identity(&identity)
        .with_name(&identity)
        .with_ttl(std::time::Duration::from_secs(3600))
        .with_grants(grants)
        .to_jwt();

    match token {
        Ok(jwt) => {
            let payload = format!(
                "{{\"serverUrl\":{},\"participantToken\":{}}}",
                json::quote(DEV_URL),
                json::quote(&jwt)
            );
            JsRuntime::with(|rt| rt.ctx().string(&payload).0)
        }
        Err(e) => vn_runtime::bridge::throw_error(g, 0, &format!("could not mint a dev token: {e}")),
    }
});

/// Registers the dev-only host fns. Called from `main` next to
/// `devtools::install`, inside the same `#[cfg(not(feature = "prod"))]`.
pub fn install(rt: &JsRuntime) {
    rt.register("__vn_dev_voice_token", 2, vn_dev_voice_token as *const _);
}

const DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/vue-native/dist");

/// How often the frame pump calls [`poll`] — every 30th frame, ~2×/s at 60 fps.
const POLL_EVERY_FRAMES: u32 = 30;

pub(crate) fn dist(rel: &str) -> std::path::PathBuf {
    // VN_DIST points one exe at another app's build output (build.ts already
    // takes [entry] [dist-dir] positionals). This is what lets several dev
    // hosts run side by side without sharing packages/vue-native/dist.
    match std::env::var_os("VN_DIST") {
        Some(dir) if !dir.is_empty() => std::path::PathBuf::from(dir).join(rel),
        _ => std::path::Path::new(DIST).join(rel),
    }
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
/// The base a relative `<img src>` resolves against, derived the only way the
/// dev host can know it: from the manifest's entry module.
///
/// `apps/imgperf/src/main.js` → `apps/imgperf/` — the entry's directory with a
/// trailing `src/` stripped, i.e. where a Vite app would keep `index.html`. A
/// browser resolves a relative URL against the document; this is our document's
/// directory. `VN_APP_ROOT` (read in `main.rs`) overrides and wins, so this
/// never clobbers an explicit choice.
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

/// The `window` block of `apps/<app>/vn.config.json`, which `build.ts` copies
/// verbatim into `dist/manifest.json` (docs/WINDOWING.md §4.1).
///
/// Read BEFORE `cx.open_window`, which is why it is a third function next to
/// [`boot`] and [`poll_frame`] rather than something the boot path hands over:
/// creation-time-only gpui options cannot be applied afterwards without a
/// visible flash.
pub fn window_config() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    Some(v.get("window")?.to_json())
}

pub fn boot(rt: &JsRuntime) {
    let Some(manifest) = read_manifest() else { return };
    publish_asset_base(&manifest.entry);
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
