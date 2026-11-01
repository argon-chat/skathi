use super::*;

/// xorshift64*, for repeatable noise.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Bytes with runs of the extremes, where clamping and saturation live.
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| match self.next() % 8 {
                0 => 0,
                1 => 255,
                _ => self.next() as u8,
            })
            .collect()
    }
}

const ALL: [Colorimetry; 10] = {
    const fn c(matrix: Matrix, full_range: bool) -> Colorimetry {
        Colorimetry { matrix, full_range }
    }
    [
        c(Matrix::Bt601, false),
        c(Matrix::Bt601, true),
        c(Matrix::Bt709, false),
        c(Matrix::Bt709, true),
        c(Matrix::Bt2020, false),
        c(Matrix::Bt2020, true),
        c(Matrix::Smpte240, false),
        c(Matrix::Smpte240, true),
        c(Matrix::Fcc, false),
        c(Matrix::Fcc, true),
    ]
};

#[test]
fn every_level_gives_the_scalar_bytes_at_every_width() {
    let mut noise = Noise(0x9e37_79b9_7f4a_7c15);
    for level in Level::available() {
        for w in 1..=97usize {
            let cw = w.div_ceil(2);
            let (near, far) = (noise.bytes(cw), noise.bytes(cw));
            let (mut want, mut got) = (vec![0; w], vec![0; w]);
            let mut tmp = vec![0u16; cw + 2];
            chroma_row(Level::Scalar, &near, &far, &mut tmp, &mut want);
            chroma_row(level, &near, &far, &mut tmp, &mut got);
            assert_eq!(got, want, "chroma, {} width {w}", level.name());

            let (y, u, v, a) = (noise.bytes(w), noise.bytes(w), noise.bytes(w), noise.bytes(w));
            for colour in ALL {
                let k = colour.coeffs();
                for alpha in [Some(&a[..]), None] {
                    let (mut want, mut got) = (vec![0; 4 * w], vec![0; 4 * w]);
                    yuva_row(Level::Scalar, &k, &y, &u, &v, alpha, &mut want);
                    yuva_row(level, &k, &y, &u, &v, alpha, &mut got);
                    assert_eq!(got, want, "yuva, {} width {w} {colour:?} alpha {}", level.name(), alpha.is_some());
                }
            }
        }
    }
}

/// The exact conversion of one sample, in f64.
fn exact(colour: Colorimetry, y: f64, u: f64, v: f64) -> [f64; 3] {
    let (kr, kb) = colour.matrix.kr_kb();
    let kg = 1.0 - kr - kb;
    let (ky, kc, yoff) = if colour.full_range { (1.0, 1.0, 0.0) } else { (255.0 / 219.0, 255.0 / 224.0, 16.0) };
    let (y, u, v) = (ky * (y - yoff), kc * (u - 128.0), kc * (v - 128.0));
    let r = y + 2.0 * (1.0 - kr) * v;
    let b = y + 2.0 * (1.0 - kb) * u;
    let g = (y - kr * r - kb * b) / kg;
    [b, g, r]
}

#[test]
fn the_integer_matrix_is_the_exact_one_rounded() {
    // Every Y with a spread of U, V: off by at most 1, and rarely.
    for colour in ALL {
        let k = colour.coeffs();
        let (mut total, mut off) = (0, 0);
        for y in 0..=255u8 {
            for u in (0..=255u8).step_by(5) {
                for v in (0..=255u8).step_by(3) {
                    let mut out = [0u8; 4];
                    yuva_row(Level::Scalar, &k, &[y], &[u], &[v], None, &mut out);
                    let want = exact(colour, y as f64, u as f64, v as f64);
                    for c in 0..3 {
                        let w = want[c].round().clamp(0.0, 255.0);
                        let d = (out[c] as f64 - w).abs();
                        assert!(d <= 1.0, "{colour:?} Y{y} U{u} V{v}: {} against {}", out[c], want[c]);
                        total += 1;
                        off += (d > 0.0) as u32;
                    }
                }
            }
        }
        // Measured: 0.3 % for BT.601 studio range (the 13-bit coefficients'
        // error tips a value lying within ~0.01 of a half).
        assert!(off * 100 < total, "{colour:?}: {off} of {total} off by one");
    }
}

#[test]
fn the_reference_colours_come_out() {
    // RGB, each channel within `tolerance` of `want` (the YUV of a primary is
    // itself rounded, so a primary comes back within 2).
    let check = |colour: Colorimetry, (y, u, v): (u8, u8, u8), want: [u8; 3], tolerance: i32| {
        let mut out = [0u8; 4];
        yuva_row(Level::best(), &colour.coeffs(), &[y], &[u], &[v], None, &mut out);
        let rgb = [out[2], out[1], out[0]];
        assert!(rgb.iter().zip(want).all(|(&g, w)| (g as i32 - w as i32).abs() <= tolerance), "{colour:?} {y} {u} {v}: {rgb:?}, want {want:?}");
    };
    let studio601 = Colorimetry { matrix: Matrix::Bt601, full_range: false };
    check(studio601, (16, 128, 128), [0, 0, 0], 0);
    check(studio601, (235, 128, 128), [255, 255, 255], 0);
    check(studio601, (126, 128, 128), [128, 128, 128], 0);
    // BT.601's primaries in studio range.
    check(studio601, (81, 90, 240), [255, 0, 0], 2);
    check(studio601, (145, 54, 34), [0, 255, 0], 2);
    check(studio601, (41, 240, 110), [0, 0, 255], 2);
    // BT.709's: the same red read as BT.601 is visibly off.
    let studio709 = Colorimetry { matrix: Matrix::Bt709, full_range: false };
    check(studio709, (63, 102, 240), [255, 0, 0], 2);
    check(studio709, (173, 42, 26), [0, 255, 0], 2);
    check(studio601, (63, 102, 240), [234, 0, 2], 1);
    let full601 = Colorimetry { matrix: Matrix::Bt601, full_range: true };
    check(full601, (0, 128, 128), [0, 0, 0], 0);
    check(full601, (255, 128, 128), [255, 255, 255], 0);
    check(full601, (76, 85, 255), [254, 0, 0], 1);
}

#[test]
fn chroma_is_centre_sited_and_bilinear() {
    // A step from 0 to 160 between chroma samples 1 and 2: luma 3 and 4 sit
    // a quarter and three quarters across it.
    let near = [0, 0, 160, 160];
    let mut tmp = [0u16; 6];
    let mut out = [0u8; 8];
    chroma_row(Level::Scalar, &near, &near, &mut tmp, &mut out);
    assert_eq!(out, [0, 0, 0, 40, 120, 160, 160, 160]);
    // Vertically: 3/4 near, 1/4 far.
    chroma_row(Level::Scalar, &[200], &[40], &mut tmp, &mut out[..2]);
    assert_eq!(&out[..2], &[160, 160]);
    // An odd width ends on an even luma sample.
    let mut odd = [0u8; 3];
    chroma_row(Level::Scalar, &[0, 160], &[0, 160], &mut tmp, &mut odd);
    assert_eq!(odd, [0, 40, 120]);
}

/// A frame of noise with gradients in it, `w` × `h`, owned.
struct Frame {
    w: usize,
    h: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    a: Vec<u8>,
    stride: usize,
}

impl Frame {
    fn new(w: usize, h: usize, seed: u64) -> Frame {
        let mut noise = Noise(seed);
        let stride = w + 7; // planes with padding, as libvpx's
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut plane = |pw: usize, ph: usize, f: &dyn Fn(usize, usize) -> u8| -> Vec<u8> {
            let mut p = noise.bytes(stride * ph);
            for y in 0..ph {
                for x in 0..pw {
                    if !noise.next().is_multiple_of(4) {
                        p[y * stride + x] = f(x, y);
                    }
                }
            }
            p
        };
        let y = plane(w, h, &|x, y| (16 + (x * 7 + y * 3) % 220) as u8);
        let u = plane(cw, ch, &|x, _| (40 + x * 11 % 180) as u8);
        let v = plane(cw, ch, &|_, y| (60 + y * 13 % 150) as u8);
        let a = plane(w, h, &|x, y| if (x / 5 + y / 4) % 3 == 0 { 0 } else { ((x * 30 + y * 20) % 256) as u8 });
        Frame { w, h, y, u, v, a, stride }
    }

    fn i420<'s>(&'s self, alpha: bool) -> I420<'s> {
        let p = |data: &'s [u8]| Plane { data, stride: self.stride };
        I420 { width: self.w as u32, height: self.h as u32, y: p(&self.y), u: p(&self.u), v: p(&self.v), a: alpha.then(|| p(&self.a)) }
    }

    /// The straight BGRA of every pixel at full size, from the scalar rows.
    fn bgra(&self, colour: Colorimetry, alpha: bool) -> Vec<u8> {
        let mut out = vec![0; self.w * self.h * 4];
        Converter::with_level(Level::Scalar).to_bgra(&self.i420(alpha), colour, &mut out, self.w as u32, self.h as u32).unwrap();
        out
    }
}

const STUDIO601: Colorimetry = Colorimetry { matrix: Matrix::Bt601, full_range: false };

#[test]
fn a_frame_at_its_own_size_is_its_pixels_at_every_level() {
    for (w, h) in [(1, 1), (2, 2), (41, 29), (64, 48), (100, 100), (33, 1), (1, 17)] {
        let frame = Frame::new(w, h, (w * 1000 + h) as u64);
        for alpha in [true, false] {
            let want = frame.bgra(STUDIO601, alpha);
            // Pixel by pixel from the definition: chroma bilinear, then the
            // matrix, then the alpha as is.
            for y in 0..h {
                for x in 0..w {
                    let px = &want[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    assert_eq!(px[3], if alpha { frame.a[y * frame.stride + x] } else { 255 });
                }
            }
            for level in Level::available() {
                let mut got = vec![0; w * h * 4];
                Converter::with_level(level).to_bgra(&frame.i420(alpha), STUDIO601, &mut got, w as u32, h as u32).unwrap();
                assert_eq!(got, want, "{w}x{h} {}", level.name());
            }
        }
    }
}

/// The reference resampler, in f64: per axis area (down or 1:1) or
/// bilinear (up), on premultiplied values.
fn reference_scale(src: &[u8], (w, h): (usize, usize), (tw, th): (usize, usize)) -> Vec<f64> {
    fn taps(n: usize, t: usize) -> Vec<Vec<(usize, f64)>> {
        let s = n as f64 / t as f64;
        (0..t)
            .map(|o| {
                if t <= n {
                    let (lo, hi) = (o as f64 * s, (o + 1) as f64 * s);
                    (0..n)
                        .filter_map(|i| {
                            let w = (hi.min(i as f64 + 1.0) - lo.max(i as f64)).max(0.0) / s;
                            (w > 0.0).then_some((i, w))
                        })
                        .collect()
                } else {
                    let p = ((o as f64 + 0.5) * s - 0.5).clamp(0.0, (n - 1) as f64);
                    let i = p.floor() as usize;
                    let f = p - i as f64;
                    vec![(i, 1.0 - f), ((i + 1).min(n - 1), f)]
                }
            })
            .collect()
    }
    let (xt, yt) = (taps(w, tw), taps(h, th));
    let mut out = vec![0.0; tw * th * 4];
    for (oy, ty) in yt.iter().enumerate() {
        for (ox, tx) in xt.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            for &(sy, wy) in ty {
                for &(sx, wx) in tx {
                    let p = &src[(sy * w + sx) * 4..];
                    let a = p[3] as f64;
                    for c in 0..3 {
                        acc[c] += wy * wx * p[c] as f64 * a;
                    }
                    acc[3] += wy * wx * a;
                }
            }
            let o = &mut out[(oy * tw + ox) * 4..];
            for c in 0..3 {
                o[c] = if acc[3] > 0.0 { acc[c] / acc[3] } else { 0.0 };
            }
            o[3] = acc[3];
        }
    }
    out
}

#[test]
fn scaling_is_the_reference_resampler_within_a_rounding() {
    let sizes = [((41, 29), (20, 14)), ((41, 29), (17, 13)), ((64, 48), (32, 24)), ((100, 100), (37, 37)), ((41, 29), (100, 70)), ((20, 20), (41, 9)), ((41, 29), (1, 1)), ((7, 5), (41, 29))];
    for ((w, h), (tw, th)) in sizes {
        let frame = Frame::new(w, h, 77 + w as u64);
        let src = frame.bgra(STUDIO601, true);
        let want = reference_scale(&src, (w, h), (tw, th));
        for level in Level::available() {
            let mut got = vec![0; tw * th * 4];
            Converter::with_level(level).to_bgra(&frame.i420(true), STUDIO601, &mut got, tw as u32, th as u32).unwrap();
            for (i, (&g, &r)) in got.iter().zip(&want).enumerate() {
                // Colour under almost no alpha is the ratio of two tiny sums:
                // only meaningful where some alpha is left.
                if i % 4 != 3 && want[i - i % 4 + 3] < 0.5 {
                    continue;
                }
                assert!((g as f64 - r).abs() <= 0.5 + 1e-3, "{w}x{h} to {tw}x{th} {} byte {i}: {g} against {r}", level.name());
            }
        }
    }
}

#[test]
fn a_transparent_pixels_colour_does_not_bleed_into_an_edge() {
    // Columns 0..3 opaque red, 3..8 transparent green.
    let (w, h) = (8usize, 2usize);
    let y: Vec<u8> = (0..w * h).map(|i| if i % w < 3 { 81 } else { 145 }).collect();
    let u: Vec<u8> = (0..4).map(|i| if i < 2 { 90 } else { 54 }).collect();
    let v: Vec<u8> = (0..4).map(|i| if i < 2 { 240 } else { 34 }).collect();
    let a: Vec<u8> = (0..w * h).map(|i| if i % w < 3 { 255 } else { 0 }).collect();
    let frame = I420 {
        width: w as u32,
        height: h as u32,
        y: Plane { data: &y, stride: w },
        u: Plane { data: &u, stride: 4 },
        v: Plane { data: &v, stride: 4 },
        a: Some(Plane { data: &a, stride: w }),
    };
    let mut out = vec![0; 3 * 4];
    Converter::new().to_bgra(&frame, STUDIO601, &mut out, 3, 1).unwrap();
    let mut full = vec![0; w * h * 4];
    Converter::new().to_bgra(&frame, STUDIO601, &mut full, w as u32, h as u32).unwrap();
    // The middle output pixel covers columns 2.67..5.33: an eighth of it is
    // column 2, the last opaque one, and the rest transparent green. Its
    // alpha is an eighth, and its colour is column 2's alone.
    let mid = &out[4..8];
    assert_eq!(mid[3], 32);
    assert_eq!(&mid[..3], &full[8..11]);
    assert!(mid[1] < 40, "no green from the transparent part: {mid:?}");
}

#[test]
fn alpha_only_writes_white_and_the_alpha() {
    let frame = Frame::new(41, 29, 5);
    let plane = Plane { data: &frame.a, stride: frame.stride };
    let mut c = Converter::new();
    let mut same = vec![0; 41 * 29 * 4];
    c.alpha_to_bgra(Some(plane), (41, 29), &mut same, 41, 29).unwrap();
    for y in 0..29 {
        for x in 0..41 {
            assert_eq!(&same[(y * 41 + x) * 4..(y * 41 + x) * 4 + 4], &[255, 255, 255, frame.a[y * frame.stride + x]]);
        }
    }
    // Scaled: the alpha of the colour path, exactly (it averages alpha alike).
    for (tw, th) in [(20, 14), (100, 70), (3, 3)] {
        let mut alpha = vec![0; tw * th * 4];
        c.alpha_to_bgra(Some(plane), (41, 29), &mut alpha, tw as u32, th as u32).unwrap();
        let mut colour = vec![0; tw * th * 4];
        c.to_bgra(&frame.i420(true), STUDIO601, &mut colour, tw as u32, th as u32).unwrap();
        for (a, c) in alpha.chunks_exact(4).zip(colour.chunks_exact(4)) {
            assert_eq!(&a[..3], &[255, 255, 255]);
            assert_eq!(a[3], c[3]);
        }
    }
    let mut opaque = vec![0; 16];
    c.alpha_to_bgra(None, (41, 29), &mut opaque, 2, 2).unwrap();
    assert!(opaque.iter().all(|&b| b == 255));
}

#[test]
fn an_opaque_frame_scales_like_a_plain_average() {
    let frame = Frame::new(64, 48, 9);
    let src = frame.bgra(STUDIO601, false);
    let mut got = vec![0; 32 * 24 * 4];
    Converter::new().to_bgra(&frame.i420(false), STUDIO601, &mut got, 32, 24).unwrap();
    for oy in 0..24 {
        for ox in 0..32 {
            for c in 0..4 {
                let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|&(dx, dy)| src[((oy * 2 + dy) * 64 + ox * 2 + dx) * 4 + c] as u32).sum();
                let want = (sum as f64 / 4.0).round();
                let g = got[(oy * 32 + ox) * 4 + c] as f64;
                assert!((g - want).abs() <= 1.0, "({ox},{oy}) channel {c}: {g} against {want}");
            }
        }
    }
}

#[test]
fn wrong_sizes_are_errors() {
    let frame = Frame::new(16, 16, 1);
    let mut c = Converter::new();
    let mut out = vec![0; 8 * 8 * 4];
    assert_eq!(c.to_bgra(&frame.i420(true), STUDIO601, &mut out[..10], 8, 8), Err(Error::Target));
    let mut small = frame.i420(true);
    small.u.data = &frame.u[..20];
    assert_eq!(c.to_bgra(&small, STUDIO601, &mut out, 8, 8), Err(Error::Plane("U")));
    let mut small = frame.i420(true);
    small.a = Some(Plane { data: &frame.a[..100], stride: frame.stride });
    assert_eq!(c.to_bgra(&small, STUDIO601, &mut out, 8, 8), Err(Error::Plane("alpha")));
    small.y.stride = 3;
    assert_eq!(c.to_bgra(&small, STUDIO601, &mut out, 8, 8), Err(Error::Plane("Y")));
    assert_eq!(c.alpha_to_bgra(Some(Plane { data: &[0; 10], stride: 16 }), (16, 16), &mut out, 8, 8), Err(Error::Plane("alpha")));
    // Empty targets write nothing and fail nothing.
    assert_eq!(c.to_bgra(&frame.i420(true), STUDIO601, &mut [], 0, 5), Ok(()));
}

#[test]
fn scratch_stays_a_few_rows() {
    let frame = Frame::new(100, 100, 3);
    let mut c = Converter::new();
    let mut out = vec![0; 37 * 37 * 4];
    c.to_bgra(&frame.i420(true), STUDIO601, &mut out, 37, 37).unwrap();
    assert!(c.memory_bytes() < 100 * 4 * 4 * 8, "{}", c.memory_bytes());
}

/// `cargo test --release -p vn-yuv bench -- --ignored --nocapture`: µs a
/// frame for each level, source size and target size, one thread.
#[test]
#[ignore]
fn bench() {
    // The fastest of many runs: the cost with the machine's other load
    // (other builds, other apps) taken out.
    let time = |f: &mut dyn FnMut()| {
        f();
        let start = std::time::Instant::now();
        let mut best = f64::MAX;
        while start.elapsed().as_millis() < 300 {
            let t = std::time::Instant::now();
            f();
            best = best.min(t.elapsed().as_secs_f64() * 1e6);
        }
        best
    };
    for (w, targets) in [(512usize, [512usize, 384, 256, 160, 64]), (100, [100, 64, 48, 32, 160])] {
        let frame = Frame::new(w, w, 11);
        for level in Level::available() {
            let mut c = Converter::with_level(level);
            let mut line = format!("{w:>3}² {:<6}", level.name());
            for t in targets {
                let mut out = vec![0; t * t * 4];
                let colour = time(&mut || c.to_bgra(&frame.i420(true), STUDIO601, &mut out, t as u32, t as u32).unwrap());
                let alpha = time(&mut || {
                    c.alpha_to_bgra(Some(Plane { data: &frame.a, stride: frame.stride }), (w as u32, w as u32), &mut out, t as u32, t as u32).unwrap()
                });
                line += &format!(" | →{t}: {colour:7.1} µs, alpha {alpha:6.1}");
            }
            println!("{line}");
        }
    }
}
