//! vn-yuv: an I420 frame, with an optional alpha plane, to STRAIGHT BGRA at
//! any size. The video stickers' pixel kernel (`vn-sticker`'s `video`).
//!
//! # What it computes
//!
//! * **Chroma** is upsampled to luma resolution bilinearly, centre-sited (a
//!   chroma sample sits between its two luma rows and columns: weights 9, 3,
//!   3, 1 over 16), as a GPU samples the chroma plane of a `<video>` and as
//!   libyuv's bilinear I420 conversion does.
//! * **The matrix** is the caller's [`Colorimetry`] (which one a file gets is
//!   the caller's rule; vn-sticker follows Chromium's). Studio range maps
//!   Y 16..235 and C 16..240 to 0..255, full range takes them as they are.
//! * **Alpha** is the alpha plane as is, or 255. Colour is never
//!   premultiplied in the output.
//! * **Scaling** is separable, per axis: an area average when the axis
//!   shrinks (or stays), bilinear when it grows. Both weigh colour by alpha
//!   (they work on premultiplied values and divide back), so the colour of a
//!   transparent pixel never bleeds into an edge. At 1:1 nothing is
//!   resampled: the conversion writes the target directly.
//!
//! # Speed
//!
//! The row kernels (chroma upsampling, YUV to BGRA) are SSE2 and AVX2,
//! picked at run time ([`Level`]), with a scalar version that gives the same
//! bytes. The resampling is f32, SSE2 one pixel to a register and AVX2 two,
//! the same operations in the same order as its scalar loops, so every level
//! gives the same bytes there too. Rows stream: a [`Converter`] holds a few
//! rows of scratch, never a frame.
//!
//! One thread, AVX2, a 512² frame with alpha (`tests::bench`, the fastest of
//! many runs): 90 µs at 512², 280 µs to 256², 240 µs to 160²; its alpha
//! alone, 60 and 40 µs. A 100² frame: 5 µs at 100², 14 µs to 48². The
//! scalar rows take 1.6 ms at 512².

mod rows;
mod scale;

pub use rows::Level;
use rows::{chroma_row, yuva_row, Coeffs, SHIFT};
use scale::Scaler;

/// A YUV to RGB matrix: H.273 `MatrixCoefficients`, the ones a video sticker
/// can declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Matrix {
    /// BT.601 (H.273 5 and 6: BT.470BG, SMPTE 170M).
    Bt601,
    /// BT.709 (1).
    Bt709,
    /// BT.2020 non-constant luminance (9).
    Bt2020,
    /// SMPTE 240M (7).
    Smpte240,
    /// FCC (4).
    Fcc,
}

impl Matrix {
    /// `(Kr, Kb)`.
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

/// How to read a frame's samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colorimetry {
    pub matrix: Matrix,
    /// Full range (0..255) rather than studio range (Y 16..235, C 16..240).
    pub full_range: bool,
}

impl Colorimetry {
    fn coeffs(self) -> Coeffs {
        let (kr, kb) = self.matrix.kr_kb();
        let kg = 1.0 - kr - kb;
        let (ky, kc, yoff) = if self.full_range { (1.0, 1.0, 0) } else { (255.0 / 219.0, 255.0 / 224.0, 16) };
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

/// One plane: rows `stride` bytes apart.
#[derive(Clone, Copy, Debug)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
}

impl<'a> Plane<'a> {
    /// Row `y`, `width` bytes.
    #[inline]
    fn row(&self, y: usize, width: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + width]
    }

    /// Holds `height` rows of `width` bytes.
    fn holds(&self, width: usize, height: usize) -> bool {
        self.stride >= width && height.checked_sub(1).is_some_and(|h| h.checked_mul(self.stride).and_then(|n| n.checked_add(width)).is_some_and(|n| n <= self.data.len()))
    }
}

/// An I420 frame: Y at `width` × `height`, U and V at half that (rounded up),
/// and an optional alpha plane at full size.
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
    /// A plane is smaller than its frame says.
    Plane(&'static str),
    /// `pixels.len()` is not `width × height × 4`.
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

/// Converts frames, keeping a few rows of scratch between calls.
pub struct Converter {
    level: Level,
    /// The vertical chroma blend, `cw + 2`.
    tmp: Vec<u16>,
    /// U and V of one luma row at luma resolution.
    u: Vec<u8>,
    v: Vec<u8>,
    scaler: Scaler,
}

impl Default for Converter {
    fn default() -> Self {
        Converter::new()
    }
}

impl Converter {
    /// At the best [`Level`] this CPU runs.
    pub fn new() -> Converter {
        Converter::with_level(Level::best())
    }

    /// At `level`, which must be one of [`Level::available`] (else the best).
    pub fn with_level(level: Level) -> Converter {
        let level = if Level::available().contains(&level) { level } else { Level::best() };
        Converter { level, tmp: Vec::new(), u: Vec::new(), v: Vec::new(), scaler: Scaler::default() }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    /// Scratch bytes held.
    pub fn memory_bytes(&self) -> usize {
        self.tmp.capacity() * 2 + self.u.capacity() + self.v.capacity() + self.scaler.bytes()
    }

    /// `frame` into `pixels`, `width` × `height` straight BGRA, rows packed.
    pub fn to_bgra(&mut self, frame: &I420, colour: Colorimetry, pixels: &mut [u8], width: u32, height: u32) -> Result<(), Error> {
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
        let Converter { tmp, u, v, scaler, .. } = self;
        // Source row `sy`, straight BGRA, into `out` (`w · 4`).
        let mut convert = |sy: usize, out: &mut [u8]| {
            let j = sy / 2;
            let far = if sy.is_multiple_of(2) { j.saturating_sub(1) } else { (j + 1).min(ch - 1) };
            chroma_row(level, frame.u.row(j, cw), frame.u.row(far, cw), tmp, u);
            chroma_row(level, frame.v.row(j, cw), frame.v.row(far, cw), tmp, v);
            yuva_row(level, &k, frame.y.row(sy, w), u, v, frame.a.map(|a| a.row(sy, w)), out);
        };
        if (tw, th) == (w, h) {
            for (sy, out) in pixels.chunks_exact_mut(w * 4).enumerate() {
                convert(sy, out);
            }
            return Ok(());
        }
        scaler.run::<4>(level, (w, h), (tw, th), &mut convert, |oy, row| {
            scale::unpremultiply(row, &mut pixels[oy * tw * 4..(oy + 1) * tw * 4])
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
                for (p, &a) in pixels[oy * tw * 4..(oy + 1) * tw * 4].chunks_exact_mut(4).zip(row) {
                    p.copy_from_slice(&[255, 255, 255, (a + 0.5) as u8]);
                }
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
