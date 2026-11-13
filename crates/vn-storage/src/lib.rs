//! The bytes half of Web Storage: the `localStorage` file, its folder, optional encryption at
//! rest, and the `__vn_storage_*` host fns behind `appStorage`. The browser-facing surface and
//! the `StorageHost` trait live in redjs; IndexedDB (feature `indexeddb`) shares this folder
//! and encryption setting.
//!
//! Default folder: `%LOCALAPPDATA%\<app name>\storage` (local, not roaming: DPAPI sealing does
//! not survive roaming). App name, first hit wins: `VN_APP_NAME`, the name given to [`install`],
//! the executable's file stem.
//!
//! Encryption is sticky: a folder that already holds a sealed area keeps that format unless
//! `setStorageFolder(path, { encrypted: false })` says otherwise, so a restart never silently
//! rewrites the data as plaintext.
//!
//! Writes replace the whole file by rename, so a reader never sees a torn file; two processes on
//! one folder are last-writer-wins, so give concurrent dev instances different folders
//! (`VN_APP_NAME`). [`pump`] coalesces writes to one per frame: a hard kill loses at most the
//! last frame, `appStorage.flush()` writes synchronously, and failed writes retry at most once
//! per second.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use vn_runtime::redjs::webapi::storage::{self as api, StorageHost};
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{host_fn, JsRuntime};

pub mod cookies;
pub mod crypt;
pub mod file;
pub mod folders;
pub mod http_cache;
#[cfg(feature = "indexeddb")]
pub mod idb;

pub use folders::{user_folder, KINDS};

#[derive(Default)]
struct State {
    folder: PathBuf,
    encrypted: bool,
    /// Whether the app said which format it wants. Makes encryption sticky: a restarted host
    /// starts with `encrypted: false` and must not rewrite a sealed area as plaintext.
    encryption_explicit: bool,
    /// The area as last loaded or stored; re-written on a folder or format change.
    snapshot: Vec<(String, String)>,
    /// Set once `load` has run; before that this process must not write.
    loaded: bool,
    dirty: bool,
    /// Persistent I/O failures must not re-encrypt and fsync on every frame.
    retry_after: Option<Instant>,
    error: Option<String>,
    /// Old-format file to remove once the new one is written.
    retire: Option<PathBuf>,
}

/// The file-backed `localStorage` for this process.
pub struct FileStorage {
    state: Mutex<State>,
}

/// The one backend (`OnceLock`: a `PathBuf` cannot be built in a const context).
fn storage() -> &'static FileStorage {
    static S: OnceLock<FileStorage> = OnceLock::new();
    S.get_or_init(|| FileStorage { state: Mutex::new(State::default()) })
}

impl FileStorage {
    fn note(&self, st: &mut State, what: String) {
        eprintln!("[storage] {what}");
        st.error = Some(what);
    }

    /// Reads the area from `folder` in whichever format is there. `migrate` is true when it came
    /// from the other format and must be re-written.
    fn read_folder(&self, st: &mut State) -> (Vec<(String, String)>, bool) {
        let plain = file::plain_path(&st.folder);
        let sealed = file::sealed_path(&st.folder);

        // Sticky encryption: an existing sealed area decides the format.
        if !st.encryption_explicit && !st.encrypted && sealed.exists() {
            eprintln!(
                "[storage] {} is encrypted and nothing asked for plaintext — keeping it encrypted",
                sealed.display()
            );
            st.encrypted = true;
        }

        let (want, other) =
            if st.encrypted { (&sealed, &plain) } else { (&plain, &sealed) };

        match self.read_one(st, want) {
            Some(entries) => (entries, false),
            None => {
                // Only the other format is present: a migration, not an empty area.
                match self.read_one(st, other) {
                    Some(entries) => {
                        eprintln!(
                            "[storage] migrating {} into {} ({})",
                            other.display(),
                            want.display(),
                            if st.encrypted { "encrypting" } else { "decrypting" }
                        );
                        st.retire = Some(other.clone());
                        (entries, true)
                    }
                    None => (Vec::new(), false),
                }
            }
        }
    }

    /// `None` if the file is absent. An unreadable file is an empty area plus an advisory.
    fn read_one(&self, st: &mut State, path: &Path) -> Option<Vec<(String, String)>> {
        let sealed = path.extension().is_some_and(|e| e == "sealed");
        let raw = std::fs::read(path).ok()?;
        let text = if sealed {
            match crypt::provider() {
                None => {
                    self.note(
                        st,
                        format!(
                            "{} is encrypted and this platform has no decryption provider — \
                             starting empty, and NOT overwriting it",
                            path.display()
                        ),
                    );
                    return Some(Vec::new());
                }
                Some(p) => match file::unseal(p, &raw) {
                    Ok(t) => t,
                    Err(e) => {
                        self.note(
                            st,
                            format!("could not decrypt {}: {e} — starting empty", path.display()),
                        );
                        return Some(Vec::new());
                    }
                },
            }
        } else {
            match String::from_utf8(raw) {
                Ok(t) => t,
                Err(e) => {
                    self.note(st, format!("{} is not UTF-8: {e} — starting empty", path.display()));
                    return Some(Vec::new());
                }
            }
        };
        match file::decode(&text) {
            Ok(entries) => Some(entries),
            Err(e) => {
                self.note(st, format!("could not read {}: {e} — starting empty", path.display()));
                Some(Vec::new())
            }
        }
    }

    /// Automatic retries are bounded; explicit flush/relocation bypass this delay.
    fn write_due(&self, st: &mut State, now: Instant) -> Result<bool, String> {
        if st.retry_after.is_some_and(|retry| now < retry) {
            return Ok(false);
        }
        let result = self.write_now(st);
        if result.is_err() {
            st.retry_after = Some(now + Duration::from_secs(1));
        }
        result
    }

    /// Writes the staged snapshot; `Ok(false)` if nothing was staged.
    fn write_now(&self, st: &mut State) -> Result<bool, String> {
        if !st.dirty {
            return Ok(false);
        }
        // Keep the snapshot pending until persistence and plaintext retirement succeed.
        let json = file::encode(&st.snapshot);
        let (path, bytes) = if st.encrypted {
            let p = crypt::provider().ok_or_else(|| {
                String::from("this platform has no encryption provider")
            })?;
            (file::sealed_path(&st.folder), file::seal(p, &json)?)
        } else {
            (file::plain_path(&st.folder), json.into_bytes())
        };
        file::write_atomic(&path, &bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        // Only once the new file is safely on disk.
        if let Some(old) = &st.retire {
            match std::fs::remove_file(old) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("could not retire migrated {}: {error}", old.display())),
            }
            st.retire = None;
        }
        st.dirty = false;
        st.retry_after = None;
        st.error = None;
        Ok(true)
    }
}

impl StorageHost for FileStorage {
    fn load(&self) -> Vec<(String, String)> {
        let mut st = self.state.lock().unwrap();
        if st.folder.as_os_str().is_empty() {
            st.folder = default_folder(None);
        }
        let (entries, migrate) = self.read_folder(&mut st);
        st.snapshot = entries.clone();
        st.loaded = true;
        // A migration is a write owed to disk even if the app never calls `setItem`.
        st.dirty = migrate;
        entries
    }

    fn store(&self, entries: &[(String, String)]) {
        let mut st = self.state.lock().unwrap();
        st.snapshot = entries.to_vec();
        st.loaded = true;
        st.dirty = true;
    }

    fn describe(&self) -> String {
        let st = self.state.lock().unwrap();
        st.folder.display().to_string()
    }
}

/// `%LOCALAPPDATA%\<app name>\storage`.
pub fn default_folder(app_name: Option<&str>) -> PathBuf {
    let name = resolve_app_name(app_name);
    match folders::user_folder("LOCALAPPDATA", &name) {
        Ok(p) => p.join("storage"),
        Err(e) => {
            // No known-folder API and no HOME: beside the exe beats no storage.
            eprintln!("[storage] no local app data folder ({e}); falling back beside the exe");
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."))
                .join(format!("{name}-storage"))
        }
    }
}

pub(crate) fn resolve_app_name(from_embedder: Option<&str>) -> String {
    let env = std::env::var("VN_APP_NAME").ok();
    let stem = exe_stem().unwrap_or_default();
    pick_app_name(env.as_deref(), from_embedder, &stem)
}

/// The program's file stem. Under the dynamic loader (`ld-linux-x86-64.so.2 ./Argon`)
/// `current_exe` is the loader, so `argv[0]` names the program.
pub(crate) fn exe_stem() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let program = if cfg!(target_os = "linux") && is_dynamic_loader(&exe) {
        PathBuf::from(std::env::args_os().next()?)
    } else {
        exe
    };
    program.file_stem().map(|s| s.to_string_lossy().into_owned())
}

fn is_dynamic_loader(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| ["ld-linux", "ld64.so", "ld-musl"].iter().any(|p| name.starts_with(p)))
}

/// The precedence rule on its own, testable without touching the environment.
fn pick_app_name(env: Option<&str>, embedder: Option<&str>, exe_stem: &str) -> String {
    for candidate in [env, embedder, Some(exe_stem)] {
        if let Some(n) = candidate.filter(|n| !n.is_empty()) {
            return sanitize(n);
        }
    }
    sanitize("")
}

/// A folder name, not an app's display name: no separators, no drive letters,
/// no Windows-reserved characters, and never empty.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
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
        String::from("vue-native-app")
    } else {
        trimmed.to_string()
    }
}

/// Points the backend at `folder` (created) and sets encryption; `None` leaves the format as
/// is, so a folder change is never a silent decryption. Safe at any time: once loaded, the
/// in-memory area is re-written into the new place and format.
pub fn set_storage_folder(folder: &Path, encrypted: Option<bool>) -> Result<PathBuf, String> {
    if encrypted == Some(true) && crypt::provider().is_none() {
        return Err(format!(
            "encrypted storage is not available on this platform — refusing to write plaintext \
             into {} after being asked for encryption",
            folder.display()
        ));
    }
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    // Canonicalise to resolve `..`; strip the `\\?\` prefix, which is noise in advisories.
    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let folder = strip_verbatim(&folder);

    let r = relocate(&folder, encrypted);
    // IndexedDB latches its folder and key at first use; later changes apply at next start.
    // Outside the State lock: the IndexedDB host takes it from its own thread.
    #[cfg(feature = "indexeddb")]
    idb::storage_changed(&folder, encrypted);
    // The HTTP cache follows encryption at once; it has nothing to migrate.
    http_cache::storage_changed(encrypted);
    r.map(|()| folder)
}

/// The `localStorage` half of [`set_storage_folder`].
fn relocate(folder: &Path, encrypted: Option<bool>) -> Result<(), String> {
    let folder = folder.to_path_buf();
    let s = storage();
    let mut st = s.state.lock().unwrap();
    let same_folder = st.folder == folder;
    let was_encrypted = st.encrypted;
    // `None` keeps the format under the sticky rule: take on a sealed area in the new folder
    // as `read_folder` would.
    let encrypted = match encrypted {
        Some(e) => {
            st.encryption_explicit = true;
            e
        }
        None => was_encrypted || (!st.encryption_explicit && file::sealed_path(&folder).exists()),
    };
    let moved = !same_folder || was_encrypted != encrypted;
    st.folder = folder.clone();
    if moved { st.retry_after = None; }
    st.encrypted = encrypted;
    st.error = None;
    if moved && st.loaded {
        // Flipping encryption in place retires the old-format file. Moving to another folder
        // leaves the old one: it is not this function's data to delete.
        if same_folder && was_encrypted != encrypted {
            st.retire = Some(if was_encrypted {
                file::sealed_path(&folder)
            } else {
                file::plain_path(&folder)
            });
        }
        st.dirty = true;
        s.write_now(&mut st).map_err(|e| {
            st.error = Some(e.clone());
            e
        })?;
    }
    Ok(())
}

/// The configured folder, else the default `install` would set.
pub(crate) fn current_folder() -> PathBuf {
    let folder = storage().state.lock().unwrap().folder.clone();
    if folder.as_os_str().is_empty() {
        default_folder(embedder_app_name().as_deref())
    } else {
        folder
    }
}

/// `Some(encrypted)` once the app called `setStorageFolder`; `None` leaves it to the sticky rule.
#[cfg_attr(not(feature = "indexeddb"), allow(dead_code))]
fn encryption_intent() -> Option<bool> {
    let st = storage().state.lock().unwrap();
    st.encryption_explicit.then_some(st.encrypted)
}

/// `\\?\C:\x` → `C:\x`. Only ever applied to a path we just canonicalised.
fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.len() > 2 && rest.as_bytes()[1] == b':' => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

/// Converts an existing localStorage file without loading the backend. The source is removed
/// only after a verified sealed file exists. Call before any process opens the profile.
pub fn encrypt_local_storage_at(folder: &Path) -> Result<bool, String> {
    let provider = crypt::provider().ok_or("localStorage encryption is unavailable on this platform")?;
    encrypt_local_storage_with(folder, provider)
}

fn encrypt_local_storage_with(folder: &Path, provider: &dyn crypt::SecretProvider) -> Result<bool, String> {
    if !folder.is_absolute() {
        return Err("localStorage encryption requires an absolute profile directory".into());
    }
    let plain = file::plain_path(folder);
    let sealed = file::sealed_path(folder);
    let read_sealed = |bytes: &[u8]| -> Result<Vec<(String, String)>, String> {
        file::decode(&file::unseal(provider, bytes)?)
    };
    let mut changed = false;
    match std::fs::read(&sealed) {
        Ok(bytes) => {
            // Match the backend's precedence: an existing sealed area is authoritative.
            read_sealed(&bytes).map_err(|e| format!("verifying {}: {e}", sealed.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let source = match std::fs::read(&plain) {
                Ok(source) => source,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(format!("reading {}: {error}", plain.display())),
            };
            let text = std::str::from_utf8(&source).map_err(|e| format!("reading {}: {e}", plain.display()))?;
            let entries = file::decode(text).map_err(|e| format!("reading {}: {e}", plain.display()))?;
            let bytes = file::seal(provider, &file::encode(&entries))?;
            file::write_atomic(&sealed, &bytes)
                .map_err(|e| format!("sealing {}: {e}", plain.display()))?;
            let written = std::fs::read(&sealed).map_err(|e| format!("verifying {}: {e}", sealed.display()))?;
            if read_sealed(&written)? != entries {
                return Err("encrypted localStorage did not round-trip; keeping plaintext source".into());
            }
            changed = true;
        }
        Err(error) => return Err(format!("reading {}: {error}", sealed.display())),
    }
    match std::fs::remove_file(&plain) {
        Ok(()) => changed = true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not retire plaintext {}: {error}", plain.display())),
    }
    Ok(changed)
}

/// Writes the staged area now (behind `appStorage.flush()`).
pub fn flush() -> Result<bool, String> {
    let s = storage();
    let mut st = s.state.lock().unwrap();
    let r = s.write_now(&mut st);
    if let Err(e) = &r {
        st.error = Some(e.clone());
    }
    r
}

/// One coalesced write per frame; call beside `vn_permissions::pump`.
pub fn pump() {
    cookies::pump();
    // Common case is nothing changed: one uncontended lock.
    let s = storage();
    let mut st = s.state.lock().unwrap();
    if !st.dirty {
        return;
    }
    if let Err(e) = s.write_due(&mut st, Instant::now()) {
        // Log once per distinct message.
        if st.error.as_deref() != Some(e.as_str()) {
            eprintln!("[storage] {e}");
        }
        st.error = Some(e);
    }
}

pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn err_json(msg: &str) -> String {
    format!("{{\"error\":\"{}\"}}", json_escape(msg))
}

// `__vn_storage_user_folder(kind, subfolder) -> json`: `{"path"}` or `{"error"}`. Creates nothing.
host_fn!(fn vn_storage_user_folder(g, args) {
    let kind = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let sub = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let json = match folders::user_folder(&kind, &sub) {
        Ok(p) => format!("{{\"path\":\"{}\"}}", json_escape(&p.display().to_string())),
        Err(e) => err_json(&e),
    };
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

// `__vn_storage_set_folder(path, encrypted) -> json`; `encrypted` is "true", "false" or "keep"
// (the app did not say).
host_fn!(fn vn_storage_set_folder(g, args) {
    let path = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let encrypted = match unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    let json = if path.trim().is_empty() {
        err_json("setStorageFolder needs a path")
    } else {
        match set_storage_folder(Path::new(&path), encrypted) {
            Ok(p) => format!("{{\"path\":\"{}\"}}", json_escape(&p.display().to_string())),
            Err(e) => err_json(&e),
        }
    };
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

// `__vn_storage_info() -> json`: folder, format, file size, pending write, last error.
host_fn!(fn vn_storage_info(_g, _args) {
    let st = storage().state.lock().unwrap();
    let folder = if st.folder.as_os_str().is_empty() {
        default_folder(embedder_app_name().as_deref())
    } else {
        st.folder.clone()
    };
    let path = if st.encrypted { file::sealed_path(&folder) } else { file::plain_path(&folder) };
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let provider = crypt::provider().map(|p| p.name()).unwrap_or("");
    #[cfg(feature = "indexeddb")]
    let indexed_db = idb::info_json(&folder);
    #[cfg(not(feature = "indexeddb"))]
    let indexed_db = "null";
    let cookie_jar = cookies::info_json();
    let json = format!(
        "{{\"folder\":\"{}\",\"file\":\"{}\",\"encrypted\":{},\"bytes\":{},\"loaded\":{},\
          \"pending\":{},\"provider\":\"{}\",\"kinds\":[{}],\"error\":{},\"indexedDB\":{},\"cookies\":{}}}",
        json_escape(&folder.display().to_string()),
        json_escape(&path.display().to_string()),
        st.encrypted,
        bytes,
        st.loaded,
        st.dirty,
        json_escape(provider),
        KINDS.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(","),
        match &st.error {
            Some(e) => format!("\"{}\"", json_escape(e)),
            None => String::from("null"),
        },
        indexed_db,
        cookie_jar,
    );
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

// `__vn_storage_flush() -> json` — `{"wrote":true|false}` or `{"error":"…"}`.
host_fn!(fn vn_storage_flush(_g, _args) {
    let json = match flush() {
        Ok(wrote) => format!("{{\"wrote\":{wrote}}}"),
        Err(e) => err_json(&e),
    };
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

static APP_NAME: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn embedder_app_name() -> Option<String> {
    APP_NAME.lock().unwrap().clone()
}

/// Installs the backend into redjs and registers the extension surface. `app_name` is the
/// embedder's name for the app (`None` in prod: the exe name). Call before any app code: the
/// area loads lazily, and a late install would hand the app an empty `localStorage`.
pub fn install(rt: &JsRuntime, app_name: Option<String>) {
    *APP_NAME.lock().unwrap() = app_name.clone();
    {
        let mut st = storage().state.lock().unwrap();
        if st.folder.as_os_str().is_empty() {
            st.folder = default_folder(app_name.as_deref());
        }
    }
    api::set_host(storage());
    // Load the cookie jar from the current folder before anything can set a cookie.
    cookies::install(current_folder());
    // The IndexedDB host before app code: redjs reads its host once, at first use.
    #[cfg(feature = "indexeddb")]
    idb::install();
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_storage_user_folder", 2, vn_storage_user_folder as *const _),
        ("__vn_storage_set_folder", 2, vn_storage_set_folder as *const _),
        ("__vn_storage_info", 0, vn_storage_info as *const _),
        ("__vn_storage_flush", 0, vn_storage_flush as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_failed_sealed_write_or_plaintext_retirement_keeps_the_snapshot_pending() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = std::env::temp_dir().join(format!("vn-sealed-retry-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&base).unwrap();
        let entries = vec![("preference".to_string(), "preserved".to_string())];
        let plain = file::plain_path(&base);
        let sealed = file::sealed_path(&base);
        std::fs::write(&plain, file::encode(&entries)).unwrap();
        let backend = FileStorage { state: Mutex::new(State::default()) };
        let mut state = State { folder: base.clone(), encrypted: true, loaded: true,
            dirty: true, snapshot: entries.clone(), retire: Some(plain.clone()), ..State::default() };
        std::fs::create_dir(&sealed).unwrap();
        let now = Instant::now();
        assert!(backend.write_due(&mut state, now).is_err());
        assert!(state.dirty && plain.exists());
        std::fs::remove_dir(&sealed).unwrap();
        assert!(!backend.write_due(&mut state, now + Duration::from_millis(500)).unwrap());
        assert!(!sealed.exists());
        let locked = std::fs::OpenOptions::new().read(true).share_mode(1).open(&plain).unwrap();
        assert!(backend.write_due(&mut state, now + Duration::from_secs(1)).is_err());
        assert!(state.dirty && state.retire.is_some() && plain.exists());
        drop(locked);
        assert!(!backend.write_due(&mut state, now + Duration::from_millis(1500)).unwrap());
        assert!(plain.exists());
        assert!(backend.write_now(&mut state).unwrap());
        assert!(!state.dirty && state.retry_after.is_none() && state.retire.is_none() && !plain.exists());
        let bytes = std::fs::read(&sealed).unwrap();
        assert_eq!(file::decode(&file::unseal(crypt::provider().unwrap(), &bytes).unwrap()).unwrap(), entries);
        std::fs::remove_file(sealed).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn startup_encryption_preserves_values_and_retires_plaintext_only_after_verification() {
        struct Protector;
        impl crypt::SecretProvider for Protector {
            fn name(&self) -> &'static str { "test envelope" }
            fn id(&self) -> u8 { 98 }
            fn protect(&self, bytes: &[u8]) -> Result<Vec<u8>, String> { Ok(bytes.iter().map(|byte| byte ^ 0x5a).collect()) }
            fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>, String> { self.protect(bytes) }
        }
        let base = std::env::temp_dir().join(format!("vn-encrypt-startup-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&base).unwrap();
        let entries = vec![("settings".to_string(), "private value π".to_string())];
        std::fs::write(file::plain_path(&base), file::encode(&entries)).unwrap();
        assert!(encrypt_local_storage_with(&base, &Protector).unwrap());
        assert!(!file::plain_path(&base).exists());
        let sealed = std::fs::read(file::sealed_path(&base)).unwrap();
        assert_eq!(file::decode(&file::unseal(&Protector, &sealed).unwrap()).unwrap(), entries);
        assert!(!sealed.windows(b"private value".len()).any(|bytes| bytes == b"private value"));
        assert!(!encrypt_local_storage_with(&base, &Protector).unwrap());
        std::fs::write(file::plain_path(&base), file::encode(&[("stale".into(), "old".into())])).unwrap();
        assert!(encrypt_local_storage_with(&base, &Protector).unwrap());
        assert_eq!(std::fs::read(file::sealed_path(&base)).unwrap(), sealed);
        std::fs::write(file::plain_path(&base), file::encode(&entries)).unwrap();
        std::fs::write(file::sealed_path(&base), b"broken ciphertext").unwrap();
        assert!(encrypt_local_storage_with(&base, &Protector).is_err());
        assert!(file::plain_path(&base).is_file());
        std::fs::remove_file(file::plain_path(&base)).unwrap();
        std::fs::remove_file(file::sealed_path(&base)).unwrap();
        std::fs::write(file::plain_path(&base), b"broken json").unwrap();
        assert!(encrypt_local_storage_with(&base, &Protector).is_err());
        assert_eq!(std::fs::read(file::plain_path(&base)).unwrap(), b"broken json");
        assert!(!file::sealed_path(&base).exists());
        std::fs::remove_file(file::plain_path(&base)).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn an_app_name_becomes_a_folder_name_that_windows_will_accept() {
        assert_eq!(sanitize("hello"), "hello");
        assert_eq!(sanitize("vue-native — hello"), "vue-native---hello");
        assert_eq!(sanitize("a/b\\c:d"), "a-b-c-d");
        assert_eq!(sanitize("My App 1.0"), "My-App-1.0");
        // No leading dot (hidden folder) or trailing dot (Windows drops it).
        assert_eq!(sanitize(""), "vue-native-app");
        assert_eq!(sanitize("///"), "vue-native-app");
        assert_eq!(sanitize(".hidden."), "hidden");
        assert_eq!(sanitize(&"x".repeat(200)).len(), 64);
    }

    #[test]
    fn the_env_override_wins_over_the_embedder_and_the_exe() {
        assert_eq!(pick_app_name(Some("pinned"), Some("config"), "vn-app"), "pinned");
        assert_eq!(pick_app_name(None, Some("config"), "vn-app"), "config");
        assert_eq!(pick_app_name(Some(""), Some("config"), "vn-app"), "config");
        assert_eq!(pick_app_name(None, None, "my-shipped-app"), "my-shipped-app");
        assert_eq!(pick_app_name(None, None, ""), "vue-native-app");
        assert!(!resolve_app_name(None).is_empty());
    }

    #[test]
    fn the_default_folder_is_local_app_data_and_ends_in_storage() {
        let p = default_folder(Some("unit-test-app"));
        assert!(p.is_absolute(), "{p:?}");
        assert!(p.ends_with("storage"), "{p:?}");
        assert!(
            p.to_string_lossy().contains("unit-test-app"),
            "the app name has to be in the path: {p:?}"
        );
        if cfg!(target_os = "windows") {
            let local = folders::user_folder("LOCALAPPDATA", "").unwrap();
            assert!(p.starts_with(&local), "{p:?} is not under {local:?}");
        }
    }

    #[test]
    fn json_escaping_survives_a_windows_path_and_a_quote() {
        assert_eq!(json_escape(r"C:\Users\x"), r"C:\\Users\\x");
        assert_eq!(json_escape("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
        assert!(err_json(r"C:\a").starts_with("{\"error\":\"C:\\\\a\""));
    }

    struct Temp(PathBuf);
    impl Temp {
        fn new(tag: &str) -> Temp {
            let p = std::env::temp_dir().join(format!("vn-storage-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Temp(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// One backend per process, so tests that drive it run one at a time; poison-tolerant.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cold() {
        let mut st = storage().state.lock().unwrap();
        *st = State::default();
    }

    fn cycle(folder: &Path, encrypted: bool, entries: &[(String, String)]) {
        set_storage_folder(folder, Some(encrypted)).expect("set folder");
        let _ = storage().load();
        storage().store(entries);
        flush().expect("flush");
    }

    fn read_back(folder: &Path, encrypted: bool) -> Vec<(String, String)> {
        cold();
        set_storage_folder(folder, Some(encrypted)).expect("set folder");
        storage().load()
    }

    fn read_back_unconfigured(folder: &Path) -> Vec<(String, String)> {
        cold();
        storage().state.lock().unwrap().folder = folder.to_path_buf();
        storage().load()
    }

    #[test]
    fn an_area_survives_a_restart_of_the_backend() {
        let _g = serial();
        let t = Temp::new("persist");
        let entries = vec![("theme".to_string(), "dark".to_string())];
        cycle(&t.0, false, &entries);
        assert!(file::plain_path(&t.0).exists(), "the file was written");
        assert_eq!(read_back(&t.0, false), entries);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn an_encrypted_area_round_trips_and_the_bytes_on_disk_are_not_plaintext() {
        let _g = serial();
        let t = Temp::new("sealed");
        let entries = vec![("token".to_string(), "s3cr3t-value".to_string())];
        cold();
        cycle(&t.0, true, &entries);
        let path = file::sealed_path(&t.0);
        assert!(path.exists(), "the sealed file was written");
        assert!(!file::plain_path(&t.0).exists(), "no plaintext file alongside it");
        let raw = std::fs::read(&path).unwrap();
        assert!(
            !raw.windows(12).any(|w| w == b"s3cr3t-value"),
            "the secret is legible on disk"
        );
        assert_eq!(read_back(&t.0, true), entries);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn turning_encryption_on_migrates_the_existing_plaintext_area() {
        let _g = serial();
        let t = Temp::new("migrate");
        let entries = vec![("keep".to_string(), "me".to_string())];
        cold();
        cycle(&t.0, false, &entries);
        assert!(file::plain_path(&t.0).exists());

        assert_eq!(read_back(&t.0, true), entries, "the data came across");
        assert!(flush().expect("the migration write"), "the migration owes a write");
        assert!(file::sealed_path(&t.0).exists(), "sealed file written");
        assert!(
            !file::plain_path(&t.0).exists(),
            "the plaintext copy must not be left behind next to the encrypted one"
        );
        assert_eq!(read_back(&t.0, true), entries);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn flipping_encryption_while_the_area_is_live_also_retires_the_old_file() {
        let _g = serial();
        let t = Temp::new("flip");
        let entries = vec![("k".to_string(), "v".to_string())];
        cold();
        cycle(&t.0, false, &entries);
        set_storage_folder(&t.0, Some(true)).expect("flip to encrypted");
        assert!(file::sealed_path(&t.0).exists());
        assert!(!file::plain_path(&t.0).exists(), "the plaintext copy is retired");
        assert_eq!(read_back(&t.0, true), entries);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_restart_that_says_nothing_keeps_an_encrypted_area_encrypted() {
        let _g = serial();
        let t = Temp::new("sticky");
        let entries = vec![("secret".to_string(), "value".to_string())];
        cold();
        cycle(&t.0, true, &entries);
        assert!(file::sealed_path(&t.0).exists());

        assert_eq!(read_back_unconfigured(&t.0), entries, "the data came back");
        assert!(storage().state.lock().unwrap().encrypted, "and it is still encrypted");
        assert!(flush().is_ok());
        assert!(!file::plain_path(&t.0).exists(), "nothing was written in plaintext");
        assert!(file::sealed_path(&t.0).exists());

        assert_eq!(read_back(&t.0, false), entries);
        assert!(flush().expect("the decryption write"));
        assert!(file::plain_path(&t.0).exists());
        assert!(!file::sealed_path(&t.0).exists(), "the sealed copy is retired");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_folder_change_that_says_nothing_about_encryption_keeps_the_format() {
        let _g = serial();
        let a = Temp::new("keep-a");
        let b = Temp::new("keep-b");
        let entries = vec![("secret".to_string(), "value".to_string())];
        cold();
        cycle(&a.0, true, &entries);

        set_storage_folder(&a.0, None).expect("same folder");
        assert!(storage().state.lock().unwrap().encrypted);
        assert!(file::sealed_path(&a.0).exists());
        assert!(!file::plain_path(&a.0).exists(), "nothing was decrypted");

        set_storage_folder(&b.0, None).expect("move");
        assert!(file::sealed_path(&b.0).exists());
        assert!(!file::plain_path(&b.0).exists(), "the move did not decrypt");

        cold();
        set_storage_folder(&b.0, None).expect("cold, unsaid");
        assert_eq!(storage().load(), entries);
        let st = storage().state.lock().unwrap();
        assert!(st.encrypted, "the sealed area kept its format");
        assert!(!st.encryption_explicit, "and nothing was recorded as the app's choice");
    }

    #[test]
    fn a_folder_change_that_says_nothing_keeps_an_explicit_plaintext_choice() {
        let _g = serial();
        let t = Temp::new("keep-plain");
        let entries = vec![("k".to_string(), "v".to_string())];
        cold();
        cycle(&t.0, false, &entries);
        set_storage_folder(&t.0, None).expect("same folder");
        let st = storage().state.lock().unwrap();
        assert!(!st.encrypted && st.encryption_explicit);
        drop(st);
        assert!(file::plain_path(&t.0).exists());
        assert!(!file::sealed_path(&t.0).exists());
    }

    #[test]
    fn a_corrupt_file_is_an_empty_area_and_a_recorded_error_not_a_panic() {
        let _g = serial();
        let t = Temp::new("corrupt");
        std::fs::create_dir_all(&t.0).unwrap();
        std::fs::write(file::plain_path(&t.0), b"{ this is not our file").unwrap();
        assert_eq!(read_back(&t.0, false), Vec::<(String, String)>::new());
        let err = storage().state.lock().unwrap().error.clone();
        assert!(err.is_some(), "the failure has to be visible in info()");
        assert!(err.unwrap().contains("could not read"), "and it has to say what happened");
    }

    #[test]
    fn moving_the_folder_after_the_area_is_live_takes_the_data_with_it() {
        let _g = serial();
        let a = Temp::new("move-a");
        let b = Temp::new("move-b");
        let entries = vec![("k".to_string(), "v".to_string())];
        cold();
        cycle(&a.0, false, &entries);
        set_storage_folder(&b.0, Some(false)).expect("move");
        assert_eq!(read_back(&b.0, false), entries);
        assert!(file::plain_path(&a.0).exists(), "the old folder is left alone");
    }

    #[test]
    fn a_folder_that_cannot_be_written_reports_rather_than_pretending() {
        let _g = serial();
        let t = Temp::new("blocked");
        std::fs::create_dir_all(&t.0).unwrap();
        let blocked = t.0.join("not-a-dir");
        std::fs::write(&blocked, b"x").unwrap();
        let e = set_storage_folder(&blocked, Some(false)).unwrap_err();
        assert!(e.contains("could not create"), "{e}");
    }

    #[test]
    fn startup_encryption_under_the_keychain_envelope_round_trips() {
        let p = crypt::envelope::FixedKey::new(8);
        let t = Temp::new("envelope-startup");
        std::fs::create_dir_all(&t.0).unwrap();
        let base = std::fs::canonicalize(&t.0).unwrap();
        let entries = vec![("session".to_string(), "private-token".to_string())];
        std::fs::write(file::plain_path(&base), file::encode(&entries)).unwrap();
        assert!(encrypt_local_storage_with(&base, &p).unwrap());
        assert!(!file::plain_path(&base).exists());
        let sealed = std::fs::read(file::sealed_path(&base)).unwrap();
        assert_eq!(&sealed[..8], b"VNSTOR\x01\x02");
        assert!(!sealed.windows(13).any(|w| w == b"private-token"));
        assert_eq!(file::decode(&file::unseal(&p, &sealed).unwrap()).unwrap(), entries);
        let other = crypt::envelope::FixedKey::new(9);
        assert!(encrypt_local_storage_with(&base, &other).unwrap_err().contains("different storage key"));
        assert_eq!(std::fs::read(file::sealed_path(&base)).unwrap(), sealed);
    }

    #[test]
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    fn asking_for_encryption_without_a_provider_is_refused_not_silently_plaintext() {
        let _g = serial();
        let t = Temp::new("no-provider");
        let e = set_storage_folder(&t.0, Some(true)).unwrap_err();
        assert!(e.contains("not available on this platform"), "{e}");
    }
}
