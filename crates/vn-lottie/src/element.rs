//! The per-frame gpui element for one `<lottie>` node.
//!
//! Like `vn_elements::text_input`, this is a *component*, not a view: the host
//! owns the [`LottieState`] in a side map and rebuilds the element tree every
//! frame, so everything here is recomputed from scratch each time and nothing
//! but the state survives.

use gpui::{div, img, AnyElement, App, IntoElement, ObjectFit, Styled, StyledImage, Window};

use crate::LottieState;

/// CSS `object-fit`, as the cascade produces it.
///
/// Mirrors gpui's own [`ObjectFit`] one-for-one; it exists because gpui's enum
/// derives nothing, and the host needs a plain value it can compare and store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LottieFit {
    /// `object-fit: fill` — stretch to the box, ignoring aspect ratio.
    Fill,
    /// `object-fit: contain` — letterbox inside the box. The default, matching
    /// `<lottie-player>`'s `preserveAspectRatio: "xMidYMid meet"`.
    #[default]
    Contain,
    /// `object-fit: cover` — fill the box, cropping the overflow.
    Cover,
    /// `object-fit: scale-down` — `none` or `contain`, whichever is smaller.
    ScaleDown,
    /// `object-fit: none` — paint at intrinsic size.
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

/// Per-frame view parameters. Style comes from the host's CSS engine; nothing
/// in here is remembered between frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LottieOptions {
    /// The content box the animation paints into, in **logical** (CSS) pixels.
    /// The host takes this from the cascade, falling back to the animation's
    /// intrinsic [`LottieMeta::width`](crate::LottieMeta::width)/`height`.
    pub size: (f32, f32),
    /// Device pixels per logical pixel — `window.scale_factor()`. Drives the
    /// physical raster size; see [`LottieState::render_frame`].
    pub scale: f32,
    /// CSS `object-fit`. `Contain` matches `<lottie-player>`'s default
    /// `preserveAspectRatio: "xMidYMid meet"`.
    pub object_fit: LottieFit,
    /// CSS `filter: grayscale(1)`, which gpui's image element implements
    /// natively.
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
    /// Options that paint `state` at its intrinsic size on a `scale` display.
    pub fn intrinsic(state: &LottieState, scale: f32) -> Self {
        let meta = state.meta();
        Self {
            size: (meta.width as f32, meta.height as f32),
            scale,
            ..Self::default()
        }
    }
}

/// Builds the element for one frame.
///
/// Rasterises the current playhead (from cache when nothing moved), paints it,
/// and releases the atlas tiles of rasters that have fallen out of use — which
/// is why this needs `window`/`cx` and why the host must call *this* rather
/// than [`LottieState::render_frame`] directly.
///
/// A frame that cannot be rasterised (degenerate box, backend error) paints an
/// empty box of the same size rather than collapsing the layout.
pub fn lottie(
    state: &mut LottieState,
    options: &LottieOptions,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let rendered = state.render_frame(options.size, options.scale);

    for image in state.take_retired() {
        // `Some(window)`: we are inside this window's update, so `App.windows`
        // has its slot taken and `drop_image` would otherwise skip it.
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
