//! `navigator.mediaSession` as the system media controls: the window's System Media Transport Controls on
//! Windows, the app's Now Playing entry on macOS. Button presses arrive as token-0 `mediasession` events.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
pub use mac::update;
#[cfg(target_os = "windows")]
pub use win::update;

/// Whether the system loads http(s) artwork itself; otherwise every image is sent as bytes.
pub const LOADS_ARTWORK_URLS: bool = cfg!(target_os = "windows");

fn event(action: &str, seek_time: Option<f64>) {
    let mut m = serde_json::Map::new();
    m.insert("kind".into(), "mediasession".into());
    m.insert("action".into(), action.into());
    if let Some(t) = seek_time {
        m.insert("seekTime".into(), t.into());
    }
    crate::emit(serde_json::Value::Object(m).to_string());
}
