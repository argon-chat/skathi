//! `<video>` in a build without the `video` feature: no player, and the media element reads as one
//! with nothing loaded.

use gpui::{App, Window};

pub(crate) fn count() -> usize {
    0
}

pub(crate) fn audible_count() -> usize {
    0
}

pub(crate) fn gc(_live: impl Fn(u32) -> bool) {}

pub fn device_changed() -> usize {
    0
}

pub fn any_playing() -> bool {
    false
}

pub fn tick(_window: &mut Window, _cx: &mut App) -> bool {
    false
}

pub(crate) fn drain_pending_uploads(_window: &mut Window) {}

pub(crate) fn drain_pending_drops_for(window: &mut Window, cx: &mut App) {
    crate::windows::drain_drops(window, cx);
}

pub(crate) fn media_state(_id: u32) -> Option<String> {
    None
}

pub(crate) fn media_prop(_id: u32, _code: u32) -> Option<f64> {
    None
}

pub(crate) fn media_call(_id: u32, _verb: u32, _arg: f64) -> bool {
    false
}

pub(crate) fn can_play_type(_id: u32, _mime_type: &str) -> String {
    String::new()
}

pub(crate) fn drain_recent(_id: u32) -> String {
    String::new()
}
