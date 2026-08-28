//! `app.getUserFolder(kind, subfolder)` — the five portable places a desktop
//! app is allowed to keep things.
//!
//! # Why a closed set of five
//!
//! A path is the least portable thing an application can hold. `%APPDATA%` has
//! no macOS spelling, `~/Library/Application Support` has no Windows one, and an
//! app that hard-codes either is an app that only ships to one OS. What IS
//! portable is the *intent*: roaming settings, machine-local data, scratch
//! space, user documents, home. Five names, one meaning each, resolved per
//! platform — the same reasoning as
//! [`redjs::webapi::permissions::Capability`]'s closed capability set, and for
//! the same reason: an open string would let a typo become a folder nobody
//! notices is wrong until a user's data is in it.
//!
//! # Windows: known folders, not environment variables
//!
//! `SHGetKnownFolderPath` rather than `std::env::var("APPDATA")`. The
//! environment variables are inherited from whoever launched the process, so
//! they are forgeable and occasionally simply absent (a service, a scheduled
//! task, an elevated shell that lost them); and there is no environment
//! variable at all for Documents, which is the one folder a user has genuinely
//! moved to a different drive. `TEMP` is the exception and is an environment
//! lookup even in the Win32 API — `GetTempPath2W` reads TMP/TEMP/USERPROFILE in
//! that order — so we call that, not `SHGetKnownFolderPath`.
//!
//! # macOS: the mapping table (design; not compiled here)
//!
//! | kind            | macOS                                   | note |
//! |-----------------|-----------------------------------------|------|
//! | `APPDATA`       | `~/Library/Application Support`         | |
//! | `LOCALAPPDATA`  | `~/Library/Application Support`         | macOS has NO roaming/local split; both names resolve to the same folder and that is the honest answer. `~/Library/Caches` would be wrong: the OS may delete it. |
//! | `TEMP`          | `NSTemporaryDirectory()` (`$TMPDIR`)    | per-user, per-boot; the OS reclaims it. |
//! | `DOCUMENTS`     | `~/Documents`                           | `NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, …)` |
//! | `HOME`          | `NSHomeDirectory()`                     | sandboxed if the app is sandboxed, which is the correct answer either way. |
//!
//! The two names collapsing onto one folder on macOS is a real difference, and
//! an app that needs "definitely not synced to another machine" has to say so
//! some other way. Documented rather than papered over.

use std::path::PathBuf;

/// The five folder kinds, in the order a picker should list them.
pub const KINDS: &[&str] = &["APPDATA", "LOCALAPPDATA", "TEMP", "DOCUMENTS", "HOME"];

/// Resolves `kind` (case-insensitive; `APPDATA` is the canonical spelling) and
/// joins `subfolder`, which may be empty.
///
/// Creates nothing. A path is not a directory, and an app that only wants to
/// show a user where its data *would* go should not leave an empty folder
/// behind. [`crate::set_storage_folder`] is what creates directories.
pub fn user_folder(kind: &str, subfolder: &str) -> Result<PathBuf, String> {
    let base = base_of(&kind.to_ascii_uppercase())?;
    if subfolder.is_empty() {
        return Ok(base);
    }
    Ok(base.join(check_subfolder(subfolder)?))
}

/// A subfolder must be RELATIVE and must not climb.
///
/// The failure this refuses: `getUserFolder("APPDATA", "../../Windows")`
/// resolving to somewhere the app had no business naming. Rejecting is not
/// paranoia about a hostile app — the app is trusted — it is about a typo or a
/// user-supplied string landing somewhere destructive, which is exactly how
/// this class of bug happens in practice.
fn check_subfolder(sub: &str) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for seg in sub.split(['/', '\\']) {
        match seg {
            "" | "." => continue,
            ".." => {
                return Err(format!(
                    "subfolder {sub:?} must not contain '..' — it names a place under the known \
                     folder, not a path to anywhere"
                ))
            }
            s if s.contains(':') => {
                return Err(format!(
                    "subfolder {sub:?} must be relative — {s:?} looks like a drive or a stream name"
                ))
            }
            s => out.push(s),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("subfolder {sub:?} names nothing"));
    }
    Ok(out)
}

fn unknown(kind: &str) -> String {
    format!(
        "{kind:?} is not a user folder kind — expected one of {}",
        KINDS.join(", ")
    )
}

#[cfg(target_os = "windows")]
fn base_of(kind: &str) -> Result<PathBuf, String> {
    use windows::Win32::Storage::FileSystem::GetTempPath2W;
    use windows::Win32::UI::Shell::{
        FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData,
        KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };

    let id = match kind {
        "APPDATA" => FOLDERID_RoamingAppData,
        "LOCALAPPDATA" => FOLDERID_LocalAppData,
        "DOCUMENTS" => FOLDERID_Documents,
        "HOME" => FOLDERID_Profile,
        "TEMP" => {
            // GetTempPath2W wants a buffer and reports the length it needed;
            // MAX_PATH+1 is what the API documents as sufficient for the
            // non-2 variant, and the 2 variant is bounded the same way.
            let mut buf = [0u16; 261];
            let n = unsafe { GetTempPath2W(Some(&mut buf)) } as usize;
            if n == 0 || n > buf.len() {
                return Err("GetTempPath2W did not return a path".into());
            }
            return Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
        }
        other => return Err(unknown(other)),
    };
    // SAFETY: `id` is one of the constants above; the handle argument is
    // `None` (the calling user's own folder); the returned PWSTR is ours to
    // free and is freed on both paths below.
    let wide = unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) }
        .map_err(|e| format!("SHGetKnownFolderPath failed for {kind}: {e}"))?;
    let path = unsafe { wide.to_string() };
    unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(wide.0 as *const core::ffi::c_void))
    };
    path.map(PathBuf::from)
        .map_err(|e| format!("SHGetKnownFolderPath returned an unreadable path for {kind}: {e}"))
}

/// The portable fallback: environment variables, which is all a platform
/// without a known-folder API has. Also what a macOS build gets until the
/// Foundation calls in this module's header are written.
#[cfg(not(target_os = "windows"))]
fn base_of(kind: &str) -> Result<PathBuf, String> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| String::from("$HOME is not set"))
    };
    match kind {
        // No roaming/local split outside Windows — see the header's table.
        "APPDATA" | "LOCALAPPDATA" => {
            if cfg!(target_os = "macos") {
                Ok(home()?.join("Library").join("Application Support"))
            } else {
                match std::env::var_os("XDG_DATA_HOME") {
                    Some(d) if !d.is_empty() => Ok(PathBuf::from(d)),
                    _ => Ok(home()?.join(".local").join("share")),
                }
            }
        }
        "TEMP" => Ok(std::env::temp_dir()),
        "DOCUMENTS" => Ok(home()?.join("Documents")),
        "HOME" => home(),
        other => Err(unknown(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_resolves_to_an_absolute_path_that_is_not_empty() {
        // The contract `setStorageFolder` depends on: a kind either names a
        // real place or errors — it never hands back "" or a relative path.
        for kind in KINDS {
            let p = user_folder(kind, "").unwrap_or_else(|e| panic!("{kind}: {e}"));
            assert!(p.is_absolute(), "{kind} -> {p:?} is not absolute");
            assert!(!p.as_os_str().is_empty(), "{kind} -> empty");
        }
    }

    #[test]
    fn the_kind_is_case_insensitive_but_a_typo_still_fails() {
        assert_eq!(user_folder("appdata", "").unwrap(), user_folder("APPDATA", "").unwrap());
        for bad in ["APP_DATA", "appdata2", "", "ROAMING", "LOCAL"] {
            let e = user_folder(bad, "").unwrap_err();
            assert!(e.contains("not a user folder kind"), "{bad:?}: {e}");
            assert!(e.contains("LOCALAPPDATA"), "the error must list the kinds: {e}");
        }
    }

    #[test]
    fn the_two_appdata_kinds_are_distinct_on_windows_and_the_same_elsewhere() {
        // The one place the portable abstraction genuinely leaks, pinned so it
        // stays a documented fact rather than a surprise.
        let roaming = user_folder("APPDATA", "").unwrap();
        let local = user_folder("LOCALAPPDATA", "").unwrap();
        if cfg!(target_os = "windows") {
            assert_ne!(roaming, local, "Windows has a roaming/local split");
        } else {
            assert_eq!(roaming, local, "no other platform does");
        }
    }

    #[test]
    fn a_subfolder_is_joined_and_separators_are_normalised() {
        let base = user_folder("LOCALAPPDATA", "").unwrap();
        assert_eq!(user_folder("LOCALAPPDATA", "myApp").unwrap(), base.join("myApp"));
        // Both separators, because an app written on either OS will use either.
        assert_eq!(
            user_folder("LOCALAPPDATA", "a/b").unwrap(),
            user_folder("LOCALAPPDATA", "a\\b").unwrap()
        );
        assert_eq!(user_folder("LOCALAPPDATA", "./a//b/").unwrap(), base.join("a").join("b"));
    }

    #[test]
    fn a_subfolder_cannot_climb_out_of_its_known_folder() {
        for bad in ["..", "../x", "a/../..", "..\\..\\Windows"] {
            let e = user_folder("APPDATA", bad).unwrap_err();
            assert!(e.contains(".."), "{bad:?}: {e}");
        }
        let e = user_folder("APPDATA", "C:\\Windows").unwrap_err();
        assert!(e.contains("relative"), "{e}");
        // A subfolder that normalises away is an error, not the base folder —
        // otherwise `getUserFolder("APPDATA", userInput)` with empty-ish input
        // silently returns the whole of AppData.
        assert!(user_folder("APPDATA", "./").is_err());
    }
}
