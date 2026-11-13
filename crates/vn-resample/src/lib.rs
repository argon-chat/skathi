//! The pixel kernel behind `vn-tree`'s downscaled image copies (`vn_tree::resample` decides when a
//! copy is made). It has a crate of its own so the dev profile can give it `opt-level = 3`:
//! `image::imageops::resize` is generic, so it is compiled in whichever crate calls it, and from
//! `vn-tree` it ran unoptimised in debug builds.

/// `w` × `h` pixels of 4 bytes, BGRA or RGBA (the channel order does not
/// matter here), filtered down to `tw` × `th`. Straight alpha unless
/// `premultiplied`; the result keeps the input's convention.
pub fn downscale(bytes: &[u8], w: u32, h: u32, tw: u32, th: u32, premultiplied: bool) -> Vec<u8> {
    use image::{imageops, ImageBuffer, Rgba};
    let (tw, th) = (tw.clamp(1, w.max(1)), th.clamp(1, h.max(1)));
    // First a k×k box average, k the largest power of two that leaves the image at least twice the
    // target, so Lanczos has a ratio under 4 left. In integers off the bytes, premultiplied so the
    // filter weighs colour by coverage; this pass reads every source pixel and is most of the cost.
    let mut k = 1u32;
    while w / (k * 2) >= tw * 2 && h / (k * 2) >= th * 2 && k < 128 {
        k *= 2;
    }
    let (bw, bh) = (w / k, h / k);
    let mut px = vec![0f32; (bw * bh * 4) as usize];
    // Straight alpha sums c·a (up to 255² a pixel), premultiplied sums c.
    let alpha_scale = 1.0 / (255.0 * (k * k) as f32);
    let colour_scale = if premultiplied { alpha_scale } else { alpha_scale / 255.0 };
    let mut acc = vec![[0u32; 4]; bw as usize];
    for by in 0..bh {
        acc.iter_mut().for_each(|a| *a = [0; 4]);
        for y in by * k..(by + 1) * k {
            let row = &bytes[(y * w * 4) as usize..((y * w + bw * k) * 4) as usize];
            if premultiplied {
                box_row::<true>(row, k, &mut acc);
            } else {
                box_row::<false>(row, k, &mut acc);
            }
        }
        let out = &mut px[(by * bw * 4) as usize..((by + 1) * bw * 4) as usize];
        for (o, s) in out.chunks_exact_mut(4).zip(&acc) {
            o[0] = s[0] as f32 * colour_scale;
            o[1] = s[1] as f32 * colour_scale;
            o[2] = s[2] as f32 * colour_scale;
            o[3] = s[3] as f32 * alpha_scale;
        }
    }
    let (w, h) = (bw, bh);
    let buffer: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::from_raw(w, h, px).expect("w * h * 4 floats");
    let small = if (w, h) == (tw, th) { buffer } else { imageops::resize(&buffer, tw, th, imageops::FilterType::Lanczos3) };
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for p in small.pixels() {
        let a = p[3].clamp(0.0, 1.0);
        let k = if premultiplied {
            255.0
        } else if a > 0.0 {
            255.0 / a
        } else {
            0.0
        };
        let c = |v: f32| (v.clamp(0.0, a) * k).round() as u8;
        out.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8]);
    }
    out
}

#[inline(always)]
fn box_row<const PREMULTIPLIED: bool>(row: &[u8], k: u32, acc: &mut [[u32; 4]]) {
    for (block, s) in row.chunks_exact((k * 4) as usize).zip(acc.iter_mut()) {
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        for p in block.chunks_exact(4) {
            let pa = p[3] as u32;
            let w = if PREMULTIPLIED { 1 } else { pa };
            r += p[0] as u32 * w;
            g += p[1] as u32 * w;
            b += p[2] as u32 * w;
            a += pa;
        }
        s[0] += r;
        s[1] += g;
        s[2] += b;
        s[3] += a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn checker(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = if (x + y) % 2 == 0 { 255 } else { 0 };
                v.extend_from_slice(&[c, c, c, 255]);
            }
        }
        v
    }

    #[test]
    fn a_checkerboard_averages_to_grey_instead_of_aliasing() {
        let out = downscale(&checker(1080, 1080), 1080, 1080, 40, 40, false);
        assert_eq!(out.len(), 40 * 40 * 4);
        for p in out.chunks_exact(4) {
            assert!((p[0] as i32 - 128).abs() <= 2, "{p:?}");
            assert_eq!(p[3], 255);
        }
        let out = downscale(&checker(1080, 1080), 1080, 1080, 47, 47, false);
        for p in out.chunks_exact(4) {
            assert!((p[0] as i32 - 128).abs() <= 3, "{p:?}");
        }
    }

    #[test]
    fn a_transparent_pixel_adds_no_colour() {
        // Left half opaque red, right half fully transparent GREEN: a filter
        // that ignores alpha bleeds green into the edge.
        let (w, h) = (64u32, 8u32);
        let mut v = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                v.extend_from_slice(if x < w / 2 { &[255, 0, 0, 255] } else { &[0, 255, 0, 0] });
            }
        }
        let out = downscale(&v, w, h, 8, 1, false);
        for p in out.chunks_exact(4) {
            if p[3] > 0 {
                assert!(p[1] <= 2 && p[0] >= 250, "edge pixel {p:?}");
            }
        }
        let pre: Vec<u8> = v.iter().enumerate().map(|(i, b)| if i % 4 == 1 { 0 } else { *b }).collect();
        for p in downscale(&pre, w, h, 8, 1, true).chunks_exact(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "{p:?}");
        }
    }

    fn best_of(n: usize, f: impl Fn()) -> Duration {
        let mut best = Duration::MAX;
        for _ in 0..n {
            let t = Instant::now();
            f();
            best = best.min(t.elapsed());
        }
        best
    }

    /// The avatar case `vn_tree::resample::SYNC_PIXELS` is sized by, and a banner drawn at half size. The budget holds in debug too.
    #[test]
    fn an_avatar_and_a_banner_downscale_quickly() {
        let avatar = checker(1080, 1080);
        let a = best_of(5, || drop(std::hint::black_box(downscale(&avatar, 1080, 1080, 80, 80, false))));
        let banner = checker(1920, 1080);
        let b = best_of(3, || drop(std::hint::black_box(downscale(&banner, 1920, 1080, 960, 540, false))));
        eprintln!("1080x1080 -> 80x80: {a:?}; 1920x1080 -> 960x540: {b:?}");
        assert!(a < Duration::from_millis(8), "{a:?}");
    }
}
