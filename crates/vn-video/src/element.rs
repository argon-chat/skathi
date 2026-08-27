//! The per-frame gpui element for one `<video>` node.
//!
//! Like `vn_lottie::element`, this is a *component*, not a view: the host owns
//! the [`VideoState`] in a side map and rebuilds the element tree every frame,
//! so everything here is recomputed from scratch each time and nothing but the
//! state survives.

use gpui::{div, img, AnyElement, App, IntoElement, ObjectFit, Styled, StyledImage, Window};

use crate::state::VideoState;

/// CSS `object-fit`, as the cascade produces it.
///
/// Mirrors gpui's own [`ObjectFit`] one-for-one; it exists because gpui's enum
/// derives nothing, and the host needs a plain value it can compare and store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoFit {
    /// `object-fit: fill` — stretch to the box, ignoring aspect ratio.
    Fill,
    /// `object-fit: contain` — letterbox inside the box.
    ///
    /// **The default, and it is the spec's.** HTML says the video is "shown
    /// centred in the playback area at the largest possible size that fits
    /// completely within it, with the video's aspect ratio being preserved" —
    /// i.e. `contain`, not the CSS initial value `fill` that a plain `<img>`
    /// takes.
    #[default]
    Contain,
    /// `object-fit: cover` — fill the box, cropping the overflow.
    Cover,
    /// `object-fit: scale-down` — `none` or `contain`, whichever is smaller.
    ScaleDown,
    /// `object-fit: none` — paint at intrinsic size.
    None,
}

impl From<VideoFit> for ObjectFit {
    fn from(fit: VideoFit) -> Self {
        match fit {
            VideoFit::Fill => Self::Fill,
            VideoFit::Contain => Self::Contain,
            VideoFit::Cover => Self::Cover,
            VideoFit::ScaleDown => Self::ScaleDown,
            VideoFit::None => Self::None,
        }
    }
}

/// Per-frame view parameters. Style comes from the host's CSS engine; nothing
/// in here is remembered between frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoOptions {
    /// The content box the video paints into, in **logical** (CSS) pixels.
    pub size: (f32, f32),
    /// Device pixels per logical pixel — `window.scale_factor()`.
    pub scale: f32,
    pub object_fit: VideoFit,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self { size: (0.0, 0.0), scale: 1.0, object_fit: VideoFit::Contain }
    }
}

impl VideoOptions {
    /// The physical-pixel box a decoded frame should be capped at — what goes
    /// into [`crate::FrameRequest`].
    ///
    /// Under `cover` the box is *filled*, so the frame has to be at least as
    /// large as the box's longer relative axis or the upscale would be visible;
    /// `fit_within` clamps to the native size anyway, so asking for the box's
    /// bounding square is safe and costs nothing on the common `contain` path.
    pub fn max_frame_size(&self) -> (u32, u32) {
        let w = (self.size.0 * self.scale).round().max(0.0) as u32;
        let h = (self.size.1 * self.scale).round().max(0.0) as u32;
        match self.object_fit {
            // `cover` crops: the visible area is the box, but the frame that
            // covers it is larger on one axis. Asking for a square of the
            // larger dimension is the cheap, correct-enough bound.
            VideoFit::Cover => {
                let m = w.max(h);
                (m, m)
            }
            _ => (w, h),
        }
    }
}

/// Builds the element for one frame.
///
/// Paints the frame the last [`VideoState::advance`] produced and releases the
/// atlas tiles of frames that have fallen out of use — which is why this needs
/// `window`/`cx` and why the host must call *this* rather than reading
/// [`VideoState::frame`] directly.
///
/// A state with no frame yet (loading, a broken `src`, a zero-area box) paints
/// an empty box of the same size rather than collapsing the layout — the same
/// thing a browser does before `loadeddata`.
pub fn video(
    state: &mut VideoState,
    options: &VideoOptions,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    for image in state.take_retired() {
        // `Some(window)`: we are inside this window's update, so `App.windows`
        // has its slot taken and `drop_image` would otherwise skip it.
        cx.drop_image(image, Some(window));
    }

    match state.frame() {
        Some(image) => img(image)
            .object_fit(options.object_fit.into())
            .size_full()
            .into_any_element(),
        None => div().size_full().into_any_element(),
    }
}
