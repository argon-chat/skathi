//! The `appWindow` verbs and state on macOS and Linux, through gpui's platform window where
//! Windows goes through the HWND.

use std::sync::atomic::Ordering;

use gpui::{point, px, size, Window};

use super::{advise, Config, Op, Snapshot, APP_HIDDEN, LIVE, SNAPSHOT};

const NO_POSITION: &str = "appWindow.setBounds()/center(): this platform does not let an app \
                           place its own window (Wayland), so the position was ignored; a size \
                           still applies. Let the compositor place the window.";
const NO_ON_TOP: &str = "appWindow.alwaysOnTop: this platform does not let an app keep its \
                         window above others (Wayland), so it stays false. Ask the user to pin \
                         the window from the compositor instead.";

/// macOS: a custom title bar's traffic lights (three 14 pt buttons 6 pt apart, at x = 12, see
/// [`super::traffic_light_position`]) plus the same margin after them.
pub(super) fn titlebar_inset() -> f32 {
    if cfg!(target_os = "macos") {
        12.0 + 3.0 * 14.0 + 2.0 * 6.0 + 12.0
    } else {
        0.0
    }
}

pub(super) fn init(cfg: &Config, window: &mut Window) {
    if cfg!(target_os = "macos") && LIVE.with(|l| l.borrow_mut().overlay.take()).is_some() {
        advise(super::NO_OVERLAY_MACOS);
    }
    if cfg.always_on_top {
        set_always_on_top(window, true);
    }
    if !cfg.maximizable {
        window.vn_set_maximizable(false);
    }
    sk_desktop::launch::set_raiser(super::request_activate);
}

/// The ops Windows applies through the HWND; anything else goes back to the shared `apply`.
pub(super) fn apply(op: Op, window: &mut Window, cx: &mut gpui::App) -> Option<Op> {
    match op {
        Op::Maximize => window.vn_set_maximized(true),
        Op::Unmaximize => {
            if window.vn_is_minimized() {
                window.vn_unminimize();
            } else {
                window.vn_set_maximized(false);
            }
        }
        Op::ToggleMaximize => window.vn_set_maximized(!window.vn_is_zoomed()),
        Op::Center => center(window),
        Op::AlwaysOnTop(on) => set_always_on_top(window, on),
        Op::StartDragging => window.start_window_move(),
        Op::Hide => {
            if !window.vn_hide() {
                window.minimize_window();
            }
            APP_HIDDEN.store(true, Ordering::Release);
        }
        // A raise brings the app forward too: AppKit raises a window only within its app.
        Op::Activate(true) if cfg!(target_os = "macos") && !crate::quiet::never_focus() => {
            cx.activate(true);
            return Some(op);
        }
        op => return Some(op),
    }
    None
}

/// The platform's own state where the Windows probes (`IsIconic`, `IsWindowVisible`) do not exist.
pub(super) fn refine(mut s: Snapshot, window: &Window) -> Snapshot {
    s.maximized = window.vn_is_zoomed();
    s.minimized = window.vn_is_minimized();
    s.visible = super::startup_visibility(window.vn_is_shown());
    s
}

fn set_always_on_top(window: &Window, on: bool) {
    let applied = window.vn_set_always_on_top(on);
    if !applied && on {
        advise(NO_ON_TOP);
    }
    LIVE.with(|l| l.borrow_mut().always_on_top = on && applied);
}

fn center(window: &Window) {
    let s = SNAPSHOT.with(|s| s.borrow().clone());
    let (_, _, ow, oh) = s.outer;
    let (_, _, aw, ah, al, at) = s.screen;
    if aw <= 0.0 {
        return;
    }
    let origin = point(px(al + (aw - ow) / 2.0), px(at + (ah - oh) / 2.0));
    if !window.vn_set_frame(Some(origin), None) {
        advise(NO_POSITION);
    }
}

/// CSS px are gpui's logical px. The size is clamped to the work area; the position is not, so a
/// window can move to another display (the platform keeps it reachable).
pub(super) fn set_bounds(
    window: &mut Window,
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    h: Option<f32>,
    content: bool,
) {
    let s = SNAPSHOT.with(|s| s.borrow().clone());
    let (_, _, aw, ah, _, _) = s.screen;
    let (ox, oy, ow, oh) = s.outer;
    // macOS with an OS title bar: the frame is taller than the content by the title bar.
    let title_bar = (oh - s.client.1).max(0.0);
    let new_size = (w.is_some() || h.is_some()).then(|| {
        let (cw, ch) = if content {
            (ow, oh - title_bar)
        } else {
            (ow, oh)
        };
        let mut nw = w.unwrap_or(cw).max(1.0);
        let mut nh = h.unwrap_or(ch).max(1.0);
        if aw > 0.0 {
            nw = nw.min(aw);
            nh = nh.min(ah);
        }
        size(px(nw), px(nh))
    });
    let origin = (x.is_some() || y.is_some()).then(|| {
        let nx = x.unwrap_or(ox);
        let ny = match y {
            Some(y) if content => y - title_bar,
            Some(y) => y,
            None => oy,
        };
        point(px(nx), px(ny))
    });
    if content {
        if let Some(sz) = new_size {
            window.resize(sz);
        }
        if origin.is_some() && !window.vn_set_frame(origin, None) {
            advise(NO_POSITION);
        }
        return;
    }
    if !window.vn_set_frame(origin, new_size) {
        if let Some(sz) = new_size {
            window.resize(sz);
        }
        if origin.is_some() {
            advise(NO_POSITION);
        }
    }
}
