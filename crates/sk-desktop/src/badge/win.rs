//! The badge as the taskbar button's overlay icon, drawn as Chromium does: the count, or a dot for a flag.

use std::cell::RefCell;
use std::ffi::c_void;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, GdiFlush, SelectObject, SetBkMode,
    SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_BOLD, HBITMAP, HDC, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO, SM_CXSMICON};

use super::label;

thread_local! {
    static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
}

fn taskbar() -> Option<ITaskbarList3> {
    TASKBAR.with(|t| {
        if t.borrow().is_none() {
            // SAFETY: plain COM setup on this thread; an already initialised apartment is fine.
            let list: ITaskbarList3 = unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER).ok()?
            };
            // SAFETY: `list` is a live ITaskbarList3.
            unsafe { list.HrInit() }.ok()?;
            *t.borrow_mut() = Some(list);
        }
        t.borrow().clone()
    })
}

pub fn set(owner: Option<isize>, contents: i64) -> bool {
    let Some(hwnd) = owner else { return false };
    let Some(list) = taskbar() else { return false };
    let hwnd = HWND(hwnd as *mut c_void);
    let Some(text) = label(contents) else {
        // SAFETY: a null icon removes the overlay.
        return unsafe { list.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null()) }.is_ok();
    };
    // SAFETY: plain metrics queries.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd)) }.max(16) as usize;
    let Some(icon) = icon(&draw(&text, size), size) else { return false };
    let desc = HSTRING::from(if text.is_empty() { "•" } else { text.as_str() });
    // SAFETY: the taskbar copies the icon, so it is destroyed right after.
    unsafe {
        let ok = list.SetOverlayIcon(hwnd, icon, &desc).is_ok();
        let _ = DestroyIcon(icon);
        ok
    }
}

const RED: [f32; 3] = [209.0, 52.0, 56.0];

/// Straight-alpha BGRA, top-down.
fn draw(text: &str, size: usize) -> Vec<u8> {
    let s = size as f32;
    let c = s / 2.0;
    let r = if text.is_empty() { s * 0.32 } else { c };
    let cov = if text.is_empty() { Vec::new() } else { coverage(text, size).unwrap_or_default() };
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let i = y * size + x;
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let a = (r - d + 0.5).clamp(0.0, 1.0);
            let t = cov.get(i).map_or(0.0, |&v| v as f32 / 255.0);
            let px = &mut out[i * 4..i * 4 + 4];
            for (k, ch) in [2usize, 1, 0].into_iter().enumerate() {
                px[ch] = (RED[k] + (255.0 - RED[k]) * t).round() as u8;
            }
            px[3] = (a.max(t) * 255.0).round() as u8;
        }
    }
    out
}

unsafe fn dib(dc: Option<HDC>, size: usize) -> Option<(HBITMAP, *mut u8)> {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    (!bits.is_null()).then_some((bmp, bits.cast()))
}

/// The text's grayscale coverage: GDI draws white on black with no alpha of its own.
fn coverage(text: &str, size: usize) -> Option<Vec<u8>> {
    // SAFETY: every GDI object made here is selected out and deleted before returning.
    unsafe {
        let dc = CreateCompatibleDC(None);
        let Some((bmp, bits)) = dib(Some(dc), size) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let scale = match text.chars().count() {
            1 => 0.8,
            2 => 0.66,
            _ => 0.44,
        };
        let font = CreateFontW(
            -((size as f32 * scale).round() as i32),
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old_bmp = SelectObject(dc, bmp.into());
        let old_font = SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        SetBkMode(dc, TRANSPARENT);
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = RECT { left: 0, top: 0, right: size as i32, bottom: size as i32 };
        DrawTextW(dc, &mut wide, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        let _ = GdiFlush();
        let px = std::slice::from_raw_parts(bits, size * size * 4);
        let cov = px.chunks_exact(4).map(|p| p[0].max(p[1]).max(p[2])).collect();
        SelectObject(dc, old_font);
        SelectObject(dc, old_bmp);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        Some(cov)
    }
}

fn icon(bgra: &[u8], size: usize) -> Option<HICON> {
    // SAFETY: both bitmaps are deleted once CreateIconIndirect has copied them.
    unsafe {
        let (color, bits) = dib(None, size)?;
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits, bgra.len());
        let mask_bits = vec![0u8; size.div_ceil(16) * 2 * size];
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_is_a_disc_with_the_count_on_it() {
        let size = 32;
        let px = draw("7", size);
        let at = |x: usize, y: usize| &px[(y * size + x) * 4..(y * size + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "the corner is outside the disc");
        let white = (0..size * size).filter(|&i| px[i * 4] > 240 && px[i * 4 + 3] == 255).count();
        assert!(white > 10, "the digit is drawn ({white} white pixels)");
        let dot = draw("", size);
        assert_eq!(dot[(16 * size + 16) * 4 + 3], 255);
        assert_eq!(dot[(16 * size + 2) * 4 + 3], 0, "a flag is a small dot");
        assert!(icon(&px, size).is_some_and(|i| unsafe { DestroyIcon(i) }.is_ok()));
    }
}
