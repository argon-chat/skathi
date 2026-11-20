//! Chromium's DownloadURL is <mime>:<suggested name>:<url>. Resolve only
//! sources available to this document. Native targets request bytes by range.
use gpui::PlatformDragFile;
use sk_runtime::redjs::webapi::{
    blob::{DiskFile, Range, Source},
    url::{resolve_blob_url, BlobTarget},
};
use std::{
    ffi::c_void,
    io,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

pub(super) fn range_file(
    name: String,
    range: Range,
    path: Option<PathBuf>,
    directory: bool,
) -> PlatformDragFile {
    let size = range.len;
    PlatformDragFile {
        name,
        path,
        directory,
        size,
        read: Arc::new(move |at, out| {
            if at >= range.len {
                return Ok(0);
            }
            let mut part = range.clone();
            part.offset += at;
            part.len = (range.len - at).min(out.len() as u64);
            let n = part.len as usize;
            if let Some(bytes) = part.in_memory() {
                out[..n].copy_from_slice(bytes);
            } else {
                let bytes = part
                    .read()
                    .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
                out[..n].copy_from_slice(&bytes);
            }
            Ok(n)
        }),
    }
}

fn copy_slice(source: &[u8], at: u64, out: &mut [u8]) -> usize {
    let bytes = usize::try_from(at)
        .ok()
        .and_then(|at| source.get(at..))
        .unwrap_or_default();
    let n = bytes.len().min(out.len());
    out[..n].copy_from_slice(&bytes[..n]);
    n
}

pub(super) fn promised_download(g: *mut c_void, value: &str) -> Option<PlatformDragFile> {
    let (_, rest) = value.split_once(':')?;
    let (name, url) = rest.split_once(':')?;
    // A promise always names a single leaf, also on file managers that use
    // the name verbatim. The host adapter remains responsible for reserved names.
    let name = name.replace(['/', '\\', ':', '\0'], "_");
    if name.trim_matches([' ', '.']).is_empty() {
        return None;
    }
    if url.starts_with("blob:") {
        let range = match resolve_blob_url(g, url)? {
            BlobTarget::Bytes {
                bytes, offset, len, ..
            } => Range {
                source: Source::Memory(bytes),
                offset: offset as u64,
                len: len as u64,
            },
            BlobTarget::Path {
                path,
                range,
                size,
                mtime,
                ..
            } => {
                let (offset, len) = range.unwrap_or((0, size));
                Range {
                    source: Source::Disk(Arc::new(DiskFile { path, size, mtime })),
                    offset,
                    len,
                }
            }
        };
        return Some(range_file(name, range, None, false));
    }
    if let Some(body) = url.strip_prefix("data:") {
        let (meta, payload) = body.split_once(',')?;
        let is_base64 = meta
            .split(';')
            .any(|part| part.trim().eq_ignore_ascii_case("base64"));
        let encoded = if is_base64 && payload.contains('%') {
            String::from_utf8(crate::img::percent_decode_bytes(payload)).ok()?
        } else {
            payload.to_owned()
        };
        let bytes = encoded.as_bytes();
        let size = if is_base64 {
            let alphabet_length = bytes
                .iter()
                .copied()
                .take_while(|b| *b != b'=')
                .filter(|b| !b.is_ascii_whitespace())
                .count();
            alphabet_length as u64 * 6 / 8
        } else {
            let (mut n, mut cursor) = (0, 0);
            while cursor < bytes.len() {
                let escaped = bytes[cursor] == b'%'
                    && bytes
                        .get(cursor + 1..cursor + 3)
                        .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit));
                cursor += if escaped { 3 } else { 1 };
                n += 1;
            }
            n
        };
        let decoded: OnceLock<Option<Vec<u8>>> = OnceLock::new();
        return Some(PlatformDragFile {
            name,
            path: None,
            directory: false,
            size,
            read: Arc::new(move |at, out| {
                let data = decoded.get_or_init(|| {
                    if is_base64 {
                        crate::img::base64_decode(&encoded)
                    } else {
                        Some(crate::img::percent_decode_bytes(&encoded))
                    }
                });
                Ok(copy_slice(
                    data.as_deref().ok_or(io::ErrorKind::InvalidData)?,
                    at,
                    out,
                ))
            }),
        });
    }
    let url = match url.strip_prefix("app://redjs/") {
        Some(path) => path,
        None if !url.split('/').next().unwrap_or_default().contains(':')
            && !url.starts_with("//") =>
        {
            url
        }
        None => return None,
    };
    let key = crate::img::content_key(url)?;
    if let Some(store) = crate::img::asset_store() {
        let bytes = store(&key)?;
        return Some(PlatformDragFile {
            name,
            path: None,
            directory: false,
            size: bytes.len() as u64,
            read: Arc::new(move |at, out| Ok(copy_slice(bytes, at, out))),
        });
    }
    let path = crate::img::dev_asset_file(url).or_else(|| {
        let root = crate::img::asset_base()?;
        [root.join(&key), root.join("public").join(&key)]
            .into_iter()
            .find(|path| path.is_file())
    })?;
    let meta = path.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let disk = DiskFile {
        path,
        size,
        mtime: meta.modified().ok(),
    };
    Some(range_file(
        name,
        Range {
            source: Source::Disk(Arc::new(disk)),
            offset: 0,
            len: size,
        },
        None,
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_download_names_ranges_and_non_app_schemes() {
        for spec in [
            "text/plain:drawing.txt:data:text/plain,a%20b",
            "text/plain:drawing.txt:data:text/plain;base64,YSBi",
        ] {
            let file = promised_download(std::ptr::null_mut(), spec).unwrap();
            assert_eq!(file.name, "drawing.txt");
            assert_eq!(file.size, 3);
            let mut out = [0; 8];
            assert_eq!((file.read)(1, &mut out).unwrap(), 2);
            assert_eq!(&out[..2], b" b");
        }
        let escaped =
            promised_download(std::ptr::null_mut(), "text/plain:name:data:;base64,YWI%3D").unwrap();
        let mut out = Vec::new();
        escaped.write_to(&mut out).unwrap();
        assert_eq!(out, b"ab");
        assert!(
            promised_download(std::ptr::null_mut(), "text/plain:x:https://example.com/x").is_none()
        );
        assert!(promised_download(std::ptr::null_mut(), "text/plain:x:file:///secret").is_none());
    }
}
