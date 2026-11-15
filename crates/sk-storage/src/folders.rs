//! `app.getUserFolder(kind, subfolder)`: five portable places an app may keep things (`APPDATA`,
//! `LOCALAPPDATA`, `TEMP`, `DOCUMENTS`, `HOME`), a closed set so a typo cannot become a wrong folder.
//!
//! Windows uses known folders rather than environment variables (inherited, forgeable, absent for
//! Documents). Elsewhere `APPDATA` and `LOCALAPPDATA` resolve to the same folder, so an app needing
//! "never synced" must say so another way.

use std::path::PathBuf;

pub const KINDS: &[&str] = &["APPDATA", "LOCALAPPDATA", "TEMP", "DOCUMENTS", "HOME"];

/// Resolves `kind` (case-insensitive) and joins `subfolder`, which may be empty. Creates nothing.
pub fn user_folder(kind: &str, subfolder: &str) -> Result<PathBuf, String> {
    let base = base_of(&kind.to_ascii_uppercase())?;
    if subfolder.is_empty() {
        return Ok(base);
    }
    Ok(base.join(check_subfolder(subfolder)?))
}

/// A subfolder must be relative and must not climb.
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
    use windows_sys::Win32::Storage::FileSystem::GetTempPath2W;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData,
        SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };

    let id = match kind {
        "APPDATA" => FOLDERID_RoamingAppData,
        "LOCALAPPDATA" => FOLDERID_LocalAppData,
        "DOCUMENTS" => FOLDERID_Documents,
        "HOME" => FOLDERID_Profile,
        "TEMP" => {
            let mut buf = [0u16; 261];
            let n = unsafe { GetTempPath2W(buf.len() as u32, buf.as_mut_ptr()) } as usize;
            if n == 0 || n > buf.len() {
                return Err("GetTempPath2W did not return a path".into());
            }
            return Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
        }
        other => return Err(unknown(other)),
    };
    // SAFETY: `id` is one of the constants above; the handle argument is
    // null (the calling user's own folder); the returned PWSTR is ours to
    // free, also on failure, and is freed below.
    let mut wide = std::ptr::null_mut();
    let hr = unsafe {
        SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut wide)
    };
    let path = (hr >= 0).then(|| {
        // SAFETY: on success `wide` is a NUL-terminated string.
        let len = (0..).take_while(|&i| unsafe { *wide.add(i) } != 0).count();
        String::from_utf16(unsafe { std::slice::from_raw_parts(wide, len) })
    });
    unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(wide.cast()) };
    match path {
        None => Err(format!(
            "SHGetKnownFolderPath failed for {kind}: 0x{:08x}",
            hr as u32
        )),
        Some(path) => path.map(PathBuf::from).map_err(|e| {
            format!("SHGetKnownFolderPath returned an unreadable path for {kind}: {e}")
        }),
    }
}

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
        assert_eq!(
            user_folder("appdata", "").unwrap(),
            user_folder("APPDATA", "").unwrap()
        );
        for bad in ["APP_DATA", "appdata2", "", "ROAMING", "LOCAL"] {
            assert!(user_folder(bad, "").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_subfolder_is_joined_and_separators_are_normalised() {
        let base = user_folder("LOCALAPPDATA", "").unwrap();
        assert_eq!(
            user_folder("LOCALAPPDATA", "myApp").unwrap(),
            base.join("myApp")
        );
        assert_eq!(
            user_folder("LOCALAPPDATA", "a/b").unwrap(),
            user_folder("LOCALAPPDATA", "a\\b").unwrap()
        );
        assert_eq!(
            user_folder("LOCALAPPDATA", "./a//b/").unwrap(),
            base.join("a").join("b")
        );
    }

    #[test]
    fn a_subfolder_cannot_climb_out_of_its_known_folder() {
        for bad in ["..", "../x", "a/../..", "..\\..\\Windows", "C:\\Windows"] {
            assert!(user_folder("APPDATA", bad).is_err(), "{bad:?}");
        }
        // A subfolder that normalises away is an error, not the whole base folder.
        assert!(user_folder("APPDATA", "./").is_err());
    }
}
