//! A native image stored as PE sections that `sk build` appends: to a container DLL beside the
//! app, or to the running executable itself (`"publishSingleFile"`). The image is never copied
//! or expanded; the engine maps it where it lies.
use super::cache::MAX_IMAGE_BYTES;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const PAGE: u64 = 4096;
const MAX_HEADERS: u64 = 64 * 1024;
/// The first of the image's sections: its header and tables. Code and the rest follow.
pub(super) const APP_SECTION: &[u8] = b".skapp";
pub(super) const INSPECTOR_SECTION: &[u8] = b".skins";

#[derive(Debug, PartialEq, Eq)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
fn image_size(header: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(header.get(32..40)?.try_into().ok()?))
}

/// Where the optional header ends and the section table starts, and its size of headers.
fn table(headers: &[u8]) -> Option<(usize, usize, u32)> {
    if headers.get(..2)? != b"MZ" {
        return None;
    }
    let pe = le32(headers, 0x3c)? as usize;
    if headers.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let count = le16(headers, pe + 6)? as usize;
    let optional = pe + 24;
    let size_of_headers = le32(headers, optional + 60)?;
    Some((
        optional + le16(headers, pe + 20)? as usize,
        count,
        size_of_headers,
    ))
}

fn section(headers: &[u8], name: &[u8]) -> Option<Section> {
    let (start, count, _) = table(headers)?;
    (0..count).find_map(|index| {
        let at = start + index * 40;
        let raw = headers.get(at..at + 8)?;
        let length = raw.iter().position(|&b| b == 0).unwrap_or(8);
        (&raw[..length] == name).then_some(())?;
        Some(Section {
            virtual_size: le32(headers, at + 8)?,
            virtual_address: le32(headers, at + 12)?,
            raw_offset: le32(headers, at + 20)?,
        })
    })
}

/// Where a container beside the app lives: beside the executable, or in the Resources of the
/// macOS bundle that holds it.
pub(super) fn beside_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    #[cfg(target_os = "macos")]
    if dir.file_name().is_some_and(|n| n == "MacOS") {
        if let Some(contents) = dir
            .parent()
            .filter(|c| c.file_name().is_some_and(|n| n == "Contents"))
        {
            return Some(contents.join("Resources"));
        }
    }
    Some(dir.to_path_buf())
}

/// The image in the container at `path`: the file, opened so that nothing can write it while
/// it is in use, and the image's offset and length in it.
pub(super) fn open(path: &Path, name: &[u8]) -> Result<(File, u64, u64), std::io::Error> {
    let invalid =
        |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string());
    let mut file = super::open_image(path)?;
    let length = file.metadata()?.len();
    let mut headers = vec![0; length.min(MAX_HEADERS) as usize];
    file.read_exact(&mut headers)?;
    let found = section(&headers, name).ok_or_else(|| invalid("it holds no native image"))?;
    let offset = found.raw_offset as u64;
    let mut header = [0; 40];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut header)?;
    let size = image_size(&header).unwrap_or(0);
    if offset % PAGE != 0
        || size == 0
        || size % PAGE != 0
        || size > MAX_IMAGE_BYTES
        || size > length.saturating_sub(offset)
    {
        return Err(invalid("its native image is cut short or misplaced"));
    }
    Ok((file, offset, size))
}

/// The image the Windows loader mapped from the sections of this executable, if it has them.
#[cfg(windows)]
pub(super) fn in_executable(name: &[u8]) -> Option<&'static [u8]> {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let base = unsafe { GetModuleHandleW(None) }.ok()?.0 as *const u8;
    // SAFETY: the main module stays mapped for the life of the process, and its headers fill at
    // least its first page.
    let first = unsafe { std::slice::from_raw_parts(base, PAGE as usize) };
    let (_, _, size_of_headers) = table(first)?;
    let pe = le32(first, 0x3c)? as usize;
    let size_of_image = le32(first, pe + 24 + 56)? as usize;
    if size_of_headers as usize > size_of_image {
        return None;
    }
    // SAFETY: as above; SizeOfHeaders bytes of a mapped module are its headers.
    let headers = unsafe { std::slice::from_raw_parts(base, size_of_headers as usize) };
    let found = section(headers, name)?;
    let start = found.virtual_address as usize;
    if found.virtual_size < 40 || start >= size_of_image {
        return None;
    }
    // SAFETY: the section is mapped and at least 40 bytes long.
    let header = unsafe { std::slice::from_raw_parts(base.add(start), 40) };
    let size = image_size(header)? as usize;
    if size == 0 || size > size_of_image - start {
        return None;
    }
    // SAFETY: [start, start + size) lies inside the mapped module, which is never unloaded.
    Some(unsafe { std::slice::from_raw_parts(base.add(start), size) })
}

#[cfg(not(windows))]
pub(super) fn in_executable(_: &[u8]) -> Option<&'static [u8]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn container(offset: u32, size: u64) -> Vec<u8> {
        let mut bytes = vec![0u8; offset as usize + size.max(PAGE) as usize];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        let table = 0x80 + 24 + 240;
        bytes[table..table + 6].copy_from_slice(b".rdata");
        let at = table + 40;
        bytes[at..at + 6].copy_from_slice(APP_SECTION);
        bytes[at + 8..at + 12].copy_from_slice(&(size as u32).to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&0x2000u32.to_le_bytes());
        bytes[at + 20..at + 24].copy_from_slice(&offset.to_le_bytes());
        let image = offset as usize;
        bytes[image + 32..image + 40].copy_from_slice(&size.to_le_bytes());
        bytes
    }
    fn written(bytes: &[u8]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "sk-container-{}-{}.dll",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }
    #[test]
    fn the_image_is_found_by_its_section_and_must_lie_on_pages_inside_the_file() {
        let good = written(&container(0x1000, 2 * PAGE));
        let (_, offset, size) = open(&good, APP_SECTION).unwrap();
        assert_eq!((offset, size), (0x1000, 2 * PAGE));
        assert!(open(&good, INSPECTOR_SECTION).is_err());
        let misplaced = written(&container(0x1200, PAGE));
        let mut long = container(0x1000, PAGE);
        long[0x1000 + 32..0x1000 + 40].copy_from_slice(&(4 * PAGE).to_le_bytes());
        let long = written(&long);
        for path in [&misplaced, &long] {
            assert!(open(path, APP_SECTION).is_err());
        }
        for path in [good, misplaced, long] {
            let _ = std::fs::remove_file(path);
        }
    }
}
