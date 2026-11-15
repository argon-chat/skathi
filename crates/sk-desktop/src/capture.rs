//! Desktop capture sources and bounded PNG previews, off the frame thread. IDs are `screen:<handle>` /
//! `window:<handle>`, decimal strings in JS. Under `SK_MEDIA_CAPTURE=fake` the list is one synthetic screen.

use base64::Engine;
use image::ImageEncoder;
use serde_json::{json, Value};

use crate::{Done, Fail};

#[cfg(any(target_os = "macos", test))]
mod listing;
#[cfg(target_os = "macos")]
mod mac;
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

pub fn start_sources(types: &str, options: &str) -> Result<u32, Fail> {
    let options = Options::parse(types, options)?;
    let token = crate::next_token();
    let enumerate = move || {
        crate::pool::submit(token, move || {
            let sources = sources(options)?;
            Ok(Done::json(&Value::Array(sources).to_string()))
        });
    };
    // The Screen Recording prompt can wait on the user: its own thread, since a pool job would count as in flight.
    #[cfg(target_os = "macos")]
    if options.screens || options.windows {
        let asked = std::thread::Builder::new()
            .name("sk-capture-consent".into())
            .spawn(move || match sk_permissions::require_display_capture() {
                Ok(()) => enumerate(),
                Err((name, message)) => crate::settle(token, Err(Fail::new(name, message))),
            });
        if asked.is_err() {
            crate::settle(
                token,
                Err(Fail::new(
                    "OperationError",
                    "could not start the screen recording check",
                )),
            );
        }
        return Ok(token);
    }
    enumerate();
    Ok(token)
}

pub fn sources(options: Options) -> Result<Vec<Value>, Fail> {
    if !options.screens && !options.windows {
        return Ok(Vec::new());
    }
    if fake_capture() {
        return fake_sources(options);
    }
    // Names and thumbnails count as screen recording to TCC: ask first, never on the frame thread.
    #[cfg(target_os = "macos")]
    sk_permissions::require_display_capture()
        .map_err(|(name, message)| Fail::new(name, message))?;
    #[cfg(target_os = "windows")]
    {
        refuse_denied(sk_permissions::status(sk_permissions::Cap::DisplayCapture))?;
        win::sources(options)
    }
    #[cfg(target_os = "macos")]
    {
        mac::sources(options)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err(Fail::new(
            "NotSupportedError",
            "desktop capture sources are not implemented on this platform",
        ))
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn refuse_denied(state: sk_permissions::State) -> Result<(), Fail> {
    if state != sk_permissions::State::Denied {
        return Ok(());
    }
    let (name, message) = sk_permissions::display_capture_refusal();
    Err(Fail::new(name, message))
}

fn fake_capture() -> bool {
    std::env::var("SK_MEDIA_CAPTURE").is_ok_and(|v| v.trim().eq_ignore_ascii_case("fake"))
}

fn fake_sources(options: Options) -> Result<Vec<Value>, Fail> {
    if !options.screens {
        return Ok(Vec::new());
    }
    let (sw, sh) = (1920u32, 1080u32);
    let (w, h) = fit(sw, sh, options.thumbnail_width, options.thumbnail_height);
    let thumbnail = if w == 0 || h == 0 {
        String::new()
    } else {
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x * sw / w, y * sh / h);
                let v = if (fx / 40 + fy / 40) % 2 == 0 {
                    220
                } else {
                    40
                };
                let o = ((y * w + x) * 4) as usize;
                rgba[o..o + 4].copy_from_slice(&[(fy % 256) as u8, (fx % 256) as u8, v, 255]);
            }
        }
        png_url(&rgba, w, h)?
    };
    let bounds = json!({"x": 0, "y": 0, "width": sw, "height": sh});
    Ok(vec![json!({
        "id": "screen:1",
        "name": "Screen 1",
        "kind": "screen",
        "displayId": "1",
        "deviceName": "fake",
        "bounds": bounds,
        "workArea": bounds,
        "width": sw, "height": sh,
        "isPrimary": true,
        "thumbnailDataUrl": thumbnail, "thumbnailError": Value::Null,
        "appIconDataUrl": Value::Null,
    })])
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
    fn a_fake_capture_lists_one_synthetic_screen_and_no_windows() {
        let all = Options::parse("[\"screen\",\"window\"]", "{}").unwrap();
        let sources = fake_sources(all).unwrap();
        assert_eq!(sources.len(), 1, "no window of the machine's");
        let screen = &sources[0];
        assert_eq!(screen["id"], "screen:1");
        assert_eq!(screen["kind"], "screen");
        let url = screen["thumbnailDataUrl"].as_str().unwrap();
        let data = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory(&data).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (320, 180));
        assert_ne!(
            image.get_pixel(0, 0),
            image.get_pixel(12, 0),
            "the fake frame's checkerboard"
        );
        let windows = Options::parse("[\"window\"]", "{}").unwrap();
        assert!(fake_sources(windows).unwrap().is_empty());
        let none = Options::parse(
            "[\"screen\"]",
            "{\"thumbnailSize\":{\"width\":0,\"height\":0}}",
        )
        .unwrap();
        assert_eq!(fake_sources(none).unwrap()[0]["thumbnailDataUrl"], "");
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
        let token = start_sources("[]", "{}").unwrap();
        let got = crate::tests::wait_for_settles(&[token]);
        assert_eq!(got[&token], (true, "[]".into()));
    }
}
