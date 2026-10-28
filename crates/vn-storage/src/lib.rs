//! vn-storage — the bytes half of vue-native's Web Storage.
//!
//! # What is where, and why
//!
//! The browser-shaped surface (`localStorage`, `sessionStorage`, `Storage`,
//! `StorageEvent`, string coercion, the quota, `QuotaExceededError`) lives in
//! **redjs** (`redjs::webapi::storage`), because it is browser semantics and
//! redjs is where browser semantics live. So does the [`StorageHost`] trait it
//! reads through — a trait redjs consumes has to be declared by redjs.
//!
//! This crate is the other half: the file on the disk, the folder it lives in,
//! the optional encryption, and the small `__vn_storage_*` host-fn surface
//! behind vue-native's own `appStorage` extension. The same split
//! `vn-permissions` has, and for the same reason — it keeps redjs shippable to
//! a consumer (PEWPEW) that has no `%LOCALAPPDATA%` and no opinion about DPAPI.
//!
//! ```text
//!   apps/hello  ──import { appStorage } from "vue-native"──┐
//!                                                          │  tier 2: extension
//!   packages/vue-native/src/runtime.js ──__vn_storage_*────┤  (folders, encryption)
//!                                                          │
//!   apps/*      ──localStorage.setItem(…)──────────────────┤  tier 1: Web Storage
//!                                                          │
//!   redjs::webapi::storage  ── dyn StorageHost ────────────┘
//!                                    ▲
//!                            vn_storage::FileStorage
//!                                    │
//!                     <folder>/localStorage.json   (plaintext)
//!                     <folder>/localStorage.sealed (DPAPI)
//!
//!   apps/*      ──indexedDB.open(…)── redjs::webapi::indexeddb (SQLite)
//!                                    ▲  dyn IndexedDbHost
//!                            vn_storage::idb::Host   (feature `indexeddb`)
//!                                    │
//!                     <folder>/indexeddb/<h>.sqlite3   (ChaCha20-Poly1305
//!                     <folder>/indexeddb/key.sealed     under a DPAPI-sealed key
//!                                                       when the folder is encrypted)
//! ```
//!
//! IndexedDB follows the SAME folder and the SAME encryption setting, with the
//! same sticky rule — see [`idb`] (compiled with the `indexeddb` feature).
//!
//! # The default folder, and how the app name is found
//!
//! An app that never calls `appStorage.setStorageFolder` gets
//! `%LOCALAPPDATA%\<app name>\storage`.
//!
//! **LOCAL**, not roaming: a `localStorage` area is a cache of one machine's
//! session state — a scroll position, a collapsed sidebar, a JWT that is
//! probably expired by the time the profile lands on another machine — and
//! roaming it is the case where DPAPI encryption stops working (see
//! [`crypt`]'s roaming caveat). An app that genuinely wants its data to follow
//! the user says so with one call to `getUserFolder("APPDATA", …)`.
//!
//! The app name is resolved in this order, first hit wins:
//!
//! 1. `VN_APP_NAME` — the override. This is what isolates two dev hosts that
//!    would otherwise share a folder, and what the storage-persistence test
//!    pins so it cannot touch a real app's data.
//! 2. Whatever the embedder passed to [`install`] — in vue-native, the `name`
//!    field of `apps/<app>/vn.config.json` if it has one, else the app
//!    directory from the build manifest's entry (`apps/hello/src/main.js` →
//!    `hello`). Dev builds only: a dev host runs many apps out of one exe, so
//!    the exe cannot be the name.
//! 3. The executable's file stem. A shipped build is one exe per app and its
//!    file name IS the app's name, so a `prod` build needs no new artifact and
//!    no config to find its folder.
//!
//! # Encryption is sticky
//!
//! `{ encrypted: true }` is a runtime call, so a restarted host boots with the
//! flag off. A folder that already holds a sealed area therefore **decides its
//! own format**: the load keeps it encrypted and says so, and only an explicit
//! `setStorageFolder(path, { encrypted: false })` opens it back up. Without
//! that rule, restarting an app that had turned encryption on would silently
//! write the user's data back out in plaintext, which is the worst thing this
//! crate could do.
//!
//! # Concurrency: last writer wins, and nothing is ever half-written
//!
//! Each process loads the area once and holds it; writes replace the whole file
//! by rename ([`file`]). Two processes on one folder therefore cannot corrupt
//! the file — a reader sees the old area or the new one, never a torn one — but
//! the second writer's copy is the one that survives, and neither sees the
//! other's changes while running.
//!
//! That is exactly the situation a browser has with two tabs, and exactly what
//! the `storage` event exists to solve. We do not fire it (redjs's storage
//! module documents why), so the honest answer for two concurrent dev instances
//! is to give them different folders: `VN_APP_NAME=hello-b`, or an explicit
//! `setStorageFolder`. `VN_DIST` already separates their *builds*; this
//! separates their *data*.
//!
//! # Writes are coalesced to one per frame
//!
//! `StorageHost::store` is called by redjs on every mutation. Writing the file
//! there would turn a loop of a hundred `setItem` calls into a hundred
//! `fsync`+rename pairs. So `store` stages the snapshot and [`pump`] — one call
//! per frame, beside `vn_permissions::pump` — writes at most once. A burst
//! inside one JS wave is one write, and the data is on disk within a frame
//! (~6 ms at 165 Hz).
//!
//! The cost, stated: a hard kill in that window loses the last frame's writes.
//! `appStorage.flush()` is the synchronous escape hatch for code that wants the
//! guarantee at a specific moment.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use vn_runtime::redjs::webapi::storage::{self as api, StorageHost};
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{host_fn, JsRuntime};

pub mod crypt;
pub mod file;
pub mod folders;
#[cfg(feature = "indexeddb")]
pub mod idb;

pub use folders::{user_folder, KINDS};

// ---------------------------------------------------------------------------
// The backend
// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    folder: PathBuf,
    encrypted: bool,
    /// Did the app actually *say* which format it wants?
    ///
    /// This is what makes encryption STICKY, and it is not a nicety. The
    /// encryption flag is a runtime call, so a restarted host starts with
    /// `encrypted: false` — and without this bit, an app that turned
    /// encryption on and then restarted would find a sealed area, decide it
    /// was in the wrong format, and helpfully write the user's data back out
    /// in plaintext. Silently un-encrypting data is the single worst thing
    /// this crate could do, so it does not: a sealed area stays sealed unless
    /// something explicitly asks for `{ encrypted: false }`.
    encryption_explicit: bool,
    /// The area as last loaded or last handed to `store` — what a folder or
    /// encryption change has to re-write to the new place/format.
    snapshot: Vec<(String, String)>,
    /// Set once `load` has run: before that, this process has no opinion about
    /// what the area contains and must not write one.
    loaded: bool,
    /// A staged write waiting for [`pump`].
    dirty: bool,
    /// The last thing that went wrong, for `appStorage.info()`.
    error: Option<String>,
    /// A plaintext file that was migrated into a sealed one (or the reverse)
    /// and should be removed once the new one is safely written.
    retire: Option<PathBuf>,
}

/// The file-backed `localStorage` for this process. One per process; there is
/// one disk and one area.
pub struct FileStorage {
    state: Mutex<State>,
}

/// The one backend. A `OnceLock` rather than a `static` initialiser because a
/// `PathBuf` cannot be built in a const context.
fn storage() -> &'static FileStorage {
    static S: OnceLock<FileStorage> = OnceLock::new();
    S.get_or_init(|| FileStorage { state: Mutex::new(State::default()) })
}

impl FileStorage {
    fn note(&self, st: &mut State, what: String) {
        eprintln!("[storage] {what}");
        st.error = Some(what);
    }

    /// Reads the area out of `folder`, in whichever of the two formats is
    /// there.
    ///
    /// Returns `(entries, migrate)` — `migrate` meaning "this came out of the
    /// OTHER format and has to be re-written in the configured one".
    fn read_folder(&self, st: &mut State) -> (Vec<(String, String)>, bool) {
        let plain = file::plain_path(&st.folder);
        let sealed = file::sealed_path(&st.folder);

        // Encryption is sticky (see `State::encryption_explicit`): an existing
        // sealed area decides the format for a host that never said.
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
                // Nothing in the configured format. Something in the other one
                // is a MIGRATION — the user turned encryption on (or off) on an
                // area that already existed — not an empty area.
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

    /// One file, or `None` if it is not there. A file that IS there but cannot
    /// be read is an empty area **plus an advisory** — never a hard failure.
    /// Losing preferences is survivable; refusing to start is not.
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

    /// Writes the staged snapshot now. Returns `Ok(false)` when there was
    /// nothing to write.
    fn write_now(&self, st: &mut State) -> Result<bool, String> {
        if !st.dirty {
            return Ok(false);
        }
        st.dirty = false;
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
        if let Some(old) = st.retire.take() {
            if let Err(e) = std::fs::remove_file(&old) {
                eprintln!("[storage] could not remove the migrated {}: {e}", old.display());
            }
        }
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
        // A migration is a write this process owes the disk even if the app
        // never calls `setItem`.
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

// ---------------------------------------------------------------------------
// The folder
// ---------------------------------------------------------------------------

/// `%LOCALAPPDATA%\<app name>\storage` — see the crate header.
pub fn default_folder(app_name: Option<&str>) -> PathBuf {
    let name = resolve_app_name(app_name);
    match folders::user_folder("LOCALAPPDATA", &name) {
        Ok(p) => p.join("storage"),
        Err(e) => {
            // The one case with no good answer: no known folder API and no
            // HOME. A folder beside the executable is at least writable often
            // enough to be better than giving up on storage entirely.
            eprintln!("[storage] no local app data folder ({e}); falling back beside the exe");
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."))
                .join(format!("{name}-storage"))
        }
    }
}

fn resolve_app_name(from_embedder: Option<&str>) -> String {
    let env = std::env::var("VN_APP_NAME").ok();
    let stem = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_default();
    pick_app_name(env.as_deref(), from_embedder, &stem)
}

/// The precedence rule on its own, so it is testable without mutating the
/// process environment out from under a parallel test.
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

/// Points the backend at `folder`, creating it, and turns encryption on or off.
///
/// Idempotent, and safe to call at any time: before the first `localStorage`
/// touch it simply decides where the lazy load will read from; after it, the
/// in-memory area is authoritative and gets re-written into the new place and
/// format (which is what makes turning encryption on a migration rather than a
/// data loss).
pub fn set_storage_folder(folder: &Path, encrypted: bool) -> Result<PathBuf, String> {
    if encrypted && crypt::provider().is_none() {
        return Err(format!(
            "encrypted storage is not available on this platform — refusing to write plaintext \
             into {} after being asked for encryption",
            folder.display()
        ));
    }
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    // Canonicalising resolves `..` and gives `info()` a path a human can
    // compare against Explorer; on Windows it also gives the `\\?\` prefix,
    // which is stripped because it is noise in an advisory.
    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let folder = strip_verbatim(&folder);

    let r = relocate(&folder, encrypted);
    // IndexedDB latches its folder and key at its first use; a change after
    // that is for the next start (src/idb.rs, "A late setStorageFolder").
    // Outside the State lock: the IndexedDB host takes it from its own thread.
    #[cfg(feature = "indexeddb")]
    idb::storage_changed(&folder, encrypted);
    r.map(|()| folder)
}

/// The `localStorage` half of [`set_storage_folder`].
fn relocate(folder: &Path, encrypted: bool) -> Result<(), String> {
    let folder = folder.to_path_buf();
    let s = storage();
    let mut st = s.state.lock().unwrap();
    let same_folder = st.folder == folder;
    let was_encrypted = st.encrypted;
    let moved = !same_folder || was_encrypted != encrypted;
    st.folder = folder.clone();
    st.encrypted = encrypted;
    st.encryption_explicit = true;
    st.error = None;
    if moved && st.loaded {
        // The area this process is holding is the truth; put it where it now
        // belongs.
        //
        // Flipping encryption IN PLACE also retires the file in the old format
        // — otherwise the plaintext copy would sit next to the encrypted one
        // and the encryption would have bought nothing. Moving to a DIFFERENT
        // folder does not: the old folder is somebody else's data now, and
        // deleting a file there because an app changed its mind is not this
        // function's business.
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

/// The storage folder as of now: the configured one, else the default
/// `install` would have set. What IndexedDB's host latches (src/idb.rs).
#[cfg_attr(not(feature = "indexeddb"), allow(dead_code))]
fn current_folder() -> PathBuf {
    let folder = storage().state.lock().unwrap().folder.clone();
    if folder.as_os_str().is_empty() {
        default_folder(embedder_app_name().as_deref())
    } else {
        folder
    }
}

/// What the app SAID about encryption this run: `Some(encrypted)` once it
/// called `setStorageFolder`, `None` if it never did (the sticky rule then
/// decides — see `State::encryption_explicit`).
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

/// Writes the staged area now, synchronously. The escape hatch behind
/// `appStorage.flush()`.
pub fn flush() -> Result<bool, String> {
    let s = storage();
    let mut st = s.state.lock().unwrap();
    let r = s.write_now(&mut st);
    if let Err(e) = &r {
        st.error = Some(e.clone());
    }
    r
}

/// One coalesced write per frame. Goes in `vn-app`'s `pump_frame` beside
/// `vn_permissions::pump`, and is NOT dev-gated: remembering a preference is
/// application work.
pub fn pump() {
    // The common case is "nothing changed", and it must cost one uncontended
    // lock and a bool read.
    let s = storage();
    let mut st = s.state.lock().unwrap();
    if !st.dirty {
        return;
    }
    if let Err(e) = s.write_now(&mut st) {
        // Once per distinct message: a folder that cannot be written will fail
        // on every frame, and a log line per frame is not a diagnostic.
        if st.error.as_deref() != Some(e.as_str()) {
            eprintln!("[storage] {e}");
        }
        st.error = Some(e);
    }
}

// ---------------------------------------------------------------------------
// host functions
// ---------------------------------------------------------------------------

fn json_escape(s: &str) -> String {
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

// `__vn_storage_user_folder(kind, subfolder) -> json` — `{"path":"…"}` or
// `{"error":"…"}`. Creates nothing.
host_fn!(fn vn_storage_user_folder(g, args) {
    let kind = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let sub = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let json = match folders::user_folder(&kind, &sub) {
        Ok(p) => format!("{{\"path\":\"{}\"}}", json_escape(&p.display().to_string())),
        Err(e) => err_json(&e),
    };
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

// `__vn_storage_set_folder(path, encrypted) -> json` — `{"path":"…"}` (the
// canonical path it settled on) or `{"error":"…"}`.
host_fn!(fn vn_storage_set_folder(g, args) {
    let path = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let encrypted = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default() == "true";
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

// `__vn_storage_info() -> json` — everything a settings screen or a test wants
// in one call: where the area is, whether it is encrypted, which file it is in,
// how big that file is, and the last thing that went wrong.
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
    let json = format!(
        "{{\"folder\":\"{}\",\"file\":\"{}\",\"encrypted\":{},\"bytes\":{},\"loaded\":{},\
          \"pending\":{},\"provider\":\"{}\",\"kinds\":[{}],\"error\":{},\"indexedDB\":{}}}",
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

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

static APP_NAME: Mutex<Option<String>> = Mutex::new(None);

fn embedder_app_name() -> Option<String> {
    APP_NAME.lock().unwrap().clone()
}

/// Installs the backend into redjs and registers the extension surface.
///
/// `app_name` is what the embedder knows and this crate cannot: in vue-native,
/// the dev build's `vn.config.json` name or manifest-derived app directory. A
/// `prod` build passes `None` and the executable's own name is used.
///
/// Called from `vn-app`'s boot beside `vn_permissions::install`, and BEFORE any
/// app code runs — the area is loaded lazily on first touch, so a host that
/// installs late would hand the app an empty `localStorage` and then quietly
/// start honouring the disk.
pub fn install(rt: &JsRuntime, app_name: Option<String>) {
    *APP_NAME.lock().unwrap() = app_name.clone();
    {
        let mut st = storage().state.lock().unwrap();
        if st.folder.as_os_str().is_empty() {
            st.folder = default_folder(app_name.as_deref());
        }
    }
    api::set_host(storage());
    // …and the same folder (its `indexeddb\` subfolder) for IndexedDB, BEFORE
    // app code: redjs reads its host once, at the first `indexedDB` use, and a
    // host installed after that would leave the run in memory.
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

    #[test]
    fn an_app_name_becomes_a_folder_name_that_windows_will_accept() {
        // A display name is not a folder name. `vue-native — hello` is a real
        // window title in this repo and contains an em dash and spaces.
        assert_eq!(sanitize("hello"), "hello");
        assert_eq!(sanitize("vue-native — hello"), "vue-native---hello");
        assert_eq!(sanitize("a/b\\c:d"), "a-b-c-d");
        assert_eq!(sanitize("My App 1.0"), "My-App-1.0");
        // Never empty, never a leading dot (a hidden folder), never a trailing
        // one (Windows silently drops trailing dots).
        assert_eq!(sanitize(""), "vue-native-app");
        assert_eq!(sanitize("///"), "vue-native-app");
        assert_eq!(sanitize(".hidden."), "hidden");
        assert_eq!(sanitize(&"x".repeat(200)).len(), 64);
    }

    #[test]
    fn the_env_override_wins_over_the_embedder_and_the_exe() {
        // The precedence two concurrent dev instances rely on, and the one the
        // persistence test pins so it cannot touch a real app's folder.
        assert_eq!(pick_app_name(Some("pinned"), Some("config"), "vn-app"), "pinned");
        assert_eq!(pick_app_name(None, Some("config"), "vn-app"), "config");
        assert_eq!(pick_app_name(Some(""), Some("config"), "vn-app"), "config");
        // Prod: no manifest, so the exe's own name IS the app's name.
        assert_eq!(pick_app_name(None, None, "my-shipped-app"), "my-shipped-app");
        // And never nothing.
        assert_eq!(pick_app_name(None, None, ""), "vue-native-app");
        // The live resolver agrees with the rule it delegates to.
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
        // Every host fn here hand-rolls its JSON, and a backslash is the one
        // character a Windows path is full of.
        assert_eq!(json_escape(r"C:\Users\x"), r"C:\\Users\\x");
        assert_eq!(json_escape("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
        assert!(err_json(r"C:\a").starts_with("{\"error\":\"C:\\\\a\""));
    }

    /// A scratch folder that cleans itself up.
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

    /// There is exactly ONE backend per process — which is the design — so the
    /// tests that drive it have to run one at a time. Poison-tolerant, so one
    /// failing test does not turn the rest into confusing lock panics.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The state a process has at boot: nothing loaded, nothing staged.
    fn cold() {
        let mut st = storage().state.lock().unwrap();
        *st = State::default();
    }

    /// Drives the backend exactly as redjs does: load, store, flush.
    fn cycle(folder: &Path, encrypted: bool, entries: &[(String, String)]) {
        set_storage_folder(folder, encrypted).expect("set folder");
        let _ = storage().load();
        storage().store(entries);
        flush().expect("flush");
    }

    /// A cold start against `folder` with the format spelled out: what a
    /// restarted host does when the app calls `setStorageFolder` at boot.
    fn read_back(folder: &Path, encrypted: bool) -> Vec<(String, String)> {
        cold();
        set_storage_folder(folder, encrypted).expect("set folder");
        storage().load()
    }

    /// A cold start with NOTHING configured — the app never called
    /// `setStorageFolder`, so only `install`'s default folder is set and the
    /// format is whatever the folder already holds.
    fn read_back_unconfigured(folder: &Path) -> Vec<(String, String)> {
        cold();
        storage().state.lock().unwrap().folder = folder.to_path_buf();
        storage().load()
    }

    #[test]
    fn an_area_survives_a_restart_of_the_backend() {
        // The whole point of the crate, at the trait level: what `store` was
        // given is what the next process's `load` returns.
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
        // The honest note from the crate header, executed: an app that flips
        // `{ encrypted: true }` keeps its data and loses the plaintext copy.
        let _g = serial();
        let t = Temp::new("migrate");
        let entries = vec![("keep".to_string(), "me".to_string())];
        cold();
        cycle(&t.0, false, &entries);
        assert!(file::plain_path(&t.0).exists());

        // A restarted host, now asking for encryption: the load finds only the
        // plaintext file and adopts it.
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
        // The other half of the migration: the app calls setStorageFolder with
        // `{ encrypted: true }` at runtime rather than at boot.
        let _g = serial();
        let t = Temp::new("flip");
        let entries = vec![("k".to_string(), "v".to_string())];
        cold();
        cycle(&t.0, false, &entries);
        set_storage_folder(&t.0, true).expect("flip to encrypted");
        assert!(file::sealed_path(&t.0).exists());
        assert!(!file::plain_path(&t.0).exists(), "the plaintext copy is retired");
        assert_eq!(read_back(&t.0, true), entries);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_restart_that_says_nothing_keeps_an_encrypted_area_encrypted() {
        // THE failure this design refuses: the encryption flag is a runtime
        // call, so a restarted host boots with `encrypted: false`. Without the
        // sticky rule it would find a sealed area, decide it was in the wrong
        // format, and write the user's data back out in plaintext — silently
        // un-encrypting it. Once sealed, only an explicit `{ encrypted: false }`
        // opens it back up.
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

        // …and an app that genuinely wants plaintext still gets it.
        assert_eq!(read_back(&t.0, false), entries);
        assert!(flush().expect("the decryption write"));
        assert!(file::plain_path(&t.0).exists());
        assert!(!file::sealed_path(&t.0).exists(), "the sealed copy is retired");
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
        // …and now the app calls setStorageFolder for the first time.
        set_storage_folder(&b.0, false).expect("move");
        assert_eq!(read_back(&b.0, false), entries);
        assert!(file::plain_path(&a.0).exists(), "the old folder is left alone");
    }

    #[test]
    fn a_folder_that_cannot_be_written_reports_rather_than_pretending() {
        // A file where a directory should be: `create_dir_all` fails, and the
        // caller has to hear about it instead of silently keeping the old one.
        let _g = serial();
        let t = Temp::new("blocked");
        std::fs::create_dir_all(&t.0).unwrap();
        let blocked = t.0.join("not-a-dir");
        std::fs::write(&blocked, b"x").unwrap();
        let e = set_storage_folder(&blocked, false).unwrap_err();
        assert!(e.contains("could not create"), "{e}");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn asking_for_encryption_without_a_provider_is_refused_not_silently_plaintext() {
        let _g = serial();
        let t = Temp::new("no-provider");
        let e = set_storage_folder(&t.0, true).unwrap_err();
        assert!(e.contains("not available on this platform"), "{e}");
    }
}
