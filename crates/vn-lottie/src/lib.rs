//! Lottie (Bodymovin) animation playback for gpui, the crate behind `<lottie src="anim.json" autoplay loop />`.
//!
//! Split as in `vn-elements`: [`LottieState`] survives a frame (prepared animation, transport,
//! rendered-frame cache; the host keeps one per node), [`LottieOptions`] is recomputed per frame
//! (content box, display scale, `object-fit`, `grayscale`), and [`lottie`] builds the element.
//!
//! [`Animation`] is the same load without the transport or any gpui type: whole frames,
//! straight-alpha bytes, `Send`. The sticker engine (`crates/vn-sticker`) draws with it on a worker thread.
//!
//! ```ignore
//! let mut state = LottieState::load(&bytes)?;
//! let advance = state.advance(dt);
//! for event in advance.events() { /* dispatch to JS */ }
//! let element = lottie(&mut state, &options, window, cx);
//! ```
//!
//! Rasterisation happens behind [`Backend`]; the `.tgs` gunzip, the [`normalize`] pass,
//! [`LottieMeta`], the transport and the events are shared above it.
//!
//! * [`Backend::ThorVg`]: vendored ThorVG 1.1.0 C++ behind the `thorvg` feature, on by default;
//!   wider format coverage and faster, with no cmake or system libraries.
//! * [`Backend::Rasterlottie`]: pure Rust on `tiny-skia`, always compiled in; the fallback for
//!   `bell.json`, whose Fill effect ThorVG drops.
//!
//! `VN_LOTTIE_BACKEND=thorvg|rasterlottie` overrides the choice at run time. `rasterlottie` rejects
//! unaltered After Effects exports, so [`normalize`] rewrites two Bodymovin idioms first. Prop
//! names, transport semantics and event names mirror lottie-web.

pub mod animation;
pub mod backend;
pub mod element;
pub mod normalize;
pub mod state;
pub mod transport;

pub use animation::Animation;
pub use transport::Transport;
pub use backend::Backend;
pub use element::{lottie, LottieFit, LottieOptions};
pub use normalize::Normalization;
pub use state::{
    raster_size_for, Advance, Direction, LoopMode, LottieError, LottieEvent, LottieMeta, LottieState, PlayMode,
};

/// True if any state in `states` still needs the frame pump running.
pub fn any_playing<'a>(states: impl IntoIterator<Item = &'a LottieState>) -> bool {
    states.into_iter().any(LottieState::needs_frame)
}
