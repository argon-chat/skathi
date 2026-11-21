use crate::power::PowerState;
use gpui::EffectQuality;
use sk_css::{ComputePressure, PowerScheme};

impl PowerState {
    pub(crate) fn effect_quality(&self) -> EffectQuality {
        automatic_quality(self.scheme, self.pressure, self.on_battery)
    }
}

fn automatic_quality(
    scheme: PowerScheme,
    pressure: ComputePressure,
    on_battery: bool,
) -> EffectQuality {
    if pressure >= ComputePressure::Serious {
        EffectQuality::Minimal
    } else if pressure == ComputePressure::Fair
        || scheme == PowerScheme::Saving
        || (on_battery && scheme != PowerScheme::Performance)
    {
        EffectQuality::Reduced
    } else {
        EffectQuality::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_quality_respects_pressure_saver_and_power_source() {
        use ComputePressure::*;
        use EffectQuality::*;
        use PowerScheme::*;
        for (scheme, pressure, battery, expected) in [
            (Balanced, Nominal, false, Full),
            (Balanced, Nominal, true, Reduced),
            (Performance, Nominal, true, Full),
            (Saving, Nominal, false, Reduced),
            (Performance, Fair, false, Reduced),
            (Balanced, Serious, false, Minimal),
            (Performance, Critical, true, Minimal),
        ] {
            assert_eq!(automatic_quality(scheme, pressure, battery), expected);
        }
    }
}
