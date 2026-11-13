//! `VN_WINDOW_FOCUS=never`: a host that never takes the foreground.
//!
//! What vn-test sets for every launch (packages/vn-test/src/app.ts), so a spec run on a machine
//! somebody is working at does not pop windows over their work or take the caret from what they type:
//!
//! - every window opens without activation (`SW_SHOWNOACTIVATE`) and is sent to the bottom of the
//!   z-order;
//! - the startup `cx.activate(true)` is skipped;
//! - `window.focus()` neither raises nor flashes the taskbar button, whatever `window.focus` in
//!   vn.config.json says;
//! - the devtools `raise` command answers `raised: false`.
//!
//! The window still draws every frame behind other windows (DirectComposition does not throttle an
//! occluded window), and the devtools screenshot reads gpui's last scene rather than the screen.
//!
//! The window is never active at the OS level, and gpui runs focus only in an active window (an
//! inactive one's focus events carry empty paths, so `on_focus` never fires). So every quiet window
//! is also marked assumed-active (`Window::set_assume_active`, a fork item): focus moves, keyboard
//! events, `document.hasFocus()` and the window's `focused` flag all read as they do in front. Every
//! window counts as active at once, which no OS allows. A spec about real window activation sets
//! `VN_WINDOW_FOCUS` itself.

use std::sync::OnceLock;

use gpui::Window;

/// `VN_WINDOW_FOCUS=never`, read once.
pub fn never_focus() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("VN_WINDOW_FOCUS").is_ok_and(|v| v == "never"))
}

/// Sends a just-opened window behind every other one, without activating it, and marks it
/// assumed-active (see the module comment). Called from `open_window`'s builder, before the first
/// frame, so it is never seen in front.
pub fn send_to_back(window: &mut Window) {
    if !never_focus() {
        return;
    }
    window.set_assume_active(true);
    if let Some(h) = crate::window::capture_hwnd(window) {
        send_hwnd_to_back(h);
    }
}

/// Reapply the background policy after a deferred native show, outside any
/// App update. The earlier hidden-window ordering is not the final show.
pub(crate) fn send_hwnd_to_back(raw: isize) {
    if never_focus() {
        to_bottom(raw);
    }
}

#[cfg(target_os = "windows")]
fn to_bottom(raw: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    let _ = unsafe {
        SetWindowPos(
            HWND(raw as *mut _),
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

#[cfg(not(target_os = "windows"))]
fn to_bottom(_raw: isize) {}
