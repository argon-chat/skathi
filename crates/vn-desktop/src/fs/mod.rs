//! File System Access, the host side (docs/DESKTOP.md §2; design §4.3, §4.7).
//!
//! # Handles are ids, and JS never holds a path
//!
//! A `FileSystemHandle` in JS carries a number. The path lives HERE, in
//! [`HANDLES`], and a handle exists only for something the user granted:
//!
//! - a picker's result (`dialog`), a forwarded or initial launch's existing
//!   paths (`launch`), a dropped path (the `getAsFileSystemHandle` hook);
//! - a child of a directory handle, reached through the NAME FENCE
//!   ([`check_name`]): one path component, never `..`, a separator, an NTFS
//!   stream (`:`) or a device name. A directory handle therefore reaches its
//!   own subtree and nothing else.
//!
//! So no host function takes a path from script, and none returns one: the
//! owner dropped the tier-2 `appFs`, and these `__vn_fs_*` globals must not
//! be an undocumented one. What a page can learn about where a file is stays
//! what a browser shows: a `File`'s `name` and its `webkitRelativePath`.
//!
//! # Threads
//!
//! Everything below that touches the disk runs on the pool (`pool.rs`). The
//! `start_*` functions run on the frame thread inside the host fns: they look
//! the handle up, apply the name fence, and either refuse synchronously
//! (`{error, name}`) or queue the operation and answer `{token}`.
//!
//! # Errors
//!
//! [`fail_io`] is design §3.3's table, tier 1 only (DOMException names, with
//! Chromium's messages, because pasted errors get searched).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;

use crate::{pool, Done, Fail};

pub mod writable;

// ---------------------------------------------------------------------------
// the handle table
// ---------------------------------------------------------------------------

/// What a handle names.
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

/// Grants a handle on `path`. Called only by the places listed in the module
/// docs; the kind comes from a stat that already happened (a dialog thread, a
/// launch, a drop) or from the child lookup that made it.
pub(crate) fn register(path: PathBuf, kind: Kind) -> u64 {
    let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    handles().lock().unwrap_or_else(|p| p.into_inner()).insert(id, Entry { path, kind });
    id
}

fn lookup(id: u64) -> Option<Entry> {
    handles().lock().unwrap_or_else(|p| p.into_inner()).get(&id).cloned()
}

/// Forgets a handle whose JS object was collected (`FinalizationRegistry`).
pub fn release(id: u64) {
    handles().lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
}

/// The name a handle reports: the last component, or the whole root for a
/// drive (`C:\`), which is what Chromium names a picked drive.
pub fn name_of(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// `{h, name, kind}` — how a handle crosses into JS.
pub(crate) fn handle_json(path: PathBuf, kind: Kind) -> serde_json::Value {
    let name = name_of(&path);
    json!({ "h": register(path, kind), "name": name, "kind": kind.as_str() })
}

fn token_json(token: u32) -> String {
    format!("{{\"token\":{token}}}")
}

fn gone() -> Fail {
    // A handle id the table does not hold: released already, or never ours.
    // Chromium has no such state (a handle cannot outlive its object), so the
    // nearest honest name is the one for "the state you cached is stale".
    Fail::new("InvalidStateError", INVALID_STATE)
}

// ---------------------------------------------------------------------------
// the error map (design §3.3, tier 1)
// ---------------------------------------------------------------------------

pub const NOT_FOUND: &str = "A requested file or directory could not be found at the time an operation was processed.";
pub const TYPE_MISMATCH: &str = "The path supplied exists, but was not an entry of requested type.";
pub const INVALID_MODIFICATION: &str = "The object can not be modified in this way.";
pub const NOT_ALLOWED: &str = "The request is not allowed by the user agent or the platform in the current context.";
pub const NO_MODIFICATION: &str =
    "An attempt was made to write to a file or directory which could not be modified due to the state of the underlying filesystem.";
pub const QUOTA: &str = "The operation failed because it would cause the application to exceed its storage quota.";
pub const INVALID_STATE: &str =
    "An operation that depends on state cached in an interface object was made but the state had changed since it was read from disk.";
pub const NOT_READABLE: &str =
    "The requested file could not be read, typically due to permission problems that have occurred after a reference to a file was acquired.";
/// Chromium's TypeError for a name the fence refuses.
pub const BAD_NAME: &str = "Name is not allowed.";

/// Whether the operation that failed was reading or writing: a refusal is
/// `NotAllowedError` for one and `NoModificationAllowedError` for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// An `io::Error` → the DOMException a File System Access promise rejects
/// with. The Win32 code decides first, because std folds several of them
/// into `Other` / `Uncategorized`.
pub fn fail_io(e: &std::io::Error, access: Access) -> Fail {
    use std::io::ErrorKind as K;
    let os = e.raw_os_error();
    let named = |name: &'static str, msg: &str| Fail::new(name, msg);
    let f = match (os, e.kind()) {
        (Some(2 | 3), _) | (_, K::NotFound) => named("NotFoundError", NOT_FOUND),
        // ERROR_DIR_NOT_EMPTY: removeEntry without `recursive`.
        (Some(145), _) | (_, K::DirectoryNotEmpty) => named("InvalidModificationError", INVALID_MODIFICATION),
        // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL.
        (Some(112 | 39), _) | (_, K::StorageFull) => named("QuotaExceededError", QUOTA),
        // ERROR_DIRECTORY ("the directory name is invalid": a file where a
        // folder was expected) and its std spellings.
        (Some(267), _) | (_, K::NotADirectory | K::IsADirectory) => named("TypeMismatchError", TYPE_MISMATCH),
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION,
        // ERROR_WRITE_PROTECT, and ReplaceFileW's three "could not swap" codes.
        (Some(5 | 32 | 33 | 19 | 1175 | 1176 | 1177), _) | (_, K::PermissionDenied | K::ReadOnlyFilesystem) => {
            match access {
                Access::Read => named("NotAllowedError", NOT_ALLOWED),
                Access::Write => named("NoModificationAllowedError", NO_MODIFICATION),
            }
        }
        _ => Fail::new("InvalidStateError", format!("{INVALID_STATE} ({e})")),
    };
    match os {
        Some(code) => f.os(code),
        None => f,
    }
}

// ---------------------------------------------------------------------------
// the name fence
// ---------------------------------------------------------------------------

/// Windows device names: `CON` opens the console, `NUL` the bit bucket, with
/// or without an extension (`nul.txt` is still NUL on every Windows before
/// 11, and still is there for some APIs).
const DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "conin$", "conout$", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "com¹", "com²", "com³", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
    "lpt9", "lpt¹", "lpt²", "lpt³",
];

/// The traversal fence of `getFileHandle` / `getDirectoryHandle` /
/// `removeEntry` (design §4.3). Chromium's rule is "not empty, not `.` or
/// `..`, no separator"; on Windows the same name must also not be able to
/// leave the folder or open something that is not a file in it:
///
/// - any `:` — `a.txt:payload` is an NTFS alternate data stream, `C:x` is
///   drive-relative;
/// - a device name, with or without an extension;
/// - a trailing `.` or space, which Windows strips (`a.txt.` IS `a.txt`, so
///   two names would reach one file);
/// - the characters Windows never allows in a name (`< > " | ? *`) and the
///   control characters.
///
/// Every refusal is Chromium's `TypeError` "Name is not allowed."
pub fn check_name(name: &str) -> Result<(), Fail> {
    let bad = || Err(Fail::type_error(BAD_NAME));
    if name.is_empty() || name == "." || name == ".." {
        return bad();
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 0x20) {
        return bad();
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return bad();
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_lowercase();
    if DEVICE_NAMES.contains(&stem.as_str()) {
        return bad();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// the operations (pure: run on the pool, unit-tested without JS)
// ---------------------------------------------------------------------------

/// `getFile()`: the file as it is now — its size and mtime are the snapshot
/// the `File` checks every read against.
pub fn op_stat_file(path: &Path) -> Result<(u64, Option<std::time::SystemTime>), Fail> {
    let m = std::fs::metadata(path).map_err(|e| fail_io(&e, Access::Read))?;
    if m.is_dir() {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    Ok((m.len(), m.modified().ok()))
}

/// The directory a handle names, still a directory.
fn ensure_dir(dir: &Path) -> Result<(), Fail> {
    let m = std::fs::metadata(dir).map_err(|e| fail_io(&e, Access::Read))?;
    if !m.is_dir() {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    Ok(())
}

/// `getFileHandle(name, {create})` / `getDirectoryHandle(name, {create})`.
/// `name` has passed [`check_name`]. An existing entry of the other kind is
/// `TypeMismatchError` whether or not `create` is set, as the spec says.
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
                // `create_new`: two racing creates must not truncate each
                // other's file; the loser finds it and returns it below.
                Kind::File => std::fs::OpenOptions::new().write(true).create_new(true).open(&child).map(|_| ()),
            };
            match made {
                Ok(()) => Ok(child),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => op_child(dir, name, kind, false),
                Err(e) => Err(fail_io(&e, Access::Write)),
            }
        }
        Err(e) => Err(fail_io(&e, Access::Read)),
    }
}

/// `removeEntry(name, {recursive})`. A non-empty folder without `recursive`
/// is `InvalidModificationError`.
pub fn op_remove(dir: &Path, name: &str, recursive: bool) -> Result<(), Fail> {
    ensure_dir(dir)?;
    let child = dir.join(name);
    let m = std::fs::symlink_metadata(&child).map_err(|e| fail_io(&e, Access::Write))?;
    // A link (a symlink or a junction) is removed as the link: never what it
    // points at. `remove_dir_all` does not follow links either.
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

/// The path to compare two handles by: canonical where it still resolves,
/// absolute and lexical where it does not (a deleted file is still the same
/// entry as another handle on the same path).
fn identity(p: &Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    let s = canon.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    // NTFS compares names case-insensitively, and so does Explorer.
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

/// `isSameEntry`: the same kind and the same entry.
pub fn op_same(a: &Path, a_kind: Kind, b: &Path, b_kind: Kind) -> bool {
    a_kind == b_kind && identity(a) == identity(b)
}

/// `resolve(possibleDescendant)`: the components from `root` down to `desc`
/// (`[]` for the same entry), or `None` when it is not inside.
pub fn op_resolve(root: &Path, desc: &Path) -> Option<Vec<String>> {
    let (r, d) = (identity(root), identity(desc));
    let sep = std::path::MAIN_SEPARATOR;
    if d == r {
        return Some(Vec::new());
    }
    let prefix = if r.ends_with(sep) { r.clone() } else { format!("{r}{sep}") };
    if !d.starts_with(&prefix) {
        return None;
    }
    // The components are the DESCENDANT's own spelling, not the lowercased
    // comparison key: count how many there are below the root, and take that
    // many from the end of its path.
    let below = d[prefix.len()..].split(sep).filter(|c| !c.is_empty()).count();
    let comps: Vec<String> = desc.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if comps.len() < below {
        return None;
    }
    Some(comps[comps.len() - below..].to_vec())
}

// ---------------------------------------------------------------------------
// the directory cursor
// ---------------------------------------------------------------------------

/// Entries per `__vn_fs_dir_next` batch: a 100 000-entry folder is iterated
/// in 391 small JSON strings, never one giant one (design §4.3).
pub const DIR_BATCH: usize = 256;

pub struct Cursor {
    dir: PathBuf,
    iter: Option<std::fs::ReadDir>,
}

impl Cursor {
    pub fn open(dir: &Path) -> Result<Cursor, Fail> {
        ensure_dir(dir)?;
        let iter = std::fs::read_dir(dir).map_err(|e| fail_io(&e, Access::Read))?;
        Ok(Cursor { dir: dir.to_path_buf(), iter: Some(iter) })
    }

    /// Up to `max` entries and whether the folder is exhausted. A link reports
    /// its target's kind, as Chromium does; a link whose target is gone, and
    /// anything that is neither a file nor a folder, is skipped.
    pub fn next(&mut self, max: usize) -> Result<(Vec<(PathBuf, Kind)>, bool), Fail> {
        let mut out = Vec::new();
        let Some(iter) = self.iter.as_mut() else { return Ok((out, true)) };
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

// ---------------------------------------------------------------------------
// starting an operation (frame thread, inside the host fns)
// ---------------------------------------------------------------------------

/// `__vn_fs_get_file(h)`: settles `{name, size, lastModified}` and hands the
/// dispatcher the path-backed `File` (made on the JS thread at delivery from
/// the stat that ran here).
pub fn start_get_file(id: u64) -> String {
    let Some(e) = lookup(id) else { return gone().to_sync_json() };
    if e.kind != Kind::File {
        return Fail::new("TypeMismatchError", TYPE_MISMATCH).to_sync_json();
    }
    let token = crate::next_token();
    pool::submit(token, move || {
        let (size, mtime) = op_stat_file(&e.path)?;
        let name = name_of(&e.path);
        let payload = json!({ "name": name, "size": size }).to_string();
        let path = e.path.clone();
        Ok(Done::with_value(payload, move |g| unsafe {
            vn_runtime::redjs::webapi::file::create_file(g, &path, &name, None, size, mtime, "")
        }))
    });
    token_json(token)
}

/// `__vn_fs_child(dir, name, isDirectory, create)`: settles `{h, name, kind}`.
pub fn start_child(dir: u64, name: &str, kind: Kind, create: bool) -> String {
    if let Err(f) = check_name(name) {
        return f.to_sync_json();
    }
    let Some(e) = lookup(dir) else { return gone().to_sync_json() };
    if e.kind != Kind::Directory {
        return Fail::new("TypeMismatchError", TYPE_MISMATCH).to_sync_json();
    }
    let token = crate::next_token();
    let name = name.to_string();
    pool::submit(token, move || {
        let child = op_child(&e.path, &name, kind, create)?;
        Ok(Done::json(&handle_json(child, kind).to_string()))
    });
    token_json(token)
}

/// `__vn_fs_remove(dir, name, recursive)`.
pub fn start_remove(dir: u64, name: &str, recursive: bool) -> String {
    if let Err(f) = check_name(name) {
        return f.to_sync_json();
    }
    let Some(e) = lookup(dir) else { return gone().to_sync_json() };
    if e.kind != Kind::Directory {
        return Fail::new("TypeMismatchError", TYPE_MISMATCH).to_sync_json();
    }
    let token = crate::next_token();
    let name = name.to_string();
    pool::submit(token, move || {
        op_remove(&e.path, &name, recursive)?;
        Ok(Done::json("{}"))
    });
    token_json(token)
}

/// `__vn_fs_same(a, b)`: settles `{same}`. Canonicalising is I/O, so this is
/// async even though the answer is a boolean.
pub fn start_same(a: u64, b: u64) -> String {
    let (Some(ea), Some(eb)) = (lookup(a), lookup(b)) else { return gone().to_sync_json() };
    let token = crate::next_token();
    pool::submit(token, move || Ok(Done::json(&json!({ "same": op_same(&ea.path, ea.kind, &eb.path, eb.kind) }).to_string())));
    token_json(token)
}

/// `__vn_fs_resolve(root, other)`: settles `{path: string[] | null}`.
pub fn start_resolve(root: u64, other: u64) -> String {
    let (Some(er), Some(eo)) = (lookup(root), lookup(other)) else { return gone().to_sync_json() };
    let token = crate::next_token();
    pool::submit(token, move || {
        let path = if er.kind == Kind::Directory { op_resolve(&er.path, &eo.path) } else { op_same(&er.path, er.kind, &eo.path, eo.kind).then(Vec::new) };
        Ok(Done::json(&json!({ "path": path }).to_string()))
    });
    token_json(token)
}

/// `__vn_fs_dir_open(dir)`: settles `{cursor}`.
pub fn start_dir_open(dir: u64) -> String {
    let Some(e) = lookup(dir) else { return gone().to_sync_json() };
    if e.kind != Kind::Directory {
        return Fail::new("TypeMismatchError", TYPE_MISMATCH).to_sync_json();
    }
    let token = crate::next_token();
    pool::submit(token, move || {
        let c = Cursor::open(&e.path)?;
        let id = NEXT_CURSOR.fetch_add(1, Ordering::Relaxed);
        cursors().lock().unwrap_or_else(|p| p.into_inner()).insert(id, Arc::new(Mutex::new(c)));
        Ok(Done::json(&json!({ "cursor": id }).to_string()))
    });
    token_json(token)
}

/// `__vn_fs_dir_next(cursor)`: settles `{entries: [{h, name, kind}], done}`.
/// Each entry is a handle granted here: it is inside the folder the user
/// picked, which is what a directory handle grants.
pub fn start_dir_next(cursor: u64) -> String {
    let Some(c) = cursors().lock().unwrap_or_else(|p| p.into_inner()).get(&cursor).cloned() else {
        return gone().to_sync_json();
    };
    let token = crate::next_token();
    pool::submit(token, move || {
        let (batch, done) = c.lock().unwrap_or_else(|p| p.into_inner()).next(DIR_BATCH)?;
        let entries: Vec<serde_json::Value> = batch.into_iter().map(|(p, k)| handle_json(p, k)).collect();
        if done {
            cursors().lock().unwrap_or_else(|p| p.into_inner()).remove(&cursor);
        }
        Ok(Done::json(&json!({ "entries": entries, "done": done }).to_string()))
    });
    token_json(token)
}

/// `__vn_fs_dir_close(cursor)`: an iteration abandoned early (`break` out of
/// `for await`). Dropping the `ReadDir` closes the OS handle.
pub fn dir_close(cursor: u64) {
    cursors().lock().unwrap_or_else(|p| p.into_inner()).remove(&cursor);
}

/// The path behind a handle id, for the other host modules (a picker's
/// `startIn`). Never handed to JS.
pub(crate) fn path_of(id: u64) -> Option<(PathBuf, Kind)> {
    lookup(id).map(|e| (e.path, e.kind))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A scratch folder under the OS temp dir, removed on drop.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new(tag: &str) -> Scratch {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir()
                .join(format!("vn-desktop-fs-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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
        for ok in ["a.txt", "notes", ".gitignore", "Ünïcødé ✓.md", "con-sole.txt", "a..b", "my console"] {
            assert!(check_name(ok).is_ok(), "{ok:?} must pass");
        }
        for bad in [
            "", ".", "..", "a/b", r"a\b", "../x", r"..\x", "a.txt:stream", "C:x", "CON", "con", "NUL.txt", "com1",
            "LPT9.log", "trailing.", "trailing ", "a<b", "a|b", "a?b", "a*b", "a\"b", "tab\there", "nul\u{0}",
        ] {
            let f = check_name(bad).expect_err(bad);
            assert_eq!((f.name, f.message.as_str()), ("TypeError", BAD_NAME), "{bad:?}");
        }
    }

    #[test]
    fn io_errors_map_to_file_system_access_names() {
        let os = std::io::Error::from_raw_os_error;
        assert_eq!(fail_io(&os(2), Access::Read).name, "NotFoundError");
        assert_eq!(fail_io(&os(3), Access::Write).name, "NotFoundError");
        assert_eq!(fail_io(&os(145), Access::Write).name, "InvalidModificationError");
        assert_eq!(fail_io(&os(5), Access::Read).name, "NotAllowedError");
        assert_eq!(fail_io(&os(32), Access::Write).name, "NoModificationAllowedError");
        assert_eq!(fail_io(&os(1175), Access::Write).name, "NoModificationAllowedError");
        assert_eq!(fail_io(&os(112), Access::Write).name, "QuotaExceededError");
        assert_eq!(fail_io(&os(267), Access::Read).name, "TypeMismatchError");
        let other = fail_io(&os(1117), Access::Read);
        assert_eq!(other.name, "InvalidStateError");
        assert!(other.message.starts_with(INVALID_STATE), "{}", other.message);
        assert_eq!(fail_io(&os(2), Access::Read).message, NOT_FOUND);
        assert_eq!(fail_io(&os(2), Access::Read).os_error, Some(2));
    }

    #[test]
    fn a_child_is_found_made_or_refused_by_kind() {
        let s = Scratch::new("child");
        std::fs::write(s.0.join("a.txt"), b"x").unwrap();
        std::fs::create_dir(s.0.join("sub")).unwrap();
        assert_eq!(op_child(&s.0, "a.txt", Kind::File, false).unwrap(), s.0.join("a.txt"));
        assert_eq!(op_child(&s.0, "a.txt", Kind::Directory, false).unwrap_err().name, "TypeMismatchError");
        assert_eq!(op_child(&s.0, "a.txt", Kind::Directory, true).unwrap_err().name, "TypeMismatchError");
        assert_eq!(op_child(&s.0, "sub", Kind::File, false).unwrap_err().name, "TypeMismatchError");
        assert_eq!(op_child(&s.0, "missing", Kind::File, false).unwrap_err().name, "NotFoundError");
        let made = op_child(&s.0, "new.txt", Kind::File, true).unwrap();
        assert_eq!(std::fs::read(&made).unwrap(), b"");
        // `create` on an existing file returns it and leaves its bytes alone.
        assert_eq!(op_child(&s.0, "a.txt", Kind::File, true).unwrap(), s.0.join("a.txt"));
        assert_eq!(std::fs::read(s.0.join("a.txt")).unwrap(), b"x");
        assert!(op_child(&s.0, "newdir", Kind::Directory, true).unwrap().is_dir());
        // A handle on a file is not a folder to look into.
        assert_eq!(op_child(&s.0.join("a.txt"), "x", Kind::File, false).unwrap_err().name, "TypeMismatchError");
        assert_eq!(op_stat_file(&s.0.join("sub")).unwrap_err().name, "TypeMismatchError");
        assert_eq!(op_stat_file(&s.0.join("a.txt")).unwrap().0, 1);
    }

    #[test]
    fn remove_entry_needs_recursive_for_a_non_empty_folder() {
        let s = Scratch::new("remove");
        std::fs::create_dir_all(s.0.join("full/deeper")).unwrap();
        std::fs::write(s.0.join("full/deeper/f"), b"1").unwrap();
        std::fs::create_dir(s.0.join("empty")).unwrap();
        std::fs::write(s.0.join("f.txt"), b"1").unwrap();
        assert_eq!(op_remove(&s.0, "full", false).unwrap_err().name, "InvalidModificationError");
        assert!(s.0.join("full/deeper/f").exists());
        op_remove(&s.0, "full", true).unwrap();
        assert!(!s.0.join("full").exists());
        op_remove(&s.0, "empty", false).unwrap();
        op_remove(&s.0, "f.txt", false).unwrap();
        assert_eq!(op_remove(&s.0, "f.txt", false).unwrap_err().name, "NotFoundError");
    }

    #[test]
    fn same_entry_and_resolve_compare_canonical_paths() {
        let s = Scratch::new("same");
        std::fs::create_dir_all(s.0.join("Sub/Deep")).unwrap();
        std::fs::write(s.0.join("Sub/Deep/File.txt"), b"1").unwrap();
        let file = s.0.join("Sub/Deep/File.txt");
        assert!(op_same(&file, Kind::File, &s.0.join("Sub").join("Deep").join("File.txt"), Kind::File));
        if cfg!(windows) {
            assert!(op_same(&file, Kind::File, &s.0.join("sub/deep/file.TXT"), Kind::File));
        }
        assert!(!op_same(&file, Kind::File, &file, Kind::Directory), "a different kind is never the same entry");
        assert!(!op_same(&file, Kind::File, &s.0.join("Sub"), Kind::File));
        assert_eq!(op_resolve(&s.0, &file), Some(vec!["Sub".into(), "Deep".into(), "File.txt".into()]));
        assert_eq!(op_resolve(&s.0, &s.0), Some(vec![]));
        assert_eq!(op_resolve(&s.0.join("Sub"), &s.0), None);
        // A sibling whose name merely starts with the root's is not inside it.
        std::fs::create_dir(s.0.join("Sub2")).unwrap();
        assert_eq!(op_resolve(&s.0.join("Sub"), &s.0.join("Sub2")), None);
        // A deleted file still resolves lexically.
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
        // Exhausted stays exhausted.
        assert_eq!(c.next(DIR_BATCH).unwrap(), (vec![], true));
        assert_eq!(Cursor::open(&s.0.join("f000")).err().map(|f| f.name), Some("TypeMismatchError"));
    }

    #[test]
    fn handles_are_granted_and_released_by_id() {
        // By id, not by counting the table: other tests grant handles in
        // parallel.
        let v = handle_json(PathBuf::from(r"C:\x\report.txt"), Kind::File);
        assert_eq!((v["name"].as_str(), v["kind"].as_str()), (Some("report.txt"), Some("file")));
        let id = v["h"].as_u64().unwrap();
        assert_eq!(lookup(id).map(|e| e.path), Some(PathBuf::from(r"C:\x\report.txt")));
        release(id);
        assert!(lookup(id).is_none());
        // An unknown id answers synchronously, with no token.
        let out: serde_json::Value = serde_json::from_str(&start_get_file(id)).unwrap();
        assert_eq!(out["name"], "InvalidStateError");
        assert!(out.get("token").is_none());
        if cfg!(windows) {
            assert_eq!(name_of(Path::new(r"C:\")), r"C:\");
        }
    }

    #[test]
    fn a_refused_name_never_reaches_the_pool() {
        let dir = register(std::env::temp_dir(), Kind::Directory);
        let out: serde_json::Value = serde_json::from_str(&start_child(dir, "..", Kind::File, true)).unwrap();
        assert_eq!((out["name"].as_str(), out["error"].as_str()), (Some("TypeError"), Some(BAD_NAME)));
        let out: serde_json::Value = serde_json::from_str(&start_remove(dir, "a:b", false)).unwrap();
        assert_eq!(out["name"], "TypeError");
        release(dir);
    }
}
