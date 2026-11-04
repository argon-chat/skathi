//! TEMPORARY (m26): the `ArgonSecure` cookie the Argon client's Electron host
//! sets on every launch, reproduced here without its native plugin.
//!
//! # Why it exists, and why it is temporary
//!
//! The Argon API identifies an installed client by one cookie, `ArgonSecure`,
//! on `.argon.gl`. Electron's security plugin (`libsec`, `SecPlugin.cs`) sets it
//! through `session.cookies.set` before the first window opens. Without it
//! every call fails with "MachineId is not defined", and the sign-in screen
//! cannot even ask whether an email exists. vue-native has no plugin host yet
//! (research/reports/m26/client-gap/C-host-cookie.md §4, "Layer 3"). Until it
//! does, this file mints the same cookie in Rust, opt-in, once per launch,
//! before any JS runs.
//!
//! **Removing it is one step:** delete this file, its `mod` line and its one
//! call in `lib.rs`. The `argonSecure` key in the build tooling then does
//! nothing and can go in the same commit (build.ts, cli/config.ts,
//! cli/build-app.ts, payload.rs, dev.rs, prod.rs, each marked TEMPORARY).
//!
//! # The value (C-host-cookie.md §2.3)
//!
//! `hwid=verified&rum=0&ert=<hex>&scid=<guid N>&colt=<machine id>&ner=<app id>`
//!
//! - `scid`: a fresh random GUID every launch, in .NET's "N" format. The
//!   server's sign-out tombstones key on it, so it must change.
//! - `colt`: 16 random bytes, base64url, the shape the server and the web
//!   client mint. Created once and kept sealed (DPAPI) in
//!   `<storage folder>\argon-secure.colt.sealed`. It must stay stable, because
//!   every access token is bound to it (`mh`), and a new one would sign the
//!   user out. It is not Electron's hardware-derived id: to the server, this
//!   is a second device, which is honest.
//! - `ner`: the desktop application id. A public identifier, cited below.
//! - `ert`: random hex. Nothing reads it; it is there so the value has the
//!   same parts.
//! - `hwv` and `dev` are left out: there is no hardware vector (its pepper is
//!   deliberately not copied) and no TPM proof. The server treats that as
//!   an unknown device, which is what a Mac or Linux Electron build gets.
//!
//! # The attributes (`SecPlugin.cs`, C-host-cookie.md §2.2)
//!
//! `url https://argon.gl`, `Domain=.argon.gl`, `Path=/`, `Secure`, `HttpOnly`,
//! `SameSite=None`, 30 days. Set through redjs's `cookies::set`, under the
//! same rules a server's `Set-Cookie` faces.
//!
//! # Opting in
//!
//! `"argonSecure": true` in the app's `vn.config.json` (dev: the manifest;
//! prod: the payload META), or `VN_ARGON_SECURE=1` for a dev run.
//! `VN_ARGON_SECURE=0` turns it off even when the config says on. Nothing is
//! keyed on the app's name or id.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use vn_runtime::redjs::webapi::cookies::{self, HostCookie, SameSite};
use vn_storage::{crypt, file};

/// The Argon desktop application id: `SecurityExports.AppId`, from
/// `desktop/src/Plugins/Argon.Security/SecurityExports.cs:29` in the Argon
/// monorepo. A public identifier, not a secret: the server's own
/// `docs/internal/architecture/device-identity.md` lists it too.
const NER: &str = "875180ED6396874C0536D95B30BB7B47";

const COOKIE: &str = "ArgonSecure";
const URL: &str = "https://argon.gl";
const DOMAIN: &str = ".argon.gl";
const LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const COLT_FILE: &str = "argon-secure.colt.sealed";

/// Is the producer on for this run? `config` is the build's `argonSecure`.
pub fn wanted(config: bool) -> bool {
    decide(config, std::env::var("VN_ARGON_SECURE").ok().as_deref())
}

fn decide(config: bool, env: Option<&str>) -> bool {
    match env.map(str::trim) {
        Some("1") | Some("true") => true,
        Some("0") | Some("false") => false,
        _ => config,
    }
}

/// Sets the cookie. Called once, after `vn_storage::install` (the jar's file
/// is loaded, and the folder is latched) and before any JS.
pub fn run() {
    let colt = match vn_storage::cookies::folder() {
        Some(folder) => colt_in(&folder),
        None => {
            eprintln!("[argon-secure] no storage folder — using a colt for this launch only");
            fresh_colt()
        }
    };
    let Some(colt) = colt else {
        eprintln!("[argon-secure] no random generator — ArgonSecure NOT set");
        return;
    };
    let (Some(scid), Some(ert)) = (scid(), hex(16)) else {
        eprintln!("[argon-secure] no random generator — ArgonSecure NOT set");
        return;
    };
    let value = format!("hwid=verified&rum=0&ert={ert}&scid={scid}&colt={colt}&ner={NER}");
    let r = cookies::set(HostCookie {
        url: URL.into(),
        name: COOKIE.into(),
        value,
        domain: Some(DOMAIN.into()),
        path: Some("/".into()),
        secure: true,
        http_only: true,
        same_site: SameSite::None,
        expires: Some(SystemTime::now() + LIFETIME),
    });
    match r {
        Ok(()) => eprintln!(
            "[argon-secure] TEMPORARY producer: {COOKIE} set for {DOMAIN} (scid={scid}; no hwv, no dev) — \
             crates/vn-app/src/temp_argon_secure.rs"
        ),
        Err(e) => eprintln!("[argon-secure] {COOKIE} NOT set: {e}"),
    }
}

/// The persisted machine id, created on first use.
fn colt_in(folder: &Path) -> Option<String> {
    let path: PathBuf = folder.join(COLT_FILE);
    let Some(p) = crypt::provider() else {
        eprintln!("[argon-secure] no encryption provider — the colt is NOT persisted (a new device each launch)");
        return fresh_colt();
    };
    if let Ok(raw) = std::fs::read(&path) {
        match file::unseal(p, &raw) {
            Ok(c) if valid_colt(&c) => return Some(c),
            Ok(_) => eprintln!("[argon-secure] {} holds no valid colt — minting a new one", path.display()),
            Err(e) => eprintln!("[argon-secure] could not open {}: {e} — minting a new one", path.display()),
        }
        // A new colt is a new device to the server: every session bound to
        // the old one ends. Said above, once, at the moment it happens.
    }
    let colt = fresh_colt()?;
    match file::seal(p, &colt).and_then(|b| file::write_atomic(&path, &b).map_err(|e| e.to_string())) {
        Ok(()) => eprintln!("[argon-secure] minted a colt, kept sealed in {}", path.display()),
        Err(e) => eprintln!("[argon-secure] could not keep the colt in {}: {e} — it lasts this launch only", path.display()),
    }
    Some(colt)
}

fn fresh_colt() -> Option<String> {
    let mut b = [0u8; 16];
    crypt::random_bytes(&mut b).ok()?;
    Some(base64url(&b))
}

/// 16 bytes, base64url without padding: 22 characters.
fn valid_colt(s: &str) -> bool {
    s.len() == 22 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// A version-4 GUID in .NET's "N" format: 32 lower-case hex digits.
fn scid() -> Option<String> {
    let mut b = [0u8; 16];
    crypt::random_bytes(&mut b).ok()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn hex(n: usize) -> Option<String> {
    let mut b = vec![0u8; n];
    crypt::random_bytes(&mut b).ok()?;
    Some(b.iter().map(|x| format!("{x:02X}")).collect())
}

fn base64url(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &x)| acc | (x as u32) << (16 - 8 * i));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_env_var_overrides_the_config_both_ways() {
        assert!(!decide(false, None));
        assert!(decide(true, None));
        assert!(decide(false, Some("1")));
        assert!(!decide(true, Some("0")));
        assert!(decide(true, Some("")), "empty = unset");
    }

    #[test]
    fn the_parts_have_the_servers_shapes() {
        assert_eq!(base64url(&[0xfb, 0xff, 0xbf]), "-_-_");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        if crypt::random_bytes(&mut [0u8; 1]).is_ok() {
            let c = fresh_colt().unwrap();
            assert!(valid_colt(&c), "{c}");
            let s = scid().unwrap();
            assert_eq!(s.len(), 32);
            assert_eq!(&s[12..13], "4");
            assert!(s.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert_ne!(scid(), scid());
        }
        assert!(!valid_colt("short"));
        assert!(!valid_colt("aaaaaaaaaaaaaaaaaaaaa="));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn the_colt_is_minted_once_and_then_read_back() {
        let dir = std::env::temp_dir().join(format!("vn-argon-colt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = colt_in(&dir).unwrap();
        assert_eq!(colt_in(&dir).unwrap(), a);
        let raw = std::fs::read(dir.join(COLT_FILE)).unwrap();
        assert!(!raw.windows(a.len()).any(|w| w == a.as_bytes()), "sealed");
        std::fs::write(dir.join(COLT_FILE), b"garbage").unwrap();
        assert_ne!(colt_in(&dir).unwrap(), a, "an unreadable file mints a new one");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
