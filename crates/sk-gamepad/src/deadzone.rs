use crate::state::identity;
use crate::Pad;
use std::collections::BTreeMap;

/// Radial dead zones for the two standard sticks. A radius of zero passes the axes through.
/// Each override is a finite fraction in [0, 1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeadZones {
    pub global: Option<f64>,
    /// Keyed by [`DeadZones::model_key`] so documented transport aliases share a setting.
    pub models: BTreeMap<(u16, u16), f64>,
    pub devices: BTreeMap<String, f64>,
    pub connections: BTreeMap<u32, f64>,
}

impl DeadZones {
    pub fn model_key(vendor: u16, product: u16) -> (u16, u16) {
        identity::model_key(vendor, product)
    }

    pub(super) fn effective(&self, pad: &Pad, hardware_default: f64) -> f64 {
        let model = Self::model_key(pad.vendor, pad.product);
        [
            self.connections.get(&pad.instance),
            pad.system_id.as_ref().and_then(|id| self.devices.get(id)),
            self.models.get(&model),
            self.global.as_ref(),
        ]
        .into_iter()
        .flatten()
        .copied()
        .find(|value| (0.0..1.0).contains(value))
        .unwrap_or(hardware_default)
    }
}

pub(super) fn stick([x, y]: [f64; 2], dead_zone: f64) -> [f64; 2] {
    if dead_zone == 0.0 {
        return [x, y];
    }
    let radius = (x * x + y * y).sqrt();
    if radius <= dead_zone {
        return [0.0, 0.0];
    }
    if radius >= 1.0 {
        return [x, y];
    }
    let scale = (radius - dead_zone) / ((1.0 - dead_zone) * radius);
    [x * scale, y * scale]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_response_is_continuous_and_preserves_direction_and_full_travel() {
        assert_eq!(stick([0.04, 0.], 0.04), [0., 0.]);
        let just_outside = stick([0.04001, 0.], 0.04)[0];
        assert!(just_outside > 0. && just_outside < 0.00002);
        let [x, y] = stick([-0.3, 0.4], 0.04);
        assert!((x / y + 0.75).abs() < 1e-12);
        assert!((x.hypot(y) - (0.5 - 0.04) / 0.96).abs() < 1e-12);
        assert_eq!(stick([0.6, -0.8], 0.04), [0.6, -0.8]);
        assert_eq!(stick([-1., 1.], 0.9), [-1., 1.]);
        assert_eq!(stick([0.01, -0.01], 0.0), [0.01, -0.01]);
    }
}
