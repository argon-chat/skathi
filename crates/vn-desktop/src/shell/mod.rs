//! The shell: open a URL with its handler, open a file with its default app,
//! reveal a file in Explorer, move a file to the Recycle Bin (docs/SHELL.md).
//!
//! The bodies of gpui's `open_target` / `open_target_in_explorer`, re-hosted so
//! a failure reaches JS — gpui's versions are fire-and-forget behind `log_err`,
//! and an app that cannot tell "opened" from "nothing is associated with .xyz"
//! cannot tell its user either.
//!
//! Everything here splits in two, on purpose:
//!
//! * **validation** — the scheme allowlist, absolute paths, the executable
//!   guard — is pure, runs on the frame thread inside the host fn, and refuses
//!   SYNCHRONOUSLY (`{error, name}`), so a refused call never costs a thread hop
//!   and the rules are unit-tested without a shell;
//! * **the OS call** runs on ONE `vn-shell` thread (a COM STA, which is what the
//!   shell's own objects want) and settles through the results queue. That
//!   thread also does the `stat` in front of each call: the frame thread never
//!   touches a user path (design §3.2).
//!
//! # `VN_SHELL=record` — the test seam
//!
//! With `VN_SHELL=record` in the environment nothing reaches the OS: every call
//! is validated and stat'ed exactly as it would be, then settles successfully
//! and writes one `[shell] <op> <target> (recorded)` line into the log ring
//! (kind `shell`). It exists so an automated run can drive `<a href>`,
//! `window.open` and `appShell` end to end without opening a browser, an
//! Explorer window or a document on the machine it runs on.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use crate::Fail;

#[cfg(target_os = "windows")]
mod win;
#[cfg(target_os = "windows")]
use win as imp;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod stub;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
use stub as imp;

/// One shell request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    OpenExternal(String),
    OpenPath(PathBuf),
    ShowItem(PathBuf),
    Trash(PathBuf),
}

impl Op {
    fn verb(&self) -> &'static str {
        match self {
            Op::OpenExternal(_) => "openExternal",
            Op::OpenPath(_) => "openPath",
            Op::ShowItem(_) => "showItemInFolder",
            Op::Trash(_) => "trashItem",
        }
    }
    fn target(&self) -> String {
        match self {
            Op::OpenExternal(u) => u.clone(),
            Op::OpenPath(p) | Op::ShowItem(p) | Op::Trash(p) => p.display().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// validation (pure)
// ---------------------------------------------------------------------------

/// The schemes CONTENT may hand to the OS (design §5.1). Tier-1 surfaces are
/// reachable by whatever the app renders — a chat message's link, a Markdown
/// file — so `file:`, custom protocols and `ms-settings:` never pass here; that
/// closes the classic `shell.openExternal(untrusted)` → `file:///…/calc.exe`
/// vector by default. `appShell.openExternal` is the escape hatch for a URL the
/// app's own code trusts.
pub const CONTENT_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// `https://x` → `Some("https")`, lowercased. RFC 3986 `ALPHA *( ALPHA /
/// DIGIT / "+" / "-" / "." ) ":"`. A drive letter (`C:\x`) parses as a
/// one-letter scheme, which is exactly why `file`-ish inputs are refused by
/// name and paths have their own call.
pub fn scheme_of(url: &str) -> Option<String> {
    let colon = url.find(':')?;
    let s = &url[..colon];
    let mut chars = s.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some(s.to_ascii_lowercase())
}

/// The URL `openExternal` may hand to the shell, or why not.
pub fn validate_external(url: &str, content: bool) -> Result<String, Fail> {
    if url.is_empty() {
        return Err(Fail::type_error("openExternal: the URL is empty"));
    }
    if url.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
        return Err(Fail::type_error("openExternal: the URL contains a control character"));
    }
    let Some(scheme) = scheme_of(url) else {
        return Err(Fail::type_error(format!("openExternal: {url:?} is not an absolute URL")));
    };
    if scheme.len() == 1 {
        // `C:\x`, `D:/y` — a Windows drive path, not a URL.
        return Err(Fail::type_error(format!(
            "openExternal: {url:?} is a file path — use appShell.openPath()"
        )));
    }
    if content {
        if !CONTENT_SCHEMES.contains(&scheme.as_str()) {
            return Err(Fail::new(
                "NotAllowedError",
                format!(
                    "a {scheme}: link was refused: content links only open http, https and mailto"
                ),
            )
            .code("ESCHEME"));
        }
    } else if scheme == "file" {
        return Err(Fail::type_error(format!(
            "openExternal: {url:?} is a file: URL — use appShell.openPath() for files"
        )));
    }
    Ok(url.to_string())
}

/// An absolute path, or why not. `\\?\` long paths and UNC are fine; a
/// relative path is a TypeError rather than something resolved against a cwd
/// the app does not control.
pub fn validate_path(path: &str, what: &str) -> Result<PathBuf, Fail> {
    if path.is_empty() {
        return Err(Fail::type_error(format!("{what}: the path is empty")));
    }
    if path.contains('\0') {
        return Err(Fail::type_error(format!("{what}: the path contains a NUL")));
    }
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(Fail::type_error(format!(
            "{what}: {path:?} is not an absolute path (appStorage.getUserFolder() names the usual places)"
        )));
    }
    // `C:\x\report.txt:payload.exe` names an NTFS alternate data stream. There
    // is no legitimate reason to hand one to the shell, and the executable
    // guard below reads extensions off the path — refuse the shape outright.
    if alternate_stream(&p) {
        return Err(Fail::type_error(format!(
            "{what}: {path:?} names an alternate data stream (a `:` after the drive)"
        )));
    }
    Ok(p)
}

/// Does the path carry a `:` anywhere but a drive designator or the `\\?\`
/// prefix?
fn alternate_stream(p: &Path) -> bool {
    let s = p.to_string_lossy();
    let rest = s
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| s.strip_prefix(r"\\?\"))
        .or_else(|| s.strip_prefix(r"\\.\"))
        .unwrap_or(&s);
    // Skip a drive designator (`C:`).
    let rest = if rest.len() >= 2 && rest.as_bytes()[1] == b':' && rest.as_bytes()[0].is_ascii_alphabetic() {
        &rest[2..]
    } else {
        rest
    };
    rest.contains(':')
}

/// Extensions `openPath` refuses without `{ allowExecutable: true }`.
///
/// The design's list (every `PATHEXT` entry plus `.lnk .url .scf .pif .msi .msp
/// .hta .ps1 .vbs .vbe .js .jse .wsf .wsh .reg .cpl`) and then some: everything
/// Windows will RUN, install, or hand to a script host when it is "opened" —
/// ClickOnce (`.application`, `.appref-ms`), the `.settingcontent-ms` that was
/// CVE-2018-8414, MSIX/AppX installers, compiled help, the screensaver `.scr`
/// (a PE image) and the PowerShell module/format files. A Save-dialog-then-
/// "open it" flow must not launch an attacker-named `report.pdf.exe`.
const EXECUTABLE_EXTS: &[&str] = &[
    "exe", "com", "bat", "cmd", "scr", "pif", "cpl", "msc", "msi", "msp", "mst", "lnk", "url",
    "scf", "hta", "ps1", "psm1", "psd1", "ps1xml", "ps2", "ps2xml", "psc1", "psc2", "vb", "vbs",
    "vbe", "js", "jse", "ws", "wsf", "wsc", "wsh", "reg", "inf", "ins", "isp", "jar", "chm", "hlp",
    "gadget", "application", "appref-ms", "settingcontent-ms", "msix", "msixbundle", "appx",
    "appxbundle", "appinstaller", "diagcab", "library-ms", "search-ms", "searchconnector-ms",
    "xbap", "msh", "msh1", "msh2", "mshxml", "msh1xml", "msh2xml", "sct", "shb", "xll", "website",
];

/// Would the shell RUN this rather than open it?
///
/// The extension is read the way Windows reads it: trailing dots and spaces are
/// stripped first (`evil.exe.` and `evil.exe ` both run `evil.exe`), and the
/// comparison is case-insensitive. `PATHEXT` from the environment is honoured
/// on top of the built-in list — it only ever ADDS refusals.
pub fn is_executable(path: &Path, pathext: Option<&str>) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = name.trim_end_matches(['.', ' ']);
    let Some(dot) = name.rfind('.') else { return false };
    let ext = name[dot + 1..].to_ascii_lowercase();
    if ext.is_empty() {
        return false;
    }
    if EXECUTABLE_EXTS.contains(&ext.as_str()) {
        return true;
    }
    pathext.is_some_and(|list| {
        list.split(';')
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .any(|e| !e.is_empty() && e == ext)
    })
}

/// The io::Error → (name, code) table of design §3.3, as far as the shell
/// needs it.
pub fn fail_from_io(e: &std::io::Error, what: &str, target: &str) -> Fail {
    use std::io::ErrorKind as K;
    let os = e.raw_os_error();
    let f = match e.kind() {
        K::NotFound => Fail::new(
            "NotFoundError",
            format!(
                "{what}: {target} — a requested file or directory could not be found at the time an operation was processed."
            ),
        )
        .code("ENOENT"),
        K::PermissionDenied => Fail::new("NotAllowedError", format!("{what}: {target} — access is denied")).code("EACCES"),
        _ => Fail::new("InvalidStateError", format!("{what}: {target} — {e}")).code("EIO"),
    };
    match os {
        Some(code) => f.os(code),
        None => f,
    }
}

// ---------------------------------------------------------------------------
// starting a call (frame thread)
// ---------------------------------------------------------------------------

fn token_json(token: u32) -> String {
    format!("{{\"token\":{token}}}")
}

pub fn start_open_external(url: &str, content: bool) -> String {
    match validate_external(url, content) {
        Ok(url) => token_json(submit(Op::OpenExternal(url))),
        Err(f) => f.to_sync_json(),
    }
}

pub fn start_open_path(path: &str, allow_executable: bool) -> String {
    let p = match validate_path(path, "openPath") {
        Ok(p) => p,
        Err(f) => return f.to_sync_json(),
    };
    if !allow_executable && is_executable(&p, std::env::var("PATHEXT").ok().as_deref()) {
        return Fail::new(
            "NotAllowedError",
            format!(
                "openPath refused {path:?}: it is a program or a script, and opening it would run it — \
                 pass {{ allowExecutable: true }} if that is intended"
            ),
        )
        .code("EEXECUTABLE")
        .to_sync_json();
    }
    token_json(submit(Op::OpenPath(p)))
}

pub fn start_show_item(path: &str) -> String {
    match validate_path(path, "showItemInFolder") {
        Ok(p) => token_json(submit(Op::ShowItem(p))),
        Err(f) => f.to_sync_json(),
    }
}

pub fn start_trash(path: &str) -> String {
    match validate_path(path, "trashItem") {
        Ok(p) => token_json(submit(Op::Trash(p))),
        Err(f) => f.to_sync_json(),
    }
}

// ---------------------------------------------------------------------------
// the shell thread
// ---------------------------------------------------------------------------

struct Job {
    token: u32,
    op: Op,
}

/// `VN_SHELL=record`: validate and stat, never reach the OS (module docs).
pub fn recording() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| std::env::var("VN_SHELL").is_ok_and(|v| v.eq_ignore_ascii_case("record")))
}

fn submit(op: Op) -> u32 {
    let token = crate::next_token();
    static TX: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("vn-shell".into())
            .spawn(move || {
                imp::thread_init();
                for job in rx {
                    let result = run(&job.op);
                    crate::settle(job.token, result.map(|()| "{}".to_string()));
                }
            })
            .expect("spawning the vn-shell thread");
        Mutex::new(tx)
    });
    let sent = tx.lock().map(|tx| tx.send(Job { token, op: op.clone() }).is_ok()).unwrap_or(false);
    if !sent {
        crate::settle(token, Err(Fail::new("InvalidStateError", "the shell thread is gone").code("EIO")));
    }
    token
}

/// One request, on the shell thread: the stat, then the OS call (or the record).
fn run(op: &Op) -> Result<(), Fail> {
    match op {
        Op::OpenExternal(_) => {}
        Op::OpenPath(p) | Op::ShowItem(p) | Op::Trash(p) => {
            if let Err(e) = std::fs::symlink_metadata(p) {
                return Err(fail_from_io(&e, op.verb(), &p.display().to_string()));
            }
        }
    }
    if recording() {
        crate::log_later("shell", format!("[shell] {} {} (recorded — VN_SHELL=record)", op.verb(), op.target()));
        return Ok(());
    }
    match op {
        Op::OpenExternal(url) => imp::open_external(url),
        Op::OpenPath(p) => imp::open_path(p),
        Op::ShowItem(p) => imp::show_item(p),
        Op::Trash(p) => imp::trash(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_parse_per_rfc_3986() {
        assert_eq!(scheme_of("https://x").as_deref(), Some("https"));
        assert_eq!(scheme_of("MAILTO:a@b").as_deref(), Some("mailto"));
        assert_eq!(scheme_of("web+app:x").as_deref(), Some("web+app"));
        assert_eq!(scheme_of("ms-settings:privacy").as_deref(), Some("ms-settings"));
        assert_eq!(scheme_of("1x:y"), None);
        assert_eq!(scheme_of("/relative"), None);
        assert_eq!(scheme_of("no colon"), None);
        assert_eq!(scheme_of(":x"), None);
        assert_eq!(scheme_of("a b:c"), None);
    }

    #[test]
    fn content_links_open_only_http_https_and_mailto() {
        assert!(validate_external("https://example.com/a?b", true).is_ok());
        assert!(validate_external("http://example.com", true).is_ok());
        assert!(validate_external("mailto:a@example.com", true).is_ok());
        for refused in ["file:///C:/Windows/System32/calc.exe", "myapp://x", "ms-settings:privacy", "javascript:alert(1)", "search-ms:query=x"] {
            let e = validate_external(refused, true).unwrap_err();
            assert_eq!(e.name, "NotAllowedError", "{refused}");
            assert_eq!(e.code, Some("ESCHEME"));
        }
    }

    #[test]
    fn tier_two_takes_any_scheme_but_file() {
        assert!(validate_external("myapp://x", false).is_ok());
        assert!(validate_external("ms-settings:privacy", false).is_ok());
        assert_eq!(validate_external("file:///C:/x.txt", false).unwrap_err().name, "TypeError");
        assert_eq!(validate_external("FILE:///C:/x.txt", false).unwrap_err().name, "TypeError");
        assert_eq!(validate_external(r"C:\x.txt", false).unwrap_err().name, "TypeError");
        assert_eq!(validate_external("relative/x", false).unwrap_err().name, "TypeError");
        assert_eq!(validate_external("", false).unwrap_err().name, "TypeError");
        assert_eq!(validate_external("https://x/\na", false).unwrap_err().name, "TypeError");
    }

    #[cfg(windows)]
    #[test]
    fn paths_must_be_absolute_and_name_no_stream() {
        assert!(validate_path(r"C:\Users\x\a.txt", "t").is_ok());
        assert!(validate_path(r"\\server\share\a.txt", "t").is_ok());
        assert!(validate_path(r"\\?\C:\very\long\a.txt", "t").is_ok());
        assert_eq!(validate_path("a.txt", "t").unwrap_err().name, "TypeError");
        assert_eq!(validate_path(r"..\a.txt", "t").unwrap_err().name, "TypeError");
        assert_eq!(validate_path("", "t").unwrap_err().name, "TypeError");
        assert_eq!(validate_path(r"C:\x\report.txt:evil.exe", "t").unwrap_err().name, "TypeError");
        assert_eq!(validate_path(r"\\?\C:\x\a.txt:s", "t").unwrap_err().name, "TypeError");
    }

    #[cfg(unix)]
    #[test]
    fn paths_must_be_absolute() {
        assert!(validate_path("/home/x/a.txt", "t").is_ok());
        assert_eq!(validate_path("a.txt", "t").unwrap_err().name, "TypeError");
        assert_eq!(validate_path("../a.txt", "t").unwrap_err().name, "TypeError");
        assert_eq!(validate_path("", "t").unwrap_err().name, "TypeError");
    }

    #[test]
    fn the_executable_guard_reads_extensions_the_way_windows_does() {
        let exe = |p: &str| is_executable(Path::new(p), None);
        for yes in [
            r"C:\a\setup.exe", r"C:\a\SETUP.EXE", r"C:\a\report.pdf.exe", r"C:\a\x.bat", r"C:\a\x.cmd",
            r"C:\a\x.lnk", r"C:\a\x.url", r"C:\a\x.ps1", r"C:\a\x.vbs", r"C:\a\x.js", r"C:\a\x.hta",
            r"C:\a\x.msi", r"C:\a\x.reg", r"C:\a\x.cpl", r"C:\a\x.scr", r"C:\a\x.settingcontent-ms",
            r"C:\a\x.appref-ms", r"C:\a\x.msix",
            // Windows strips trailing dots and spaces before it runs anything.
            r"C:\a\evil.exe.", r"C:\a\evil.exe ", r"C:\a\evil.exe. . ",
        ] {
            assert!(exe(yes), "{yes} must be refused");
        }
        for no in [r"C:\a\report.pdf", r"C:\a\notes.txt", r"C:\a\photo.PNG", r"C:\a\folder", r"C:\a\.gitignore", r"C:\a\archive.tar.gz", r"C:\a\exe"] {
            assert!(!exe(no), "{no} must be allowed");
        }
        // PATHEXT only ever adds.
        assert!(is_executable(Path::new(r"C:\a\x.foo"), Some(".COM;.EXE;.FOO")));
        assert!(!is_executable(Path::new(r"C:\a\x.txt"), Some(".COM;.EXE;.FOO")));
    }

    #[test]
    fn io_errors_map_to_dom_names() {
        let nf = std::io::Error::from_raw_os_error(2);
        let f = fail_from_io(&nf, "openPath", "C:\\x");
        assert_eq!((f.name, f.code, f.os_error), ("NotFoundError", Some("ENOENT"), Some(2)));
        // ERROR_ACCESS_DENIED, EACCES.
        let ad = std::io::Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 });
        assert_eq!(fail_from_io(&ad, "t", "x").name, "NotAllowedError");
    }

    #[test]
    fn a_refused_call_answers_synchronously_without_a_token() {
        let v: serde_json::Value = serde_json::from_str(&start_open_external("file:///x", false)).unwrap();
        assert_eq!(v["name"], "TypeError");
        assert!(v.get("token").is_none());
        let setup = if cfg!(windows) { r"C:\x\setup.exe" } else { "/x/setup.exe" };
        let v: serde_json::Value = serde_json::from_str(&start_open_path(setup, false)).unwrap();
        assert_eq!(v["name"], "NotAllowedError");
        assert_eq!(v["code"], "EEXECUTABLE");
    }
}
