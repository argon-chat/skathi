//! `<lottie>` in a build without the `lottie` feature: nothing animates.

use gpui::{App, Window};

use crate::windows::WinKey;

pub(crate) fn count() -> usize {
    0
}

pub(crate) fn gc(_live: impl Fn(u32) -> bool) {}

pub(crate) fn forget_window(_key: WinKey) {}

pub fn any_playing() -> bool {
    false
}

/// Other elements queue their retired images here; with `<lottie>` this tick drained them.
pub fn tick(window: &mut Window, cx: &mut App) -> bool {
    crate::windows::drain_drops(window, cx);
    false
}

pub(crate) fn window_tick(window: &mut Window, cx: &mut App) {
    crate::windows::drain_drops(window, cx);
}
