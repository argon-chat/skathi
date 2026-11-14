//! A `ClipboardItem`'s representations on the Windows clipboard, in the formats Chrome uses for them.
//!
//! | MIME type    | written as                                         | read from                     |
//! |--------------|----------------------------------------------------|-------------------------------|
//! | `text/plain` | `CF_UNICODETEXT`                                   | `CF_UNICODETEXT`              |
//! | `text/html`  | `HTML Format` (CF_HTML: the fragment in a page)    | its fragment                  |
//! | `image/png`  | `PNG` (the bytes as given) + `CF_DIBV5` + `CF_DIB` | `PNG`, else a bitmap as a PNG |
//!
//! Most apps paste only the bitmaps; the PNG keeps alpha and the exact bytes. Both bitmaps are 32-bit BGRA,
//! straight alpha, bottom-up. Every representation is prepared before the clipboard opens, so a PNG that
//! does not decode leaves it untouched.

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

pub(crate) const TYPES: &[&str] = &["text/plain", "text/html", "image/png"];

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_DIBV5: u32 = 17;

fn png_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

fn html_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("HTML Format")) }
}

struct Open;

impl Open {
    // Another process (a clipboard manager) may hold it briefly: retry.
    fn new() -> Result<Self, String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(format!(
            "the clipboard is held by another process ({})",
            std::io::Error::last_os_error()
        ))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

pub(crate) fn write(items: &[(String, Vec<u8>)]) -> Result<(), String> {
    let mut formats: Vec<(u32, Vec<u8>)> = Vec::new();
    for (ty, bytes) in items {
        match ty.as_str() {
            "text/plain" => formats.push((CF_UNICODETEXT, utf16z(&String::from_utf8_lossy(bytes)))),
            "text/html" => formats.push((html_format(), cf_html(&String::from_utf8_lossy(bytes)))),
            "image/png" => {
                let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                    .map_err(|e| format!("the image/png data does not decode: {e}"))?
                    .to_rgba8();
                let (w, h) = rgba.dimensions();
                formats.push((png_format(), bytes.clone()));
                formats.push((CF_DIBV5, dib(w, h, rgba.as_raw(), true)));
                formats.push((CF_DIB, dib(w, h, rgba.as_raw(), false)));
            }
            other => return Err(format!("{other} is not a type this clipboard writes")),
        }
    }
    let _open = Open::new()?;
    unsafe { EmptyClipboard() }.map_err(|e| format!("the clipboard could not be emptied: {e}"))?;
    for (format, data) in formats {
        set(format, &data)?;
    }
    Ok(())
}

pub(crate) fn read() -> Result<Vec<(String, Vec<u8>)>, String> {
    let _open = Open::new()?;
    let mut out = Vec::new();
    let png = get(png_format()).or_else(|| {
        get(CF_DIBV5)
            .or_else(|| get(CF_DIB))
            .and_then(|d| dib_to_png(&d))
    });
    if let Some(png) = png {
        out.push(("image/png".to_string(), png));
    }
    if let Some(fragment) = get(html_format()).and_then(|h| html_fragment(&h)) {
        out.push(("text/html".to_string(), fragment.into_bytes()));
    }
    if let Some(text) = get(CF_UNICODETEXT)
        .map(|b| from_utf16z(&b))
        .filter(|t| !t.is_empty())
    {
        out.push(("text/plain".to_string(), text.into_bytes()));
    }
    Ok(out)
}

fn set(format: u32, data: &[u8]) -> Result<(), String> {
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1))
            .map_err(|e| format!("out of memory for the clipboard: {e}"))?;
        let ptr = GlobalLock(global);
        if ptr.is_null() {
            let _ = GlobalFree(Some(global));
            return Err("the clipboard memory could not be locked".into());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
        let _ = GlobalUnlock(global);
        if let Err(e) = SetClipboardData(format, Some(HANDLE(global.0))) {
            let _ = GlobalFree(Some(global));
            return Err(format!("the clipboard refused format {format}: {e}"));
        }
    }
    Ok(())
}

fn get(format: u32) -> Option<Vec<u8>> {
    unsafe {
        IsClipboardFormatAvailable(format).ok()?;
        let handle = GetClipboardData(format).ok()?;
        let global = HGLOBAL(handle.0);
        let size = GlobalSize(global);
        let ptr = GlobalLock(global);
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

fn utf16z(text: &str) -> Vec<u8> {
    // CF_UNICODETEXT ends at its first NUL, so one inside the text would cut
    // it short: a space instead, as gpui's own copy does.
    text.encode_utf16()
        .map(|u| if u == 0 { 0x20 } else { u })
        .chain(Some(0))
        .flat_map(|u| u.to_le_bytes())
        .collect()
}

fn from_utf16z(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn cf_html(fragment: &str) -> Vec<u8> {
    const HEADER: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let before = "<html><body>\r\n<!--StartFragment-->";
    let after = "<!--EndFragment-->\r\n</body></html>";
    let start_html = HEADER.len();
    let start_fragment = start_html + before.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + after.len();
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    debug_assert_eq!(header.len(), HEADER.len());
    let mut out = format!("{header}{before}{fragment}{after}").into_bytes();
    out.push(0);
    out
}

fn html_fragment(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let offset = |key: &str| -> Option<usize> {
        let at = text.find(key)? + key.len();
        text[at..].lines().next()?.trim().parse().ok()
    };
    let (start, end) = (offset("StartFragment:")?, offset("EndFragment:")?);
    let slice = bytes.get(start..end.min(bytes.len()))?;
    Some(String::from_utf8_lossy(slice).into_owned())
}

fn dib(w: u32, h: u32, rgba: &[u8], v5: bool) -> Vec<u8> {
    let header = if v5 { 124u32 } else { 40 };
    let size = w as usize * h as usize * 4;
    let mut out = Vec::with_capacity(header as usize + size);
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    u32le(&mut out, header);
    u32le(&mut out, w);
    u32le(&mut out, h); // positive: bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    u32le(&mut out, if v5 { 3 } else { 0 }); // BI_BITFIELDS / BI_RGB
    u32le(&mut out, size as u32);
    u32le(&mut out, 2835); // 72 dpi, as Chrome writes
    u32le(&mut out, 2835);
    u32le(&mut out, 0); // colours used
    u32le(&mut out, 0); // colours important
    if v5 {
        u32le(&mut out, 0x00ff_0000); // red mask
        u32le(&mut out, 0x0000_ff00); // green
        u32le(&mut out, 0x0000_00ff); // blue
        u32le(&mut out, 0xff00_0000); // alpha
        u32le(&mut out, 0x7352_4742); // LCS_sRGB
        out.extend_from_slice(&[0u8; 36]); // endpoints
        out.extend_from_slice(&[0u8; 12]); // gamma
        u32le(&mut out, 4); // LCS_GM_IMAGES
        u32le(&mut out, 0); // profile data
        u32le(&mut out, 0); // profile size
        u32le(&mut out, 0); // reserved
    }
    debug_assert_eq!(out.len(), header as usize);
    let row = w as usize * 4;
    for y in (0..h as usize).rev() {
        for px in rgba[y * row..(y + 1) * row].chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    out
}

fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |at: usize| Some(u32::from_le_bytes(dib.get(at..at + 4)?.try_into().ok()?));
    let header = u32_at(0)? as usize;
    let w = i32::from_le_bytes(dib.get(4..8)?.try_into().ok()?);
    let h = i32::from_le_bytes(dib.get(8..12)?.try_into().ok()?);
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let compression = u32_at(16)?;
    if w <= 0 || h == 0 || !(bits == 24 || bits == 32) || !(compression == 0 || compression == 3) {
        return None;
    }
    let (w, top_down, h) = (w as u32, h < 0, h.unsigned_abs());
    // BI_BITFIELDS after a plain 40-byte header carries its three masks after it.
    let masks = if compression == 3 && header == 40 {
        12
    } else {
        0
    };
    // Alpha only where the header says there is some: a V4/V5 alpha mask.
    let has_alpha = bits == 32 && header >= 108 && u32_at(52).is_some_and(|m| m != 0);
    let stride = (w as usize * bits as usize / 8 + 3) & !3;
    let pixels = dib.get(header + masks..)?;
    if pixels.len() < stride * h as usize {
        return None;
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let src = if top_down { y } else { h as usize - 1 - y };
        let line = &pixels[src * stride..src * stride + stride];
        for x in 0..w as usize {
            let p = &line[x * bits as usize / 8..];
            let a = if has_alpha { p[3] } else { 255 };
            rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
        }
    }
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dib_round_trips_through_the_png_a_read_makes_of_it() {
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ];
        for v5 in [true, false] {
            let png = dib_to_png(&dib(2, 2, &rgba, v5)).expect("a 32-bit DIB reads");
            let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                .unwrap()
                .to_rgba8();
            assert_eq!(back.dimensions(), (2, 2));
            let expected: Vec<u8> = if v5 {
                rgba.to_vec()
            } else {
                // A BI_RGB bitmap has no alpha to speak of: opaque.
                rgba.chunks(4)
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect()
            };
            assert_eq!(back.as_raw(), &expected, "v5 = {v5}");
        }
    }

    #[test]
    fn cf_html_offsets_frame_the_fragment() {
        let block = cf_html("<b>héllo</b>");
        assert_eq!(block.last(), Some(&0));
        assert_eq!(
            html_fragment(&block[..block.len() - 1]).as_deref(),
            Some("<b>héllo</b>")
        );
        let text = String::from_utf8_lossy(&block);
        assert!(
            text.starts_with("Version:0.9\r\nStartHTML:0000000"),
            "{text}"
        );
    }

    #[test]
    fn text_is_nul_terminated_utf16_and_reads_back_to_its_first_nul() {
        let bytes = utf16z("a\0b");
        assert_eq!(from_utf16z(&bytes), "a b");
        assert_eq!(&bytes[bytes.len() - 2..], &[0, 0]);
    }
}
