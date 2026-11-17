//! `<sk-sticker>` in a build without the `lottie` feature.

use gpui::{App, Window};

use crate::windows::WinKey;

pub(crate) fn mem_stats() -> (usize, usize, usize) {
    (0, 0, 0)
}

pub(crate) fn gc(_live: impl Fn(u32) -> bool) {}

pub(crate) fn forget_window(_key: WinKey) {}

pub(crate) fn tick(_window: &mut Window, _cx: &mut App) -> bool {
    false
}

pub(crate) fn window_tick(_window: &mut Window) {}
