//! Where the main window opens: `window.x`/`y`/`center`/`monitor` of `sk.config.json`, resolved
//! against gpui's displays before `open_window`, so the window is created in place.

use gpui::{point, px, size, App, Bounds, DisplayId, Pixels};

use crate::json::Json;
use crate::window::{advise, Config};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Monitor {
    #[default]
    Primary,
    Cursor,
    Index(usize),
}

/// Reads `window.monitor`; anything else advises and keeps the primary monitor.
pub fn parse_monitor(v: Option<&Json>, advisories: &mut Vec<String>) -> Monitor {
    match v {
        None | Some(Json::Null) => Monitor::Primary,
        Some(Json::Str(s)) if s == "primary" => Monitor::Primary,
        Some(Json::Str(s)) if s == "cursor" => Monitor::Cursor,
        Some(Json::Num(n)) if *n >= 0.0 && n.fract() == 0.0 => Monitor::Index(*n as usize),
        Some(other) => {
            advisories.push(format!(
                "`window.monitor: {}` is not a monitor — expected \"primary\", \"cursor\" or a \
                 monitor index (0, 1, …). The window opens on the primary monitor.",
                other.to_json()
            ));
            Monitor::Primary
        }
    }
}

/// The client bounds and display for the main window, in that display's logical px as gpui reads
/// `WindowOptions::window_bounds` on every platform.
pub fn initial_bounds(cfg: &Config, cx: &App) -> (Bounds<Pixels>, Option<DisplayId>) {
    let display = match cfg.monitor {
        Monitor::Primary => None,
        Monitor::Cursor => match cursor_display() {
            Some(id) => cx.find_display(id),
            None => {
                if !cfg!(target_os = "windows") {
                    advise(
                        "`window.monitor: \"cursor\"` is only read on Windows for now; the window \
                         opens on the primary monitor.",
                    );
                }
                None
            }
        },
        Monitor::Index(i) => {
            let all = cx.displays();
            let found = all.get(i).cloned();
            if found.is_none() {
                advise(&format!(
                    "`window.monitor: {i}`: there is no monitor {i} ({} connected, numbered from \
                     0). The window opens on the primary monitor.",
                    all.len()
                ));
            }
            found
        }
    }
    .or_else(|| cx.primary_display());

    let wanted = size(px(cfg.width), px(cfg.height));
    let Some(display) = display else {
        return (
            Bounds {
                origin: point(px(cfg.x.unwrap_or(0.0)), px(cfg.y.unwrap_or(0.0))),
                size: wanted,
            },
            None,
        );
    };
    let work = display.visible_bounds();
    let work = Rect {
        x: f32::from(work.origin.x),
        y: f32::from(work.origin.y),
        w: f32::from(work.size.width),
        h: f32::from(work.size.height),
    };
    let frame = caption_frame(cfg, display.id());
    let (x, y, w, h) = place(work, (cfg.width, cfg.height), cfg.x, cfg.y, frame);
    (
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        },
        Some(display.id()),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Client rect `(x, y, w, h)` in the work area: `x`/`y` offset from its top-left, an absent axis
/// centred, everything clamped inside. `frame` is the OS chrome above and below the client.
pub fn place(
    work: Rect,
    (width, height): (f32, f32),
    x: Option<f32>,
    y: Option<f32>,
    (above, below): (f32, f32),
) -> (f32, f32, f32, f32) {
    let w = width.min(work.w).max(1.0);
    let h = height.min(work.h - above - below).max(1.0);
    let left = match x {
        Some(x) => work.x + x,
        None => work.x + (work.w - w) / 2.0,
    };
    let top = match y {
        Some(y) => work.y + above + y,
        None => work.y + above + (work.h - above - below - h) / 2.0,
    };
    let left = left.clamp(work.x, (work.x + work.w - w).max(work.x));
    let top = top.clamp(
        work.y + above,
        (work.y + work.h - below - h).max(work.y + above),
    );
    (left, top, w, h)
}

#[cfg(target_os = "windows")]
fn cursor_display() -> Option<DisplayId> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.ok()?;
    let m = unsafe { MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST) };
    (!m.is_invalid()).then(|| DisplayId::new(m.0 as u64))
}

#[cfg(not(target_os = "windows"))]
fn cursor_display() -> Option<DisplayId> {
    None
}

/// The OS chrome above and below the client under `titleBarStyle: "default"`, in the display's
/// logical px.
#[cfg(target_os = "windows")]
fn caption_frame(cfg: &Config, display: DisplayId) -> (f32, f32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::HMONITOR;
    use windows::Win32::UI::HiDpi::{
        AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        WS_CAPTION, WS_EX_APPWINDOW, WS_SYSMENU, WS_THICKFRAME,
    };
    if cfg.title_bar_style != crate::window::TitleBarStyle::Default {
        return (0.0, 0.0);
    }
    let monitor = HMONITOR(u64::from(display) as _);
    let (mut dpi, mut dpi_y) = (0, 0);
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }.is_err()
        || dpi == 0
    {
        return (0.0, 0.0);
    }
    let mut r = RECT::default();
    let style = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME;
    if unsafe { AdjustWindowRectExForDpi(&mut r, style, false, WS_EX_APPWINDOW, dpi) }.is_err() {
        return (0.0, 0.0);
    }
    // The side and bottom borders are invisible resize margins; only the caption must stay on screen.
    ((-r.top) as f32 * 96.0 / dpi as f32, 0.0)
}

#[cfg(not(target_os = "windows"))]
fn caption_frame(_cfg: &Config, _display: DisplayId) -> (f32, f32) {
    (0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1032.0,
    };

    #[test]
    fn an_offset_is_from_the_work_area_and_an_absent_axis_is_centred() {
        let work = Rect {
            x: -1280.0,
            y: 40.0,
            ..WORK
        };
        assert_eq!(
            place(work, (800.0, 600.0), Some(40.0), None, (0.0, 0.0)),
            (-1240.0, 256.0, 800.0, 600.0)
        );
    }

    #[test]
    fn the_window_is_clamped_inside_the_work_area() {
        assert_eq!(
            place(WORK, (800.0, 600.0), Some(5000.0), Some(-300.0), (0.0, 0.0)),
            (1120.0, 0.0, 800.0, 600.0)
        );
        assert_eq!(
            place(WORK, (4000.0, 3000.0), Some(10.0), Some(10.0), (0.0, 0.0)),
            (0.0, 0.0, 1920.0, 1032.0)
        );
    }

    #[test]
    fn os_chrome_stays_on_screen() {
        let (_, top, _, h) = place(WORK, (800.0, 2000.0), Some(0.0), Some(0.0), (20.0, 19.0));
        assert_eq!((top, h), (20.0, 993.0));
    }
}
