//! Is a window covered by the windows above it?
//!
//! gpui presents through DirectComposition, which keeps a fully covered window drawing at the display
//! rate. Chromium's `NativeWindowOcclusionTrackerWin` counts such a window as hidden, as it does a
//! minimised or cloaked one, and so does this ([`crate::window::occluded`]): `document.hidden`, no
//! `requestAnimationFrame`, and JS on the background tick (src/background.rs).
//!
//! Chromium recomputes on window events from a thread of its own. Here the frame pump asks, and
//! [`RECHECK`] bounds how often the walk runs; the foreground window is never covered (it is read on
//! every ask, so it comes back at once).

use std::time::Duration;

pub const RECHECK: Duration = Duration::from_millis(200);

/// Past this many uncovered pieces the answer is "not covered": a pathological
/// tiling must not make the walk expensive, and drawing is the safe default.
const MAX_PIECES: usize = 256;

pub type Rect = (i32, i32, i32, i32);

fn empty(r: Rect) -> bool {
    r.0 >= r.2 || r.1 >= r.3
}

fn intersect(a: Rect, b: Rect) -> Rect {
    (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3))
}

fn subtract(r: Rect, cut: Rect, out: &mut Vec<Rect>) {
    let i = intersect(r, cut);
    if empty(i) {
        out.push(r);
        return;
    }
    for piece in [
        (r.0, r.1, r.2, i.1),
        (r.0, i.3, r.2, r.3),
        (r.0, i.1, i.0, i.3),
        (i.2, i.1, r.2, i.3),
    ] {
        if !empty(piece) {
            out.push(piece);
        }
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn covered(target: Rect, displays: &[Rect], above: impl IntoIterator<Item = Rect>) -> bool {
    let mut visible: Vec<Rect> = displays.iter().map(|&d| intersect(target, d)).filter(|&r| !empty(r)).collect();
    if visible.is_empty() {
        return true;
    }
    let mut next = Vec::new();
    for cut in above {
        next.clear();
        for &r in &visible {
            subtract(r, cut, &mut next);
        }
        std::mem::swap(&mut visible, &mut next);
        if visible.is_empty() {
            return true;
        }
        if visible.len() > MAX_PIECES {
            return false;
        }
    }
    false
}

#[cfg(target_os = "windows")]
mod win {
    use super::Rect;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetWindowRgnBox, HDC, HMONITOR};
    use windows::Win32::UI::WindowsAndMessaging::{
        GW_HWNDPREV, GWL_EXSTYLE, GetForegroundWindow, GetLayeredWindowAttributes, GetWindow,
        GetWindowLongW, GetWindowRect, IsIconic, IsWindowVisible, LAYERED_WINDOW_ATTRIBUTES_FLAGS,
        LWA_ALPHA, LWA_COLORKEY, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    /// The rect DWM draws: `GetWindowRect` includes the invisible resize
    /// borders, which would cover a strip of whatever is beside the window.
    fn bounds(h: HWND) -> Option<Rect> {
        let mut r = RECT::default();
        let dwm = unsafe {
            DwmGetWindowAttribute(
                h,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut RECT as *mut _,
                std::mem::size_of::<RECT>() as u32,
            )
        };
        if dwm.is_err() {
            unsafe { GetWindowRect(h, &mut r) }.ok()?;
        }
        Some((r.left, r.top, r.right, r.bottom))
    }

    fn cloaked(h: HWND) -> bool {
        let mut c: u32 = 0;
        let ok = unsafe {
            DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut c as *mut u32 as *mut _, std::mem::size_of::<u32>() as u32)
        };
        ok.is_ok() && c != 0
    }

    pub fn shown(h: HWND) -> bool {
        let shown = unsafe { IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() };
        shown && !cloaked(h)
    }

    fn opaque(h: HWND) -> bool {
        if !shown(h) {
            return false;
        }
        let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
        if ex & WS_EX_TRANSPARENT.0 != 0 {
            return false;
        }
        if ex & WS_EX_LAYERED.0 != 0 {
            let mut key = COLORREF(0);
            let mut alpha = 0u8;
            let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
            // Fails for a window that draws with per-pixel alpha
            // (`UpdateLayeredWindow`): it may be transparent anywhere.
            let ok = unsafe { GetLayeredWindowAttributes(h, Some(&mut key), Some(&mut alpha), Some(&mut flags)) };
            if ok.is_err() || flags.0 & LWA_COLORKEY.0 != 0 || (flags.0 & LWA_ALPHA.0 != 0 && alpha != 255) {
                return false;
            }
        }
        let mut rgn = RECT::default();
        if unsafe { GetWindowRgnBox(h, &mut rgn) }.0 != 0 {
            return false;
        }
        true
    }

    fn displays() -> Vec<Rect> {
        unsafe extern "system" fn each(_: HMONITOR, _: HDC, r: *mut RECT, data: LPARAM) -> windows::core::BOOL {
            let out = unsafe { &mut *(data.0 as *mut Vec<Rect>) };
            let r = unsafe { *r };
            out.push((r.left, r.top, r.right, r.bottom));
            true.into()
        }
        let mut out: Vec<Rect> = Vec::new();
        let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut Vec<Rect> as isize)) };
        out
    }

    pub fn covered(raw: isize) -> bool {
        let h = HWND(raw as *mut _);
        if unsafe { GetForegroundWindow() } == h {
            return false;
        }
        let Some(target) = bounds(h) else { return false };
        let displays = displays();
        let mut above = Vec::new();
        let mut w = unsafe { GetWindow(h, GW_HWNDPREV) }.ok();
        while let Some(next) = w {
            if opaque(next) {
                if let Some(r) = bounds(next) {
                    above.push(r);
                }
            }
            w = unsafe { GetWindow(next, GW_HWNDPREV) }.ok();
        }
        super::covered(target, &displays, above)
    }

    pub fn unseen(raw: isize) -> bool {
        !shown(HWND(raw as *mut _)) || covered(raw)
    }
}

#[cfg(not(target_os = "windows"))]
mod win {
    pub fn covered(_raw: isize) -> bool {
        false
    }
    pub fn unseen(_raw: isize) -> bool {
        false
    }
}

pub use win::{covered as hwnd_covered, unseen as hwnd_unseen};

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = (0, 0, 1920, 1080);

    #[test]
    fn a_window_under_a_bigger_one_is_covered() {
        assert!(covered((100, 100, 900, 700), &[SCREEN], [(0, 0, 1920, 1040)]));
    }

    #[test]
    fn a_sliver_left_uncovered_is_enough_to_draw() {
        assert!(!covered((100, 100, 900, 700), &[SCREEN], [(101, 100, 900, 700)]));
        assert!(!covered((100, 100, 900, 700), &[SCREEN], [(100, 100, 900, 699)]));
    }

    #[test]
    fn several_windows_can_cover_it_together() {
        let target = (0, 0, 1000, 1000);
        let halves = [(0, 0, 500, 1000), (500, 0, 1000, 600), (400, 500, 1200, 1100)];
        assert!(covered(target, &[SCREEN], halves));
        assert!(!covered(target, &[SCREEN], halves[..2].iter().copied()));
    }

    #[test]
    fn no_windows_above_leaves_it_visible() {
        assert!(!covered((100, 100, 900, 700), &[SCREEN], []));
    }

    #[test]
    fn only_the_part_on_a_display_counts() {
        let target = (1500, 100, 2300, 700);
        assert!(covered(target, &[SCREEN], [(1400, 0, 1920, 1080)]));
        assert!(!covered(target, &[SCREEN, (1920, 0, 3840, 1080)], [(1400, 0, 1920, 1080)]));
        assert!(covered((-3000, 0, -2000, 500), &[SCREEN], []));
    }

    #[test]
    fn subtraction_leaves_the_ring_around_a_hole() {
        let mut out = Vec::new();
        subtract((0, 0, 10, 10), (3, 3, 6, 6), &mut out);
        let area: i32 = out.iter().map(|r| (r.2 - r.0) * (r.3 - r.1)).sum();
        assert_eq!(area, 100 - 9);
        for (i, a) in out.iter().enumerate() {
            for b in &out[i + 1..] {
                assert!(empty(intersect(*a, *b)), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn a_pathological_tiling_gives_up_and_draws() {
        let cuts = (0..40).flat_map(|y| (0..40).map(move |x| (x * 20 + 5, y * 20 + 5, x * 20 + 6, y * 20 + 6)));
        assert!(!covered((0, 0, 800, 800), &[SCREEN], cuts));
    }
}
