//! Separable resampling of premultiplied rows: per axis an area average
//! (down, or 1:1) or a bilinear filter (up), in f32.
//!
//! Order: when the frame shrinks vertically, source rows are summed into an
//! output row first and the horizontal filter runs once per output row; when
//! it grows, each source row is filtered horizontally once and kept in a
//! small ring that the output rows blend. Either way the horizontal filter
//! runs on the fewer rows.
//!
//! Every output index reads the same number of taps (`n`, the most any
//! output needs), zero-padded, so the inner loops have no per-output setup.
//! A zero tap adds an exact 0, so the padding changes no result.

use crate::Level;

/// One axis: for each output index, `n` consecutive source indices from
/// `first` and their weights (summing to 1, zero-padded).
#[derive(Default)]
pub(crate) struct Taps {
    key: (usize, usize),
    first: Vec<u32>,
    weights: Vec<f32>,
    /// Taps per output.
    pub n: usize,
}

impl Taps {
    /// The taps from `n_src` to `n_dst` (both > 0); kept while they stay.
    pub fn build(&mut self, n_src: usize, n_dst: usize) {
        if self.key == (n_src, n_dst) && !self.first.is_empty() {
            return;
        }
        self.key = (n_src, n_dst);
        let scale = n_src as f64 / n_dst as f64;
        let mut all: Vec<Vec<(usize, f64)>> = Vec::with_capacity(n_dst);
        for o in 0..n_dst {
            let mut taps = Vec::with_capacity(scale.ceil() as usize + 2);
            if n_dst <= n_src {
                // Area: output `o` covers [o·s, (o+1)·s) of the source.
                let lo = o as f64 * scale;
                let hi = ((o + 1) as f64 * scale).min(n_src as f64);
                let mut i = lo.floor() as usize;
                while (i as f64) < hi && i < n_src {
                    let w = (hi.min(i as f64 + 1.0) - lo.max(i as f64)) / scale;
                    if w > 1e-9 {
                        taps.push((i, w));
                    }
                    i += 1;
                }
            } else {
                // Bilinear between the two source centres around the output's.
                let pos = ((o as f64 + 0.5) * scale - 0.5).clamp(0.0, (n_src - 1) as f64);
                let i = pos.floor() as usize;
                let f = pos - i as f64;
                if f > 1e-9 && i + 1 < n_src {
                    taps.push((i, 1.0 - f));
                    taps.push((i + 1, f));
                } else {
                    taps.push((i, 1.0));
                }
            }
            all.push(taps);
        }
        // Taps are consecutive, so `n` of them fit in the source.
        let n = all.iter().map(Vec::len).max().unwrap_or(1);
        self.n = n;
        self.first.clear();
        self.weights.clear();
        self.weights.resize(n_dst * n, 0.0);
        for (o, taps) in all.iter().enumerate() {
            let sum: f64 = taps.iter().map(|t| t.1).sum();
            let first = taps[0].0.min(n_src - n);
            self.first.push(first as u32);
            for &(i, w) in taps {
                self.weights[o * n + i - first] = (w / sum) as f32;
            }
        }
    }

    /// Output `o`'s first source index and its `n` weights.
    #[inline]
    pub fn get(&self, o: usize) -> (usize, &[f32]) {
        (self.first[o] as usize, &self.weights[o * self.n..(o + 1) * self.n])
    }

    pub fn is_identity(&self) -> bool {
        self.key.0 == self.key.1
    }

    pub fn bytes(&self) -> usize {
        (self.first.capacity() + self.weights.capacity()) * 4
    }
}

/// The scaler's buffers.
#[derive(Default)]
pub(crate) struct Scaler {
    xt: Taps,
    yt: Taps,
    /// One source row as `source` fills it, `w · C` bytes, and (growing)
    /// as floats, premultiplied when colour.
    bytes: Vec<u8>,
    src: Vec<f32>,
    /// Shrinking: the output row being summed, `w · C`, and the same
    /// filtered horizontally, `tw · C`.
    /// Growing: the output row being blended, `tw · C`.
    acc: Vec<f32>,
    row: Vec<f32>,
    /// Growing: the horizontally filtered source rows the output rows blend,
    /// a ring of `yt.n` rows of `tw · C`, and which source row each holds.
    ring: Vec<f32>,
    ring_rows: Vec<usize>,
}

impl Scaler {
    /// Scales `w` × `h` rows of `C` channels to `tw` × `th`. `source(sy,
    /// row)` fills `row` (`w · C` bytes) with source row `sy`: straight BGRA
    /// (`C` 4, which is weighed by its alpha) or alpha (`C` 1); `finish(oy,
    /// row)` takes output row `oy` (`tw · C` floats, premultiplied). Source
    /// rows are asked for in ascending order, each once per run.
    pub fn run<const C: usize>(
        &mut self,
        level: Level,
        (w, h): (usize, usize),
        (tw, th): (usize, usize),
        mut source: impl FnMut(usize, &mut [u8]),
        mut finish: impl FnMut(usize, &[f32]),
    ) {
        self.xt.build(w, tw);
        self.yt.build(h, th);
        self.bytes.resize(w * C, 0);
        if th <= h {
            self.acc.resize(w * C, 0.0);
            self.row.resize(tw * C, 0.0);
            let mut held = usize::MAX;
            for oy in 0..th {
                let (first, wy) = self.yt.get(oy);
                self.acc.fill(0.0);
                for (k, &weight) in wy.iter().enumerate() {
                    if weight == 0.0 {
                        continue; // padding: its row may be one no output reads
                    }
                    if held != first + k {
                        held = first + k;
                        source(held, &mut self.bytes);
                    }
                    accumulate::<C>(level, &mut self.acc, weight, &self.bytes);
                }
                horizontal::<C>(&self.xt, &self.acc, &mut self.row);
                finish(oy, &self.row);
            }
            return;
        }
        let slots = self.yt.n;
        let width = tw * C;
        self.src.resize(w * C, 0.0);
        self.acc.resize(width, 0.0);
        self.ring.resize(slots * width, 0.0);
        self.ring_rows.clear();
        self.ring_rows.resize(slots, usize::MAX);
        for oy in 0..th {
            let (first, wy) = self.yt.get(oy);
            for (k, &weight) in wy.iter().enumerate() {
                let sy = first + k;
                let slot = sy % slots;
                if weight != 0.0 && self.ring_rows[slot] != sy {
                    source(sy, &mut self.bytes);
                    to_f32::<C>(level, &self.bytes, &mut self.src);
                    horizontal::<C>(&self.xt, &self.src, &mut self.ring[slot * width..(slot + 1) * width]);
                    self.ring_rows[slot] = sy;
                }
            }
            self.acc.fill(0.0);
            for (k, &weight) in wy.iter().enumerate() {
                if weight != 0.0 {
                    let slot = (first + k) % slots;
                    axpy(level, &mut self.acc, weight, &self.ring[slot * width..(slot + 1) * width]);
                }
            }
            finish(oy, &self.acc);
        }
    }

    pub fn bytes(&self) -> usize {
        self.xt.bytes()
            + self.yt.bytes()
            + self.bytes.capacity()
            + (self.src.capacity() + self.acc.capacity() + self.row.capacity() + self.ring.capacity()) * 4
            + self.ring_rows.capacity() * std::mem::size_of::<usize>()
    }
}

/// A source row as floats: premultiplied when `C` is 4 (BGRA).
fn to_f32<const C: usize>(level: Level, bytes: &[u8], out: &mut [f32]) {
    if C == 4 {
        premultiply(level, bytes, out);
    } else {
        for (o, &b) in out.iter_mut().zip(bytes) {
            *o = b as f32;
        }
    }
}

/// `acc += weight · to_f32(bytes)`, in one pass.
fn accumulate<const C: usize>(level: Level, acc: &mut [f32], weight: f32, bytes: &[u8]) {
    #[cfg(target_arch = "x86_64")]
    let done = match (level, C) {
        // SAFETY: AVX2 was detected (it is the level).
        (Level::Avx2, 4) => unsafe { avx2::accumulate_premultiplied(acc, weight, bytes) },
        (Level::Avx2, _) => unsafe { avx2::accumulate_plain(acc, weight, bytes) },
        (_, 4) => sse::accumulate_premultiplied(acc, weight, bytes),
        _ => 0,
    };
    #[cfg(not(target_arch = "x86_64"))]
    let done = {
        let _ = level;
        0
    };
    if C == 4 {
        for (a, s) in as_pixels_mut::<4>(&mut acc[done..]).iter_mut().zip(bytes[done..].chunks_exact(4)) {
            let alpha = s[3] as f32;
            let p = [s[0] as f32 * alpha, s[1] as f32 * alpha, s[2] as f32 * alpha, alpha];
            for c in 0..4 {
                a[c] += weight * p[c];
            }
        }
    } else {
        for (a, &s) in acc[done..].iter_mut().zip(&bytes[done..]) {
            *a += weight * s as f32;
        }
    }
}

/// `acc += weight · src`.
fn axpy(level: Level, acc: &mut [f32], weight: f32, src: &[f32]) {
    #[cfg(target_arch = "x86_64")]
    if level == Level::Avx2 {
        // SAFETY: AVX2 was detected (it is the level).
        unsafe { axpy_avx2(acc, weight, src) };
        return;
    }
    let _ = level;
    for (a, &s) in acc.iter_mut().zip(src) {
        *a += weight * s;
    }
}

/// The same loop, compiled for AVX2 (8 floats a step; no FMA, so the same
/// results).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn axpy_avx2(acc: &mut [f32], weight: f32, src: &[f32]) {
    for (a, &s) in acc.iter_mut().zip(src) {
        *a += weight * s;
    }
}

/// Filters one row of `C`-channel pixels from `xt.key.0` to `xt.key.1`.
pub(crate) fn horizontal<const C: usize>(xt: &Taps, src: &[f32], out: &mut [f32]) {
    if xt.is_identity() {
        out.copy_from_slice(src);
        return;
    }
    match xt.n {
        1 => taps_n::<C, 1>(xt, src, out),
        2 => taps_n::<C, 2>(xt, src, out),
        3 => taps_n::<C, 3>(xt, src, out),
        4 => taps_n::<C, 4>(xt, src, out),
        5 => taps_n::<C, 5>(xt, src, out),
        _ => taps_any::<C>(xt, src, out),
    }
}

/// `N` taps an output, unrolled.
fn taps_n<const C: usize, const N: usize>(xt: &Taps, src: &[f32], out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if C == 4 {
        sse::taps4::<N>(xt, src, out);
        return;
    }
    let src: &[[f32; C]] = as_pixels(src);
    for ((px, &first), weights) in as_pixels_mut::<C>(out).iter_mut().zip(&xt.first).zip(xt.weights.chunks_exact(N)) {
        let taps: &[[f32; C]; N] = src[first as usize..first as usize + N].try_into().expect("N taps");
        let mut acc = [0f32; C];
        for k in 0..N {
            for c in 0..C {
                acc[c] += weights[k] * taps[k][c];
            }
        }
        *px = acc;
    }
}

fn taps_any<const C: usize>(xt: &Taps, src: &[f32], out: &mut [f32]) {
    let src: &[[f32; C]] = as_pixels(src);
    for (o, px) in as_pixels_mut::<C>(out).iter_mut().enumerate() {
        let (first, weights) = xt.get(o);
        let mut acc = [0f32; C];
        for (s, &w) in src[first..first + weights.len()].iter().zip(weights) {
            for c in 0..C {
                acc[c] += w * s[c];
            }
        }
        *px = acc;
    }
}

/// Straight BGRA bytes to premultiplied `[b·a, g·a, r·a, a]` floats.
pub(crate) fn premultiply(level: Level, bgra: &[u8], out: &mut [f32]) {
    debug_assert_eq!(bgra.len(), out.len());
    #[cfg(target_arch = "x86_64")]
    let done = match level {
        // SAFETY: AVX2 was detected (it is the level).
        Level::Avx2 => unsafe { avx2::premultiply(bgra, out) },
        _ => sse::premultiply(bgra, out),
    };
    #[cfg(not(target_arch = "x86_64"))]
    let done = {
        let _ = level;
        0
    };
    for (p, s) in as_pixels_mut::<4>(&mut out[done * 4..]).iter_mut().zip(bgra[done * 4..].chunks_exact(4)) {
        let a = s[3] as f32;
        *p = [s[0] as f32 * a, s[1] as f32 * a, s[2] as f32 * a, a];
    }
}

/// Premultiplied floats (colour × alpha, alpha 0..255) to straight BGRA
/// bytes, rounded.
pub(crate) fn unpremultiply(row: &[f32], out: &mut [u8]) {
    debug_assert_eq!(row.len(), out.len());
    #[cfg(target_arch = "x86_64")]
    let done = sse::unpremultiply(row, out);
    #[cfg(not(target_arch = "x86_64"))]
    let done = 0;
    for (p, o) in as_pixels::<4>(&row[done * 4..]).iter().zip(out[done * 4..].chunks_exact_mut(4)) {
        let a = p[3];
        let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
        // `as u8` truncates and saturates; the + 0.5 rounds (all are >= 0).
        o[0] = (p[0] * inv + 0.5) as u8;
        o[1] = (p[1] * inv + 0.5) as u8;
        o[2] = (p[2] * inv + 0.5) as u8;
        o[3] = (a + 0.5) as u8;
    }
}

/// The 4-channel loops in SSE2 (x86_64's baseline, so no detection), one
/// pixel to a register: the same operations in the same order as the scalar
/// loops, so the same results.
#[cfg(target_arch = "x86_64")]
mod sse {
    use super::Taps;
    use std::arch::x86_64::*;

    /// [`super::accumulate`] of BGRA bytes, a pixel to a register. Returns
    /// the floats done.
    pub fn accumulate_premultiplied(acc: &mut [f32], weight: f32, bytes: &[u8]) -> usize {
        let n = bytes.len().min(acc.len()) / 4;
        // SAFETY: SSE2 is the baseline; 4 pixels at a time while 4 are left
        // in both slices.
        unsafe {
            let zero = _mm_setzero_si128();
            let colour = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
            let one_w = _mm_set_ps(1.0, 0.0, 0.0, 0.0);
            let w = _mm_set1_ps(weight);
            let mut i = 0;
            while i + 4 <= n {
                let v = _mm_loadu_si128(bytes.as_ptr().add(4 * i).cast());
                let lo = _mm_unpacklo_epi8(v, zero);
                let hi = _mm_unpackhi_epi8(v, zero);
                let px = [_mm_unpacklo_epi16(lo, zero), _mm_unpackhi_epi16(lo, zero), _mm_unpacklo_epi16(hi, zero), _mm_unpackhi_epi16(hi, zero)];
                for (k, p) in px.into_iter().enumerate() {
                    let f = _mm_cvtepi32_ps(p);
                    let m = _mm_or_ps(_mm_and_ps(_mm_shuffle_ps::<0xFF>(f, f), colour), one_w);
                    let dst = acc.as_mut_ptr().add(4 * (i + k));
                    _mm_storeu_ps(dst, _mm_add_ps(_mm_loadu_ps(dst), _mm_mul_ps(w, _mm_mul_ps(f, m))));
                }
                i += 4;
            }
            4 * i
        }
    }

    /// Returns the pixels done.
    pub fn premultiply(bgra: &[u8], out: &mut [f32]) -> usize {
        let n = bgra.len() / 4;
        // SAFETY: SSE2 is x86_64's baseline; loads and stores stay inside
        // the slices (4 pixels at a time while 4 are left).
        unsafe {
            let zero = _mm_setzero_si128();
            let colour = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
            let one_w = _mm_set_ps(1.0, 0.0, 0.0, 0.0);
            let mut i = 0;
            while i + 4 <= n {
                let v = _mm_loadu_si128(bgra.as_ptr().add(4 * i).cast());
                let lo = _mm_unpacklo_epi8(v, zero);
                let hi = _mm_unpackhi_epi8(v, zero);
                let px = [_mm_unpacklo_epi16(lo, zero), _mm_unpackhi_epi16(lo, zero), _mm_unpacklo_epi16(hi, zero), _mm_unpackhi_epi16(hi, zero)];
                for (k, p) in px.into_iter().enumerate() {
                    let f = _mm_cvtepi32_ps(p);
                    // [a, a, a, 1]
                    let m = _mm_or_ps(_mm_and_ps(_mm_shuffle_ps::<0xFF>(f, f), colour), one_w);
                    _mm_storeu_ps(out.as_mut_ptr().add(4 * (i + k)), _mm_mul_ps(f, m));
                }
                i += 4;
            }
            i
        }
    }

    pub fn taps4<const N: usize>(xt: &Taps, src: &[f32], out: &mut [f32]) {
        let n_src = src.len() / 4;
        let n_dst = out.len() / 4;
        assert!(xt.first.len() >= n_dst && xt.weights.len() >= n_dst * N);
        assert!(xt.first[..n_dst].iter().all(|&f| f as usize + N <= n_src));
        // SAFETY: SSE2 is the baseline; every tap is inside `src` and every
        // weight inside `xt.weights` (asserted), each store inside `out`.
        unsafe {
            let w = xt.weights.as_ptr();
            for o in 0..n_dst {
                let s = src.as_ptr().add(4 * *xt.first.get_unchecked(o) as usize);
                let mut acc = _mm_setzero_ps();
                for k in 0..N {
                    acc = _mm_add_ps(acc, _mm_mul_ps(_mm_set1_ps(*w.add(o * N + k)), _mm_loadu_ps(s.add(4 * k))));
                }
                _mm_storeu_ps(out.as_mut_ptr().add(4 * o), acc);
            }
        }
    }

    /// Returns the pixels done.
    pub fn unpremultiply(row: &[f32], out: &mut [u8]) -> usize {
        let n = row.len() / 4;
        // SAFETY: SSE2 is the baseline; one pixel in, 4 bytes out, inside both.
        unsafe {
            let colour = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
            let one = _mm_set1_ps(1.0);
            let half = _mm_set1_ps(0.5);
            let zero = _mm_setzero_ps();
            for i in 0..n {
                let p = _mm_loadu_ps(row.as_ptr().add(4 * i));
                let a = _mm_shuffle_ps::<0xFF>(p, p);
                // 1 / a where a > 0, else 0 (the division's inf is masked).
                let inv = _mm_and_ps(_mm_div_ps(one, a), _mm_cmpgt_ps(a, zero));
                // [c·inv, c·inv, c·inv, a]
                let v = _mm_or_ps(_mm_and_ps(_mm_mul_ps(p, inv), colour), _mm_andnot_ps(colour, p));
                let q = _mm_cvttps_epi32(_mm_add_ps(v, half));
                let b = _mm_packus_epi16(_mm_packs_epi32(q, q), q);
                let px = _mm_cvtsi128_si32(b) as u32;
                out.as_mut_ptr().add(4 * i).cast::<u32>().write_unaligned(px);
            }
            n
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    /// [`super::accumulate`] of BGRA bytes, two pixels to a register: the
    /// scalar loop's operations, so its results. Returns the floats done.
    #[target_feature(enable = "avx2")]
    pub unsafe fn accumulate_premultiplied(acc: &mut [f32], weight: f32, bytes: &[u8]) -> usize {
        let n = bytes.len().min(acc.len()) / 4;
        let colour = _mm256_castsi256_ps(_mm256_set_epi32(0, -1, -1, -1, 0, -1, -1, -1));
        let one_w = _mm256_set_ps(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0);
        let w = _mm256_set1_ps(weight);
        let mut i = 0;
        while i + 2 <= n {
            let f = _mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(_mm_loadl_epi64(bytes.as_ptr().add(4 * i).cast())));
            let m = _mm256_or_ps(_mm256_and_ps(_mm256_shuffle_ps::<0xFF>(f, f), colour), one_w);
            let dst = acc.as_mut_ptr().add(4 * i);
            _mm256_storeu_ps(dst, _mm256_add_ps(_mm256_loadu_ps(dst), _mm256_mul_ps(w, _mm256_mul_ps(f, m))));
            i += 2;
        }
        4 * i
    }

    /// [`super::accumulate`] of single bytes: the scalar loop, compiled for
    /// AVX2. Returns the floats done.
    #[target_feature(enable = "avx2")]
    pub unsafe fn accumulate_plain(acc: &mut [f32], weight: f32, bytes: &[u8]) -> usize {
        for (a, &s) in acc.iter_mut().zip(bytes) {
            *a += weight * s as f32;
        }
        acc.len().min(bytes.len())
    }

    /// [`super::premultiply`], two pixels to a register. Returns the pixels
    /// done.
    #[target_feature(enable = "avx2")]
    pub unsafe fn premultiply(bgra: &[u8], out: &mut [f32]) -> usize {
        let n = bgra.len() / 4;
        let colour = _mm256_castsi256_ps(_mm256_set_epi32(0, -1, -1, -1, 0, -1, -1, -1));
        let one_w = _mm256_set_ps(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0);
        let mut i = 0;
        while i + 4 <= n {
            let v = _mm_loadu_si128(bgra.as_ptr().add(4 * i).cast());
            for (k, p) in [_mm256_cvtepu8_epi32(v), _mm256_cvtepu8_epi32(_mm_unpackhi_epi64(v, v))].into_iter().enumerate() {
                let f = _mm256_cvtepi32_ps(p);
                // Per 128-bit lane (a pixel): [a, a, a, 1].
                let m = _mm256_or_ps(_mm256_and_ps(_mm256_shuffle_ps::<0xFF>(f, f), colour), one_w);
                _mm256_storeu_ps(out.as_mut_ptr().add(4 * (i + 2 * k)), _mm256_mul_ps(f, m));
            }
            i += 4;
        }
        i
    }
}

pub(crate) fn as_pixels<const C: usize>(row: &[f32]) -> &[[f32; C]] {
    let (pixels, rest) = row.as_chunks::<C>();
    debug_assert!(rest.is_empty());
    pixels
}

pub(crate) fn as_pixels_mut<const C: usize>(row: &mut [f32]) -> &mut [[f32; C]] {
    let (pixels, rest) = row.as_chunks_mut::<C>();
    debug_assert!(rest.is_empty());
    pixels
}
