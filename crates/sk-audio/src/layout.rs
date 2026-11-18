//! The static bed and the object coordinates, without the platform: Web Audio's speaker masks onto
//! `AudioObjectType`s, and its world space onto the listener-relative space of Windows spatial
//! audio (meters, +x right, +y up, +z behind the listener).

use redjs::webapi::audio::ObjectPlacement;

/// One static bed channel: its WAVEFORMATEXTENSIBLE speaker bit and its `AudioObjectType` bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BedChannel {
    pub speaker: u32,
    pub object_type: u32,
}

const fn ch(speaker: u32, object_type: u32) -> BedChannel {
    BedChannel {
        speaker,
        object_type,
    }
}

/// 7.1.4 in WAVEFORMATEXTENSIBLE channel order.
pub const BED_7_1_4: [BedChannel; 12] = [
    ch(0x1, 0x2),        // front left
    ch(0x2, 0x4),        // front right
    ch(0x4, 0x8),        // front center
    ch(0x8, 0x10),       // low frequency
    ch(0x10, 0x80),      // back left
    ch(0x20, 0x100),     // back right
    ch(0x200, 0x20),     // side left
    ch(0x400, 0x40),     // side right
    ch(0x1000, 0x200),   // top front left
    ch(0x4000, 0x400),   // top front right
    ch(0x8000, 0x800),   // top back left
    ch(0x20000, 0x1000), // top back right
];

/// The bed for a renderer whose native static objects are `native` (an `AudioObjectType` mask):
/// those of 7.1.4 it has, in channel order, and always front left and right.
pub fn bed(native: u32) -> Vec<BedChannel> {
    BED_7_1_4
        .iter()
        .enumerate()
        .filter(|(i, c)| *i < 2 || native & c.object_type != 0)
        .map(|(_, c)| *c)
        .collect()
}

pub fn speaker_mask(bed: &[BedChannel]) -> u32 {
    bed.iter().fold(0, |m, c| m | c.speaker)
}

pub fn object_type_mask(bed: &[BedChannel]) -> u32 {
    bed.iter().fold(0, |m, c| m | c.object_type)
}

type V = [f32; 3];

fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(a: V) -> Option<V> {
    let n = dot(a, a).sqrt();
    (n > 1e-6 && n.is_finite()).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

/// The panner's position relative to the listener, in Windows' spatial coordinates. A listener
/// whose forward and up are parallel (undefined in the spec) faces the default direction.
pub fn listener_space(p: &ObjectPlacement) -> V {
    let d = [
        p.position[0] - p.listener_position[0],
        p.position[1] - p.listener_position[1],
        p.position[2] - p.listener_position[2],
    ];
    let basis = normalized(p.listener_forward).and_then(|f| {
        let r = normalized(cross(f, p.listener_up))?;
        Some((f, r, cross(r, f)))
    });
    let (f, r, u) = basis.unwrap_or(([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
    let v = [dot(d, r), dot(d, u), -dot(d, f)];
    if v.iter().all(|c| c.is_finite()) {
        v
    } else {
        [0.0; 3]
    }
}

/// `SetVolume` takes 0..=1; a spec gain above 1 (a rolloff past the reference distance) is applied
/// to the samples instead. Returns `(volume, sample gain)`.
pub fn object_gain(gain: f32) -> (f32, f32) {
    if !gain.is_finite() || gain <= 0.0 {
        (0.0, 1.0)
    } else if gain > 1.0 {
        (1.0, gain)
    } else {
        (gain, 1.0)
    }
}

/// An object's mono signal: a stereo panner input is mixed down as the spec's speakers rule does.
pub fn object_samples(left: &[f32], right: &[f32], channels: u32, gain: f32, out: &mut [f32]) {
    let n = out.len().min(left.len());
    if channels >= 2 {
        for ((o, l), r) in out[..n].iter_mut().zip(left).zip(right) {
            *o = 0.5 * (l + r) * gain;
        }
    } else {
        for (o, l) in out[..n].iter_mut().zip(left) {
            *o = l * gain;
        }
    }
    out[n..].fill(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(position: V, listener: V, forward: V, up: V) -> ObjectPlacement {
        ObjectPlacement {
            position,
            listener_position: listener,
            listener_forward: forward,
            listener_up: up,
            gain: 1.0,
            ..Default::default()
        }
    }

    fn near(a: V, b: V) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn a_full_renderer_gets_the_whole_7_1_4_bed_in_wave_order() {
        let all = BED_7_1_4.iter().fold(0, |m, c| m | c.object_type);
        let b = bed(all);
        assert_eq!(b.len(), 12);
        assert_eq!(speaker_mask(&b), 0x2d63f);
        assert!(b.windows(2).all(|w| w[0].speaker < w[1].speaker));
    }

    #[test]
    fn a_7_1_renderer_drops_the_heights_and_ignores_channels_outside_7_1_4() {
        let native = 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x80 | 0x100 | 0x2000 | 0x20000;
        let b = bed(native);
        assert_eq!(speaker_mask(&b), 0x63f);
        assert_eq!(object_type_mask(&b), 0x1fe);
    }

    #[test]
    fn a_renderer_without_native_objects_still_has_front_left_and_right() {
        let b = bed(0);
        assert_eq!(b, vec![ch(0x1, 0x2), ch(0x2, 0x4)]);
    }

    #[test]
    fn the_default_listener_maps_web_audio_axes_unchanged() {
        let o = [0.0; 3];
        let (f, u) = ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        assert!(near(
            listener_space(&place([1.0, 0.0, 0.0], o, f, u)),
            [1.0, 0.0, 0.0]
        ));
        assert!(near(
            listener_space(&place([0.0, 2.0, 0.0], o, f, u)),
            [0.0, 2.0, 0.0]
        ));
        assert!(near(
            listener_space(&place([0.0, 0.0, -3.0], o, f, u)),
            [0.0, 0.0, -3.0]
        ));
    }

    #[test]
    fn a_turned_and_moved_listener_hears_relative_to_its_own_axes() {
        // Facing +x: a source further along +x is in front, one at +z is to the right.
        let l = [10.0, 0.0, 0.0];
        let (f, u) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(near(
            listener_space(&place([12.0, 0.0, 0.0], l, f, u)),
            [0.0, 0.0, -2.0]
        ));
        assert!(near(
            listener_space(&place([10.0, 0.0, 5.0], l, f, u)),
            [5.0, 0.0, 0.0]
        ));
        // An unnormalised, not quite orthogonal up is projected out.
        let tilted = listener_space(&place(
            [10.0, 1.0, 0.0],
            l,
            [4.0, 0.0, 0.0],
            [1.0, 3.0, 0.0],
        ));
        assert!(near(tilted, [0.0, 1.0, 0.0]), "{tilted:?}");
    }

    #[test]
    fn a_degenerate_listener_faces_the_default_direction() {
        let p = place([1.0, 0.0, -1.0], [0.0; 3], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(near(listener_space(&p), [1.0, 0.0, -1.0]));
        let nan = place(
            [f32::NAN, 0.0, 0.0],
            [0.0; 3],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
        );
        assert_eq!(listener_space(&nan), [0.0; 3]);
    }

    #[test]
    fn gains_above_one_move_into_the_samples() {
        assert_eq!(object_gain(0.25), (0.25, 1.0));
        assert_eq!(object_gain(3.0), (1.0, 3.0));
        assert_eq!(object_gain(f32::NAN), (0.0, 1.0));
        let mut out = [9.0; 4];
        object_samples(&[1.0, 0.5], &[0.0, 0.5], 2, 2.0, &mut out);
        assert_eq!(out, [1.0, 1.0, 0.0, 0.0]);
    }
}
