pub(crate) mod identity;

use crate::{deadzone::stick, ButtonChange, DeadZones, Pad, PadEvent};
use gilrs::{Axis, Button, Gamepad, MappingSource};

const WEB_BUTTONS: [Button; 17] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
    Button::Mode,
];

fn standard_layout(gamepad: &Gamepad<'_>) -> bool {
    if gamepad.mapping_source() == MappingSource::SdlMappings {
        return true;
    }
    // A WGI Gamepad has a fixed layout and nil UUID. RawGameController indices are vendor-defined.
    if cfg!(target_os = "windows") {
        return gamepad.uuid() == [0; 16];
    }
    // Linux's evdev gamepad button and stick codes have assigned positional meanings.
    cfg!(target_os = "linux")
        && gamepad.button_code(Button::South).is_some()
        && gamepad.axis_code(Axis::LeftStickX).is_some()
        && gamepad.axis_code(Axis::LeftStickY).is_some()
}

pub(super) struct State {
    pub(super) pad: Pad,
    raw_sticks: [f64; 4],
    hardware_dead_zone: f64,
    // gilrs exposes physical codes as events arrive. Keep their assigned slots until disconnect.
    extra_axes: Vec<u32>,
    extra_buttons: Vec<u32>,
}

fn extra_slot(codes: &mut Vec<u32>, values: &mut Vec<f64>, offset: usize, code: u32) -> usize {
    match codes.iter().position(|&v| v == code) {
        Some(index) => offset + index,
        None => {
            codes.push(code);
            values.push(0.0);
            values.len() - 1
        }
    }
}

impl State {
    pub(super) fn connected(instance: u32, gamepad: &Gamepad<'_>, zones: &DeadZones) -> Self {
        let standard = standard_layout(gamepad);
        let vendor = gamepad.vendor_id().unwrap_or(0);
        let product = gamepad.product_id().unwrap_or(0);
        let (name, family) =
            identity::resolve(vendor, product, gamepad.os_name(), gamepad.map_name());
        let pad = Pad {
            instance,
            system_id: crate::system_id::gilrs_id(gamepad),
            name: name.to_owned(),
            vendor,
            product,
            family,
            standard,
            dead_zone: 0.0,
            can_rumble: gamepad.is_ff_supported(),
            axes: vec![0.0; if standard { 4 } else { 0 }],
            buttons: vec![0.0; if standard { WEB_BUTTONS.len() } else { 0 }],
        };
        Self::from_pad(pad, zones)
    }

    /// At connection, the source supplies its native default and unfiltered axes.
    pub(super) fn from_pad(mut pad: Pad, zones: &DeadZones) -> Self {
        let hardware_dead_zone = pad.dead_zone;
        pad.dead_zone = zones.effective(&pad, hardware_dead_zone);
        let mut raw_sticks = [0.; 4];
        if pad.standard {
            raw_sticks.copy_from_slice(&pad.axes[..4]);
            for offset in [0, 2] {
                let pair = [raw_sticks[offset], raw_sticks[offset + 1]];
                pad.axes[offset..offset + 2].copy_from_slice(&stick(pair, pad.dead_zone));
            }
        }
        Self {
            pad,
            raw_sticks,
            hardware_dead_zone,
            extra_buttons: Vec::new(),
            extra_axes: Vec::new(),
        }
    }

    pub(super) fn configure(&mut self, zones: &DeadZones, changes: &mut Vec<PadEvent>) {
        let radius = zones.effective(&self.pad, self.hardware_dead_zone);
        if self.pad.dead_zone != radius {
            self.pad.dead_zone = radius;
            changes.push(PadEvent::DeadZone {
                instance: self.pad.instance,
                radius,
            });
            if self.pad.standard {
                self.set_sticks(self.raw_sticks, true, changes);
            }
        }
    }

    pub(super) fn set_sticks(
        &mut self,
        raw: [f64; 4],
        recalibrated: bool,
        changes: &mut Vec<PadEvent>,
    ) {
        self.raw_sticks = raw;
        self.publish_stick(0, recalibrated, changes);
        self.publish_stick(2, recalibrated, changes);
    }

    fn publish_stick(&mut self, offset: usize, recalibrated: bool, changes: &mut Vec<PadEvent>) {
        let pair = [self.raw_sticks[offset], self.raw_sticks[offset + 1]];
        let [x, y] = stick(pair, self.pad.dead_zone);
        self.change_axis(offset, x, recalibrated, changes);
        self.change_axis(offset + 1, y, recalibrated, changes);
    }

    pub(super) fn change_button(&mut self, index: usize, value: f64, changes: &mut Vec<PadEvent>) {
        let before = self.pad.buttons[index];
        if before != value {
            self.pad.buttons[index] = value;
            changes.push(PadEvent::Button(ButtonChange {
                instance: self.pad.instance,
                index,
                before,
                value,
            }));
        }
    }

    pub(super) fn button(
        &mut self,
        button: Button,
        code: u32,
        value: f32,
        changes: &mut Vec<PadEvent>,
    ) {
        if !value.is_finite() {
            return;
        }
        let standard = self.pad.standard;
        let index = if standard && let Some(i) = WEB_BUTTONS.iter().position(|&b| b == button) {
            i
        } else {
            extra_slot(
                &mut self.extra_buttons,
                &mut self.pad.buttons,
                if standard { WEB_BUTTONS.len() } else { 0 },
                code,
            )
        };
        self.change_button(index, f64::from(value.clamp(0.0, 1.0)), changes);
    }

    pub(super) fn axis(&mut self, axis: Axis, code: u32, value: f32, changes: &mut Vec<PadEvent>) {
        if !value.is_finite() {
            return;
        }
        let value = f64::from(value.clamp(-1.0, 1.0));
        let standard = self.pad.standard;
        let (index, value) = if standard {
            match axis {
                Axis::LeftStickX => (0, value),
                Axis::LeftStickY => (1, -value),
                Axis::RightStickX => (2, value),
                Axis::RightStickY => (3, -value),
                Axis::LeftZ | Axis::RightZ => {
                    return self.change_button(
                        if axis == Axis::LeftZ { 6 } else { 7 },
                        (value + 1.0) * 0.5,
                        changes,
                    );
                }
                Axis::DPadX | Axis::DPadY => {
                    let (negative, positive) = if axis == Axis::DPadX {
                        (14, 15)
                    } else {
                        (13, 12)
                    };
                    let (released, pressed) = if value < 0.0 {
                        (positive, negative)
                    } else {
                        (negative, positive)
                    };
                    self.change_button(released, 0.0, changes);
                    return self.change_button(pressed, f64::from(value.abs() >= 0.5), changes);
                }
                _ => {
                    let index = extra_slot(&mut self.extra_axes, &mut self.pad.axes, 4, code);
                    (index, value)
                }
            }
        } else {
            let index = extra_slot(&mut self.extra_axes, &mut self.pad.axes, 0, code);
            (index, value)
        };
        if standard && index < 4 {
            self.raw_sticks[index] = value;
            self.publish_stick(index & !1, false, changes);
        } else {
            self.change_axis(index, value, false, changes);
        }
    }

    fn change_axis(
        &mut self,
        index: usize,
        value: f64,
        recalibrated: bool,
        changes: &mut Vec<PadEvent>,
    ) {
        let before = self.pad.axes[index];
        if before != value {
            self.pad.axes[index] = value;
            changes.push(PadEvent::Axis {
                instance: self.pad.instance,
                index,
                before,
                value,
                recalibrated,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(standard: bool) -> State {
        State::from_pad(
            Pad {
                instance: 17,
                system_id: None,
                name: String::from("Test"),
                vendor: 0,
                product: 0,
                family: "generic",
                standard,
                dead_zone: 0.0,
                can_rumble: false,
                axes: vec![0.0; if standard { 4 } else { 0 }],
                buttons: vec![0.0; if standard { 17 } else { 0 }],
            },
            &DeadZones::default(),
        )
    }

    #[test]
    fn settings_resolve_by_connection_endpoint_model_global_then_source_default() {
        let mut pad = state(true).pad;
        pad.vendor = 0x1234;
        pad.product = 0x5678;
        pad.system_id = Some("first".into());
        let mut other = pad.clone();
        other.instance += 1;
        other.system_id = Some("second".into());

        let mut zones = DeadZones::default();
        assert_eq!(zones.effective(&pad, 0.04), 0.04);
        zones.global = Some(0.1);
        assert_eq!(zones.effective(&pad, 0.04), 0.1);
        zones.models.insert((pad.vendor, pad.product), 0.2);
        assert_eq!(zones.effective(&pad, 0.04), 0.2);
        zones.devices.insert("first".into(), 0.3);
        assert_eq!(zones.effective(&other, 0.04), 0.2);
        assert_eq!(zones.effective(&pad, 0.04), 0.3);
        zones.connections.insert(pad.instance, 0.);
        assert_eq!(zones.effective(&pad, 0.04), 0.);
        zones.connections.insert(pad.instance, f64::NAN);
        assert_eq!(zones.effective(&pad, 0.04), 0.3);
        other.system_id = None;
        assert_eq!(zones.effective(&other, 0.04), 0.2);
    }

    #[test]
    fn elite_model_settings_follow_the_controller_across_transports() {
        let mut pad = state(true).pad;
        pad.vendor = 0x045e;
        pad.family = "xbox";
        let mut zones = DeadZones::default();
        zones
            .models
            .insert(DeadZones::model_key(pad.vendor, 0x0b05), 0.15);
        for product in [0x0b00, 0x0b05, 0x0b22] {
            pad.product = product;
            assert_eq!(zones.effective(&pad, 0.), 0.15);
        }
        // A virtual device advertising the same model IDs shares the model rule.
        pad.family = "generic";
        assert_eq!(zones.effective(&pad, 0.), 0.15);
        pad.vendor = 0x28de;
        pad.product = 0x11ff;
        assert_eq!(zones.effective(&pad, 0.), 0.);
        pad.family = "xbox";
        pad.vendor = 0x045e;
        pad.product = 0x02e3;
        assert_eq!(zones.effective(&pad, 0.), 0.);
    }

    #[test]
    fn changing_a_held_sticks_radius_emits_metadata_then_ordered_axes_from_the_raw_pair() {
        let mut state = state(true);
        let mut events = Vec::new();
        state.axis(Axis::RightStickX, 10, 0.15, &mut events);
        state.axis(Axis::RightStickY, 11, -0.2, &mut events);
        state.axis(Axis::Unknown, 23, 0.12, &mut events);
        state.axis(Axis::LeftZ, 55, 0., &mut events);
        let initial = state.pad.axes.clone();
        let buttons = state.pad.buttons.clone();
        events.clear();

        let zones = DeadZones {
            global: Some(0.3),
            ..Default::default()
        };
        state.configure(&zones, &mut events);
        state.configure(&zones, &mut events);
        assert!(matches!(
            events.as_slice(),
            [
                PadEvent::DeadZone {
                    instance: 17,
                    radius: 0.3
                },
                PadEvent::Axis {
                    instance: 17,
                    index: 2,
                    value: 0.,
                    recalibrated: true,
                    ..
                },
                PadEvent::Axis {
                    instance: 17,
                    index: 3,
                    value: 0.,
                    recalibrated: true,
                    ..
                },
            ]
        ));
        assert_eq!(state.pad.axes[4], initial[4]);
        assert_eq!(state.pad.buttons, buttons);

        events.clear();
        state.configure(&DeadZones::default(), &mut events);
        assert_eq!(state.pad.axes, initial);
        assert!(matches!(
            events.as_slice(),
            [
                PadEvent::DeadZone { radius: 0., .. },
                PadEvent::Axis { index: 2, .. },
                PadEvent::Axis { index: 3, .. },
            ]
        ));
    }

    #[test]
    fn recalibration_and_physical_motion_in_one_sample_keep_their_origin_and_previous_value() {
        let mut state = state(true);
        let mut events = Vec::new();
        state.axis(Axis::LeftStickX, 10, 0.75, &mut events);
        events.clear();
        state.configure(
            &DeadZones {
                global: Some(0.75),
                ..Default::default()
            },
            &mut events,
        );
        state.axis(Axis::LeftStickX, 10, 1.0, &mut events);
        state.axis(Axis::LeftStickX, 10, 0.0, &mut events);
        assert!(matches!(
            events.as_slice(),
            [
                PadEvent::DeadZone { radius: 0.75, .. },
                PadEvent::Axis {
                    index: 0,
                    before: 0.75,
                    value: 0.,
                    recalibrated: true,
                    ..
                },
                PadEvent::Axis {
                    index: 0,
                    before: 0.,
                    value: 1.,
                    recalibrated: false,
                    ..
                },
                PadEvent::Axis {
                    index: 0,
                    before: 1.,
                    value: 0.,
                    recalibrated: false,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn unknown_controller_axes_remain_raw_with_any_radius() {
        let mut state = state(false);
        let mut events = Vec::new();
        state.axis(Axis::LeftStickX, 77, -0.01, &mut events);
        let before = state.pad.axes.clone();
        events.clear();
        state.configure(
            &DeadZones {
                global: Some(0.5),
                ..Default::default()
            },
            &mut events,
        );
        assert_eq!(state.pad.dead_zone, 0.5);
        assert_eq!(state.pad.axes, before);
        assert!(matches!(events.as_slice(), [PadEvent::DeadZone { .. }]));
        state.axis(Axis::RightStickX, 99, 0.02, &mut events);
        assert_eq!(state.pad.axes, [f64::from(-0.01_f32), f64::from(0.02_f32)]);
    }

    #[test]
    fn standard_mapping_preserves_analog_triggers_and_short_presses() {
        let mut state = state(true);
        let mut changes = Vec::new();
        state.button(Button::RightTrigger2, 112, 0.75, &mut changes);
        state.button(Button::West, 110, 1.0, &mut changes);
        state.button(Button::West, 110, 1.0, &mut changes);
        state.button(Button::West, 110, 0.0, &mut changes);
        state.axis(Axis::LeftStickY, 20, 1.0, &mut changes);
        assert_eq!(state.pad.axes, [0., -1., 0., 0.]);
        assert_eq!(state.pad.buttons[2], 0.);
        assert_eq!(state.pad.buttons[7], 0.75);
        let transitions: Vec<_> = changes
            .iter()
            .filter_map(PadEvent::button)
            .map(|c| (c.instance, c.index, c.before, c.value))
            .collect();
        assert_eq!(
            transitions,
            [(17, 7, 0., 0.75), (17, 2, 0., 1.), (17, 2, 1., 0.)]
        );
    }

    #[test]
    fn dpad_axis_releases_the_old_direction_before_pressing_the_new_one() {
        let mut state = state(true);
        let mut changes = Vec::new();
        state.axis(Axis::DPadY, 11, 1.0, &mut changes);
        state.axis(Axis::DPadY, 11, -1.0, &mut changes);
        state.axis(Axis::DPadY, 11, 0.0, &mut changes);
        let edges: Vec<_> = changes
            .iter()
            .filter_map(PadEvent::button)
            .map(|c| (c.index, c.value))
            .collect();
        assert_eq!(edges, [(12, 1.), (12, 0.), (13, 1.), (13, 0.)]);
    }

    #[test]
    fn unmapped_controls_keep_their_slots_for_the_connection() {
        let mut raw = state(false);
        let mut changes = Vec::new();
        raw.button(Button::Unknown, 945, 1., &mut changes);
        raw.button(Button::Unknown, 17, 1., &mut changes);
        raw.button(Button::Unknown, 945, 0., &mut changes);
        raw.axis(Axis::Unknown, 666, -2., &mut changes);
        raw.axis(Axis::Unknown, 2, 0.5, &mut changes);
        raw.axis(Axis::Unknown, 666, f32::NAN, &mut changes);
        assert_eq!(raw.pad.buttons, [0., 1.]);
        assert_eq!(raw.pad.axes, [-1., 0.5]);
        let mut mapped = state(true);
        mapped.button(Button::Unknown, 429, 1., &mut changes);
        assert_eq!(mapped.pad.buttons[17], 1.);
    }
}
