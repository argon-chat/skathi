//! `<img src>` — the browser's URL grammar, resolved against a native base.
//!
//! # What a `src` may be, in the order this module tries
//!
//! | form | example | becomes |
//! |---|---|---|
//! | data URL | `data:image/png;base64,iVBOR…` | `ImageSource::Image` (bytes decoded once, cached per node) |
//! | http(s) URL | `https://host/a.png` | `Resource::Uri` — fetched by gpui's asset loader through the `ReqwestClient` `vn-app` installs |
//! | `file://` URL | `file:///C:/x/a.png` | `Resource::Path` |
//! | absolute path | `C:\x\a.png`, `C:/x/a.png` | `Resource::Path` |
//! | root-relative | `/assets/a.png` | `Resource::Path` under the **app root** |
//! | relative | `assets/a.png`, `./a.png`, `../a.png` | `Resource::Path` joined onto the **app root** |
//!
//! # The base — what "the document" is here
//!
//! A browser resolves a relative `src` against the **document's** URL. Our
//! document is the app: the dev host knows the entry module from
//! `dist/manifest.json` (`apps/imgperf/src/main.js`), and the base is that
//! app's ROOT directory — `apps/imgperf/` — i.e. the entry's directory with a
//! trailing `src/` stripped. That is deliberately the same place a Vite app
//! puts `index.html`, so `src="assets/x.png"` means `apps/<app>/assets/x.png`
//! in both worlds, and a leading `/` behaves like a web server root (the app
//! root), not like the filesystem root.
//!
//! The host publishes it with [`set_asset_base`]; `VN_APP_ROOT` overrides.
//! With no base set at all, relative paths resolve against the process working
//! directory (and say so once).
//!
//! # Caching
//!
//! Resolution runs on the render walk, i.e. per node per frame. Everything
//! expensive — path normalisation, base64 decoding — is therefore memoised per
//! node and only redone when the node's `src` string actually changes. gpui's
//! own asset cache (keyed by the `Resource` / `Image` id) is what dedupes the
//! *decode* across nodes that share a source.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use gpui::{Image, ImageFormat, ImageSource, Resource};

thread_local! {
    /// The app root relative `src`s resolve against (see module docs).
    static BASE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Per-node memo: the `src` string it was built from → the resolved source.
    /// Keyed by node id and GC'd with the rest of the per-node side state.
    static RESOLVED: RefCell<HashMap<u32, (String, Option<ImageSource>)>> =
        RefCell::new(HashMap::new());
}

/// Publishes the directory relative `src`s resolve against. The host calls this
/// once at boot (dev: derived from the manifest entry; `VN_APP_ROOT` wins).
pub fn set_asset_base(dir: PathBuf) {
    BASE.with(|b| *b.borrow_mut() = Some(dir));
}

/// The current base, or `None` when the host never published one.
pub fn asset_base() -> Option<PathBuf> {
    BASE.with(|b| b.borrow().clone())
}

/// Drops memoised sources for nodes that no longer exist.
pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    RESOLVED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// The resolved `ImageSource` for `id`'s current `src`, memoised.
///
/// `None` means the `src` could not be turned into anything loadable at all
/// (empty, or a URL scheme we do not speak) — the caller renders `alt` instead,
/// exactly as a browser does for a broken image.
pub(crate) fn source_for(id: u32, src: &str) -> Option<ImageSource> {
    RESOLVED.with(|m| {
        let mut map = m.borrow_mut();
        if let Some((cached_src, source)) = map.get(&id) {
            if cached_src == src {
                return source.clone();
            }
        }
        let source = resolve(src);
        map.insert(id, (src.to_string(), source.clone()));
        source
    })
}

/// The raw FILE bytes behind a `src`, for elements that need the document
/// rather than a decoded image — `<lottie>` parses Bodymovin JSON (or a gzipped
/// `.tgs`) itself, so `ImageSource` is no use to it.
///
/// The URL grammar is [`resolve`]'s, verbatim: `data:` URIs, `file://` URLs,
/// absolute Windows paths and relative/root-relative paths against the app root
/// all mean here exactly what they mean for `<img src>`. Only the *result* type
/// differs, so `<lottie src>` and `<img src>` can never drift apart.
///
/// `Err` carries the advisory text the caller should route through its compat
/// channel — nothing is printed here, because the caller knows the element name.
pub(crate) fn read_bytes(tag: &str, src: &str) -> Result<Vec<u8>, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err(format!("<{tag} src=\"\">: the src is empty — nothing to load"));
    }
    if scheme_of(src) == "data" {
        return data_bytes(tag, src);
    }
    let path = resolve_path(tag, src)?;
    std::fs::read(&path).map_err(|e| {
        format!(
            "<{tag} src=\"{src}\">: could not read {} — {e}",
            path.display()
        )
    })
}

/// The FILE PATH behind a `src`, for elements that hand the *file* to something
/// else rather than reading it — `<video>` gives Media Foundation a URL and lets
/// the OS own the read-ahead buffer, which is the whole point (a 30-minute clip
/// must not become a 2 GB `Vec<u8>`).
///
/// The grammar is [`resolve`]'s minus `data:`, verbatim: `file://` URLs,
/// absolute Windows paths and relative/root-relative paths against the app root
/// all mean here exactly what they mean for `<img src>`, so the three elements
/// can never drift apart.
///
/// `Err` carries the advisory text the caller should route through its compat
/// channel — nothing is printed here, because the caller knows the element name.
pub(crate) fn resolve_path(tag: &str, src: &str) -> Result<PathBuf, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err(format!("<{tag} src=\"\">: the src is empty — nothing to load"));
    }
    match scheme_of(src).as_str() {
        "data" => Err(format!(
            "<{tag} src=\"data:…\">: this element loads by PATH, and a data: URI has none. \
             (`<video>` takes them — see `resolve_media`.)"
        )),
        "http" | "https" => Err(format!(
            "<{tag} src=\"{src}\">: this element loads by PATH, and an http(s) URL is not one. \
             (`<img>` and `<video>` take them — see `resolve` and `resolve_media`.)"
        )),
        "file" => {
            file_url(src).ok_or_else(|| format!("<{tag} src=\"{src}\">: malformed file: URL"))
        }
        // A single-letter "scheme" is a Windows drive, never a URL scheme.
        s if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()) => Ok(PathBuf::from(src)),
        "" => Ok(relative_to_base(src)),
        other => Err(format!(
            "<{tag} src=\"{src}\">: the `{other}:` URL scheme is not supported — use a file \
             path or a data: URI."
        )),
    }
}

/// The [`vn_video::MediaSource`] behind a `<video src>` — the same grammar as
/// `<img>`, resolved to what a media pipeline can take rather than to an image.
///
/// Three shapes, because the platform genuinely wants three different things
/// and flattening them would cost something real each time:
///
/// * **a path**, so the OS owns the read-ahead buffer and a 30-minute clip
///   never becomes a `Vec<u8>`;
/// * **an http(s) URL verbatim**, so the platform's own network source streams
///   it. Downloading first would turn "starts in 200 ms" into "starts when the
///   whole file has arrived", which is not what a browser does either;
/// * **bytes**, for `data:`, which has neither a path nor a server.
///
/// `Err` carries the advisory text the caller routes through its compat
/// channel — nothing is printed here, because the caller knows the element name.
pub(crate) fn resolve_media(tag: &str, src: &str) -> Result<vn_video::MediaSource, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err(format!("<{tag} src=\"\">: the src is empty — nothing to load"));
    }
    match scheme_of(src).as_str() {
        "data" => {
            // The media type is the only hint the platform gets about what the
            // bytes are: a `data:` URI has no filename and no `Content-Type`.
            let mime = src
                .get(5..)
                .unwrap_or_default()
                .split_once(',')
                .map(|(meta, _)| {
                    meta.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
                })
                .unwrap_or_default();
            let data = data_bytes(tag, src)?;
            if data.is_empty() {
                return Err(format!(
                    "<{tag} src=\"{}…\">: the data: URI decoded to zero bytes",
                    head(src)
                ));
            }
            Ok(vn_video::MediaSource::Bytes {
                mime,
                data: std::sync::Arc::new(data),
            })
        }
        "http" | "https" => Ok(vn_video::MediaSource::Uri(src.to_string())),
        _ => resolve_path(tag, src).map(vn_video::MediaSource::Path),
    }
}

fn scheme_of(src: &str) -> String {
    src.split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default()
}

/// `data:` URI → payload bytes. The media type is *not* checked: unlike `<img>`,
/// which needs it to pick a decoder, a `<lottie>` payload is sniffed by the
/// crate itself (gzip magic → `.tgs`, otherwise JSON).
fn data_bytes(tag: &str, src: &str) -> Result<Vec<u8>, String> {
    let body = src.get(5..).unwrap_or_default(); // after "data:"
    let Some((meta, payload)) = body.split_once(',') else {
        return Err(format!(
            "<{tag} src=\"{}…\">: malformed data: URL (no comma)",
            head(src)
        ));
    };
    if meta.to_ascii_lowercase().split(';').any(|p| p.trim() == "base64") {
        base64_decode(payload).ok_or_else(|| {
            format!(
                "<{tag} src=\"{}…\">: data: URL payload is not valid base64",
                head(src)
            )
        })
    } else {
        Ok(percent_decode(payload).into_bytes())
    }
}

/// Advisories are one-shot per message, like the renderer's.
fn warn(msg: String) {
    thread_local! {
        static SEEN: RefCell<std::collections::HashSet<String>> =
            RefCell::new(std::collections::HashSet::new());
    }
    SEEN.with(|s| {
        if s.borrow_mut().insert(msg.clone()) {
            eprintln!("[vue-native compat] {msg}");
            crate::log::record("advisory", msg);
        }
    });
}

fn resolve(src: &str) -> Option<ImageSource> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    let lower_scheme = src
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();

    match lower_scheme.as_str() {
        "data" => data_url(src),
        // gpui's asset loader fetches a `Resource::Uri` through
        // `cx.http_client()`, which `vn-app` now installs as a real
        // `ReqwestClient`. The fetch is ASYNCHRONOUS: the first build gets
        // `None` back from `use_asset`, gpui spawns the request and notifies
        // the view when the bytes land, so the element shows its `alt` for a
        // frame or two and then the picture — which is what a browser does.
        //
        // NOT `ImageSource::from(&str)`: gpui's `is_uri` classifies a bare
        // relative path as `Embedded`, so the scheme is decided here.
        "http" | "https" => Some(ImageSource::Resource(Resource::Uri(src.to_string().into()))),
        "file" => file_url(src).map(path_source),
        // A single-letter "scheme" is a Windows drive (`C:\x\a.png`), never a
        // URL scheme — no registered scheme is one character long.
        s if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()) => {
            Some(path_source(PathBuf::from(src)))
        }
        "" => Some(path_source(relative_to_base(src))),
        other => {
            warn(format!(
                "<img src=\"{src}\">: the `{other}:` URL scheme is not supported — use a file \
                 path, a data: URI, or (once an HTTP client is wired) http(s)."
            ));
            None
        }
    }
}

fn path_source(p: PathBuf) -> ImageSource {
    // Explicitly `Resource::Path`: `impl From<&str> for ImageSource` would
    // route a Windows path through `url::Url::parse`, where `C:` parses as a
    // scheme and the file becomes an unfetchable URI.
    ImageSource::Resource(Resource::Path(p.into()))
}

/// Joins a relative or root-relative `src` onto the app root and folds away
/// `.` / `..` lexically (the paths are URLs, not filesystem walks — a browser
/// resolves them purely textually too).
fn relative_to_base(src: &str) -> PathBuf {
    let base = match asset_base() {
        Some(b) => b,
        None => {
            warn(
                "<img src>: no app root is published, so relative image paths resolve against the \
                 process working directory. Set VN_APP_ROOT, or run the dev host (which derives \
                 it from the manifest entry)."
                    .into(),
            );
            std::env::current_dir().unwrap_or_default()
        }
    };
    // A leading `/` is a *server* root in a browser; our server root is the app.
    let rel = src.trim_start_matches(['/', '\\']);
    normalize(&base.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `file:///C:/x/a.png` → `C:\x\a.png`. Percent escapes are decoded; a host
/// component (`file://server/share`) becomes a UNC path.
fn file_url(src: &str) -> Option<PathBuf> {
    let rest = src.get(5..)?; // after "file:"
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let decoded = percent_decode(rest);
    let decoded = decoded.replace('/', "\\");
    // "///C:/x" → "\C:\x": drop the empty authority's slash before a drive.
    let trimmed = decoded.strip_prefix('\\').unwrap_or(&decoded);
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' {
        return Some(PathBuf::from(trimmed));
    }
    Some(PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// data: URLs
// ---------------------------------------------------------------------------

/// `data:[<mediatype>][;base64],<data>` (RFC 2397). The media type decides the
/// decoder, defaulting — as the RFC does — to nothing useful for images, so an
/// absent one is sniffed from the payload instead of guessed.
fn data_url(src: &str) -> Option<ImageSource> {
    let body = src.get(5..)?; // after "data:"
    let (meta, payload) = match body.split_once(',') {
        Some(pair) => pair,
        None => {
            warn(format!("<img src=\"{}…\">: malformed data: URL (no comma)", head(src)));
            return None;
        }
    };
    let meta_lower = meta.to_ascii_lowercase();
    let is_base64 = meta_lower.split(';').any(|p| p.trim() == "base64");
    let mime = meta_lower.split(';').next().unwrap_or("").trim().to_string();

    let bytes = if is_base64 {
        match base64_decode(payload) {
            Some(b) => b,
            None => {
                warn(format!(
                    "<img src=\"{}…\">: data: URL payload is not valid base64",
                    head(src)
                ));
                return None;
            }
        }
    } else {
        percent_decode(payload).into_bytes()
    };

    let format = image_format(&mime).or_else(|| sniff(&bytes));
    let Some(format) = format else {
        warn(format!(
            "<img src=\"{}…\">: unsupported data: URL media type {mime:?} — png, jpeg, webp, gif, \
             bmp, tiff, ico and svg+xml decode",
            head(src)
        ));
        return None;
    };
    Some(ImageSource::Image(std::sync::Arc::new(Image::from_bytes(
        format, bytes,
    ))))
}

/// A data: URL IS its content — never echo the whole thing into the log.
fn head(src: &str) -> String {
    src.chars().take(40).collect()
}

fn image_format(mime: &str) -> Option<ImageFormat> {
    Some(match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::Webp,
        "image/gif" => ImageFormat::Gif,
        "image/svg+xml" => ImageFormat::Svg,
        "image/bmp" | "image/x-bmp" => ImageFormat::Bmp,
        "image/tiff" | "image/tif" => ImageFormat::Tiff,
        "image/x-icon" | "image/vnd.microsoft.icon" | "image/ico" => ImageFormat::Ico,
        _ => return None,
    })
}

/// Magic-number sniff, for `data:,<bytes>` with no media type.
fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    let b = bytes;
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(ImageFormat::Png)
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if b.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if b.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else if b.starts_with(b"<svg") || b.starts_with(b"<?xml") {
        Some(ImageFormat::Svg)
    } else {
        None
    }
}

/// Standard base64 with optional padding; whitespace (data URLs get wrapped by
/// every editor on earth) is skipped. URL-safe `-_` is accepted too.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' | 0x0c => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_path(s: Option<ImageSource>) -> Option<PathBuf> {
        match s {
            Some(ImageSource::Resource(Resource::Path(p))) => Some(p.to_path_buf()),
            _ => None,
        }
    }

    #[test]
    fn relative_paths_resolve_against_the_app_root() {
        set_asset_base(PathBuf::from("Y:\\vue-native\\apps\\imgperf"));
        for src in ["assets/a.png", "./assets/a.png", "/assets/a.png"] {
            assert_eq!(
                as_path(resolve(src)),
                Some(PathBuf::from("Y:\\vue-native\\apps\\imgperf\\assets\\a.png")),
                "{src}"
            );
        }
        // `..` folds lexically, like a URL.
        assert_eq!(
            as_path(resolve("../hello/assets/a.png")),
            Some(PathBuf::from("Y:\\vue-native\\apps\\hello\\assets\\a.png"))
        );
    }

    #[test]
    fn absolute_windows_paths_are_paths_not_uris() {
        set_asset_base(PathBuf::from("Y:\\vue-native\\apps\\imgperf"));
        assert_eq!(
            as_path(resolve("C:/pics/a.png")),
            Some(PathBuf::from("C:/pics/a.png"))
        );
        assert_eq!(
            as_path(resolve("C:\\pics\\a.png")),
            Some(PathBuf::from("C:\\pics\\a.png"))
        );
    }

    #[test]
    fn file_urls_decode_to_paths() {
        assert_eq!(
            as_path(resolve("file:///C:/my%20pics/a.png")),
            Some(PathBuf::from("C:\\my pics\\a.png"))
        );
    }

    #[test]
    fn http_becomes_a_uri_resource_for_gpuis_asset_loader() {
        // Was `is_none()` + an advisory until m13 wired a real HTTP client into
        // gpui. `Resource::Uri` is what makes gpui fetch it.
        assert!(matches!(
            resolve("https://example.com/a.png"),
            Some(ImageSource::Resource(Resource::Uri(_)))
        ));
        assert!(matches!(
            resolve("http://example.com/a.png"),
            Some(ImageSource::Resource(Resource::Uri(_)))
        ));
        // …and a bare relative path still resolves against the app root rather
        // than being mistaken for a URL, which is the trap `ImageSource::from`
        // walks into.
        assert!(matches!(
            resolve("assets/a.png"),
            Some(ImageSource::Resource(Resource::Path(_)))
        ));
    }

    /// `<video>`'s grammar is `<img>`'s, resolved to a media source. The three
    /// shapes are not interchangeable: a path lets the OS own the read-ahead,
    /// a URL lets its network source stream, and bytes are for `data:`, which
    /// has neither.
    #[test]
    fn video_sources_resolve_to_the_shape_the_platform_wants() {
        use vn_video::MediaSource;
        assert!(matches!(
            resolve_media("video", "https://example.com/a.mp4"),
            Ok(MediaSource::Uri(u)) if u == "https://example.com/a.mp4"
        ));
        // base64, with the media type carried through — it is the only hint
        // the platform gets about what the bytes are.
        match resolve_media("video", "data:video/mp4;base64,AAAA") {
            Ok(MediaSource::Bytes { mime, data }) => {
                assert_eq!(mime, "video/mp4");
                assert_eq!(&*data, &[0u8, 0, 0]);
            }
            other => panic!("{other:?}"),
        }
        // A URI that decodes to nothing is an error, not a zero-byte load that
        // fails four layers down inside Media Foundation.
        assert!(resolve_media("video", "data:video/mp4;base64,").is_err());
        assert!(resolve_media("video", "").is_err());
        // Everything else is still a path.
        assert!(matches!(
            resolve_media("video", "assets/clip.mp4"),
            Ok(MediaSource::Path(_))
        ));
    }

    #[test]
    fn data_urls_decode_and_sniff() {
        // 1x1 transparent PNG.
        const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
        let Some(ImageSource::Image(img)) = resolve(PNG) else {
            panic!("data: URL did not resolve to an image");
        };
        assert_eq!(img.format, ImageFormat::Png);
        assert!(img.bytes.starts_with(&[0x89, b'P', b'N', b'G']));

        // No media type at all: sniffed from the bytes.
        let sniffed = resolve("data:;base64,iVBORw0KGgoAAAANSUhEUg==");
        assert!(matches!(sniffed, Some(ImageSource::Image(_))));

        // Percent-encoded (non-base64) SVG, the other common spelling.
        let svg = resolve("data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A//www.w3.org/2000/svg%22/%3E");
        let Some(ImageSource::Image(img)) = svg else {
            panic!("percent-encoded data: URL did not resolve");
        };
        assert_eq!(img.format, ImageFormat::Svg);
        assert!(String::from_utf8_lossy(&img.bytes).starts_with("<svg"));
    }

    #[test]
    fn empty_and_unknown_schemes_yield_nothing() {
        assert!(resolve("").is_none());
        assert!(resolve("   ").is_none());
        assert!(resolve("gopher://x/a.png").is_none());
    }

    #[test]
    fn base64_ignores_wrapping_whitespace() {
        assert_eq!(base64_decode("QUJD").unwrap(), b"ABC");
        assert_eq!(base64_decode("QU\n JD\r\n").unwrap(), b"ABC");
        assert_eq!(base64_decode("QQ==").unwrap(), b"A");
        assert!(base64_decode("**").is_none());
    }

    /// The memo must follow the `src`, not the node id — Vue reuses ids.
    #[test]
    fn the_per_node_memo_follows_the_src_string() {
        set_asset_base(PathBuf::from("Y:\\base"));
        assert_eq!(
            as_path(source_for(7, "a.png")),
            Some(PathBuf::from("Y:\\base\\a.png"))
        );
        assert_eq!(
            as_path(source_for(7, "b.png")),
            Some(PathBuf::from("Y:\\base\\b.png"))
        );
    }
}
