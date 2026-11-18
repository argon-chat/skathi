//! `<iframe>`'s document, in the platform's webview. Implements `sk_tree::iframe::Backend` over WebView2 on
//! Windows (`win`) and WKWebView on macOS (`mac`); elsewhere [`backend`] returns `None` and every
//! `<iframe src>` answers `error`.
//!
//! The webview never loads the app's document directly: its top-level page is the relay page (`relay.html`,
//! which also documents the message protocol) holding the one real `<iframe>`. Both are served from the
//! internal origin [`APP_ORIGIN`] by a request handler. The app's `location.origin` (`app://redjs`) gets no
//! origin from a browser, so [`internal_url`] maps it.

use std::path::PathBuf;
use std::rc::Rc;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use mac::frame_in_view;

pub struct Config {
    pub assets: Rc<dyn Fn(&str) -> Option<Vec<u8>>>,
    pub user_data: PathBuf,
    pub page_origin: String,
    pub devtools: bool,
}

/// The origin of the relay page and the app's files inside the webview. `.invalid` never resolves
/// (RFC 6761), so a request the handler misses fails instead of reaching a network.
#[cfg(not(target_os = "macos"))]
pub const APP_ORIGIN: &str = "https://app.vn.invalid";
/// WKWebView hands no `https:` request to the app, so its files come from a scheme of their own.
#[cfg(target_os = "macos")]
pub const APP_ORIGIN: &str = "sk-app://app";
pub const RELAY_PATH: &str = "/__sk_iframe__/relay.html";

pub fn backend(config: Config) -> Option<Rc<dyn sk_tree::iframe::Backend>> {
    #[cfg(windows)]
    {
        Some(win::WebView2::new(config))
    }
    #[cfg(target_os = "macos")]
    {
        Some(mac::WebKit::new(config))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = config;
        None
    }
}

pub fn internal_url(src: &str, page_origin: &str) -> String {
    let src = src.trim();
    if let Some(rest) = src.strip_prefix(page_origin) {
        if rest.is_empty() || rest.starts_with(['/', '?', '#']) {
            return format!(
                "{APP_ORIGIN}{}{rest}",
                if rest.starts_with('/') { "" } else { "/" }
            );
        }
    }
    // `//host/path` is another host; a scheme is another origin.
    if src.starts_with("//") {
        return format!("https:{src}");
    }
    let has_scheme = src.split_once(':').is_some_and(|(s, _)| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && !s.contains('/')
    });
    if has_scheme {
        return src.to_string();
    }
    format!("{APP_ORIGIN}/{}", src.trim_start_matches('/'))
}

pub fn internal_origin(target: &str, page_origin: &str) -> String {
    if target == page_origin {
        APP_ORIGIN.to_string()
    } else {
        target.to_string()
    }
}

pub fn external_origin(origin: &str, page_origin: &str) -> String {
    if origin == APP_ORIGIN {
        page_origin.to_string()
    } else {
        origin.to_string()
    }
}

pub fn content_type(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "app://redjs";

    #[test]
    fn the_apps_own_urls_become_the_internal_origin() {
        assert_eq!(
            internal_url("app://redjs/games/pong/index.html", PAGE),
            format!("{APP_ORIGIN}/games/pong/index.html")
        );
        assert_eq!(
            internal_url("/games/pong/index.html?x=1#y", PAGE),
            format!("{APP_ORIGIN}/games/pong/index.html?x=1#y")
        );
        assert_eq!(
            internal_url("games/pong/index.html", PAGE),
            format!("{APP_ORIGIN}/games/pong/index.html")
        );
        assert_eq!(internal_url("app://redjs", PAGE), format!("{APP_ORIGIN}/"));
    }

    #[test]
    fn other_origins_are_left_alone() {
        assert_eq!(
            internal_url("https://games.example.com/a", PAGE),
            "https://games.example.com/a"
        );
        assert_eq!(
            internal_url("//cdn.example.com/a", PAGE),
            "https://cdn.example.com/a"
        );
        assert_eq!(internal_url("data:text/html,hi", PAGE), "data:text/html,hi");
        assert_eq!(
            internal_url("app://redjsx/evil", PAGE),
            "app://redjsx/evil",
            "a longer host is another origin"
        );
    }

    #[test]
    fn origins_map_both_ways() {
        assert_eq!(internal_origin("*", PAGE), "*");
        assert_eq!(internal_origin("/", PAGE), "/");
        assert_eq!(internal_origin(PAGE, PAGE), APP_ORIGIN);
        assert_eq!(external_origin(APP_ORIGIN, PAGE), PAGE);
        assert_eq!(external_origin("null", PAGE), "null");
    }
}
