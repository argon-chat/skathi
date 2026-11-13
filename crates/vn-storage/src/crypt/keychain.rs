//! The macOS provider's Keychain half: the only code here that needs a Mac. One generic-password item
//! per app in the default (login) keychain (service `gl.argon.vue-native.storage`).
//!
//! The file-based login keychain, not the data-protection one, which needs an entitlement unsigned dev
//! builds lack. Another app (or an ad-hoc rebuild) makes macOS prompt out of process, which blocks this
//! call but cannot deadlock the UI thread. The key is loaded once per process, failure included, so a
//! refused prompt does not reappear every frame and a run that could not open old data never writes
//! over it.

use std::sync::OnceLock;

use core_foundation::base::TCFType;
use core_foundation::data::CFData;
use core_foundation::string::CFString;
use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};
use security_framework::passwords::{generic_password, PasswordOptions};

use super::envelope::{self, MasterKey, Origin, KEY_LEN};
use super::SecretProvider;

pub const SERVICE: &str = "gl.argon.vue-native.storage";

pub struct Keychain {
    key: OnceLock<Result<MasterKey, String>>,
}

impl Keychain {
    pub const fn new() -> Keychain {
        Keychain { key: OnceLock::new() }
    }

    fn key(&self) -> Result<&MasterKey, String> {
        self.key.get_or_init(|| load(&account())).as_ref().map_err(String::clone)
    }
}

impl SecretProvider for Keychain {
    fn name(&self) -> &'static str {
        "macOS Keychain + AES-256-GCM (per-user)"
    }

    fn id(&self) -> u8 {
        envelope::PROVIDER_ID
    }

    fn ready(&self) -> Result<(), String> {
        self.key().map(|_| ())
    }

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        self.key()?.seal(plaintext)
    }

    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        self.key()?.open(ciphertext)
    }
}

fn account() -> String {
    // SAFETY: CFBundleGetMainBundle and CFBundleGetIdentifier follow the Get
    // rule (nothing to release); both may be null and are checked.
    let bundle_id = unsafe {
        use core_foundation::bundle::{CFBundleGetIdentifier, CFBundleGetMainBundle};
        let bundle = CFBundleGetMainBundle();
        if bundle.is_null() {
            None
        } else {
            let id = CFBundleGetIdentifier(bundle);
            (!id.is_null()).then(|| CFString::wrap_under_get_rule(id).to_string())
        }
    };
    let stem = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()));
    envelope::account_for(bundle_id.as_deref(), stem.as_deref())
}

fn load(account: &str) -> Result<MasterKey, String> {
    let read = || match generic_password(PasswordOptions::new_generic_password(SERVICE, account)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.code() == envelope::ERR_ITEM_NOT_FOUND => Ok(None),
        Err(e) => Err(e.code()),
    };
    // SecItemAdd only: `set_generic_password` would update an existing item, the very race
    // `load_or_create` prevents.
    let add = |key: &[u8; KEY_LEN]| {
        let mut item = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(key),
        });
        item.set_service(SERVICE)
            .set_account_name(account)
            .set_label(format!("vue-native storage key ({account})"))
            .set_description("encryption key for this app's local data")
            .set_location(Location::DefaultFileKeychain);
        item.add().map_err(|e| e.code())
    };
    let fresh = || {
        let mut k = [0u8; KEY_LEN];
        envelope::fill_random(&mut k)?;
        Ok(k)
    };
    let (mut bytes, origin) = envelope::load_or_create(read, add, fresh)?;
    let key = MasterKey::new(&bytes);
    bytes.fill(0);
    match origin {
        Origin::Found => {}
        Origin::Created => {
            eprintln!("[storage] a new storage key is in the login keychain (service {SERVICE}, account {account})")
        }
        Origin::Raced => {
            eprintln!("[storage] another process created the storage key first (account {account}); using that one")
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use security_framework::passwords::delete_generic_password;

    #[test]
    #[ignore = "touches the login keychain; run on a Mac with --ignored"]
    fn keychain_a_key_is_created_once_and_read_back() {
        let account = format!("vn-storage-test-{}", std::process::id());
        let _ = delete_generic_password(SERVICE, &account);
        let first = load(&account).expect("create");
        let again = load(&account).expect("read back");
        assert_eq!(again.open(&first.seal(b"x").unwrap()).unwrap(), b"x");
        delete_generic_password(SERVICE, &account).expect("clean up");
    }

    #[test]
    #[ignore = "touches the login keychain; run on a Mac with --ignored"]
    fn keychain_racing_creators_end_up_with_one_key() {
        let account = format!("vn-storage-race-{}", std::process::id());
        let _ = delete_generic_password(SERVICE, &account);
        let keys: Vec<MasterKey> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..8).map(|_| s.spawn(|| load(&account).expect("load"))).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let sealed = keys[0].seal(b"one key").unwrap();
        for k in &keys {
            assert_eq!(k.open(&sealed).unwrap(), b"one key");
        }
        delete_generic_password(SERVICE, &account).expect("clean up");
    }
}
