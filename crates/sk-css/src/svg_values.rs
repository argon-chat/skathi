use super::*;
use lightningcss::traits::Parse;
use lightningcss::values::alpha::AlphaValue;

pub(super) fn property(name: &str) -> Option<&'static str> {
    [
        "x",
        "y",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "stop-color",
        "stop-opacity",
    ]
    .into_iter()
    .find(|p| name.eq_ignore_ascii_case(p))
}

pub(super) fn computed(name: &str, value: &str) -> Option<String> {
    match name {
        "stop-color" => {
            let css = CssColor::parse_string(value).ok()?;
            if is_current_color(&css) {
                Some("currentColor".into())
            } else {
                Some(SvgPaint::Color(color(&css)?).as_attr().into_owned())
            }
        }
        "stop-opacity" => Some(
            AlphaValue::parse_string(value)
                .ok()?
                .0
                .clamp(0., 1.)
                .to_string(),
        ),
        "rx" | "ry" if value.eq_ignore_ascii_case("auto") => Some("auto".into()),
        _ => Some(svg_length_text(lp_definite(
            &LengthPercentage::parse_string(value).ok()?,
        )?)),
    }
}

/// Serializes a computed length with its percentage precision intact.
pub fn svg_length_text(length: DefiniteLength) -> String {
    match length {
        DefiniteLength::Absolute(v) => f32::from(v.to_pixels(px(rem_basis()))).to_string(),
        DefiniteLength::Fraction(f) => format!("{}%", f * 100.),
        DefiniteLength::Calc { fraction, pixels } => {
            let p = f32::from(pixels);
            let sign = if p < 0. { '-' } else { '+' };
            format!("calc({}% {sign} {}px)", fraction * 100., p.abs())
        }
    }
}

/// Evaluates SVG lengths in the user units of the nearest viewport.
pub fn svg_user_length(text: &str, basis: f32) -> Option<f32> {
    if let Ok(value) = text.trim().parse::<f32>() {
        return value.is_finite().then_some(value);
    }
    fn resolve(lp: &LengthPercentage, basis: f32) -> Option<f32> {
        match lp {
            DimensionPercentage::Dimension(v) => length_value_px(v),
            DimensionPercentage::Percentage(p) => Some(p.0 * basis),
            DimensionPercentage::Calc(c) => calc_term(c, &|v| resolve(v, basis)),
        }
    }
    resolve(&LengthPercentage::parse_string(text).ok()?, basis).filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_geometry_and_gradient_stops_accept_their_own_value_grammars() {
        assert_eq!(computed("stop-opacity", "75%").as_deref(), Some("0.75"));
        assert_eq!(computed("stop-opacity", "red"), None);
        assert_eq!(computed("stop-color", "2px"), None);
        assert_eq!(computed("rx", "auto").as_deref(), Some("auto"));
        assert_eq!(computed("cx", "not-a-length"), None);
        let value = computed("x", "calc(12.5% - .5px)").unwrap();
        assert_eq!(svg_user_length(&value, 24.), Some(2.5));
        assert_eq!(svg_user_length(&value, 40.), Some(4.5));
    }
}
