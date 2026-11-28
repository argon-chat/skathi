//! Numbered HID input reports from the Steam Controller (2026) and its wireless receiver.
//! Wire formats: [Valve's packed Triton reports](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/steam/controller_structs.h).

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Controls {
    pub buttons: [f64; 26],
    pub axes: [f64; 4],
}

#[derive(Debug, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "A report is decoded and consumed on the stack; avoid a heap allocation on every HID sample."
)]
pub(super) enum Report {
    State(Controls),
    Connected,
    Disconnected,
}

fn radio_status(state: u8) -> Option<Report> {
    match state {
        1 => Some(Report::Disconnected),
        2 => Some(Report::Connected),
        _ => None,
    }
}

pub(super) fn decode(bytes: &[u8]) -> Option<Report> {
    match *bytes.first()? {
        // USB, BLE and newer timestamped packets each carry a fresh complete control sample.
        // They share a control prefix; some firmware appends sensor data or HID padding.
        0x42 | 0x45 | 0x47 if bytes.len() >= 46 => {
            let bits = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
            let button = |bit: u32| f64::from(bits & (1u32 << bit) != 0);
            let word = |i: usize| i16::from_le_bytes([bytes[i], bytes[i + 1]]);
            let trigger = |i| f64::from(word(i).max(0)) / f64::from(i16::MAX);
            let axis = |i| {
                let v = word(i);
                f64::from(v) / if v < 0 { 32768. } else { 32767. }
            };
            Some(Report::State(Controls {
                buttons: [
                    // Face buttons A, B, X, Y.
                    button(0),
                    button(1),
                    button(2),
                    button(3),
                    // Bumpers and analog trigger travel.
                    button(19),
                    button(9),
                    trigger(6),
                    trigger(8),
                    // Back, start, left stick click, right stick click.
                    button(14),
                    button(6),
                    button(15),
                    button(5),
                    // D-pad up, down, left, right, then the Steam button.
                    button(13),
                    button(10),
                    button(12),
                    button(11),
                    button(16),
                    // Buttons beyond the standard mapping: quick access, R4/L4, R5/L5.
                    button(4),
                    button(7),
                    button(17),
                    button(8),
                    button(18),
                    // Right/left pad clicks, left/right trigger clicks.
                    button(22),
                    button(26),
                    button(27),
                    button(23),
                ],
                axes: [axis(10), -axis(12), axis(14), -axis(16)],
            }))
        }
        // The controller can already be awake when the radio connect edge is missed.
        // As in hid-steam, a valid controller battery sample also establishes presence.
        0x43 if bytes.len() >= 15 && (1..=4).contains(&bytes[1]) && bytes[2] <= 100 => {
            Some(Report::Connected)
        }
        0x46 | 0x79 => radio_status(*bytes.get(1)?),
        // The receiver's GET_WIRELESS_STATE feature reply echoes the opcode and payload length.
        2 => match bytes {
            [2, 0xb4, len, payload @ ..] if *len != 0 && usize::from(*len) <= payload.len() => {
                radio_status(payload[0])
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_sticks_retain_small_deflections_for_calibration() {
        let mut data = [0_u8; 46];
        data[0] = 0x42;
        // Sampled near the outer edge of both sticks' idle jitter on a wireless controller.
        for (offset, word) in [(10, -331_i16), (12, 617), (14, 208), (16, 961)] {
            data[offset..offset + 2].copy_from_slice(&word.to_le_bytes());
        }
        let Some(Report::State(state)) = decode(&data) else {
            panic!("missing state")
        };
        assert_eq!(
            state.axes,
            [
                -331. / 32768.,
                -617. / 32767.,
                208. / 32767.,
                -961. / 32767.
            ]
        );
    }

    #[test]
    fn triton_decodes_wired_ble_and_timestamped_state() {
        // Right D-pad, south, start and L4 pressed; the right trigger is fully down.
        let mut data = [0_u8; 64];
        data[1..18].copy_from_slice(&[
            0xff, 0x41, 0x08, 0x02, 0x00, 0x00, 0x40, 0xff, 0x7f, 0x00, 0x80, 0xff, 0x7f, 0xff,
            0x7f, 0x00, 0x80,
        ]);
        for id in [0x42, 0x45, 0x47] {
            data[0] = id;
            let Some(Report::State(state)) = decode(&data[..46]) else {
                panic!("missing state")
            };
            assert_eq!(state.axes, [-1., -1., 1., 1.]);
            let pressed: Vec<_> = state
                .buttons
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| (v == 1.).then_some(i))
                .collect();
            assert_eq!(pressed, [0, 7, 9, 15, 19]);
            assert!((state.buttons[6] - 0.5).abs() < 0.0001);
            assert_eq!(decode(&data), Some(Report::State(state)));
            assert_eq!(decode(&data[..45]), None);
        }
    }

    #[test]
    fn triton_radio_status_identifies_connected_slots() {
        for id in [0x46, 0x79] {
            assert_eq!(decode(&[id, 1]), Some(Report::Disconnected));
            assert_eq!(decode(&[id, 2]), Some(Report::Connected));
            assert_eq!(decode(&[id, 0]), None);
            assert_eq!(decode(&[id]), None);
        }
        assert_eq!(decode(&[2, 0xb4, 1, 2]), Some(Report::Connected));
        assert_eq!(decode(&[2, 0xb4, 1, 1, 0, 0]), Some(Report::Disconnected));
        for invalid in [
            &[2, 0xb4, 2, 2][..],
            &[2, 0xb4, 0, 2],
            &[2, 0xb4, 1],
            &[2, 0xb4, 1, 3],
            &[1, 0xb4, 1, 2],
            &[2, 0x83, 1, 2],
            &[0; 64],
            &[],
        ] {
            assert_eq!(decode(invalid), None);
        }
    }

    #[test]
    fn triton_controller_battery_can_recover_a_missed_radio_connect() {
        let mut battery = [0; 64];
        battery[..3].copy_from_slice(&[0x43, 1, 67]);
        assert_eq!(decode(&battery[..15]), Some(Report::Connected));
        assert_eq!(decode(&battery), Some(Report::Connected));
        assert_eq!(decode(&battery[..14]), None);
        battery[1] = 0;
        assert_eq!(decode(&battery), None);
        battery[1] = 2;
        battery[2] = 255;
        assert_eq!(decode(&battery), None);
        assert_eq!(decode(&[0x7b; 64]), None);
    }
}
