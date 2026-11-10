//! The Linux provider's key store: the Secret Service, else a key file. The
//! envelope and the master key are `envelope.rs`, as on macOS.
//!
//! One item per app in the user's default collection ("login"):
//!
//! ```text
//!   service  gl.argon.vue-native.storage
//!   account  exe:<file stem>
//!   secret   32 random bytes — the master key
//! ```
//!
//! - **No add-only call.** `CreateItem` with `replace = false` adds a second
//!   item rather than failing, so two processes racing on first run each add
//!   one. Every read takes the OLDEST matching item; the loser of a race sees
//!   that its own is not it, deletes it and uses the winner's.
//! - **Absent, not refused.** The key file is used only where nothing owns
//!   `org.freedesktop.secrets` on the session bus (or there is no session bus).
//!   A service that is there but locked, or whose unlock prompt was dismissed,
//!   is an error latched for the process: a new key would orphan the data
//!   sealed under the one the keyring holds.
//! - **A key file is sticky.** Once one exists it is the key, whatever the
//!   bus offers later.
//! - **Loaded once per process**, success or failure, like the Keychain.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::OnceLock;

use futures_lite::future::block_on;
use oo7::dbus::{Collection, Error as DbusError, Service};

use super::envelope::{self, MasterKey, Origin, KEY_LEN};
use super::SecretProvider;

/// The Secret Service attribute every vue-native app's key is filed under.
pub const SERVICE: &str = "gl.argon.vue-native.storage";
/// This provider's byte in the `VNSTOR` file header.
const PROVIDER_ID: u8 = 3;

pub struct SecretService {
    key: OnceLock<Result<(MasterKey, &'static str), String>>,
}

impl SecretService {
    pub const fn new() -> SecretService {
        SecretService { key: OnceLock::new() }
    }

    fn key(&self) -> Result<&MasterKey, String> {
        self.key.get_or_init(|| load(&account())).as_ref().map(|(k, _)| k).map_err(String::clone)
    }
}

impl SecretProvider for SecretService {
    fn name(&self) -> &'static str {
        match self.key.get() {
            Some(Ok((_, store))) => *store,
            _ => KEYRING,
        }
    }

    fn id(&self) -> u8 {
        PROVIDER_ID
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

const KEYRING: &str = "Linux Secret Service + AES-256-GCM (per-user)";
const KEY_FILE: &str = "Linux key file + AES-256-GCM (per-user, no Secret Service)";

/// This app's account name: see [`envelope::account_for`].
fn account() -> String {
    envelope::account_for(None, crate::exe_stem().as_deref())
}

fn load(account: &str) -> Result<(MasterKey, &'static str), String> {
    let file = key_file(account)?;
    let (mut bytes, origin, store) = if file.exists() {
        let (bytes, origin) = from_file(&file)?;
        (bytes, origin, KEY_FILE)
    } else {
        match block_on(Service::new()) {
            Ok(service) => {
                let (bytes, origin) = from_keyring(&service, account)?;
                (bytes, origin, KEYRING)
            }
            Err(e) if absent(&e) => {
                eprintln!("[storage] no Secret Service on this session ({e}); the storage key is a file");
                let (bytes, origin) = from_file(&file)?;
                (bytes, origin, KEY_FILE)
            }
            Err(e) => return Err(format!("Secret Service: could not connect: {e}")),
        }
    };
    let key = MasterKey::new(&bytes);
    bytes.fill(0);
    match (origin, store) {
        (Origin::Found, _) => {}
        (Origin::Created, KEYRING) => {
            eprintln!("[storage] a new storage key is in the Secret Service (service {SERVICE}, account {account})")
        }
        (Origin::Created, _) => eprintln!("[storage] a new storage key is in {}", file.display()),
        (Origin::Raced, _) => {
            eprintln!("[storage] another process created the storage key first (account {account}); using that one")
        }
    }
    key.map(|k| (k, store))
}

fn fresh() -> Result<[u8; KEY_LEN], String> {
    let mut k = [0u8; KEY_LEN];
    envelope::fill_random(&mut k)?;
    Ok(k)
}

// ---------------------------------------------------------------------------
// The Secret Service
// ---------------------------------------------------------------------------

/// No Secret Service to ask: nothing owns the name and nothing can be
/// activated for it, or there is no session bus at all.
fn absent(e: &DbusError) -> bool {
    let what = format!("{e:?}");
    ["ServiceUnknown", "NameHasNoOwner", "Address(", "InputOutput("].iter().any(|s| what.contains(s))
}

/// Why `add` failed: our item lost a first-run race, or the service failed.
enum KeyringError {
    Raced,
    Failed(DbusError),
}

fn from_keyring(service: &Service, account: &str) -> Result<([u8; KEY_LEN], Origin), String> {
    let attributes = [("service", SERVICE), ("account", account)];
    let collection = block_on(async {
        let collection = service.default_collection().await?;
        if collection.is_locked().await? {
            collection.unlock(None).await?;
        }
        Ok::<_, DbusError>(collection)
    })
    .map_err(|e| describe("open the keyring for", &KeyringError::Failed(e)))?;

    let read = || block_on(oldest(&collection, &attributes)).map_err(KeyringError::Failed);
    let add = |key: &[u8; KEY_LEN]| {
        block_on(async {
            let label = format!("vue-native storage key ({account})");
            let ours = collection
                .create_item(&label, &attributes, key, false, None)
                .await
                .map_err(KeyringError::Failed)?;
            // Another process may have added one first; the oldest is the key.
            match oldest_item(&collection, &attributes).await.map_err(KeyringError::Failed)? {
                Some(winner) if winner.path() != ours.path() => {
                    let _ = ours.delete(None).await;
                    Err(KeyringError::Raced)
                }
                _ => Ok(()),
            }
        })
    };
    envelope::load_or_create_in(
        "Secret Service",
        read,
        add,
        fresh,
        |e| matches!(e, KeyringError::Raced),
        describe,
    )
}

async fn oldest_item(collection: &Collection, attributes: &[(&str, &str); 2]) -> Result<Option<oo7::dbus::Item>, DbusError> {
    let mut best: Option<(std::time::Duration, oo7::dbus::Item)> = None;
    for item in collection.search_items(attributes).await? {
        let created = item.created().await?;
        if best.as_ref().is_none_or(|(at, _)| created < *at) {
            best = Some((created, item));
        }
    }
    Ok(best.map(|(_, item)| item))
}

async fn oldest(collection: &Collection, attributes: &[(&str, &str); 2]) -> Result<Option<Vec<u8>>, DbusError> {
    let Some(item) = oldest_item(collection, attributes).await? else {
        return Ok(None);
    };
    if item.is_locked().await? {
        item.unlock(None).await?;
    }
    Ok(Some(item.secret().await?.as_bytes().to_vec()))
}

fn describe(op: &str, e: &KeyringError) -> String {
    match e {
        KeyringError::Raced => format!("Secret Service: could not {op} the storage key: another process added one"),
        KeyringError::Failed(DbusError::Dismissed) => {
            format!("Secret Service: could not {op} the storage key: the unlock prompt was dismissed")
        }
        KeyringError::Failed(e) => format!("Secret Service: could not {op} the storage key: {e}"),
    }
}

// ---------------------------------------------------------------------------
// The key file
// ---------------------------------------------------------------------------

/// `$XDG_DATA_HOME/vue-native/keys/<account>.key`, `~/.local/share` without
/// an absolute `XDG_DATA_HOME`.
fn key_file(account: &str) -> Result<PathBuf, String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("neither XDG_DATA_HOME nor HOME is set")?;
    let name: String = account.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    Ok(data.join("vue-native/keys").join(format!("{name}.key")))
}

fn from_file(path: &PathBuf) -> Result<([u8; KEY_LEN], Origin), String> {
    let read = || match fs::File::open(path) {
        Ok(mut f) => {
            let mut bytes = Vec::with_capacity(KEY_LEN);
            f.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    };
    // create_new: add-only, so a racing process gets AlreadyExists and re-reads.
    let add = |key: &[u8; KEY_LEN]| {
        if let Some(dir) = path.parent() {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(key)?;
        f.sync_all()
    };
    envelope::load_or_create_in(
        "Key file",
        read,
        add,
        fresh,
        |e: &std::io::Error| e.kind() == ErrorKind::AlreadyExists,
        |op, e| format!("Key file: could not {op} the storage key at {}: {e}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_file_is_created_once_private_and_read_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("vn-storage-keyfile-{}", std::process::id()));
        let path = dir.join("keys/exe_test.key");
        let (a, origin) = from_file(&path).unwrap();
        assert_eq!(origin, Origin::Created);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let (b, origin) = from_file(&path).unwrap();
        assert_eq!((a, origin), (b, Origin::Found));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_key_file_that_is_not_a_key_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("vn-storage-badkey-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("short.key");
        fs::write(&path, b"short").unwrap();
        assert!(from_file(&path).unwrap_err().contains("not a 256-bit key"));
        assert_eq!(fs::read(&path).unwrap(), b"short");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn account_names_become_file_names() {
        let path = key_file("exe:argon/../x").unwrap();
        assert_eq!(path.file_name().unwrap(), "exe_argon_.._x.key");
    }
}
