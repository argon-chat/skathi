//! File System Access, the host side.
//!
//! Handles are ids: a JS `FileSystemHandle` carries a number and the path lives in [`HANDLES`]; no host
//! function takes a path from script or returns one. A handle exists only for something the user granted
//! (a picker result, a launch path, a dropped path) or for a child of a directory handle reached through
//! the name fence ([`check_name`]). Permission belongs to the path: a GRANT ([`grant`]) covers everything
//! below it for this session; a handle on an uncovered path is `"prompt"` and every operation through it
//! is `NotAllowedError` until `requestPermission()` grants it ([`permission`]).
//!
//! Disk work runs on the pool; the `start_*` functions run on the frame thread, apply the name fence and
//! either refuse synchronously or queue the operation and answer `{token}`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;

use crate::{pool, Done, Fail};

pub mod writable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Directory => "directory",
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    kind: Kind,
}

fn handles() -> &'static Mutex<HashMap<u64, Entry>> {
    static H: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn register(path: PathBuf, kind: Kind) -> u64 {
    let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    handles()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id, Entry { path, kind });
    id
}

fn lookup(id: u64) -> Option<Entry> {
    handles()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&id)
        .cloned()
}

pub fn release(id: u64) {
    handles()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id);
}

pub fn name_of(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

pub(crate) fn handle_json(path: PathBuf, kind: Kind) -> serde_json::Value {
    let name = name_of(&path);
    json!({ "h": register(path, kind), "name": name, "kind": kind.as_str() })
}

pub(crate) fn granted_handle_json(path: PathBuf, kind: Kind) -> serde_json::Value {
    grant(&path);
    handle_json(path, kind)
}

fn grants() -> &'static Mutex<Vec<String>> {
    static G: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    G.get_or_init(|| Mutex::new(Vec::new()))
}

/// The key a grant is compared by: lexical, no I/O (checked on the frame thread before every operation).
fn grant_key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if cfg!(windows) {
        let s = s.replace('/', "\\").to_lowercase();
        let t = s.trim_end_matches('\\');
        if t.ends_with(':') || t.is_empty() {
            s
        } else {
            t.to_string()
        }
    } else {
        let t = s.trim_end_matches('/');
        if t.is_empty() {
            s.to_string()
        } else {
            t.to_string()
        }
    }
}

pub(crate) fn grant(path: &Path) {
    let k = grant_key(path);
    let mut g = grants().lock().unwrap_or_else(|p| p.into_inner());
    if !g.contains(&k) {
        g.push(k);
    }
}

pub fn is_granted(path: &Path) -> bool {
    let k = grant_key(path);
    let sep = if cfg!(windows) { '\\' } else { '/' };
    grants()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .any(|g| {
            k == *g
                || (k.starts_with(g.as_str())
                    && (g.ends_with(sep) || k[g.len()..].starts_with(sep)))
        })
}

/// `"granted"` or `"prompt"`; `request` grants the handle's path (only on a user activation). `None` for an unknown id.
pub fn permission(id: u64, request: bool) -> Option<&'static str> {
    let e = lookup(id)?;
    if request {
        grant(&e.path);
    }
    Some(if is_granted(&e.path) {
        "granted"
    } else {
        "prompt"
    })
}

fn not_granted() -> Fail {
    Fail::new("NotAllowedError", NOT_ALLOWED)
}

fn granted(id: u64) -> Result<Entry, Fail> {
    let e = lookup(id).ok_or_else(gone)?;
    if !is_granted(&e.path) {
        return Err(not_granted());
    }
    Ok(e)
}

pub(crate) fn to_wire(id: u64) -> Option<Vec<u8>> {
    let e = lookup(id)?;
    Some(
        json!({ "v": 1, "kind": e.kind.as_str(), "path": e.path.to_string_lossy() })
            .to_string()
            .into_bytes(),
    )
}

pub(crate) fn from_wire(bytes: &[u8]) -> Option<(PathBuf, Kind)> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if v["v"].as_u64() != Some(1) {
        return None;
    }
    let kind = match v["kind"].as_str()? {
        "file" => Kind::File,
        "directory" => Kind::Directory,
        _ => return None,
    };
    let path = PathBuf::from(v["path"].as_str()?);
    path.is_absolute().then_some((path, kind))
}

fn gone() -> Fail {
    // A released or unknown handle id: the nearest honest name is the one for stale cached state.
    Fail::new("InvalidStateError", INVALID_STATE)
}

pub const NOT_FOUND: &str =
    "A requested file or directory could not be found at the time an operation was processed.";
pub const TYPE_MISMATCH: &str = "The path supplied exists, but was not an entry of requested type.";
pub const INVALID_MODIFICATION: &str = "The object can not be modified in this way.";
pub const NOT_ALLOWED: &str =
    "The request is not allowed by the user agent or the platform in the current context.";
pub const NO_MODIFICATION: &str =
    "An attempt was made to write to a file or directory which could not be modified due to the state of the underlying filesystem.";
pub const QUOTA: &str =
    "The operation failed because it would cause the application to exceed its storage quota.";
pub const INVALID_STATE: &str =
    "An operation that depends on state cached in an interface object was made but the state had changed since it was read from disk.";
pub const NOT_READABLE: &str =
    "The requested file could not be read, typically due to permission problems that have occurred after a reference to a file was acquired.";
pub const BAD_NAME: &str = "Name is not allowed.";

/// Whether the failed operation was reading or writing: a refusal is `NotAllowedError` for one and
/// `NoModificationAllowedError` for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// An `io::Error` → the DOMException a promise rejects with. The Win32 code decides first, since
/// std folds several of them into `Other`.
pub fn fail_io(e: &std::io::Error, access: Access) -> Fail {
    use std::io::ErrorKind as K;
    let os = e.raw_os_error();
    let named = |name: &'static str, msg: &str| Fail::new(name, msg);
    let f = match (os, e.kind()) {
        (Some(2 | 3), _) | (_, K::NotFound) => named("NotFoundError", NOT_FOUND),
        // ERROR_DIR_NOT_EMPTY: removeEntry without `recursive`.
        (Some(145), _) | (_, K::DirectoryNotEmpty) => {
            named("InvalidModificationError", INVALID_MODIFICATION)
        }
        // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL.
        (Some(112 | 39), _) | (_, K::StorageFull) => named("QuotaExceededError", QUOTA),
        // ERROR_DIRECTORY ("the directory name is invalid": a file where a
        // folder was expected) and its std spellings.
        (Some(267), _) | (_, K::NotADirectory | K::IsADirectory) => {
            named("TypeMismatchError", TYPE_MISMATCH)
        }
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION,
        // ERROR_WRITE_PROTECT, and ReplaceFileW's three "could not swap" codes.
        (Some(5 | 32 | 33 | 19 | 1175 | 1176 | 1177), _)
        | (_, K::PermissionDenied | K::ReadOnlyFilesystem) => match access {
            Access::Read => named("NotAllowedError", NOT_ALLOWED),
            Access::Write => named("NoModificationAllowedError", NO_MODIFICATION),
        },
        _ => Fail::new("InvalidStateError", format!("{INVALID_STATE} ({e})")),
    };
    match os {
        Some(code) => f.os(code),
        None => f,
    }
}

const DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "conin$", "conout$", "com0", "com1", "com2", "com3", "com4",
    "com5", "com6", "com7", "com8", "com9", "com¹", "com²", "com³", "lpt0", "lpt1", "lpt2", "lpt3",
    "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9", "lpt¹", "lpt²", "lpt³",
];

/// The traversal fence of `getFileHandle` / `getDirectoryHandle` / `removeEntry`: not empty, not `.` or
/// `..`, no separator, and on Windows also no `:` (alternate data stream, drive-relative), no device
/// name, no trailing `.` or space (two names would reach one file), none of `< > " | ? *` or controls.
/// Every refusal is Chromium's `TypeError` "Name is not allowed."
pub fn check_name(name: &str) -> Result<(), Fail> {
    let bad = || Err(Fail::type_error(BAD_NAME));
    if name.is_empty() || name == "." || name == ".." {
        return bad();
    }
    if name.chars().any(|c| {
        matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 0x20
    }) {
        return bad();
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return bad();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_lowercase();
    if DEVICE_NAMES.contains(&stem.as_str()) {
        return bad();
    }
    Ok(())
}

pub fn op_stat_file(path: &Path) -> Result<(u64, Option<std::time::SystemTime>), Fail> {
    let m = std::fs::metadata(path).map_err(|e| fail_io(&e, Access::Read))?;
    if m.is_dir() {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    Ok((m.len(), m.modified().ok()))
}

fn ensure_dir(dir: &Path) -> Result<(), Fail> {
    let m = std::fs::metadata(dir).map_err(|e| fail_io(&e, Access::Read))?;
    if !m.is_dir() {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    Ok(())
}

/// `name` has passed [`check_name`]. An existing entry of the other kind is `TypeMismatchError`.
pub fn op_child(dir: &Path, name: &str, kind: Kind, create: bool) -> Result<PathBuf, Fail> {
    ensure_dir(dir)?;
    let child = dir.join(name);
    match std::fs::metadata(&child) {
        Ok(m) => {
            let is_dir = m.is_dir();
            if is_dir != (kind == Kind::Directory) {
                return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
            }
            Ok(child)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            let made = match kind {
                Kind::Directory => std::fs::create_dir(&child),
                // `create_new`: two racing creates must not truncate each other's file.
                Kind::File => std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&child)
                    .map(|_| ()),
            };
            match made {
                Ok(()) => Ok(child),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    op_child(dir, name, kind, false)
                }
                Err(e) => Err(fail_io(&e, Access::Write)),
            }
        }
        Err(e) => Err(fail_io(&e, Access::Read)),
    }
}

pub fn op_remove(dir: &Path, name: &str, recursive: bool) -> Result<(), Fail> {
    ensure_dir(dir)?;
    let child = dir.join(name);
    let m = std::fs::symlink_metadata(&child).map_err(|e| fail_io(&e, Access::Write))?;
    // A link is removed as the link, never what it points at.
    let r = if m.is_dir() {
        if recursive {
            std::fs::remove_dir_all(&child)
        } else {
            std::fs::remove_dir(&child)
        }
    } else if m.file_type().is_symlink() && std::fs::metadata(&child).is_ok_and(|t| t.is_dir()) {
        std::fs::remove_dir(&child)
    } else {
        std::fs::remove_file(&child)
    };
    r.map_err(|e| fail_io(&e, Access::Write))
}

fn identity(p: &Path) -> String {
    let canon = canonical_prefix(p);
    let s = canon.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

/// A removed entry still compares equal to its live siblings: the deepest existing ancestor is
/// canonicalized (symlinks, 8.3 names) and the missing tail is appended as written.
fn canonical_prefix(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut tail = Vec::new();
    let mut at = abs.as_path();
    loop {
        if let Ok(c) = std::fs::canonicalize(at) {
            return tail.iter().rev().fold(c, |acc, name| acc.join(name));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                at = parent;
            }
            _ => return abs,
        }
    }
}

pub fn op_same(a: &Path, a_kind: Kind, b: &Path, b_kind: Kind) -> bool {
    a_kind == b_kind && identity(a) == identity(b)
}

pub fn op_resolve(root: &Path, desc: &Path) -> Option<Vec<String>> {
    let (r, d) = (identity(root), identity(desc));
    let sep = std::path::MAIN_SEPARATOR;
    if d == r {
        return Some(Vec::new());
    }
    let prefix = if r.ends_with(sep) {
        r.clone()
    } else {
        format!("{r}{sep}")
    };
    if !d.starts_with(&prefix) {
        return None;
    }
    let below = d[prefix.len()..]
        .split(sep)
        .filter(|c| !c.is_empty())
        .count();
    let comps: Vec<String> = desc
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if comps.len() < below {
        return None;
    }
    Some(comps[comps.len() - below..].to_vec())
}

pub const DIR_BATCH: usize = 256;

pub struct Cursor {
    dir: PathBuf,
    iter: Option<std::fs::ReadDir>,
}

impl Cursor {
    pub fn open(dir: &Path) -> Result<Cursor, Fail> {
        ensure_dir(dir)?;
        let iter = std::fs::read_dir(dir).map_err(|e| fail_io(&e, Access::Read))?;
        Ok(Cursor {
            dir: dir.to_path_buf(),
            iter: Some(iter),
        })
    }

    pub fn next(&mut self, max: usize) -> Result<(Vec<(PathBuf, Kind)>, bool), Fail> {
        let mut out = Vec::new();
        let Some(iter) = self.iter.as_mut() else {
            return Ok((out, true));
        };
        while out.len() < max {
            match iter.next() {
                None => {
                    self.iter = None;
                    return Ok((out, true));
                }
                Some(Err(e)) => return Err(fail_io(&e, Access::Read)),
                Some(Ok(entry)) => {
                    let path = entry.path();
                    let Ok(ft) = entry.file_type() else { continue };
                    let kind = if ft.is_symlink() {
                        match std::fs::metadata(&path) {
                            Ok(m) if m.is_dir() => Kind::Directory,
                            Ok(m) if m.is_file() => Kind::File,
                            _ => continue,
                        }
                    } else if ft.is_dir() {
                        Kind::Directory
                    } else if ft.is_file() {
                        Kind::File
                    } else {
                        continue;
                    };
                    out.push((path, kind));
                }
            }
        }
        Ok((out, false))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn cursors() -> &'static Mutex<HashMap<u64, Arc<Mutex<Cursor>>>> {
    static C: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Cursor>>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_CURSOR: AtomicU64 = AtomicU64::new(1);

pub fn start_get_file(id: u64) -> Result<u32, Fail> {
    let e = match granted(id) {
        Ok(e) => e,
        Err(f) => return Err(f),
    };
    if e.kind != Kind::File {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    let token = crate::next_token();
    pool::submit(token, move || {
        let (size, mtime) = op_stat_file(&e.path)?;
        let name = name_of(&e.path);
        let payload = json!({ "name": name, "size": size }).to_string();
        let path = e.path.clone();
        Ok(Done::with_value(payload, move |g| unsafe {
            sk_runtime::redjs::webapi::file::create_file(g, &path, &name, None, size, mtime, "")
        }))
    });
    Ok(token)
}

pub fn start_child(dir: u64, name: &str, kind: Kind, create: bool) -> Result<u32, Fail> {
    if let Err(f) = check_name(name) {
        return Err(f);
    }
    let e = match granted(dir) {
        Ok(e) => e,
        Err(f) => return Err(f),
    };
    if e.kind != Kind::Directory {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    let token = crate::next_token();
    let name = name.to_string();
    pool::submit(token, move || {
        let child = op_child(&e.path, &name, kind, create)?;
        Ok(Done::json(&handle_json(child, kind).to_string()))
    });
    Ok(token)
}

pub fn start_remove(dir: u64, name: &str, recursive: bool) -> Result<u32, Fail> {
    if let Err(f) = check_name(name) {
        return Err(f);
    }
    let e = match granted(dir) {
        Ok(e) => e,
        Err(f) => return Err(f),
    };
    if e.kind != Kind::Directory {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    let token = crate::next_token();
    let name = name.to_string();
    pool::submit(token, move || {
        op_remove(&e.path, &name, recursive)?;
        Ok(Done::json("{}"))
    });
    Ok(token)
}

pub fn start_same(a: u64, b: u64) -> Result<u32, Fail> {
    let (Some(ea), Some(eb)) = (lookup(a), lookup(b)) else {
        return Err(gone());
    };
    let token = crate::next_token();
    pool::submit(token, move || {
        Ok(Done::json(
            &json!({ "same": op_same(&ea.path, ea.kind, &eb.path, eb.kind) }).to_string(),
        ))
    });
    Ok(token)
}

pub fn start_resolve(root: u64, other: u64) -> Result<u32, Fail> {
    let (Some(er), Some(eo)) = (lookup(root), lookup(other)) else {
        return Err(gone());
    };
    let token = crate::next_token();
    pool::submit(token, move || {
        let path = if er.kind == Kind::Directory {
            op_resolve(&er.path, &eo.path)
        } else {
            op_same(&er.path, er.kind, &eo.path, eo.kind).then(Vec::new)
        };
        Ok(Done::json(&json!({ "path": path }).to_string()))
    });
    Ok(token)
}

pub fn start_dir_open(dir: u64) -> Result<u32, Fail> {
    let e = match granted(dir) {
        Ok(e) => e,
        Err(f) => return Err(f),
    };
    if e.kind != Kind::Directory {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    let token = crate::next_token();
    pool::submit(token, move || {
        let c = Cursor::open(&e.path)?;
        let id = NEXT_CURSOR.fetch_add(1, Ordering::Relaxed);
        cursors()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, Arc::new(Mutex::new(c)));
        Ok(Done::json(&json!({ "cursor": id }).to_string()))
    });
    Ok(token)
}

pub fn start_dir_next(cursor: u64) -> Result<u32, Fail> {
    let Some(c) = cursors()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&cursor)
        .cloned()
    else {
        return Err(gone());
    };
    let token = crate::next_token();
    pool::submit(token, move || {
        let (batch, done) = c
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .next(DIR_BATCH)?;
        let entries: Vec<serde_json::Value> =
            batch.into_iter().map(|(p, k)| handle_json(p, k)).collect();
        if done {
            cursors()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&cursor);
        }
        Ok(Done::json(
            &json!({ "entries": entries, "done": done }).to_string(),
        ))
    });
    Ok(token)
}

pub fn dir_close(cursor: u64) {
    cursors()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&cursor);
}

pub(crate) fn path_of(id: u64) -> Option<(PathBuf, Kind)> {
    lookup(id).map(|e| (e.path, e.kind))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new(tag: &str) -> Scratch {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "sk-desktop-fs-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_name_fence_refuses_everything_that_leaves_the_folder() {
        for ok in [
            "a.txt",
            "notes",
            ".gitignore",
            "Ünïcødé ✓.md",
            "con-sole.txt",
            "a..b",
            "my console",
        ] {
            assert!(check_name(ok).is_ok(), "{ok:?} must pass");
        }
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            r"a\b",
            "../x",
            r"..\x",
            "a.txt:stream",
            "C:x",
            "CON",
            "con",
            "NUL.txt",
            "com1",
            "LPT9.log",
            "trailing.",
            "trailing ",
            "a<b",
            "a|b",
            "a?b",
            "a*b",
            "a\"b",
            "tab\there",
            "nul\u{0}",
        ] {
            let f = check_name(bad).expect_err(bad);
            assert_eq!(
                (f.name, f.message.as_str()),
                ("TypeError", BAD_NAME),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_child_is_found_made_or_refused_by_kind() {
        let s = Scratch::new("child");
        std::fs::write(s.0.join("a.txt"), b"x").unwrap();
        std::fs::create_dir(s.0.join("sub")).unwrap();
        assert_eq!(
            op_child(&s.0, "a.txt", Kind::File, false).unwrap(),
            s.0.join("a.txt")
        );
        assert_eq!(
            op_child(&s.0, "a.txt", Kind::Directory, false)
                .unwrap_err()
                .name,
            "TypeMismatchError"
        );
        assert_eq!(
            op_child(&s.0, "a.txt", Kind::Directory, true)
                .unwrap_err()
                .name,
            "TypeMismatchError"
        );
        assert_eq!(
            op_child(&s.0, "sub", Kind::File, false).unwrap_err().name,
            "TypeMismatchError"
        );
        assert_eq!(
            op_child(&s.0, "missing", Kind::File, false)
                .unwrap_err()
                .name,
            "NotFoundError"
        );
        let made = op_child(&s.0, "new.txt", Kind::File, true).unwrap();
        assert_eq!(std::fs::read(&made).unwrap(), b"");
        assert_eq!(
            op_child(&s.0, "a.txt", Kind::File, true).unwrap(),
            s.0.join("a.txt")
        );
        assert_eq!(std::fs::read(s.0.join("a.txt")).unwrap(), b"x");
        assert!(op_child(&s.0, "newdir", Kind::Directory, true)
            .unwrap()
            .is_dir());
        assert_eq!(
            op_child(&s.0.join("a.txt"), "x", Kind::File, false)
                .unwrap_err()
                .name,
            "TypeMismatchError"
        );
        assert_eq!(
            op_stat_file(&s.0.join("sub")).unwrap_err().name,
            "TypeMismatchError"
        );
        assert_eq!(op_stat_file(&s.0.join("a.txt")).unwrap().0, 1);
    }

    #[test]
    fn remove_entry_needs_recursive_for_a_non_empty_folder() {
        let s = Scratch::new("remove");
        std::fs::create_dir_all(s.0.join("full/deeper")).unwrap();
        std::fs::write(s.0.join("full/deeper/f"), b"1").unwrap();
        std::fs::create_dir(s.0.join("empty")).unwrap();
        std::fs::write(s.0.join("f.txt"), b"1").unwrap();
        assert_eq!(
            op_remove(&s.0, "full", false).unwrap_err().name,
            "InvalidModificationError"
        );
        assert!(s.0.join("full/deeper/f").exists());
        op_remove(&s.0, "full", true).unwrap();
        assert!(!s.0.join("full").exists());
        op_remove(&s.0, "empty", false).unwrap();
        op_remove(&s.0, "f.txt", false).unwrap();
        assert_eq!(
            op_remove(&s.0, "f.txt", false).unwrap_err().name,
            "NotFoundError"
        );
    }

    #[test]
    fn same_entry_and_resolve_compare_canonical_paths() {
        let s = Scratch::new("same");
        std::fs::create_dir_all(s.0.join("Sub/Deep")).unwrap();
        std::fs::write(s.0.join("Sub/Deep/File.txt"), b"1").unwrap();
        let file = s.0.join("Sub/Deep/File.txt");
        assert!(op_same(
            &file,
            Kind::File,
            &s.0.join("Sub").join("Deep").join("File.txt"),
            Kind::File
        ));
        if cfg!(windows) {
            assert!(op_same(
                &file,
                Kind::File,
                &s.0.join("sub/deep/file.TXT"),
                Kind::File
            ));
        }
        assert!(
            !op_same(&file, Kind::File, &file, Kind::Directory),
            "a different kind is never the same entry"
        );
        assert!(!op_same(&file, Kind::File, &s.0.join("Sub"), Kind::File));
        assert_eq!(
            op_resolve(&s.0, &file),
            Some(vec!["Sub".into(), "Deep".into(), "File.txt".into()])
        );
        assert_eq!(op_resolve(&s.0, &s.0), Some(vec![]));
        assert_eq!(op_resolve(&s.0.join("Sub"), &s.0), None);
        std::fs::create_dir(s.0.join("Sub2")).unwrap();
        assert_eq!(op_resolve(&s.0.join("Sub"), &s.0.join("Sub2")), None);
        std::fs::remove_file(&file).unwrap();
        assert_eq!(op_resolve(&s.0, &file).map(|v| v.len()), Some(3));
    }

    #[test]
    fn the_cursor_comes_back_in_batches_and_reports_link_targets() {
        let s = Scratch::new("cursor");
        for i in 0..600 {
            std::fs::write(s.0.join(format!("f{i:03}")), b"").unwrap();
        }
        std::fs::create_dir(s.0.join("d")).unwrap();
        let mut c = Cursor::open(&s.0).unwrap();
        let mut sizes = Vec::new();
        let mut all = Vec::new();
        loop {
            let (batch, done) = c.next(DIR_BATCH).unwrap();
            sizes.push(batch.len());
            all.extend(batch);
            if done {
                break;
            }
        }
        assert_eq!(all.len(), 601);
        assert_eq!(sizes[..2], [256, 256]);
        assert!(sizes.iter().all(|&n| n <= DIR_BATCH));
        assert_eq!(all.iter().filter(|(_, k)| *k == Kind::Directory).count(), 1);
        assert_eq!(c.next(DIR_BATCH).unwrap(), (vec![], true));
        assert_eq!(
            Cursor::open(&s.0.join("f000")).err().map(|f| f.name),
            Some("TypeMismatchError")
        );
    }

    #[test]
    fn handles_are_granted_and_released_by_id() {
        let report = PathBuf::from(if cfg!(windows) {
            r"C:\x\report.txt"
        } else {
            "/x/report.txt"
        });
        let v = handle_json(report.clone(), Kind::File);
        assert_eq!(
            (v["name"].as_str(), v["kind"].as_str()),
            (Some("report.txt"), Some("file"))
        );
        let id = v["h"].as_u64().unwrap();
        assert_eq!(lookup(id).map(|e| e.path), Some(report));
        release(id);
        assert!(lookup(id).is_none());
        assert_eq!(start_get_file(id).unwrap_err().name, "InvalidStateError");
        if cfg!(windows) {
            assert_eq!(name_of(Path::new(r"C:\")), r"C:\");
        }
    }

    #[test]
    fn a_grant_covers_its_path_and_what_is_below_it_and_nothing_else() {
        let s = Scratch::new("grant");
        let root = s.0.join("Picked");
        std::fs::create_dir_all(root.join("inner")).unwrap();
        std::fs::write(root.join("inner/a.txt"), b"a").unwrap();
        std::fs::write(s.0.join("Picked2.txt"), b"b").unwrap();
        let h = register(root.clone(), Kind::Directory);
        assert_eq!(permission(h, false), Some("prompt"));
        for refused in [
            start_dir_open(h),
            start_child(h, "inner", Kind::Directory, false),
            start_remove(h, "inner", true),
        ] {
            let f = refused.unwrap_err();
            assert_eq!(
                (f.name, f.message.as_str()),
                ("NotAllowedError", NOT_ALLOWED)
            );
        }
        assert!(root.join("inner/a.txt").exists(), "nothing ran");
        assert_eq!(permission(h, true), Some("granted"));
        assert!(is_granted(&root.join("inner").join("a.txt")));
        if cfg!(windows) {
            assert!(
                is_granted(Path::new(&root.to_string_lossy().to_uppercase())),
                "NTFS names compare without case"
            );
            assert!(is_granted(&PathBuf::from(format!("{}\\", root.display()))));
        }
        assert!(!is_granted(&s.0.join("Picked2.txt")));
        assert!(!is_granted(&s.0));
        let f = register(s.0.join("Picked2.txt"), Kind::File);
        assert_eq!(start_get_file(f).unwrap_err().name, "NotAllowedError");
        assert_eq!(permission(f, false), Some("prompt"));
        assert_eq!(
            permission(u64::MAX, true),
            None,
            "an unknown id grants nothing"
        );
        assert_eq!(
            writable::start_open(f, false).unwrap_err().name,
            "NotAllowedError"
        );
        release(h);
        release(f);
    }

    #[test]
    fn a_serialized_handle_is_its_path_and_kind_and_nothing_else_parses() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\data\ünï\report.txt")
        } else {
            PathBuf::from("/data/report.txt")
        };
        let id = register(path.clone(), Kind::File);
        let bytes = to_wire(id).unwrap();
        assert_eq!(from_wire(&bytes), Some((path.clone(), Kind::File)));
        release(id);
        assert_eq!(to_wire(id), None, "a released handle has nothing to write");
        let dir = register(path.parent().unwrap().to_path_buf(), Kind::Directory);
        assert_eq!(
            from_wire(&to_wire(dir).unwrap()).map(|(_, k)| k),
            Some(Kind::Directory)
        );
        release(dir);
        for bad in [
            &br#"{"v":2,"kind":"file","path":"C:\\x"}"#[..],
            br#"{"v":1,"kind":"link","path":"C:\\x"}"#,
            br#"{"v":1,"kind":"file","path":"relative\\x"}"#,
            br#"{"v":1,"kind":"file"}"#,
            b"not json",
        ] {
            assert_eq!(from_wire(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn a_refused_name_never_reaches_the_pool() {
        let dir = register(std::env::temp_dir(), Kind::Directory);
        let f = start_child(dir, "..", Kind::File, true).unwrap_err();
        assert_eq!((f.name, f.message.as_str()), ("TypeError", BAD_NAME));
        assert_eq!(
            start_remove(dir, "a:b", false).unwrap_err().name,
            "TypeError"
        );
        release(dir);
    }
}
