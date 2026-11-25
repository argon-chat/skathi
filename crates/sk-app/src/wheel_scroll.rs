//! `"input": { "wheelScrollScale" }` in sk.config.json and `app.wheelScrollScale`: how far a wheel
//! notch moves the app's own scrolling. `WheelEvent` deltas and pixel (touchpad) deltas are not scaled.

pub(crate) const MIN: f64 = 0.1;
pub(crate) const MAX: f64 = 10.0;

/// `None` for a value that is not a finite number above 0; anything else is kept within [`MIN`, `MAX`].
pub(crate) fn clamp(scale: f64) -> Option<f32> {
    (scale.is_finite() && scale > 0.0).then(|| scale.clamp(MIN, MAX) as f32)
}

pub(crate) fn get() -> f64 {
    f64::from(gpui::vn_wheel_scroll_scale())
}

pub(crate) fn set(scale: f32) {
    gpui::vn_set_wheel_scroll_scale(scale);
}

fn configured(window_json: Option<&str>) -> Option<f64> {
    window_json
        .and_then(crate::json::parse)?
        .get("input")?
        .get("wheelScrollScale")?
        .as_f64()
}

/// At start: the app's setting, else 1.
pub(crate) fn apply(window_json: Option<&str>) {
    let scale = match configured(window_json) {
        Some(value) => clamp(value).unwrap_or_else(|| {
            log::warn!(
                "sk.config.json \"input.wheelScrollScale\" is {value}: it must be a number above 0; using 1"
            );
            1.0
        }),
        None => 1.0,
    };
    set(scale);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_values_are_refused_and_the_rest_clamped() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(clamp(bad), None);
        }
        assert_eq!(clamp(2.0), Some(2.0));
        assert_eq!(clamp(1e-6), Some(MIN as f32));
        assert_eq!(clamp(1e6), Some(MAX as f32));
    }
}
