//! Two screenshot backends, and the rule for choosing between them.
//!
//! # `render_to_image` — the default
//!
//! gpui re-renders the last frame's scene into the D3D11 render target, copies
//! it into a `D3D11_USAGE_STAGING` texture and reads it back (zed#63012; the
//! fork ungates it, see the `gpui: make render_to_image available outside test
//! builds` patch). What comes back is the **client area, exactly** — no window
//! frame, no invisible resize border — so `clientX`/`clientY` are `0` and a
//! `query` rectangle crops with nothing but the scale factor. It needs no
//! compositor: occluded, unfocused and off-screen windows all read back the
//! scene the app last painted, byte for byte.
//!
//! Minimized is a special case that neither backend can actually be *asked*
//! about: gpui parks the window's `request_frame` callback on
//! `SIZE_MINIMIZED`, and devtools commands execute one per frame, so the whole
//! transport stops until the window is restored (measured: `ping` times out
//! while iconic and answers in 64 ms the moment it is restored). The render
//! target survives untouched, so if a capture *does* land while `IsIconic` is
//! true — the narrow race around the transition — the readback is the last real
//! scene rather than the stale composition `PrintWindow` would hand back.
//!
//! What it does **not** capture is anything the app did not draw itself. On a
//! `mica`/`acrylic` window the render target is cleared to transparent and the
//! system backdrop is composited *underneath* by DWM, so those pixels come back
//! with alpha < 255 and no backdrop behind them. That is the honest answer to
//! "what did the app paint", and it is the wrong answer to "what does the user
//! see".
//!
//! # `PrintWindow(PW_RENDERFULLCONTENT)` — kept, not retired
//!
//! Asks DWM for the *composed* content, which is exactly the pixels on screen —
//! system backdrop included — and works while the window is unfocused or fully
//! occluded. Why not `BitBlt` from the window DC: gpui paints through a DirectX
//! swapchain, and a swapchain's back buffer is not in the window's GDI surface,
//! so a `BitBlt` returns the (usually black) GDI layer.
//!
//! It captures the whole WINDOW rect, which on Windows 10/11 includes the
//! invisible resize border, hence `client_origin`. And DWM having nothing live
//! to compose for a minimized window is its documented hole: the call returns
//! whatever it last held.
//!
//! It stays for two reasons. It is the only backend that shows the mica/acrylic
//! backdrop, and it is the parachute for the day an upstream release breaks
//! `render_to_image` — tooling that can only see through one hole goes blind
//! when that hole closes. `backend` picks explicitly; `auto` (the default)
//! tries the readback and falls back with the reason recorded in the reply.

use std::path::{Path, PathBuf};

#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, ClientToScreen, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, ReleaseDC, SelectObject,
};
// `PrintWindow` lives under Storage::Xps in windows-rs' generated metadata
// (it is a user32 export the XPS printing headers declare), while the flag it
// takes is a plain `u32` constant in WindowsAndMessaging.
#[cfg(windows)]
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    PW_RENDERFULLCONTENT,
};

/// Which capture path to use. `Auto` is what the `screenshot` command sends
/// when the caller says nothing.
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
    /// Where the CLIENT area starts inside the captured bitmap, in bitmap
    /// pixels. `PrintWindow` captures the whole window rectangle — which on
    /// Windows 10/11 includes the invisible resize border (typically 8 px on
    /// the left, right and bottom) — while `query`/`tree` report bounds in CSS
    /// pixels relative to the client area. Without this offset the two cannot
    /// be lined up, so cropping a screenshot to one element is guesswork.
    ///
    /// `render_to_image` reads the client area back directly, so it reports
    /// `(0, 0)`. The FIELD stays in the reply either way: the crop formula is
    /// the same one, and a client that subtracts a constant zero keeps working
    /// against both backends and against whatever a future one reports.
    pub client_origin: (i32, i32),
    /// Bitmap pixels per CSS pixel, i.e. the window's scale factor. Completes
    /// the mapping: `bitmap_x = css_x * scale + client_origin.0`. Both backends
    /// capture in PHYSICAL pixels, so this is the real scale factor for both —
    /// it does not become 1 just because the offsets became 0.
    pub scale: f32,
    /// Which path actually produced the file: `"render_to_image"` or
    /// `"print_window"`.
    pub backend: &'static str,
    /// Why `Auto` did not use `render_to_image`, when it did not.
    pub fallback: Option<String>,
    /// `true` when the pixels carry real transparency, i.e. the app's own
    /// drawing does not fill the client area. Only `render_to_image` can
    /// produce it, and it means the window's mica/acrylic backdrop (which DWM
    /// composites underneath, not the app) is absent from this capture.
    pub has_alpha: bool,
}

/// `<dist>/screenshots/<n>.png` — next to the bundle the host is running, so
/// parallel `VN_DIST` slots never collide and an agent can find them without
/// being told a path.
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

/// The gpui window's HWND.
///
/// gpui implements `raw_window_handle::HasWindowHandle` on `Window`, but the
/// inherent `Window::window_handle()` (an `AnyWindowHandle`) shadows the trait
/// method — hence the UFCS call. The fallback exists because that is one
/// dependency's worth of coupling for something we can also derive from our own
/// pid, and a screenshot is exactly the command you want to still work when
/// something upstream moved.
#[cfg(windows)]
fn hwnd_of_window() -> Option<isize> {
    super::with_target(|window, _| {
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

/// No HWND to report off Windows.
#[cfg(not(windows))]
fn hwnd_of_window() -> Option<isize> {
    None
}

/// The compositor capture is `PrintWindow`'s, so Windows-only; `auto` reports
/// the readback's failure through this.
#[cfg(not(windows))]
fn capture_print_window(_path: &Path) -> Result<Shot, String> {
    Err("print_window is Windows-only; use render_to_image".into())
}

/// Capture through `backend`, writing a PNG to `path`.
pub fn capture(path: &Path, backend: Backend) -> Result<Shot, String> {
    match backend {
        Backend::RenderToImage => capture_render_to_image(path),
        Backend::PrintWindow => capture_print_window(path),
        Backend::Auto => match capture_render_to_image(path) {
            Ok(shot) => Ok(shot),
            // The readback is the better capture but it is one upstream release
            // away from breaking, so `auto` never lets that take the tooling
            // down with it — it degrades to the compositor and says so.
            Err(why) => capture_print_window(path).map(|mut shot| {
                shot.fallback = Some(why);
                shot
            }),
        },
    }
}

/// gpui's D3D11 staging readback of the last rendered scene.
fn capture_render_to_image(path: &Path) -> Result<Shot, String> {
    // Not `?`: the handle is reply metadata (and the `iconic` flag), not
    // something this path needs in order to work — the readback never touches
    // the HWND.
    let hwnd_raw = hwnd_of_window();
    #[cfg(windows)]
    let iconic = hwnd_raw.is_some_and(|raw| {
        unsafe { IsIconic(HWND(raw as *mut core::ffi::c_void)) }.as_bool()
    });
    #[cfg(not(windows))]
    let iconic = false;

    let captured = super::with_target(|window, _| {
        (window.render_to_image(), window.scale_factor())
    })
    .ok_or_else(|| "no live gpui window (not inside a committed frame)".to_string())?;
    let (image, scale) = captured;
    let image = image.map_err(|e| format!("render_to_image failed: {e}"))?;

    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err("render_to_image returned a zero-sized image".into());
    }
    // Deliberately NO minimized special-case. gpui parks the window's
    // `request_frame` callback on `WM_SIZE`/`SIZE_MINIMIZED` and does not
    // resize the renderer, so an iconic window keeps a full-size render target
    // holding the last scene it painted: the readback is correct, while the
    // `PrintWindow` this would otherwise fall back to is exactly the case that
    // returns something stale or blank. (In practice the flag is nearly
    // unreachable here — with `request_frame` parked, the devtools pump stops
    // too, so commands queue until the window is restored. See
    // docs/DEVTOOLS.md.)
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
        // The readback IS the client area.
        client_origin: (0, 0),
        scale,
        backend: "render_to_image",
        fallback: None,
        has_alpha,
    })
}

#[cfg(windows)]
fn capture_print_window(path: &Path) -> Result<Shot, String> {
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
    let scale = super::with_target(|window, _| window.scale_factor()).unwrap_or(1.0);

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
        backend: "print_window",
        fallback: None,
        // The DIB conversion forces alpha to 255 (DWM does not always fill it).
        has_alpha: false,
    })
}

/// The client area's top-left corner expressed inside the captured window
/// rectangle. `ClientToScreen(0,0)` gives it in screen space; subtracting the
/// window rect's origin puts it in bitmap space, which is where a caller
/// cropping a screenshot to a `query` rectangle needs it.
///
/// Falls back to `(0, 0)` rather than failing: a screenshot with a slightly
/// wrong offset is far more useful than no screenshot.
#[cfg(windows)]
fn client_origin(hwnd: HWND, window_rect: &RECT) -> (i32, i32) {
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        (origin.x - window_rect.left, origin.y - window_rect.top)
    } else {
        (0, 0)
    }
}
