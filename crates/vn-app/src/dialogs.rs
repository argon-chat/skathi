//! The windows file dialogs belong to (vn-desktop `dialog`, design §4.5).
//!
//! Two things live here, both because only vn-app can name a window:
//!
//! - **the owner** of a dialog asked for now: the window published for this
//!   call (the one an event came from, the primary for a timer), as its
//!   vn-tree key and HWND;
//! - **closing a window that owns a dialog.** A dialog runs its modal loop on
//!   its own thread, with this window as owner. Destroying the owner while
//!   that loop still uses it hangs or crashes, so every close path goes
//!   through here: the dialog is told to close (gpui's `DialogOwner`
//!   discipline, ported in vn-desktop), and the window is removed once the
//!   native loop has exited, usually within the dialog thread's 100 ms timer.

use std::time::Duration;

use gpui::{AnyWindowHandle, App};

/// vn-desktop's owner resolver.
pub fn owner() -> Option<vn_desktop::dialog::Owner> {
    if let Some(o) = vn_runtime::with_commit(|window, _| {
        let key = vn_tree::windows::key(window);
        crate::window::capture_hwnd(window).map(|hwnd| vn_desktop::dialog::Owner { key, hwnd })
    }) {
        return o;
    }
    // Outside a Commit publication (a host callback with no window): the
    // primary, the window a timer's work belongs to.
    let key = vn_tree::windows::primary();
    crate::window::hwnd_of(key).map(|hwnd| vn_desktop::dialog::Owner { key, hwnd })
}

/// Removes `handle`'s window, after its file dialog if it has one open.
pub fn remove_window(handle: AnyWindowHandle, cx: &mut App) {
    if vn_desktop::dialog::owner_closing(handle.window_id().as_u64()) {
        remove_when_idle(handle, cx);
    } else {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// A window's own close request (its X, Alt+F4, the taskbar): `true` closes
/// it now; `false` means it has a dialog open and is removed once that has
/// closed.
pub fn may_close_now(handle: AnyWindowHandle, cx: &mut App) -> bool {
    if !vn_desktop::dialog::owner_closing(handle.window_id().as_u64()) {
        return true;
    }
    remove_when_idle(handle, cx);
    false
}

fn remove_when_idle(handle: AnyWindowHandle, cx: &mut App) {
    let key = handle.window_id().as_u64();
    vn_tree::log::record("dialog", format!("[dialog] window {key} closes once its file dialog has closed"));
    // A gpui task rather than the frame pump: the pump is the primary
    // window's, and this may be a pop-out outliving it by those 100 ms.
    cx.spawn(async move |cx| {
        while !vn_desktop::dialog::is_idle(key) {
            cx.background_executor().timer(Duration::from_millis(16)).await;
        }
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    })
    .detach();
}
