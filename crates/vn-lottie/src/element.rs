//! The per-frame gpui element for one `<lottie>` node: a component, not a view, so everything is
//! recomputed each frame and only the [`LottieState`] survives.

use gpui::{div, img, AnyElement, App, IntoElement, ObjectFit, Styled, StyledImage, Window};

use crate::LottieState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LottieFit {
    Fill,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LottieOptions {
    pub size: (f32, f32),
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
/// paints it and releases atlas tiles of rasters out of use; call this rather than [`LottieState::render_frame`].
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
