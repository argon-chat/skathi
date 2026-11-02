//! The Win32 half of a `<VnWindow>` that is more than a palette: one that is
//! shown and hidden rather than opened and closed, that the main window OWNS,
//! and that covers a whole monitor — a media viewer (docs/MULTIWINDOW.md §7.9).
//!
//! gpui has no words for any of it (`WindowOptions` knows a size, a kind and
//! whether to show at all), so these act on the HWND the window host cached,
//! the way `alwaysOnTop` and the dark mode already do. Every function takes the
//! raw HWND and does nothing on a handle that is not one.

#[cfg(target_os = "windows")]
mod imp {
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetClientRect, GetForegroundWindow, GetPropW,
        GetWindowLongPtrW, GetWindowRect, IsWindowVisible, RemovePropW, SW_HIDE, SW_SHOW, SW_SHOWNA,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
        SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_PAINT,
        WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
        WS_SYSMENU, WS_THICKFRAME,
    };
    use windows::core::w;

    fn h(raw: isize) -> HWND {
        HWND(raw as *mut _)
    }

    /// `[left, top, right, bottom]`, physical px.
    pub type Rect = [i32; 4];

    fn rect(r: RECT) -> Rect {
        [r.left, r.top, r.right, r.bottom]
    }

    /// The whole monitor `of` is on (or nearest to), taskbar included:
    /// `rcMonitor`, not the work area. Physical px.
    pub fn monitor_rect(of: isize) -> Option<Rect> {
        let m = unsafe { MonitorFromWindow(h(of), MONITOR_DEFAULTTONEAREST) };
        if m.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe { GetMonitorInfoW(m, &mut info) }.as_bool().then(|| rect(info.rcMonitor))
    }

    pub fn window_rect(raw: isize) -> Option<Rect> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(h(raw), &mut r) }.ok()?;
        Some(rect(r))
    }

    /// The client area in screen coordinates, physical px.
    pub fn client_rect_on_screen(raw: isize) -> Option<Rect> {
        let mut r = RECT::default();
        unsafe { GetClientRect(h(raw), &mut r) }.ok()?;
        let mut origin = POINT::default();
        unsafe { ClientToScreen(h(raw), &mut origin) }.ok().ok()?;
        Some([origin.x, origin.y, origin.x + r.right, origin.y + r.bottom])
    }

    /// Owned by `owner`, and without the taskbar button `WS_EX_APPWINDOW`
    /// forces even on an owned window. An owned window has no button of its
    /// own, stays above its owner, hides when the owner is minimised, and
    /// Alt-Tab lists the pair once. The taskbar reads the styles when the
    /// window is SHOWN, so this is done while it is hidden.
    pub fn make_owned(raw: isize, owner: isize) {
        unsafe {
            SetWindowLongPtrW(h(raw), GWLP_HWNDPARENT, owner);
            let ex = GetWindowLongPtrW(h(raw), GWL_EXSTYLE);
            SetWindowLongPtrW(h(raw), GWL_EXSTYLE, ex & !(WS_EX_APPWINDOW.0 as isize));
        }
    }

    /// What gpui's own fullscreen does to a window's styles, done once: no
    /// frame, no caption, no system menu, so the client area IS the window
    /// rect. Plus what a viewer laid over the desktop wants from DWM: square
    /// corners and no 1 px border, and no `NonRudeHWND`, which gpui sets on
    /// every frameless window precisely so that the taskbar does NOT step back
    /// for it — the opposite of what covering the monitor asks for.
    pub fn make_cover(raw: isize) {
        let hwnd = h(raw);
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let strip = (WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX).0 as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !strip) | WS_POPUP.0 as isize);
            let _ = RemovePropW(hwnd, w!("NonRudeHWND"));
            let corner = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const _,
                std::mem::size_of_val(&corner) as u32,
            );
            // DWMWA_COLOR_NONE.
            let none: u32 = 0xFFFF_FFFE;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &none as *const _ as *const _,
                std::mem::size_of_val(&none) as u32,
            );
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Puts the window at `r` and makes it stay there.
    ///
    /// A move onto a monitor of another DPI makes Windows send
    /// `WM_DPICHANGED` with a SUGGESTED rect, which gpui applies (the window
    /// keeps its logical size) — so the first call can end somewhere else, and
    /// a second, made at the new DPI, does not move monitors again.
    pub fn set_rect(raw: isize, r: Rect) -> Option<Rect> {
        for _ in 0..3 {
            unsafe {
                let _ = SetWindowPos(
                    h(raw),
                    None,
                    r[0],
                    r[1],
                    r[2] - r[0],
                    r[3] - r[1],
                    SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                );
            }
            if window_rect(raw) == Some(r) {
                break;
            }
        }
        window_rect(raw)
    }

    /// Shows the window. `activate`: it also becomes the foreground window,
    /// which a process may make it when it owns the foreground already — the
    /// case for a viewer opened by a click in the main window.
    pub fn show(raw: isize, activate: bool) {
        unsafe {
            if activate {
                let _ = ShowWindow(h(raw), SW_SHOW);
                let _ = SetForegroundWindow(h(raw));
            } else {
                let _ = ShowWindow(h(raw), SW_SHOWNA);
            }
        }
    }

    /// Hides the window. When it is the foreground window, `return_to` gets
    /// the activation first: hiding the active window lets Windows pick the
    /// next one, and it does not always pick the owner.
    pub fn hide(raw: isize, return_to: Option<isize>) {
        unsafe {
            if let Some(to) = return_to.filter(|_| GetForegroundWindow() == h(raw)) {
                let _ = SetForegroundWindow(h(to));
            }
            let _ = ShowWindow(h(raw), SW_HIDE);
        }
    }

    pub fn is_visible(raw: isize) -> bool {
        unsafe { IsWindowVisible(h(raw)) }.as_bool()
    }

    pub fn is_foreground(raw: isize) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        fg == h(raw)
    }

    /// Draws the window NOW, through its own `WM_PAINT`: gpui's frame for a
    /// dirty window, presented to its swapchain. A hidden window gets no
    /// `WM_PAINT` of its own, so this is what lets one be drawn before it is
    /// shown. Must not run inside an `App` update: the draw takes the app.
    pub fn paint_now(raw: isize) {
        unsafe {
            let _ = SendMessageW(h(raw), WM_PAINT, Some(WPARAM(0)), Some(LPARAM(0)));
        }
    }

    /// What a test asks of the styles: `(owner, appwindow, toolwindow, popup,
    /// thickframe, non_rude)`.
    pub fn describe(raw: isize) -> (isize, bool, bool, bool, bool, bool) {
        unsafe {
            let hwnd = h(raw);
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let owner = GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT);
            let non_rude = !GetPropW(hwnd, w!("NonRudeHWND")).is_invalid();
            (
                owner,
                ex & WS_EX_APPWINDOW.0 as isize != 0,
                ex & WS_EX_TOOLWINDOW.0 as isize != 0,
                style & WS_POPUP.0 as isize != 0,
                style & WS_THICKFRAME.0 as isize != 0,
                non_rude,
            )
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub type Rect = [i32; 4];
    pub fn monitor_rect(_of: isize) -> Option<Rect> { None }
    pub fn window_rect(_raw: isize) -> Option<Rect> { None }
    pub fn client_rect_on_screen(_raw: isize) -> Option<Rect> { None }
    pub fn make_owned(_raw: isize, _owner: isize) {}
    pub fn make_cover(_raw: isize) {}
    pub fn set_rect(_raw: isize, _r: Rect) -> Option<Rect> { None }
    pub fn show(_raw: isize, _activate: bool) {}
    pub fn hide(_raw: isize, _return_to: Option<isize>) {}
    pub fn is_visible(_raw: isize) -> bool { false }
    pub fn is_foreground(_raw: isize) -> bool { false }
    pub fn paint_now(_raw: isize) {}
    pub fn describe(_raw: isize) -> (isize, bool, bool, bool, bool, bool) { (0, false, false, false, false, false) }
}

pub use imp::*;

/// Maps a rectangle from one window's client space to another's, both in CSS
/// px: `from` in the source window at `from_scale`, through the screen, into
/// the target window at `to_scale`. `from_origin` / `to_origin` are the two
/// client areas' screen origins, physical px. Pure, so the arithmetic is
/// tested without windows.
pub fn map_rect(
    rect: (f32, f32, f32, f32),
    from_origin: (i32, i32),
    from_scale: f32,
    to_origin: (i32, i32),
    to_scale: f32,
) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    let sx = from_origin.0 as f32 + x * from_scale;
    let sy = from_origin.1 as f32 + y * from_scale;
    (
        (sx - to_origin.0 as f32) / to_scale,
        (sy - to_origin.1 as f32) / to_scale,
        w * from_scale / to_scale,
        h * from_scale / to_scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thumbnail at (100, 50) in a 1.5x main window whose client starts at
    /// (300, 200) on screen, seen from a 1x viewer covering a monitor at
    /// (-1920, 0) — and from one on the same monitor at 1.25x.
    #[test]
    fn a_rect_crosses_windows_through_the_screen() {
        let r = map_rect((100.0, 50.0, 40.0, 20.0), (300, 200), 1.5, (0, 0), 1.0);
        assert_eq!(r, (450.0, 275.0, 60.0, 30.0));
        let r = map_rect((100.0, 50.0, 40.0, 20.0), (-1500, 200), 1.0, (-1920, 0), 1.0);
        assert_eq!(r, (520.0, 250.0, 40.0, 20.0));
        let r = map_rect((0.0, 0.0, 80.0, 40.0), (250, 125), 1.25, (0, 0), 1.25);
        assert_eq!(r, (200.0, 100.0, 80.0, 40.0));
    }
}
