//! Production boot: the app comes out of the payload stamped into this exe.
//!
//! Enabled by `--features prod`. There is no manifest, module registry, watcher or polling:
//! [`poll_frame`] is an empty inline function. Everything app-specific reads the payload
//! ([`crate::payload`]).

use sk_runtime::JsRuntime;

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
        payload::Source::File(p) => format!("SK_PAYLOAD={}", p.display()),
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
    if !meta.host_variant.is_empty() && wants_no_devtools(&meta.host_variant) == cfg!(sk_devtools) {
        let line = if cfg!(sk_devtools) {
            "[skathi compat] this app was published without devtools and runs on a host built with \
             them: they start off, but SK_DEVTOOLS=1 or appDevtools.enable() can turn them on. \
             Build the host with the no-devtools feature to leave them out."
        } else {
            "[skathi compat] this app was published with devtools (\"publishDevtools\": true) and \
             runs on a host built without them: it has none."
        };
        eprintln!("{line}");
    }
    // `fetch("assets/x")` and `new Image()`: redjs hands us its own cleaned key
    // (query cut, `./` and `/` trimmed) and we run it through the SAME
    // normaliser `<img>` uses, so the two cannot disagree about a URL.
    rt.set_content_provider(|key| {
        let Some(k) = sk_payload::content_key(key) else { return None };
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
                         sk.config.json \"assets\" decides what is packaged (default: the assets/ directory)"
                    ),
                );
                None
            }
        }
    });

    #[cfg(feature = "native-aot")]
    if crate::native_aot::selected(crate::native_aot::Kind::Application) {
        let ran = crate::perf::boot_eval("app native image (+mount)", || {
            crate::native_aot::boot(rt, crate::native_aot::Kind::Application)
        });
        if let Err(e) = ran {
            eprintln!("[prod] app.prod.js (native image): {e}");
        }
        return;
    }
    #[cfg(feature = "no-interpreter")]
    eprintln!("[prod] the host has no interpreter and no native app image was selected");
    #[cfg(not(feature = "no-interpreter"))]
    {
        let Some(script) = loaded.payload.script() else {
            eprintln!("[prod] the payload has no sk/app.js — nothing to run");
            return;
        };
        if let Err(e) = crate::bytecode::eval(rt, script) {
            eprintln!("[prod] app.prod.js: {}\n{}", e.header, e.stack);
        }
    }
}

#[cfg(not(feature = "no-interpreter"))]
pub fn reboot(rt: &JsRuntime) {
    let Some(script) = payload::get().and_then(|l| l.payload.script()) else {
        eprintln!("[prod] reload: the payload has no sk/app.js — nothing to run");
        return;
    };
    // A native entry is a module: it evaluates once per process, so a reload runs the source.
    #[cfg(feature = "native-aot")]
    if crate::native_aot::selected(crate::native_aot::Kind::Application) {
        eprintln!(
            "[native-aot] reload: running app.prod.js; the native entry runs once per process"
        );
        if let Err(e) = rt.eval(script, "sk://app.prod.js") {
            eprintln!("[prod] app.prod.js (reload): {}\n{}", e.header, e.stack);
        }
        return;
    }
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

/// False for a payload published for the no-devtools host (`"publishDevtools"` unset or false).
#[cfg(sk_devtools)]
pub fn devtools_published() -> bool {
    payload::get().is_none_or(|l| !wants_no_devtools(&l.meta.host_variant))
}

fn wants_no_devtools(variant: &str) -> bool {
    variant.split('+').any(|part| part == "no-devtools")
}

/// The payload's `name`, resolved by `sk build` with dev's own chain (`sk.config.json` `"name"`, else
/// the app directory). It beats the exe's file stem, so renaming `Hello.exe` does not lose a user's
/// data and prod apps do not share one storage folder. `SK_APP_NAME` still wins.
pub fn app_name() -> Option<String> {
    payload::get()
        .map(|l| l.meta.name.clone())
        .filter(|n| !n.is_empty())
}

/// `SK_APP_ROOT` is ignored in a packaged exe: the package is the document, and an env var pointing
/// `<img>` at a directory while `fetch()` read the payload would be a dev/prod split.
pub fn publish_asset_store() -> bool {
    if payload::get().is_none() {
        return false;
    }
    sk_tree::set_asset_store(payload::lookup);
    if std::env::var_os("SK_APP_ROOT").is_some_and(|v| !v.is_empty()) {
        eprintln!(
            "[prod] SK_APP_ROOT is ignored: this exe resolves relative URLs out of its own package"
        );
    }
    true
}

pub fn require_payload() {
    if payload::get().is_some() {
        return;
    }
    let msg = format!(
        "This is the Skathi {} host with no app inside.\n\nBuild one with `sk build <app>` \
         (bun packages/cli/cli/sk.ts build apps/hello), or run a payload directly with \
         SK_PAYLOAD=<app.skpk>.",
        env!("SK_PKG_VERSION")
    );
    eprintln!("[prod] {}", msg.replace("\n\n", " "));
    payload::message_box(&msg);
    std::process::exit(2);
}
