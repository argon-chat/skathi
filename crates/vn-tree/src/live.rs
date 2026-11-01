//! Paint-only invalidation: a native element's picture changes without a build
//! (docs/ARCHITECTURE.md decision 28, fork item 40).
//!
//! A `<video>` frame, a moving `<canvas>`, a Lottie or sticker raster: each is
//! a new picture in a box that did not move. Before this, every one of them was
//! a full window draw, so the build, layout, prepaint and paint all ran again
//! at the display rate. Now:
//!
//! 1. **One atlas tile per picture, for its whole life.** gpui keys a tile by
//!    `RenderImage::id`. A producer keeps one id: a zero-copy video's
//!    placeholder, or [`gpui::RenderImage::successor`] for CPU frames. The
//!    image is `dynamic`, so the resampler never swaps in a copy with a tile of
//!    its own.
//! 2. **The frame goes into that tile from the pump, before the draw**
//!    (`Window::write_image_tile`), by GPU blit or CPU bytes. gpui answers
//!    `InPlace` when the tile the last drawn scene samples was overwritten
//!    where it lies, and `Placed` when the frame took a new tile.
//! 3. **What the frame then needs** ([`shown`]): `InPlace` with a place that
//!    can be on screen is a [`Shown::Present`], `Window::request_present`
//!    re-encoding the scene already drawn. `Placed` there is a
//!    [`Shown::Draw`]. With no place on screen it is [`Shown::Nowhere`]: the
//!    next draw that paints the image samples the tile, which holds the newest
//!    frame.
//! 4. **The tick reports it** with [`note`]. `crate::frame_tick` turns a Draw
//!    into its `true` (the host notifies) and a Present into
//!    `request_present`. No build means the layout samplers stay parked too.
//!
//! [`LiveTile`] packages 1–3 for an element that owns its picture and rasterises
//! it itself: the `<lottie>` port and the shared sticker element are its users.
//! `<video>` (its image belongs to vn-video) and `<canvas>` (its still/moving
//! split) use the free functions directly.
//!
//! Rules for a producer:
//!
//! * Write in the tick, never in a build.
//! * Paint the tile's image and nothing derived from it.
//! * Never swap a painted image for one with a new id without a
//!   [`Shown::Draw`] in the same frame. A retired image's tile is freed on the
//!   next pump, and a present-only frame would still sample it.
//! * A tile lives in ONE window's atlas, and it is written with that window.
//!   The pump holds the primary window, so a pop-out's tile is written from its
//!   own `window_frame_tick` and shown by its own draw.
//!
//! A real draw is still needed for the first write, a new size, an
//! image-identity change (a video's poster to its first frame, a canvas going
//! still or starting to move), the video CPU-readback fallback (a new image a
//! frame), a pop-out's tile, and a device-lost.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gpui::{GpuTextureHandle, RenderImage, TileSource, TileWrite, Window};

use crate::windows::{self, WinKey};

/// What a written frame needs before it can be seen. Ordered: the frame takes
/// the most any producer asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Shown {
    /// No place that paints the picture can be on screen.
    Nowhere,
    /// The scene already drawn samples the tile: present it again.
    Present,
    /// Only a draw shows it.
    Draw,
}

/// The [`Shown`] for a write gpui reported as `write`, when a place that
/// paints the image can be on screen (`visible`).
///
/// A refused GPU write is a Draw: the caller falls back to CPU frames, which
/// starts with a new image.
pub(crate) fn shown(write: TileWrite, visible: bool) -> Shown {
    match (write, visible) {
        (_, false) => Shown::Nowhere,
        (TileWrite::InPlace, true) => Shown::Present,
        (TileWrite::Placed | TileWrite::Refused, true) => Shown::Draw,
    }
}

thread_local! {
    /// The most any producer asked for during this tick.
    static PENDING: Cell<Shown> = const { Cell::new(Shown::Nowhere) };
}

/// Presents asked of a window since the process started (devtools `perf`).
static PRESENTS: AtomicU64 = AtomicU64::new(0);

/// Reports what a frame written during this tick needs.
pub(crate) fn note(shown: Shown) {
    PENDING.with(|p| p.set(p.get().max(shown)));
}

/// What this tick's writes need, reset for the next one. `crate::frame_tick`
/// asks once at its end: a Draw becomes its `true`, a Present
/// `window.request_present()` (see [`present`]).
pub(crate) fn take() -> Shown {
    PENDING.with(|p| p.replace(Shown::Nowhere))
}

/// Presents `window`'s last drawn frame again, and counts it.
pub(crate) fn present(window: &Window) {
    PRESENTS.fetch_add(1, Ordering::Relaxed);
    window.request_present();
}

/// Presents asked since the process started.
pub(crate) fn present_count() -> u64 {
    PRESENTS.load(Ordering::Relaxed)
}

/// Can a pixel of node `id` be on screen in window `win`? Its painted box (laid
/// out, or as transformed) meets that window's viewport, by the last drawn
/// frame's bounds. A node that was not painted (inside `display: none`, never
/// built) has none. Clipping by an ancestor is not considered: a clipped box
/// counts as on screen, the safe side. A window with no published viewport yet
/// counts everything as on screen.
pub(crate) fn on_screen(id: u32, win: WinKey) -> bool {
    windows::peek(win, |w| {
        let Some(env) = w.env.as_ref() else { return true };
        let (vw, vh) = (env.viewport_width, env.viewport_height);
        let meets = |&(x, y, bw, bh): &windows::Rect| x + bw > 0.0 && y + bh > 0.0 && x < vw && y < vh;
        w.bounds.get(&id).is_some_and(meets) || w.transformed.get(&id).is_some_and(meets)
    })
    .unwrap_or(true)
}

/// One picture a native element repaints by itself: one image id, so one tile
/// in one window's atlas, for the life of the picture, written frame by frame.
///
/// The build paints [`LiveTile::image`], at as many places as show the
/// picture. The tick writes the next frame ([`LiveTile::write_bgra`] or
/// [`LiveTile::write_gpu`]) and [`note`]s the answer. `visible` is the
/// caller's: can any place that paints this tile be on screen ([`on_screen`]
/// over the places).
#[allow(dead_code)]
pub(crate) struct LiveTile {
    /// The newest frame under the tile's id. Its bytes are the CPU frame last
    /// written (zeros for a GPU-fed tile), so a re-upload after the atlas lost
    /// the tile shows the newest picture.
    image: Arc<RenderImage>,
    /// The window whose atlas holds the tile.
    win: WinKey,
}

// The API the `<lottie>` port and the shared sticker element build on
// (module docs); nothing in the tree writes one yet.
#[allow(dead_code)]
impl LiveTile {
    /// A transparent `width` × `height` picture in window `win`'s atlas.
    pub(crate) fn new(win: WinKey, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let buffer = image::RgbaImage::new(width, height);
        let image = RenderImage::new(vec![image::Frame::new(buffer)]).dynamic();
        LiveTile { image: Arc::new(image), win }
    }

    /// What the build paints (`gpui::img(ImageSource::Render(..))`).
    pub(crate) fn image(&self) -> Arc<RenderImage> {
        self.image.clone()
    }

    /// The picture's size in pixels.
    pub(crate) fn size(&self) -> (u32, u32) {
        let s = self.image.size(0);
        (s.width.0 as u32, s.height.0 as u32)
    }

    /// The window whose atlas holds the tile.
    pub(crate) fn window(&self) -> WinKey {
        self.win
    }

    /// Writes a CPU frame: premultiplied BGRA, the tile's size, rows packed.
    /// A frame of another size is ignored with a Draw: resize with a new
    /// [`LiveTile`] (retiring this one), which the build then paints.
    ///
    /// `window` must be the tile's window.
    pub(crate) fn write_bgra(&mut self, window: &mut Window, bgra: Vec<u8>, visible: bool) -> Shown {
        debug_assert_eq!(windows::key(window), self.win, "a tile is written with its own window");
        let (w, h) = self.size();
        let Some(buffer) = image::RgbaImage::from_raw(w, h, bgra) else {
            return Shown::Draw;
        };
        let next = Arc::new(self.image.successor(vec![image::Frame::new(buffer)]));
        let write = window.write_image_tile(&next, 0, TileSource::Bytes).unwrap_or(TileWrite::Placed);
        self.image = next;
        shown(write, visible)
    }

    /// Writes a GPU frame (the contract of `Window::write_image_tile`'s
    /// `TileSource::Gpu`). `None` when the renderer refused the texture: the
    /// caller falls back to [`LiveTile::write_bgra`].
    pub(crate) fn write_gpu(&mut self, window: &mut Window, texture: GpuTextureHandle, visible: bool) -> Option<Shown> {
        debug_assert_eq!(windows::key(window), self.win, "a tile is written with its own window");
        match window.write_image_tile(&self.image, 0, TileSource::Gpu(texture)).unwrap_or(TileWrite::Refused) {
            TileWrite::Refused => None,
            write => Some(shown(write, visible)),
        }
    }

    /// Hands the tile back to its window's atlas on that window's next frame,
    /// when no scene samples it any more. The frame that stops painting it
    /// must be a draw (the producer's rules, module docs).
    pub(crate) fn retire(self) {
        windows::queue_drops(self.win, [self.image]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_in_place_is_a_present_and_a_new_tile_a_draw() {
        assert_eq!(shown(TileWrite::InPlace, true), Shown::Present);
        assert_eq!(shown(TileWrite::Placed, true), Shown::Draw);
        assert_eq!(shown(TileWrite::Refused, true), Shown::Draw);
        for w in [TileWrite::InPlace, TileWrite::Placed, TileWrite::Refused] {
            assert_eq!(shown(w, false), Shown::Nowhere, "nothing to show off screen");
        }
    }

    #[test]
    fn a_tick_takes_the_most_any_producer_asked_for() {
        assert_eq!(take(), Shown::Nowhere);
        note(Shown::Present);
        note(Shown::Nowhere);
        assert_eq!(take(), Shown::Present);
        note(Shown::Present);
        note(Shown::Draw);
        note(Shown::Present);
        assert_eq!(take(), Shown::Draw);
        assert_eq!(take(), Shown::Nowhere, "reset for the next tick");
    }

    #[test]
    fn a_live_tile_is_a_dynamic_picture_whose_frames_keep_its_id() {
        let tile = LiveTile::new(windows::UNBOUND, 4, 2);
        assert_eq!(tile.size(), (4, 2));
        let image = tile.image();
        assert!(image.is_dynamic(), "the resampler must never swap in a copy");
        let next = image.successor(vec![image::Frame::new(image::RgbaImage::new(4, 2))]);
        assert_eq!(next.id, image.id, "one id, one atlas tile");
        assert!(next.is_dynamic());
    }
}
