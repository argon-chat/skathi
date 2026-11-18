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
    static PENDING: Cell<Shown> = const { Cell::new(Shown::Nowhere) };
}

static PRESENTS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn note(shown: Shown) {
    PENDING.with(|p| p.set(p.get().max(shown)));
}

pub(crate) fn take() -> Shown {
    PENDING.with(|p| p.replace(Shown::Nowhere))
}

static HIDDEN: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_hidden(hidden: bool) {
    HIDDEN.store(hidden, Ordering::Relaxed);
}

pub(crate) fn page_hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

pub(crate) fn present(window: &Window) {
    if HIDDEN.load(Ordering::Relaxed) || !window.is_visible() {
        return;
    }
    PRESENTS.fetch_add(1, Ordering::Relaxed);
    window.request_present();
}

pub(crate) fn present_count() -> u64 {
    PRESENTS.load(Ordering::Relaxed)
}

/// Can a pixel of node `id` be on screen without being clipped by an ancestor?
pub(crate) fn visible(id: u32, win: WinKey) -> bool {
    let Some(Some((rect, viewport))) = windows::peek(win, |w| {
        let env = w.env.as_ref()?;
        let rect = w
            .transformed
            .get(&id)
            .or_else(|| w.bounds.get(&id))
            .copied();
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
            let Some(cs) = p.computed.as_ref() else {
                continue;
            };
            let clips =
                |o: Option<gpui::Overflow>| !matches!(o, None | Some(gpui::Overflow::Visible));
            let (cx, cy) = (clips(cs.base.overflow.x), clips(cs.base.overflow.y));
            if !cx && !cy {
                continue;
            }
            let Some(prect) = windows::peek(win, |w| {
                w.transformed
                    .get(&cur)
                    .or_else(|| w.bounds.get(&cur))
                    .copied()
            })
            .flatten() else {
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

#[allow(dead_code)]
pub(crate) struct LiveTile {
    image: Arc<RenderImage>,
    win: WinKey,
}

#[allow(dead_code)]
impl LiveTile {
    pub(crate) fn new(win: WinKey, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let buffer = image::RgbaImage::new(width, height);
        let image = RenderImage::new(vec![image::Frame::new(buffer)]).dynamic();
        LiveTile {
            image: Arc::new(image),
            win,
        }
    }

    pub(crate) fn image(&self) -> Arc<RenderImage> {
        self.image.clone()
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        let s = self.image.size(0);
        (s.width.0 as u32, s.height.0 as u32)
    }

    pub(crate) fn window(&self) -> WinKey {
        self.win
    }

    /// Writes a CPU frame: BGRA, the tile's size, rows packed, STRAIGHT alpha (gpui's polychrome
    /// pipeline blends `SRC_ALPHA`, so a premultiplied raster comes out darker where translucent).
    pub(crate) fn write_bgra(
        &mut self,
        window: &mut Window,
        bgra: Vec<u8>,
        visible: bool,
    ) -> Shown {
        debug_assert_eq!(
            windows::key(window),
            self.win,
            "a tile is written with its own window"
        );
        let (w, h) = self.size();
        let Some(buffer) = image::RgbaImage::from_raw(w, h, bgra) else {
            return Shown::Draw;
        };
        let next = Arc::new(self.image.successor(vec![image::Frame::new(buffer)]));
        let write = window
            .write_image_tile(&next, 0, TileSource::Bytes)
            .unwrap_or(TileWrite::Placed);
        self.image = next;
        shown(write, visible)
    }

    /// Writes a half-float CPU frame (linear, straight alpha, rows packed) into an HDR tile; `None`
    /// when the renderer has none: fall back to [`LiveTile::write_bgra`].
    pub(crate) fn write_rgba16f(
        &mut self,
        window: &mut Window,
        halves: &[u16],
        visible: bool,
    ) -> Option<Shown> {
        match window.write_image_tile_rgba16f(&self.image, 0, halves) {
            Ok(TileWrite::Refused) | Err(_) => None,
            Ok(write) => Some(shown(write, visible)),
        }
    }

    /// `Err` names why the renderer refused the texture.
    pub(crate) fn write_gpu(
        &mut self,
        window: &mut Window,
        texture: GpuTextureHandle,
        visible: bool,
    ) -> Result<Shown, String> {
        debug_assert_eq!(
            windows::key(window),
            self.win,
            "a tile is written with its own window"
        );
        tile_written(
            window.write_image_tile(&self.image, 0, TileSource::Gpu(texture)),
            visible,
        )
    }

    /// [`LiveTile::write_gpu`] for a D3D12 resource on the renderer's device; `owner` is retained
    /// until the atlas copy has run.
    #[cfg(windows)]
    pub(crate) fn write_d3d12(
        &mut self,
        window: &mut Window,
        texture: gpui::D3D12TextureHandle,
        owner: Arc<dyn std::any::Any + Send + Sync>,
        visible: bool,
    ) -> Result<Shown, String> {
        tile_written(
            window.write_image_tile_d3d12(&self.image, 0, texture, owner),
            visible,
        )
    }

    /// [`LiveTile::write_gpu`] for a Metal texture on the renderer's device, written once the
    /// `MTLSharedEvent` in `ready` reaches its value; `owner` is retained until the atlas copy
    /// has run.
    #[cfg(target_os = "macos")]
    pub(crate) fn write_metal(
        &mut self,
        window: &mut Window,
        texture: GpuTextureHandle,
        ready: (*mut std::ffi::c_void, u64),
        owner: Arc<dyn std::any::Any + Send + Sync>,
        visible: bool,
    ) -> Result<Shown, String> {
        tile_written(
            window.write_image_tile_metal(&self.image, 0, texture, Some(ready), owner),
            visible,
        )
    }

    /// Hands the tile back to its window's atlas on that window's next frame, when no scene samples
    /// it any more. The frame that stops painting it must be a draw (see the module docs).
    pub(crate) fn retire(self) {
        windows::queue_drops(self.win, [self.image]);
    }
}

fn tile_written<E: std::fmt::Display>(
    write: Result<TileWrite, E>,
    visible: bool,
) -> Result<Shown, String> {
    match write {
        Ok(TileWrite::Refused) => Err(
            "the renderer's atlas takes no texture of this kind, format or size, or from this device"
                .into(),
        ),
        Ok(write) => Ok(shown(write, visible)),
        Err(error) => Err(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_screen(id: u32, win: WinKey) -> bool {
        windows::peek(win, |w| {
            let Some(env) = w.env.as_ref() else {
                return true;
            };
            let (vw, vh) = (env.viewport_width, env.viewport_height);
            let meets =
                |&(x, y, bw, bh): &windows::Rect| x + bw > 0.0 && y + bh > 0.0 && x < vw && y < vh;
            w.bounds.get(&id).is_some_and(meets) || w.transformed.get(&id).is_some_and(meets)
        })
        .unwrap_or(true)
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
        tree::set_engine(sk_css::Engine::compile(
            ".clip { overflow: hidden } .abs { position: absolute }",
        ));
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
        windows::set_env(
            win,
            sk_css::Env {
                viewport_width: 400.0,
                viewport_height: 300.0,
                ..Default::default()
            },
        );
        windows::of(win, |w| {
            w.bounds.insert(list, (0.0, 0.0, 100.0, 100.0));
            w.bounds.insert(inside, (0.0, 10.0, 100.0, 20.0));
            w.bounds.insert(outside, (0.0, 150.0, 100.0, 20.0));
            w.bounds.insert(escaped, (0.0, 150.0, 100.0, 20.0));
        });
        assert!(visible(inside, win));
        assert!(on_screen(outside, win), "inside the viewport");
        assert!(!visible(outside, win), "but cut away by the list's clip");
        assert!(
            visible(escaped, win),
            "an absolute box may escape it: the safe side"
        );
        assert!(!visible(0xDEAD, win), "never painted");
        windows::forget(win);
    }

    #[test]
    fn a_live_tile_is_a_dynamic_picture_whose_frames_keep_its_id() {
        let tile = LiveTile::new(windows::UNBOUND, 4, 2);
        assert_eq!(tile.size(), (4, 2));
        let image = tile.image();
        assert!(
            image.is_dynamic(),
            "the resampler must never swap in a copy"
        );
        let next = image.successor(vec![image::Frame::new(image::RgbaImage::new(4, 2))]);
        assert_eq!(next.id, image.id, "one id, one atlas tile");
        assert!(next.is_dynamic());
    }
}
