//! Read-only, demand-paged application files.
//!
//! Keep the source handle for the whole mapping lifetime. On Windows readers
//! may coexist and a publisher may atomically rename/replace the path, but
//! in-place writers/truncation are refused by the sharing mode. Existing
//! mappings continue to refer to the original file after a replacement.
//!
//! On POSIX, publishers must likewise replace the file atomically: the OS does
//! not enforce Windows-style sharing, and an active mapped file must never be
//! modified or truncated in place.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Owns both the immutable view and the file it maps. No writable mapping or
/// mutable byte slice is exposed. Empty files have an empty view, so callers
/// can report their usual format errors rather than an OS-specific mmap error.
#[derive(Debug)]
pub(crate) struct MappedFile {
    // Fields drop in declaration order: unmap before releasing the source.
    view: Option<memmap2::Mmap>,
    _file: File,
}

impl MappedFile {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Self::open_bounded(path, isize::MAX as usize)
    }

    /// Bounds the mapped address range before creating the view. Metadata is
    /// queried on the opened handle, never through a second path lookup.
    pub(crate) fn open_bounded(path: &Path, max_len: usize) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_SHARE_READ | FILE_SHARE_DELETE. Denying FILE_SHARE_WRITE
            // also refuses an already-open writable handle, before any parse.
            options.share_mode(0x0000_0001 | 0x0000_0004);
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a mapped application file must be a regular file",
            ));
        }
        let limit = max_len.min(isize::MAX as usize) as u64;
        if metadata.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "application file is {} bytes; the mapping limit is {limit}",
                    metadata.len()
                ),
            ));
        }
        let len = metadata.len() as usize;
        let view = if len == 0 {
            None
        } else {
            // SAFETY: the handle is read-only and retained until after unmap.
            // Windows sharing excludes concurrent writers. POSIX publishers
            // must keep active application files immutable (see module docs).
            Some(unsafe { memmap2::MmapOptions::new().len(len).map(&file)? })
        };
        Ok(Self { view, _file: file })
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.view.as_deref().unwrap_or(&[])
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.as_bytes().len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct Files {
        root: PathBuf,
        names: Vec<&'static str>,
    }

    impl Files {
        pub(crate) fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "vn-mapped-file-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self {
                root,
                names: Vec::new(),
            }
        }

        pub(crate) fn path(&mut self, name: &'static str) -> PathBuf {
            if !self.names.contains(&name) {
                self.names.push(name);
            }
            self.root.join(name)
        }

        pub(crate) fn write(&mut self, name: &'static str, bytes: &[u8]) -> PathBuf {
            let path = self.path(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            for name in &self.names {
                let _ = std::fs::remove_file(self.root.join(name));
            }
            // Only direct owned files exist; never recursively remove a path.
            let _ = std::fs::remove_dir(&self.root);
        }
    }

    #[test]
    fn mapped_file_survives_owner_moves_and_exposes_only_its_file_bytes() {
        let mut files = Files::new();
        let path = files.write("app.vnpk", b"mapped bytes");
        let mapped = MappedFile::open(&path).unwrap();
        let pointer = mapped.as_bytes().as_ptr();
        let moved = Box::new(mapped);
        assert_eq!(moved.len(), 12);
        assert_eq!(moved.as_bytes(), b"mapped bytes");
        assert_eq!(moved.as_bytes().as_ptr(), pointer);
        assert_eq!(std::fs::read(&path).unwrap(), b"mapped bytes");
    }

    #[test]
    fn mapped_file_empty_and_bounds_failures_leave_no_open_handle() {
        let mut files = Files::new();
        let path = files.write("empty.vnpk", b"");
        let empty = MappedFile::open(&path).unwrap();
        assert_eq!(empty.len(), 0);
        assert!(empty.as_bytes().is_empty());
        drop(empty);
        std::fs::write(&path, b"1234").unwrap();
        assert_eq!(
            MappedFile::open_bounded(&path, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::write(&path, b"still writable").unwrap();
        assert!(MappedFile::open(&files.root).is_err());
        assert_eq!(
            MappedFile::open(&files.path("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn mapped_file_denies_writers_and_truncation_until_drop() {
        let mut files = Files::new();
        let path = files.write("app.vnpk", b"original");
        let mapped = MappedFile::open(&path).unwrap();
        assert!(File::open(&path).is_ok());
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert!(OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .is_err());
        assert_eq!(mapped.as_bytes(), b"original");
        drop(mapped);
        std::fs::write(&path, b"replacement").unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn mapped_file_refuses_an_already_open_writer_without_leaking_a_handle() {
        use std::os::windows::fs::OpenOptionsExt;
        let mut files = Files::new();
        let path = files.write("app.vnpk", b"original");
        let writer = OpenOptions::new()
            .write(true)
            .share_mode(0x7)
            .open(&path)
            .unwrap();
        assert!(MappedFile::open(&path).is_err());
        drop(writer);
        assert_eq!(MappedFile::open(&path).unwrap().as_bytes(), b"original");
        std::fs::write(&path, b"released").unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn mapped_file_atomic_replacement_keeps_the_original_view_alive() {
        let mut files = Files::new();
        let path = files.write("app.vnpk", b"original");
        let old = files.path("app.previous.vnpk");
        let replacement = files.write("app.next.vnpk", b"replacement");
        let mapped = MappedFile::open(&path).unwrap();
        std::fs::rename(&path, &old).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(mapped.as_bytes(), b"original");
        assert_eq!(MappedFile::open(&path).unwrap().as_bytes(), b"replacement");
        assert!(OpenOptions::new().write(true).open(&old).is_err());
        drop(mapped);
        std::fs::write(&old, b"released").unwrap();
    }
}
