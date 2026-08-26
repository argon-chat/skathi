//! `PrintWindow(PW_RENDERFULLCONTENT)` → BGRA DIB → PNG.
//!
//! Why not `BitBlt` from the window DC: gpui paints through a DirectX
//! swapchain, and a swapchain's back buffer is not in the window's GDI surface
//! — a `BitBlt` returns the (usually black) GDI layer. `PrintWindow` with
//! `PW_RENDERFULLCONTENT` (Win 8.1+) asks DWM for the composed content
//! instead, which is exactly the pixels on screen, and works while the window
//! is unfocused or fully occluded. That property is the whole point of this
//! track: an agent must never have to raise or focus the user's window.
//!
//! Minimized windows are the documented hole: DWM has nothing live to compose,
//! so the call returns whatever it last held. We report `iconic: true` rather
//! than pretend the frame is current.

use std::path::{Path, PathBuf};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, ClientToScreen, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, ReleaseDC, SelectObject,
};
// `PrintWindow` lives under Storage::Xps in windows-rs' generated metadata
// (it is a user32 export the XPS printing headers declare), while the flag it
// takes is a plain `u32` constant in WindowsAndMessaging.
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    PW_RENDERFULLCONTENT,
};

pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub hwnd: isize,
    pub iconic: bool,
    /// Where the CLIENT area starts inside the captured bitmap, in bitmap
    /// pixels. `PrintWindow` captures the whole window rectangle — which on
    /// Windows 10/11 includes the invisible resize border (typically 8 px on
    /// the left, right and bottom) — while `query`/`tree` report bounds in CSS
    /// pixels relative to the client area. Without this offset the two cannot
    /// be lined up, so cropping a screenshot to one element is guesswork.
    pub client_origin: (i32, i32),
    /// Bitmap pixels per CSS pixel, i.e. the window's scale factor. Completes
    /// the mapping: `bitmap_x = css_x * scale + client_origin.0`.
    pub scale: f32,
}

/// `<dist>/screenshots/<n>.png` — next to the bundle the host is running, so
/// parallel `VN_DIST` slots never collide and an agent can find them without
/// being told a path.
pub fn next_default_path() -> PathBuf {
    let dir = crate::dev::dist("screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let next = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_suffix(".png"))
                        .and_then(|n| n.parse::<u32>().ok())
                })
                .max()
                .map_or(1, |m| m + 1)
        })
        .unwrap_or(1);
    dir.join(format!("{next}.png"))
}

/// The gpui window's HWND.
///
/// gpui implements `raw_window_handle::HasWindowHandle` on `Window`, but the
/// inherent `Window::window_handle()` (an `AnyWindowHandle`) shadows the trait
/// method — hence the UFCS call. The fallback exists because that is one
/// dependency's worth of coupling for something we can also derive from our own
/// pid, and a screenshot is exactly the command you want to still work when
/// something upstream moved.
fn hwnd_of_window() -> Option<isize> {
    vn_runtime::with_commit(|window, _| {
        let handle = <gpui::Window as HasWindowHandle>::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
            _ => None,
        }
    })
    .flatten()
    .or_else(hwnd_by_pid)
}

/// Robust fallback: the first visible top-level window owned by this process.
fn hwnd_by_pid() -> Option<isize> {
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Option<isize>) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == std::process::id() && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            *out = Some(hwnd.0 as isize);
            return false.into();
        }
        TRUE
    }
    let mut found: Option<isize> = None;
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// RAII for the GDI objects, so an early return cannot leak a DC or a bitmap.
struct Gdi {
    screen: HDC,
    mem: HDC,
    bitmap: HBITMAP,
    old: windows::Win32::Graphics::Gdi::HGDIOBJ,
}

impl Drop for Gdi {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.mem);
            ReleaseDC(None, self.screen);
        }
    }
}

pub fn capture(path: &Path) -> Result<Shot, String> {
    let hwnd_raw = hwnd_of_window()
        .ok_or_else(|| "could not find this process' window handle".to_string())?;
    let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .map_err(|e| format!("GetWindowRect failed: {e}"))?;
    let width = (rect.right - rect.left).max(0);
    let height = (rect.bottom - rect.top).max(0);
    if width == 0 || height == 0 {
        return Err("the window has a zero-sized rectangle — nothing to capture".into());
    }
    let iconic = unsafe { IsIconic(hwnd) }.as_bool();
    let client_origin = client_origin(hwnd, &rect);
    let scale = vn_runtime::with_commit(|window, _| window.scale_factor()).unwrap_or(1.0);

    // Top-down (negative height) 32-bit BGRA, so row 0 is the top row and the
    // buffer needs no vertical flip afterwards.
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let gdi = unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC(NULL) failed".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            ReleaseDC(None, screen);
            return Err("CreateCompatibleDC failed".into());
        }
        let bitmap = match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return Err(format!("CreateDIBSection failed: {e}"));
            }
        };
        let old = SelectObject(mem, bitmap.into());
        Gdi { screen, mem, bitmap, old }
    };

    let ok = unsafe { PrintWindow(hwnd, gdi.mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)) }.as_bool();
    if !ok {
        return Err(
            "PrintWindow(PW_RENDERFULLCONTENT) failed — the window may have been destroyed".into(),
        );
    }
    if bits.is_null() {
        return Err("the DIB has no backing pixels".into());
    }

    // BGRA (and an alpha byte DWM does not always fill) → opaque RGBA.
    let (w, h) = (width as usize, height as usize);
    let src = unsafe { std::slice::from_raw_parts(bits as *const u8, w * h * 4) };
    let mut rgba = vec![0u8; w * h * 4];
    for (dst, px) in rgba.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        dst[0] = px[2];
        dst[1] = px[1];
        dst[2] = px[0];
        dst[3] = 255;
    }
    drop(gdi);

    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    image::save_buffer(path, &rgba, width as u32, height as u32, image::ColorType::Rgba8)
        .map_err(|e| format!("writing {}: {e}", path.display()))?;

    Ok(Shot {
        path: path.to_path_buf(),
        width: width as u32,
        height: height as u32,
        hwnd: hwnd_raw,
        iconic,
        client_origin,
        scale,
    })
}

/// The client area's top-left corner expressed inside the captured window
/// rectangle. `ClientToScreen(0,0)` gives it in screen space; subtracting the
/// window rect's origin puts it in bitmap space, which is where a caller
/// cropping a screenshot to a `query` rectangle needs it.
///
/// Falls back to `(0, 0)` rather than failing: a screenshot with a slightly
/// wrong offset is far more useful than no screenshot.
fn client_origin(hwnd: HWND, window_rect: &RECT) -> (i32, i32) {
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        (origin.x - window_rect.left, origin.y - window_rect.top)
    } else {
        (0, 0)
    }
}
