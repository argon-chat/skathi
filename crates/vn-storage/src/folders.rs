//! `app.getUserFolder(kind, subfolder)`: five portable places an app may keep things
//! (`APPDATA`, `LOCALAPPDATA`, `TEMP`, `DOCUMENTS`, `HOME`), resolved per platform. A closed set,
//! so a typo cannot become a folder nobody notices is wrong.
//!
//! Windows uses known folders (`SHGetKnownFolderPath`) rather than environment variables, which
//! are inherited, forgeable, sometimes absent, and do not exist for Documents. `TEMP` goes
//! through `GetTempPath2W`. Elsewhere there is no roaming/local split: `APPDATA` and
//! `LOCALAPPDATA` resolve to the same folder (macOS `~/Library/Application Support`, otherwise
//! `$XDG_DATA_HOME` or `~/.local/share`), so an app needing "never synced" must say so another way.

use std::path::PathBuf;

/// The five folder kinds, in the order a picker should list them.
pub const KINDS: &[&str] = &["APPDATA", "LOCALAPPDATA", "TEMP", "DOCUMENTS", "HOME"];

/// Resolves `kind` (case-insensitive; `APPDATA` is the canonical spelling) and
/// joins `subfolder`, which may be empty.
///
/// Creates nothing; [`crate::set_storage_folder`] is what creates directories.
pub fn user_folder(kind: &str, subfolder: &str) -> Result<PathBuf, String> {
    let base = base_of(&kind.to_ascii_uppercase())?;
    if subfolder.is_empty() {
        return Ok(base);
    }
    Ok(base.join(check_subfolder(subfolder)?))
}

/// A subfolder must be relative and must not climb: a typo or user-supplied string must not
/// land outside the known folder.
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
            // GetTempPath2W reports the length it needed; MAX_PATH+1 suffices.
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

/// Environment-variable fallback for platforms without a known-folder API.
#[cfg(not(target_os = "windows"))]
fn base_of(kind: &str) -> Result<PathBuf, String> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| String::from("$HOME is not set"))
    };
    match kind {
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
        // A subfolder that normalises away is an error, not the whole base folder.
        assert!(user_folder("APPDATA", "./").is_err());
    }
}
