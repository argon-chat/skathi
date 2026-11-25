//! macOS: `NSWorkspace` and `NSFileManager` on the `sk-shell` thread. Both are thread-safe, so nothing
//! hops to the main queue; each call runs in its own autorelease pool. Opening asks Launch Services for
//! a handler first, so "nothing opens this" is an error instead of a "choose an application" prompt.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained};
use objc2::Message;
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSError, NSFileManager, NSString, NSURL};

use crate::Fail;

const OPEN_ANSWER: Duration = Duration::from_secs(120);

pub fn thread_init() {}

#[derive(Clone, Copy)]
enum Kind {
    NoHandler,
    NotFound,
    Denied,
    Busy,
    Cancelled,
    NoTrash,
}

fn kind_of(domain: &str, code: isize, trashing: bool) -> Option<Kind> {
    let readonly = if trashing {
        Kind::NoTrash
    } else {
        Kind::Denied
    };
    if domain == "NSCocoaErrorDomain" {
        match code {
            4 | 260 => Some(Kind::NotFound),
            257 | 513 => Some(Kind::Denied),
            642 => Some(readonly),
            3072 => Some(Kind::Cancelled),
            3328 if trashing => Some(Kind::NoTrash),
            _ => None,
        }
    } else if domain == "NSPOSIXErrorDomain" {
        match code as i32 {
            libc::ENOENT => Some(Kind::NotFound),
            libc::EACCES | libc::EPERM => Some(Kind::Denied),
            libc::EBUSY => Some(Kind::Busy),
            libc::EROFS => Some(readonly),
            libc::ENOTSUP if trashing => Some(Kind::NoTrash),
            _ => None,
        }
    } else if domain == "NSOSStatusErrorDomain" {
        match code {
            -10814 => Some(Kind::NoHandler),
            -128 => Some(Kind::Cancelled),
            -43 => Some(Kind::NotFound),
            -54 | -5000 => Some(Kind::Denied),
            _ => None,
        }
    } else {
        None
    }
}

fn fail_of(verb: &str, target: &str, kind: Kind) -> Fail {
    match kind {
        Kind::NoHandler => Fail::new(
            "NotFoundError",
            format!("{verb}: no application is set to open {target}"),
        )
        .code("ENOASSOC"),
        Kind::NotFound => Fail::new(
            "NotFoundError",
            format!(
                "{verb}: {target} — a requested file or directory could not be found at the time an operation was processed."
            ),
        )
        .code("ENOENT"),
        Kind::Denied => Fail::new("NotAllowedError", format!("{verb}: {target} — access is denied")).code("EACCES"),
        Kind::Busy => Fail::new(
            "NoModificationAllowedError",
            format!("{verb}: {target} is in use by another program"),
        )
        .code("EBUSY"),
        Kind::Cancelled => Fail::new("AbortError", format!("{verb}: {target} — the operation was cancelled")).code("ECANCELED"),
        Kind::NoTrash => Fail::new(
            "InvalidStateError",
            format!("{verb}: {target} cannot go to the Trash (its volume has none) — nothing was deleted"),
        )
        .code("ENOTRASH"),
    }
}

fn fail(verb: &str, target: &str, err: &NSError, trashing: bool) -> Fail {
    let mut chain = vec![err.retain()];
    chain.extend(err.underlyingErrors().iter());
    for e in &chain {
        if let Some(kind) = kind_of(&e.domain().to_string(), e.code(), trashing) {
            return fail_of(verb, target, kind).os(e.code() as i32);
        }
    }
    Fail::new(
        "InvalidStateError",
        format!("{verb}: {target} — {}", err.localizedDescription()),
    )
    .code("EIO")
    .os(err.code() as i32)
}

fn file_url(verb: &str, path: &Path) -> Result<Retained<NSURL>, Fail> {
    let Some(s) = path.to_str() else {
        return Err(Fail::type_error(format!(
            "{verb}: {} is not valid UTF-8",
            path.display()
        )));
    };
    Ok(NSURL::fileURLWithPath(&NSString::from_str(s)))
}

fn open(verb: &'static str, target: String, url: &NSURL, path: Option<&Path>) -> Result<(), Fail> {
    let workspace = NSWorkspace::sharedWorkspace();
    if workspace.URLForApplicationToOpenURL(url).is_none() {
        if let Some(Err(e)) = path.map(std::fs::symlink_metadata) {
            return Err(crate::shell::fail_from_io(&e, verb, &target));
        }
        return Err(fail_of(verb, &target, Kind::NoHandler));
    }
    let (tx, rx) = mpsc::channel::<Result<(), Fail>>();
    let named = target.clone();
    let done = RcBlock::new(move |_: *mut NSRunningApplication, err: *mut NSError| {
        // SAFETY: the handler's error is nil or a live NSError for the duration of the call.
        let answer = match unsafe { err.as_ref() } {
            None => Ok(()),
            Some(e) => Err(fail(verb, &named, e, false)),
        };
        let _ = tx.send(answer);
    });
    let config = NSWorkspaceOpenConfiguration::configuration();
    workspace.openURL_configuration_completionHandler(url, &config, Some(&done));
    drop(done);
    rx.recv_timeout(OPEN_ANSWER).unwrap_or_else(|_| {
        Err(Fail::new(
            "InvalidStateError",
            format!("{verb}: {target} — the system did not say whether it opened"),
        )
        .code("EIO"))
    })
}

pub fn open_external(url: &str) -> Result<(), Fail> {
    autoreleasepool(|_| {
        let Some(ns) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return Err(Fail::type_error(format!(
                "openExternal: {url:?} is not a valid URL"
            )));
        };
        open("openExternal", url.to_string(), &ns, None)
    })
}

pub fn open_path(path: &Path) -> Result<(), Fail> {
    autoreleasepool(|_| {
        let url = file_url("openPath", path)?;
        open("openPath", path.display().to_string(), &url, Some(path))
    })
}

pub fn show_item(path: &Path) -> Result<(), Fail> {
    autoreleasepool(|_| {
        let url = file_url("showItemInFolder", path)?;
        NSWorkspace::sharedWorkspace()
            .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        Ok(())
    })
}

pub fn trash(path: &Path) -> Result<(), Fail> {
    trash_to(path).map(|_| ())
}

fn trash_to(path: &Path) -> Result<Option<std::path::PathBuf>, Fail> {
    autoreleasepool(|_| {
        let url = file_url("trashItem", path)?;
        let mut moved: Option<Retained<NSURL>> = None;
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, Some(&mut moved))
            .map_err(|e| fail("trashItem", &path.display().to_string(), &e, true))?;
        Ok(moved
            .and_then(|u| u.path())
            .map(|p| std::path::PathBuf::from(p.to_string())))
    })
}

/// Would Launch Services run this rather than open it? Applications and Unix executables (an executable
/// file whose name gives no other type) conform to `public.executable`; the type comes from the target of a link.
#[allow(deprecated)]
pub fn runs_as_program(path: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(path) else {
        return false;
    };
    if crate::shell::is_executable(&real, None) {
        return true;
    }
    let Some(s) = real.to_str() else {
        return false;
    };
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let Ok(uti) = workspace.typeOfFile_error(&NSString::from_str(s)) else {
            return false;
        };
        workspace.type_conformsToType(&uti, &NSString::from_str("public.executable"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("sk-desktop-shell-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn file(&self, name: &str, mode: u32) -> std::path::PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            p
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn trash_moves_a_file_out_of_its_folder() {
        let dir = Scratch::new("trash");
        let file = dir.file("probe.txt", 0o644);
        let moved = trash_to(&file).expect("trash");
        assert!(!file.exists(), "the file left its folder");
        let moved = moved.expect("the Trash names where the file went");
        assert!(moved.exists(), "the file is in the Trash");
        std::fs::remove_file(&moved).unwrap();
    }

    #[test]
    fn trashing_a_missing_file_is_not_found() {
        let dir = Scratch::new("trash-missing");
        let f = trash(&dir.0.join("missing.txt")).unwrap_err();
        assert_eq!((f.name, f.code), ("NotFoundError", Some("ENOENT")));
    }

    #[test]
    fn a_scheme_nothing_handles_has_no_handler() {
        let f = open_external("sk-desktop-no-such-scheme-a8f3://x").unwrap_err();
        assert_eq!((f.name, f.code), ("NotFoundError", Some("ENOASSOC")));
    }

    #[test]
    fn opening_a_missing_path_is_not_found() {
        let dir = Scratch::new("open-missing");
        let f = open_path(&dir.0.join("missing.txt")).unwrap_err();
        assert_eq!((f.name, f.code), ("NotFoundError", Some("ENOENT")));
    }

    #[test]
    fn launch_services_types_decide_what_runs() {
        let dir = Scratch::new("runs");
        assert!(runs_as_program(&dir.file("tool", 0o755)));
        assert!(!runs_as_program(&dir.file("notes", 0o644)));
        assert!(!runs_as_program(&dir.file("notes.txt", 0o755)));
        let link = dir.0.join("innocent.txt");
        std::os::unix::fs::symlink(dir.file("Thing.app", 0o644), &link).unwrap();
        assert!(runs_as_program(&link));
    }
}
