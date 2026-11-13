//! Bodymovin → rasterlottie compatibility pass, run over the raw JSON before the backend parses it.
//!
//! [`rasterlottie`] refuses to `prepare()` anything it cannot draw exactly, and two ordinary
//! Bodymovin idioms trip it:
//!
//! 1. Terminator keyframes: Bodymovin ≥ 5.5 ends every animated property with `{"t": 42}` and no
//!    `"s"`, whose implicit value is the previous keyframe's `"e"` (or its own `"s"` for a hold).
//!    rasterlottie reads `"s"` unconditionally, so one terminator makes the whole animation
//!    unsupported (6 of 8 real-world fixtures failed on this alone).
//! 2. Solid layers: a `ty: 1` layer is a `sw × sh` rectangle of colour `sc`; rasterlottie has no
//!    solid renderer, so it is rewritten into the equivalent shape layer.
//!
//! Both rewrites are value-preserving and idempotent. Anything else is left alone and surfaces as a
//! [`SupportReport`](rasterlottie::SupportReport) issue, which
//! [`LottieState::load`](crate::LottieState::load) turns into a compat advisory.

use serde_json::{json, Map, Value};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Normalization {
    /// Terminator keyframes (`{"t": n}` with no `"s"`) given their implicit value.
    pub terminator_keyframes: usize,
    /// `ty: 1` solid layers rewritten into equivalent shape layers.
    pub solid_layers: usize,
}

impl Normalization {
    pub fn is_empty(&self) -> bool {
        self.terminator_keyframes == 0 && self.solid_layers == 0
    }
}

pub fn normalize(root: &mut Value) -> Normalization {
    let mut stats = Normalization::default();
    walk(root, &mut stats);
    stats
}

fn walk(value: &mut Value, stats: &mut Normalization) {
    match value {
        Value::Object(map) => {
            fill_terminator_keyframes(map, stats);
            rewrite_solid_layer(map, stats);
            for (_, child) in map.iter_mut() {
                walk(child, stats);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, stats);
            }
        }
        _ => {}
    }
}

/// Gives every keyframe that omits `"s"` the value the previous keyframe ends on (as lottie-web
/// resolves it). Only touches `"k"` keyframe lists (first entry an object with a `"t"`); static
/// number arrays and scalars are skipped.
fn fill_terminator_keyframes(map: &mut Map<String, Value>, stats: &mut Normalization) {
    let Some(Value::Array(keyframes)) = map.get_mut("k") else {
        return;
    };
    if !keyframes
        .first()
        .is_some_and(|first| first.get("t").is_some())
    {
        return;
    }

    // The value the timeline is at when the previous keyframe ends: its `"e"`, else its own `"s"` (a hold).
    let mut carried: Option<Value> = None;
    for keyframe in keyframes.iter_mut() {
        let Some(keyframe) = keyframe.as_object_mut() else {
            continue;
        };
        if !keyframe.contains_key("s") {
            if let Some(value) = carried.clone() {
                keyframe.insert("s".to_string(), value);
                stats.terminator_keyframes += 1;
            }
        }
        carried = keyframe
            .get("e")
            .or_else(|| keyframe.get("s"))
            .cloned();
    }
}

/// Rewrites a solid layer into the equivalent shape layer: a `sw × sh` rectangle centred on the
/// layer origin, filled with `sc`. Only the content changes; transform, timing, parenting and
/// mattes are untouched.
fn rewrite_solid_layer(map: &mut Map<String, Value>, stats: &mut Normalization) {
    // Layer `ty` is a number and a shape item's is a string, so they cannot collide.
    if map.get("ty").and_then(Value::as_u64) != Some(1) {
        return;
    }
    let (Some(width), Some(height)) = (
        map.get("sw").and_then(Value::as_f64),
        map.get("sh").and_then(Value::as_f64),
    ) else {
        return;
    };
    let [red, green, blue] = map
        .get("sc")
        .and_then(Value::as_str)
        .and_then(parse_hex_color)
        .unwrap_or([0.0, 0.0, 0.0]);

    // Lottie places solid content top-left at the layer origin, so the rectangle is centred at (sw/2, sh/2).
    let group = json!({
        "ty": "gr",
        "nm": "vn-lottie:solid",
        "it": [
            {
                "ty": "rc",
                "d": 1,
                "s": { "a": 0, "k": [width, height] },
                "p": { "a": 0, "k": [width / 2.0, height / 2.0] },
                "r": { "a": 0, "k": 0 },
            },
            {
                "ty": "fl",
                "r": 1,
                "c": { "a": 0, "k": [red, green, blue, 1.0] },
                "o": { "a": 0, "k": 100 },
            },
            {
                "ty": "tr",
                "p": { "a": 0, "k": [0.0, 0.0] },
                "a": { "a": 0, "k": [0.0, 0.0] },
                "s": { "a": 0, "k": [100.0, 100.0] },
                "r": { "a": 0, "k": 0 },
                "o": { "a": 0, "k": 100 },
            },
        ],
    });

    map.insert("ty".to_string(), json!(4));
    map.insert("shapes".to_string(), json!([group]));
    map.remove("sw");
    map.remove("sh");
    map.remove("sc");
    stats.solid_layers += 1;
}

/// `#rrggbb` (or `#rgb`) → unit floats, the encoding Lottie uses for fill colours.
fn parse_hex_color(text: &str) -> Option<[f64; 3]> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    let bytes = match digits.len() {
        6 => [
            u8::from_str_radix(&digits[0..2], 16).ok()?,
            u8::from_str_radix(&digits[2..4], 16).ok()?,
            u8::from_str_radix(&digits[4..6], 16).ok()?,
        ],
        3 => {
            let nibble = |index: usize| -> Option<u8> {
                let value = u8::from_str_radix(&digits[index..index + 1], 16).ok()?;
                Some(value * 17)
            };
            [nibble(0)?, nibble(1)?, nibble(2)?]
        }
        _ => return None,
    };
    Some([
        f64::from(bytes[0]) / 255.0,
        f64::from(bytes[1]) / 255.0,
        f64::from(bytes[2]) / 255.0,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_trailing_terminator_keyframe() {
        let mut document = json!({
            "position": { "a": 1, "k": [
                { "t": 0, "s": [0.0, 0.0], "e": [10.0, 5.0] },
                { "t": 30 },
            ] },
        });
        let stats = normalize(&mut document);
        assert_eq!(stats.terminator_keyframes, 1);
        assert_eq!(document["position"]["k"][1]["s"], json!([10.0, 5.0]));
    }

    #[test]
    fn a_hold_terminator_carries_the_held_value() {
        let mut document = json!({
            "opacity": { "a": 1, "k": [
                { "t": 0, "s": [100.0], "h": 1 },
                { "t": 30 },
            ] },
        });
        normalize(&mut document);
        assert_eq!(document["opacity"]["k"][1]["s"], json!([100.0]));
    }

    #[test]
    fn static_properties_are_left_alone() {
        let mut document = json!({ "size": { "a": 0, "k": [64.0, 64.0] } });
        let before = document.clone();
        assert_eq!(normalize(&mut document), Normalization::default());
        assert_eq!(document, before);
    }

    #[test]
    fn normalisation_is_idempotent() {
        let mut once = json!({
            "layers": [{
                "ty": 1, "sw": 100, "sh": 50, "sc": "#ff8000",
                "ks": { "o": { "a": 1, "k": [{ "t": 0, "s": [0], "e": [100] }, { "t": 10 }] } },
            }],
        });
        normalize(&mut once);
        let mut twice = once.clone();
        assert_eq!(normalize(&mut twice), Normalization::default());
        assert_eq!(once, twice);
    }

    #[test]
    fn solid_layers_become_filled_rectangles() {
        let mut document = json!({
            "layers": [{ "ty": 1, "sw": 100, "sh": 50, "sc": "#ff0000", "ks": {} }],
        });
        let stats = normalize(&mut document);
        assert_eq!(stats.solid_layers, 1);

        let layer = &document["layers"][0];
        assert_eq!(layer["ty"], json!(4));
        assert!(layer.get("sw").is_none(), "solid fields are consumed");

        let items = &layer["shapes"][0]["it"];
        assert_eq!(items[0]["ty"], json!("rc"));
        assert_eq!(items[0]["s"]["k"], json!([100.0, 50.0]));
        assert_eq!(items[0]["p"]["k"], json!([50.0, 25.0]));
        assert_eq!(items[1]["ty"], json!("fl"));
        assert_eq!(items[1]["c"]["k"], json!([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(items[2]["ty"], json!("tr"));
    }

    #[test]
    fn shape_items_are_not_mistaken_for_solid_layers() {
        let mut document = json!({ "it": [{ "ty": "rc", "sw": 1, "sh": 1 }] });
        assert_eq!(normalize(&mut document).solid_layers, 0);
    }

    #[test]
    fn parses_both_hex_colour_widths() {
        assert_eq!(parse_hex_color("#ffffff"), Some([1.0, 1.0, 1.0]));
        assert_eq!(parse_hex_color("#000"), Some([0.0, 0.0, 0.0]));
        assert_eq!(parse_hex_color("#fff"), Some([1.0, 1.0, 1.0]));
        assert_eq!(parse_hex_color("nope"), None);
    }
}
