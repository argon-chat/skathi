//! Desktop capture sources and bounded PNG previews, off the frame thread.
//!
//! IDs use `screen:<HMONITOR>` / `window:<HWND>` on Windows. The decimal
//! handle is a string throughout JS and is the same handle capture consumes;
//! display enumeration order and display names are not source identities.

use base64::Engine;
use image::ImageEncoder;
use serde_json::{json, Value};

use crate::{Done, Fail};

#[cfg(target_os = "windows")]
mod win;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub screens: bool,
    pub windows: bool,
    pub with_icons: bool,
    pub thumbnail_width: u32,
    pub thumbnail_height: u32,
}

impl Options {
    fn parse(types: &str, options: &str) -> Result<Self, Fail> {
        let types: Value = serde_json::from_str(types)
            .map_err(|_| Fail::type_error("getScreenSources: types must be an array"))?;
        let types = types
            .as_array()
            .ok_or_else(|| Fail::type_error("getScreenSources: types must be an array"))?;
        if types
            .iter()
            .any(|t| !matches!(t.as_str(), Some("screen" | "window")))
        {
            return Err(Fail::type_error(
                "getScreenSources: types may contain only screen and window",
            ));
        }
        let options: Value = if options.is_empty() || options == "undefined" {
            json!({})
        } else {
            serde_json::from_str(options)
                .map_err(|_| Fail::type_error("getScreenSources: invalid options"))?
        };
        if !options.is_object() {
            return Err(Fail::type_error(
                "getScreenSources: options must be an object",
            ));
        }
        let dimension = |name: &str, fallback| -> Result<u32, Fail> {
            match options.get("thumbnailSize").and_then(|size| size.get(name)) {
                None => Ok(fallback),
                Some(v) => v.as_u64().filter(|n| *n <= 1024).map(|n| n as u32)
                    .ok_or_else(|| Fail::type_error("getScreenSources: thumbnail dimensions must be integers from 0 to 1024")),
            }
        };
        if options.get("thumbnailSize").is_some_and(|v| !v.is_object()) {
            return Err(Fail::type_error(
                "getScreenSources: thumbnailSize must be an object",
            ));
        }
        let with_icons = match options.get("withIcons") {
            None => true,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| Fail::type_error("getScreenSources: withIcons must be boolean"))?,
        };
        Ok(Self {
            screens: types.iter().any(|t| t == "screen"),
            windows: types.iter().any(|t| t == "window"),
            with_icons,
            thumbnail_width: dimension("width", 320)?,
            thumbnail_height: dimension("height", 180)?,
        })
    }
}

/// Starts an asynchronous enumeration, answering the desktop token protocol.
pub fn start_sources(types: &str, options: &str) -> String {
    let options = match Options::parse(types, options) {
        Ok(options) => options,
        Err(error) => return error.to_sync_json(),
    };
    let token = crate::next_token();
    crate::pool::submit(token, move || {
        let sources = sources(options)?;
        Ok(Done::json(&Value::Array(sources).to_string()))
    });
    json!({"token": token}).to_string()
}

/// Enumeration usable by native callers too. Unsupported hosts reject rather
/// than pretending that desktop capture is supported with an empty list.
pub fn sources(options: Options) -> Result<Vec<Value>, Fail> {
    if !options.screens && !options.windows {
        return Ok(Vec::new());
    }
    #[cfg(target_os = "windows")]
    {
        win::sources(options)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(Fail::new(
            "NotSupportedError",
            "desktop capture sources are not implemented on this platform",
        ))
    }
}

fn fit(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return (0, 0);
    }
    let scale = (max_width as f64 / width as f64)
        .min(max_height as f64 / height as f64)
        .min(1.0);
    (
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    )
}

fn png_url(rgba: &[u8], width: u32, height: u32) -> Result<String, Fail> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| {
            Fail::new(
                "OperationError",
                format!("encoding capture thumbnail: {error}"),
            )
        })?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_types_and_bounded_previews() {
        for types in ["null", "{}", "[1]", "[\"camera\"]"] {
            assert!(Options::parse(types, "{}").is_err(), "{types}");
        }
        for options in [
            "null",
            "[]",
            "{\"withIcons\":1}",
            "{\"thumbnailSize\":1}",
            "{\"thumbnailSize\":{\"width\":1025}}",
            "{\"thumbnailSize\":{\"height\":-1}}",
        ] {
            assert!(Options::parse("[]", options).is_err(), "{options}");
        }
        let options = Options::parse("[\"window\"]", "{\"withIcons\":false}").unwrap();
        assert!(!options.screens && options.windows && !options.with_icons);
        assert_eq!(
            (options.thumbnail_width, options.thumbnail_height),
            (320, 180)
        );
        assert_eq!(fit(1920, 1080, 320, 180), (320, 180));
        assert_eq!(fit(100, 200, 320, 180), (90, 180));
        assert_eq!(fit(100, 50, 320, 180), (100, 50));
        assert_eq!(fit(100, 50, 0, 180), (0, 0));
    }

    #[test]
    fn encodes_real_png_and_settles_empty_requests_as_an_array() {
        let url = png_url(&[240, 20, 40, 255, 10, 200, 30, 255], 2, 1).unwrap();
        let data = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory(&data).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.as_raw(), &[240, 20, 40, 255, 10, 200, 30, 255]);
        let token = crate::tests::token_of(&start_sources("[]", "{}"));
        let got = crate::tests::wait_for_settles(&[token]);
        assert_eq!(got[&token], (true, "[]".into()));
    }
}
