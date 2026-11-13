//! At-rest encryption for the storage file: a provider trait and one provider per platform.
//! The key must live somewhere the attacker does not have, so each provider leans on the OS.
//!
//! - Windows (id 1): DPAPI. The OS derives the key from the user's logon credential; this
//!   process never sees it. It protects against other users and a copied file or backup, not
//!   against anything running as the same user. A blob can become unreadable (local account,
//!   reset password, profile that did not roam), which is why an unreadable blob is an empty
//!   area plus an advisory, never a fatal error, and why the default folder is `LOCALAPPDATA`.
//! - macOS (id 2): one random 256-bit master key per app in the login keychain (service
//!   `gl.argon.vue-native.storage`, account = bundle identifier or `exe:<name>`), sealing
//!   blobs with AES-256-GCM in `envelope` (versioned, random nonce, the DPAPI entropy string
//!   as associated data). The key is created add-only so racing processes agree on one
//!   (`envelope::load_or_create`). Only `keychain` needs a Mac; the rest compiles under
//!   `cfg(test)` everywhere.
//! - Linux (id 3): the same envelope and key, held by the Secret Service. With no Secret
//!   Service on the bus (bare window manager, WSLg) the key goes in a user-only file under
//!   `$XDG_DATA_HOME/vue-native/keys/`; once made it is used even if a keyring appears later,
//!   since a second key would orphan the data. See `secret_service`.
//!
//! A keychain or keyring that cannot be read is an error latched for the process: never
//! plaintext, and never a new key that would orphan data sealed under the old one.
//! [`CryptProtectData`]: https://learn.microsoft.com/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata

#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) mod envelope;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "linux")]
mod secret_service;

/// One way of turning bytes into bytes only this user can read back on disk.
pub trait SecretProvider: Send + Sync {
    /// How an advisory names this: `"Windows DPAPI (per-user)"`.
    fn name(&self) -> &'static str;

    /// The byte identifying this provider in a file header, so a file written by one provider is
    /// refused by another. `1` is DPAPI, `2` the macOS Keychain, `3` the Linux one.
    fn id(&self) -> u8;

    /// Whether this provider can seal and open right now. Lets a caller tell an unreadable blob
    /// from an unavailable provider (locked or refused keychain: refuse, touch nothing).
    fn ready(&self) -> Result<(), String> {
        Ok(())
    }

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, String>;
    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String>;
}

/// Entropy mixed into every blob. Not a secret (it is in the binary); it stops an unrelated
/// program running as the same user from decrypting our blob by passing it to
/// `CryptUnprotectData`.
#[cfg(target_os = "windows")]
const ENTROPY: &[u8] = b"vue-native/storage/v1";

/// The provider for this build, or `None` on a platform without one; asking for
/// `{ encrypted: true }` there is refused rather than written as plaintext.
pub fn provider() -> Option<&'static dyn SecretProvider> {
    #[cfg(target_os = "windows")]
    {
        static P: Dpapi = Dpapi;
        Some(&P)
    }
    #[cfg(target_os = "macos")]
    {
        static P: keychain::Keychain = keychain::Keychain::new();
        Some(&P)
    }
    #[cfg(target_os = "linux")]
    {
        static P: secret_service::SecretService = secret_service::SecretService::new();
        Some(&P)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Fills `buf` from the OS CSPRNG; an error, never a weaker fallback.
pub fn random_bytes(buf: &mut [u8]) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
        let st = unsafe { BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if st.0 < 0 {
            return Err(format!("BCryptGenRandom failed: 0x{:08x}", st.0 as u32));
        }
        Ok(())
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        envelope::fill_random(buf)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = buf;
        Err(String::from("no random generator on this platform"))
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

/// One body for both directions so the `LocalFree` cannot be forgotten in one of them.
#[cfg(target_os = "windows")]
unsafe fn dpapi(input: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    let mut inb = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut entb = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB::default();

    // CRYPTPROTECT_UI_FORBIDDEN: this runs on the JS/render thread, which a UI prompt would deadlock.
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
    #[cfg(target_os = "linux")]
    fn linux_has_the_secret_service_provider() {
        assert_eq!(provider().unwrap().id(), 3);
    }

    #[test]
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    fn a_platform_without_a_provider_says_so_rather_than_faking_one() {
        assert!(provider().is_none());
    }
}
