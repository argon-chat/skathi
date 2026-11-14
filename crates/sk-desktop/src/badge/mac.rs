//! The badge as the Dock tile's label; a flag is a dot, as Chromium shows it.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

use super::label;

/// The Dock tile is the app's, so no window is needed.
pub fn set(_owner: Option<isize>, contents: i64) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let text = label(contents).map(|t| NSString::from_str(if t.is_empty() { "•" } else { &t }));
    NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(text.as_deref());
    true
}
