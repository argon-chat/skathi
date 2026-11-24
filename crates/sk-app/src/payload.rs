//! The app inside a packaged exe.
//!
//! A `--features prod` build is a bare host: the engine and no app. `sk build` copies it and stamps
//! one SKPK blob (bundle, window config, identity, assets) into the copy as the `RT_RCDATA` resource
//! `SKPAYLOAD` (Linux: appended to the ELF, [`sk_payload::appended`]), so an app author ships their
//! own exe without a Rust toolchain.
//!
//! **File** (`SK_PAYLOAD=app.skpk`) is read through a read-only file mapping whose owner and handle
//! live with the parsed payload, so nothing is copied or faulted in unused. Windows denies in-place
//! writes while allowing atomic replacement; POSIX publishers must keep active files immutable and
//! publish a new inode. It is honoured only when nothing is embedded (the in-repo iteration loop): a
//! stamped exe ignores it with one line, since one environment variable could otherwise run
//! unsigned JS under a signed app's identity.
#![cfg_attr(not(feature = "prod"), allow(dead_code))]

use std::borrow::Cow;
use std::sync::OnceLock;

use sk_payload::Payload;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Embedded,
    Bundled(std::path::PathBuf),
    File(std::path::PathBuf),
    Embedder(String),
}

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub name: String,
    pub product_name: String,
    pub version: String,
    pub skathi: String,
    pub host_variant: String,
    pub publish_devtools: Option<bool>,
    pub launch: Option<String>,
    pub packs: Vec<PackRef>,
    /// `"invariant"`: the app asked for the root locale only; anything else is every locale.
    pub intl: String,
    /// `"low"`: the app asked for the smaller footprint (`crate::memory`).
    pub memory: String,
    /// `"skathi-mode"` or `"optimized"` (`crate::text_rendering`); empty is the platform's default.
    pub text_rendering: String,
    /// `"permissions"` of sk.config.json, as JSON (`crate::permissions`).
    pub permissions: String,
    /// Why the app's runtime cannot run on this host ([`runtime_mismatch`]).
    pub runtime_mismatch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackRef {
    pub name: String,
    pub file: String,
    pub id: String,
}

pub struct Pack {
    pub name: String,
    pub payload: AppPayload,
}

pub(crate) enum PayloadBytes {
    Embedded(&'static [u8]),
    File(crate::mapped_file::MappedFile),
    Owned(Vec<u8>),
}

impl PayloadBytes {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Embedded(bytes) => bytes,
            Self::File(file) => file.as_bytes(),
            Self::Owned(bytes) => bytes,
        }
    }
}

self_cell::self_cell! {
    struct PayloadCell {
        owner: PayloadBytes,
        #[covariant]
        dependent: Payload,
    }
}

/// The parser borrows its owner, never a widened/fabricated `'static` slice.
/// Only the process-wide Loaded reference makes successful app assets static.
pub struct AppPayload(PayloadCell);

impl AppPayload {
    fn parse(bytes: PayloadBytes) -> Result<Self, String> {
        PayloadCell::try_new(bytes, |bytes| Payload::parse(bytes.as_bytes()))
            .map(Self)
            .map_err(|e| format!("the app payload is unreadable — {e}"))
    }

    #[cfg(all(test, feature = "native-aot"))]
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        Self::parse(PayloadBytes::Owned(bytes))
    }

    fn meta(&self) -> Option<&str> {
        self.0.borrow_dependent().meta()
    }
    pub(crate) fn is_file(&self) -> bool {
        matches!(self.0.borrow_owner(), PayloadBytes::File(_))
    }
    pub fn len(&self) -> usize {
        self.0.borrow_dependent().len()
    }
    pub fn asset_count(&self) -> usize {
        self.0.borrow_dependent().asset_count()
    }
    pub fn script(&self) -> Option<&str> {
        self.0.borrow_dependent().script()
    }
    pub fn window_json(&self) -> Option<&str> {
        self.0.borrow_dependent().window_json()
    }
    #[cfg(feature = "native-aot")]
    pub fn native_manifest(&self) -> Option<&str> {
        self.0.borrow_dependent().native_manifest()
    }
    pub fn effects(&self) -> Vec<(&str, &[u8])> {
        self.0.borrow_dependent().effects().collect()
    }
    pub fn assets(&self) -> &[sk_payload::Entry<'_>] {
        self.0.borrow_dependent().assets()
    }
    pub fn asset(&self, key: &str) -> Option<&[u8]> {
        self.0.borrow_dependent().asset(key)
    }
    pub fn asset_ignoring_case(&self, key: &str) -> Option<(&str, &[u8])> {
        self.0.borrow_dependent().asset_ignoring_case(key)
    }
    pub(crate) fn icu_data(&self) -> Option<&[u8]> {
        self.0.borrow_dependent().icu_data()
    }
}

pub struct Loaded {
    pub payload: AppPayload,
    pub packs: Vec<Pack>,
    pub meta: Meta,
    pub source: Source,
    /// The folder of the files `sk build` writes beside the payload: packs, ICU, the native image.
    pub dir: Option<std::path::PathBuf>,
}

impl Loaded {
    pub fn asset_count(&self) -> usize {
        self.payload.asset_count()
            + self
                .packs
                .iter()
                .map(|p| p.payload.asset_count())
                .sum::<usize>()
    }

    pub fn assets(&self) -> impl Iterator<Item = &sk_payload::Entry<'_>> {
        self.payload
            .assets()
            .iter()
            .chain(self.packs.iter().flat_map(|p| p.payload.assets().iter()))
    }
}

pub fn pack_id(payload: &AppPayload) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(payload.0.borrow_dependent().index())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn payload_dir(dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    match dir {
        Some(dir) => Some(dir.to_path_buf()),
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
    }
}

fn open_packs(meta: &Meta, dir: Option<&std::path::Path>) -> Result<Vec<Pack>, String> {
    if meta.packs.is_empty() {
        return Ok(Vec::new());
    }
    let dir = payload_dir(dir)
        .ok_or("the app's asset packs are beside the executable, whose folder is unknown")?;
    meta.packs
        .iter()
        .map(|r| {
            // A bare file name: the META is the app's, but a pack is never
            // looked for outside the payload's own folder.
            if r.file.is_empty() || r.file.contains(['/', '\\', ':']) || r.file.starts_with('.') {
                return Err(format!("the asset pack {:?} has the file name {:?}, which is not a plain file name", r.name, r.file));
            }
            let path = dir.join(&r.file);
            let file = crate::mapped_file::MappedFile::open(&path)
                .map_err(|e| format!("the asset pack {:?} ({}): {e}", r.name, path.display()))?;
            let payload = AppPayload::parse(PayloadBytes::File(file))
                .map_err(|e| format!("the asset pack {:?} ({}): {e}", r.name, path.display()))?;
            let id = pack_id(&payload);
            if id != r.id {
                return Err(format!(
                    "the asset pack {} is from another build of the app (its id is {id}, the app expects {}) — \
                     reinstall or rebuild so the app and its packs match",
                    path.display(),
                    r.id
                ));
            }
            Ok(Pack { name: r.name.clone(), payload })
        })
        .collect()
}

/// The full ICU data, beside the payload like the asset packs.
pub const ICU_PACK: &str = "icu.skpk";

/// The ICU data a payload stamped into this executable carries (`"publishSingleFile"`).
fn stamped_icu(payload: &AppPayload, source: &Source) -> Option<&'static [u8]> {
    if !matches!(source, Source::Embedded) {
        return None;
    }
    let data = payload.icu_data()?;
    // SAFETY: an embedded payload borrows the executable's own image, mapped and never written
    // for the life of the process.
    Some(unsafe { std::slice::from_raw_parts(data.as_ptr(), data.len()) })
}

/// Hands ICU the full data when this host links only the root locale and the app did not ask
/// for that alone: from the payload itself, else from [`ICU_PACK`]. Without either, Intl runs
/// on the root locale.
fn install_icu(stamped: Option<&'static [u8]>, meta: &Meta, dir: Option<&std::path::Path>) {
    if redjs::icu::LINKED != redjs::icu::Linked::Root || meta.intl == "invariant" {
        return;
    }
    if let Some(data) = stamped {
        if let Err(e) = redjs::icu::set_common_data(data) {
            eprintln!(
                "[payload] the ICU data in the app: {e} — Intl falls back to the root locale"
            );
        }
        return;
    }
    let Some(path) = payload_dir(dir).map(|d| d.join(ICU_PACK)) else {
        return;
    };
    if let Err(e) = open_icu(&path) {
        eprintln!(
            "[payload] {e} — Intl and toLocale* fall back to the root locale. Ship the {ICU_PACK} \
             that sk build writes beside the app, or build with \"intl\": \"invariant\""
        );
    }
}

fn open_icu(path: &std::path::Path) -> Result<(), String> {
    let file = crate::mapped_file::MappedFile::open(path)
        .map_err(|e| format!("the ICU data {}: {e}", path.display()))?;
    let pack: &'static AppPayload = Box::leak(Box::new(
        AppPayload::parse(PayloadBytes::File(file))
            .map_err(|e| format!("the ICU data {}: {e}", path.display()))?,
    ));
    let data = pack
        .icu_data()
        .ok_or_else(|| format!("the ICU data {} holds no ICU package", path.display()))?;
    redjs::icu::set_common_data(data).map_err(|e| format!("the ICU data {}: {e}", path.display()))
}

static LOADED: OnceLock<Option<Loaded>> = OnceLock::new();

/// Resolved on first ask. `None` on a bare host; a corrupt payload is fatal, since running no app silently would be worse.
pub fn get() -> Option<&'static Loaded> {
    LOADED.get_or_init(load).as_ref()
}

/// The embedder's payload (src/embed.rs), parsed now so a bad one is the embedder's `Err` rather
/// than [`fatal`]. Before the first [`get`]; it then wins over a stamped resource and `SK_PAYLOAD`.
pub(crate) fn set_external(
    bytes: PayloadBytes,
    label: String,
    dir: Option<&std::path::Path>,
) -> Result<(), String> {
    let payload = AppPayload::parse(bytes)?;
    let meta = payload.meta().map(parse_meta).unwrap_or_default();
    if !meta.skathi.is_empty() && meta.skathi != env!("SK_PKG_VERSION") {
        return Err(format!(
            "this app was built by Skathi {} and this library is {}: the bundle carries the JS half of \
             the host protocol, so rebuild it with the matching sk",
            meta.skathi,
            env!("SK_PKG_VERSION")
        ));
    }
    let packs = open_packs(&meta, dir)?;
    install_icu(None, &meta, dir);
    LOADED
        .set(Some(Loaded {
            payload,
            packs,
            meta,
            source: Source::Embedder(label),
            dir: payload_dir(dir),
        }))
        .map_err(|_| "a payload was already resolved in this process".to_string())
}

/// Is a payload STAMPED into this exe? Unlike [`get`], `SK_PAYLOAD` does not
/// count — this is the "bare host or app?" question `--sk-host-info` asks.
pub fn embedded_present() -> bool {
    embedded().is_some() || bundled().is_some()
}

fn load() -> Option<Loaded> {
    let carried = match embedded() {
        Some(bytes) => Some((PayloadBytes::Embedded(bytes), Source::Embedded)),
        None => bundled().map(|path| match crate::mapped_file::MappedFile::open(&path) {
            Ok(file) => (PayloadBytes::File(file), Source::Bundled(path)),
            Err(e) => fatal(&format!("{}: {e}", path.display())),
        }),
    };
    let (bytes, source) = match carried {
        Some(carried) => {
            if std::env::var_os("SK_PAYLOAD").is_some_and(|v| !v.is_empty()) {
                eprintln!(
                    "[payload] SK_PAYLOAD is ignored: this exe carries its own app. (It is honoured only \
                     by a bare host, so a stamped — later signed — exe can never be made to run other JS.)"
                );
            }
            carried
        }
        None => {
            let path = std::env::var_os("SK_PAYLOAD").filter(|v| !v.is_empty())?;
            let path = std::path::PathBuf::from(path);
            match crate::mapped_file::MappedFile::open(&path) {
                Ok(file) => (PayloadBytes::File(file), Source::File(path)),
                Err(e) => fatal(&format!("SK_PAYLOAD={}: {e}", path.display())),
            }
        }
    };
    let payload = match AppPayload::parse(bytes) {
        Ok(p) => p,
        Err(e) => fatal(&e),
    };
    let meta = payload.meta().map(parse_meta).unwrap_or_default();
    if !meta.skathi.is_empty() && meta.skathi != env!("SK_PKG_VERSION") {
        // Compat-first: say it loudly and carry on. `sk build` refuses a
        // mismatched host, so reaching this means someone stamped by hand.
        eprintln!(
            "[payload] this app was built by Skathi {} and is running on a {} host — the bundle \
             carries the JS half of the host protocol, so expect breakage; rebuild it with the matching sk",
            meta.skathi,
            env!("SK_PKG_VERSION")
        );
    }
    let dir = match &source {
        Source::Bundled(path) | Source::File(path) => {
            path.parent().map(std::path::Path::to_path_buf)
        }
        Source::Embedded | Source::Embedder(_) => None,
    };
    let packs = open_packs(&meta, dir.as_deref()).unwrap_or_else(|e| fatal(&e));
    install_icu(stamped_icu(&payload, &source), &meta, dir.as_deref());
    Some(Loaded {
        payload,
        packs,
        meta,
        source,
        dir: payload_dir(dir.as_deref()),
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
        skathi: s("skathi"),
        host_variant: v
            .get("host")
            .and_then(|h| h.get("variant"))
            .and_then(crate::json::Json::as_str)
            .unwrap_or_default()
            .to_string(),
        publish_devtools: v
            .get("publishDevtools")
            .and_then(crate::json::Json::as_bool),
        launch: v.get("launch").map(crate::json::Json::to_json),
        packs: v
            .get("packs")
            .and_then(crate::json::Json::as_arr)
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let s = |k: &str| {
                    p.get(k)
                        .and_then(crate::json::Json::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                PackRef {
                    name: s("name"),
                    file: s("file"),
                    id: s("id"),
                }
            })
            .collect(),
        intl: s("intl"),
        memory: s("memory"),
        text_rendering: s("textRendering"),
        permissions: s("permissions"),
        runtime_mismatch: v.get("runtime").and_then(runtime_mismatch),
    }
}

/// Why an app whose bundle records `runtime` (`{ version, protocol }`, packages/cli/cli/protocol.ts)
/// cannot run on this host; `None` when it can, or when either side does not know its protocol.
pub(crate) fn runtime_mismatch(runtime: &crate::json::Json) -> Option<String> {
    let field = |k: &str| runtime.get(k).and_then(crate::json::Json::as_str);
    let ours = env!("SK_RUNTIME_PROTOCOL");
    let theirs = field("protocol").filter(|p| !p.is_empty())?;
    if ours.is_empty() || ours == theirs {
        return None;
    }
    Some(format!(
        "this host is Skathi {} (runtime protocol {ours}) and the app was built with \
         @skathi-native/runtime {} (protocol {theirs}): the app would call host functions this host \
         does not have. Use a host and a runtime from the same Skathi build: rebuild the host, or \
         build the app again with the sk of the host's version.",
        env!("SK_PKG_VERSION"),
        field("version").unwrap_or("?"),
    ))
}

/// Tells the user and stops. Before any window exists, so stderr AND a message
/// box: a double-clicked GUI-subsystem exe has no console to print into.
pub fn fatal(msg: &str) -> ! {
    eprintln!("[payload] {msg}");
    message_box(&format!("{msg}\n\nRebuild the app with `sk build`."));
    std::process::exit(3);
}

/// A modal box, unless a harness owns this process (`SK_OWNER_PID`): a test run
/// must never block on a dialog nobody will dismiss.
pub fn message_box(text: &str) {
    if std::env::var_os("SK_OWNER_PID").is_some() {
        return;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let _ = MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("Skathi"),
            MB_OK | MB_ICONERROR,
        );
    }
    #[cfg(not(target_os = "windows"))]
    let _ = text;
}

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
        let res = FindResourceW(None, w!("SKPAYLOAD"), RT_RCDATA);
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

#[cfg(target_os = "linux")]
fn embedded() -> Option<&'static [u8]> {
    use crate::mapped_file::MappedFile;
    static EXE: OnceLock<Option<(MappedFile, std::ops::Range<usize>)>> = OnceLock::new();
    EXE.get_or_init(|| {
        // The running image, even after an update replaced the file at its path.
        let exe = MappedFile::open(std::path::Path::new("/proc/self/exe")).ok()?;
        let range = sk_payload::appended(exe.as_bytes())?;
        Some((exe, range))
    })
    .as_ref()
    .map(|(exe, range)| &exe.as_bytes()[range.clone()])
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn embedded() -> Option<&'static [u8]> {
    None
}

/// signature; a resource of the bundle is covered by it, so like the stamped
/// resource on Windows it wins over `SK_PAYLOAD`.
#[cfg(target_os = "macos")]
fn bundled() -> Option<std::path::PathBuf> {
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        bundle_payload_path(&exe).filter(|path| path.is_file())
    })
    .clone()
}

#[cfg(not(target_os = "macos"))]
fn bundled() -> Option<std::path::PathBuf> {
    None
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn bundle_payload_path(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_app = bundle
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app)
        .then(|| contents.join("Resources").join("app.skpk"))
}

/// Hands the payload's `--shading` effects to the renderers and the CSS registry, before the first
/// window. `SK_EFFECTS_DIR` wins over the payload.
pub fn install_effects() {
    let Some(loaded) = get() else { return };
    let mut manifest = None;
    let mut files = Vec::new();
    for (name, data) in loaded.payload.effects() {
        let Ok(text) = std::str::from_utf8(data) else {
            eprintln!("[prod] the app's effect file {name} is not UTF-8; it is skipped");
            continue;
        };
        if name == "effects.json" {
            manifest = Some(text.to_string());
        } else {
            files.push((name.to_string(), text.to_string()));
        }
    }
    let Some(manifest) = manifest else { return };
    let effects = match gpui::AppEffects::new("the app's payload", manifest, files) {
        Ok(effects) => effects,
        Err(error) => {
            eprintln!("[prod] {error} — the app's own `--shading` effects are unavailable");
            return;
        }
    };
    if !gpui::set_app_effects(effects) && std::env::var_os(gpui::APP_EFFECTS_DIR_ENV).is_some() {
        eprintln!(
            "[prod] {} is set, so the app's own `--shading` effects come from it, not from the payload",
            gpui::APP_EFFECTS_DIR_ENV
        );
    }
}

/// Exact first (a binary search); on a miss, an ASCII-case-insensitive scan with one advisory naming
/// both spellings, since dev runs on NTFS and a packaged exe must not be stricter than the dev host.
/// A plain `fn` for `sk_tree::set_asset_store`, thread-safe because gpui calls [`GpuiAssets`] from
/// its loader threads.
pub fn lookup(key: &str) -> Option<&'static [u8]> {
    let loaded = get()?;
    let all = || std::iter::once(&loaded.payload).chain(loaded.packs.iter().map(|p| &p.payload));
    if let Some(bytes) = all().find_map(|p| p.asset(key)) {
        return Some(bytes);
    }
    let (stored, bytes) = all().find_map(|p| p.asset_ignoring_case(key))?;
    advise_once(
        key,
        &format!(
            "{key:?} matched the packaged asset {stored:?} only by ignoring case — it works (NTFS is \
             case-insensitive and so is this lookup), but the spellings should agree"
        ),
    );
    Some(bytes)
}

pub fn advise_once(key: &str, msg: &str) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    let seen = seen.get_or_insert_with(HashSet::new);
    if seen.len() >= 64 || !seen.insert(key.to_string()) {
        return;
    }
    eprintln!("[skathi compat] {msg}");
    // The ring is per-thread and the main thread's is the one `logs` reads;
    // from a loader thread this lands nowhere, which is why stderr goes first.
    sk_tree::log::record("advisory", msg.to_string());
}

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
            .assets()
            .filter(|e| e.name.starts_with(path))
            .map(|e| gpui::SharedString::from(e.name.to_string()))
            .collect())
    }
}

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
        "sk-host".to_string()
    } else {
        trimmed.to_string()
    }
}

/// After devtools, the same list and order as `ENGINE_FEATURES` in packages/cli/cli/config.ts:
/// `sk build` stamps only hosts that carry all of them.
pub const OPTIONAL: [(&str, bool); 6] = [
    ("devtools", cfg!(sk_devtools)),
    ("audio", cfg!(feature = "audio")),
    ("clipboard", cfg!(feature = "clipboard")),
    ("storages", cfg!(feature = "storages")),
    ("indexeddb", cfg!(feature = "indexeddb")),
    ("webgpu", cfg!(feature = "webgpu")),
];

/// This host's variant: `aot-nojit` or `aot-nojit-devtools` without the JIT, `standard` when nothing optional was left out,
/// else what was, as `no-<part>` joined by `+` in [`OPTIONAL`]'s order. `sk build` keys its host
/// cache by it, and asks for one by it.
pub fn host_variant() -> String {
    if cfg!(feature = "no-jit") {
        return if cfg!(sk_devtools) {
            "aot-nojit-devtools"
        } else {
            "aot-nojit"
        }
        .to_string();
    }
    let off: Vec<String> = OPTIONAL
        .iter()
        .filter(|(_, on)| !on)
        .map(|(name, _)| format!("no-{name}"))
        .collect();
    if off.is_empty() {
        "standard".to_string()
    } else {
        off.join("+")
    }
}

/// `--sk-host-info`: what this exe is, as one JSON line, for `sk build`'s host
/// check. Printed only by an exe with NO payload stamped into it — a packaged
/// app passes its argv to the app untouched.
pub fn host_info_json() -> String {
    use crate::json::Json;
    let mut features: Vec<Json> = Vec::new();
    for (on, name) in [
        (cfg!(feature = "prod"), "prod"),
        (cfg!(feature = "native-aot"), "native-aot"),
        (cfg!(feature = "no-jit"), "no-jit"),
        (cfg!(feature = "no-wasm"), "no-wasm"),
        (cfg!(feature = "no-interpreter"), "no-interpreter"),
        (cfg!(feature = "no-devtools"), "no-devtools"),
        (cfg!(feature = "audio"), "audio"),
        (cfg!(feature = "storages"), "storages"),
        (cfg!(feature = "clipboard"), "clipboard"),
        (cfg!(feature = "indexeddb"), "indexeddb"),
        (cfg!(feature = "webgpu"), "webgpu"),
    ] {
        if on {
            features.push(Json::s(name));
        }
    }
    #[cfg(feature = "native-aot")]
    let aot = crate::native_aot::host_identity();
    #[cfg(not(feature = "native-aot"))]
    let aot = Json::Null;
    Json::obj(vec![
        (
            "mode",
            Json::s(if cfg!(feature = "prod") {
                "prod"
            } else {
                "dev"
            }),
        ),
        ("aot", aot),
        ("skathi", Json::s(env!("SK_PKG_VERSION"))),
        ("protocol", Json::s(env!("SK_RUNTIME_PROTOCOL"))),
        ("crate", Json::s(env!("CARGO_PKG_VERSION"))),
        ("variant", Json::s(host_variant())),
        ("devtools", Json::Bool(cfg!(sk_devtools))),
        ("features", Json::arr(features)),
        ("jscPin", Json::s(env!("SK_JSC_PIN"))),
        ("payloadFormat", Json::n(f64::from(sk_payload::FORMAT))),
        // `root`: `sk build` ships the rest as [`ICU_PACK`], cut from `<package>.dat`.
        (
            "icu",
            Json::obj(vec![
                ("linked", Json::s(redjs::icu::LINKED.as_str())),
                ("package", redjs::icu::package().map_or(Json::Null, Json::s)),
            ]),
        ),
        // What a dev host takes from `sk dev` in place, besides JS modules (`dev::poll_frame`).
        (
            "devReload",
            Json::arr(if cfg!(feature = "prod") {
                Vec::new()
            } else {
                vec![Json::s("effects")]
            }),
        ),
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
    const GOLDEN: &[u8] = include_bytes!("../../../packages/cli/test/fixtures/golden.skpk");

    #[test]
    fn only_an_executable_inside_an_app_bundle_has_a_bundled_payload() {
        use std::path::Path;
        assert_eq!(
            super::bundle_payload_path(Path::new("/Applications/Argon.app/Contents/MacOS/Argon")),
            Some(Path::new("/Applications/Argon.app/Contents/Resources/app.skpk").to_path_buf())
        );
        for exe in [
            "/usr/local/bin/sk-app",
            "/tmp/Contents/MacOS/sk-app",
            "/Applications/Argon.app/Contents/Resources/sk-app",
            "/Applications/Argon.app/sk-app",
        ] {
            assert_eq!(super::bundle_payload_path(Path::new(exe)), None, "{exe}");
        }
    }

    #[test]
    fn mapped_payload_golden_views_borrow_the_mapping_after_owner_moves() {
        let mut files = crate::mapped_file::tests::Files::new();
        let path = files.write("app.skpk", GOLDEN);
        let mapping = crate::mapped_file::MappedFile::open(&path).unwrap();
        let address = mapping.as_bytes().as_ptr() as usize;
        let payload =
            Box::new(super::AppPayload::parse(super::PayloadBytes::File(mapping)).unwrap());
        assert_eq!(
            payload.0.borrow_owner().as_bytes().as_ptr() as usize,
            address
        );
        assert_eq!(payload.len(), GOLDEN.len());
        assert_eq!(payload.asset_count(), 4);
        let script = payload.script().unwrap();
        assert_eq!(script, "globalThis.__golden = 42;\n");
        assert!((address..address + GOLDEN.len()).contains(&(script.as_ptr() as usize)));
        assert_eq!(
            payload.window_json(),
            Some(r#"{"title":"Golden","width":640}"#)
        );
        assert_eq!(payload.asset("assets/hello.txt"), Some(&b"hello\n"[..]));
        assert_eq!(
            payload.asset_ignoring_case("ASSETS/HELLO.TXT").unwrap().0,
            "assets/hello.txt"
        );
        assert_eq!(payload.assets().len(), 4);
        assert_eq!(super::parse_meta(payload.meta().unwrap()).name, "golden");
        drop(payload);
        // A leaked map/handle would deny this in-place write on Windows.
        std::fs::write(&path, b"released").unwrap();
    }

    #[test]
    fn mapped_payload_empty_and_corrupt_parses_release_the_mapping() {
        let mut files = crate::mapped_file::tests::Files::new();
        for (name, bytes) in [("empty.skpk", &b""[..]), ("corrupt.skpk", &[0u8; 64][..])] {
            let path = files.write(name, bytes);
            let mapping = crate::mapped_file::MappedFile::open(&path).unwrap();
            assert!(super::AppPayload::parse(super::PayloadBytes::File(mapping)).is_err());
            std::fs::write(&path, b"released").unwrap();
        }
    }

    #[test]
    fn mapped_payload_version_rejection_releases_the_mapping() {
        let mut files = crate::mapped_file::tests::Files::new();
        let mut bytes = GOLDEN.to_vec();
        let original = sk_payload::Payload::parse(GOLDEN).unwrap();
        let version = super::parse_meta(original.meta().unwrap()).skathi;
        let field = format!("\"skathi\":\"{version}\"");
        let start = bytes
            .windows(field.len())
            .position(|part| part == field.as_bytes())
            .unwrap()
            + "\"skathi\":\"".len();
        bytes[start..start + version.len()].fill(b'x');
        let path = files.write("mismatch.skpk", &bytes);
        let mapping = crate::mapped_file::MappedFile::open(&path).unwrap();
        assert!(
            super::set_external(super::PayloadBytes::File(mapping), "test".into(), None).is_err()
        );
        std::fs::write(&path, b"released").unwrap();
    }

    #[test]
    fn payload_owned_bytes_and_embedded_resources_keep_their_original_allocations() {
        let embedded = super::AppPayload::parse(super::PayloadBytes::Embedded(GOLDEN)).unwrap();
        assert_eq!(
            embedded.0.borrow_owner().as_bytes().as_ptr(),
            GOLDEN.as_ptr()
        );
        let bytes = GOLDEN.to_vec();
        let address = bytes.as_ptr();
        let owned = super::AppPayload::parse(super::PayloadBytes::Owned(bytes)).unwrap();
        assert_eq!(owned.0.borrow_owner().as_bytes().as_ptr(), address);
        assert_eq!(owned.script(), embedded.script());
    }

    #[test]
    fn asset_packs_open_beside_the_payload_and_must_match_their_id() {
        let mut files = crate::mapped_file::tests::Files::new();
        let path = files.write("fonts.skpk", GOLDEN);
        let dir = path.parent().unwrap();
        let id = super::pack_id(
            &super::AppPayload::parse(super::PayloadBytes::Embedded(GOLDEN)).unwrap(),
        );
        assert_eq!(id.len(), 64);
        let meta_with = |file: &str, id: &str| {
            super::parse_meta(&format!(
                r#"{{"name":"a","packs":[{{"name":"fonts","file":"{file}","id":"{id}"}}]}}"#
            ))
        };
        let meta = meta_with("fonts.skpk", &id);
        assert_eq!(
            meta.packs,
            vec![super::PackRef {
                name: "fonts".into(),
                file: "fonts.skpk".into(),
                id: id.clone()
            }]
        );
        let packs = super::open_packs(&meta, Some(dir)).unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].payload.asset_count(), 4);
        drop(packs);
        assert!(super::open_packs(&meta_with("fonts.skpk", &"0".repeat(64)), Some(dir)).is_err());
        for file in ["../fonts.skpk", "sub/fonts.skpk", "C:fonts.skpk", ""] {
            assert!(
                super::open_packs(&meta_with(file, &id), Some(dir)).is_err(),
                "{file}"
            );
        }
        assert!(super::open_packs(&meta_with("missing.skpk", &id), Some(dir)).is_err());
        assert!(super::open_packs(&super::Meta::default(), None)
            .unwrap()
            .is_empty());
        std::fs::write(&path, b"released").unwrap();
    }

    #[test]
    fn meta_is_read_from_the_payload_json() {
        let m = super::parse_meta(
            r#"{"format":1,"name":"hello","productName":"Hello","version":"0.1.0","skathi":"0.0.1","host":{"version":"0.0.1","variant":"no-devtools"},"entry":"src/main.ts","assets":3}"#,
        );
        assert_eq!(m.name, "hello");
        assert_eq!(m.product_name, "Hello");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.host_variant, "no-devtools");
        assert_eq!(super::parse_meta("not json").name, "");
    }

    #[test]
    fn only_a_known_different_runtime_protocol_stops_the_app() {
        let ours = env!("SK_RUNTIME_PROTOCOL");
        let other = if ours == "00000000" {
            "00000001"
        } else {
            "00000000"
        };
        let with = |runtime: &str| {
            super::parse_meta(&format!(r#"{{"name":"a"{runtime}}}"#)).runtime_mismatch
        };
        assert!(with(&format!(
            r#","runtime":{{"version":"9.9.9","protocol":"{other}"}}"#
        ))
        .is_some());
        assert!(with(&format!(
            r#","runtime":{{"version":"9.9.9","protocol":"{ours}"}}"#
        ))
        .is_none());
        assert!(with(r#","runtime":{"version":"9.9.9"}"#).is_none());
        assert!(with("").is_none());
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
            v.get("skathi").and_then(crate::json::Json::as_str),
            Some(env!("SK_PKG_VERSION"))
        );
        let variant = v
            .get("variant")
            .and_then(crate::json::Json::as_str)
            .unwrap();
        assert_eq!(variant, super::host_variant());
        if !cfg!(feature = "no-jit") {
            assert_eq!(
                variant == "standard",
                super::OPTIONAL.iter().all(|(_, on)| *on)
            );
            assert_eq!(variant.contains("no-devtools"), !cfg!(sk_devtools));
        }
        assert_eq!(
            v.get("payloadFormat").and_then(crate::json::Json::as_f64),
            Some(1.0)
        );
    }

    /// A part this host left out must be out of sk-runtime too. Cargo unifies
    /// features, so one crate depending on sk-runtime with its defaults would
    /// put them all back and the variant would name a host it is not.
    #[test]
    fn the_runtime_carries_exactly_the_parts_this_host_names() {
        use sk_runtime::built;
        assert_eq!(built::WEBGPU, cfg!(feature = "webgpu"), "webgpu");
        assert_eq!(built::AUDIO, cfg!(feature = "audio"), "audio");
        assert_eq!(built::STORAGES, cfg!(feature = "storages"), "storages");
        assert_eq!(built::CLIPBOARD, cfg!(feature = "clipboard"), "clipboard");
        assert_eq!(built::INDEXEDDB, cfg!(feature = "indexeddb"), "indexeddb");
    }

    #[test]
    fn a_test_binary_carries_no_payload() {
        // `cargo test` builds an unstamped exe, so the embedded lookup must
        // report absence rather than misread something else.
        assert!(!super::embedded_present());
    }
}
