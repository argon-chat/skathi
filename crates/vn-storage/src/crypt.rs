//! At-rest encryption for the storage file: a provider trait, and the one
//! provider Windows makes it possible to write honestly.
//!
//! # Why a trait, and why DPAPI is the Windows implementation
//!
//! Encrypting a file is easy. *Keeping the key* is the entire problem, and it
//! is the part every "we encrypt your local data" feature gets wrong: a key
//! stored next to the ciphertext protects against nothing, and a key derived
//! from something the app knows is a key the attacker also knows, because the
//! attacker has the app.
//!
//! DPAPI ([`CryptProtectData`]) is the answer Windows already has. The key is
//! derived by the OS from the user's logon credential and lives in LSA; our
//! process never sees it, never stores it, and cannot leak it. There is no key
//! file to manage, no first-run key generation, no key rotation, and no
//! "where do we put the master key" conversation. That is the whole reason it
//! is the default and the only Windows provider.
//!
//! # What DPAPI does and does not protect against — stated plainly
//!
//! It protects the file **from other Windows users on this machine, and from
//! the file being copied off it**. Someone who steals the disk, or the folder,
//! or the backup, gets bytes they cannot decrypt.
//!
//! It does **not** protect against anything running as the same user. A process
//! running as you can call `CryptUnprotectData` on our file exactly as we do.
//! That is not a flaw in the design, it is what per-user encryption *means*, and
//! an application that needs more than that needs a passphrase the user types —
//! which is a product decision, not a storage backend.
//!
//! # The roaming caveat
//!
//! DPAPI blobs are tied to the Windows **user**, and by default to the machine's
//! copy of that user's master key. A roaming profile carries the master key with
//! it, so `APPDATA` (roaming) plus DPAPI usually survives a move to another
//! domain-joined machine — but "usually" is doing real work in that sentence: a
//! local account, a reset password without the old one, or a profile that did
//! not roam all leave a blob nobody can open again.
//!
//! The consequence is designed for, not hoped away: an unreadable blob is
//! treated as **an empty area plus a loud advisory**, never as a fatal error.
//! Losing preferences is survivable; refusing to start is not. This is why the
//! default storage folder is `LOCALAPPDATA` — encrypted data that is explicitly
//! machine-local cannot be surprised by roaming.
//!
//! # macOS (design; not compiled here)
//!
//! The equivalent is a Keychain-derived key rather than a system-encrypt call:
//! generate 32 random bytes on first use, store them as a generic password item
//! (`SecItemAdd`, service `gl.argon.vue-native.storage`, account = the app
//! name, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` so it neither
//! syncs to iCloud Keychain nor is readable before first unlock), and encrypt
//! the file with AES-256-GCM under it. `SecItemCopyMatching` on later runs. The
//! ACL binds the item to the signed application, so the same
//! same-user-can-read-it caveat applies, with the same honesty.
//!
//! [`CryptProtectData`]: https://learn.microsoft.com/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata

/// One way of turning bytes nobody else can read into bytes on a disk.
///
/// Whole-blob, not per-value: see [`crate::file`] for why the file is
/// encrypted as one unit.
pub trait SecretProvider: Send + Sync {
    /// How an advisory names this: `"Windows DPAPI (per-user)"`.
    fn name(&self) -> &'static str;

    /// The byte that identifies this provider in a file header, so a file
    /// written by one provider is refused by another instead of decrypting to
    /// garbage. `1` is DPAPI.
    fn id(&self) -> u8;

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, String>;
    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String>;
}

/// Additional entropy mixed into every blob.
///
/// NOT a secret and not pretending to be one — it is compiled into the binary
/// and anyone can read it. What it buys is that a blob of ours cannot be
/// decrypted by simply passing it to `CryptUnprotectData` from an unrelated
/// program that happens to run as the same user: that program has to know it
/// was ours. A speed bump with a real name, not a security boundary.
#[cfg(target_os = "windows")]
const ENTROPY: &[u8] = b"vue-native/storage/v1";

/// The provider for this build, or `None` on a platform with no honest answer.
///
/// `None` is not a stub that stores plaintext under an encrypted name — a
/// storage folder asked for `{ encrypted: true }` on such a platform is
/// **refused**, loudly. Silently writing plaintext when an app asked for
/// encryption is the worst failure available here.
pub fn provider() -> Option<&'static dyn SecretProvider> {
    #[cfg(target_os = "windows")]
    {
        static P: Dpapi = Dpapi;
        Some(&P)
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
pub struct Dpapi;

#[cfg(target_os = "windows")]
impl SecretProvider for Dpapi {
    fn name(&self) -> &'static str {
        "Windows DPAPI (per-user)"
    }

    fn id(&self) -> u8 {
        1
    }

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        unsafe { dpapi(plaintext, true) }
    }

    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        unsafe { dpapi(ciphertext, false) }
    }
}

/// One body for both directions: the two APIs are the same call with the same
/// blob shapes and the same cleanup, and writing it twice is how the `LocalFree`
/// gets forgotten in one of them.
#[cfg(target_os = "windows")]
unsafe fn dpapi(input: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    // `cbData: 0` with a dangling pointer is legal for DPAPI, but an empty
    // input is never something we ask it to do — the caller has real bytes.
    let mut inb = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut entb = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB::default();

    // CRYPTPROTECT_UI_FORBIDDEN: this runs on the JS/render thread inside a
    // frame. A DPAPI call that decided to raise a UI prompt would deadlock the
    // very thread that would have to pump it.
    let r = if protect {
        CryptProtectData(
            &mut inb,
            None,
            Some(&mut entb),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    } else {
        CryptUnprotectData(
            &mut inb,
            None,
            Some(&mut entb),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };

    let verb = if protect { "CryptProtectData" } else { "CryptUnprotectData" };
    r.map_err(|e| format!("{verb} failed: {e}"))?;

    let bytes = if out.pbData.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec()
    };
    // DPAPI allocates the output with LocalAlloc; freeing it is the caller's.
    if !out.pbData.is_null() {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn a_round_trip_returns_exactly_what_went_in() {
        let p = provider().expect("Windows always has DPAPI");
        for plain in [
            &b""[..],
            &b"a"[..],
            b"{\"entries\":[[\"token\",\"hunter2\"]]}",
            &[0u8, 255, 128, 7][..],
        ] {
            let blob = p.protect(plain).expect("protect");
            let back = p.unprotect(&blob).expect("unprotect");
            assert_eq!(back, plain, "round trip");
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn the_ciphertext_does_not_contain_the_plaintext() {
        // The property the whole feature exists for, checked the way the demo
        // checks it: the secret must not be findable in the bytes on disk.
        let p = provider().unwrap();
        let secret = b"correct-horse-battery-staple";
        let blob = p.protect(secret).unwrap();
        assert!(
            !blob.windows(secret.len()).any(|w| w == secret),
            "the plaintext survived into the ciphertext"
        );
        assert!(blob.len() > secret.len(), "a DPAPI blob carries its own header");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_tampered_or_foreign_blob_is_an_error_not_garbage() {
        // What makes "treat an unreadable blob as an empty area" safe: an
        // unreadable blob is DETECTED, never silently decoded to nonsense.
        let p = provider().unwrap();
        let mut blob = p.protect(b"payload").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(p.unprotect(&blob).is_err(), "a flipped bit must not decrypt");
        assert!(p.unprotect(b"not a dpapi blob at all").is_err());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn the_provider_identifies_itself_for_the_file_header_and_for_advisories() {
        let p = provider().unwrap();
        assert_eq!(p.id(), 1);
        assert!(p.name().contains("DPAPI"), "{}", p.name());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_platform_without_a_provider_says_so_rather_than_faking_one() {
        assert!(provider().is_none());
    }
}
