//! sk-yuv: an I420 frame, with an optional alpha plane, to STRAIGHT BGRA at any size. The video
//! stickers' pixel kernel (`sk-sticker`'s `video`).
//!
//! * **Chroma** is upsampled bilinearly, centre-sited (weights 9, 3, 3, 1 over 16), as a GPU samples
//!   a `<video>` and as libyuv does.
//! * **The matrix** is the caller's [`Colorimetry`]. Studio range maps Y 16..235 and C 16..240 to
//!   0..255, full range takes them as they are.
//! * **Primaries and transfer** other than sRGB's (the curves Chromium draws as sRGB's included)
//!   are converted to sRGB in linear light per pixel (`gamut.rs`), on whichever is smaller, the
//!   frame or the target. sRGB's own pair skips it.
//! * **Alpha** is the alpha plane as is, or 255. Colour is never premultiplied in the output.
//! * **Scaling** is separable: an area average when an axis shrinks (or stays), bilinear when it
//!   grows. Both weigh colour by alpha, so a transparent pixel's colour never bleeds into an edge.
//!   At 1:1 nothing is resampled.
//!
//! # Speed
//!
//! The row kernels are SSE2 and AVX2, picked at run time ([`Level`]), with a scalar version that
//! gives the same bytes; the f32 resampling does the same operations in the same order at every
//! level. Rows stream: a [`Converter`] holds a few rows of scratch, never a frame.

mod gamut;
mod rows;
mod scale;

use gamut::Gamut;
pub use rows::Level;
use rows::{chroma_row, yuva_row, Coeffs, SHIFT};
use scale::Scaler;

/// A YUV to RGB matrix: H.273 `MatrixCoefficients`, the ones a video sticker can declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Matrix {
    /// BT.601 (H.273 5 and 6: BT.470BG, SMPTE 170M).
    Bt601,
    Bt709,
    Bt2020,
    Smpte240,
    Fcc,
}

impl Matrix {
    fn kr_kb(self) -> (f64, f64) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
            Matrix::Smpte240 => (0.212, 0.087),
            Matrix::Fcc => (0.30, 0.11),
        }
    }
}

/// H.273 `ColourPrimaries`: the gamut a frame's RGB is in. Anything but [`Primaries::Bt709`] is
/// converted to sRGB's, as Chromium does on an sRGB display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Primaries {
    Bt709,
    Bt470M,
    Bt470Bg,
    Smpte170,
    Smpte240,
    Film,
    Bt2020,
    Smpte428,
    Smpte431,
    P3,
    Ebu3213,
}

/// H.273 `TransferCharacteristics`, as Chromium draws them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transfer {
    /// sRGB (13), and the SDR video curves Chromium draws as sRGB's:
    /// BT.709 (1), SMPTE 170M (6), BT.2020 (14, 15).
    Srgb,
    Gamma22,
    Gamma28,
    Smpte240,
    Linear,
    Smpte428,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colorimetry {
    pub matrix: Matrix,
    pub full_range: bool,
    /// With [`Transfer::Srgb`], [`Primaries::Bt709`] costs nothing; any
    /// other pair is converted to sRGB per pixel (module `gamut`).
    pub primaries: Primaries,
    pub transfer: Transfer,
}

impl Colorimetry {
    pub const fn new(matrix: Matrix, full_range: bool) -> Colorimetry {
        Colorimetry {
            matrix,
            full_range,
            primaries: Primaries::Bt709,
            transfer: Transfer::Srgb,
        }
    }

    fn coeffs(self) -> Coeffs {
        let (kr, kb) = self.matrix.kr_kb();
        let kg = 1.0 - kr - kb;
        let (ky, kc, yoff) = if self.full_range {
            (1.0, 1.0, 0)
        } else {
            (255.0 / 219.0, 255.0 / 224.0, 16)
        };
        let q = |x: f64| (x * (1 << SHIFT) as f64).round() as i16;
        Coeffs {
            yoff,
            ky: q(ky),
            krv: q(2.0 * (1.0 - kr) * kc),
            kgu: q(2.0 * kb * (1.0 - kb) / kg * kc),
            kgv: q(2.0 * kr * (1.0 - kr) / kg * kc),
            kbu: q(2.0 * (1.0 - kb) * kc),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
}

impl<'a> Plane<'a> {
    #[inline]
    fn row(&self, y: usize, width: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + width]
    }

    fn holds(&self, width: usize, height: usize) -> bool {
        self.stride >= width
            && height.checked_sub(1).is_some_and(|h| {
                h.checked_mul(self.stride)
                    .and_then(|n| n.checked_add(width))
                    .is_some_and(|n| n <= self.data.len())
            })
    }
}

/// An I420 frame: Y at `width` × `height`, U and V at half that (rounded up), optional full-size alpha.
#[derive(Clone, Copy, Debug)]
pub struct I420<'a> {
    pub width: u32,
    pub height: u32,
    pub y: Plane<'a>,
    pub u: Plane<'a>,
    pub v: Plane<'a>,
    pub a: Option<Plane<'a>>,
}

/// Why a conversion did not run: the caller's sizes do not add up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Plane(&'static str),
    Target,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Plane(which) => write!(f, "the {which} plane is smaller than the frame"),
            Error::Target => f.write_str("the target is not width × height BGRA pixels"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Converter {
    level: Level,
    tmp: Vec<u16>,
    u: Vec<u8>,
    v: Vec<u8>,
    scaler: Scaler,
    gamut: Option<Gamut>,
}

impl Default for Converter {
    fn default() -> Self {
        Converter::new()
    }
}

impl Converter {
    pub fn new() -> Converter {
        Converter::with_level(Level::best())
    }

    /// At `level`, which must be one of [`Level::available`] (else the best).
    pub fn with_level(level: Level) -> Converter {
        let level = if Level::available().contains(&level) {
            level
        } else {
            Level::best()
        };
        Converter {
            level,
            tmp: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
            scaler: Scaler::default(),
            gamut: None,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn memory_bytes(&self) -> usize {
        self.tmp.capacity() * 2
            + self.u.capacity()
            + self.v.capacity()
            + self.scaler.bytes()
            + self.gamut.as_ref().map_or(0, Gamut::bytes)
    }

    /// `frame` into `pixels`, `width` × `height` straight BGRA, rows packed.
    pub fn to_bgra(
        &mut self,
        frame: &I420,
        colour: Colorimetry,
        pixels: &mut [u8],
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        let (w, h) = (frame.width as usize, frame.height as usize);
        let (tw, th) = (width as usize, height as usize);
        if pixels.len() != tw * th * 4 {
            return Err(Error::Target);
        }
        if w == 0 || h == 0 || tw == 0 || th == 0 {
            pixels.fill(0);
            return Ok(());
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        if !frame.y.holds(w, h) {
            return Err(Error::Plane("Y"));
        }
        if !frame.u.holds(cw, ch) {
            return Err(Error::Plane("U"));
        }
        if !frame.v.holds(cw, ch) {
            return Err(Error::Plane("V"));
        }
        if frame.a.is_some_and(|a| !a.holds(w, h)) {
            return Err(Error::Plane("alpha"));
        }
        let k = colour.coeffs();
        let level = self.level;
        self.tmp.resize(cw + 2, 0);
        self.u.resize(w, 0);
        self.v.resize(w, 0);
        let key = (colour.primaries, colour.transfer);
        if self.gamut.as_ref().is_none_or(|g| g.key != key) {
            self.gamut = Gamut::new(key.0, key.1).or(self.gamut.take());
        }
        let gamut = self.gamut.as_ref().filter(|g| g.key == key);
        // The conversion runs on the fewer pixels: the frame's rows before
        // scaling (growing, or 1:1), or the target's after (shrinking).
        let before = gamut.filter(|_| tw * th >= w * h);
        let after = gamut.filter(|_| tw * th < w * h);
        let Converter {
            tmp, u, v, scaler, ..
        } = self;
        let mut convert = |sy: usize, out: &mut [u8]| {
            let j = sy / 2;
            let far = if sy.is_multiple_of(2) {
                j.saturating_sub(1)
            } else {
                (j + 1).min(ch - 1)
            };
            chroma_row(level, frame.u.row(j, cw), frame.u.row(far, cw), tmp, u);
            chroma_row(level, frame.v.row(j, cw), frame.v.row(far, cw), tmp, v);
            yuva_row(
                level,
                &k,
                frame.y.row(sy, w),
                u,
                v,
                frame.a.map(|a| a.row(sy, w)),
                out,
            );
            if let Some(g) = before {
                g.apply(out);
            }
        };
        if (tw, th) == (w, h) {
            for (sy, out) in pixels.chunks_exact_mut(w * 4).enumerate() {
                convert(sy, out);
            }
            return Ok(());
        }
        scaler.run::<4>(level, (w, h), (tw, th), &mut convert, |oy, row| {
            let out = &mut pixels[oy * tw * 4..(oy + 1) * tw * 4];
            scale::unpremultiply(row, out);
            if let Some(g) = after {
                g.apply(out);
            }
        });
        Ok(())
    }

    /// An alpha plane (`None`: opaque) into `pixels` as B = G = R = 255 and
    /// its alpha, `width` × `height`, rows packed. `size` is the plane's.
    pub fn alpha_to_bgra(
        &mut self,
        alpha: Option<Plane>,
        (w, h): (u32, u32),
        pixels: &mut [u8],
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        let (w, h) = (w as usize, h as usize);
        let (tw, th) = (width as usize, height as usize);
        if pixels.len() != tw * th * 4 {
            return Err(Error::Target);
        }
        let Some(plane) = alpha.filter(|_| w > 0 && h > 0) else {
            pixels.fill(255);
            return Ok(());
        };
        if !plane.holds(w, h) {
            return Err(Error::Plane("alpha"));
        }
        if tw == 0 || th == 0 {
            return Ok(());
        }
        if (tw, th) == (w, h) {
            for (sy, out) in pixels.chunks_exact_mut(w * 4).enumerate() {
                for (p, &a) in out.chunks_exact_mut(4).zip(plane.row(sy, w)) {
                    p.copy_from_slice(&[255, 255, 255, a]);
                }
            }
            return Ok(());
        }
        self.scaler.run::<1>(
            self.level,
            (w, h),
            (tw, th),
            |sy, row| row.copy_from_slice(plane.row(sy, w)),
            |oy, row| {
                for (p, &a) in pixels[oy * tw * 4..(oy + 1) * tw * 4]
                    .chunks_exact_mut(4)
                    .zip(row)
                {
                    p.copy_from_slice(&[255, 255, 255, (a + 0.5) as u8]);
                }
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
