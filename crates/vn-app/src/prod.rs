//! Production boot: everything the app needs is inside the exe.
//!
//! Enabled by `--features prod`. There is no manifest, no module registry, no
//! watcher and no polling — [`poll_frame`] is an empty inline function, so the
//! frame pump costs nothing extra. Two things are baked in at compile time:
//!
//!   1. `packages/vue-native/dist/app.prod.js` — the single IIFE produced by
//!      `bun prod.ts` (see that file's header for how it differs from the dev
//!      two-tier output). `include_str!` means **the JS build must run before
//!      `cargo build --features prod`**; a missing file is a compile error, on
//!      purpose — shipping an exe with a stale or absent bundle is worse.
//!   2. `apps/hello/assets/**` — served to `fetch()` / `Image` through redjs'
//!      content provider (research/reports/vue-renderer.md §e.4), which is
//!      consulted BEFORE the on-disk roots. So `fetch("assets/hello.txt")`
//!      resolves out of the binary with no filesystem and no HTTP.
//!
//! v1 keeps the asset table hand-rolled — one `include_bytes!` per entry via
//! the [`embed`] macro below — rather than pulling in `rust-embed`. Adding a
//! file means adding a line here; a build script that walks the directory is
//! the obvious follow-up when the list stops being short.

use vn_runtime::JsRuntime;

/// The whole Vue app, compiled and minified, as one classic script.
const APP_JS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/vue-native/dist/app.prod.js"));

/// Content-relative key -> bytes. Keys are exactly what redjs hands the
/// provider: `/`-separated, no leading `./` or `/`, query/hash already stripped.
macro_rules! embed {
    ($($rel:literal),* $(,)?) => {
        &[$((
            concat!("assets/", $rel),
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/hello/assets/", $rel))
                as &[u8],
        )),*]
    };
}

static ASSETS: &[(&str, &[u8])] = embed!["hello.txt"];

fn lookup(key: &str) -> Option<&'static [u8]> {
    // redjs already normalizes, but a provider that only ever matches one
    // spelling of a path is a footgun; be liberal about the leading slash.
    let key = key.trim_start_matches('/');
    ASSETS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Proof that the embedded-asset path is live, printed into the shell log.
/// Prod-only and Rust-side on purpose: `apps/hello` stays a plain Vue app with
/// no host-probe code in it.
const ASSET_PROBE_JS: &str = r#"
    fetch("assets/hello.txt")
        .then((r) => (r.ok ? r.text() : Promise.reject(new Error("HTTP " + r.status))))
        .then((t) => __vn_log("[asset] " + String(t).trim()))
        .catch((e) => __vn_log("[asset] fetch failed: " + e));
"#;

/// Installs the content provider, evaluates the embedded bundle (which mounts
/// the app), then fires the asset probe.
pub fn boot(rt: &JsRuntime) {
    rt.set_content_provider(|key| {
        match lookup(key) {
            Some(bytes) => Some(bytes.to_vec()),
            None => {
                // A miss is not an error — redjs falls through to the on-disk
                // roots and `fetch` ends up with a browser-shaped 404. But in a
                // shipped exe there is no disk to fall through to, so say so
                // once with the key list rather than leaving a silent 404.
                if key.starts_with("assets/") {
                    warn_missing_asset(key);
                }
                None
            }
        }
    });

    if let Err(e) = rt.eval(APP_JS, "vn://app.prod.js") {
        eprintln!("[prod] app.prod.js: {}\n{}", e.header, e.stack);
        return;
    }
    if let Err(e) = rt.eval(ASSET_PROBE_JS, "vn://asset-probe.js") {
        eprintln!("[prod] asset probe: {}\n{}", e.header, e.stack);
    }
}

/// No dev transport in prod: nothing to poll, nothing to reload.
#[inline(always)]
pub fn poll_frame(_frame_no: u32) {}

fn warn_missing_asset(key: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if WARNED.swap(true, Ordering::Relaxed) {
        return;
    }
    let keys: Vec<&str> = ASSETS.iter().map(|(k, _)| *k).collect();
    eprintln!(
        "[prod] no embedded asset {key:?} — this build embeds: {}\n\
         [prod] (add it to ASSETS in crates/vn-app/src/prod.rs; further misses are silent)",
        keys.join(", ")
    );
}
