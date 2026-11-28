use core::ffi::c_void;

use serde_json::{json, Value};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{fit, png_url, Options};
use crate::Fail;

struct Dpi(DPI_AWARENESS_CONTEXT);
impl Drop for Dpi {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn failure(operation: &str) -> Fail {
    Fail::new(
        "OperationError",
        format!("{operation}: {}", std::io::Error::last_os_error()),
    )
}

pub(super) fn sources(options: Options) -> Result<Vec<Value>, Fail> {
    // DPI virtualisation would mix logical window coordinates with physical pixels on mixed-DPI displays.
    let _dpi =
        Dpi(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) });
    let mut result = Vec::new();
    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe extern "system" fn monitor_cb(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        unsafe {
            (&mut *(data.0 as *mut Vec<HMONITOR>)).push(monitor);
        }
        TRUE
    }
    if !unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_cb),
            LPARAM(&mut monitors as *mut _ as isize),
        )
    }
    .as_bool()
    {
        return Err(failure("EnumDisplayMonitors"));
    }
    if options.screens {
        for (index, monitor) in monitors.iter().copied().enumerate() {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if !unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) }
                .as_bool()
            {
                continue;
            }
            let rect = info.monitorInfo.rcMonitor;
            let device_name = wide(&info.szDevice);
            let (thumbnail, error) = preview(screen_thumbnail(rect, options));
            result.push(json!({
                "id": format!("screen:{}", monitor.0 as usize),
                "name": format!("Screen {}", index + 1),
                "kind": "screen",
                "displayId": (monitor.0 as usize).to_string(),
                "deviceName": device_name,
                "bounds": bounds(rect),
                "workArea": bounds(info.monitorInfo.rcWork),
                "width": rect.right - rect.left, "height": rect.bottom - rect.top,
                "isPrimary": info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                "thumbnailDataUrl": thumbnail, "thumbnailError": error,
                "appIconDataUrl": Value::Null,
            }));
        }
    }
    if options.windows {
        let mut windows: Vec<HWND> = Vec::new();
        unsafe extern "system" fn window_cb(window: HWND, data: LPARAM) -> windows::core::BOOL {
            let windows = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
            if windows.len() < 512 {
                windows.push(window);
            }
            TRUE
        }
        unsafe { EnumWindows(Some(window_cb), LPARAM(&mut windows as *mut _ as isize)) }
            .map_err(|e| Fail::new("OperationError", format!("EnumWindows: {e}")))?;
        for window in windows {
            let Some((name, rect, process_id)) = window_info(window) else {
                continue;
            };
            let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
            let (thumbnail, error) = preview(window_thumbnail(window, rect, options));
            let icon = if options.with_icons {
                window_icon(window).ok().flatten()
            } else {
                None
            };
            result.push(json!({
                "id": format!("window:{}", window.0 as usize),
                "name": name, "kind": "window", "processId": process_id,
                "displayId": (monitor.0 as usize).to_string(),
                "bounds": bounds(rect),
                "width": rect.right - rect.left, "height": rect.bottom - rect.top,
                "thumbnailDataUrl": thumbnail, "thumbnailError": error,
                "appIconDataUrl": icon,
            }));
        }
    }
    Ok(result)
}

fn wide(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())])
}

fn bounds(rect: RECT) -> Value {
    json!({"x":rect.left,"y":rect.top,"width":rect.right-rect.left,"height":rect.bottom-rect.top})
}

fn preview(value: Result<String, Fail>) -> (String, Option<String>) {
    match value {
        Ok(url) => (url, None),
        Err(e) => (String::new(), Some(e.message)),
    }
}

fn window_info(window: HWND) -> Option<(String, RECT, u32)> {
    unsafe {
        if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
            return None;
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        if style & WS_EX_TOOLWINDOW.0 != 0 && style & WS_EX_APPWINDOW.0 == 0 {
            return None;
        }
        if GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.0.is_null())
            && style & WS_EX_APPWINDOW.0 == 0
        {
            return None;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            4,
        )
        .is_ok()
            && cloaked != 0
        {
            return None;
        }
        let mut affinity = 0u32;
        if GetWindowDisplayAffinity(window, &mut affinity).is_ok() && affinity != 0 {
            return None;
        }
        let mut text = vec![0u16; (GetWindowTextLengthW(window).max(0) as usize + 1).min(32768)];
        let length = GetWindowTextW(window, &mut text).max(0) as usize;
        let title = String::from_utf16_lossy(&text[..length]);
        if title.trim().is_empty() {
            return None;
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err()
            || rect.right <= rect.left
            || rect.bottom <= rect.top
        {
            return None;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(window, Some(&mut process_id));
        Some((title, rect, process_id))
    }
}

struct Surface {
    screen: HDC,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
}

impl Surface {
    fn new(width: u32, height: u32) -> Result<Self, Fail> {
        if width == 0 || height == 0 || width as u64 * height as u64 > 32_000_000 {
            return Err(Fail::new(
                "OperationError",
                "capture surface is empty or exceeds 32 megapixels",
            ));
        }
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err(failure("GetDC"));
            }
            let dc = CreateCompatibleDC(Some(screen));
            if dc.is_invalid() {
                ReleaseDC(None, screen);
                return Err(failure("CreateCompatibleDC"));
            }
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) if !bits.is_null() => bitmap,
                other => {
                    if let Ok(bitmap) = other {
                        let _ = DeleteObject(bitmap.into());
                    }
                    let _ = DeleteDC(dc);
                    ReleaseDC(None, screen);
                    return Err(failure("CreateDIBSection"));
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            if previous.is_invalid() {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(dc);
                ReleaseDC(None, screen);
                return Err(failure("SelectObject"));
            }
            Ok(Self {
                screen,
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    fn png(&self) -> Result<String, Fail> {
        unsafe {
            let _ = GdiFlush();
        }
        let bgra = unsafe {
            std::slice::from_raw_parts(self.bits, self.width as usize * self.height as usize * 4)
        };
        let mut rgba = bgra.to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        png_url(&rgba, self.width, self.height)
    }

    fn stretch_from(&self, source: HDC, rect: RECT, include_layered: bool) -> Result<(), Fail> {
        unsafe {
            SetStretchBltMode(self.dc, HALFTONE);
            let _ = SetBrushOrgEx(self.dc, 0, 0, None);
            let operation = if include_layered {
                SRCCOPY | CAPTUREBLT
            } else {
                SRCCOPY
            };
            if !StretchBlt(
                self.dc,
                0,
                0,
                self.width as i32,
                self.height as i32,
                Some(source),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                operation,
            )
            .as_bool()
            {
                return Err(failure("StretchBlt"));
            }
        }
        Ok(())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
            ReleaseDC(None, self.screen);
        }
    }
}

fn screen_thumbnail(rect: RECT, options: Options) -> Result<String, Fail> {
    let (width, height) = fit(
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
        options.thumbnail_width,
        options.thumbnail_height,
    );
    if width == 0 || height == 0 {
        return Ok(String::new());
    }
    let surface = Surface::new(width, height)?;
    surface.stretch_from(surface.screen, rect, true)?;
    surface.png()
}

fn window_thumbnail(window: HWND, rect: RECT, options: Options) -> Result<String, Fail> {
    let (width, height) = fit(
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
        options.thumbnail_width,
        options.thumbnail_height,
    );
    if width == 0 || height == 0 {
        return Ok(String::new());
    }
    // PrintWindow may call the target's window procedure: a hung process must neither strand this
    // enumeration nor consume the I/O pool, so at most two isolated print workers are outstanding.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PRINT_WORKERS: AtomicUsize = AtomicUsize::new(0);
    if PRINT_WORKERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 2).then_some(n + 1)
        })
        .is_err()
    {
        return Err(Fail::new(
            "OperationError",
            "window preview workers are busy",
        ));
    }
    struct Worker;
    impl Drop for Worker {
        fn drop(&mut self) {
            PRINT_WORKERS.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let worker = Worker;
    let handle = window.0 as usize;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("sk-window-preview".into())
        .spawn(move || {
            let _worker = worker;
            let _dpi = Dpi(unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            });
            let result = std::panic::catch_unwind(|| {
                print_thumbnail(HWND(handle as *mut c_void), rect, width, height)
            })
            .unwrap_or_else(|_| {
                Err(Fail::new(
                    "OperationError",
                    "window preview failed inside the host",
                ))
            });
            let _ = send.send(result);
        })
        .map_err(|error| {
            Fail::new(
                "OperationError",
                format!("starting window preview: {error}"),
            )
        })?;
    receive
        .recv_timeout(std::time::Duration::from_millis(250))
        .map_err(|_| {
            Fail::new(
                "TimeoutError",
                "the window did not supply a preview within 250ms",
            )
        })?
}

fn print_thumbnail(window: HWND, rect: RECT, width: u32, height: u32) -> Result<String, Fail> {
    if unsafe { IsHungAppWindow(window) }.as_bool() {
        return Err(Fail::new("OperationError", "the window is not responding"));
    }
    // Printing its own surface preserves content when another window covers it.
    let full = Surface::new(
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
    )?;
    let printed =
        unsafe { sk_windows::raw::PrintWindow(window.0, full.dc.0, PW_RENDERFULLCONTENT) };
    if printed == 0 {
        return Err(failure("PrintWindow"));
    }
    let thumbnail = Surface::new(width, height)?;
    thumbnail.stretch_from(
        full.dc,
        RECT {
            left: 0,
            top: 0,
            right: full.width as i32,
            bottom: full.height as i32,
        },
        false,
    )?;
    thumbnail.png()
}

fn window_icon(window: HWND) -> Result<Option<String>, Fail> {
    let mut icon = 0usize;
    unsafe {
        for kind in [ICON_SMALL2, ICON_SMALL, ICON_BIG] {
            SendMessageTimeoutW(
                window,
                WM_GETICON,
                WPARAM(kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                50,
                Some(&mut icon),
            );
            if icon != 0 {
                break;
            }
        }
        if icon == 0 {
            icon = GetClassLongPtrW(window, GCLP_HICONSM);
        }
        if icon == 0 {
            icon = GetClassLongPtrW(window, GCLP_HICON);
        }
    }
    if icon == 0 {
        return Ok(None);
    }
    let icon = HICON(icon as *mut c_void);
    let black = Surface::new(32, 32)?;
    let white = Surface::new(32, 32)?;
    unsafe {
        std::ptr::write_bytes(black.bits, 0, 32 * 32 * 4);
        std::ptr::write_bytes(white.bits, 255, 32 * 32 * 4);
        DrawIconEx(black.dc, 0, 0, icon, 32, 32, 0, None, DI_NORMAL)
            .and_then(|_| DrawIconEx(white.dc, 0, 0, icon, 32, 32, 0, None, DI_NORMAL))
            .map_err(|e| Fail::new("OperationError", format!("DrawIconEx: {e}")))?;
        let _ = GdiFlush();
    }
    let black_pixels = unsafe { std::slice::from_raw_parts(black.bits, 32 * 32 * 4) };
    let white_pixels = unsafe { std::slice::from_raw_parts(white.bits, 32 * 32 * 4) };
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for ((black, white), output) in black_pixels
        .chunks_exact(4)
        .zip(white_pixels.chunks_exact(4))
        .zip(rgba.chunks_exact_mut(4))
    {
        let alpha = 255
            - (0..3)
                .map(|c| white[c].saturating_sub(black[c]))
                .max()
                .unwrap_or(0);
        output[3] = alpha;
        if alpha != 0 {
            for c in 0..3 {
                output[c] = (black[2 - c] as u32 * 255 / alpha as u32).min(255) as u8;
            }
        }
    }
    png_url(&rgba, 32, 32).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn desktop_enumerates_real_sources_and_decodable_previews() {
        let result = sources(Options {
            screens: true,
            windows: true,
            with_icons: true,
            thumbnail_width: 160,
            thumbnail_height: 90,
        })
        .unwrap();
        assert!(result.iter().any(|s| s["kind"] == "screen"));
        for source in result {
            let id = source["id"].as_str().unwrap();
            assert!(id.starts_with("screen:") || id.starts_with("window:"));
            assert!(source["width"].as_i64().unwrap() > 0);
            let thumbnail = source["thumbnailDataUrl"].as_str().unwrap();
            if thumbnail.is_empty() {
                assert!(source["thumbnailError"].is_string());
                continue;
            }
            use base64::Engine;
            let png = base64::engine::general_purpose::STANDARD
                .decode(thumbnail.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap();
            let image = image::load_from_memory(&png).unwrap();
            assert!(image.width() <= 160 && image.height() <= 90);
        }
    }
}
