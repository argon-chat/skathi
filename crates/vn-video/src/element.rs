//! The per-frame gpui element for one `<video>` node: a component, not a view, so everything is
//! recomputed each frame and only the [`VideoState`] survives.

use gpui::{div, img, AnyElement, App, IntoElement, ObjectFit, Styled, StyledImage, Window};

use crate::state::VideoState;

/// CSS `object-fit`; mirrors gpui's [`ObjectFit`] one-for-one as a comparable, storable value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoFit {
    Fill,
    /// The default, as in the HTML spec (not `fill`, as for `<img>`).
    #[default]
    Contain,
    Cover,
    ScaleDown,
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

/// Per-frame view parameters; nothing is remembered between frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoOptions {
    /// The content box in logical (CSS) pixels.
    pub size: (f32, f32),
    pub scale: f32,
    pub object_fit: VideoFit,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self { size: (0.0, 0.0), scale: 1.0, object_fit: VideoFit::Contain }
    }
}

impl VideoOptions {
    /// The physical-pixel box a decoded frame is capped at ([`crate::FrameRequest`]). `cover` needs
    /// the larger axis, so the bounding square is requested; `fit_within` clamps to native size.
    pub fn max_frame_size(&self) -> (u32, u32) {
        let w = (self.size.0 * self.scale).round().max(0.0) as u32;
        let h = (self.size.1 * self.scale).round().max(0.0) as u32;
        match self.object_fit {
            // `cover` crops, so the covering frame is larger on one axis: ask for a square of the larger dimension.
            VideoFit::Cover => {
                let m = w.max(h);
                (m, m)
            }
            _ => (w, h),
        }
    }
}

/// Builds the element for one frame: paints the frame the last [`VideoState::advance`] produced and
/// releases atlas tiles of frames out of use, so call this rather than reading [`VideoState::frame`].
/// A state with no frame yet paints an empty box of the same size.
pub fn video(
    state: &mut VideoState,
    options: &VideoOptions,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    for image in state.take_retired() {
        // Inside this window's update `App.windows` has its slot taken, so `drop_image` would skip it.
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
