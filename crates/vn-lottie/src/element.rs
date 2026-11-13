//! The per-frame gpui element for one `<lottie>` node: a component, not a view, so everything is
//! recomputed each frame and only the [`LottieState`] survives.

use gpui::{div, img, AnyElement, App, IntoElement, ObjectFit, Styled, StyledImage, Window};

use crate::LottieState;

/// CSS `object-fit`; mirrors gpui's [`ObjectFit`] one-for-one as a comparable, storable value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LottieFit {
    Fill,
    /// The default, matching `<lottie-player>`'s `preserveAspectRatio: "xMidYMid meet"`.
    #[default]
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl From<LottieFit> for ObjectFit {
    fn from(fit: LottieFit) -> Self {
        match fit {
            LottieFit::Fill => Self::Fill,
            LottieFit::Contain => Self::Contain,
            LottieFit::Cover => Self::Cover,
            LottieFit::ScaleDown => Self::ScaleDown,
            LottieFit::None => Self::None,
        }
    }
}

/// Per-frame view parameters; nothing is remembered between frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LottieOptions {
    /// The content box in logical (CSS) pixels; the host falls back to the animation's intrinsic size.
    pub size: (f32, f32),
    /// Device pixels per logical pixel (`window.scale_factor()`); drives the raster size.
    pub scale: f32,
    pub object_fit: LottieFit,
    pub grayscale: bool,
}

impl Default for LottieOptions {
    fn default() -> Self {
        Self {
            size: (0.0, 0.0),
            scale: 1.0,
            object_fit: LottieFit::Contain,
            grayscale: false,
        }
    }
}

impl LottieOptions {
    pub fn intrinsic(state: &LottieState, scale: f32) -> Self {
        let meta = state.meta();
        Self {
            size: (meta.width as f32, meta.height as f32),
            scale,
            ..Self::default()
        }
    }
}

/// Builds the element for one frame: rasterises the current playhead (cached when nothing moved),
/// paints it and releases atlas tiles of rasters out of use, so call this rather than
/// [`LottieState::render_frame`]. An unrasterisable frame paints an empty box of the same size.
pub fn lottie(
    state: &mut LottieState,
    options: &LottieOptions,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let rendered = state.render_frame(options.size, options.scale);

    for image in state.take_retired() {
        // Inside this window's update `App.windows` has its slot taken, so `drop_image` would skip it.
        cx.drop_image(image, Some(window));
    }

    match rendered {
        Some(image) => img(image)
            .object_fit(options.object_fit.into())
            .grayscale(options.grayscale)
            .size_full()
            .into_any_element(),
        None => div().size_full().into_any_element(),
    }
}
