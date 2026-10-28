//! Per-user protocol registration in the registry (docs/LAUNCH.md §5).
//!
//! ```text
//!   HKCU\Software\Classes\<scheme>
//!     (Default)                  = "URL:<scheme>"
//!     URL Protocol               = ""
//!     VnOwner                    = "<app id>"         ← the ownership marker
//!     DefaultIcon\(Default)      = "\"<exe>\",0"
//!     shell\open\command\(Default) = "\"<exe>\" [--vn-dist \"<slot>\"] --vn-protocol-launch -- \"%1\""
//! ```
//!
//! HKCU only — never HKLM, never HKCR (which is the merged view). A scheme
//! that already exists WITHOUT our marker, in HKCU or machine-wide in HKLM, is
//! another app's and is never overwritten: registering it is a TypeError that
//! names the owner. Writes are idempotent — only a value that differs is
//! written, which is also what heals a registration after the exe moved — and
//! each write that changed something is followed by
//! `SHChangeNotify(SHCNE_ASSOCCHANGED)`.

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

use super::protocol::{command_line, command_targets};

/// The value that marks a key as ours.
pub const OWNER_VALUE: &str = "VnOwner";

/// Where registrations live. [`Hive::user`] is the real one; tests point a
/// hive at a scratch subtree of HKCU so they never touch `Software\Classes`.
#[derive(Debug, Clone)]
pub struct Hive {
    /// Under HKCU: `Software\Classes`.
    pub classes: String,
    /// Under HKLM, checked read-only for machine-wide owners. `None` in tests.
    pub machine: Option<String>,
    /// Broadcast `SHCNE_ASSOCCHANGED` after a write (off in tests).
    pub notify: bool,
}

impl Hive {
    pub fn user() -> Hive {
        Hive { classes: r"Software\Classes".into(), machine: Some(r"Software\Classes".into()), notify: true }
    }
}

/// What a scheme's key says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Absent,
    /// Ours (our marker), with the command it currently runs.
    Ours { command: String },
    /// Somebody else's; `owner` describes them for the error message.
    Foreign { owner: String },
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(root: HKEY, path: &str, write: bool) -> Result<Option<Key>, WIN32_ERROR> {
    let mut h = HKEY::default();
    let r = unsafe { RegOpenKeyExW(root, &HSTRING::from(path), None, if write { KEY_READ | KEY_WRITE } else { KEY_READ }, &mut h) };
    if r == ERROR_SUCCESS {
        Ok(Some(Key(h)))
    } else if r == ERROR_FILE_NOT_FOUND {
        Ok(None)
    } else {
        Err(r)
    }
}

fn create(root: HKEY, path: &str) -> Result<Key, WIN32_ERROR> {
    let mut h = HKEY::default();
    let r = unsafe {
        RegCreateKeyExW(
            root,
            &HSTRING::from(path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut h,
            None,
        )
    };
    if r == ERROR_SUCCESS {
        Ok(Key(h))
    } else {
        Err(r)
    }
}

/// A `REG_SZ` value (`""` names the default value), or None.
fn read_sz(key: &Key, name: &str) -> Option<String> {
    let name = HSTRING::from(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut len = 0u32;
    if unsafe { RegQueryValueExW(key.0, &name, None, Some(&mut ty), None, Some(&mut len)) } != ERROR_SUCCESS {
        return None;
    }
    if ty != REG_SZ {
        return Some(String::new());
    }
    let mut buf = vec![0u8; len as usize + 2];
    let mut len2 = buf.len() as u32;
    if unsafe { RegQueryValueExW(key.0, &name, None, None, Some(buf.as_mut_ptr()), Some(&mut len2)) } != ERROR_SUCCESS {
        return None;
    }
    let wide: Vec<u16> = buf[..len2 as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    Some(String::from_utf16_lossy(&wide[..end]))
}

/// Writes a `REG_SZ` value if it differs. Returns whether it wrote.
fn write_sz(key: &Key, name: &str, value: &str) -> Result<bool, WIN32_ERROR> {
    if read_sz(key, name).as_deref() == Some(value) {
        return Ok(false);
    }
    let mut bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes.extend_from_slice(&[0, 0]);
    let r = unsafe { RegSetValueExW(key.0, &HSTRING::from(name), None, REG_SZ, Some(&bytes)) };
    if r == ERROR_SUCCESS {
        Ok(true)
    } else {
        Err(r)
    }
}

fn describe_owner(key: &Key, scheme: &str) -> String {
    if let Some(owner) = read_sz(key, OWNER_VALUE).filter(|o| !o.is_empty()) {
        return format!("the vue-native app \"{owner}\"");
    }
    match read_sz(key, "").filter(|d| !d.is_empty() && !d.eq_ignore_ascii_case(&format!("URL:{scheme}"))) {
        Some(desc) => format!("another application (\"{desc}\")"),
        None => "another application".into(),
    }
}

/// What `<scheme>` is right now, for app `id`.
pub fn status(hive: &Hive, scheme: &str, id: &str) -> Result<Status, String> {
    let path = format!(r"{}\{scheme}", hive.classes);
    match open(HKEY_CURRENT_USER, &path, false).map_err(|e| format!("reading HKCU\\{path}: {e:?}"))? {
        Some(key) => {
            if read_sz(&key, OWNER_VALUE).as_deref() == Some(id) {
                let command = open(HKEY_CURRENT_USER, &format!(r"{path}\shell\open\command"), false)
                    .ok()
                    .flatten()
                    .and_then(|k| read_sz(&k, ""))
                    .unwrap_or_default();
                Ok(Status::Ours { command })
            } else {
                Ok(Status::Foreign { owner: describe_owner(&key, scheme) })
            }
        }
        None => {
            if let Some(machine) = &hive.machine {
                let mpath = format!(r"{machine}\{scheme}");
                if let Ok(Some(_)) = open(HKEY_LOCAL_MACHINE, &mpath, false) {
                    // A per-user key would SHADOW a machine-wide one for this
                    // user — taking it over all the same.
                    return Ok(Status::Foreign { owner: "a machine-wide registration (HKLM)".into() });
                }
            }
            Ok(Status::Absent)
        }
    }
}

/// Registers `<scheme>` for app `id` → `exe`. `Ok(true)` when something was
/// written, `Ok(false)` when it was already exactly so. `Err` names why not
/// (a foreign owner, a registry failure) — the caller turns it into a
/// TypeError or an advisory.
pub fn register(hive: &Hive, scheme: &str, id: &str, exe: &str, dev_slot: Option<&str>) -> Result<bool, String> {
    if let Status::Foreign { owner } = status(hive, scheme, id)? {
        return Err(format!("the \"{scheme}\" scheme is registered to {owner}; vue-native never takes over another app's scheme"));
    }
    let path = format!(r"{}\{scheme}", hive.classes);
    let fail = |what: &str, e: WIN32_ERROR| format!("writing HKCU\\{path}{what}: {e:?}");
    let root = create(HKEY_CURRENT_USER, &path).map_err(|e| fail("", e))?;
    let mut wrote = false;
    // The marker first: a half-written key is still recognisably ours.
    wrote |= write_sz(&root, OWNER_VALUE, id).map_err(|e| fail("", e))?;
    wrote |= write_sz(&root, "", &format!("URL:{scheme}")).map_err(|e| fail("", e))?;
    wrote |= write_sz(&root, "URL Protocol", "").map_err(|e| fail("", e))?;
    let icon = create(HKEY_CURRENT_USER, &format!(r"{path}\DefaultIcon")).map_err(|e| fail(r"\DefaultIcon", e))?;
    wrote |= write_sz(&icon, "", &format!("\"{exe}\",0")).map_err(|e| fail(r"\DefaultIcon", e))?;
    let cmd = create(HKEY_CURRENT_USER, &format!(r"{path}\shell\open\command"))
        .map_err(|e| fail(r"\shell\open\command", e))?;
    wrote |= write_sz(&cmd, "", &command_line(exe, dev_slot)).map_err(|e| fail(r"\shell\open\command", e))?;
    if wrote && hive.notify {
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
    Ok(wrote)
}

/// Is `<scheme>` ours AND does it start `exe`?
pub fn is_registered(hive: &Hive, scheme: &str, id: &str, exe: &str) -> bool {
    matches!(status(hive, scheme, id), Ok(Status::Ours { command }) if command_targets(&command, exe))
}

fn delete(hive: &Hive, scheme: &str) -> bool {
    let Ok(Some(classes)) = open(HKEY_CURRENT_USER, &hive.classes, true) else { return false };
    let ok = unsafe { RegDeleteTreeW(classes.0, &HSTRING::from(scheme)) } == ERROR_SUCCESS;
    if ok && hive.notify {
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
    ok
}

/// Removes `<scheme>` — only when it carries our marker AND runs `exe`, so a
/// copy of the app elsewhere keeps its registration. Returns whether it did.
pub fn unregister(hive: &Hive, scheme: &str, id: &str, exe: &str) -> bool {
    is_registered(hive, scheme, id, exe) && delete(hive, scheme)
}

/// `--vn-unregister-protocols`: every key under the hive's classes carrying
/// our marker, whichever exe it points at (this is the uninstaller's hook;
/// the exe may be the one being removed). Returns the schemes removed.
pub fn unregister_all(hive: &Hive, id: &str) -> Vec<String> {
    let Ok(Some(classes)) = open(HKEY_CURRENT_USER, &hive.classes, false) else { return Vec::new() };
    let mut ours = Vec::new();
    let mut i = 0u32;
    loop {
        let mut name = vec![0u16; 256];
        let mut len = name.len() as u32;
        let r = unsafe { RegEnumKeyExW(classes.0, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if r == ERROR_NO_MORE_ITEMS {
            break;
        }
        i += 1;
        if r != ERROR_SUCCESS {
            continue;
        }
        let sub = String::from_utf16_lossy(&name[..len as usize]);
        // File extensions and CLSIDs are not schemes; skip the obvious ones
        // without opening them.
        if sub.starts_with('.') || sub.starts_with('{') {
            continue;
        }
        if let Ok(Some(k)) = open(HKEY_CURRENT_USER, &format!(r"{}\{sub}", hive.classes), false) {
            if read_sz(&k, OWNER_VALUE).as_deref() == Some(id) {
                ours.push(sub);
            }
        }
    }
    drop(classes);
    ours.into_iter().filter(|s| delete(hive, s)).collect()
}

/// Does this process run with MSIX package identity? Then HKCU\Classes writes
/// are virtualised and register nothing: protocols belong in the
/// AppxManifest (`uap:Protocol`) instead (A-F9).
pub fn has_package_identity() -> bool {
    let mut len = 0u32;
    // APPMODEL_ERROR_NO_PACKAGE (15700) = no identity; ERROR_INSUFFICIENT_BUFFER
    // (122) = there is one and we did not pass a buffer.
    let r = unsafe { GetCurrentPackageFullName(&mut len, None) };
    r.0 == 122
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch hive under `HKCU\Software\vue-native-test-<pid>-<tag>`, deleted
    /// on drop — the tests never touch `Software\Classes`.
    struct Scratch(Hive, String);
    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let base = format!(r"Software\vue-native-test-{}-{tag}", std::process::id());
            let hive = Hive { classes: format!(r"{base}\Classes"), machine: None, notify: false };
            create(HKEY_CURRENT_USER, &hive.classes).expect("scratch hive");
            Scratch(hive, base)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Ok(Some(sw)) = open(HKEY_CURRENT_USER, "Software", true) {
                let name = self.1.trim_start_matches(r"Software\");
                unsafe {
                    let _ = RegDeleteTreeW(sw.0, &HSTRING::from(name));
                }
            }
            assert!(
                matches!(open(HKEY_CURRENT_USER, &self.1, false), Ok(None)),
                "the scratch hive HKCU\\{} must be gone",
                self.1
            );
        }
    }

    const EXE: &str = r"C:\Program Files\Notes\Notes.exe";

    #[test]
    fn register_writes_the_documented_shape_and_is_idempotent() {
        let s = Scratch::new("shape");
        assert_eq!(status(&s.0, "vntest", "gl.argon.notes"), Ok(Status::Absent));
        assert_eq!(register(&s.0, "vntest", "gl.argon.notes", EXE, None), Ok(true));
        assert_eq!(register(&s.0, "vntest", "gl.argon.notes", EXE, None), Ok(false), "nothing changed, nothing written");
        let root = open(HKEY_CURRENT_USER, &format!(r"{}\vntest", s.0.classes), false).unwrap().unwrap();
        assert_eq!(read_sz(&root, "").as_deref(), Some("URL:vntest"));
        assert_eq!(read_sz(&root, "URL Protocol").as_deref(), Some(""));
        assert_eq!(read_sz(&root, OWNER_VALUE).as_deref(), Some("gl.argon.notes"));
        let icon = open(HKEY_CURRENT_USER, &format!(r"{}\vntest\DefaultIcon", s.0.classes), false).unwrap().unwrap();
        assert_eq!(read_sz(&icon, "").unwrap(), format!("\"{EXE}\",0"));
        assert_eq!(
            status(&s.0, "vntest", "gl.argon.notes"),
            Ok(Status::Ours { command: command_line(EXE, None) })
        );
        assert!(is_registered(&s.0, "vntest", "gl.argon.notes", EXE));
        // The exe moved: re-registering heals the command (and only it).
        let moved = r"D:\Notes\Notes.exe";
        assert!(!is_registered(&s.0, "vntest", "gl.argon.notes", moved));
        assert_eq!(register(&s.0, "vntest", "gl.argon.notes", moved, Some(r"C:\slot")), Ok(true));
        assert!(is_registered(&s.0, "vntest", "gl.argon.notes", moved));
    }

    #[test]
    fn a_foreign_scheme_is_never_overwritten() {
        let s = Scratch::new("foreign");
        // Another app's key: no marker.
        let k = create(HKEY_CURRENT_USER, &format!(r"{}\other", s.0.classes)).unwrap();
        write_sz(&k, "", "URL:Other App Protocol").unwrap();
        drop(k);
        let e = register(&s.0, "other", "gl.argon.notes", EXE, None).unwrap_err();
        assert!(e.contains("Other App Protocol"), "{e}");
        assert!(!unregister(&s.0, "other", "gl.argon.notes", EXE), "and never deleted");
        assert!(open(HKEY_CURRENT_USER, &format!(r"{}\other", s.0.classes), false).unwrap().is_some());
        // Another vue-native app's key: its marker, named.
        register(&s.0, "theirs", "gl.argon.other", EXE, None).unwrap();
        let e = register(&s.0, "theirs", "gl.argon.notes", EXE, None).unwrap_err();
        assert!(e.contains("gl.argon.other"), "{e}");
    }

    #[test]
    fn unregister_needs_our_marker_and_our_exe() {
        let s = Scratch::new("unreg");
        register(&s.0, "vntest", "gl.argon.notes", EXE, None).unwrap();
        assert!(!unregister(&s.0, "vntest", "gl.argon.notes", r"D:\elsewhere\Notes.exe"), "another copy's");
        assert!(!unregister(&s.0, "vntest", "gl.argon.other", EXE), "another app's");
        assert!(unregister(&s.0, "vntest", "gl.argon.notes", EXE));
        assert_eq!(status(&s.0, "vntest", "gl.argon.notes"), Ok(Status::Absent));
        assert!(!unregister(&s.0, "vntest", "gl.argon.notes", EXE), "already gone");
    }

    #[test]
    fn unregister_all_takes_exactly_our_keys() {
        let s = Scratch::new("all");
        register(&s.0, "one", "gl.argon.notes", EXE, None).unwrap();
        register(&s.0, "two", "gl.argon.notes", r"D:\moved\Notes.exe", None).unwrap();
        register(&s.0, "three", "gl.argon.other", EXE, None).unwrap();
        create(HKEY_CURRENT_USER, &format!(r"{}\.txt", s.0.classes)).unwrap();
        let mut gone = unregister_all(&s.0, "gl.argon.notes");
        gone.sort();
        assert_eq!(gone, vec!["one".to_string(), "two".to_string()]);
        assert!(matches!(status(&s.0, "three", "gl.argon.other"), Ok(Status::Ours { .. })));
    }

    #[test]
    fn this_test_process_has_no_package_identity() {
        assert!(!has_package_identity());
    }
}
