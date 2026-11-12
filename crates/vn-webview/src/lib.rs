//! `<iframe>`'s document, in the platform's webview.
//!
//! `vn_tree::iframe` knows the element; this crate knows a browser. It
//! implements `vn_tree::iframe::Backend` over WebView2 on Windows (`win`), and
//! has no backend elsewhere yet: [`backend`] returns `None`, the host installs
//! nothing, and every `<iframe src>` answers `error` (requirements R8, in
//! research/reports/m26/iframe-webview-requirements-2026-11-12.md).
//!
//! # What the webview loads
//!
//! Never the app's document directly. Its top-level page is the relay page
//! (`relay.html`), which holds the one real `<iframe>` with the attributes the
//! app wrote; see that file for why and for the message protocol.
//!
//! Both are served from one internal origin, [`APP_ORIGIN`], by a request
//! handler: the relay page at [`RELAY_PATH`], and under every other path the
//! app's own files (`Config::assets`). The app's `location.origin`
//! (`app://redjs`) is not a scheme a browser gives an origin to, so the
//! backend maps it to the internal one on the way in ([`internal_url`]).

use std::path::PathBuf;
use std::rc::Rc;

#[cfg(windows)]
mod win;

/// What the host gives the backend.
pub struct Config {
    /// One of the app's own files by its URL path (`/games/pong/index.html`).
    pub assets: Rc<dyn Fn(&str) -> Option<Vec<u8>>>,
    /// Where the browser keeps its profile: a directory of this app's own.
    pub user_data: PathBuf,
    /// The app's `location.origin`, which `src` values are written against.
    pub page_origin: String,
    /// May the page's devtools be opened?
    pub devtools: bool,
}

/// The origin the relay page and the app's files have INSIDE the webview.
/// `.invalid` never resolves (RFC 6761), so a request the handler misses fails
/// instead of reaching a network.
pub const APP_ORIGIN: &str = "https://app.vn.invalid";
/// The relay page's path under [`APP_ORIGIN`].
pub const RELAY_PATH: &str = "/__vn_iframe__/relay.html";

/// The platform's backend, or `None` where there is none.
pub fn backend(config: Config) -> Option<Rc<dyn vn_tree::iframe::Backend>> {
    #[cfg(windows)]
    {
        Some(win::WebView2::new(config))
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        None
    }
}

/// The URL the webview loads for an app-side `src`: the app's own origin and
/// origin-relative paths become [`APP_ORIGIN`]; anything else is itself.
pub fn internal_url(src: &str, page_origin: &str) -> String {
    let src = src.trim();
    if let Some(rest) = src.strip_prefix(page_origin) {
        if rest.is_empty() || rest.starts_with(['/', '?', '#']) {
            return format!("{APP_ORIGIN}{}{rest}", if rest.starts_with('/') { "" } else { "/" });
        }
    }
    // `//host/path` is another host; a scheme is another origin.
    if src.starts_with("//") {
        return format!("https:{src}");
    }
    let has_scheme = src
        .split_once(':')
        .is_some_and(|(s, _)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) && !s.contains('/'));
    if has_scheme {
        return src.to_string();
    }
    format!("{APP_ORIGIN}/{}", src.trim_start_matches('/'))
}

/// `targetOrigin` as the relay page should use it: `*` and `/` as they are,
/// the app's origin as the internal one.
pub fn internal_origin(target: &str, page_origin: &str) -> String {
    if target == page_origin { APP_ORIGIN.to_string() } else { target.to_string() }
}

/// An origin the document reported, as the app knows it.
pub fn external_origin(origin: &str, page_origin: &str) -> String {
    if origin == APP_ORIGIN { page_origin.to_string() } else { origin.to_string() }
}

/// The `Content-Type` of an app file, by its extension.
pub fn content_type(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
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
        assert_eq!(internal_url("app://redjs/games/pong/index.html", PAGE), "https://app.vn.invalid/games/pong/index.html");
        assert_eq!(internal_url("/games/pong/index.html?x=1#y", PAGE), "https://app.vn.invalid/games/pong/index.html?x=1#y");
        assert_eq!(internal_url("games/pong/index.html", PAGE), "https://app.vn.invalid/games/pong/index.html");
        assert_eq!(internal_url("app://redjs", PAGE), "https://app.vn.invalid/");
    }

    #[test]
    fn other_origins_are_left_alone() {
        assert_eq!(internal_url("https://games.example.com/a", PAGE), "https://games.example.com/a");
        assert_eq!(internal_url("//cdn.example.com/a", PAGE), "https://cdn.example.com/a");
        assert_eq!(internal_url("data:text/html,hi", PAGE), "data:text/html,hi");
        assert_eq!(internal_url("app://redjsx/evil", PAGE), "app://redjsx/evil", "a longer host is another origin");
    }

    #[test]
    fn origins_map_both_ways() {
        assert_eq!(internal_origin("*", PAGE), "*");
        assert_eq!(internal_origin("/", PAGE), "/");
        assert_eq!(internal_origin(PAGE, PAGE), APP_ORIGIN);
        assert_eq!(external_origin(APP_ORIGIN, PAGE), PAGE);
        assert_eq!(external_origin("null", PAGE), "null");
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type("/games/pong/index.html?v=2"), "text/html; charset=utf-8");
        assert_eq!(content_type("/a/b.WASM"), "application/wasm");
        assert_eq!(content_type("/noext"), "application/octet-stream");
    }
}
