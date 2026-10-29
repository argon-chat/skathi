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

// ---------------------------------------------------------------------------
// pointing the dev host at a server that is not the loopback one
// ---------------------------------------------------------------------------
//
// Two environment overrides, both generic and neither carrying a credential:
//
//   VN_VOICE_URL              the server to hand JS, instead of DEV_URL
//   VN_VOICE_TOKEN_<IDENTITY> a ready-made JWT for that identity
//   VN_VOICE_TOKEN            a ready-made JWT for any identity (fallback)
//
// `<IDENTITY>` is the `identity` argument upper-cased with every character
// outside `[A-Z0-9]` folded to `_`, so the two-window rig's `alice`/`bob` read
// `VN_VOICE_TOKEN_ALICE` / `VN_VOICE_TOKEN_BOB`, and slots named `a`/`b` read
// `VN_VOICE_TOKEN_A` / `VN_VOICE_TOKEN_B`. Per-identity is the only shape that
// generalises: a token names the participant it was minted for, so N
// participants need N tokens.
//
// **A supplied token is used verbatim and nothing is minted** — which is the
// point. An API key/secret for somebody's real server must never have to be
// handed to this process (or written into this file) merely to point a dev
// build at it; a token minted elsewhere, out of band, is enough. `VN_VOICE_URL`
// on its own still mints with the loopback dev key, which is what you want when
// the thing that moved is the address of *our* dev SFU.
//
// This is not staging-specific and not test-specific: it is the one seam that
// makes "which server does the Voice page join?" answerable without a rebuild.

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// `alice` -> `VN_VOICE_TOKEN_ALICE`
fn token_var_for(identity: &str) -> String {
    let mut name = String::from("VN_VOICE_TOKEN_");
    for c in identity.chars() {
        name.push(match c.to_ascii_uppercase() {
            c @ ('A'..='Z' | '0'..='9') => c,
            _ => '_',
        });
    }
    name
}

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

    let url = env_nonempty("VN_VOICE_URL").unwrap_or_else(|| DEV_URL.to_string());

    // An override is used AS IS: no minting, so no key or secret is needed for
    // a server whose credentials we do not have.
    let supplied =
        env_nonempty(&token_var_for(&identity)).or_else(|| env_nonempty("VN_VOICE_TOKEN"));

    let token = match supplied {
        Some(jwt) => Ok(jwt),
        None => {
            // `VideoGrants` defaults can_publish / can_subscribe /
            // can_publish_data to TRUE, so naming the room and `room_join` is
            // the whole grant.
            let grants = VideoGrants { room_join: true, room: room.clone(), ..Default::default() };
            AccessToken::with_api_key(DEV_API_KEY, DEV_API_SECRET)
                .with_identity(&identity)
                .with_name(&identity)
                .with_ttl(std::time::Duration::from_secs(3600))
                .with_grants(grants)
                .to_jwt()
        }
    };

    match token {
        Ok(jwt) => {
            let payload = format!(
                "{{\"serverUrl\":{},\"participantToken\":{}}}",
                json::quote(&url),
                json::quote(&jwt)
            );
            JsRuntime::with(|rt| rt.ctx().string(&payload).0)
        }
        Err(e) => vn_runtime::bridge::throw_error(g, 0, &format!("could not mint a dev token: {e}")),
    }
});

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

    #[test]
    fn token_var_names_are_derived_from_the_identity() {
        assert_eq!(super::token_var_for("alice"), "VN_VOICE_TOKEN_ALICE");
        assert_eq!(super::token_var_for("a"), "VN_VOICE_TOKEN_A");
        // Anything an identity may legally contain that an env var may not.
        assert_eq!(super::token_var_for("user-1.x"), "VN_VOICE_TOKEN_USER_1_X");
    }
}

/// Registers the dev-only host fns. Called from `main` next to
/// `devtools::install`, inside the same `#[cfg(not(feature = "prod"))]`.
pub fn install(rt: &JsRuntime) {
    rt.register("__vn_dev_voice_token", 2, vn_dev_voice_token as *const _);
}

/// How often the frame pump calls [`poll`] — every 30th frame, ~2×/s at 60 fps.
const POLL_EVERY_FRAMES: u32 = 30;

/// A file in this host's dist slot.
///
/// `VN_DIST` points one exe at another app's build output (build.ts already
/// takes [entry] [dist-dir] positionals), which is what lets several dev hosts
/// run side by side without sharing packages/vue-native/dist. The rule moved to
/// [`crate::slot`] when a prod build gained a slot of its own with no dist in
/// it; this is the dev half's spelling of the same question.
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

/// The `launch` block `build.ts` gathers from `vn.config.json`'s top-level
/// `id` / `singleInstance` / `raiseOnSecondInstance` / `protocols`
/// (docs/LAUNCH.md). Read by `main()` before anything else exists — the
/// single-instance check is the first thing a process does — which is why it
/// is a function here beside [`window_config`] rather than part of [`boot`].
pub fn launch_config() -> Option<String> {
    let text = std::fs::read_to_string(dist("manifest.json")).ok()?;
    let v = json::parse(&text)?;
    Some(v.get("launch")?.to_json())
}

/// The manifest's `argonSecure`, copied by `build.ts` from `vn.config.json`
/// (src/temp_argon_secure.rs). TEMPORARY.
pub fn argon_secure() -> bool {
    let Ok(text) = std::fs::read_to_string(dist("manifest.json")) else { return false };
    json::parse(&text).and_then(|v| v.get("argonSecure").and_then(json::Json::as_bool)).unwrap_or(false)
}

/// The app's own name — what a per-app place on disk is named after
/// (`%LOCALAPPDATA%\<app name>\storage`, see crates/vn-storage).
///
/// A DEV host cannot use its executable's name for this: one `vn-app.exe` runs
/// `apps/hello`, `apps/shadcn` and whatever else `VN_DIST` points at, and they
/// must not share a `localStorage`. So it comes from the build:
/// `apps/<app>/vn.config.json`'s optional `"name"`, and failing that the app
/// directory the entry module sits in — `apps/hello/src/main.js` → `hello`,
/// the same derivation [`publish_asset_base`] uses for the asset base, for the
/// same reason (it is the directory a Vite app would keep `index.html` in).
///
/// `None` only when there is no manifest at all, and then vn-storage falls
/// through to the executable name.
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

/// Serves `fetch("assets/x.wav")` out of the app directory in a dev build.
///
/// A browser resolves `fetch()` and `<img src>` against the SAME document URL.
/// Here they were resolved against two different things: `<img>`/`<video>` go
/// through `vn_tree`'s asset base (the app directory), while `fetch()` fell
/// through redjs to the process working directory, because the dev host never
/// installed a content provider and passed empty roots to `redjs_webshim_install`.
/// So `<img src="assets/a.png">` worked and `fetch("assets/a.png")` 404'd from
/// the same component — a divergence with no upside.
///
/// This is the same seam `prod.rs` uses to serve a packaged exe's payload, and
/// since m25 it uses the same KEY: `vn_payload::content_key` (percent-decoded,
/// `.`/`..` folded, never above the root) — the normaliser `<img>` uses too —
/// so a URL that resolves here resolves to the same file once packaged, and
/// one that cannot be packaged cannot be read here either.
///
/// A miss is a 404. It used to fall through to redjs's on-disk resolution,
/// which read it relative to the PROCESS CWD; that is gone in every mode
/// (vn-runtime `NO_DISK_ROOT`), because a packaged exe has no meaningful cwd.
/// A miss that the old fallthrough WOULD have served says so, once.
fn install_asset_provider(rt: &JsRuntime) {
    // `boot` runs again on a cold restart, and the provider is process-wide and
    // set-once; re-registering would only print an advisory.
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(base) = vn_tree::asset_base() else { return };
    ONCE.call_once(move || {
        rt.set_content_provider(move |raw| {
            let key = vn_payload::content_key(raw)?;
            let path = base.join(key.replace('/', std::path::MAIN_SEPARATOR_STR));
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
         exists relative to the working directory, which hosts before m25 read as a fallback — a packaged \
         exe has no such fallback, so neither does dev. Move it under the app (and into vn.config.json \
         \"assets\") to have it served."
    );
}

pub fn boot(rt: &JsRuntime) {
    let Some(manifest) = read_manifest() else { return };
    publish_asset_base(&manifest.entry);
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
            // After the first frame, so boot accounting has closed and this
            // label is never charged.
            eval_path(rt, &module_path(id), &format!("vn://{id}"), "hmr");
        }
        if let Err(e) = rt.eval(&format!("__VN__.hmrApply({text});"), "vn://hmr.js") {
            eprintln!("[dev] hmrApply: {}\n{}", e.header, e.stack);
        }
    });
}
