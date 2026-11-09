//! IndexedDB's host: where the databases live and the key that protects them
//! (m25 P2, research/reports/m25/indexeddb-design.md §5).
//!
//! redjs owns everything browser-shaped about IndexedDB — the API, the
//! transactions, the SQLite files themselves (`redjs::webapi::indexeddb::
//! sqlite`). What it cannot know, and this crate already does, is the same two
//! things `localStorage` needs from a desktop host: **which folder** and
//! **whether it is encrypted**. So this is the second `*Host` this crate
//! implements, over the SAME state `appStorage.setStorageFolder` writes:
//!
//! ```text
//!   <storage folder>\localStorage.json | .sealed      (lib.rs)
//!   <storage folder>\indexeddb\                       (this file)
//!        lock                    held by the running instance (redjs)
//!        <h>.sqlite3 (+ -wal)    one database each (redjs, format 1)
//!        key.sealed              the database key, sealed by DPAPI
//!        key.retire              "decrypt at the next start" (a late call)
//! ```
//!
//! # The key
//!
//! SQLite3 Multiple Ciphers encrypts every page with ChaCha20-Poly1305 under a
//! raw 256-bit key. The key is 32 bytes from `BCryptGenRandom`, generated the
//! first time a folder is asked to be encrypted, and stored ONLY sealed —
//! `file::seal` over DPAPI, the same provider and header as
//! `localStorage.sealed`, so the same honest statement applies (it protects the
//! files from other users and from being copied off the machine; not from code
//! running as the same user). The plaintext key exists in this process's
//! memory and in redjs's server thread, never on disk. On macOS the 32 bytes
//! come from `getentropy` (ring) and `key.sealed` is sealed by the Keychain
//! provider (`crypt`); a keychain that cannot be read right now refuses the
//! run and leaves `key.sealed` where it is, rather than moving it aside below.
//!
//! # Encryption is sticky — the same rule as `localStorage`
//!
//! Decided once, when IndexedDB starts (the first `indexedDB` use):
//!
//! | the app said                    | key.sealed     | → redjs runs           |
//! |---------------------------------|----------------|------------------------|
//! | `{ encrypted: true }`           | readable       | Encrypted(key)         |
//! | `{ encrypted: true }`           | absent         | Encrypted(new key)     |
//! | `{ encrypted: false }`          | readable       | Decrypting → plaintext |
//! | `{ encrypted: false }`          | absent         | Plaintext              |
//! | nothing (this run)              | exists         | Encrypted (sticky)     |
//! | nothing, localStorage is sealed | absent         | Encrypted(new key)     |
//! | nothing, `key.retire` exists    | readable       | Decrypting → plaintext |
//! | nothing                         | absent         | Plaintext              |
//!
//! A folder that is encrypted stays encrypted until something explicitly asks
//! for plaintext; "nothing" never decrypts. A sealed `localStorage` next door
//! counts as having asked: one `setStorageFolder(…, { encrypted: true })`
//! covers the whole folder, so an app relying on the sticky rule does not end
//! up with an encrypted `localStorage` and plaintext databases.
//!
//! A `key.sealed` that cannot be unsealed (another Windows user, a reset
//! password, a profile copied from another machine) is moved aside as
//! `key.unreadable-<unix time>.sealed`, never deleted, and a new key is made;
//! redjs then moves aside every database the new key cannot open, with an
//! advisory. Losing the data is survivable; refusing to start is not — the
//! same rule `crypt` states for `localStorage`.
//!
//! # A late `setStorageFolder`
//!
//! redjs reads the folder and the key ONCE, at its first use, and does not move
//! open databases under live connections. A call after that applies at the
//! next start, and is said once (redjs's `config-changed` advisory). What this
//! file adds is that the ENCRYPTION half survives to that next start even if
//! the app makes the same late call again: `{ encrypted: true }` creates the
//! folder's `key.sealed` now (the next start finds it — sticky), and
//! `{ encrypted: false }` leaves `key.retire` next to an existing key (the next
//! start decrypts). A late call that does not mention `encrypted` changes no
//! format: it flips nothing, and a move carries an encrypted IndexedDB's
//! `key.sealed` into the new folder. The FOLDER half cannot be remembered without a pointer
//! file in a folder the app moved away from, so it is not: a new folder takes
//! effect at a start whose `setStorageFolder` runs before the first
//! `indexedDB` call — the advisory says so.
//!
//! # Quota
//!
//! `navigator.storage.estimate().quota` = min(60% of the volume, what is
//! already used + what is free) — Chromium's per-origin share of the disk,
//! never more than the disk can actually take. Re-read by redjs at most every
//! two seconds.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use vn_runtime::redjs::webapi::indexeddb::{self as api, IdbEncryption, IndexedDbHost};

use crate::crypt::{self, SecretProvider};
use crate::file;

pub const FOLDER_NAME: &str = "indexeddb";
pub const KEY_NAME: &str = "key.sealed";
pub const RETIRE_NAME: &str = "key.retire";

/// `<storage folder>\indexeddb`.
pub fn folder_of(storage: &Path) -> PathBuf {
    storage.join(FOLDER_NAME)
}

/// The mode redjs was handed at its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Plaintext,
    Encrypted,
    /// Encrypted files are being rekeyed to plaintext; becomes `Plaintext`
    /// once redjs says every file is (`migrated`).
    Decrypting,
    /// Encryption was asked for and cannot be had: every open fails.
    Refused,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Plaintext => "plaintext",
            Mode::Encrypted => "encrypted",
            Mode::Decrypting => "decrypting",
            Mode::Refused => "refused",
        }
    }
}

/// What redjs latched at its start: the folder, then the mode.
#[derive(Default)]
struct Latched {
    folder: Option<PathBuf>,
    mode: Option<Mode>,
    error: Option<String>,
}

/// Lock order: `LATCHED` is never held while the storage `State` is taken
/// (lib.rs holds `State` while it asks for [`info_json`]).
static LATCHED: Mutex<Latched> = Mutex::new(Latched { folder: None, mode: None, error: None });

/// The folder redjs latched, or the one it would latch now.
fn latched_or_current() -> PathBuf {
    let latched = LATCHED.lock().unwrap().folder.clone();
    latched.unwrap_or_else(|| folder_of(&crate::current_folder()))
}

pub struct Host;
pub static HOST: Host = Host;

impl IndexedDbHost for Host {
    fn folder(&self) -> Result<PathBuf, String> {
        let dir = folder_of(&crate::current_folder());
        std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        LATCHED.lock().unwrap().folder = Some(dir.clone());
        Ok(dir)
    }

    fn encryption(&self) -> Result<IdbEncryption, String> {
        let storage = crate::current_folder();
        let intent = crate::encryption_intent();
        let latched = LATCHED.lock().unwrap().folder.clone();
        let dir = latched.unwrap_or_else(|| folder_of(&storage));
        let r = decide(&dir, &storage, intent, crypt::provider());
        let mut l = LATCHED.lock().unwrap();
        match &r {
            Ok((mode, _)) => l.mode = Some(*mode),
            Err(e) => {
                l.mode = Some(Mode::Refused);
                l.error = Some(e.clone());
            }
        }
        r.map(|(_, e)| e)
    }

    fn quota_bytes(&self, usage: u64) -> u64 {
        quota_for(&latched_or_current(), usage)
    }

    fn describe(&self) -> String {
        latched_or_current().display().to_string()
    }

    /// Every file is in the mode redjs was given. After a decryption that
    /// means the key has nothing left to open: it goes, and so does a
    /// `key.retire` that asked for it.
    fn migrated(&self) {
        let mut l = LATCHED.lock().unwrap();
        let Some(dir) = l.folder.clone() else { return };
        match l.mode {
            Some(Mode::Decrypting) => {
                for name in [KEY_NAME, RETIRE_NAME] {
                    let p = dir.join(name);
                    if p.exists() {
                        if let Err(e) = std::fs::remove_file(&p) {
                            eprintln!("[storage] IndexedDB is decrypted but {} could not be removed: {e}", p.display());
                        }
                    }
                }
                eprintln!("[storage] IndexedDB in {} is decrypted (the sealed key is removed)", dir.display());
                l.mode = Some(Mode::Plaintext);
            }
            Some(Mode::Encrypted) => {
                // An explicit `{ encrypted: true }` this run outranks an older
                // late request to decrypt (`decide` already ignored it).
                let _ = std::fs::remove_file(dir.join(RETIRE_NAME));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

/// The table in the module header. `intent` is `Some(encrypted)` when the app
/// called `setStorageFolder` this run, `None` when it did not.
fn decide(
    dir: &Path,
    storage: &Path,
    intent: Option<bool>,
    provider: Option<&'static dyn SecretProvider>,
) -> Result<(Mode, IdbEncryption), String> {
    let key_path = dir.join(KEY_NAME);
    let retire = dir.join(RETIRE_NAME);
    let want = match intent {
        Some(e) => e,
        None if retire.exists() => false,
        None => key_path.exists() || file::sealed_path(storage).exists(),
    };
    if intent == Some(true) {
        let _ = std::fs::remove_file(&retire);
    }
    if want {
        let Some(p) = provider else {
            return Err(String::from(
                "encrypted storage is not available on this platform — IndexedDB refuses to write plaintext \
                 into a folder that asked for encryption",
            ));
        };
        let key = match read_key(p, &key_path) {
            Ok(Some(k)) => k,
            Ok(None) => {
                let k = new_key()?;
                write_key(p, &key_path, &k)?;
                eprintln!("[storage] IndexedDB in {} is encrypted at rest (a new key, sealed by {})", dir.display(), p.name());
                k
            }
            // The provider itself is unavailable (a locked or refused
            // keychain; never the case for DPAPI): that says nothing about
            // the key file, so it stays where it is and this run is refused.
            Err(e) if p.ready().is_err() => {
                return Err(format!("the IndexedDB key {} cannot be unsealed right now ({e})", key_path.display()));
            }
            Err(e) => {
                let aside = move_aside(&key_path);
                eprintln!(
                    "[storage] the IndexedDB key {} cannot be unsealed ({e}); it is kept as {} and a new key is made — \
                     databases encrypted under the old one are moved aside, not opened",
                    key_path.display(),
                    aside.display()
                );
                let k = new_key()?;
                write_key(p, &key_path, &k)?;
                k
            }
        };
        return Ok((Mode::Encrypted, IdbEncryption::Encrypted(key)));
    }
    // Plaintext asked for (or nothing asked and nothing encrypted here).
    match provider.map(|p| read_key(p, &key_path)) {
        Some(Ok(Some(old_key))) => {
            eprintln!("[storage] IndexedDB in {} is being decrypted, as asked", dir.display());
            Ok((Mode::Decrypting, IdbEncryption::Decrypting { old_key }))
        }
        Some(Err(e)) => {
            // Nothing can decrypt those files now; redjs moves them aside.
            eprintln!("[storage] the IndexedDB key {} cannot be unsealed ({e}); its databases cannot be decrypted", key_path.display());
            Ok((Mode::Plaintext, IdbEncryption::Plaintext))
        }
        _ => Ok((Mode::Plaintext, IdbEncryption::Plaintext)),
    }
}

fn read_key(p: &dyn SecretProvider, path: &Path) -> Result<Option<[u8; 32]>, String> {
    let raw = match std::fs::read(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read it: {e}")),
    };
    let mut hex = file::unseal(p, &raw)?;
    let key = parse_hex(hex.trim());
    // The plaintext key lives in this process anyway; do not leave a second
    // copy of it in a freed String.
    unsafe { hex.as_bytes_mut().fill(0) };
    key.map(Some).ok_or_else(|| String::from("it does not hold a 256-bit key"))
}

fn write_key(p: &dyn SecretProvider, path: &Path, key: &[u8; 32]) -> Result<(), String> {
    let mut hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let sealed = file::seal(p, &hex);
    unsafe { hex.as_bytes_mut().fill(0) };
    file::write_atomic(path, &sealed?).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn parse_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut k = [0u8; 32];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(k)
}

fn move_aside(path: &Path) -> PathBuf {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let aside = path.with_file_name(format!("key.unreadable-{secs}.sealed"));
    if let Err(e) = std::fs::rename(path, &aside) {
        eprintln!("[storage] could not move {} aside: {e}", path.display());
    }
    aside
}

/// 32 bytes from the OS's preferred RNG.
#[cfg(target_os = "windows")]
fn new_key() -> Result<[u8; 32], String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut k = [0u8; 32];
    let st = unsafe { BCryptGenRandom(None, &mut k, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if st.0 < 0 {
        return Err(format!("BCryptGenRandom failed: 0x{:08x}", st.0 as u32));
    }
    Ok(k)
}

#[cfg(target_os = "macos")]
fn new_key() -> Result<[u8; 32], String> {
    let mut k = [0u8; 32];
    crypt::random_bytes(&mut k)?;
    Ok(k)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn new_key() -> Result<[u8; 32], String> {
    Err(String::from("no key generator on this platform"))
}

// ---------------------------------------------------------------------------
// Quota
// ---------------------------------------------------------------------------

/// min(60% of the volume holding `dir`, `usage` + its free space). The design's
/// 1 GiB default when the volume cannot be asked.
pub fn quota_for(dir: &Path, usage: u64) -> u64 {
    match disk(dir) {
        Some((free, total)) => quota_from(free, total, usage),
        None => api::DEFAULT_QUOTA,
    }
}

fn quota_from(free: u64, total: u64, usage: u64) -> u64 {
    (total / 10 * 6).min(usage.saturating_add(free))
}

/// (bytes free to this user, volume size).
#[cfg(target_os = "windows")]
fn disk(dir: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut free, mut total) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), Some(&mut total), None) }.ok()?;
    (total > 0).then_some((free, total))
}

/// `statfs` rather than `statvfs`: Darwin's `statvfs` has 32-bit block counts
/// and rescales its block size to fit them, `statfs` is 64-bit throughout.
/// `f_bavail` is what a non-root user may use; on APFS it does not count
/// purgeable space, so the quota errs low.
#[cfg(target_os = "macos")]
fn disk(dir: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: a NUL-terminated path and a zeroed, correctly typed out struct.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = u64::from(st.f_bsize);
    let (free, total) = (st.f_bavail.saturating_mul(block), st.f_blocks.saturating_mul(block));
    (total > 0).then_some((free, total))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn disk(_dir: &Path) -> Option<(u64, u64)> {
    None
}

// ---------------------------------------------------------------------------
// The seam to lib.rs
// ---------------------------------------------------------------------------

/// Installs this host into redjs. From `crate::install`, before app code runs.
pub fn install() {
    api::set_host(&HOST);
}

/// `setStorageFolder` ran. Before IndexedDB's first use there is nothing to
/// do: redjs reads the new state when it starts. After it, the change is for
/// the next start — say so (redjs's advisory), and make the encryption half
/// stick (module header, "A late setStorageFolder"). `encrypted: None` — the
/// app did not say — keeps the latched format: it flips nothing, and a move
/// takes an encrypted IndexedDB's encryption to the new folder rather than
/// leaving it to start there in plaintext.
pub fn storage_changed(storage: &Path, encrypted: Option<bool>) {
    let (latched_dir, latched_mode) = {
        let l = LATCHED.lock().unwrap();
        match (&l.folder, l.mode) {
            (Some(f), Some(m)) => (f.clone(), m),
            // Not started (or starting right now, and about to read the new
            // state): nothing to defer.
            _ => return,
        }
    };
    let dir = folder_of(storage);
    let moved = dir != latched_dir;
    let (changed, persist) = late_change(latched_mode, moved, encrypted);
    if !changed {
        return;
    }
    api::config_changed();
    if moved {
        eprintln!(
            "[storage] IndexedDB stays in {} for this run; {} takes effect at a start whose setStorageFolder runs \
             before the first indexedDB call",
            latched_dir.display(),
            dir.display()
        );
    }
    let Some(intent) = persist else { return };
    if let Err(e) = persist_intent(&dir, intent) {
        eprintln!("[storage] could not record the IndexedDB encryption for the next start: {e}");
        LATCHED.lock().unwrap().error = Some(e);
    }
}

/// A late call, decided: does it change anything for the next start (the
/// advisory), and which encryption, if any, to record for it.
fn late_change(latched: Mode, moved: bool, encrypted: Option<bool>) -> (bool, Option<bool>) {
    let flipped = match (latched, encrypted) {
        (_, None) => false,
        (Mode::Encrypted, Some(e)) => !e,
        (Mode::Plaintext | Mode::Decrypting, Some(e)) => e,
        (Mode::Refused, Some(_)) => true,
    };
    if !moved && !flipped {
        return (false, None);
    }
    let persist = match encrypted {
        Some(e) => Some(e),
        // Unsaid, only an encryption is carried over: "plaintext" written into
        // the new folder would be a `key.retire`, i.e. a decryption nobody
        // asked for.
        None => (latched == Mode::Encrypted).then_some(true),
    };
    (true, persist)
}

/// The encryption half of a late call, made to outlive this run.
fn persist_intent(dir: &Path, encrypted: bool) -> Result<(), String> {
    let key_path = dir.join(KEY_NAME);
    let retire = dir.join(RETIRE_NAME);
    if encrypted {
        let p = crypt::provider().ok_or("this platform has no encryption provider")?;
        // Not ready (a locked keychain): an existing key must not be moved
        // aside as "unreadable" below.
        p.ready()?;
        let _ = std::fs::remove_file(&retire);
        let readable = matches!(read_key(p, &key_path), Ok(Some(_)));
        if !readable {
            if key_path.exists() {
                move_aside(&key_path);
            }
            write_key(p, &key_path, &new_key()?)?;
        }
        eprintln!("[storage] IndexedDB in {} will be encrypted from the next start", dir.display());
    } else if key_path.exists() {
        file::write_atomic(&retire, b"decrypt at the next start (appStorage.setStorageFolder { encrypted: false })\n")
            .map_err(|e| format!("could not write {}: {e}", retire.display()))?;
        eprintln!("[storage] IndexedDB in {} will be decrypted at the next start", dir.display());
    }
    Ok(())
}

/// `appStorage.info().indexedDB`, as JSON: where the databases are (`storage`
/// is the storage folder as lib.rs knows it, used until redjs has latched
/// one), the mode redjs is running in (`null` before the first IndexedDB use),
/// whether a sealed key exists, and the last error.
pub fn info_json(storage: &Path) -> String {
    let l = LATCHED.lock().unwrap();
    let folder = l.folder.clone().unwrap_or_else(|| folder_of(storage));
    let key = folder.join(KEY_NAME).exists();
    format!(
        "{{\"folder\":\"{}\",\"started\":{},\"mode\":{},\"encrypted\":{},\"key\":{},\"error\":{}}}",
        crate::json_escape(&folder.display().to_string()),
        l.mode.is_some(),
        l.mode.map(|m| format!("\"{}\"", m.name())).unwrap_or_else(|| "null".into()),
        match l.mode {
            Some(m) => (m == Mode::Encrypted).to_string(),
            None => "null".into(),
        },
        key,
        match &l.error {
            Some(e) => format!("\"{}\"", crate::json_escape(e)),
            None => "null".into(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new(tag: &str) -> Temp {
            let p = std::env::temp_dir().join(format!("vn-storage-idb-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(p.join(FOLDER_NAME)).unwrap();
            Temp(p)
        }
        fn dir(&self) -> PathBuf {
            folder_of(&self.0)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn mode(t: &Temp, intent: Option<bool>) -> (Mode, IdbEncryption) {
        decide(&t.dir(), &t.0, intent, crypt::provider()).expect("decide")
    }

    #[test]
    fn hex_round_trips_and_rejects_anything_else() {
        let k: [u8; 32] = core::array::from_fn(|i| (i * 7) as u8);
        let hex: String = k.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(parse_hex(&hex), Some(k));
        assert_eq!(parse_hex(&hex[..62]), None);
        assert_eq!(parse_hex(&format!("{}zz", &hex[..62])), None);
    }

    #[test]
    fn the_quota_is_sixty_percent_of_the_disk_and_never_more_than_fits() {
        const G: u64 = 1 << 30;
        assert_eq!(quota_from(800 * G, 1000 * G, 0), 600 * G, "60% of the volume");
        assert_eq!(quota_from(500 * G, 1000 * G, 0), 500 * G, "never more than is free");
        assert_eq!(quota_from(10 * G, 1000 * G, 3 * G), 13 * G, "what is used + what is free");
        assert!(quota_for(&std::env::temp_dir(), 0) > 0);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn the_sticky_rule_table() {
        // Nothing asked, nothing here: plaintext, and no key is made.
        let t = Temp::new("table");
        assert_eq!(mode(&t, None).0, Mode::Plaintext);
        assert!(!t.dir().join(KEY_NAME).exists());

        // Asked for encryption: a key is made, sealed — not legible on disk.
        let (m, e) = mode(&t, Some(true));
        assert_eq!(m, Mode::Encrypted);
        let IdbEncryption::Encrypted(k1) = e else { panic!("{e:?}") };
        let raw = std::fs::read(t.dir().join(KEY_NAME)).unwrap();
        let hex: String = k1.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!raw.windows(64).any(|w| w == hex.as_bytes()), "the key is legible in key.sealed");

        // A restart that says nothing: STILL encrypted, same key.
        assert_eq!(mode(&t, None).1, IdbEncryption::Encrypted(k1));
        // Asked again: the same key, not a new one.
        assert_eq!(mode(&t, Some(true)).1, IdbEncryption::Encrypted(k1));

        // Explicit plaintext: decrypt with the old key; the key goes only once
        // redjs says every file is plaintext.
        assert_eq!(mode(&t, Some(false)).1, IdbEncryption::Decrypting { old_key: k1 });
        assert!(t.dir().join(KEY_NAME).exists());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_sealed_local_storage_next_door_counts_as_having_asked() {
        let t = Temp::new("ls-sealed");
        std::fs::write(file::sealed_path(&t.0), b"VNSTOR").unwrap();
        assert_eq!(mode(&t, None).0, Mode::Encrypted);
        assert!(t.dir().join(KEY_NAME).exists());
        // …but an explicit plaintext still wins.
        let t2 = Temp::new("ls-sealed-plain");
        std::fs::write(file::sealed_path(&t2.0), b"VNSTOR").unwrap();
        assert_eq!(mode(&t2, Some(false)).0, Mode::Plaintext);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn an_unreadable_key_is_kept_aside_and_replaced() {
        let t = Temp::new("unreadable");
        std::fs::write(t.dir().join(KEY_NAME), b"VNSTOR\x01\x01not a dpapi blob").unwrap();
        let (m, e) = mode(&t, None);
        assert_eq!(m, Mode::Encrypted);
        assert!(matches!(e, IdbEncryption::Encrypted(_)));
        let aside: Vec<_> = std::fs::read_dir(t.dir())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("key.unreadable-"))
            .collect();
        assert_eq!(aside.len(), 1, "the unreadable key is kept, not deleted");
        assert!(read_key(crypt::provider().unwrap(), &t.dir().join(KEY_NAME)).unwrap().is_some(), "and a new one is in place");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_late_call_is_remembered_for_the_next_start() {
        let t = Temp::new("late");
        // A late `{ encrypted: true }`: the key exists now, so a next start
        // that says nothing encrypts.
        persist_intent(&t.dir(), true).unwrap();
        let IdbEncryption::Encrypted(k) = mode(&t, None).1 else { panic!() };
        // A late `{ encrypted: false }`: the next start that says nothing
        // decrypts…
        persist_intent(&t.dir(), false).unwrap();
        assert!(t.dir().join(RETIRE_NAME).exists());
        assert_eq!(mode(&t, None).1, IdbEncryption::Decrypting { old_key: k });
        // …unless that start asks for encryption again, which wins.
        assert_eq!(mode(&t, Some(true)).1, IdbEncryption::Encrypted(k));
        assert!(!t.dir().join(RETIRE_NAME).exists());
    }

    /// The macOS envelope under the same table (a fixed key stands in for the
    /// keychain): sealed, sticky, and the key is not legible on disk.
    #[test]
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn the_sticky_rule_holds_under_the_keychain_envelope() {
        let p: &'static dyn SecretProvider = Box::leak(Box::new(crypt::envelope::FixedKey::new(6)));
        let t = Temp::new("envelope");
        let (m, e) = decide(&t.dir(), &t.0, Some(true), Some(p)).unwrap();
        assert_eq!(m, Mode::Encrypted);
        let IdbEncryption::Encrypted(k) = e else { panic!("{e:?}") };
        let raw = std::fs::read(t.dir().join(KEY_NAME)).unwrap();
        assert_eq!(&raw[..8], b"VNSTOR\x01\x02");
        let hex: String = k.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!raw.windows(64).any(|w| w == hex.as_bytes()));
        assert_eq!(decide(&t.dir(), &t.0, None, Some(p)).unwrap().1, IdbEncryption::Encrypted(k));
    }

    /// A keychain that cannot be read is not an unreadable key: the run is
    /// refused and `key.sealed` stays exactly where it was.
    #[test]
    fn a_provider_that_is_not_ready_refuses_without_moving_the_key_aside() {
        struct Locked;
        impl SecretProvider for Locked {
            fn name(&self) -> &'static str { "locked" }
            fn id(&self) -> u8 { 2 }
            fn ready(&self) -> Result<(), String> { Err("macOS Keychain: locked".into()) }
            fn protect(&self, _: &[u8]) -> Result<Vec<u8>, String> { Err("locked".into()) }
            fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, String> { Err("locked".into()) }
        }
        static LOCKED: Locked = Locked;
        let t = Temp::new("not-ready");
        std::fs::write(t.dir().join(KEY_NAME), b"VNSTOR\x01\x02sealed key").unwrap();
        assert!(decide(&t.dir(), &t.0, None, Some(&LOCKED)).is_err());
        assert_eq!(std::fs::read(t.dir().join(KEY_NAME)).unwrap(), b"VNSTOR\x01\x02sealed key");
        let others = std::fs::read_dir(t.dir()).unwrap().count();
        assert_eq!(others, 1, "nothing was moved aside or created");
    }

    #[test]
    fn a_late_call_that_does_not_mention_encryption_changes_no_format() {
        use Mode::*;
        // Same folder, unsaid: nothing at all — in particular no decryption.
        for m in [Encrypted, Plaintext, Decrypting, Refused] {
            assert_eq!(late_change(m, false, None), (false, None), "{m:?}");
        }
        // A move, unsaid: an encrypted IndexedDB stays encrypted in the new
        // folder; a plaintext one records nothing (no `key.retire` there).
        assert_eq!(late_change(Encrypted, true, None), (true, Some(true)));
        assert_eq!(late_change(Plaintext, true, None), (true, None));
        assert_eq!(late_change(Decrypting, true, None), (true, None));
        // Said: as before.
        assert_eq!(late_change(Encrypted, false, Some(false)), (true, Some(false)));
        assert_eq!(late_change(Encrypted, false, Some(true)), (false, None));
        assert_eq!(late_change(Plaintext, false, Some(true)), (true, Some(true)));
        assert_eq!(late_change(Plaintext, true, Some(false)), (true, Some(false)));
    }
}
