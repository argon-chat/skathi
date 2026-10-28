//! Production boot: the app comes out of the payload stamped into this exe.
//!
//! Enabled by `--features prod`. There is no manifest, no module registry, no
//! watcher and no polling — [`poll_frame`] is an empty inline function, so the
//! frame pump costs nothing extra.
//!
//! Until m25 this file `include_str!`d `packages/vue-native/dist/app.prod.js`,
//! `dist/window.json` and a hand-rolled table of `apps/hello/assets/**`, which
//! meant the prod exe WAS apps/hello: every other app needed a Rust edit and a
//! full engine build. Now a prod build is a BARE HOST and `vn build` stamps an
//! app into a copy of it (docs/PACKAGING.md); everything app-specific reads the
//! payload ([`crate::payload`]):
//!
//! | what | from |
//! |---|---|
//! | the bundle | `vn/app.js` — exactly `bun prod.ts`'s output |
//! | the window config | `vn/window.json` — exactly what dev reads from the manifest |
//! | the app's name | `vn/app.json` `name` — `vn.config.json` `"name"`, else the app directory |
//! | `fetch()` / `Image` | the content provider below, over the payload's assets |
//! | `<img>` / `<lottie>` / `<video>` | `vn_tree::set_asset_store` (main.rs) — the same lookup |
//!
//! The inspector bundle and the preludes stay `include_str!`: they belong to
//! the host, not to the app.

use vn_runtime::JsRuntime;

use crate::payload;

/// Installs the content provider and evaluates the packaged bundle, which
/// mounts the app. `main` has already refused to get here without a payload.
pub fn boot(rt: &JsRuntime) {
    let Some(loaded) = payload::get() else {
        eprintln!("[prod] no app payload — nothing to boot");
        return;
    };
    let meta = &loaded.meta;
    let from = match &loaded.source {
        payload::Source::Embedded => "embedded".to_string(),
        payload::Source::File(p) => format!("VN_PAYLOAD={}", p.display()),
    };
    eprintln!(
        "[prod] {} {} (\"{}\") — payload {from}, {} KiB, {} asset(s)",
        meta.product_name,
        meta.version,
        meta.name,
        loaded.payload.len() / 1024,
        loaded.payload.asset_count()
    );
    let variant = if cfg!(vn_devtools) {
        "standard"
    } else {
        "no-devtools"
    };
    if !meta.host_variant.is_empty() && meta.host_variant != variant {
        // `vn build` picks the host from this field, so a mismatch means a
        // hand-made stamp. Harmless, but the app asked for the other build.
        eprintln!(
            "[vue-native compat] this app was packaged for the {} host and is running on the {variant} one",
            meta.host_variant
        );
    }
    // `fetch("assets/x")` and `new Image()`: redjs hands us its own cleaned key
    // (query cut, `./` and `/` trimmed) and we run it through the SAME
    // normaliser `<img>` uses, so the two cannot disagree about a URL.
    rt.set_content_provider(|key| {
        let Some(k) = vn_payload::content_key(key) else { return None };
        match payload::lookup(&k) {
            Some(bytes) => Some(bytes.to_vec()),
            None => {
                // A miss is not an error — redjs answers with a browser-shaped
                // 404. But a packaged app's author should hear WHY once per
                // key, instead of debugging a silent 404.
                let count = payload::get().map_or(0, |l| l.payload.asset_count());
                payload::advise_once(
                    &k,
                    &format!(
                        "fetch({key:?}): {k:?} is not in this app's package ({count} packaged asset(s)) — \
                         vn.config.json \"assets\" decides what is packaged (default: the assets/ directory)"
                    ),
                );
                None
            }
        }
    });

    let Some(script) = loaded.payload.script() else {
        eprintln!("[prod] the payload has no vn/app.js — nothing to run");
        return;
    };
    if let Err(e) = crate::perf::boot_eval("app.prod.js (+mount)", || {
        rt.eval(script, "vn://app.prod.js")
    }) {
        eprintln!("[prod] app.prod.js: {}\n{}", e.header, e.stack);
    }
}

/// No dev transport in prod: nothing to poll, nothing to reload.
#[inline(always)]
pub fn poll_frame(_frame_no: u32) {}

/// The `window` block of `vn.config.json`, as `bun prod.ts` wrote it into the
/// payload (`{}` when the app declares none) — the same JSON the dev host reads
/// out of `dist/manifest.json`.
pub fn window_config() -> Option<String> {
    payload::get()?.payload.window_json().map(str::to_string)
}

/// The app's name — see [`crate::dev::app_name`] for what it is for.
///
/// The payload's `name`, resolved by `vn build` with dev's own chain
/// (`vn.config.json` `"name"`, else the app directory). It beats the exe's file
/// stem: renaming `Hello.exe` must not lose a user's data, and before m25 every
/// prod exe was `vn-app.exe` and so every prod app shared ONE storage folder
/// (docs/STORAGE.md §3 row 4, owner decision m25). `VN_APP_NAME` still wins.
pub fn app_name() -> Option<String> {
    payload::get()
        .map(|l| l.meta.name.clone())
        .filter(|n| !n.is_empty())
}

/// `<img>`, `<lottie>`, `<video>`, `poster`: resolve relative URLs out of the
/// payload instead of the disk. Returns whether a store was installed.
///
/// `VN_APP_ROOT` is ignored in a packaged exe — the package IS the document,
/// and an env var that pointed `<img>` at a directory while `fetch()` kept
/// reading the payload would be exactly the dev/prod split this replaced.
pub fn publish_asset_store() -> bool {
    if payload::get().is_none() {
        return false;
    }
    vn_tree::set_asset_store(payload::lookup);
    if std::env::var_os("VN_APP_ROOT").is_some_and(|v| !v.is_empty()) {
        eprintln!(
            "[prod] VN_APP_ROOT is ignored: this exe resolves relative URLs out of its own package"
        );
    }
    true
}

/// A prod build with no payload is a bare host: say what it is and stop.
/// `--devtools` (the inspector mode) needs no app and keeps working.
pub fn require_payload() {
    if payload::get().is_some() {
        return;
    }
    let msg = format!(
        "This is the vue-native {} host with no app inside.\n\nBuild one with `vn build <app>` \
         (bun packages/vue-native/cli/vn.ts build apps/hello), or run a payload directly with \
         VN_PAYLOAD=<app.vnpk>.",
        env!("VN_PKG_VERSION")
    );
    eprintln!("[prod] {}", msg.replace("\n\n", " "));
    payload::message_box(&msg);
    std::process::exit(2);
}
