//! The rasteriser seam.
//!
//! [`LottieState`](crate::LottieState) owns the transport, the metadata and
//! the atlas discipline; everything below this module is "turn frame *n* into
//! BGRA pixels". Two implementations sit behind it:
//!
//! | | crate | build cost |
//! | --- | --- | --- |
//! | [`Backend::ThorVg`] (default) | `vn-thorvg-sys` (vendored ThorVG 1.1.0 C++) | feature `thorvg`, ~54 s once |
//! | [`Backend::Rasterlottie`] | `rasterlottie` (pure Rust, `tiny-skia`) | none |
//!
//! The split is deliberately *below* metadata: [`LottieMeta`] is read from the
//! JSON document by [`crate::state`], not from whatever the backend happens to
//! expose, so `meta()` is bit-identical whichever engine is drawing. The same
//! goes for the `.tgs` gunzip and the [`crate::normalize`] pass — both run on
//! the document before a backend ever sees it.
//!
//! # Choosing one
//!
//! [`Backend::preferred`] resolves, in order:
//!
//! 1. `VN_LOTTIE_BACKEND=thorvg|rasterlottie` in the environment — which is
//!    how the comparison harness runs the *same* test binary on both engines,
//!    and how a user works around a file one engine gets wrong without a
//!    rebuild.
//! 2. the `thorvg-default` cargo feature (on by default),
//! 3. `rasterlottie`.
//!
//! An unavailable or misspelled value falls back rather than panicking: a
//! typo in an env var must not take the app down.

use serde_json::Value;

use crate::state::{LottieError, LottieMeta};

pub(crate) mod raster;
#[cfg(feature = "thorvg")]
pub(crate) mod thorvg;

/// Which rasteriser draws the frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// [`rasterlottie`] — pure Rust on `tiny-skia`. Always compiled in.
    Rasterlottie,
    /// Vendored ThorVG 1.1.0, software engine. Needs the `thorvg` feature;
    /// [`Backend::is_available`] reports whether it was compiled in.
    ThorVg,
}

impl Backend {
    /// The backend a plain [`LottieState::load`](crate::LottieState::load)
    /// uses. See the module docs for the resolution order.
    pub fn preferred() -> Self {
        // Read once: this is consulted on every load, and an env var cannot
        // meaningfully change mid-process.
        static PREFERRED: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();
        *PREFERRED.get_or_init(|| {
            let requested = std::env::var("VN_LOTTIE_BACKEND").ok();
            match requested.as_deref().map(str::trim) {
                Some("thorvg") | Some("ThorVG") => Backend::ThorVg,
                Some("rasterlottie") => Backend::Rasterlottie,
                _ => Backend::COMPILED_DEFAULT,
            }
            .available_or_fallback()
        })
    }

    /// What the build was configured to prefer, ignoring the environment.
    pub const COMPILED_DEFAULT: Self = if cfg!(feature = "thorvg-default") {
        Self::ThorVg
    } else {
        Self::Rasterlottie
    };

    /// True when this backend was compiled into the binary.
    pub fn is_available(self) -> bool {
        match self {
            Self::Rasterlottie => true,
            Self::ThorVg => cfg!(feature = "thorvg"),
        }
    }

    /// The name accepted by `VN_LOTTIE_BACKEND`, and used in diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rasterlottie => "rasterlottie",
            Self::ThorVg => "thorvg",
        }
    }

    fn available_or_fallback(self) -> Self {
        if self.is_available() {
            self
        } else {
            Self::Rasterlottie
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::preferred()
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One rasterised frame, in the byte order gpui's atlas wants.
pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    /// BGRA8, alpha-premultiplied, `width * height * 4` bytes.
    pub bgra: Vec<u8>,
}

/// A loaded animation, ready to draw. One variant per [`Backend`].
///
/// The `rasterlottie` payload is boxed because it is five times the size of
/// the ThorVG one (which is four pointers and a `Vec`), and this enum lives
/// inline in every `LottieState` the host keeps per node. One pointer chase
/// per `render_frame` against a rasterisation measured in microseconds.
pub(crate) enum Rasteriser {
    Rasterlottie(Box<raster::Rasterlottie>),
    #[cfg(feature = "thorvg")]
    ThorVg(thorvg::ThorVg),
}

impl Rasteriser {
    /// Hands the normalised document to `backend`.
    ///
    /// `document` is consumed because both backends re-encode it: the
    /// rasterlottie one deserialises it into its own model, the ThorVG one
    /// serialises it back to text for the C loader.
    // `meta` is only read by the ThorVG arm; without the feature it is dead.
    #[cfg_attr(not(feature = "thorvg"), allow(unused_variables))]
    pub fn load(
        backend: Backend,
        document: Value,
        meta: &LottieMeta,
    ) -> Result<Self, LottieError> {
        match backend {
            Backend::Rasterlottie => raster::Rasterlottie::load(document)
                .map(|inner| Self::Rasterlottie(Box::new(inner))),
            #[cfg(feature = "thorvg")]
            Backend::ThorVg => thorvg::ThorVg::load(document, meta).map(Self::ThorVg),
            #[cfg(not(feature = "thorvg"))]
            Backend::ThorVg => Err(LottieError::Parse(
                "the `thorvg` backend was not compiled into this binary \
                 (build with --features thorvg)"
                    .into(),
            )),
        }
    }

    /// Which engine this is.
    pub fn backend(&self) -> Backend {
        match self {
            Self::Rasterlottie(_) => Backend::Rasterlottie,
            #[cfg(feature = "thorvg")]
            Self::ThorVg(_) => Backend::ThorVg,
        }
    }

    /// Draws absolute source `frame` at `scale` into a `width × height`
    /// raster.
    ///
    /// `width`/`height` are what [`raster_size`] computed from `scale`; they
    /// are passed rather than recomputed so the two backends cannot disagree
    /// about the output dimensions by a rounding step.
    // Ditto for the explicit dimensions: `rasterlottie` derives them itself.
    #[cfg_attr(not(feature = "thorvg"), allow(unused_variables))]
    pub fn render(&mut self, frame: f32, scale: f32, width: u32, height: u32) -> Option<Raster> {
        match self {
            Self::Rasterlottie(inner) => inner.render(frame, scale),
            #[cfg(feature = "thorvg")]
            Self::ThorVg(inner) => inner.render(frame, scale, width, height),
        }
    }
}

/// The raster dimensions an animation of `(width, height)` produces at `scale`.
///
/// `ceil` — not `round` — because that is what `rasterlottie`'s
/// `resolve_output_canvas_size` does, and the two backends must agree on the
/// size of the texture gpui gets. A 100×50 animation in a 128×128 box scales
/// by 1.28 and rasterises 128×64.
pub(crate) fn raster_size(intrinsic: (u32, u32), scale: f32) -> (u32, u32) {
    let width = ((intrinsic.0 as f32) * scale).ceil().max(1.0) as u32;
    let height = ((intrinsic.1 as f32) * scale).ceil().max(1.0) as u32;
    (width, height)
}
