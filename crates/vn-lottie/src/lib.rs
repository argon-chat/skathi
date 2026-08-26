//! vn-lottie — Lottie (Bodymovin) animation playback for gpui, the crate behind
//! a future `<lottie src="anim.json" autoplay loop />` element.
//!
//! # Shape of the API
//!
//! It follows `vn-elements`' split, so wiring it into the renderer is
//! mechanical (the full wiring spec is in `CONTRACT.md`):
//!
//! * [`LottieState`] — everything that must survive a frame: the prepared
//!   animation, the transport (play/pause/seek/speed/direction/loop) and the
//!   rendered-frame cache. The host keeps one per node in a side map keyed by
//!   node id, exactly as it keeps `TextInputState` for `<input>`.
//! * [`LottieOptions`] — what the CSS engine recomputes per frame: the content
//!   box, the display scale, `object-fit`, `grayscale`.
//! * [`lottie`] — builds the element for one frame out of the two.
//!
//! ```ignore
//! // once per <lottie> node, kept in the host's side map
//! let mut state = LottieState::load(&bytes)?;
//!
//! // every frame
//! let advance = state.advance(dt);
//! for event in advance.events() { /* dispatch to JS */ }
//! let element = lottie(&mut state, &options, window, cx);
//! ```
//!
//! # Backend
//!
//! Rasterisation is [`rasterlottie`] (pure Rust, `tiny-skia`), rendering ARGB
//! frames into a [`gpui::RenderImage`] — the same raster-frames approach
//! Telegram uses for stickers. It was chosen over rlottie, ThorVG and Skia's
//! Skottie because it is the only option that builds on Windows MSVC with
//! `+crt-static` in one `cargo build`, with no cmake, no `libclang`, no network
//! fetch and no system libraries. `research/reports/m10/lottie.md` has the
//! comparison and the measurements.
//!
//! Real After Effects exports do not reach that backend unaltered: see
//! [`normalize`] for the two Bodymovin idioms that have to be rewritten first,
//! and why skipping that step rejects most files in the wild.
//!
//! # Compat
//!
//! Prop names, transport semantics and event names mirror lottie-web so a
//! `<lottie>` component ports from the browser unchanged. Where lottie-web and
//! `<lottie-player>` disagree — notably the defaults of `autoplay` and `loop` —
//! `CONTRACT.md` records which one this follows and why.

pub mod element;
pub mod normalize;
pub mod state;

pub use element::{lottie, LottieFit, LottieOptions};
pub use normalize::Normalization;
pub use state::{
    Advance, Direction, LoopMode, LottieError, LottieEvent, LottieMeta, LottieState, PlayMode,
};

/// True if any state in `states` still needs the frame pump running.
///
/// The host folds this into its `animating()` OR-chain next to the CSS
/// transition animator, so a playing Lottie keeps requesting frames:
///
/// ```ignore
/// if vn_tree::flush_changes() || vn_tree::animating() || vn_tree::lotties_animating() {
///     // notify -> repaint
/// }
/// ```
pub fn any_playing<'a>(states: impl IntoIterator<Item = &'a LottieState>) -> bool {
    states.into_iter().any(LottieState::needs_frame)
}
