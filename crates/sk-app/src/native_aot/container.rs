//! Native images in shared-library sections, or in the Windows executable with `publishSingleFile`.
use std::path::PathBuf;

#[cfg(any(windows, test))]
const PAGE: usize = 4096;
/// The first of the image's sections: its header and tables. Code and the rest follow.
pub(super) const APP_SECTION: &[u8] = b".skapp";
pub(super) const INSPECTOR_SECTION: &[u8] = b".skins";

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
}

#[cfg(any(windows, test))]
fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
#[cfg(any(windows, test))]
fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
#[cfg(any(windows, test))]
fn image_size(header: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(header.get(32..40)?.try_into().ok()?))
}

/// Where the section table starts, its length, the size of headers and the size of image.
#[cfg(any(windows, test))]
fn table(headers: &[u8]) -> Option<(usize, usize, usize, usize)> {
    if headers.get(..2)? != b"MZ" {
        return None;
    }
    let pe = le32(headers, 0x3c)? as usize;
    if headers.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let count = le16(headers, pe + 6)? as usize;
    let optional = pe + 24;
    Some((
        optional + le16(headers, pe + 20)? as usize,
        count,
        le32(headers, optional + 60)? as usize,
        le32(headers, optional + 56)? as usize,
    ))
}

#[cfg(any(windows, test))]
fn section(headers: &[u8], name: &[u8]) -> Option<Section> {
    let (start, count, _, _) = table(headers)?;
    (0..count).find_map(|index| {
        let at = start + index * 40;
        let raw = headers.get(at..at + 8)?;
        let length = raw.iter().position(|&b| b == 0).unwrap_or(8);
        (&raw[..length] == name).then_some(())?;
        Some(Section {
            virtual_size: le32(headers, at + 8)?,
            virtual_address: le32(headers, at + 12)?,
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

/// The code location for an AOT container, with the bundle's other dylibs on macOS.
pub(super) fn beside_executable_image() -> Option<PathBuf> {
    let dir = beside_executable()?;
    #[cfg(target_os = "macos")]
    if let Some(contents) = dir.parent().filter(|c| c.ends_with("Contents")) {
        if dir.ends_with("Resources") {
            return Some(contents.join("Frameworks"));
        }
    }
    Some(dir)
}

/// The image whose first section is `name` in the module mapped at `base`.
///
/// # Safety
/// `base` is a module the loader mapped, that stays mapped for the life of the process.
#[cfg(any(windows, test))]
unsafe fn mapped(base: *const u8, name: &[u8]) -> Option<&'static [u8]> {
    // SAFETY: a mapped module's headers fill at least its first page.
    let first = unsafe { std::slice::from_raw_parts(base, PAGE) };
    let (_, _, size_of_headers, size_of_image) = table(first)?;
    if size_of_headers > size_of_image || size_of_headers > PAGE * 16 {
        return None;
    }
    // SAFETY: SizeOfHeaders bytes of a mapped module are its headers.
    let headers = unsafe { std::slice::from_raw_parts(base, size_of_headers) };
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
    // SAFETY: [start, start + size) lies inside the mapped module.
    Some(unsafe { std::slice::from_raw_parts(base.add(start), size) })
}

/// The image the Windows loader mapped from the sections of this executable, if it has them.
#[cfg(windows)]
pub(super) fn in_executable(name: &[u8]) -> Option<&'static [u8]> {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let base = unsafe { GetModuleHandleW(None) }.ok()?.0 as *const u8;
    // SAFETY: the main module stays mapped for the life of the process.
    unsafe { mapped(base, name) }
}

#[cfg(not(windows))]
pub(super) fn in_executable(_: &[u8]) -> Option<&'static [u8]> {
    None
}

/// The image in the container DLL at `path`, loaded by the Windows loader like any DLL (so code
/// integrity policy, signature checks and the sections' protections apply) and never unloaded.
/// The container has no code of its own: no entry point, no imports.
#[cfg(windows)]
pub(super) fn load(path: &std::path::Path, name: &[u8]) -> std::io::Result<&'static [u8]> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let module =
        unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|error| match error.code().0 as u32 & 0xffff {
                2 | 3 | 126 => std::io::Error::from(std::io::ErrorKind::NotFound),
                193 => std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "it is not a DLL Windows can load",
                ),
                _ => {
                    std::io::Error::other(error.message().trim_end_matches(['.', ' ']).to_string())
                }
            })?;
    // SAFETY: the module is never freed.
    unsafe { mapped(module.0 as *const u8, name) }.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "it holds no native image")
    })
}

#[cfg(target_os = "macos")]
pub(super) fn load(path: &std::path::Path, _: &[u8]) -> std::io::Result<&'static [u8]> {
    // SAFETY: This path is part of the trusted app bundle or host workload.
    unsafe { sk_runtime::redjs::aot::map_macho_image(path) }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(super) fn load(_: &std::path::Path, _: &[u8]) -> std::io::Result<&'static [u8]> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A module as the loader lays it out: headers, then one section at 0x1000 holding an image
    /// whose header says `claimed` bytes, in a module of `size_of_image` bytes.
    fn module(claimed: u64, size_of_image: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; size_of_image.max(0x2000) as usize];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        let optional = 0x80 + 24;
        bytes[optional + 56..optional + 60].copy_from_slice(&size_of_image.to_le_bytes());
        bytes[optional + 60..optional + 64].copy_from_slice(&0x400u32.to_le_bytes());
        let at = optional + 240;
        bytes[at..at + 6].copy_from_slice(APP_SECTION);
        bytes[at + 8..at + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[0x1000 + 32..0x1000 + 40].copy_from_slice(&claimed.to_le_bytes());
        bytes
    }
    #[test]
    fn an_image_is_found_by_its_section_and_must_lie_inside_the_module() {
        let good = module(0x1000, 0x2000);
        let found = unsafe { mapped(good.as_ptr(), APP_SECTION) }.unwrap();
        assert_eq!(found.as_ptr(), good[0x1000..].as_ptr());
        assert_eq!(found.len(), 0x1000);
        assert!(unsafe { mapped(good.as_ptr(), INSPECTOR_SECTION) }.is_none());
        let long = module(0x2000, 0x2000);
        assert!(unsafe { mapped(long.as_ptr(), APP_SECTION) }.is_none());
    }
    #[cfg(windows)]
    #[test]
    fn a_missing_container_is_not_found() {
        let path = std::env::temp_dir().join("sk-container-that-is-not-there.dll");
        assert_eq!(
            load(&path, APP_SECTION).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
