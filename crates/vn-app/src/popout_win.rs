//! Win32 helpers for a `<VnWindow>` that is shown and hidden, owned by the main window, or laid
//! over a monitor. They act on the raw HWND and do nothing on a handle that is not one.

#[cfg(target_os = "windows")]
mod imp {
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
        MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetClientRect, GetForegroundWindow, GetPropW,
        GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowRect, IsWindowVisible, LWA_ALPHA,
        MA_NOACTIVATE, RemovePropW, SW_HIDE, SW_SHOW, SW_SHOWNA, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetForegroundWindow,
        SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR, WDA_NONE, WM_MOUSEACTIVATE, WM_NCDESTROY,
        WM_PAINT, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    use windows::core::{BOOL, w};

    fn h(raw: isize) -> HWND {
        HWND(raw as *mut _)
    }

    /// `[left, top, right, bottom]`, physical px.
    pub type Rect = [i32; 4];

    fn rect(r: RECT) -> Rect {
        [r.left, r.top, r.right, r.bottom]
    }

    /// Taskbar included. Physical px.
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

    pub fn client_rect_on_screen(raw: isize) -> Option<Rect> {
        let mut r = RECT::default();
        unsafe { GetClientRect(h(raw), &mut r) }.ok()?;
        let mut origin = POINT::default();
        unsafe { ClientToScreen(h(raw), &mut origin) }.ok().ok()?;
        Some([origin.x, origin.y, origin.x + r.right, origin.y + r.bottom])
    }

    /// Owned by `owner`, without the taskbar button `WS_EX_APPWINDOW` forces. The taskbar reads
    /// styles on show: call while hidden.
    pub fn make_owned(raw: isize, owner: isize) {
        unsafe {
            SetWindowLongPtrW(h(raw), GWLP_HWNDPARENT, owner);
            let ex = GetWindowLongPtrW(h(raw), GWL_EXSTYLE);
            SetWindowLongPtrW(h(raw), GWL_EXSTYLE, ex & !(WS_EX_APPWINDOW.0 as isize));
        }
    }

    /// Frameless popup (client area = window rect, square corners, no border). Drops `NonRudeHWND`,
    /// which gpui sets on frameless windows so the taskbar does NOT step back; a cover wants it to.
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

    /// A move onto a monitor of another DPI makes Windows send `WM_DPICHANGED` with a suggested rect
    /// that gpui applies, so the first call can end elsewhere; hence the retry at the new DPI.
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

    /// `activate` also takes the foreground, which Windows allows when this process owns it already.
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

    /// If it is the foreground window, `return_to` gets the activation first: Windows does not
    /// always pick the owner next.
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

    /// Draws through `WM_PAINT` so a hidden window (which gets none) can be drawn before it is
    /// shown. Must not run inside an `App` update: the draw takes the app.
    pub fn paint_now(raw: isize) {
        unsafe {
            let _ = SendMessageW(h(raw), WM_PAINT, Some(WPARAM(0)), Some(LPARAM(0)));
        }
    }

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

    /// `EnumDisplayMonitors` order. Physical px.
    pub fn monitor_rect_at(index: usize) -> Option<Rect> {
        unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
                unsafe { (*(out.0 as *mut Vec<Rect>)).push(rect(info.rcMonitor)) };
            }
            true.into()
        }
        let mut all: Vec<Rect> = Vec::new();
        let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut all as *mut _ as isize)) };
        all.get(index).copied()
    }

    /// The mouse goes through to every process (`WS_EX_LAYERED | WS_EX_TRANSPARENT`; `HTTRANSPARENT`
    /// would only pass clicks to windows of the same thread). The constant alpha is set anyway: a
    /// layered window whose attributes were never set may be left undrawn.
    pub fn set_click_through(raw: isize, on: bool) {
        let hwnd = h(raw);
        let bits = (WS_EX_LAYERED | WS_EX_TRANSPARENT).0 as isize;
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let want = if on { ex | bits } else { ex & !bits };
            if want == ex {
                return;
            }
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
            if on {
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            }
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

    const NO_ACTIVATE: usize = 0x766e_6e61; // "vnna"

    unsafe extern "system" fn no_activate_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match msg {
            // gpui answers `MA_ACTIVATE` to every click, which would steal the foreground from a game.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCDESTROY => {
                let _ = unsafe { RemoveWindowSubclass(hwnd, Some(no_activate_proc), NO_ACTIVATE) };
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
        }
    }

    /// Never activated by a show or a click, and untracked by the shell: `WS_EX_NOACTIVATE` plus
    /// a `MA_NOACTIVATE` subclass (the style alone would not hold against gpui's `MA_ACTIVATE`),
    /// and `WS_EX_TOOLWINDOW` (no taskbar button, no `HSHELL_WINDOWCREATED`, so the shell does not
    /// re-check fullscreen state and the taskbar stays down over a borderless game).
    ///
    /// gpui's `NonRudeHWND` stays, or the taskbar steps back behind a click-through window
    /// covering the monitor; [`make_cover`] removes it. Call while hidden.
    pub fn make_unfocusable(raw: isize) {
        let hwnd = h(raw);
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let want = (ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW).0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
            let _ = SetWindowSubclass(hwnd, Some(no_activate_proc), NO_ACTIVATE, 0);
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

    /// `WDA_EXCLUDEFROMCAPTURE` (Windows 10 2004+), falling back to `WDA_MONITOR` (captured as
    /// black). Returns the affinity now in effect.
    pub fn set_content_protection(raw: isize, on: bool) -> u32 {
        let hwnd = h(raw);
        unsafe {
            if !on {
                let _ = SetWindowDisplayAffinity(hwnd, WDA_NONE);
            } else if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_err() {
                let _ = SetWindowDisplayAffinity(hwnd, WDA_MONITOR);
            }
        }
        display_affinity(raw)
    }

    pub fn display_affinity(raw: isize) -> u32 {
        let mut a = 0u32;
        let _ = unsafe { GetWindowDisplayAffinity(h(raw), &mut a) };
        a
    }

    pub fn describe_input(raw: isize) -> (bool, bool, bool) {
        let ex = unsafe { GetWindowLongPtrW(h(raw), GWL_EXSTYLE) };
        (
            ex & WS_EX_LAYERED.0 as isize != 0,
            ex & WS_EX_TRANSPARENT.0 as isize != 0,
            ex & WS_EX_NOACTIVATE.0 as isize != 0,
        )
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
    pub fn monitor_rect_at(_index: usize) -> Option<Rect> { None }
    pub fn set_click_through(_raw: isize, _on: bool) {}
    pub fn make_unfocusable(_raw: isize) {}
    pub fn set_content_protection(_raw: isize, _on: bool) -> u32 { 0 }
    pub fn display_affinity(_raw: isize) -> u32 { 0 }
    pub fn describe_input(_raw: isize) -> (bool, bool, bool) { (false, false, false) }
}

pub use imp::*;

/// Maps a rect between two windows' client spaces (CSS px) through the screen; the origins are
/// the client areas' screen origins, physical px.
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
