//! The macOS source list's rules and JSON, kept free of ScreenCaptureKit so
//! they run as unit tests everywhere. `mac.rs` reads the platform into these
//! rows; the JSON matches `win.rs` key for key, plus macOS-only extras.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde_json::{json, Value};

use crate::Fail;

/// Windows stops at 512 windows too (`win.rs` `window_cb`).
pub(super) const MAX_WINDOWS: usize = 512;

/// A rectangle in the global display space, in points, origin top left
/// (CoreGraphics' space, which ScreenCaptureKit frames use).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn overlap(&self, other: &Rect) -> f64 {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }

    fn center_distance(&self, other: &Rect) -> f64 {
        let dx = (self.x + self.width / 2.0) - (other.x + other.width / 2.0);
        let dy = (self.y + self.height / 2.0) - (other.y + other.height / 2.0);
        dx * dx + dy * dy
    }

    fn json(&self) -> Value {
        json!({
            "x": self.x.round() as i64, "y": self.y.round() as i64,
            "width": self.width.round() as i64, "height": self.height.round() as i64,
        })
    }
}

/// AppKit's frames are bottom-left based with the primary screen's bottom
/// edge at 0; this is the same rectangle in CoreGraphics' top-left space.
pub(super) fn flip(rect: Rect, primary_height: f64) -> Rect {
    Rect { y: primary_height - (rect.y + rect.height), ..rect }
}

/// One `SCDisplay`.
#[derive(Debug, Clone)]
pub(super) struct DisplayRow {
    pub id: u32,
    pub frame: Rect,
    /// Backing pixels per point (2 on a Retina panel), from the display mode.
    pub scale: f64,
    pub primary: bool,
    /// From the main thread's `NSScreen`, when it answered in time.
    pub work_area: Option<Rect>,
    pub name: Option<String>,
}

/// One `SCWindow`, with its owning application.
#[derive(Debug, Clone, Default)]
pub(super) struct WindowRow {
    pub id: u32,
    pub title: String,
    pub frame: Rect,
    pub layer: isize,
    pub on_screen: bool,
    /// `None` when ScreenCaptureKit names no owning application.
    pub pid: Option<i32>,
    pub bundle_id: String,
    pub app_name: String,
}

/// System UI that sits at the normal window layer but is not something a
/// person shares: the menu bar extras, Control Center, Notification Center,
/// the Dock, Stage Manager, the screenshot UI.
const SYSTEM_BUNDLES: &[&str] = &[
    "com.apple.controlcenter",
    "com.apple.dock",
    "com.apple.notificationcenterui",
    "com.apple.screencaptureui",
    "com.apple.systemuiserver",
    "com.apple.WindowManager",
    "com.apple.wallpaper.agent",
];

/// The Windows picker's rules (`win.rs` `window_info`) in macOS terms:
/// visible (on screen), an ordinary window (layer 0, as `WS_EX_TOOLWINDOW`
/// and owned windows are dropped there), titled, non-empty, and owned by an
/// application. This app's own windows are left out too: sharing one would
/// show the call inside itself.
pub(super) fn keep_window(window: &WindowRow, own_pid: i32) -> bool {
    let Some(pid) = window.pid else { return false };
    window.on_screen
        && window.layer == 0
        && pid != own_pid
        && !window.title.trim().is_empty()
        && window.frame.width >= 1.0
        && window.frame.height >= 1.0
        && !SYSTEM_BUNDLES.contains(&window.bundle_id.as_str())
}

/// The main display first, then left to right, top to bottom: the order the
/// "Screen N" names follow.
pub(super) fn order_displays(displays: &mut [DisplayRow]) {
    displays.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then(a.frame.x.total_cmp(&b.frame.x))
            .then(a.frame.y.total_cmp(&b.frame.y))
    });
}

/// The display a window is on: the one it overlaps most, else the nearest,
/// as `MonitorFromWindow(MONITOR_DEFAULTTONEAREST)` answers on Windows.
pub(super) fn display_for(frame: &Rect, displays: &[DisplayRow]) -> Option<u32> {
    let best = displays
        .iter()
        .max_by(|a, b| frame.overlap(&a.frame).total_cmp(&frame.overlap(&b.frame)))?;
    if frame.overlap(&best.frame) > 0.0 {
        return Some(best.id);
    }
    displays
        .iter()
        .min_by(|a, b| frame.center_distance(&a.frame).total_cmp(&frame.center_distance(&b.frame)))
        .map(|d| d.id)
}

/// `(thumbnailDataUrl, thumbnailError)`, as `win.rs` `preview` shapes it.
pub(super) fn preview(value: Result<String, Fail>) -> (String, Option<String>) {
    match value {
        Ok(url) => (url, None),
        Err(e) => (String::new(), Some(e.message)),
    }
}

pub(super) fn screen_json(index: usize, display: &DisplayRow, thumbnail: Result<String, Fail>) -> Value {
    let (thumbnail, error) = preview(thumbnail);
    json!({
        "id": format!("screen:{}", display.id),
        "name": format!("Screen {}", index + 1),
        "kind": "screen",
        "displayId": display.id.to_string(),
        "deviceName": display.name.clone().unwrap_or_default(),
        "bounds": display.frame.json(),
        "workArea": display.work_area.unwrap_or(display.frame).json(),
        "width": display.frame.width.round() as i64, "height": display.frame.height.round() as i64,
        "scaleFactor": display.scale,
        "isPrimary": display.primary,
        "thumbnailDataUrl": thumbnail, "thumbnailError": error,
        "appIconDataUrl": Value::Null,
    })
}

pub(super) fn window_json(
    window: &WindowRow,
    display: Option<u32>,
    thumbnail: Result<String, Fail>,
    icon: Option<String>,
) -> Value {
    let (thumbnail, error) = preview(thumbnail);
    json!({
        "id": format!("window:{}", window.id),
        "name": window.title, "kind": "window", "processId": window.pid.unwrap_or(0),
        "appName": window.app_name, "bundleId": window.bundle_id,
        "displayId": display.map(|id| id.to_string()).unwrap_or_default(),
        "bounds": window.frame.json(),
        "width": window.frame.width.round() as i64, "height": window.frame.height.round() as i64,
        "thumbnailDataUrl": thumbnail, "thumbnailError": error,
        "appIconDataUrl": icon,
    })
}

/// Thumbnails are opaque, as on Windows (`Surface::png`).
pub(super) fn opaque(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
}

/// CoreGraphics draws into premultiplied RGBA only; PNG stores straight alpha.
pub(super) fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha < 255 {
            for c in &mut pixel[..3] {
                *c = ((*c as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    fn window(title: &str) -> WindowRow {
        WindowRow {
            id: 7,
            title: title.into(),
            frame: rect(10.0, 20.0, 800.0, 600.0),
            layer: 0,
            on_screen: true,
            pid: Some(42),
            bundle_id: "com.example.editor".into(),
            app_name: "Editor".into(),
        }
    }

    fn display(id: u32, frame: Rect, primary: bool) -> DisplayRow {
        DisplayRow { id, frame, scale: 2.0, primary, work_area: None, name: None }
    }

    #[test]
    fn the_picker_keeps_ordinary_titled_windows_of_other_apps() {
        assert!(keep_window(&window("Notes"), 1));
        assert!(!keep_window(&window("Notes"), 42), "this app's own window");
        assert!(!keep_window(&window("  "), 1), "untitled");
        assert!(!keep_window(&WindowRow { layer: 25, ..window("Menu") }, 1), "a panel above the normal layer");
        assert!(!keep_window(&WindowRow { on_screen: false, ..window("Hidden") }, 1));
        assert!(!keep_window(&WindowRow { pid: None, ..window("Orphan") }, 1));
        assert!(!keep_window(&WindowRow { frame: rect(0.0, 0.0, 0.0, 30.0), ..window("Empty") }, 1));
        let dock = WindowRow { bundle_id: "com.apple.dock".into(), ..window("Dock") };
        assert!(!keep_window(&dock, 1), "system UI");
    }

    #[test]
    fn a_window_belongs_to_the_display_it_overlaps_most_else_the_nearest() {
        let displays = [
            display(1, rect(0.0, 0.0, 1440.0, 900.0), true),
            display(2, rect(1440.0, 0.0, 1920.0, 1080.0), false),
        ];
        assert_eq!(display_for(&rect(100.0, 100.0, 400.0, 300.0), &displays), Some(1));
        assert_eq!(display_for(&rect(1300.0, 100.0, 800.0, 300.0), &displays), Some(2), "mostly on the second");
        assert_eq!(display_for(&rect(4000.0, 100.0, 100.0, 100.0), &displays), Some(2), "off every display");
        assert_eq!(display_for(&rect(0.0, 0.0, 1.0, 1.0), &[]), None);
    }

    #[test]
    fn the_main_display_is_screen_1_and_the_rest_follow_left_to_right() {
        let mut displays = vec![
            display(3, rect(-1920.0, 0.0, 1920.0, 1080.0), false),
            display(2, rect(1440.0, 0.0, 1920.0, 1080.0), false),
            display(1, rect(0.0, 0.0, 1440.0, 900.0), true),
        ];
        order_displays(&mut displays);
        assert_eq!(displays.iter().map(|d| d.id).collect::<Vec<_>>(), [1, 3, 2]);
    }

    #[test]
    fn appkit_frames_flip_into_the_top_left_space() {
        // A 900 pt primary with a 25 pt menu bar and a 70 pt Dock at the bottom.
        let visible = flip(rect(0.0, 70.0, 1440.0, 805.0), 900.0);
        assert_eq!(visible, rect(0.0, 25.0, 1440.0, 805.0));
        // A display above the primary has negative top-left y.
        assert_eq!(flip(rect(0.0, 900.0, 1920.0, 1080.0), 900.0).y, -1080.0);
    }

    #[test]
    fn sources_have_the_windows_keys() {
        let mut screen = display(5, rect(0.0, 0.0, 1440.5, 900.0), true);
        screen.name = Some("Built-in Retina Display".into());
        let value = screen_json(0, &screen, Ok("data:image/png;base64,AA==".into()));
        for key in ["id", "name", "kind", "displayId", "deviceName", "bounds", "workArea", "width", "height",
            "isPrimary", "thumbnailDataUrl", "thumbnailError", "appIconDataUrl"] {
            assert!(value.get(key).is_some(), "screen {key}");
        }
        assert_eq!(value["id"], "screen:5");
        assert_eq!(value["name"], "Screen 1");
        assert_eq!(value["displayId"], "5");
        assert_eq!(value["width"], 1441);
        assert_eq!(value["workArea"], value["bounds"], "without NSScreen, the whole display");
        assert!(value["thumbnailError"].is_null() && value["appIconDataUrl"].is_null());

        let value = window_json(&window("Notes"), Some(5), Err(Fail::new("TimeoutError", "late")), Some("icon".into()));
        for key in ["id", "name", "kind", "processId", "displayId", "bounds", "width", "height",
            "thumbnailDataUrl", "thumbnailError", "appIconDataUrl"] {
            assert!(value.get(key).is_some(), "window {key}");
        }
        assert_eq!(value["id"], "window:7");
        assert_eq!(value["processId"], 42);
        assert_eq!(value["displayId"], "5");
        assert_eq!(value["bounds"], json!({"x": 10, "y": 20, "width": 800, "height": 600}));
        assert_eq!((value["thumbnailDataUrl"].as_str(), value["thumbnailError"].as_str()), (Some(""), Some("late")));
        assert_eq!(value["appIconDataUrl"], "icon");
    }

    #[test]
    fn icons_lose_premultiplication_and_thumbnails_their_alpha() {
        let mut pixels = [100, 50, 0, 128, 9, 9, 9, 0, 1, 2, 3, 255];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [199, 100, 0, 128, 0, 0, 0, 0, 1, 2, 3, 255]);
        opaque(&mut pixels);
        assert_eq!([pixels[3], pixels[7], pixels[11]], [255, 255, 255]);
    }
}
