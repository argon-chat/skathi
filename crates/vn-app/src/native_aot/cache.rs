//! Only an inspector child expands these application-owned executable bytes.
//! The path is never trusted: the exact opened file is hashed while denying
//! writes and its handle is handed directly to the native loader.
use super::{aot, open_image};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

fn digest_is_valid(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
fn root() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("VN_AOT_CACHE_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("VN_AOT_CACHE_DIR must be absolute".into());
        }
        return Ok(path);
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or("LOCALAPPDATA is required for the native inspector cache")?;
    let path = PathBuf::from(local);
    if !path.is_absolute() {
        return Err("LOCALAPPDATA must be absolute".into());
    }
    Ok(path.join("vue-native").join("native-aot").join("v1"))
}
fn verified(path: &Path, expected: &str, size: u64) -> Result<File, String> {
    let mut file = open_image(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if file.metadata().map_err(|error| error.to_string())?.len() != size {
        return Err("cached native image size differs".into());
    }
    if aot::digest_reader(&mut file).map_err(|error| error.to_string())? != expected {
        return Err("cached native image digest differs".into());
    }
    file.rewind().map_err(|error| error.to_string())?;
    Ok(file)
}
struct Temporary {
    path: PathBuf,
    file: Option<File>,
    published: bool,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn create_temporary(directory: &Path) -> Result<Temporary, String> {
    for _ in 0..32 {
        let path = directory.join(format!(
            ".inspector-{}-{}.part",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                return Ok(Temporary {
                    path,
                    file: Some(file),
                    published: false,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create native image staging file: {error}")),
        }
    }
    Err("native image staging names are exhausted".into())
}
fn materialize_in(
    directory: &Path,
    compressed: &[u8],
    expected: &str,
    size: u64,
) -> Result<File, String> {
    if !digest_is_valid(expected) || size == 0 || size > MAX_IMAGE_BYTES {
        return Err("invalid embedded native image digest or size".into());
    }
    let directory = directory.join(expected);
    let destination = directory.join("inspector.image");
    if let Ok(file) = verified(&destination, expected, size) {
        return Ok(file);
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create native image cache: {error}"))?;
    let mut temporary = create_temporary(&directory)?;
    let mut decoder = flate2::read::DeflateDecoder::new(compressed).take(size + 1);
    let mut writer = temporary
        .file
        .take()
        .ok_or("native image staging file is missing")?;
    let copied = std::io::copy(&mut decoder, &mut writer)
        .map_err(|error| format!("cannot expand native image: {error}"));
    let prepared = copied.and_then(|copied| {
        if copied != size {
            return Err("expanded native image has an unexpected size".into());
        }
        writer.flush().map_err(|error| error.to_string())?;
        writer.sync_all().map_err(|error| error.to_string())
    });
    drop(writer);
    prepared?;
    // Check the staging file before publication, then check the exact opened
    // published handle too. A racing correct publisher is safe to reuse.
    drop(verified(&temporary.path, expected, size)?);
    if let Err(error) = std::fs::rename(&temporary.path, &destination) {
        if let Ok(file) = verified(&destination, expected, size) {
            return Ok(file);
        }
        return Err(format!("cannot atomically publish native image: {error}"));
    }
    temporary.published = true;
    verified(&destination, expected, size)
}

pub(super) fn materialize(compressed: &[u8], expected: &str, size: u64) -> Result<File, String> {
    materialize_in(&root()?, compressed, expected, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vn-native-cache-{}-{}",
                std::process::id(),
                NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn published_temporary_guard_does_not_remove_a_reused_staging_path() {
        let root = Directory::new();
        let mut staging = create_temporary(&root.0).unwrap();
        drop(staging.file.take());
        let original = staging.path.clone();
        std::fs::rename(&original, root.0.join("published")).unwrap();
        staging.published = true;
        std::fs::write(&original, b"later unrelated file").unwrap();
        drop(staging);
        assert_eq!(std::fs::read(original).unwrap(), b"later unrelated file");
    }
    fn packed(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
    #[test]
    fn cold_then_warm_cache_verifies_exact_file_without_reinflating() {
        let root = Directory::new();
        let bytes = b"trusted native image";
        let digest = aot::digest_hex(bytes);
        let mut file =
            materialize_in(&root.0, &packed(bytes), &digest, bytes.len() as u64).unwrap();
        assert_eq!(aot::digest_reader(&mut file).unwrap(), digest);
        drop(file);
        // The warm path must not read the compressed bytes.
        let mut warm = materialize_in(
            &root.0,
            b"broken compressed bytes",
            &digest,
            bytes.len() as u64,
        )
        .unwrap();
        assert_eq!(aot::digest_reader(&mut warm).unwrap(), digest);
    }
    #[test]
    fn corrupted_cache_is_replaced_and_bad_expansion_is_never_published() {
        let root = Directory::new();
        let bytes = b"trusted";
        let digest = aot::digest_hex(bytes);
        let directory = root.0.join(&digest);
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("inspector.image");
        std::fs::write(&path, b"changed").unwrap();
        assert!(materialize_in(&root.0, &packed(b"tainted"), &digest, bytes.len() as u64).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"changed");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        drop(materialize_in(&root.0, &packed(bytes), &digest, bytes.len() as u64).unwrap());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn bounded_expansion_and_digest_reject_before_publication() {
        let root = Directory::new();
        let bytes = b"too much output";
        let digest = aot::digest_hex(bytes);
        assert!(materialize_in(&root.0, &packed(bytes), &digest, 1).is_err());
        assert!(materialize_in(&root.0, &packed(bytes), "../outside", 1).is_err());
        assert!(materialize_in(&root.0, &packed(bytes), &digest, MAX_IMAGE_BYTES + 1).is_err());
        assert!(!root.0.join(digest).join("inspector.image").exists());
    }
    #[test]
    fn concurrent_publishers_both_return_matching_immutable_handles() {
        let root = Directory::new();
        let bytes = b"simultaneously published native image";
        let digest = aot::digest_hex(bytes);
        let compressed = packed(bytes);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let mut file =
                            materialize_in(&root.0, &compressed, &digest, bytes.len() as u64)
                                .unwrap();
                        assert_eq!(aot::digest_reader(&mut file).unwrap(), digest);
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
        });
        assert_eq!(std::fs::read_dir(root.0.join(digest)).unwrap().count(), 1);
    }
}
