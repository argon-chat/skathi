//! Which YUV matrix, range, primaries and transfer a frame is read with: what Chromium shows for a
//! `<video src>` of the same file, which is how Argon's client shows its video stickers.
//!
//! Chromium's rules (media/ffmpeg/ffmpeg_common.cc, media/filters/vpx_video_decoder.cc,
//! ui/gfx/color_space.cc):
//!
//! 1. The container's `Colour`, when its MatrixCoefficients, TransferCharacteristics and Primaries
//!    are all present and known (none 2, "unspecified", nor reserved): its matrix, primaries and
//!    transfer, and full range when `Range` is 2, else studio range.
//! 2. Otherwise the VP9 header's `color_space` and `color_range`: BT.601 and SMPTE 170M read as
//!    SMPTE 170M, SMPTE 240M, BT.709 and BT.2020 as themselves (an 8-bit BT.2020 frame with
//!    BT.709's transfer).
//! 3. A VP9 `color_space` of 0 (unknown) or 6 (reserved) gives the frame no colour space. Chrome 156
//!    on Windows with a GPU then reads a frame with alpha as BT.601 studio range, and an opaque one
//!    as BT.709 studio range (opaque frames go through the GpuMemoryBuffer frame pool, frames with
//!    alpha never do). Without GPU compositing both are BT.601. Argon's client runs with a GPU.
//!
//! Most stickers land in 2 or 3: ffmpeg writes a `Colour` with a `Range` only, and libvpx-vp9's
//! default `color_space` is unknown.
//!
//! Chromium's compositor moves a frame to the display's colour space (sRGB here), drawing the SDR
//! video curves (BT.709, SMPTE 170M, BT.2020) with the sRGB curve, so a frame in BT.709 primaries
//! with one of those is untouched; other primaries or curves are converted in linear light
//! (sk-yuv's `gamut`). A frame with no colour space is not converted. Checked against Chrome 156
//! (GPU on, screenshots of the playing `<video>` over black): every channel within 2 levels.
//!
//! Not followed: a container matrix of 0 (GBR), 8 (YCoCg) or 11 (YDzDx), and VP9's sRGB colour
//! space, read as BT.601 (no sticker writer produces them for 4:2:0); transfers Chromium has no
//! plain curve for (log, IEC 61966-2-4, BT.1361) or tone-maps (PQ, HLG), read as sRGB's.

use sk_vpx_sys::ColorSpace;
use sk_yuv::{Colorimetry, Matrix, Primaries, Transfer};

use super::webm::Colour;

fn known_matrix(m: u64) -> bool {
    matches!(m, 0 | 1 | 4..=9 | 11)
}

fn known_transfer(t: u64) -> bool {
    t == 1 || (4..=18).contains(&t)
}

fn known_primaries(p: u64) -> bool {
    p == 1 || (4..=12).contains(&p) || p == 22
}

fn matrix(h273: u64) -> Matrix {
    match h273 {
        1 => Matrix::Bt709,
        4 => Matrix::Fcc,
        7 => Matrix::Smpte240,
        9 => Matrix::Bt2020,
        _ => Matrix::Bt601,
    }
}

fn primaries(h273: u64) -> Primaries {
    match h273 {
        4 => Primaries::Bt470M,
        5 => Primaries::Bt470Bg,
        6 => Primaries::Smpte170,
        7 => Primaries::Smpte240,
        8 => Primaries::Film,
        9 => Primaries::Bt2020,
        10 => Primaries::Smpte428,
        11 => Primaries::Smpte431,
        12 => Primaries::P3,
        22 => Primaries::Ebu3213,
        _ => Primaries::Bt709,
    }
}

fn transfer(h273: u64) -> Transfer {
    match h273 {
        4 => Transfer::Gamma22,
        5 => Transfer::Gamma28,
        7 => Transfer::Smpte240,
        8 => Transfer::Linear,
        17 => Transfer::Smpte428,
        // BT.709, SMPTE 170M, sRGB, BT.2020: Chromium's sRGB curve; and the ones it has no plain curve for.
        _ => Transfer::Srgb,
    }
}

pub(crate) fn colorimetry(container: Option<&Colour>, cs: ColorSpace, full_range: bool, alpha: bool) -> Colorimetry {
    if let Some(c) = container {
        if let (Some(m), Some(t), Some(p)) = (c.matrix, c.transfer, c.primaries) {
            if known_matrix(m) && known_transfer(t) && known_primaries(p) {
                return Colorimetry { matrix: matrix(m), full_range: c.range == Some(2), primaries: primaries(p), transfer: transfer(t) };
            }
        }
    }
    let (matrix, primaries, transfer) = match cs {
        ColorSpace::Bt601 | ColorSpace::Smpte170 => (Matrix::Bt601, Primaries::Smpte170, Transfer::Srgb),
        ColorSpace::Srgb => (Matrix::Bt601, Primaries::Bt709, Transfer::Srgb),
        ColorSpace::Smpte240 => (Matrix::Smpte240, Primaries::Smpte240, Transfer::Smpte240),
        ColorSpace::Bt709 => (Matrix::Bt709, Primaries::Bt709, Transfer::Srgb),
        ColorSpace::Bt2020 => (Matrix::Bt2020, Primaries::Bt2020, Transfer::Srgb),
        ColorSpace::Unknown | ColorSpace::Reserved => {
            return Colorimetry::new(if alpha { Matrix::Bt601 } else { Matrix::Bt709 }, false);
        }
    };
    Colorimetry { matrix, full_range, primaries, transfer }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour(matrix: Option<u64>, transfer: Option<u64>, primaries: Option<u64>, range: Option<u64>) -> Colour {
        Colour { matrix, transfer, primaries, range }
    }

    const fn c(matrix: Matrix, full_range: bool, primaries: Primaries, transfer: Transfer) -> Colorimetry {
        Colorimetry { matrix, full_range, primaries, transfer }
    }

    const fn plain(matrix: Matrix, full_range: bool) -> Colorimetry {
        Colorimetry::new(matrix, full_range)
    }

    #[test]
    fn a_complete_container_colour_wins() {
        for alpha in [true, false] {
            let bt709 = colour(Some(1), Some(1), Some(1), None);
            assert_eq!(colorimetry(Some(&bt709), ColorSpace::Bt601, true, alpha), plain(Matrix::Bt709, false), "no Range: studio");
            let emoji1 = colour(Some(6), Some(1), Some(6), Some(1));
            let smpte170 = c(Matrix::Bt601, false, Primaries::Smpte170, Transfer::Srgb);
            assert_eq!(colorimetry(Some(&emoji1), ColorSpace::Smpte170, false, alpha), smpte170);
            let full = colour(Some(6), Some(13), Some(5), Some(2));
            assert_eq!(colorimetry(Some(&full), ColorSpace::Unknown, false, alpha), c(Matrix::Bt601, true, Primaries::Bt470Bg, Transfer::Srgb));
            let bt2020 = colour(Some(9), Some(14), Some(9), Some(1));
            assert_eq!(colorimetry(Some(&bt2020), ColorSpace::Bt709, false, alpha), c(Matrix::Bt2020, false, Primaries::Bt2020, Transfer::Srgb));
            let fcc = colour(Some(4), Some(4), Some(4), None);
            assert_eq!(colorimetry(Some(&fcc), ColorSpace::Unknown, false, alpha), c(Matrix::Fcc, false, Primaries::Bt470M, Transfer::Gamma22));
            let bt240 = colour(Some(7), Some(7), Some(7), Some(3));
            let want = c(Matrix::Smpte240, false, Primaries::Smpte240, Transfer::Smpte240);
            assert_eq!(colorimetry(Some(&bt240), ColorSpace::Unknown, false, alpha), want, "Range 3: studio");
            let p3 = colour(Some(1), Some(8), Some(12), None);
            assert_eq!(colorimetry(Some(&p3), ColorSpace::Unknown, false, alpha), c(Matrix::Bt709, false, Primaries::P3, Transfer::Linear));
            // A transfer Chromium tone-maps (PQ) is drawn with sRGB's here.
            let pq = colour(Some(9), Some(16), Some(22), None);
            assert_eq!(colorimetry(Some(&pq), ColorSpace::Unknown, false, alpha), c(Matrix::Bt2020, false, Primaries::Ebu3213, Transfer::Srgb));
        }
    }

    #[test]
    fn an_incomplete_or_unknown_container_colour_defers_to_the_bitstream() {
        for alpha in [true, false] {
            let range_only = colour(None, None, None, Some(1));
            assert_eq!(colorimetry(Some(&range_only), ColorSpace::Bt709, false, alpha), plain(Matrix::Bt709, false));
            let smpte170 = c(Matrix::Bt601, true, Primaries::Smpte170, Transfer::Srgb);
            assert_eq!(colorimetry(Some(&range_only), ColorSpace::Smpte170, true, alpha), smpte170);
            assert_eq!(colorimetry(None, ColorSpace::Bt601, true, alpha), smpte170, "BT.601 is SMPTE 170M");
            assert_eq!(colorimetry(None, ColorSpace::Bt2020, false, alpha), c(Matrix::Bt2020, false, Primaries::Bt2020, Transfer::Srgb));
            let smpte240 = c(Matrix::Smpte240, true, Primaries::Smpte240, Transfer::Smpte240);
            assert_eq!(colorimetry(None, ColorSpace::Smpte240, true, alpha), smpte240);
            assert_eq!(colorimetry(None, ColorSpace::Srgb, true, alpha), plain(Matrix::Bt601, true));
            let unspecified = colour(Some(1), Some(2), Some(1), Some(2));
            assert_eq!(colorimetry(Some(&unspecified), ColorSpace::Bt709, true, alpha), plain(Matrix::Bt709, true));
            let reserved = colour(Some(3), Some(1), Some(1), Some(2));
            assert_eq!(colorimetry(Some(&reserved), ColorSpace::Bt601, false, alpha), c(Matrix::Bt601, false, Primaries::Smpte170, Transfer::Srgb));
            let reserved = colour(Some(1), Some(1), Some(13), None);
            assert_eq!(colorimetry(Some(&reserved), ColorSpace::Bt709, true, alpha), plain(Matrix::Bt709, true));
        }
    }

    #[test]
    fn no_colour_space_is_studio_range_bt601_with_alpha_and_bt709_without() {
        let range_full = colour(None, None, None, Some(2));
        for (alpha, matrix) in [(true, Matrix::Bt601), (false, Matrix::Bt709)] {
            assert_eq!(colorimetry(None, ColorSpace::Unknown, true, alpha), plain(matrix, false));
            assert_eq!(colorimetry(None, ColorSpace::Reserved, true, alpha), plain(matrix, false));
            assert_eq!(colorimetry(Some(&range_full), ColorSpace::Unknown, true, alpha), plain(matrix, false));
        }
    }
}
