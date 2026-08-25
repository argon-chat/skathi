//! Keyboard focus traversal — the `Tab` / `Shift-Tab` half of the DOM's focus
//! model.
//!
//! gpui ships the machinery (`FocusHandle::tab_index`/`tab_stop`, the per-frame
//! tab-stop order, [`Window::focus_next`]/[`Window::focus_prev`]) but binds no
//! keys to it: `crates/gpui/examples/tab_stop.rs` wires its own actions in the
//! example's root view. A framework cannot do that — the root view belongs to
//! the host app and the focusable nodes come and go with the Vue tree — so the
//! bindings live here and the handlers are registered *globally*.
//!
//! Global action handlers run at the very end of the bubble phase, so anything
//! in the tree that wants `Tab` for itself (a future `<textarea>`, a
//! completion popup) can still bind it and win. And because they are global,
//! `Tab` also works from the cold state where nothing is focused yet — which is
//! what a browser does: the first `Tab` moves to the first tabbable element.

use gpui::{App, actions};

actions!(
    vn_focus,
    [
        /// Move focus to the next tab stop (`Tab`).
        FocusNext,
        /// Move focus to the previous tab stop (`Shift-Tab`).
        FocusPrev,
    ]
);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("tab", FocusNext, None),
        gpui::KeyBinding::new("shift-tab", FocusPrev, None),
    ]);
    cx.on_action(|_: &FocusNext, cx: &mut App| move_focus(cx, true));
    cx.on_action(|_: &FocusPrev, cx: &mut App| move_focus(cx, false));
}

/// The action is dispatched from inside the window's own update, so the window
/// is checked out of the app and `update_window` would fail re-entrantly — hop
/// over the effect boundary first.
fn move_focus(cx: &mut App, forward: bool) {
    cx.defer(move |cx| {
        // `active_window` is the platform's *foreground* window, which is None
        // whenever the app is in the background — and a keystroke can still
        // reach a background window (injected input, an OS accessibility
        // client). vue-native is single-window by construction (the window is
        // the root), so the one open window is the unambiguous fallback.
        let window = cx.active_window().or_else(|| {
            let mut windows = cx.windows();
            (windows.len() == 1).then(|| windows.remove(0))
        });
        let Some(window) = window else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            if forward {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
        });
    });
}
