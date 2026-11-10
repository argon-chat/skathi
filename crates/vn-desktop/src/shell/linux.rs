//! Linux: the freedesktop tools every desktop ships, run from the `vn-shell`
//! thread and waited on so a failure reaches JS.
//!
//! - `openExternal`, `openPath`: `xdg-open`, which defers to the desktop's own
//!   opener (`gio open`, `kde-open`, …). It exits once the handler is started;
//!   a non-zero status is "nothing handles this".
//! - `showItemInFolder`: `org.freedesktop.FileManager1.ShowItems` on the
//!   session bus (Nautilus, Dolphin, Nemo, Thunar answer it), else the folder
//!   opened with `xdg-open`.
//! - `trashItem`: `gio trash` (the freedesktop trash spec, done by GLib).

use std::path::Path;
use std::process::{Command, Stdio};

use crate::Fail;

pub fn thread_init() {}

fn failed(verb: &str, why: String) -> Fail {
    Fail::new("OperationError", format!("{verb} failed: {why}"))
}

/// Runs `program args…` to completion, quietly; its status is the answer.
fn run(verb: &str, program: &str, args: &[&std::ffi::OsStr]) -> Result<(), Fail> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Fail::new("NotSupportedError", format!("{verb}: {program} is not installed")).code("ENOSYS")
            } else {
                failed(verb, format!("{program}: {e}"))
            }
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(failed(verb, format!("{program} exited with {status}")))
    }
}

pub fn open_external(url: &str) -> Result<(), Fail> {
    run("openExternal", "xdg-open", &[url.as_ref()])
}

pub fn open_path(path: &Path) -> Result<(), Fail> {
    run("openPath", "xdg-open", &[path.as_os_str()])
}

pub fn show_item(path: &Path) -> Result<(), Fail> {
    let uri = file_uri(path);
    let shown = run(
        "showItemInFolder",
        "dbus-send",
        &[
            "--session".as_ref(),
            "--print-reply".as_ref(),
            "--dest=org.freedesktop.FileManager1".as_ref(),
            "/org/freedesktop/FileManager1".as_ref(),
            "org.freedesktop.FileManager1.ShowItems".as_ref(),
            format!("array:string:{uri}").as_ref(),
            "string:".as_ref(),
        ],
    );
    match (shown, path.parent()) {
        (Ok(()), _) => Ok(()),
        (Err(_), Some(folder)) => run("showItemInFolder", "xdg-open", &[folder.as_os_str()]),
        (Err(e), None) => Err(e),
    }
}

pub fn trash(path: &Path) -> Result<(), Fail> {
    run("trashItem", "gio", &["trash".as_ref(), path.as_os_str()])
}

/// `file://` and the path, percent-encoded outside RFC 3986's unreserved set
/// and `/`.
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(super::file_uri("/home/a b/ф,x.txt".as_ref()), "file:///home/a%20b/%D1%84%2Cx.txt");
    }
}
