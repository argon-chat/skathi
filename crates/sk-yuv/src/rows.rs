//! The row kernels: chroma upsampling, and Y, U, V (+ A) to BGRA. Each has a scalar version and, on
//! x86_64, SSE2 and AVX2 versions that produce the same bytes (the tests compare them on every
//! width). The arithmetic is chosen so the SIMD versions can do it exactly:
//!
//! * Chroma upsampling is libyuv's bilinear filter, `(9a + 3b + 3c + d + 8) >> 4`, in 16-bit integers.
//! * The matrix is in Q13 ([`SHIFT`]): `R = (ky·y + RND + krv·v) >> 13` and alike, with
//!   `y = Y - yoff`, `u = U - 128`, `v = V - 128`. Each pair of products is one `pmaddwd`; 13
//!   fractional bits keep the largest coefficient (BT.2020 studio range `kbu`, 2.14) inside an i16.
//!   The result is the exact conversion rounded, or one off for a value within ~0.01 of a half.
//!
//! SSSE3 is not a level: these kernels need nothing past SSE2, and AVX2 doubles the width.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Scalar,
    Sse2,
    Avx2,
}

impl Level {
    pub fn best() -> Level {
        *Level::available().last().expect("scalar")
    }

    pub fn available() -> Vec<Level> {
        #[allow(unused_mut)]
        let mut levels = vec![Level::Scalar];
        #[cfg(target_arch = "x86_64")]
        {
            levels.push(Level::Sse2);
            if std::arch::is_x86_feature_detected!("avx2") {
                levels.push(Level::Avx2);
            }
        }
        levels
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Scalar => "scalar",
            Level::Sse2 => "sse2",
            Level::Avx2 => "avx2",
        }
    }
}

pub(crate) const SHIFT: u32 = 13;
const RND: i32 = 1 << (SHIFT - 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Coeffs {
    pub yoff: i16,
    pub ky: i16,
    pub krv: i16,
    pub kgu: i16,
    pub kgv: i16,
    pub kbu: i16,
}

/// Upsamples one chroma row to luma resolution: `near` is the chroma row a luma row sits in, `far`
/// the one on its other side (centre siting: luma row `2j` is 3/4 row `j` and 1/4 row `j - 1`), and
/// horizontally the same. `near.len() == far.len() == cw`, `tmp.len() >= cw + 2`, `out.len()` is
/// the luma width, `2cw - 1` or `2cw`.
pub(crate) fn chroma_row(level: Level, near: &[u8], far: &[u8], tmp: &mut [u16], out: &mut [u8]) {
    let cw = near.len();
    assert!(far.len() == cw && tmp.len() >= cw + 2 && out.len().div_ceil(2) == cw);
    for j in simd::vblend(level, near, far, tmp)..cw {
        tmp[1 + j] = 3 * near[j] as u16 + far[j] as u16;
    }
    tmp[0] = tmp[1];
    tmp[cw + 1] = tmp[cw];
    let tmp = &tmp[..cw + 2];
    let w = out.len();
    for j in simd::hblend(level, tmp, out)..cw {
        let m = 3 * tmp[1 + j];
        out[2 * j] = ((m + tmp[j] + 8) >> 4) as u8;
        if 2 * j + 1 < w {
            out[2 * j + 1] = ((m + tmp[2 + j] + 8) >> 4) as u8;
        }
    }
}

/// Converts one row: `y`, `u`, `v` (chroma at luma resolution) and `a` (or 255) to BGRA.
pub(crate) fn yuva_row(
    level: Level,
    k: &Coeffs,
    y: &[u8],
    u: &[u8],
    v: &[u8],
    a: Option<&[u8]>,
    out: &mut [u8],
) {
    let w = y.len();
    assert!(u.len() == w && v.len() == w && a.is_none_or(|a| a.len() == w) && out.len() == 4 * w);
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    for i in simd::yuva(level, k, y, u, v, a, out)..w {
        let yy = y[i] as i32 - k.yoff as i32;
        let uu = u[i] as i32 - 128;
        let vv = v[i] as i32 - 128;
        let yr = k.ky as i32 * yy + RND;
        out[4 * i] = clamp((yr + k.kbu as i32 * uu) >> SHIFT);
        out[4 * i + 1] = clamp((yr - k.kgu as i32 * uu - k.kgv as i32 * vv) >> SHIFT);
        out[4 * i + 2] = clamp((yr + k.krv as i32 * vv) >> SHIFT);
        out[4 * i + 3] = a.map_or(255, |a| a[i]);
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod simd {
    use super::*;

    pub fn vblend(_: Level, _: &[u8], _: &[u8], _: &mut [u16]) -> usize {
        0
    }

    pub fn hblend(_: Level, _: &[u16], _: &mut [u8]) -> usize {
        0
    }

    pub fn yuva(
        _: Level,
        _: &Coeffs,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: Option<&[u8]>,
        _: &mut [u8],
    ) -> usize {
        0
    }
}

#[cfg(target_arch = "x86_64")]
mod simd {
    use super::*;

    // SAFETY (each dispatch): a level other than Scalar was detected on this
    // CPU (`Level::available`), and the kernels read and write only inside
    // the slices they are given.

    pub fn vblend(level: Level, near: &[u8], far: &[u8], tmp: &mut [u16]) -> usize {
        match level {
            Level::Avx2 => unsafe { vblend_avx2(near, far, tmp) },
            Level::Sse2 => unsafe { vblend_sse2(near, far, tmp) },
            Level::Scalar => 0,
        }
    }

    pub fn hblend(level: Level, tmp: &[u16], out: &mut [u8]) -> usize {
        match level {
            Level::Avx2 => unsafe { hblend_avx2(tmp, out) },
            Level::Sse2 => unsafe { hblend_sse2(tmp, out) },
            Level::Scalar => 0,
        }
    }

    pub fn yuva(
        level: Level,
        k: &Coeffs,
        y: &[u8],
        u: &[u8],
        v: &[u8],
        a: Option<&[u8]>,
        out: &mut [u8],
    ) -> usize {
        match level {
            Level::Avx2 => unsafe { yuva_avx2(k, y, u, v, a, out) },
            Level::Sse2 => unsafe { yuva_sse2(k, y, u, v, a, out) },
            Level::Scalar => 0,
        }
    }

    fn pair(lo: i16, hi: i16) -> i32 {
        (lo as u16 as u32 | (hi as u16 as u32) << 16) as i32
    }

    #[target_feature(enable = "sse2")]
    unsafe fn vblend_sse2(near: &[u8], far: &[u8], tmp: &mut [u16]) -> usize {
        unsafe {
            let zero = _mm_setzero_si128();
            let mut i = 0;
            while i + 8 <= near.len() {
                let n = _mm_unpacklo_epi8(_mm_loadl_epi64(near.as_ptr().add(i).cast()), zero);
                let f = _mm_unpacklo_epi8(_mm_loadl_epi64(far.as_ptr().add(i).cast()), zero);
                let r = _mm_add_epi16(_mm_add_epi16(n, _mm_add_epi16(n, n)), f);
                _mm_storeu_si128(tmp.as_mut_ptr().add(1 + i).cast(), r);
                i += 8;
            }
            i
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn vblend_avx2(near: &[u8], far: &[u8], tmp: &mut [u16]) -> usize {
        unsafe {
            let mut i = 0;
            while i + 16 <= near.len() {
                let n = _mm256_cvtepu8_epi16(_mm_loadu_si128(near.as_ptr().add(i).cast()));
                let f = _mm256_cvtepu8_epi16(_mm_loadu_si128(far.as_ptr().add(i).cast()));
                let r = _mm256_add_epi16(_mm256_add_epi16(n, _mm256_add_epi16(n, n)), f);
                _mm256_storeu_si256(tmp.as_mut_ptr().add(1 + i).cast(), r);
                i += 16;
            }
            i
        }
    }

    #[target_feature(enable = "sse2")]
    unsafe fn hblend_sse2(tmp: &[u16], out: &mut [u8]) -> usize {
        unsafe {
            let cw = tmp.len() - 2;
            let eight = _mm_set1_epi16(8);
            let mut i = 0;
            while i + 8 <= cw && 2 * (i + 8) <= out.len() {
                let l = _mm_loadu_si128(tmp.as_ptr().add(i).cast());
                let m = _mm_loadu_si128(tmp.as_ptr().add(i + 1).cast());
                let r = _mm_loadu_si128(tmp.as_ptr().add(i + 2).cast());
                let m3 = _mm_add_epi16(_mm_add_epi16(m, _mm_add_epi16(m, m)), eight);
                let e = _mm_srli_epi16(_mm_add_epi16(m3, l), 4);
                let o = _mm_srli_epi16(_mm_add_epi16(m3, r), 4);
                let px = _mm_packus_epi16(_mm_unpacklo_epi16(e, o), _mm_unpackhi_epi16(e, o));
                _mm_storeu_si128(out.as_mut_ptr().add(2 * i).cast(), px);
                i += 8;
            }
            i
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn hblend_avx2(tmp: &[u16], out: &mut [u8]) -> usize {
        unsafe {
            let cw = tmp.len() - 2;
            let eight = _mm256_set1_epi16(8);
            let mut i = 0;
            while i + 16 <= cw && 2 * (i + 16) <= out.len() {
                let l = _mm256_loadu_si256(tmp.as_ptr().add(i).cast());
                let m = _mm256_loadu_si256(tmp.as_ptr().add(i + 1).cast());
                let r = _mm256_loadu_si256(tmp.as_ptr().add(i + 2).cast());
                let m3 = _mm256_add_epi16(_mm256_add_epi16(m, _mm256_add_epi16(m, m)), eight);
                let e = _mm256_srli_epi16(_mm256_add_epi16(m3, l), 4);
                let o = _mm256_srli_epi16(_mm256_add_epi16(m3, r), 4);
                // Per 128-bit lane, (e, o) interleaved and packed: lane 0 is
                // chroma 0..8 (luma 0..16), lane 1 chroma 8..16. In order.
                let px =
                    _mm256_packus_epi16(_mm256_unpacklo_epi16(e, o), _mm256_unpackhi_epi16(e, o));
                _mm256_storeu_si256(out.as_mut_ptr().add(2 * i).cast(), px);
                i += 16;
            }
            i
        }
    }

    macro_rules! yuva_body {
        ($k:expr_2021, $yy:expr_2021, $uu:expr_2021, $vv:expr_2021, $aa:expr_2021,
         $zero:expr_2021, $one:expr_2021, $max:expr_2021, $ky:expr_2021, $kr:expr_2021, $kg:expr_2021, $kb:expr_2021,
         $unpacklo:ident, $unpackhi:ident, $madd:ident, $add32:ident, $srai:ident,
         $packs:ident, $max16:ident, $min16:ident, $or:ident, $slli:ident) => {{
            let yr_lo = $madd($unpacklo($yy, $one), $ky);
            let yr_hi = $madd($unpackhi($yy, $one), $ky);
            let uv_lo = $unpacklo($uu, $vv);
            let uv_hi = $unpackhi($uu, $vv);
            macro_rules! channel {
                ($kc:expr_2021) => {
                    $min16(
                        $max16(
                            $packs(
                                $srai::<{ SHIFT as i32 }>($add32(yr_lo, $madd(uv_lo, $kc))),
                                $srai::<{ SHIFT as i32 }>($add32(yr_hi, $madd(uv_hi, $kc))),
                            ),
                            $zero,
                        ),
                        $max,
                    )
                };
            }
            let r = channel!($kr);
            let g = channel!($kg);
            let b = channel!($kb);
            let bg = $or(b, $slli::<8>(g));
            let ra = $or(r, $slli::<8>($aa));
            ($unpacklo(bg, ra), $unpackhi(bg, ra))
        }};
    }

    #[target_feature(enable = "sse2")]
    unsafe fn yuva_sse2(
        k: &Coeffs,
        y: &[u8],
        u: &[u8],
        v: &[u8],
        a: Option<&[u8]>,
        out: &mut [u8],
    ) -> usize {
        unsafe {
            let zero = _mm_setzero_si128();
            let one = _mm_set1_epi16(1);
            let max = _mm_set1_epi16(255);
            let yoff = _mm_set1_epi16(k.yoff);
            let c128 = _mm_set1_epi16(128);
            let ky = _mm_set1_epi32(pair(k.ky, RND as i16));
            let kr = _mm_set1_epi32(pair(0, k.krv));
            let kg = _mm_set1_epi32(pair(-k.kgu, -k.kgv));
            let kb = _mm_set1_epi32(pair(k.kbu, 0));
            macro_rules! load {
                ($s:expr_2021, $x:expr_2021) => {
                    _mm_unpacklo_epi8(_mm_loadl_epi64($s.as_ptr().add($x).cast()), zero)
                };
            }
            let mut x = 0;
            while x + 8 <= y.len() {
                let yy = _mm_sub_epi16(load!(y, x), yoff);
                let uu = _mm_sub_epi16(load!(u, x), c128);
                let vv = _mm_sub_epi16(load!(v, x), c128);
                let aa = match a {
                    Some(a) => load!(a, x),
                    None => max,
                };
                let (lo, hi) = yuva_body!(
                    k,
                    yy,
                    uu,
                    vv,
                    aa,
                    zero,
                    one,
                    max,
                    ky,
                    kr,
                    kg,
                    kb,
                    _mm_unpacklo_epi16,
                    _mm_unpackhi_epi16,
                    _mm_madd_epi16,
                    _mm_add_epi32,
                    _mm_srai_epi32,
                    _mm_packs_epi32,
                    _mm_max_epi16,
                    _mm_min_epi16,
                    _mm_or_si128,
                    _mm_slli_epi16
                );
                let dst = out.as_mut_ptr().add(4 * x);
                _mm_storeu_si128(dst.cast(), lo);
                _mm_storeu_si128(dst.add(16).cast(), hi);
                x += 8;
            }
            x
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn yuva_avx2(
        k: &Coeffs,
        y: &[u8],
        u: &[u8],
        v: &[u8],
        a: Option<&[u8]>,
        out: &mut [u8],
    ) -> usize {
        unsafe {
            let zero = _mm256_setzero_si256();
            let one = _mm256_set1_epi16(1);
            let max = _mm256_set1_epi16(255);
            let yoff = _mm256_set1_epi16(k.yoff);
            let c128 = _mm256_set1_epi16(128);
            let ky = _mm256_set1_epi32(pair(k.ky, RND as i16));
            let kr = _mm256_set1_epi32(pair(0, k.krv));
            let kg = _mm256_set1_epi32(pair(-k.kgu, -k.kgv));
            let kb = _mm256_set1_epi32(pair(k.kbu, 0));
            macro_rules! load {
                ($s:expr_2021, $x:expr_2021) => {
                    _mm256_cvtepu8_epi16(_mm_loadu_si128($s.as_ptr().add($x).cast()))
                };
            }
            let mut x = 0;
            while x + 16 <= y.len() {
                let yy = _mm256_sub_epi16(load!(y, x), yoff);
                let uu = _mm256_sub_epi16(load!(u, x), c128);
                let vv = _mm256_sub_epi16(load!(v, x), c128);
                let aa = match a {
                    Some(a) => load!(a, x),
                    None => max,
                };
                // unpacklo/hi work per 128-bit lane, so the packs and the last unpack restore pixel
                // order from (lo: 0..4, 8..12; hi: 4..8, 12..16).
                let (lo, hi) = yuva_body!(
                    k,
                    yy,
                    uu,
                    vv,
                    aa,
                    zero,
                    one,
                    max,
                    ky,
                    kr,
                    kg,
                    kb,
                    _mm256_unpacklo_epi16,
                    _mm256_unpackhi_epi16,
                    _mm256_madd_epi16,
                    _mm256_add_epi32,
                    _mm256_srai_epi32,
                    _mm256_packs_epi32,
                    _mm256_max_epi16,
                    _mm256_min_epi16,
                    _mm256_or_si256,
                    _mm256_slli_epi16
                );
                let dst = out.as_mut_ptr().add(4 * x);
                _mm256_storeu_si256(dst.cast(), _mm256_permute2x128_si256::<0x20>(lo, hi));
                _mm256_storeu_si256(
                    dst.add(32).cast(),
                    _mm256_permute2x128_si256::<0x31>(lo, hi),
                );
                x += 16;
            }
            x
        }
    }
}
