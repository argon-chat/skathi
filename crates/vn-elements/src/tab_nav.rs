//! `Tab` / `Shift-Tab` focus traversal. gpui provides the machinery but binds no keys, and the root
//! view belongs to the host, so the bindings and *global* handlers live here. Global handlers run
//! last in the bubble phase, so a node that wants `Tab` can still win, and `Tab` works when nothing
//! is focused yet.

use gpui::{App, actions};

actions!(
    vn_focus,
    [
        FocusNext,
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

/// Dispatched from inside the window's own update, where the window is checked out of the app, so
/// hop over the effect boundary first.
fn move_focus(cx: &mut App, forward: bool) {
    cx.defer(move |cx| {
        // `active_window` is None while the app is in the background, but input can still reach a
        // window; the single open window is the fallback.
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
