//! The macOS and Linux providers' platform-neutral half: the sealed envelope,
//! the AES-256-GCM cipher it (and the HTTP cache) uses, the first-run key
//! creation protocol, and what a Keychain status code means.
//!
//! Compiled for `cfg(any(target_os = "macos", target_os = "linux", test))`:
//! on a Mac and on Linux it is the provider (the key in the Keychain, or in the
//! Secret Service), elsewhere it exists only so `cargo test` can check it. None
//! of it touches a key store; `keychain.rs` and `secret_service.rs` do.
//!
//! # The envelope (version 1)
//!
//! What `SecretProvider::protect` returns, i.e. what follows the 8-byte
//! `VNSTOR | 1 | 2` file header that `file::seal` writes:
//!
//! ```text
//!   offset  size  field
//!   0       1     envelope version            = 1
//!   1       4     key id                      = SHA-256("vue-native/storage/v1/key-id" ‖ key)[..4]
//!   5       12    nonce                       random, fresh per blob
//!   17      n     ciphertext                  AES-256-GCM
//!   17+n    16    tag
//!
//!   associated data = "vue-native/storage/v1" ‖ version ‖ key id
//! ```
//!
//! - **The version** is authenticated (it is in the associated data), so a
//!   future layout cannot be confused with this one by editing one byte.
//! - **The key id** turns "the keychain item was replaced" (or "this file is
//!   another user's") into its own error rather than a generic tag failure. It
//!   is 32 bits of a hash of a random 256-bit key and says nothing about it.
//! - **The context string** is DPAPI's entropy on Windows, used for the same
//!   purpose: a blob of ours opens only for code that knows it is ours.
//! - **Random 96-bit nonces** under one key are good for 2^32 blobs (NIST SP
//!   800-38D §8.3). The busiest writer is one `localStorage` write per frame:
//!   decades of continuous writing.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

/// The byte in the `VNSTOR` file header that names this provider (DPAPI is 1).
#[cfg(any(target_os = "macos", test))]
pub const PROVIDER_ID: u8 = 2;
/// The envelope layout above.
pub const VERSION: u8 = 1;
pub const KEY_LEN: usize = 32;
const KEY_ID_LEN: usize = 4;
const TAG_LEN: usize = 16;
/// Version + key id.
const HEADER_LEN: usize = 1 + KEY_ID_LEN;
/// The same string DPAPI mixes in as entropy on Windows.
const CONTEXT: &[u8] = b"vue-native/storage/v1";
const AAD_LEN: usize = CONTEXT.len() + HEADER_LEN;

/// `errSecItemNotFound`: no such item. The one "error" that means "make it".
#[cfg(any(target_os = "macos", test))]
pub const ERR_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecDuplicateItem`: someone else made it first. Re-read, never replace.
#[cfg(any(target_os = "macos", test))]
pub const ERR_DUPLICATE_ITEM: i32 = -25299;

/// Fills `buf` from the system CSPRNG.
pub fn fill_random(buf: &mut [u8]) -> Result<(), String> {
    SystemRandom::new()
        .fill(buf)
        .map_err(|_| String::from("the system random generator failed"))
}

// ---------------------------------------------------------------------------
// The cipher
// ---------------------------------------------------------------------------

/// AES-256-GCM with a fresh random nonce per message, laid out as
/// `nonce (12) ‖ ciphertext ‖ tag (16)` — the same layout the Windows HTTP
/// cache gets from CNG, so one shape describes both.
pub struct Aes256Gcm(LessSafeKey);

impl Aes256Gcm {
    pub fn new(key: &[u8]) -> Result<Aes256Gcm, String> {
        if key.len() != KEY_LEN {
            return Err(format!("an AES-256 key is {KEY_LEN} bytes, not {}", key.len()));
        }
        let key = UnboundKey::new(&AES_256_GCM, key).map_err(|_| String::from("AES-256-GCM refused the key"))?;
        Ok(Aes256Gcm(LessSafeKey::new(key)))
    }

    pub fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut nonce)?;
        let mut out = Vec::with_capacity(NONCE_LEN + plain.len() + TAG_LEN);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(plain);
        let tag = self
            .0
            .seal_in_place_separate_tag(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut out[NONCE_LEN..])
            .map_err(|_| String::from("AES-256-GCM sealing failed"))?;
        out.extend_from_slice(tag.as_ref());
        Ok(out)
    }

    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, String> {
        if sealed.len() < NONCE_LEN + TAG_LEN {
            return Err(String::from("a sealed entry shorter than its nonce and tag"));
        }
        let nonce = Nonce::try_assume_unique_for_key(&sealed[..NONCE_LEN])
            .map_err(|_| String::from("a sealed entry with a malformed nonce"))?;
        let mut buf = sealed[NONCE_LEN..].to_vec();
        match self.0.open_in_place(nonce, Aad::from(aad), &mut buf) {
            Ok(plain) => {
                let n = plain.len();
                buf.truncate(n);
                Ok(buf)
            }
            Err(_) => {
                buf.fill(0);
                Err(String::from("AES-256-GCM authentication failed (wrong key or damaged data)"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The envelope
// ---------------------------------------------------------------------------

/// SHA-256("vue-native/storage/v1/key-id" ‖ key), first four bytes.
fn key_id(key: &[u8]) -> [u8; KEY_ID_LEN] {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    ctx.update(CONTEXT);
    ctx.update(b"/key-id");
    ctx.update(key);
    let d = ctx.finish();
    let mut id = [0u8; KEY_ID_LEN];
    id.copy_from_slice(&d.as_ref()[..KEY_ID_LEN]);
    id
}

fn aad(version: u8, id: &[u8]) -> [u8; AAD_LEN] {
    let mut a = [0u8; AAD_LEN];
    a[..CONTEXT.len()].copy_from_slice(CONTEXT);
    a[CONTEXT.len()] = version;
    a[CONTEXT.len() + 1..].copy_from_slice(id);
    a
}

/// The provider's master key, ready to seal and open envelopes.
pub struct MasterKey {
    aead: Aes256Gcm,
    id: [u8; KEY_ID_LEN],
}

impl MasterKey {
    pub fn new(key: &[u8]) -> Result<MasterKey, String> {
        Ok(MasterKey { aead: Aes256Gcm::new(key)?, id: key_id(key) })
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        let body = self.aead.seal(&aad(VERSION, &self.id), plain)?;
        let mut out = Vec::with_capacity(HEADER_LEN + body.len());
        out.push(VERSION);
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&body);
        Ok(out)
    }

    pub fn open(&self, blob: &[u8]) -> Result<Vec<u8>, String> {
        if blob.len() < HEADER_LEN {
            return Err(String::from("a sealed blob shorter than its header"));
        }
        if blob[0] != VERSION {
            return Err(format!(
                "sealed envelope version {} — this build reads version {VERSION}",
                blob[0]
            ));
        }
        let id = &blob[1..HEADER_LEN];
        if id != self.id {
            return Err(String::from(
                "sealed under a different storage key (the key's keychain or keyring item was \
                 deleted or replaced, or the file comes from another user or another machine)",
            ));
        }
        self.aead
            .open(&aad(blob[0], id), &blob[HEADER_LEN..])
            .map_err(|_| String::from("the sealed data is damaged or was tampered with"))
    }
}

// ---------------------------------------------------------------------------
// First run: find the key, or make it exactly once
// ---------------------------------------------------------------------------

/// How [`load_or_create`] came by the key — for the log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// It was already there.
    Found,
    /// This process made it.
    Created,
    /// Another process made it between our read and our add; this is theirs.
    Raced,
}

/// The key, from `read` (`Ok(None)` = no such item; `Err` = a Keychain
/// status), or created with `fresh` and stored with `add`.
///
/// `add` must be add-ONLY (`SecItemAdd`, never an update): if another process
/// created the item first it fails with [`ERR_DUPLICATE_ITEM`] and the item
/// is re-read, so both processes end up with the one key that is stored. An
/// upsert there would let the second process overwrite the first's key after
/// the first had already sealed data with it.
///
/// Nothing is created after a read that FAILED (as opposed to found nothing):
/// a locked keychain or a refused prompt does not mean the key is gone, and a
/// new key would orphan everything sealed under the old one. Nor is an item
/// that is not a 256-bit key replaced.
#[cfg(any(target_os = "macos", test))]
pub fn load_or_create(
    read: impl FnMut() -> Result<Option<Vec<u8>>, i32>,
    add: impl FnOnce(&[u8; KEY_LEN]) -> Result<(), i32>,
    fresh: impl FnOnce() -> Result<[u8; KEY_LEN], String>,
) -> Result<([u8; KEY_LEN], Origin), String> {
    load_or_create_in(
        "macOS Keychain",
        read,
        add,
        fresh,
        |code| *code == ERR_DUPLICATE_ITEM,
        |op, code| keychain_error(op, *code),
    )
}

/// [`load_or_create`] over any key store: `store` names it in errors,
/// `is_duplicate` says an add lost the race, `describe` turns a store error
/// into the message for `op` (`read`, `re-read`, `store`).
pub fn load_or_create_in<E>(
    store: &str,
    mut read: impl FnMut() -> Result<Option<Vec<u8>>, E>,
    add: impl FnOnce(&[u8; KEY_LEN]) -> Result<(), E>,
    fresh: impl FnOnce() -> Result<[u8; KEY_LEN], String>,
    is_duplicate: impl Fn(&E) -> bool,
    describe: impl Fn(&str, &E) -> String,
) -> Result<([u8; KEY_LEN], Origin), String> {
    if let Some(bytes) = read().map_err(|e| describe("read", &e))? {
        return take_key(store, bytes).map(|k| (k, Origin::Found));
    }
    let mut key = fresh()?;
    match add(&key) {
        Ok(()) => Ok((key, Origin::Created)),
        Err(e) if is_duplicate(&e) => {
            key.fill(0);
            match read().map_err(|e| describe("re-read", &e))? {
                Some(bytes) => take_key(store, bytes).map(|k| (k, Origin::Raced)),
                None => Err(format!("{store}: the storage key was reported both present (on add) and missing (on read)")),
            }
        }
        Err(e) => {
            key.fill(0);
            Err(describe("store", &e))
        }
    }
}

fn take_key(store: &str, mut bytes: Vec<u8>) -> Result<[u8; KEY_LEN], String> {
    let r = <[u8; KEY_LEN]>::try_from(bytes.as_slice()).map_err(|_| {
        format!(
            "{store}: the storage key item holds {} bytes, not a 256-bit key; it is left as it is \
             (replacing it would lose everything sealed under it)",
            bytes.len()
        )
    });
    bytes.fill(0);
    r
}

/// What a Keychain `OSStatus` means for this crate's purposes, as an error
/// for `appStorage.info()` and the boot error. Always carries the code.
#[cfg(any(target_os = "macos", test))]
pub fn keychain_error(op: &str, code: i32) -> String {
    let what = match code {
        ERR_ITEM_NOT_FOUND => "the item is not in the keychain",
        ERR_DUPLICATE_ITEM => "the item already exists",
        -25308 => "user interaction is not allowed: the keychain is locked and cannot ask to be unlocked here \
                   (no GUI session, e.g. over ssh or before login)",
        -128 => "the keychain prompt was cancelled",
        -25293 => "keychain access was denied (the unlock or access prompt was refused)",
        -25243 => "this app is not on the storage key's access list",
        -25294 | -25307 => "this user has no default keychain",
        -25291 => "keychain services are not available",
        -34018 => "the app's code signature lacks a required keychain entitlement",
        -61 => "the keychain file is not writable",
        _ => "unexpected keychain error",
    };
    format!("macOS Keychain: could not {op} the storage key: {what} (OSStatus {code})")
}

/// The keychain account that names an app's key: its bundle identifier (what
/// macOS itself names an app by, and stable across moves and updates), else
/// `exe:<file stem>` for a bare binary, else a fixed name.
pub fn account_for(bundle_id: Option<&str>, exe_stem: Option<&str>) -> String {
    if let Some(id) = bundle_id.map(str::trim).filter(|s| !s.is_empty()) {
        return id.to_string();
    }
    match exe_stem.map(str::trim).filter(|s| !s.is_empty()) {
        Some(stem) => format!("exe:{stem}"),
        None => String::from("vue-native-app"),
    }
}

/// A provider over a fixed key, for tests that need the real envelope without
/// a keychain.
#[cfg(test)]
pub(crate) struct FixedKey(pub MasterKey);

#[cfg(test)]
impl FixedKey {
    pub fn new(byte: u8) -> FixedKey {
        FixedKey(MasterKey::new(&[byte; KEY_LEN]).unwrap())
    }
}

#[cfg(test)]
impl super::SecretProvider for FixedKey {
    fn name(&self) -> &'static str {
        "test key (macOS envelope)"
    }
    fn id(&self) -> u8 {
        PROVIDER_ID
    }
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        self.0.seal(plaintext)
    }
    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        self.0.open(ciphertext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn key(b: u8) -> MasterKey {
        MasterKey::new(&[b; KEY_LEN]).unwrap()
    }

    #[test]
    fn an_envelope_round_trips_and_has_the_documented_layout() {
        let k = key(1);
        for plain in [&b""[..], b"a", b"{\"entries\":[[\"token\",\"hunter2\"]]}", &[0u8, 255, 128][..]] {
            let blob = k.seal(plain).unwrap();
            assert_eq!(blob.len(), HEADER_LEN + NONCE_LEN + plain.len() + TAG_LEN);
            assert_eq!(blob[0], VERSION);
            assert_eq!(&blob[1..5], &key_id(&[1; KEY_LEN]));
            assert_eq!(k.open(&blob).unwrap(), plain);
        }
    }

    #[test]
    fn the_ciphertext_hides_the_plaintext_and_every_seal_has_a_fresh_nonce() {
        let k = key(2);
        let secret = b"correct-horse-battery-staple";
        let a = k.seal(secret).unwrap();
        let b = k.seal(secret).unwrap();
        assert!(!a.windows(secret.len()).any(|w| w == secret));
        assert_ne!(a[HEADER_LEN..HEADER_LEN + NONCE_LEN], b[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        assert_ne!(a, b);
    }

    #[test]
    fn each_failure_says_which_check_failed() {
        let k = key(3);
        let blob = k.seal(b"payload").unwrap();
        assert!(k.open(b"").unwrap_err().contains("shorter than its header"));
        assert!(k.open(&blob[..HEADER_LEN + 3]).unwrap_err().contains("damaged"));
        let mut v = blob.clone();
        v[0] = 9;
        assert!(k.open(&v).unwrap_err().contains("version 9"));
        assert!(key(4).open(&blob).unwrap_err().contains("different storage key"));
        let mut id = blob.clone();
        id[2] ^= 1;
        assert!(k.open(&id).unwrap_err().contains("different storage key"));
        for i in [HEADER_LEN, HEADER_LEN + NONCE_LEN, blob.len() - 1] {
            let mut t = blob.clone();
            t[i] ^= 0x80;
            assert!(k.open(&t).unwrap_err().contains("damaged"), "byte {i}");
        }
    }

    #[test]
    fn the_header_is_authenticated_not_just_checked() {
        // A key whose id matches but whose header was forged is impossible to
        // build by hand; what can be checked is that the associated data the
        // tag covers is the header: the raw cipher refuses another one.
        let raw = Aes256Gcm::new(&[5; KEY_LEN]).unwrap();
        let sealed = raw.seal(&aad(VERSION, &key_id(&[5; KEY_LEN])), b"x").unwrap();
        assert_eq!(raw.open(&aad(VERSION, &key_id(&[5; KEY_LEN])), &sealed).unwrap(), b"x");
        assert!(raw.open(&aad(VERSION + 1, &key_id(&[5; KEY_LEN])), &sealed).is_err());
        assert!(raw.open(b"", &sealed).is_err());
    }

    #[test]
    fn only_a_256_bit_key_is_accepted() {
        assert!(MasterKey::new(&[0; 16]).is_err());
        assert!(Aes256Gcm::new(&[0; 33]).is_err());
        assert!(MasterKey::new(&[0; 32]).is_ok());
    }

    #[test]
    fn aes_gcm_matches_a_published_test_vector() {
        // NIST GCM test case 14 (AES-256, zero key, zero IV, one zero block):
        // the layout is nonce ‖ ciphertext ‖ tag, and the cipher is the
        // standard one, not merely self-consistent.
        let g = Aes256Gcm::new(&[0; 32]).unwrap();
        let mut sealed = vec![0u8; 12];
        sealed.extend_from_slice(&hex("cea7403d4d606b6e074ec5d3baf39d18"));
        sealed.extend_from_slice(&hex("d0d1c8a799996bf0265b98b5d48ab919"));
        assert_eq!(g.open(b"", &sealed).unwrap(), vec![0u8; 16]);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn an_existing_key_is_used_and_nothing_is_added() {
        let added = Cell::new(false);
        let (k, origin) = load_or_create(
            || Ok(Some(vec![7; KEY_LEN])),
            |_| {
                added.set(true);
                Ok(())
            },
            || panic!("no key is generated when one exists"),
        )
        .unwrap();
        assert_eq!((k, origin), ([7; KEY_LEN], Origin::Found));
        assert!(!added.get());
    }

    #[test]
    fn a_missing_key_is_created_once_and_stored() {
        let stored = RefCell::new(None);
        let (k, origin) = load_or_create(
            || Ok(None),
            |k| {
                *stored.borrow_mut() = Some(*k);
                Ok(())
            },
            || Ok([9; KEY_LEN]),
        )
        .unwrap();
        assert_eq!((k, origin), ([9; KEY_LEN], Origin::Created));
        assert_eq!(*stored.borrow(), Some([9; KEY_LEN]));
    }

    #[test]
    fn losing_the_creation_race_adopts_the_winners_key() {
        // Two processes read "missing"; the other one's add lands first, ours
        // fails as a duplicate, and we must use THEIR key, never ours.
        let reads = Cell::new(0);
        let (k, origin) = load_or_create(
            || {
                reads.set(reads.get() + 1);
                Ok(if reads.get() == 1 { None } else { Some(vec![0xAA; KEY_LEN]) })
            },
            |_| Err(ERR_DUPLICATE_ITEM),
            || Ok([0x55; KEY_LEN]),
        )
        .unwrap();
        assert_eq!((k, origin), ([0xAA; KEY_LEN], Origin::Raced));
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn a_failed_read_never_creates_a_key() {
        // A locked keychain, a refused prompt: the key may well exist. A new
        // one would orphan everything sealed under it.
        for code in [-25308, -128, -25293] {
            let e = load_or_create(|| Err(code), |_| panic!("no add after a failed read"), || panic!("no new key")).unwrap_err();
            assert!(e.contains(&format!("OSStatus {code}")), "{e}");
            assert!(e.contains("could not read"), "{e}");
        }
    }

    #[test]
    fn a_malformed_item_is_refused_not_replaced() {
        let e = load_or_create(|| Ok(Some(vec![1; 16])), |_| panic!("never replaced"), || panic!("never regenerated")).unwrap_err();
        assert!(e.contains("16 bytes"), "{e}");
    }

    #[test]
    fn a_failed_add_or_a_vanished_item_is_an_error() {
        let e = load_or_create(|| Ok(None), |_| Err(-61), || Ok([1; KEY_LEN])).unwrap_err();
        assert!(e.contains("could not store") && e.contains("not writable"), "{e}");
        let e = load_or_create(|| Ok(None), |_| Err(ERR_DUPLICATE_ITEM), || Ok([1; KEY_LEN])).unwrap_err();
        assert!(e.contains("both present"), "{e}");
        let e = load_or_create(|| Ok(None), |_| Ok(()), || Err(String::from("no entropy"))).unwrap_err();
        assert_eq!(e, "no entropy");
    }

    #[test]
    fn keychain_statuses_are_named_and_always_carry_the_code() {
        assert!(keychain_error("read", -25308).contains("locked"));
        assert!(keychain_error("read", -34018).contains("entitlement"));
        assert!(keychain_error("read", -1).contains("unexpected") && keychain_error("read", -1).contains("OSStatus -1"));
    }

    #[test]
    fn the_account_is_the_bundle_id_else_the_executable() {
        assert_eq!(account_for(Some("gl.argon.client"), Some("argon")), "gl.argon.client");
        assert_eq!(account_for(Some(""), Some("vn-app")), "exe:vn-app");
        assert_eq!(account_for(None, Some("vn-app")), "exe:vn-app");
        assert_eq!(account_for(None, None), "vue-native-app");
    }

    #[test]
    fn the_file_header_names_this_provider_and_another_provider_is_refused() {
        // Through `file::seal`, exactly as localStorage, cookies, host secrets
        // and the key files use it.
        let p = FixedKey::new(1);
        let sealed = crate::file::seal(&p, "{\"secret\":1}").unwrap();
        assert_eq!(&sealed[..8], b"VNSTOR\x01\x02");
        assert!(!sealed.windows(6).any(|w| w == b"secret"));
        assert_eq!(crate::file::unseal(&p, &sealed).unwrap(), "{\"secret\":1}");
        let mut dpapi = sealed.clone();
        dpapi[7] = 1;
        assert!(crate::file::unseal(&p, &dpapi).unwrap_err().contains("provider 1"));
    }
}
