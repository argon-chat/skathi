//! Production boot: the app comes out of the payload stamped into this exe.
//!
//! Enabled by `--features prod`. There is no manifest, module registry, watcher or polling:
//! [`poll_frame`] is an empty inline function. Everything app-specific reads the payload
//! ([`crate::payload`]).
//!
//! The inspector bundle and the preludes stay `include_str!`: they belong to the host, not the app.

use vn_runtime::JsRuntime;

use crate::payload;

pub fn boot(rt: &JsRuntime) {
    let Some(loaded) = payload::get() else {
        eprintln!("[prod] no app payload — nothing to boot");
        return;
    };
    let meta = &loaded.meta;
    let from = match &loaded.source {
        payload::Source::Embedded => "embedded".to_string(),
        payload::Source::Bundled(p) => format!("the app bundle ({})", p.display()),
        payload::Source::File(p) => format!("VN_PAYLOAD={}", p.display()),
        payload::Source::Embedder(label) => format!("from the embedder ({label})"),
    };
    let packs: String = loaded
        .packs
        .iter()
        .map(|p| format!(" + {} {} KiB", p.name, p.payload.len() / 1024))
        .collect();
    eprintln!(
        "[prod] {} {} (\"{}\") — payload {from}, {} KiB{packs}, {} asset(s)",
        meta.product_name,
        meta.version,
        meta.name,
        loaded.payload.len() / 1024,
        loaded.asset_count()
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
                let count = payload::get().map_or(0, payload::Loaded::asset_count);
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
    if let Err(e) = crate::bytecode::eval(rt, script) {
        eprintln!("[prod] app.prod.js: {}\n{}", e.header, e.stack);
    }
}

pub fn reboot(rt: &JsRuntime) {
    let Some(script) = payload::get().and_then(|l| l.payload.script()) else {
        eprintln!("[prod] reload: the payload has no vn/app.js — nothing to run");
        return;
    };
    if let Err(e) = crate::bytecode::eval(rt, script) {
        eprintln!("[prod] app.prod.js (reload): {}\n{}", e.header, e.stack);
    }
}

#[inline(always)]
pub fn poll_frame(_frame_no: u32) {}

pub fn window_config() -> Option<String> {
    payload::get()?.payload.window_json().map(str::to_string)
}

pub fn launch_config() -> Option<String> {
    payload::get()?.meta.launch.clone()
}

/// The payload's `name`, resolved by `vn build` with dev's own chain (`vn.config.json` `"name"`, else
/// the app directory). It beats the exe's file stem, so renaming `Hello.exe` does not lose a user's
/// data and prod apps do not share one storage folder. `VN_APP_NAME` still wins.
pub fn app_name() -> Option<String> {
    payload::get()
        .map(|l| l.meta.name.clone())
        .filter(|n| !n.is_empty())
}

/// `VN_APP_ROOT` is ignored in a packaged exe: the package is the document, and an env var pointing
/// `<img>` at a directory while `fetch()` read the payload would be a dev/prod split.
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
