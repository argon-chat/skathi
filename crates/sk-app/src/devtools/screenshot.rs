//! Two screenshot backends.
//!
//! `render_to_image` (default): gpui re-renders the last scene into the D3D11 render target and
//! reads it back. It returns the client area exactly and needs no compositor, so occluded,
//! unfocused and off-screen windows read back what the app last painted. It does not capture the
//! mica/acrylic backdrop (DWM composites it underneath): those pixels come back with alpha < 255.
//!
//! `PrintWindow(PW_RENDERFULLCONTENT)`: asks DWM for the composed content, backdrop included, but
//! captures the whole window rect with its invisible resize border (hence `client_origin`).
//! `BitBlt` would return the GDI layer, not gpui's swapchain. Also the fallback if
//! `render_to_image` breaks upstream. On macOS it is the window server's image of the window.
//!
//! `backend` picks explicitly; `auto` tries the readback and falls back, recording why in the reply.
//!
//! A minimized window cannot really be asked: gpui parks `request_frame` on `SIZE_MINIMIZED` and
//! commands run one per frame, so the transport stops until the window is restored.

use std::path::{Path, PathBuf};

#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
};
// `PrintWindow` is under Storage::Xps in windows-rs' metadata; its flag is a plain `u32` in WindowsAndMessaging.
#[cfg(windows)]
use sk_windows::raw::PrintWindow;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    PW_RENDERFULLCONTENT,
};

/// Which capture path to use; `Auto` is the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Backend {
    /// `render_to_image`, falling back to `PrintWindow` on any failure.
    #[default]
    Auto,
    /// `render_to_image` only — a failure is an error, not a fallback.
    RenderToImage,
    /// `PrintWindow(PW_RENDERFULLCONTENT)` only.
    PrintWindow,
}

impl Backend {
    /// `None` for a spelling this command does not know.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "" | "auto" => Some(Self::Auto),
            "render_to_image" | "render-to-image" | "gpu" => Some(Self::RenderToImage),
            "print_window" | "print-window" | "printwindow" | "dwm" => Some(Self::PrintWindow),
            _ => None,
        }
    }
}

pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub hwnd: isize,
    pub iconic: bool,
    /// Where the CLIENT area starts inside the bitmap, in bitmap px; `(0, 0)` for
    /// `render_to_image`, so one crop formula works for both backends.
    pub client_origin: (i32, i32),
    /// `bitmap_x = css_x * scale + client_origin.0`.
    pub scale: f32,
    /// Which path produced the file: `"render_to_image"` or `"print_window"`.
    pub backend: &'static str,
    /// Why `Auto` did not use `render_to_image`, when it did not.
    pub fallback: Option<String>,
    /// `true` when the pixels carry real transparency (only `render_to_image`), meaning the window's mica/acrylic backdrop is absent from the capture.
    pub has_alpha: bool,
}

/// `<dist>/screenshots/<n>.png`, next to the bundle the host is running so parallel `SK_DIST` slots never collide.
pub fn next_default_path() -> PathBuf {
    let dir = crate::slot::path("screenshots");
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

/// UFCS: the inherent `Window::window_handle()` shadows the `HasWindowHandle` method. Falls back to
/// a pid lookup only when no window was named, since that finds whichever window enumerates first.
#[cfg(windows)]
fn hwnd_of_window() -> Option<isize> {
    let named = super::TARGET_WINDOW.with(|c| c.get()).is_some();
    super::with_target(|window, _| {
        let handle = <gpui::Window as HasWindowHandle>::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
            _ => None,
        }
    })
    .flatten()
    .or_else(|| if named { None } else { hwnd_by_pid() })
}

#[cfg(windows)]
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
#[cfg(windows)]
struct Gdi {
    screen: HDC,
    mem: HDC,
    bitmap: HBITMAP,
    old: windows::Win32::Graphics::Gdi::HGDIOBJ,
}

#[cfg(windows)]
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

#[cfg(not(windows))]
fn hwnd_of_window() -> Option<isize> {
    None
}

#[cfg(not(any(windows, target_os = "macos")))]
fn capture_print_window(_path: &Path) -> Result<Shot, String> {
    Err("print_window is not available on this platform; use render_to_image".into())
}

/// The window server's composed image of the window: webviews under the app's surface included.
#[cfg(target_os = "macos")]
fn capture_print_window(path: &Path) -> Result<Shot, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let (view, scale) = super::with_target(|window, _| {
        let handle = <gpui::Window as HasWindowHandle>::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return None;
        };
        Some((h.ns_view.as_ptr(), window.scale_factor()))
    })
    .flatten()
    .ok_or_else(|| "no live gpui window (not inside a committed frame)".to_string())?;
    let shot = sk_desktop::capture::own_window(view).map_err(|e| e.message)?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    image::save_buffer(
        path,
        &shot.rgba,
        shot.width,
        shot.height,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(Shot {
        path: path.to_path_buf(),
        width: shot.width,
        height: shot.height,
        hwnd: shot.window,
        iconic: false,
        client_origin: shot.client_origin,
        scale,
        backend: "print_window",
        fallback: None,
        has_alpha: false,
    })
}

pub fn capture(path: &Path, backend: Backend) -> Result<Shot, String> {
    match backend {
        Backend::RenderToImage => capture_render_to_image(path),
        Backend::PrintWindow => capture_print_window(path),
        Backend::Auto => match capture_render_to_image(path) {
            Ok(shot) => Ok(shot),
            // The readback is one upstream release away from breaking: `auto` degrades to the compositor and says so.
            Err(why) => capture_print_window(path).map(|mut shot| {
                shot.fallback = Some(why);
                shot
            }),
        },
    }
}

fn capture_render_to_image(path: &Path) -> Result<Shot, String> {
    // Not `?`: the handle is reply metadata only; the readback never touches the HWND.
    let hwnd_raw = hwnd_of_window();
    #[cfg(windows)]
    let iconic = hwnd_raw
        .is_some_and(|raw| unsafe { IsIconic(HWND(raw as *mut core::ffi::c_void)) }.as_bool());
    #[cfg(not(windows))]
    let iconic = false;

    let captured =
        super::with_target(|window, _| (window.render_to_image(), window.scale_factor()))
            .ok_or_else(|| "no live gpui window (not inside a committed frame)".to_string())?;
    let (image, scale) = captured;
    let image = image.map_err(|e| format!("render_to_image failed: {e}"))?;

    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err("render_to_image returned a zero-sized image".into());
    }
    // No minimized special-case: gpui parks `request_frame` on `SIZE_MINIMIZED` without resizing
    // the renderer, so an iconic window keeps a full-size target holding the last scene.
    let has_alpha = image.pixels().any(|p| p.0[3] != 255);

    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    image
        .save(path)
        .map_err(|e| format!("writing {}: {e}", path.display()))?;

    Ok(Shot {
        path: path.to_path_buf(),
        width,
        height,
        hwnd: hwnd_raw.unwrap_or(0),
        iconic,
        client_origin: (0, 0),
        scale,
        backend: "render_to_image",
        fallback: None,
        has_alpha,
    })
}

#[cfg(windows)]
fn capture_print_window(path: &Path) -> Result<Shot, String> {
    let hwnd_raw =
        hwnd_of_window().ok_or_else(|| "could not find this process' window handle".to_string())?;
    let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| format!("GetWindowRect failed: {e}"))?;
    let width = (rect.right - rect.left).max(0);
    let height = (rect.bottom - rect.top).max(0);
    if width == 0 || height == 0 {
        return Err("the window has a zero-sized rectangle — nothing to capture".into());
    }
    let iconic = unsafe { IsIconic(hwnd) }.as_bool();
    let client_origin = client_origin(hwnd, &rect);
    let scale = super::with_target(|window, _| window.scale_factor()).unwrap_or(1.0);

    // Top-down (negative height) 32-bit BGRA, so row 0 is the top row and no flip is needed.
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
        Gdi {
            screen,
            mem,
            bitmap,
            old,
        }
    };

    let ok = unsafe { PrintWindow(hwnd.0, gdi.mem.0, PW_RENDERFULLCONTENT) } != 0;
    if !ok {
        return Err(
            "PrintWindow(PW_RENDERFULLCONTENT) failed — the window may have been destroyed".into(),
        );
    }
    if bits.is_null() {
        return Err("the DIB has no backing pixels".into());
    }

    // BGRA (DWM does not always fill alpha) to opaque RGBA.
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
    image::save_buffer(
        path,
        &rgba,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("writing {}: {e}", path.display()))?;

    Ok(Shot {
        path: path.to_path_buf(),
        width: width as u32,
        height: height as u32,
        hwnd: hwnd_raw,
        iconic,
        client_origin,
        scale,
        backend: "print_window",
        fallback: None,
        has_alpha: false,
    })
}

/// Falls back to `(0, 0)`: a slightly wrong offset beats no screenshot.
#[cfg(windows)]
fn client_origin(hwnd: HWND, window_rect: &RECT) -> (i32, i32) {
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        (origin.x - window_rect.left, origin.y - window_rect.top)
    } else {
        (0, 0)
    }
}
