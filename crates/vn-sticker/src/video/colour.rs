//! Which YUV matrix and range a frame is read with: what Chromium shows on
//! screen for a `<video src>` of the same file, which is how Argon's client
//! shows its video stickers (`StickerView.vue`).
//!
//! Chromium (media/ffmpeg/ffmpeg_common.cc, AVStreamToVideoDecoderConfig,
//! then media/filters/vpx_video_decoder.cc, read 2026-11):
//!
//! 1. The container's `Colour`, when its MatrixCoefficients,
//!    TransferCharacteristics and Primaries are all present and all known
//!    (none of them 2, "unspecified", nor a reserved value): its matrix, and
//!    full range when `Range` is 2, else studio range.
//! 2. Otherwise the VP9 header's `color_space` and `color_range`: BT.601 and
//!    SMPTE 170M read as BT.601, SMPTE 240M, BT.709 and BT.2020 as
//!    themselves.
//! 3. A VP9 `color_space` of 0 (unknown) or 6 (reserved) gives the frame no
//!    colour space, whatever its `color_range` says. On screen, Chrome 156
//!    on Windows with a GPU then reads a frame with alpha as BT.601 studio
//!    range, and an opaque one as BT.709 studio range: opaque frames go
//!    through the GpuMemoryBuffer frame pool, frames with alpha never do.
//!    Without GPU compositing (and in a canvas `drawImage`) both are BT.601.
//!    Argon's desktop client runs with a GPU, so this follows that.
//!
//! Most stickers land in 2 or 3: ffmpeg writes a `Colour` with a `Range`
//! only, and libvpx-vp9's default `color_space` is unknown. Of the nine emoji
//! in Argon's emojix playground, five are BT.601 studio range and two (no
//! alpha) BT.709 by rule 3, one BT.601 and one BT.709 by rule 1.
//!
//! Checked against Chrome 156 (headless, GPU on, screenshots of the playing
//! `<video>` over black; research/reports/m26/stickers-video-1.md): the nine
//! emoji and `testdata/alpha.webm`, every channel within 2 levels and 99.87
//! to 99.96 % within 1, translucent pixels included; except the one emoji
//! whose primaries Chrome converts (below).
//!
//! Not followed:
//! - Primaries. Chromium converts a colour space with primaries other than
//!   BT.709's to sRGB's, in linear light with the sRGB curve both ways (it
//!   treats SDR transfers as sRGB). That is SMPTE 170M, BT.470, SMPTE 240M,
//!   BT.2020 and the rest, from rule 1 or 2; an unknown colour space is not
//!   converted. Not doing so costs up to 10 levels on such a file (mean 2.7
//!   on the one playground emoji tagged SMPTE 170M).
//! - A container matrix of 0 (GBR), 8 (YCoCg) or 11 (YDzDx), which Chromium
//!   applies, reads as BT.601 here (no sticker writer produces them for
//!   4:2:0).

use vn_vpx_sys::ColorSpace;
use vn_yuv::{Colorimetry, Matrix};

use super::webm::Colour;

/// H.273 code points Chromium knows (media/base/video_color_space.cc).
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

/// `alpha`: the stream has an alpha stream (Chromium's frames are I420A).
pub(crate) fn colorimetry(container: Option<&Colour>, cs: ColorSpace, full_range: bool, alpha: bool) -> Colorimetry {
    if let Some(c) = container {
        if let (Some(m), Some(t), Some(p)) = (c.matrix, c.transfer, c.primaries) {
            if known_matrix(m) && known_transfer(t) && known_primaries(p) {
                return Colorimetry { matrix: matrix(m), full_range: c.range == Some(2) };
            }
        }
    }
    let matrix = match cs {
        ColorSpace::Bt601 | ColorSpace::Smpte170 | ColorSpace::Srgb => Matrix::Bt601,
        ColorSpace::Smpte240 => Matrix::Smpte240,
        ColorSpace::Bt709 => Matrix::Bt709,
        ColorSpace::Bt2020 => Matrix::Bt2020,
        ColorSpace::Unknown | ColorSpace::Reserved => {
            let matrix = if alpha { Matrix::Bt601 } else { Matrix::Bt709 };
            return Colorimetry { matrix, full_range: false };
        }
    };
    Colorimetry { matrix, full_range }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour(matrix: Option<u64>, transfer: Option<u64>, primaries: Option<u64>, range: Option<u64>) -> Colour {
        Colour { matrix, transfer, primaries, range }
    }

    const fn c(matrix: Matrix, full_range: bool) -> Colorimetry {
        Colorimetry { matrix, full_range }
    }

    #[test]
    fn a_complete_container_colour_wins() {
        for alpha in [true, false] {
            let bt709 = colour(Some(1), Some(1), Some(1), None);
            assert_eq!(colorimetry(Some(&bt709), ColorSpace::Bt601, true, alpha), c(Matrix::Bt709, false), "no Range: studio");
            let full = colour(Some(6), Some(1), Some(6), Some(2));
            assert_eq!(colorimetry(Some(&full), ColorSpace::Unknown, false, alpha), c(Matrix::Bt601, true));
            let bt2020 = colour(Some(9), Some(16), Some(9), Some(1));
            assert_eq!(colorimetry(Some(&bt2020), ColorSpace::Bt709, false, alpha), c(Matrix::Bt2020, false));
            let fcc = colour(Some(4), Some(4), Some(4), None);
            assert_eq!(colorimetry(Some(&fcc), ColorSpace::Unknown, false, alpha), c(Matrix::Fcc, false));
            let bt240 = colour(Some(7), Some(7), Some(7), Some(3));
            assert_eq!(colorimetry(Some(&bt240), ColorSpace::Unknown, false, alpha), c(Matrix::Smpte240, false), "Range 3: studio");
        }
    }

    #[test]
    fn an_incomplete_or_unknown_container_colour_defers_to_the_bitstream() {
        for alpha in [true, false] {
            // Argon's emoji: a Range alone.
            let range_only = colour(None, None, None, Some(1));
            assert_eq!(colorimetry(Some(&range_only), ColorSpace::Bt709, false, alpha), c(Matrix::Bt709, false));
            assert_eq!(colorimetry(Some(&range_only), ColorSpace::Smpte170, true, alpha), c(Matrix::Bt601, true));
            assert_eq!(colorimetry(None, ColorSpace::Bt2020, false, alpha), c(Matrix::Bt2020, false));
            assert_eq!(colorimetry(None, ColorSpace::Smpte240, true, alpha), c(Matrix::Smpte240, true));
            // "Unspecified" (2) and reserved values are not a colour space.
            let unspecified = colour(Some(1), Some(2), Some(1), Some(2));
            assert_eq!(colorimetry(Some(&unspecified), ColorSpace::Bt709, true, alpha), c(Matrix::Bt709, true));
            let reserved = colour(Some(3), Some(1), Some(1), Some(2));
            assert_eq!(colorimetry(Some(&reserved), ColorSpace::Bt601, false, alpha), c(Matrix::Bt601, false));
            let reserved = colour(Some(1), Some(1), Some(13), None);
            assert_eq!(colorimetry(Some(&reserved), ColorSpace::Bt709, true, alpha), c(Matrix::Bt709, true));
        }
    }

    #[test]
    fn no_colour_space_is_studio_range_bt601_with_alpha_and_bt709_without() {
        let range_full = colour(None, None, None, Some(2));
        for (alpha, matrix) in [(true, Matrix::Bt601), (false, Matrix::Bt709)] {
            assert_eq!(colorimetry(None, ColorSpace::Unknown, true, alpha), c(matrix, false));
            assert_eq!(colorimetry(None, ColorSpace::Reserved, true, alpha), c(matrix, false));
            assert_eq!(colorimetry(Some(&range_full), ColorSpace::Unknown, true, alpha), c(matrix, false));
        }
    }
}
