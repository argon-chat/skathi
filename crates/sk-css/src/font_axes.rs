//! `font-stretch` / `font-width`, `font-style` and `font-variation-settings` values as text: what
//! the mapper, `@font-face` descriptors and `getComputedStyle` share.

/// A `font-stretch` keyword or percentage as a percentage; `None` when it is neither.
pub fn font_width_percent(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let keyword = match value.as_str() {
        "ultra-condensed" => 50.,
        "extra-condensed" => 62.5,
        "condensed" => 75.,
        "semi-condensed" => 87.5,
        "normal" => 100.,
        "semi-expanded" => 112.5,
        "expanded" => 125.,
        "extra-expanded" => 150.,
        "ultra-expanded" => 200.,
        _ => {
            let n: f32 = value.strip_suffix('%')?.trim().parse().ok()?;
            return (n.is_finite() && n >= 0.).then_some(n);
        }
    };
    Some(keyword)
}

/// `font-variation-settings`: `normal`, or a comma-separated list of a four-character quoted tag
/// and a number. `None` when it does not parse.
pub fn font_variations(value: &str) -> Option<gpui::FontVariations> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") {
        return Some(gpui::FontVariations::default());
    }
    let mut settings = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        let quote = item.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let rest = &item[1..];
        let end = rest.find(quote)?;
        let tag = &rest[..end];
        let number: f32 = rest[end + 1..].trim().parse().ok()?;
        let bytes: [u8; 4] = tag.as_bytes().try_into().ok()?;
        if !bytes.iter().all(|b| (0x20..=0x7e).contains(b)) || !number.is_finite() {
            return None;
        }
        settings.push((gpui::axis_tag(&bytes), number));
    }
    Some(gpui::FontVariations::new(settings))
}

/// The computed `font-stretch`: a percentage.
pub fn font_width_css(width: gpui::FontWidth) -> String {
    format!("{}%", number(width.0))
}

/// The computed `font-style`.
pub fn font_style_css(style: gpui::FontStyle, angle: gpui::ObliqueAngle) -> String {
    match style {
        gpui::FontStyle::Normal => "normal".into(),
        gpui::FontStyle::Italic => "italic".into(),
        gpui::FontStyle::Oblique if angle == gpui::ObliqueAngle::DEFAULT => "oblique".into(),
        gpui::FontStyle::Oblique => format!("oblique {}deg", number(angle.0)),
    }
}

/// The computed `font-variation-settings`, in tag order.
pub fn font_variations_css(variations: &gpui::FontVariations) -> String {
    if variations.is_empty() {
        return "normal".into();
    }
    variations
        .settings()
        .iter()
        .map(|(tag, value)| {
            format!(
                "\"{}\" {}",
                String::from_utf8_lossy(&tag.to_be_bytes()),
                number(*value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn number(value: f32) -> String {
    ((value * 1000.).round() / 1000.).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variation_settings_parse_and_print() {
        let parsed = font_variations("'wdth' 80, \"GRAD\" -50.5, \"wdth\" 90").unwrap();
        assert_eq!(font_variations_css(&parsed), "\"GRAD\" -50.5, \"wdth\" 90");
        assert!(font_variations("\"wdt\" 80").is_none());
        assert!(font_variations("\"wdth\"").is_none());
        assert!(font_variations("normal").unwrap().is_empty());
    }
}
