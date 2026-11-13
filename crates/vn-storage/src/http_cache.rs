//! The HTTP cache's host: where redjs's disk cache lives and whether its entries are sealed.
//! redjs owns the cache rules, index and eviction (`redjs::webapi::http::cache`).
//!
//! ```text
//!   %LOCALAPPDATA%\<app name>\
//!        storage\              localStorage, cookies, IndexedDB (lib.rs)
//!        cache\http\           this cache (redjs's files)
//!             key.sealed       the entries' key, sealed by DPAPI (encrypted only)
//! ```
//!
//! It sits beside `storage\`, not in it: it can be deleted at any time, does not follow
//! `setStorageFolder`, and should not bloat backups of the storage folder.
//!
//! Encryption follows the storage setting. Each entry is AES-256-GCM-sealed (CNG on Windows,
//! ring elsewhere, `ring_gcm`) under one random key kept in `key.sealed`, with the provider and
//! header of `localStorage.sealed`; DPAPI per entry would be an LSA call per blob. An unreadable
//! keychain means no cache and the key file is left alone. Without encrypted storage, entries
//! are plaintext.
//!
//! The decision is made at install and is sticky: encrypted when the cache's own `key.sealed`,
//! `localStorage.sealed` or IndexedDB's `key.sealed` exists. `setStorageFolder(…, { encrypted })`
//! switches it at once (nothing to migrate: the folder is purged, and `{ encrypted: false }`
//! also deletes the key); a call without `encrypted` changes nothing. Encryption that was asked
//! for but cannot be had runs the app with no cache and says so, never with plaintext.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use vn_runtime::redjs::webapi::http::cache::{self, Config, Sealer};

use crate::{file, folders};

pub const KEY_NAME: &str = "key.sealed";

struct State {
    dir: Option<PathBuf>,
    max_bytes: u64,
    sealed: bool,
    /// Why the cache is off, when it was asked for and is.
    error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State { dir: None, max_bytes: 0, sealed: false, error: None });

pub fn folder_for(app_name: &str) -> Result<PathBuf, String> {
    Ok(folders::user_folder("LOCALAPPDATA", app_name)?.join("cache").join("http"))
}

/// Turns the cache on for this app; `max_mb` is the cap (0 = no cache). Call after
/// [`crate::install`] (which names the app), before any JS.
pub fn install(max_mb: u64) {
    if max_mb == 0 {
        return;
    }
    let name = crate::resolve_app_name(crate::embedder_app_name().as_deref());
    let dir = match folder_for(&name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[storage] no folder for the HTTP cache ({e}); running without one");
            return;
        }
    };
    install_at(max_mb, dir);
}

/// Installs an explicitly located cache without changing the storage folder.
pub fn install_at(max_mb: u64, dir: PathBuf) {
    if max_mb == 0 {
        return;
    }
    let sealed = dir.join(KEY_NAME).exists() || storage_is_encrypted(&crate::current_folder());
    apply(dir, max_mb.saturating_mul(1 << 20), sealed);
}

/// The sticky half: does the storage folder say "encrypted" on disk?
fn storage_is_encrypted(storage: &Path) -> bool {
    file::sealed_path(storage).exists() || storage.join("indexeddb").join("key.sealed").exists()
}

/// From [`crate::set_storage_folder`]: `Some(flag)` that differs from the cache's mode reinstalls
/// it in the new mode (purging it); `None` changes nothing.
pub fn storage_changed(encrypted: Option<bool>) {
    let Some(want) = encrypted else { return };
    let (dir, max_bytes, sealed) = {
        let st = STATE.lock().unwrap();
        match &st.dir {
            Some(d) => (d.clone(), st.max_bytes, st.sealed),
            None => return,
        }
    };
    if want != sealed {
        apply(dir, max_bytes, want);
    }
}

/// `{"dir":…,"sealed":…,"error":…}` for devtools.
pub fn info_json() -> String {
    let st = STATE.lock().unwrap();
    let dir = st.dir.as_deref().map(|d| d.display().to_string());
    format!(
        "{{\"dir\":{},\"sealed\":{},\"error\":{}}}",
        dir.map_or("null".into(), |d| crate::json_escape(&d)),
        st.sealed,
        st.error.as_deref().map_or("null".into(), crate::json_escape)
    )
}

fn apply(dir: PathBuf, max_bytes: u64, sealed: bool) {
    let sealer: Option<Arc<dyn Sealer>> = if sealed {
        match gcm::sealer(&dir) {
            Ok(s) => Some(s),
            Err(e) => {
                // Never plaintext after being asked for encryption.
                let msg = format!("the storage is encrypted and the HTTP cache's key could not be had ({e}); running without a cache");
                eprintln!("[storage] {msg}");
                cache::uninstall();
                *STATE.lock().unwrap() = State { dir: Some(dir), max_bytes, sealed, error: Some(msg) };
                return;
            }
        }
    } else {
        // Plaintext from now on: a leftover key would make the next start encrypted again.
        let _ = std::fs::remove_file(dir.join(KEY_NAME));
        None
    };
    let error = match cache::install(Config { dir: dir.clone(), max_bytes, sealer }) {
        Ok(_) => None,
        Err(e) => {
            eprintln!("[storage] the HTTP cache is off: {e}");
            Some(e)
        }
    };
    *STATE.lock().unwrap() = State { dir: Some(dir), max_bytes, sealed, error };
}

#[cfg(target_os = "windows")]
mod gcm {
    //! AES-256-GCM through CNG: `nonce (12) ‖ ciphertext ‖ tag (16)`.

    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use windows::Win32::Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenerateSymmetricKey,
        BCryptOpenAlgorithmProvider, BCryptSetProperty, BCRYPT_AES_ALGORITHM, BCRYPT_ALG_HANDLE,
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, BCRYPT_CHAINING_MODE,
        BCRYPT_CHAIN_MODE_GCM, BCRYPT_FLAGS, BCRYPT_HANDLE, BCRYPT_KEY_HANDLE, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS,
    };

    use super::KEY_NAME;
    use crate::{crypt, file};
    use vn_runtime::redjs::webapi::http::cache::Sealer;

    const NONCE: usize = 12;
    const TAG: usize = 16;

    struct Gcm {
        alg: BCRYPT_ALG_HANDLE,
        /// One key handle, used by one call at a time.
        key: Mutex<BCRYPT_KEY_HANDLE>,
        id: String,
    }

    // The handles are process-wide CNG objects; every key use goes through the mutex.
    unsafe impl Send for Gcm {}
    unsafe impl Sync for Gcm {}

    impl Drop for Gcm {
        fn drop(&mut self) {
            unsafe {
                let _ = BCryptDestroyKey(*self.key.lock().unwrap());
                let _ = BCryptCloseAlgorithmProvider(self.alg, 0);
            }
        }
    }

    fn check(st: windows::Win32::Foundation::NTSTATUS, what: &str) -> Result<(), String> {
        if st.0 < 0 {
            Err(format!("{what} failed: 0x{:08x}", st.0 as u32))
        } else {
            Ok(())
        }
    }

    fn info(nonce: &mut [u8], tag: &mut [u8]) -> BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
            cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
            dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
            pbNonce: nonce.as_mut_ptr(),
            cbNonce: nonce.len() as u32,
            pbTag: tag.as_mut_ptr(),
            cbTag: tag.len() as u32,
            ..Default::default()
        }
    }

    impl Gcm {
        fn new(secret: &[u8; 32], id: String) -> Result<Gcm, String> {
            unsafe {
                let mut alg = BCRYPT_ALG_HANDLE::default();
                check(
                    BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_AES_ALGORITHM, windows::core::PCWSTR::null(), BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)),
                    "BCryptOpenAlgorithmProvider(AES)",
                )?;
                let mode: Vec<u8> = BCRYPT_CHAIN_MODE_GCM
                    .as_wide()
                    .iter()
                    .chain(std::iter::once(&0u16))
                    .flat_map(|c| c.to_le_bytes())
                    .collect();
                if let Err(e) = check(BCryptSetProperty(BCRYPT_HANDLE(alg.0), BCRYPT_CHAINING_MODE, &mode, 0), "GCM mode") {
                    let _ = BCryptCloseAlgorithmProvider(alg, 0);
                    return Err(e);
                }
                let mut key = BCRYPT_KEY_HANDLE::default();
                if let Err(e) = check(BCryptGenerateSymmetricKey(alg, &mut key, None, secret, 0), "BCryptGenerateSymmetricKey") {
                    let _ = BCryptCloseAlgorithmProvider(alg, 0);
                    return Err(e);
                }
                Ok(Gcm { alg, key: Mutex::new(key), id })
            }
        }
    }

    impl Sealer for Gcm {
        fn tag(&self) -> String {
            format!("aes-256-gcm/{}", self.id)
        }

        fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
            let mut nonce = [0u8; NONCE];
            crypt::random_bytes(&mut nonce)?;
            let mut tag = [0u8; TAG];
            let mut out = vec![0u8; NONCE + plain.len() + TAG];
            out[..NONCE].copy_from_slice(&nonce);
            let mut n = 0u32;
            {
                let a = info(&mut nonce, &mut tag);
                let key = self.key.lock().unwrap();
                let body = &mut out[NONCE..NONCE + plain.len()];
                unsafe {
                    check(
                        BCryptEncrypt(
                            *key,
                            Some(plain),
                            Some(&a as *const _ as *const core::ffi::c_void),
                            None,
                            Some(body),
                            &mut n,
                            BCRYPT_FLAGS(0),
                        ),
                        "BCryptEncrypt",
                    )?;
                }
            }
            let end = out.len();
            out[end - TAG..].copy_from_slice(&tag);
            Ok(out)
        }

        fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, String> {
            if sealed.len() < NONCE + TAG {
                return Err("a sealed entry shorter than its nonce and tag".into());
            }
            let mut nonce: [u8; NONCE] = sealed[..NONCE].try_into().unwrap();
            let mut tag: [u8; TAG] = sealed[sealed.len() - TAG..].try_into().unwrap();
            let body = &sealed[NONCE..sealed.len() - TAG];
            let mut out = vec![0u8; body.len()];
            let mut n = 0u32;
            let a = info(&mut nonce, &mut tag);
            let key = self.key.lock().unwrap();
            unsafe {
                check(
                    BCryptDecrypt(
                        *key,
                        Some(body),
                        Some(&a as *const _ as *const core::ffi::c_void),
                        None,
                        Some(&mut out),
                        &mut n,
                        BCRYPT_FLAGS(0),
                    ),
                    "BCryptDecrypt (wrong key or a damaged entry)",
                )?;
            }
            out.truncate(n as usize);
            Ok(out)
        }
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
        if s.len() != N * 2 {
            return None;
        }
        let mut out = [0u8; N];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(out)
    }

    /// The folder's key (made on first use) as a [`Sealer`]. `key.sealed` holds
    /// `<64 hex key> <16 hex id>`; the id is in the sealer's tag, so a new key purges old entries.
    pub fn sealer(dir: &Path) -> Result<Arc<dyn Sealer>, String> {
        let p = crypt::provider().ok_or("no DPAPI on this platform")?;
        let path = dir.join(KEY_NAME);
        let existing = match std::fs::read(&path) {
            Ok(raw) => match file::unseal(p, &raw) {
                Ok(mut text) => {
                    let parsed = text.split_once(' ').and_then(|(k, id)| Some((unhex::<32>(k)?, unhex::<8>(id.trim())?)));
                    unsafe { text.as_bytes_mut().fill(0) };
                    parsed
                }
                Err(e) => {
                    // The entries are lost with the key; a cache can afford that.
                    eprintln!("[storage] the HTTP cache key cannot be unsealed ({e}); making a new one");
                    None
                }
            },
            Err(_) => None,
        };
        let (mut key, id) = match existing {
            Some(k) => k,
            None => {
                let mut key = [0u8; 32];
                let mut id = [0u8; 8];
                crypt::random_bytes(&mut key)?;
                crypt::random_bytes(&mut id)?;
                std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
                let mut text = format!("{} {}", hex(&key), hex(&id));
                let sealed = file::seal(p, &text);
                unsafe { text.as_bytes_mut().fill(0) };
                file::write_atomic(&path, &sealed?).map_err(|e| format!("could not write {}: {e}", path.display()))?;
                (key, id)
            }
        };
        let gcm = Gcm::new(&key, hex(&id));
        key.fill(0);
        Ok(Arc::new(gcm?))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn sealing_round_trips_and_refuses_another_key_or_a_flipped_bit() {
            let a = Gcm::new(&[7u8; 32], "a".into()).unwrap();
            let b = Gcm::new(&[8u8; 32], "b".into()).unwrap();
            let plain = b"an entry's headers and body".repeat(40);
            let sealed = a.seal(&plain).unwrap();
            assert_eq!(sealed.len(), plain.len() + NONCE + TAG);
            assert!(!sealed.windows(16).any(|w| w == &plain[..16]));
            assert_eq!(a.open(&sealed).unwrap(), plain);
            assert!(b.open(&sealed).is_err());
            let mut bad = sealed.clone();
            bad[NONCE + 5] ^= 1;
            assert!(a.open(&bad).is_err());
            assert_ne!(a.seal(&plain).unwrap(), sealed);
        }

        /// The macOS cipher (ring) and this one open each other's entries.
        #[test]
        fn ring_and_cng_agree() {
            let cng = Gcm::new(&[0x42; 32], "c".into()).unwrap();
            let ring = super::super::ring_gcm::Gcm::new(&[0x42; 32], "r".into()).unwrap();
            let plain = b"the same AES-256-GCM on both sides".repeat(3);
            assert_eq!(ring.open(&cng.seal(&plain).unwrap()).unwrap(), plain);
            assert_eq!(cng.open(&ring.seal(&plain).unwrap()).unwrap(), plain);
        }

        #[test]
        fn the_key_file_is_made_once_and_read_back() {
            let dir = std::env::temp_dir().join(format!("vn-storage-http-key-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let first = sealer(&dir).unwrap();
            let raw = std::fs::read(dir.join(KEY_NAME)).unwrap();
            let again = sealer(&dir).unwrap();
            assert_eq!(first.tag(), again.tag());
            assert_eq!(again.open(&first.seal(b"x").unwrap()).unwrap(), b"x");
            assert!(!raw.windows(8).any(|w| w.iter().all(|c| c.is_ascii_hexdigit())));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// AES-256-GCM through ring (macOS, Linux); under `test` everywhere, to check it against CNG's.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod ring_gcm {
    use std::path::Path;
    use std::sync::Arc;

    use super::KEY_NAME;
    use crate::crypt::envelope::{fill_random, Aes256Gcm};
    use crate::crypt::SecretProvider;
    use crate::file;
    use vn_runtime::redjs::webapi::http::cache::Sealer;

    pub struct Gcm {
        aead: Aes256Gcm,
        id: String,
    }

    impl Gcm {
        pub fn new(secret: &[u8; 32], id: String) -> Result<Gcm, String> {
            Ok(Gcm { aead: Aes256Gcm::new(secret)?, id })
        }
    }

    impl Sealer for Gcm {
        fn tag(&self) -> String {
            format!("aes-256-gcm/{}", self.id)
        }

        fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
            self.aead.seal(b"", plain)
        }

        fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, String> {
            self.aead.open(b"", sealed)
        }
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
        if s.len() != N * 2 {
            return None;
        }
        let mut out = [0u8; N];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(out)
    }

    /// The Windows `gcm::sealer`'s key file under any provider: `key.sealed` holds
    /// `<64 hex key> <16 hex id>` sealed by `p`.
    pub fn sealer_with(dir: &Path, p: &dyn SecretProvider) -> Result<Arc<dyn Sealer>, String> {
        // A locked or refused keychain says nothing about the key file: refuse (no cache this
        // run) rather than replace a key that may be fine.
        p.ready()?;
        let path = dir.join(KEY_NAME);
        let existing = match std::fs::read(&path) {
            Ok(raw) => match file::unseal(p, &raw) {
                Ok(mut text) => {
                    let parsed = text.split_once(' ').and_then(|(k, id)| Some((unhex::<32>(k)?, unhex::<8>(id.trim())?)));
                    unsafe { text.as_bytes_mut().fill(0) };
                    parsed
                }
                Err(e) => {
                    eprintln!("[storage] the HTTP cache key cannot be unsealed ({e}); making a new one");
                    None
                }
            },
            Err(_) => None,
        };
        let (mut key, id) = match existing {
            Some(k) => k,
            None => {
                let mut key = [0u8; 32];
                let mut id = [0u8; 8];
                fill_random(&mut key)?;
                fill_random(&mut id)?;
                std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
                let mut text = format!("{} {}", hex(&key), hex(&id));
                let sealed = file::seal(p, &text);
                unsafe { text.as_bytes_mut().fill(0) };
                file::write_atomic(&path, &sealed?).map_err(|e| format!("could not write {}: {e}", path.display()))?;
                (key, id)
            }
        };
        let gcm = Gcm::new(&key, hex(&id));
        key.fill(0);
        Ok(Arc::new(gcm?))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::crypt::envelope::FixedKey;

        #[test]
        fn sealing_round_trips_and_refuses_another_key_or_a_flipped_bit() {
            let a = Gcm::new(&[7u8; 32], "a".into()).unwrap();
            let b = Gcm::new(&[8u8; 32], "b".into()).unwrap();
            let plain = b"an entry's headers and body".repeat(40);
            let sealed = a.seal(&plain).unwrap();
            assert_eq!(sealed.len(), plain.len() + 12 + 16);
            assert!(!sealed.windows(16).any(|w| w == &plain[..16]));
            assert_eq!(a.open(&sealed).unwrap(), plain);
            assert!(b.open(&sealed).is_err());
            let mut bad = sealed.clone();
            bad[12 + 5] ^= 1;
            assert!(a.open(&bad).is_err());
            assert!(a.open(&sealed[..20]).is_err());
            assert_ne!(a.seal(&plain).unwrap(), sealed);
        }

        #[test]
        fn the_key_file_is_made_once_read_back_and_replaced_only_when_unreadable() {
            let dir = std::env::temp_dir().join(format!("vn-storage-http-ring-key-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let p = FixedKey::new(3);
            let first = sealer_with(&dir, &p).unwrap();
            let raw = std::fs::read(dir.join(KEY_NAME)).unwrap();
            assert_eq!(&raw[..8], b"VNSTOR\x01\x02");
            assert!(!raw.windows(8).any(|w| w.iter().all(|c| c.is_ascii_hexdigit())), "the key is legible");
            let again = sealer_with(&dir, &p).unwrap();
            assert_eq!(first.tag(), again.tag());
            assert_eq!(again.open(&first.seal(b"x").unwrap()).unwrap(), b"x");
            // Another key (another user, a replaced keychain item) gives a new tag; redjs purges.
            let other = sealer_with(&dir, &FixedKey::new(4)).unwrap();
            assert_ne!(other.tag(), first.tag());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_provider_that_is_not_ready_leaves_the_key_file_alone() {
            struct Locked;
            impl SecretProvider for Locked {
                fn name(&self) -> &'static str { "locked" }
                fn id(&self) -> u8 { 2 }
                fn ready(&self) -> Result<(), String> { Err("macOS Keychain: locked".into()) }
                fn protect(&self, _: &[u8]) -> Result<Vec<u8>, String> { Err("locked".into()) }
                fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, String> { Err("locked".into()) }
            }
            let dir = std::env::temp_dir().join(format!("vn-storage-http-ring-locked-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(KEY_NAME), b"an existing key file").unwrap();
            assert!(sealer_with(&dir, &Locked).err().unwrap().contains("locked"));
            assert_eq!(std::fs::read(dir.join(KEY_NAME)).unwrap(), b"an existing key file");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod gcm {
    use std::path::Path;
    use std::sync::Arc;
    use vn_runtime::redjs::webapi::http::cache::Sealer;

    /// The folder's key, sealed by the platform provider, as a [`Sealer`].
    pub fn sealer(dir: &Path) -> Result<Arc<dyn Sealer>, String> {
        let p = crate::crypt::provider().ok_or("no storage key provider on this platform")?;
        super::ring_gcm::sealer_with(dir, p)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod gcm {
    use std::path::Path;
    use std::sync::Arc;
    use vn_runtime::redjs::webapi::http::cache::Sealer;

    pub fn sealer(_dir: &Path) -> Result<Arc<dyn Sealer>, String> {
        Err(String::from("no cipher on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_is_beside_storage_under_the_app_name() {
        let d = folder_for("my-app").unwrap();
        assert!(d.ends_with(Path::new("my-app").join("cache").join("http")), "{}", d.display());
        assert!(folder_for("../escape").is_err());
    }

    #[test]
    fn the_sticky_rule_reads_the_storage_folder() {
        let dir = std::env::temp_dir().join(format!("vn-storage-http-sticky-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("indexeddb")).unwrap();
        assert!(!storage_is_encrypted(&dir));
        std::fs::write(dir.join("indexeddb").join("key.sealed"), b"x").unwrap();
        assert!(storage_is_encrypted(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
