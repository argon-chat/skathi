//! The app inside a packaged exe.
//!
//! A `--features prod` build is a BARE HOST: the whole engine and no app. `vn
//! build` (packages/vue-native/cli) copies it and stamps one VNPK blob into the
//! copy as the `RT_RCDATA` resource `VNPAYLOAD` — the bundle, the window
//! config, the app's identity and every asset — so an app author ships their
//! own exe without a Rust toolchain (docs/PACKAGING.md).
//!
//! ```text
//!   payload source            parser (crates/vn-payload)      consumers
//!  ┌───────────────────┐     ┌─────────────────────────┐     ┌──────────────────────────────┐
//!  │ Embedded          │     │ Payload::parse — O(n),  │     │ prod.rs: script, window.json,│
//!  │  FindResourceW →  ├────▶│ header + TOC only, never├────▶│   app name, content provider │
//!  │  LockResource     │     │ the asset bytes         │     │ main.rs: GpuiAssets, the     │
//!  │ File (bare host)  │     └─────────────────────────┘     │   vn_tree asset store, slot  │
//!  │  VN_PAYLOAD=…vnpk │                                     └──────────────────────────────┘
//!  └───────────────────┘
//! ```
//!
//! **Embedded** is the image itself: `LockResource` hands back a pointer into
//! the mapped exe, valid for the life of the process, demand-paged, read-only
//! and SHARED between instances of the same exe — the same memory story as the
//! `include_str!` it replaces (design V7). The lookup costs 12–36 µs.
//!
//! **File** is honoured ONLY when nothing is embedded, i.e. on a bare host: it
//! is the in-repo iteration loop (`VN_PAYLOAD=apps/hello/dist/.vn/app.vnpk
//! vn-app.exe`) and nothing else. A stamped exe ignores it, with one line —
//! otherwise one environment variable could run unsigned JS under a signed
//! app's identity (design §4.12).
//!
//! The module is in every build because `--vn-host-info` is: a dev host answers
//! it too (`"mode": "dev"`), which is how `vn build` refuses one. Everything
//! else here has only `prod` callers.
#![cfg_attr(not(feature = "prod"), allow(dead_code))]

use std::borrow::Cow;
use std::sync::OnceLock;

use vn_payload::Payload;

/// Where this process's payload came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Embedded,
    File(std::path::PathBuf),
}

/// The META entry, read once.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// The identity: `%LOCALAPPDATA%\<name>\storage`, the devtools slot.
    pub name: String,
    pub product_name: String,
    pub version: String,
    /// The vue-native version whose `vn` built this payload.
    pub vue_native: String,
    pub host_variant: String,
    /// The `launch` object (`id`, `singleInstance`, `raiseOnSecondInstance`,
    /// `protocols`) as JSON, when `vn build` wrote one (docs/LAUNCH.md).
    pub launch: Option<String>,
    /// `vn.config.json` `"argonSecure": true`: run the TEMPORARY ArgonSecure
    /// producer (src/temp_argon_secure.rs) at startup.
    pub argon_secure: bool,
}

pub struct Loaded {
    pub payload: Payload<'static>,
    pub meta: Meta,
    pub source: Source,
}

static LOADED: OnceLock<Option<Loaded>> = OnceLock::new();

/// This process's payload, resolved on first ask. `None` on a bare host with
/// no `VN_PAYLOAD`. A payload that is present but corrupt is fatal: there is no
/// app to run, and running none silently would be worse.
pub fn get() -> Option<&'static Loaded> {
    LOADED.get_or_init(load).as_ref()
}

/// Is a payload STAMPED into this exe? Unlike [`get`], `VN_PAYLOAD` does not
/// count — this is the "bare host or app?" question `--vn-host-info` asks.
pub fn embedded_present() -> bool {
    embedded().is_some()
}

fn load() -> Option<Loaded> {
    let (bytes, source) = match embedded() {
        Some(bytes) => {
            if std::env::var_os("VN_PAYLOAD").is_some_and(|v| !v.is_empty()) {
                eprintln!(
                    "[payload] VN_PAYLOAD is ignored: this exe carries its own app. (It is honoured only \
                     by a bare host, so a stamped — later signed — exe can never be made to run other JS.)"
                );
            }
            (bytes, Source::Embedded)
        }
        None => {
            let path = std::env::var_os("VN_PAYLOAD").filter(|v| !v.is_empty())?;
            let path = std::path::PathBuf::from(path);
            match std::fs::read(&path) {
                // Leaked on purpose: a payload lives as long as the process,
                // exactly like the mapped one it stands in for.
                Ok(v) => (&*Box::leak(v.into_boxed_slice()), Source::File(path)),
                Err(e) => fatal(&format!("VN_PAYLOAD={}: {e}", path.display())),
            }
        }
    };
    let payload = match Payload::parse(bytes) {
        Ok(p) => p,
        Err(e) => fatal(&format!("the app payload is unreadable — {e}")),
    };
    let meta = payload.meta().map(parse_meta).unwrap_or_default();
    if !meta.vue_native.is_empty() && meta.vue_native != env!("VN_PKG_VERSION") {
        // Compat-first: say it loudly and carry on. `vn build` refuses a
        // mismatched host, so reaching this means someone stamped by hand.
        eprintln!(
            "[payload] this app was built by vue-native {} and is running on a {} host — the bundle \
             carries the JS half of the host protocol, so expect breakage; rebuild it with the matching vn",
            meta.vue_native,
            env!("VN_PKG_VERSION")
        );
    }
    Some(Loaded {
        payload,
        meta,
        source,
    })
}

fn parse_meta(text: &str) -> Meta {
    let Some(v) = crate::json::parse(text) else {
        return Meta::default();
    };
    let s = |k: &str| {
        v.get(k)
            .and_then(crate::json::Json::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Meta {
        name: s("name"),
        product_name: s("productName"),
        version: s("version"),
        vue_native: s("vueNative"),
        host_variant: v
            .get("host")
            .and_then(|h| h.get("variant"))
            .and_then(crate::json::Json::as_str)
            .unwrap_or_default()
            .to_string(),
        launch: v.get("launch").map(crate::json::Json::to_json),
        argon_secure: v.get("argonSecure").and_then(crate::json::Json::as_bool).unwrap_or(false),
    }
}

/// Tells the user and stops. Before any window exists, so stderr AND a message
/// box: a double-clicked GUI-subsystem exe has no console to print into.
pub fn fatal(msg: &str) -> ! {
    eprintln!("[payload] {msg}");
    message_box(&format!("{msg}\n\nRebuild the app with `vn build`."));
    std::process::exit(3);
}

/// A modal box, unless a harness owns this process (`VN_OWNER_PID`): a test run
/// must never block on a dialog nobody will dismiss.
pub fn message_box(text: &str) {
    if std::env::var_os("VN_OWNER_PID").is_some() {
        return;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let _ = MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("vue-native"),
            MB_OK | MB_ICONERROR,
        );
    }
    #[cfg(not(target_os = "windows"))]
    let _ = text;
}

/// The stamped `VNPAYLOAD` resource, as the mapped image's own bytes.
#[cfg(target_os = "windows")]
fn embedded() -> Option<&'static [u8]> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::LibraryLoader::{
        FindResourceW, LoadResource, LockResource, SizeofResource,
    };
    // RT_RCDATA is MAKEINTRESOURCE(10). (windows-rs files the constant under an
    // unrelated feature, so it is spelled out rather than pulled in.)
    const RT_RCDATA: PCWSTR = PCWSTR(10 as _);
    static BYTES: OnceLock<Option<&'static [u8]>> = OnceLock::new();
    *BYTES.get_or_init(|| unsafe {
        let res = FindResourceW(None, w!("VNPAYLOAD"), RT_RCDATA);
        if res.is_invalid() {
            return None;
        }
        let global = LoadResource(None, res).ok()?;
        let ptr = LockResource(global) as *const u8;
        let len = SizeofResource(None, res) as usize;
        if ptr.is_null() || len == 0 {
            return None;
        }
        // SAFETY: a resource of the main module is mapped for the life of the
        // process and never written; the module is never unloaded.
        Some(std::slice::from_raw_parts(ptr, len))
    })
}

#[cfg(not(target_os = "windows"))]
fn embedded() -> Option<&'static [u8]> {
    None
}

// ---------------------------------------------------------------------------
// the asset lookup — one function behind fetch(), <img>, <lottie>, <video>
// ---------------------------------------------------------------------------

/// `key` (already a [`vn_payload::content_key`]) → the asset's bytes.
///
/// Exact first (a binary search); on a miss, an ASCII-case-insensitive scan
/// with ONE advisory naming both spellings — dev runs on NTFS, where
/// `assets/Logo.PNG` opens `assets/logo.png`, and a packaged exe must not be
/// stricter than the dev host the author tested on.
///
/// A plain `fn` so it can be handed to `vn_tree::set_asset_store`, and
/// thread-safe because gpui calls [`GpuiAssets`] from its loader threads.
pub fn lookup(key: &str) -> Option<&'static [u8]> {
    let loaded = get()?;
    if let Some(bytes) = loaded.payload.asset(key) {
        return Some(bytes);
    }
    let (stored, bytes) = loaded.payload.asset_ignoring_case(key)?;
    advise_once(
        key,
        &format!(
            "{key:?} matched the packaged asset {stored:?} only by ignoring case — it works (NTFS is \
             case-insensitive and so is this lookup), but the spellings should agree"
        ),
    );
    Some(bytes)
}

/// One advisory per distinct key, bounded so a loop of misses cannot grow it.
pub fn advise_once(key: &str, msg: &str) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    let seen = seen.get_or_insert_with(HashSet::new);
    if seen.len() >= 64 || !seen.insert(key.to_string()) {
        return;
    }
    eprintln!("[vue-native compat] {msg}");
    // The ring is per-thread and the main thread's is the one `logs` reads;
    // from a loader thread this lands nowhere, which is why stderr goes first.
    vn_tree::log::record("advisory", msg.to_string());
}

/// gpui's `AssetSource` over the payload: what `Resource::Embedded(key)`
/// (`vn_tree::img`'s store hit) is read back through. Borrowed, never copied
/// here — gpui decodes from a transient `to_vec` exactly as it does for
/// `Resource::Path`.
pub struct GpuiAssets;

impl gpui::AssetSource for GpuiAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(lookup(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        let Some(loaded) = get() else {
            return Ok(Vec::new());
        };
        Ok(loaded
            .payload
            .assets()
            .iter()
            .filter(|e| e.name.starts_with(path))
            .map(|e| gpui::SharedString::from(e.name.to_string()))
            .collect())
    }
}

/// The folder-name form of the app's identity — the devtools slot directory
/// (`%TEMP%\vue-native\apps\<this>\<pid>`). Same rule vn-storage applies to
/// the storage folder, so the two never disagree about a name.
pub fn slot_name() -> String {
    let raw = get().map(|l| l.meta.name.clone()).unwrap_or_default();
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let trimmed = cleaned.trim_matches(['-', '.'].as_slice());
    if trimmed.is_empty() {
        "vn-host".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The parts of the engine a host can be built without, in the order
/// [`host_variant`] names them — the same list, in the same order, as
/// `OPTIONAL` in packages/vue-native/cli/config.ts. `devtools` is the inverse
/// of the `no-devtools` feature; the rest are default features of their own.
pub const OPTIONAL: [(&str, bool); 6] = [
    ("devtools", cfg!(vn_devtools)),
    ("voice", cfg!(feature = "voice")),
    ("audio", cfg!(feature = "audio")),
    ("clipboard", cfg!(feature = "clipboard")),
    ("storages", cfg!(feature = "storages")),
    ("indexeddb", cfg!(feature = "indexeddb")),
];

/// This host's variant: `standard` when nothing optional was left out, else
/// what was, as `no-<part>` joined by `+` in [`OPTIONAL`]'s order —
/// `no-devtools`, `no-voice`, `no-devtools+no-voice`. `vn build` keys its host
/// cache by it, and asks for one by it.
pub fn host_variant() -> String {
    let off: Vec<String> = OPTIONAL.iter().filter(|(_, on)| !on).map(|(name, _)| format!("no-{name}")).collect();
    if off.is_empty() {
        "standard".to_string()
    } else {
        off.join("+")
    }
}

/// `--vn-host-info`: what this exe is, as one JSON line, for `vn build`'s host
/// check. Printed only by an exe with NO payload stamped into it — a packaged
/// app passes its argv to the app untouched.
pub fn host_info_json() -> String {
    use crate::json::Json;
    let mut features: Vec<Json> = Vec::new();
    for (on, name) in [
        (cfg!(feature = "prod"), "prod"),
        (cfg!(feature = "no-devtools"), "no-devtools"),
        (cfg!(feature = "audio"), "audio"),
        (cfg!(feature = "storages"), "storages"),
        (cfg!(feature = "clipboard"), "clipboard"),
        (cfg!(feature = "indexeddb"), "indexeddb"),
        (cfg!(feature = "voice"), "voice"),
    ] {
        if on {
            features.push(Json::s(name));
        }
    }
    Json::obj(vec![
        (
            "mode",
            Json::s(if cfg!(feature = "prod") {
                "prod"
            } else {
                "dev"
            }),
        ),
        ("vueNative", Json::s(env!("VN_PKG_VERSION"))),
        ("crate", Json::s(env!("CARGO_PKG_VERSION"))),
        ("variant", Json::s(host_variant())),
        ("devtools", Json::Bool(cfg!(vn_devtools))),
        ("features", Json::arr(features)),
        ("jscPin", Json::s(env!("VN_JSC_PIN"))),
        ("payloadFormat", Json::n(f64::from(vn_payload::FORMAT))),
        (
            "profile",
            Json::s(if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }),
        ),
    ])
    .to_json()
}

#[cfg(test)]
mod tests {
    #[test]
    fn meta_is_read_from_the_payload_json() {
        let m = super::parse_meta(
            r#"{"format":1,"name":"hello","productName":"Hello","version":"0.1.0","vueNative":"0.0.1","host":{"version":"0.0.1","variant":"no-devtools"},"entry":"src/main.ts","assets":3}"#,
        );
        assert_eq!(m.name, "hello");
        assert_eq!(m.product_name, "Hello");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.host_variant, "no-devtools");
        assert_eq!(super::parse_meta("not json").name, "");
    }

    #[test]
    fn host_info_is_one_json_line_naming_this_build() {
        let info = super::host_info_json();
        assert!(!info.contains('\n'));
        let v = crate::json::parse(&info).expect("valid JSON");
        let mode = v.get("mode").and_then(crate::json::Json::as_str).unwrap();
        assert_eq!(
            mode,
            if cfg!(feature = "prod") {
                "prod"
            } else {
                "dev"
            }
        );
        assert_eq!(
            v.get("vueNative").and_then(crate::json::Json::as_str),
            Some(env!("VN_PKG_VERSION"))
        );
        let variant = v
            .get("variant")
            .and_then(crate::json::Json::as_str)
            .unwrap();
        assert_eq!(variant, super::host_variant());
        // Every optional part in: the name every host built before features
        // could be left out already carried.
        assert_eq!(variant == "standard", super::OPTIONAL.iter().all(|(_, on)| *on));
        assert_eq!(variant.contains("no-devtools"), !cfg!(vn_devtools));
        assert_eq!(
            v.get("payloadFormat").and_then(crate::json::Json::as_f64),
            Some(1.0)
        );
    }

    #[test]
    fn a_test_binary_carries_no_payload() {
        // `cargo test` builds an unstamped exe, so the embedded lookup must
        // report absence rather than misread something else.
        assert!(!super::embedded_present());
    }
}
