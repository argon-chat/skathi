//! Windows: an owned, never-activated window placed in device px on the monitor's work area.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{px, App, Bounds, Pixels, Window};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GetWindow, GetWindowLongPtrW, GetWindowRect, IsWindow,
    SetWindowLongPtrW, SetWindowPos, GWLP_HWNDPARENT, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE,
    GW_HWNDPREV, HTTRANSPARENT, HWND_NOTOPMOST, MA_NOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SWP_SHOWWINDOW, WM_MOUSEACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY,
    WM_NCHITTEST, WNDPROC, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_POPUP,
};

use crate::windows::Rect;

#[derive(Clone, PartialEq)]
pub(super) struct Place {
    /// The window's frame, device px.
    spot: [i32; 4],
    inset: i32,
    owner: isize,
    /// The monitor's origin, device px.
    monitor: (f32, f32),
    scale: f32,
}

#[derive(Clone)]
pub(super) struct Native(HWND);

pub(super) fn locate(
    window: &Window,
    rect: Rect,
    gap: f32,
    size: (f32, f32),
    margin: f32,
    placement: super::Placement,
) -> Result<(Place, Option<Rect>), &'static str> {
    let owner = hwnd(window).ok_or("the window has no HWND")?;
    let (anchor, monitor) =
        anchor(owner, window.scale_factor(), rect).ok_or("no monitor under the anchor")?;
    let s = monitor.scale;
    let spot = super::frame_in(placement, anchor, monitor.work, gap, size, margin, s);
    let bounds = super::in_owner(spot, rect, anchor, window.scale_factor());
    Ok((
        Place {
            spot,
            inset: (margin * s).round() as i32,
            owner: owner.0 as isize,
            monitor: (monitor.bounds.0, monitor.bounds.1),
            scale: s,
        },
        Some(bounds),
    ))
}

pub(super) fn room(window: &Window, rect: Rect, gap: f32) -> Option<f32> {
    let owner = hwnd(window)?;
    let (r, m) = anchor(owner, window.scale_factor(), rect)?;
    Some(super::room_in(r, m.work, gap, m.scale))
}

pub(super) fn options(
    place: &Place,
    _owner: gpui::AnyWindowHandle,
    cx: &App,
) -> (Bounds<Pixels>, gpui::WindowKind, Option<gpui::DisplayId>) {
    let s = place.scale;
    let [x, y, w, h] = place.spot.map(|v| v as f32 / s);
    let display = cx
        .displays()
        .into_iter()
        .find(|d| {
            let o = d.bounds().origin;
            (f32::from(o.x) - place.monitor.0 / s).abs() < 1.0
                && (f32::from(o.y) - place.monitor.1 / s).abs() < 1.0
        })
        .map(|d| d.id());
    (
        Bounds {
            origin: gpui::point(px(x), px(y)),
            size: gpui::size(px(w), px(h)),
        },
        gpui::WindowKind::PopUp,
        display,
    )
}

pub(super) fn attach(
    popup: &mut Window,
    place: &Place,
    menu: bool,
) -> Result<Native, &'static str> {
    let hwnd = hwnd(popup).ok_or("the popup window has no HWND")?;
    let owner = HWND(place.owner as *mut core::ffi::c_void);
    configure(hwnd, owner, menu);
    position(hwnd, place.spot, place.inset, Some(owner));
    Ok(Native(hwnd))
}

pub(super) fn alive(native: &Native) -> bool {
    // SAFETY: only reads whether the handle names a window.
    unsafe { IsWindow(Some(native.0)) }.as_bool()
}

pub(super) fn update(
    native: &Native,
    from: &Place,
    to: &Place,
    _popup: gpui::AnyWindowHandle,
    _cx: &mut App,
) -> bool {
    if from.spot != to.spot {
        position(native.0, to.spot, to.inset, None);
    }
    true
}

fn hwnd(window: &Window) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

struct Monitor {
    /// The work area, device px.
    work: Rect,
    bounds: Rect,
    scale: f32,
}

/// `r` (client px of `owner`) on the screen in device px, and the monitor it is on.
fn anchor(owner: HWND, scale: f32, r: Rect) -> Option<(Rect, Monitor)> {
    let mut p = POINT {
        x: (r.0 * scale).round() as i32,
        y: (r.1 * scale).round() as i32,
    };
    unsafe { ClientToScreen(owner, &mut p) }.ok().ok()?;
    let (w, h) = (r.2 * scale, r.3 * scale);
    let rect = RECT {
        left: p.x,
        top: p.y,
        right: p.x + w.round() as i32,
        bottom: p.y + h.round() as i32,
    };
    let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.ok().ok()?;
    let (mut dx, mut dy) = (96u32, 96u32);
    let dpi = match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } {
        Ok(()) => dx,
        Err(_) => 96,
    };
    let of = |r: RECT| {
        (
            r.left as f32,
            r.top as f32,
            (r.right - r.left) as f32,
            (r.bottom - r.top) as f32,
        )
    };
    Some((
        (p.x as f32, p.y as f32, w, h),
        Monitor {
            work: of(info.rcWork),
            bounds: of(info.rcMonitor),
            scale: dpi as f32 / 96.0,
        },
    ))
}

thread_local! {
    /// A popup window's own procedure, and how far in its content starts, device px.
    static PROCS: RefCell<HashMap<isize, (isize, i32)>> = RefCell::new(HashMap::new());
}

/// Owned by `owner` (above it, gone with it) and never activated, so `owner` keeps the
/// keyboard and looks active; square, borderless corners: the content draws its own.
fn configure(popup: HWND, owner: HWND, menu: bool) {
    // SAFETY: both are live windows of this thread; the replaced procedure is kept and called.
    unsafe {
        SetWindowLongPtrW(popup, GWL_STYLE, (WS_POPUP | WS_CLIPSIBLINGS).0 as isize);
        let ex = GetWindowLongPtrW(popup, GWL_EXSTYLE);
        SetWindowLongPtrW(popup, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
        SetWindowLongPtrW(popup, GWLP_HWNDPARENT, owner.0 as isize);
        let previous = SetWindowLongPtrW(popup, GWLP_WNDPROC, procedure as *const () as isize);
        PROCS.with(|p| p.borrow_mut().insert(popup.0 as isize, (previous, 0)));
        let corners = if menu {
            DWMWCP_ROUND
        } else {
            DWMWCP_DONOTROUND
        };
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corners).cast(),
            std::mem::size_of_val(&corners) as u32,
        );
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_BORDER_COLOR,
            (&raw const border).cast(),
            std::mem::size_of_val(&border) as u32,
        );
    }
}

/// Moves `popup`; with `owner`, also shows it right above `owner` in the z-order.
fn position(popup: HWND, [x, y, w, h]: [i32; 4], inset: i32, owner: Option<HWND>) {
    PROCS.with(|p| {
        if let Some(e) = p.borrow_mut().get_mut(&(popup.0 as isize)) {
            e.1 = inset;
        }
    });
    let (after, flags) = match owner {
        Some(owner) => {
            // Behind the window over the owner, which also drops gpui's topmost; at the top of
            // the ordinary windows when only topmost ones are over it.
            // SAFETY: reads the z-order.
            let over = unsafe { GetWindow(owner, GW_HWNDPREV) }.ok().filter(|&w| {
                w != popup
                    && unsafe { GetWindowLongPtrW(w, GWL_EXSTYLE) } & WS_EX_TOPMOST.0 as isize == 0
            });
            (
                Some(over.unwrap_or(HWND_NOTOPMOST)),
                SWP_SHOWWINDOW | SWP_FRAMECHANGED,
            )
        }
        None => (None, SWP_NOZORDER),
    };
    // SAFETY: a live window of this thread.
    let _ = unsafe {
        SetWindowPos(
            popup,
            after,
            x,
            y,
            w,
            h,
            flags | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        )
    };
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some((previous, inset)) = PROCS.with(|p| p.borrow().get(&(hwnd.0 as isize)).copied())
    else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        // gpui activates a window on any press.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        // No frame: the whole window is client area.
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        // The shadow's room is not the popup: presses there reach the window under it.
        WM_NCHITTEST if inset > 0 => {
            let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
            let mut r = RECT::default();
            let outside = unsafe { GetWindowRect(hwnd, &mut r) }.is_ok()
                && (x < r.left + inset
                    || x >= r.right - inset
                    || y < r.top + inset
                    || y >= r.bottom - inset);
            if outside {
                return LRESULT(HTTRANSPARENT as isize);
            }
        }
        WM_NCDESTROY => {
            PROCS.with(|p| p.borrow_mut().remove(&(hwnd.0 as isize)));
        }
        _ => {}
    }
    // SAFETY: `previous` is the procedure `SetWindowLongPtrW` replaced.
    let previous: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) }
}
