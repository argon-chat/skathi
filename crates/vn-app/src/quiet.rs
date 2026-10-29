//! `VN_WINDOW_FOCUS=never` — a host that never takes the foreground.
//!
//! What vn-test sets for every launch (packages/vn-test/src/app.ts), so a spec
//! run on a machine somebody is working at does not pop windows over their
//! work or take the caret out of what they are typing:
//!
//! - every window (the primary and each pop-out) opens WITHOUT activation
//!   (`SW_SHOWNOACTIVATE`) and is then sent to the BOTTOM of the z-order, behind
//!   whatever the person has open;
//! - the startup `cx.activate(true)` is skipped;
//! - `window.focus()` neither raises nor flashes the taskbar button, whatever
//!   `window.focus` in vn.config.json says;
//! - the devtools `raise` command answers `raised: false`.
//!
//! The window still draws every frame behind other windows (gpui presents
//! through DirectComposition, which does not throttle an occluded window), and
//! the devtools screenshot reads gpui's own last scene rather than the screen
//! (devtools/screenshot.rs), so a spec sees exactly what it saw in front.
//! What does differ is OS activation: `document.hasFocus()` is false and the
//! window never receives `focus` from the OS — a spec about window activation
//! sets `VN_WINDOW_FOCUS` itself.

use std::sync::OnceLock;

use gpui::Window;

/// `VN_WINDOW_FOCUS=never`, read once.
pub fn never_focus() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("VN_WINDOW_FOCUS").is_ok_and(|v| v == "never"))
}

/// Sends a just-opened window behind every other one, without activating it.
/// Called from `open_window`'s builder, which runs before the window has
/// presented a frame — so it is never seen in front.
pub fn send_to_back(window: &Window) {
    if !never_focus() {
        return;
    }
    if let Some(h) = crate::window::capture_hwnd(window) {
        to_bottom(h);
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
