//! `FileSystemWritableFileStream`, the host side: writes go to a swap file beside the target and are
//! swapped in atomically on `close()`, as Chromium's `.crswap` does.
//!
//! `<name>.vnswap` lives in the target's own folder so the final rename never crosses a volume. The target
//! is untouched until `close()`, so a crash, `abort()` or failed write leaves it as it was; `close()`
//! replaces it (`ReplaceFileW`, keeping ACL and creation time, or `MoveFileExW` when it is gone). A swap
//! file left by a crash is swept by the next `createWritable()`. The swap file is opened without
//! `FILE_SHARE_DELETE` so another process's sweep cannot delete it mid-write; this process's own open
//! ones ([`open_swaps`]) are never swept.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;

use super::{fail_io, granted, Access, Kind, NOT_FOUND, NOT_READABLE, TYPE_MISMATCH};
use crate::{pool, Done, Fail};

pub enum Data {
    Bytes(Vec<u8>),
    Blob(sk_runtime::redjs::webapi::blob::Range),
}

pub struct Writable {
    target: PathBuf,
    swap: PathBuf,
    file: Option<File>,
    cursor: u64,
    finished: bool,
}

fn open_swaps() -> &'static Mutex<HashSet<PathBuf>> {
    static S: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_swap_of(candidate: &str, name: &str) -> bool {
    let Some(rest) = candidate.strip_prefix(name) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(".vnswap") else {
        return false;
    };
    rest.is_empty()
        || rest
            .strip_prefix('-')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub fn sweep_orphans(target: &Path) {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return;
    };
    let name = name.to_string_lossy();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let ours = open_swaps()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    for e in rd.flatten() {
        let n = e.file_name();
        if is_swap_of(&n.to_string_lossy(), &name) && !ours.contains(&e.path()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(windows)]
fn open_swap_file(p: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(p)
}

#[cfg(not(windows))]
fn open_swap_file(p: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(p)
}

impl Writable {
    /// `must_exist` is the File System Access rule (a deleted file is `NotFoundError`); a download writes a target the dialog only named.
    pub fn open(target: &Path, keep_existing: bool, must_exist: bool) -> Result<Writable, Fail> {
        match std::fs::metadata(target) {
            Ok(m) if m.is_dir() => return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH)),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => {}
            Err(e) => return Err(fail_io(&e, Access::Read)),
        }
        sweep_orphans(target);
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = target
            .parent()
            .ok_or_else(|| Fail::new("NotFoundError", NOT_FOUND))?;
        let mut last = None;
        for n in 0..64 {
            let swap = if n == 0 {
                dir.join(format!("{name}.vnswap"))
            } else {
                dir.join(format!("{name}.vnswap-{n}"))
            };
            match open_swap_file(&swap) {
                Ok(mut file) => {
                    open_swaps()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(swap.clone());
                    let mut w = Writable {
                        target: target.to_path_buf(),
                        swap,
                        file: None,
                        cursor: 0,
                        finished: false,
                    };
                    if keep_existing {
                        let copied = File::open(target)
                            .and_then(|mut src| std::io::copy(&mut src, &mut file));
                        if let Err(e) = copied {
                            w.file = Some(file);
                            w.abort();
                            return Err(fail_io(&e, Access::Write));
                        }
                    }
                    w.file = Some(file);
                    return Ok(w);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(fail_io(&e, Access::Write)),
            }
        }
        Err(fail_io(
            &last.unwrap_or_else(|| std::io::Error::from_raw_os_error(80)),
            Access::Write,
        ))
    }

    fn file(&mut self) -> Result<&mut File, Fail> {
        self.file
            .as_mut()
            .ok_or_else(|| Fail::new("InvalidStateError", super::INVALID_STATE))
    }

    fn len(&mut self) -> Result<u64, Fail> {
        self.file()?
            .metadata()
            .map(|m| m.len())
            .map_err(|e| fail_io(&e, Access::Write))
    }

    pub fn write(&mut self, position: Option<u64>, data: &Data) -> Result<(), Fail> {
        if let Some(p) = position {
            self.cursor = p;
        }
        let len = self.len()?;
        let cursor = self.cursor;
        if cursor > len {
            self.file()?
                .set_len(cursor)
                .map_err(|e| fail_io(&e, Access::Write))?;
        }
        let at = self.cursor;
        let file = self.file()?;
        file.seek(SeekFrom::Start(at))
            .map_err(|e| fail_io(&e, Access::Write))?;
        let written = match data {
            Data::Bytes(b) => {
                file.write_all(b).map_err(|e| fail_io(&e, Access::Write))?;
                b.len() as u64
            }
            Data::Blob(range) => {
                use sk_runtime::redjs::webapi::blob::CopyError;
                let mut counted = Counting { inner: file, n: 0 };
                match range.copy_into(&mut counted) {
                    Ok(()) => counted.n,
                    Err(CopyError::Write(e)) => return Err(fail_io(&e, Access::Write)),
                    Err(CopyError::Read(r)) => {
                        return Err(Fail::new(
                            r.name(),
                            if r.name() == "NotFoundError" {
                                NOT_FOUND
                            } else {
                                NOT_READABLE
                            },
                        ))
                    }
                }
            }
        };
        self.cursor = at + written;
        Ok(())
    }

    pub fn seek(&mut self, position: u64) {
        self.cursor = position;
    }

    pub fn truncate(&mut self, size: u64) -> Result<(), Fail> {
        self.file()?
            .set_len(size)
            .map_err(|e| fail_io(&e, Access::Write))?;
        if self.cursor > size {
            self.cursor = size;
        }
        Ok(())
    }

    pub fn close(mut self) -> Result<(), Fail> {
        let flushed = self.file()?.sync_all();
        self.file = None;
        if let Err(e) = flushed {
            self.abort();
            return Err(fail_io(&e, Access::Write));
        }
        let r = replace(&self.target, &self.swap);
        self.forget_swap();
        self.finished = true;
        if r.is_err() {
            let _ = std::fs::remove_file(&self.swap);
        }
        r.map_err(|e| fail_io(&e, Access::Write))
    }

    pub fn abort(&mut self) {
        self.file = None;
        let _ = std::fs::remove_file(&self.swap);
        self.forget_swap();
        self.finished = true;
    }

    fn forget_swap(&self) {
        open_swaps()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.swap);
    }

    #[cfg(test)]
    pub(crate) fn swap_path(&self) -> &Path {
        &self.swap
    }
}

impl Drop for Writable {
    fn drop(&mut self) {
        if !self.finished {
            self.abort();
        }
    }
}

struct Counting<'a> {
    inner: &'a mut File,
    n: u64,
}

impl Write for Counting<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(windows)]
fn replace(target: &Path, swap: &Path) -> std::io::Result<()> {
    use sk_windows::raw::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_IGNORE_ACL_ERRORS, REPLACEFILE_IGNORE_MERGE_ERRORS,
    };
    use std::ptr::null;
    let (t, s) = (win_path(target), win_path(swap));
    if target.exists() {
        let ok = unsafe {
            ReplaceFileW(
                t.as_ptr(),
                s.as_ptr(),
                null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS | REPLACEFILE_IGNORE_ACL_ERRORS,
                null(),
                null(),
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(2) {
            return Err(e);
        }
    }
    if unsafe {
        MoveFileExW(
            s.as_ptr(),
            t.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace(target: &Path, swap: &Path) -> std::io::Result<()> {
    std::fs::rename(swap, target)
}

/// A path the wide Win32 calls take: `\\?\` in front of a long absolute one (std adds it itself, a direct call does not).
#[cfg(windows)]
fn win_path(p: &Path) -> Vec<u16> {
    let s = p.as_os_str().to_string_lossy();
    let s = if s.len() >= 248 && !s.starts_with(r"\\?\") {
        match s.strip_prefix(r"\\") {
            Some(unc) => format!(r"\\?\UNC\{unc}"),
            None => format!(r"\\?\{s}"),
        }
    } else {
        s.into_owned()
    };
    s.encode_utf16().chain(Some(0)).collect()
}

fn writables() -> &'static Mutex<HashMap<u64, Arc<Mutex<Writable>>>> {
    static W: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Writable>>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_WRITABLE: AtomicU64 = AtomicU64::new(1);

fn closed() -> Fail {
    Fail::new("InvalidStateError", super::INVALID_STATE)
}

pub fn start_open(file: u64, keep: bool) -> Result<u32, Fail> {
    let e = match granted(file) {
        Ok(e) => e,
        Err(f) => return Err(f),
    };
    if e.kind != Kind::File {
        return Err(Fail::new("TypeMismatchError", TYPE_MISMATCH));
    }
    let token = crate::next_token();
    pool::submit(token, move || {
        let w = Writable::open(&e.path, keep, true)?;
        let id = NEXT_WRITABLE.fetch_add(1, Ordering::Relaxed);
        writables()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, Arc::new(Mutex::new(w)));
        Ok(Done::json(&json!({ "w": id }).to_string()))
    });
    Ok(token)
}

fn get(w: u64) -> Option<Arc<Mutex<Writable>>> {
    writables()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&w)
        .cloned()
}

fn run_on(
    w: u64,
    op: impl FnOnce(&mut Writable) -> Result<(), Fail> + Send + 'static,
) -> Result<u32, Fail> {
    let Some(wr) = get(w) else {
        return Err(closed());
    };
    let token = crate::next_token();
    pool::submit(token, move || {
        op(&mut wr.lock().unwrap_or_else(|p| p.into_inner()))?;
        Ok(Done::json("{}"))
    });
    Ok(token)
}

pub fn start_write(w: u64, position: Option<u64>, data: Data) -> Result<u32, Fail> {
    run_on(w, move |wr| wr.write(position, &data))
}

pub fn start_seek(w: u64, position: u64) -> Result<u32, Fail> {
    run_on(w, move |wr| {
        wr.seek(position);
        Ok(())
    })
}

pub fn start_truncate(w: u64, size: u64) -> Result<u32, Fail> {
    run_on(w, move |wr| wr.truncate(size))
}

fn finish(w: u64, close: bool) -> Result<u32, Fail> {
    let Some(wr) = writables()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&w)
    else {
        return Err(closed());
    };
    let token = crate::next_token();
    pool::submit(token, move || {
        let mut guard = wr.lock().unwrap_or_else(|p| p.into_inner());
        let taken = std::mem::replace(
            &mut *guard,
            Writable {
                target: PathBuf::new(),
                swap: PathBuf::new(),
                file: None,
                cursor: 0,
                finished: true,
            },
        );
        drop(guard);
        if close {
            taken.close()?;
        } else {
            let mut t = taken;
            t.abort();
        }
        Ok(Done::json("{}"))
    });
    Ok(token)
}

pub fn start_close(w: u64) -> Result<u32, Fail> {
    finish(w, true)
}

pub fn start_abort(w: u64) -> Result<u32, Fail> {
    finish(w, false)
}

pub fn write_whole(target: &Path, data: &Data) -> Result<(), Fail> {
    let mut w = Writable::open(target, false, false)?;
    w.write(None, data)?;
    w.close()
}

#[cfg(test)]
mod tests {
    use super::super::tests::Scratch;
    use super::*;

    fn bytes(b: &[u8]) -> Data {
        Data::Bytes(b.to_vec())
    }

    #[test]
    fn writes_land_in_the_swap_and_close_replaces_atomically() {
        let s = Scratch::new("writable");
        let target = s.0.join("doc.txt");
        std::fs::write(&target, b"original").unwrap();
        let mut w = Writable::open(&target, false, true).unwrap();
        let swap = w.swap_path().to_path_buf();
        assert_eq!(swap.file_name().unwrap(), "doc.txt.vnswap");
        w.write(None, &bytes(b"hello")).unwrap();
        w.write(None, &bytes(b" world")).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        w.close().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello world");
        assert!(!swap.exists(), "the swap file is gone after close");
    }

    #[test]
    fn positions_seek_and_truncate_follow_the_spec() {
        let s = Scratch::new("positions");
        let target = s.0.join("p.bin");
        std::fs::write(&target, b"abcdef").unwrap();
        let mut w = Writable::open(&target, true, true).unwrap();
        w.write(None, &bytes(b"XY")).unwrap();
        w.write(Some(8), &bytes(b"Z")).unwrap();
        w.seek(1);
        w.write(None, &bytes(b"q")).unwrap();
        w.truncate(4).unwrap();
        w.write(None, &bytes(b"!")).unwrap(); // the cursor was 2, inside the new end
        w.truncate(6).unwrap();
        w.close().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"Xq!d\0\0");
    }

    #[test]
    fn abort_and_drop_leave_the_target_and_no_swap() {
        let s = Scratch::new("abort");
        let target = s.0.join("keep.txt");
        std::fs::write(&target, b"keep").unwrap();
        let mut w = Writable::open(&target, false, true).unwrap();
        let swap = w.swap_path().to_path_buf();
        w.write(None, &bytes(b"lost")).unwrap();
        w.abort();
        assert!(!swap.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        {
            let mut w = Writable::open(&target, false, true).unwrap();
            w.write(None, &bytes(b"dropped")).unwrap();
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert!(
            std::fs::read_dir(&s.0).unwrap().count() == 1,
            "only the target is left"
        );
    }

    #[test]
    fn a_crash_orphan_is_swept_and_a_live_swap_is_not() {
        let s = Scratch::new("orphans");
        let target = s.0.join("t.txt");
        std::fs::write(&target, b"t").unwrap();
        std::fs::write(s.0.join("t.txt.vnswap"), b"crashed").unwrap();
        std::fs::write(s.0.join("t.txt.vnswap-3"), b"crashed").unwrap();
        std::fs::write(s.0.join("t.txt.vnswap-x"), b"not ours").unwrap();
        std::fs::write(s.0.join("other.txt.vnswap"), b"another file's").unwrap();
        let live = Writable::open(&target, false, true).unwrap();
        assert_eq!(
            live.swap_path().file_name().unwrap(),
            "t.txt.vnswap",
            "the orphan's name was freed"
        );
        assert!(!s.0.join("t.txt.vnswap-3").exists());
        assert!(
            s.0.join("t.txt.vnswap-x").exists(),
            "only our exact pattern is swept"
        );
        assert!(
            s.0.join("other.txt.vnswap").exists(),
            "another target's swap is not ours to sweep"
        );
        let second = Writable::open(&target, false, true).unwrap();
        assert_eq!(second.swap_path().file_name().unwrap(), "t.txt.vnswap-1");
        assert!(
            live.swap_path().exists(),
            "a live swap survives the second open's sweep"
        );
        drop(second);
        drop(live);
    }

    #[test]
    fn a_missing_target_is_not_found_unless_it_is_a_download() {
        let s = Scratch::new("missing");
        let target = s.0.join("new.txt");
        assert_eq!(
            Writable::open(&target, false, true).err().map(|f| f.name),
            Some("NotFoundError")
        );
        assert_eq!(
            Writable::open(&s.0, false, true).err().map(|f| f.name),
            Some("TypeMismatchError")
        );
        write_whole(&target, &bytes(b"downloaded")).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"downloaded");
        write_whole(&target, &bytes(b"again")).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"again");
    }

    #[test]
    fn a_blob_range_is_copied_and_its_snapshot_is_honoured() {
        use sk_runtime::redjs::webapi::blob::{DiskFile, Range, Source};
        use std::sync::Arc;
        let s = Scratch::new("blobcopy");
        let src = s.0.join("src.bin");
        std::fs::write(&src, b"0123456789").unwrap();
        let m = std::fs::metadata(&src).unwrap();
        let disk = Arc::new(DiskFile {
            path: src.clone(),
            size: m.len(),
            mtime: m.modified().ok(),
        });
        let target = s.0.join("dst.bin");
        std::fs::write(&target, b"").unwrap();
        let mut w = Writable::open(&target, false, true).unwrap();
        w.write(
            None,
            &Data::Blob(Range {
                source: Source::Disk(disk.clone()),
                offset: 3,
                len: 4,
            }),
        )
        .unwrap();
        w.write(None, &bytes(b"!")).unwrap();
        w.close().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"3456!");
        std::fs::write(&src, b"changed").unwrap();
        let mut w = Writable::open(&target, false, true).unwrap();
        let e = w
            .write(
                None,
                &Data::Blob(Range {
                    source: Source::Disk(disk),
                    offset: 0,
                    len: 2,
                }),
            )
            .unwrap_err();
        assert_eq!(e.name, "NotReadableError");
    }

    #[test]
    fn swap_names_match_exactly() {
        assert!(is_swap_of("a.txt.vnswap", "a.txt"));
        assert!(is_swap_of("a.txt.vnswap-12", "a.txt"));
        assert!(!is_swap_of("a.txt.vnswap-", "a.txt"));
        assert!(!is_swap_of("a.txt.vnswap-1x", "a.txt"));
        assert!(!is_swap_of("ab.txt.vnswap", "a.txt"));
        assert!(!is_swap_of("a.txt", "a.txt"));
    }
}
