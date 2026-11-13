//! [`Animation`]: a Lottie with no transport and no gpui object, for the sticker engine
//! (`crates/vn-sticker`).
//!
//! "Load once, draw whole frame `n` at any size", unlike [`crate::LottieState`] (one node, a
//! playhead, atlas images):
//!
//! * Whole frames: frame `n` is source frame `in_point + n` for `n` in `0..frame_count()`.
//! * Straight-alpha BGRA, as gpui's polychrome pipeline blends (`LiveTile::write_bgra`).
//! * Filled: the whole animation is stretched to a target of its aspect ratio, so at most a pixel
//!   of rounding.
//! * `Send`: loaded on one worker, drawn on another; ThorVG's calls are serialised by its engine
//!   lock (`backend::thorvg`).

use crate::backend::{Backend, Rasteriser};
use crate::state::{prepare, LottieError, LottieMeta, Prepared};

/// A loaded Lottie animation, ready to draw any whole frame at any size.
pub struct Animation {
    renderer: Rasteriser,
    meta: LottieMeta,
    advisories: Vec<String>,
    frame_count: u32,
    json_bytes: usize,
}

impl Animation {
    /// Parses `bytes` (Lottie JSON or gzipped `.tgs`) with the preferred backend, as
    /// [`crate::LottieState::load`] does.
    pub fn load(bytes: &[u8]) -> Result<Self, LottieError> {
        Self::load_with(Backend::preferred(), bytes)
    }

    pub fn load_with(backend: Backend, bytes: &[u8]) -> Result<Self, LottieError> {
        let Prepared {
            renderer,
            meta,
            advisories,
            json_bytes,
        } = prepare(backend, bytes)?;
        let frame_count = whole_frames(meta.total_frames);
        Ok(Self {
            renderer,
            meta,
            advisories,
            frame_count,
            json_bytes,
        })
    }

    pub fn meta(&self) -> &LottieMeta {
        &self.meta
    }

    pub fn backend(&self) -> Backend {
        self.renderer.backend()
    }

    pub fn advisories(&self) -> &[String] {
        &self.advisories
    }

    /// Whole frames in one loop, at least 1: the source frames from
    /// `in_point` up to, not including, `out_point`.
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Draws whole frame `n` (wrapped by [`Self::frame_count`]) filling `width × height`:
    /// straight-alpha BGRA, rows packed, borrowed until the next call. `None` for a zero-sized
    /// target or an engine failure.
    pub fn render(&mut self, n: u32, width: u32, height: u32) -> Option<&[u8]> {
        let frame = self.meta.in_point + (n % self.frame_count) as f32;
        let intrinsic = (self.meta.width.max(1), self.meta.height.max(1));
        self.renderer.render_straight(frame, intrinsic, width, height)
    }

    /// Draws absolute, fractional source `frame` ([`crate::Transport::frame`]) filling
    /// `width × height`, as [`Self::render`]. A `<lottie>` box's size is [`crate::raster_size_for`].
    pub fn render_at(&mut self, frame: f32, width: u32, height: u32) -> Option<&[u8]> {
        let intrinsic = (self.meta.width.max(1), self.meta.height.max(1));
        self.renderer.render_straight(frame, intrinsic, width, height)
    }

    /// An estimate of the bytes held, for the engine's budget: three times the document's JSON (a
    /// parsed `.tgs` measured 1.0–1.7x its JSON, 2–5x once drawn) plus the raster buffer.
    pub fn estimated_bytes(&self) -> usize {
        self.json_bytes * 3 + self.renderer.buffer_bytes()
    }
}

/// Whole frames in `total` (`out_point - in_point`), at least 1. A total a
/// hair under an integer (float noise in `op`) counts as that integer.
fn whole_frames(total: f32) -> u32 {
    if !total.is_finite() || total <= 0.0 {
        return 1;
    }
    ((total - 1e-3).ceil() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_frames_count_from_in_to_out_exclusive() {
        assert_eq!(whole_frames(180.0), 180);
        assert_eq!(whole_frames(179.9999), 180, "float noise in op");
        assert_eq!(whole_frames(60.5), 61, "frame 60 starts before op");
        assert_eq!(whole_frames(0.0), 1);
        assert_eq!(whole_frames(f32::NAN), 1);
    }

    #[test]
    fn unpremultiply_restores_straight_colour() {
        use crate::backend::unpremultiply;
        assert_eq!(unpremultiply([0, 0, 0, 0]), [0, 0, 0, 0]);
        assert_eq!(unpremultiply([10, 20, 30, 255]), [10, 20, 30, 255]);
        assert_eq!(unpremultiply([64, 32, 0, 128]), [128, 64, 0, 128]);
        assert_eq!(unpremultiply([200, 0, 0, 100]), [255, 0, 0, 100], "clamped");
    }
}
