//! Direct Windows input for the Steam Controller (2026) wireless receiver.

mod report;
mod windows;

use super::{connect, disconnect, DeadZones, Device, Pad, PadEvent, Source, State};
use report::{Controls, Report};

pub(super) struct Receiver {
    transport: windows::Transport,
}

impl Receiver {
    pub(super) fn for_process() -> Option<Self> {
        // In a Steam game the user's layout owns the input, including keyboard-only layouts.
        // Reading physical controls there could dispatch the same action twice.
        let virtual_info = std::env::var_os("SteamVirtualGamepadInfo");
        let game_id = std::env::var("SteamGameId")
            .ok()
            .and_then(|s| s.parse::<u64>().ok());
        if virtual_info.is_some_and(|v| !v.is_empty()) || game_id.is_some_and(|id| id != 0) {
            return None;
        }
        Some(Self {
            transport: windows::Transport::new(),
        })
    }

    pub(super) fn poll(
        &mut self,
        devices: &mut Vec<Device>,
        events: &mut Vec<PadEvent>,
        zones: &DeadZones,
    ) {
        self.transport.poll(|slot, product, path, bytes| {
            let report = match bytes {
                Some(bytes) => report::decode(bytes),
                None => Some(Report::Disconnected),
            };
            if let Some(report) = report {
                apply(devices, events, slot, product, path, report, zones);
            }
        });
    }
}

fn apply(
    devices: &mut Vec<Device>,
    events: &mut Vec<PadEvent>,
    slot: u32,
    product: u16,
    path: &[u16],
    report: Report,
    zones: &DeadZones,
) {
    let source = Source::Steam(slot);
    let controls = match report {
        Report::Disconnected => return disconnect(devices, source, events),
        Report::State(controls) => Some(controls),
        Report::Connected => None,
    };
    if let Some(device) = devices.iter_mut().find(|device| device.source == source) {
        if let Some(controls) = controls {
            for (i, value) in controls.buttons.into_iter().enumerate() {
                device.state.change_button(i, value, events);
            }
            device.state.set_sticks(controls.axes, false, events);
        }
        return;
    }
    // Each receiver collection is one radio slot. The first state can arrive before a connect notice.
    let Controls { buttons, axes } = controls.unwrap_or_default();
    if let Some(device) = connect(devices, source, |instance| {
        State::from_pad(
            Pad {
                instance,
                system_id: crate::system_id::puck_id(path),
                name: "Steam Controller (2026)".to_owned(),
                vendor: 0x28de,
                product,
                family: "steam",
                standard: true,
                // Up to 3% deflection measured at rest in the Puck's 265 Hz report stream.
                dead_zone: 0.04,
                axes: axes.to_vec(),
                buttons: buttons.to_vec(),
                can_rumble: false,
            },
            zones,
        )
    }) {
        events.push(PadEvent::Connected {
            index: device.index,
            pad: device.state.pad.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_snapshot_uses_configured_radius_and_restores_receiver_default_on_reset() {
        let mut devices = Vec::new();
        let mut events = Vec::new();
        let raw = [-0.01, -0.018, 0.006, -0.029];
        let controls = Controls {
            axes: raw,
            ..Default::default()
        };
        let mut zones = DeadZones::default();
        zones.models.insert((0x28de, 0x1304), 0.);
        apply(
            &mut devices,
            &mut events,
            2,
            0x1304,
            &[],
            Report::State(controls),
            &zones,
        );
        assert!(
            matches!(events.as_slice(), [PadEvent::Connected { pad, .. }]
            if pad.dead_zone == 0. && pad.axes == raw)
        );
        events.clear();
        devices[0]
            .state
            .configure(&DeadZones::default(), &mut events);
        assert_eq!(devices[0].state.pad.dead_zone, 0.04);
        assert_eq!(devices[0].state.pad.axes, [0.; 4]);
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn radio_slots_publish_real_connections_and_preserve_edges() {
        let apply =
            |devices: &mut Vec<Device>, events: &mut Vec<PadEvent>, slot, product, report| {
                super::apply(
                    devices,
                    events,
                    slot,
                    product,
                    &[],
                    report,
                    &DeadZones::default(),
                );
            };
        let mut devices = Vec::new();
        let mut events = Vec::new();
        let mut controls = Controls::default();
        controls.buttons[0] = 1.0;
        apply(&mut devices, &mut events, 1, 0x1304, Report::Disconnected);
        apply(
            &mut devices,
            &mut events,
            2,
            0x1304,
            Report::State(controls.clone()),
        );
        assert!(
            matches!(events.as_slice(), [PadEvent::Connected { index: 0, pad }] if pad.buttons[0] == 1.0)
        );
        let instance = devices[0].state.pad.instance;
        events.clear();
        apply(&mut devices, &mut events, 2, 0x1304, Report::Connected);
        apply(
            &mut devices,
            &mut events,
            2,
            0x1304,
            Report::State(Controls::default()),
        );
        apply(
            &mut devices,
            &mut events,
            2,
            0x1304,
            Report::State(controls),
        );
        assert_eq!(
            events
                .iter()
                .filter_map(PadEvent::button)
                .map(|b| b.value)
                .collect::<Vec<_>>(),
            [0., 1.]
        );
        events.clear();
        apply(&mut devices, &mut events, 3, 0x1304, Report::Connected);
        apply(&mut devices, &mut events, 2, 0x1304, Report::Disconnected);
        apply(&mut devices, &mut events, 2, 0x1304, Report::Disconnected);
        apply(&mut devices, &mut events, 2, 0x1304, Report::Connected);
        assert!(
            matches!(events.as_slice(), [PadEvent::Connected {index: 1, ..}, PadEvent::Disconnected(old), PadEvent::Connected {index: 0, pad: new}] if old.instance == instance && new.instance != instance)
        );
    }
}
