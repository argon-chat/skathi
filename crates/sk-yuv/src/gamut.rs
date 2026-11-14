//! A frame's RGB in other primaries, or with another transfer curve, moved to sRGB's: what
//! Chromium's compositor does to a video on an sRGB display (checked against Chrome 156 in
//! sk-sticker's video tests).
//!
//! Per pixel, on straight RGB: decode with the frame's transfer curve, a 3×3 matrix from its
//! primaries to sRGB's (Bradford-adapted from its white point to D65, as skcms adapts through D50),
//! clamp, encode with the sRGB curve. Chromium draws the SDR video curves (BT.709, SMPTE 170M,
//! BT.2020) with the sRGB curve itself, so a frame in BT.709 primaries with one of those is
//! untouched: [`Gamut::new`] gives `None`.
//!
//! The decode table is 256 floats, the encode 8193 bytes over linear light in steps of 1/8192.

use crate::{Primaries, Transfer};

const STEPS: usize = 8192;

/// Chromaticities `[r, g, b, white]`, `(x, y)`: skia's SkNamedPrimaries.
fn chromaticities(p: Primaries) -> [[f64; 2]; 4] {
    const D65: [f64; 2] = [0.3127, 0.3290];
    const C: [f64; 2] = [0.310, 0.316];
    match p {
        Primaries::Bt709 => [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060], D65],
        Primaries::Bt470M => [[0.67, 0.33], [0.21, 0.71], [0.14, 0.08], C],
        Primaries::Bt470Bg => [[0.64, 0.33], [0.29, 0.60], [0.15, 0.06], D65],
        Primaries::Smpte170 | Primaries::Smpte240 => {
            [[0.630, 0.340], [0.310, 0.595], [0.155, 0.070], D65]
        }
        Primaries::Film => [[0.681, 0.319], [0.243, 0.692], [0.145, 0.049], C],
        Primaries::Bt2020 => [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046], D65],
        Primaries::Smpte428 => [[1.0, 0.0], [0.0, 1.0], [0.0, 0.0], [1.0 / 3.0, 1.0 / 3.0]],
        Primaries::Smpte431 => [
            [0.680, 0.320],
            [0.265, 0.690],
            [0.150, 0.060],
            [0.314, 0.351],
        ],
        Primaries::P3 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060], D65],
        Primaries::Ebu3213 => [[0.630, 0.340], [0.295, 0.605], [0.155, 0.077], D65],
    }
}

type M3 = [[f64; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for (i, row) in o.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn invert(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let (ca, cb, cc) = (e * i - f * h, f * g - d * i, d * h - e * g);
    let det = a * ca + b * cb + c * cc;
    [
        [ca / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [cb / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [cc / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

fn xyz([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// RGB to XYZ for these chromaticities, white at Y = 1. A primary's column
/// only needs its direction, `(x, y, 1 − x − y)`: XYZ's own blue has y = 0.
fn to_xyz(c: [[f64; 2]; 4]) -> M3 {
    let direction = |[x, y]: [f64; 2]| [x, y, 1.0 - x - y];
    let [r, g, b] = [direction(c[0]), direction(c[1]), direction(c[2])];
    let p = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let s = apply(&invert(&p), xyz(c[3]));
    p.map(|row| [row[0] * s[0], row[1] * s[1], row[2] * s[2]])
}

fn bradford(from: [f64; 3], to: [f64; 3]) -> M3 {
    const B: M3 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let (f, t) = (apply(&B, from), apply(&B, to));
    let scale = [
        [t[0] / f[0], 0.0, 0.0],
        [0.0, t[1] / f[1], 0.0],
        [0.0, 0.0, t[2] / f[2]],
    ];
    mul(&invert(&B), &mul(&scale, &B))
}

pub(crate) fn matrix(p: Primaries) -> M3 {
    let (src, srgb) = (chromaticities(p), chromaticities(Primaries::Bt709));
    let adapt = bradford(xyz(src[3]), xyz(srgb[3]));
    mul(&invert(&to_xyz(srgb)), &mul(&adapt, &to_xyz(src)))
}

/// skcms's parametric curve: `c·x + f` below `d`, else `(a·x + b)^g + e`.
#[derive(Clone, Copy)]
struct Curve {
    g: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

fn curve(t: Transfer) -> Curve {
    let power = |g: f64, a: f64| Curve {
        g,
        a,
        b: 0.0,
        c: 0.0,
        d: 0.0,
    };
    match t {
        Transfer::Srgb => Curve {
            g: 2.4,
            a: 1.0 / 1.055,
            b: 0.055 / 1.055,
            c: 1.0 / 12.92,
            d: 0.04045,
        },
        Transfer::Gamma22 => power(2.2, 1.0),
        Transfer::Gamma28 => power(2.8, 1.0),
        Transfer::Smpte240 => Curve {
            g: 1.0 / 0.45,
            a: 1.0 / 1.1115,
            b: 0.1115 / 1.1115,
            c: 1.0 / 4.0,
            d: 0.0913,
        },
        Transfer::Linear => power(1.0, 1.0),
        Transfer::Smpte428 => power(2.6, 1.034_080_527_699),
    }
}

pub(crate) fn decode(t: Transfer, x: f64) -> f64 {
    let k = curve(t);
    if x < k.d {
        k.c * x
    } else {
        (k.a * x + k.b).max(0.0).powf(k.g)
    }
}

pub(crate) fn encode_srgb(l: f64) -> f64 {
    if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

pub(crate) struct Gamut {
    pub(crate) key: (Primaries, Transfer),
    linear: [f32; 256],
    m: [[f32; 3]; 3],
    encode: Box<[u8]>,
}

impl Gamut {
    /// `None` when there is nothing to do: BT.709 primaries and a curve Chromium draws as sRGB's.
    pub(crate) fn new(primaries: Primaries, transfer: Transfer) -> Option<Gamut> {
        if primaries == Primaries::Bt709 && transfer == Transfer::Srgb {
            return None;
        }
        let m = matrix(primaries).map(|row| row.map(|v| v as f32));
        let linear = std::array::from_fn(|i| decode(transfer, i as f64 / 255.0) as f32);
        let encode = (0..=STEPS)
            .map(|i| (encode_srgb(i as f64 / STEPS as f64) * 255.0).round() as u8)
            .collect();
        Some(Gamut {
            key: (primaries, transfer),
            linear,
            m,
            encode,
        })
    }

    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.encode.len()
    }

    pub(crate) fn apply(&self, pixels: &mut [u8]) {
        let (m, lin, enc) = (&self.m, &self.linear, &self.encode[..]);
        let out = |v: f32| enc[(v.clamp(0.0, 1.0) * STEPS as f32 + 0.5) as usize];
        for p in pixels.chunks_exact_mut(4) {
            if p[3] == 0 {
                continue;
            }
            let (b, g, r) = (lin[p[0] as usize], lin[p[1] as usize], lin[p[2] as usize]);
            p[0] = out(m[2][0] * r + m[2][1] * g + m[2][2] * b);
            p[1] = out(m[1][0] * r + m[1][1] * g + m[1][2] * b);
            p[2] = out(m[0][0] * r + m[0][1] * g + m[0][2] * b);
        }
    }
}
