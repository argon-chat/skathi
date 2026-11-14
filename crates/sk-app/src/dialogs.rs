//! The windows file dialogs belong to (sk-desktop `dialog`).
//!
//! **Closing a window that owns a dialog.** A dialog runs its modal loop on
//!   its own thread, with this window as owner. Destroying the owner while
//!   that loop still uses it hangs or crashes, so every close path goes
//!   through here: the dialog is told to close (gpui's `DialogOwner`
//!   discipline, ported in sk-desktop), and the window is removed once the
//!   native loop has exited, usually within the dialog thread's 100 ms timer.

use std::time::Duration;

use gpui::{AnyWindowHandle, App};

pub fn owner() -> Option<sk_desktop::dialog::Owner> {
    if let Some(o) = sk_runtime::with_commit(|window, _| {
        let key = sk_tree::windows::key(window);
        crate::window::dialog_parent(window).map(|hwnd| sk_desktop::dialog::Owner { key, hwnd })
    }) {
        return o;
    }
    let key = sk_tree::windows::primary();
    crate::window::dialog_parent_of(key).map(|hwnd| sk_desktop::dialog::Owner { key, hwnd })
}

/// On Linux nobody is asked: Allow, as a desktop browser without a portal grants its own capture.
/// The desktop has no per-app microphone permission to stand in for, and its only question dialogs
/// (zenity, kdialog) are often not installed. The answer is stored like any other.
pub fn ask_permission(q: sk_permissions::Question, reply: Box<dyn FnOnce(Option<usize>) + Send>) {
    let capability = q.capability;
    let ask = sk_desktop::dialog::Ask {
        what: "permission",
        title: q.title,
        instruction: q.instruction,
        content: q.content,
        buttons: q.buttons,
        default: q.default,
    };
    #[cfg(target_os = "windows")]
    sk_desktop::dialog::ask_with(
        ask,
        Box::new(move |q, owner, cancel| {
            crate::consent_window::present(capability, q, owner, cancel)
        }),
        reply,
    );
    #[cfg(target_os = "linux")]
    {
        let _ = capability;
        sk_desktop::dialog::ask_with(ask, Box::new(|_, _, _| Ok(Some(0))), reply);
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = capability;
        sk_desktop::dialog::ask(ask, reply);
    }
}

pub fn remove_window(handle: AnyWindowHandle, cx: &mut App) {
    if sk_desktop::dialog::owner_closing(handle.window_id().as_u64()) {
        remove_when_idle(handle, cx);
    } else {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

pub fn may_close_now(handle: AnyWindowHandle, cx: &mut App) -> bool {
    if !sk_desktop::dialog::owner_closing(handle.window_id().as_u64()) {
        return true;
    }
    remove_when_idle(handle, cx);
    false
}

fn remove_when_idle(handle: AnyWindowHandle, cx: &mut App) {
    let key = handle.window_id().as_u64();
    sk_tree::log::record(
        "dialog",
        format!("[dialog] window {key} closes once its file dialog has closed"),
    );
    // A gpui task rather than the frame pump: the pump is the primary
    // window's, and this may be a pop-out outliving it by those 100 ms.
    cx.spawn(async move |cx| {
        while !sk_desktop::dialog::is_idle(key) {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
        }
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    })
    .detach();
}
