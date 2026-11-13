//! Paint-only invalidation: a native element's picture (video frame, moving canvas, Lottie or
//! sticker raster) changes without a build, by rewriting its atlas tile in place.
//!
//! A producer keeps one `RenderImage` id for the picture's life (the image is `dynamic`, so the
//! resampler never swaps in a copy). The frame is written into its tile from the pump before the
//! draw (`Window::write_image_tile`); gpui answers `InPlace` (a present re-encoding the last scene
//! suffices, [`Shown::Present`]) or `Placed` (a new tile, needs [`Shown::Draw`]); with no place on
//! screen it is [`Shown::Nowhere`]. `crate::frame_tick` turns the reported result into a draw or a
//! present. [`LiveTile`] packages this for elements that rasterise their own picture.
//!
//! Rules for a producer: write in the tick, never in a build; paint the tile's image and nothing
//! derived from it; never swap a painted image for one with a new id without a `Draw` in the same
//! frame (the retired tile is freed next pump and a present-only frame would still sample it); a tile
//! lives in ONE window's atlas and is written with that window (a pop-out's from its own
//! `window_frame_tick`). A real draw is still needed for the first write, a new size, an identity
//! change, the video CPU-readback fallback, a pop-out's tile and device loss.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use gpui::{GpuTextureHandle, RenderImage, TileSource, TileWrite, Window};

use crate::windows::{self, WinKey};

/// What a written frame needs before it can be seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Shown {
    /// No place that paints the picture can be on screen.
    Nowhere,
    /// The scene already drawn samples the tile: present it again.
    Present,
    /// Only a draw shows it.
    Draw,
}

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

pub(crate) fn note(shown: Shown) {
    PENDING.with(|p| p.set(p.get().max(shown)));
}

/// What this tick's writes need, reset for the next one.
pub(crate) fn take() -> Shown {
    PENDING.with(|p| p.replace(Shown::Nowhere))
}

/// Is the page hidden (minimised, or on another virtual desktop)?
static HIDDEN: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_hidden(hidden: bool) {
    HIDDEN.store(hidden, Ordering::Relaxed);
}

/// Is the page hidden ([`set_hidden`])? Nothing on it can be seen, so a producer that plays only
/// while seen (`<vn-sticker>`) stops.
pub(crate) fn page_hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

/// Presents `window`'s last drawn frame again, and counts it.
pub(crate) fn present(window: &Window) {
    if HIDDEN.load(Ordering::Relaxed) || !window.is_visible() {
        return;
    }
    PRESENTS.fetch_add(1, Ordering::Relaxed);
    window.request_present();
}

/// Presents asked since the process started.
pub(crate) fn present_count() -> u64 {
    PRESENTS.load(Ordering::Relaxed)
}

/// Can a pixel of node `id` be on screen without being clipped by an ancestor? The node's painted
/// box, cut to the viewport and to every ancestor that clips its overflow on an axis (`hidden`,
/// `clip`, `scroll`, `auto`), is not empty.
pub(crate) fn visible(id: u32, win: WinKey) -> bool {
    let Some(Some((rect, viewport))) = windows::peek(win, |w| {
        let env = w.env.as_ref()?;
        let rect = w.transformed.get(&id).or_else(|| w.bounds.get(&id)).copied();
        Some((rect, (0.0, 0.0, env.viewport_width, env.viewport_height)))
    }) else {
        return true;
    };
    let Some(r) = rect else { return false }; // not painted
    let mut cut = clip_rect(r, viewport, true, true);
    if cut.is_none() {
        return false;
    }
    crate::tree::read_tree(|t| {
        let mut cur = id;
        while let Some(n) = t.node(cur) {
            let out_of_flow = n.computed.as_ref().is_some_and(|c| {
                matches!(c.base.position, Some(gpui::Position::Absolute)) || c.extras.may_defer()
            });
            if out_of_flow || n.parent == 0 {
                break;
            }
            cur = n.parent;
            let Some(p) = t.node(cur) else { break };
            let Some(cs) = p.computed.as_ref() else { continue };
            let clips = |o: Option<gpui::Overflow>| !matches!(o, None | Some(gpui::Overflow::Visible));
            let (cx, cy) = (clips(cs.base.overflow.x), clips(cs.base.overflow.y));
            if !cx && !cy {
                continue;
            }
            let Some(prect) = windows::peek(win, |w| w.transformed.get(&cur).or_else(|| w.bounds.get(&cur)).copied())
                .flatten()
            else {
                continue;
            };
            cut = cut.and_then(|c| clip_rect(c, prect, cx, cy));
            if cut.is_none() {
                return false;
            }
        }
        true
    })
}

/// `r` cut to `by` on the axes asked, or `None` when nothing is left.
fn clip_rect(r: windows::Rect, by: windows::Rect, x: bool, y: bool) -> Option<windows::Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (r.0, r.1, r.0 + r.2, r.1 + r.3);
    if x {
        x0 = x0.max(by.0);
        x1 = x1.min(by.0 + by.2);
    }
    if y {
        y0 = y0.max(by.1);
        y1 = y1.min(by.1 + by.3);
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// One picture a native element repaints by itself: one image id, so one tile in one window's
/// atlas, for the life of the picture, written frame by frame.
#[allow(dead_code)]
pub(crate) struct LiveTile {
    /// The newest frame under the tile's id.
    image: Arc<RenderImage>,
    /// The window whose atlas holds the tile.
    win: WinKey,
}

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

    /// Writes a CPU frame: BGRA, the tile's size, rows packed, with STRAIGHT alpha. gpui's own
    /// decoders un-premultiply (`swap_rgba_pa_to_bgra`) and the polychrome pipeline blends
    /// `SRC_ALPHA`, so a premultiplied raster (ThorVG's, a canvas's) comes out darker wherever it
    /// is translucent.
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

    /// `None` when the renderer refused the texture: the caller falls back to
    /// [`LiveTile::write_bgra`].
    pub(crate) fn write_gpu(&mut self, window: &mut Window, texture: GpuTextureHandle, visible: bool) -> Option<Shown> {
        debug_assert_eq!(windows::key(window), self.win, "a tile is written with its own window");
        match window.write_image_tile(&self.image, 0, TileSource::Gpu(texture)).unwrap_or(TileWrite::Refused) {
            TileWrite::Refused => None,
            write => Some(shown(write, visible)),
        }
    }

    /// Hands the tile back to its window's atlas on that window's next frame, when no scene samples
    /// it any more. The frame that stops painting it must be a draw (the producer's rules, module
    /// docs).
    pub(crate) fn retire(self) {
        windows::queue_drops(self.win, [self.image]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Can a pixel of node `id` be on screen in window `win`?
    fn on_screen(id: u32, win: WinKey) -> bool {
        windows::peek(win, |w| {
            let Some(env) = w.env.as_ref() else { return true };
            let (vw, vh) = (env.viewport_width, env.viewport_height);
            let meets = |&(x, y, bw, bh): &windows::Rect| x + bw > 0.0 && y + bh > 0.0 && x < vw && y < vh;
            w.bounds.get(&id).is_some_and(meets) || w.transformed.get(&id).is_some_and(meets)
        })
        .unwrap_or(true)
    }


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

    /// A box scrolled out of its clipping ancestor is inside the window but shows nothing; an
    /// absolutely positioned one may escape the clip, so it counts as visible.
    #[test]
    fn a_box_clipped_away_by_an_ancestor_is_not_visible() {
        use crate::tree;
        tree::set_engine(vn_css::Engine::compile(".clip { overflow: hidden } .abs { position: absolute }"));
        let (list, inside, outside, escaped) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let list = t.create_element(div);
            t.set_class(list, "clip");
            t.insert(list, root, 0);
            let inside = t.create_element(div);
            let outside = t.create_element(div);
            let escaped = t.create_element(div);
            t.set_class(escaped, "abs");
            for n in [inside, outside, escaped] {
                t.insert(n, list, 0);
            }
            (list, inside, outside, escaped)
        });
        crate::flush_changes();
        let win: WinKey = 0x51_7E;
        windows::set_env(win, vn_css::Env { viewport_width: 400.0, viewport_height: 300.0, ..Default::default() });
        windows::of(win, |w| {
            w.bounds.insert(list, (0.0, 0.0, 100.0, 100.0));
            w.bounds.insert(inside, (0.0, 10.0, 100.0, 20.0));
            w.bounds.insert(outside, (0.0, 150.0, 100.0, 20.0));
            w.bounds.insert(escaped, (0.0, 150.0, 100.0, 20.0));
        });
        assert!(visible(inside, win));
        assert!(on_screen(outside, win), "inside the viewport");
        assert!(!visible(outside, win), "but cut away by the list's clip");
        assert!(visible(escaped, win), "an absolute box may escape it: the safe side");
        assert!(!visible(0xDEAD, win), "never painted");
        windows::forget(win);
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
