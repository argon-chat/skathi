//! The rasteriser seam: [`LottieState`](crate::LottieState) owns the transport, metadata and atlas
//! discipline; a backend only turns frame *n* into BGRA pixels. [`LottieMeta`] is read from the JSON
//! by [`crate::state`], so `meta()` is identical whichever engine draws.
//!
//! [`Backend::preferred`] resolves, in order: `SK_LOTTIE_BACKEND=thorvg|rasterlottie`, the
//! `thorvg-default` feature, then `rasterlottie`. An unavailable or misspelled value falls back
//! rather than panicking.

use serde_json::Value;

use crate::state::{LottieError, LottieMeta};

pub(crate) mod raster;
#[cfg(feature = "thorvg")]
pub(crate) mod thorvg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Rasterlottie,
    ThorVg,
}

impl Backend {
    pub fn preferred() -> Self {
        static PREFERRED: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();
        *PREFERRED.get_or_init(|| {
            let requested = std::env::var("SK_LOTTIE_BACKEND").ok();
            match requested.as_deref().map(str::trim) {
                Some("thorvg") | Some("ThorVG") => Backend::ThorVg,
                Some("rasterlottie") => Backend::Rasterlottie,
                _ => Backend::COMPILED_DEFAULT,
            }
            .available_or_fallback()
        })
    }

    pub const COMPILED_DEFAULT: Self = if cfg!(feature = "thorvg-default") {
        Self::ThorVg
    } else {
        Self::Rasterlottie
    };

    pub fn is_available(self) -> bool {
        match self {
            Self::Rasterlottie => true,
            Self::ThorVg => cfg!(feature = "thorvg"),
        }
    }

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

pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// A loaded animation; one variant per [`Backend`]. The `rasterlottie` payload is boxed: it is five
/// times the ThorVG one and this enum is inline in every `LottieState`.
pub(crate) enum Rasteriser {
    Rasterlottie(Box<raster::Rasterlottie>),
    #[cfg(feature = "thorvg")]
    ThorVg(thorvg::ThorVg),
}

impl Rasteriser {
    // `meta` is only read by the ThorVG arm; without the feature it is dead.
    #[cfg_attr(not(feature = "thorvg"), allow(unused_variables))]
    pub fn load(backend: Backend, document: Value, meta: &LottieMeta) -> Result<Self, LottieError> {
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

    pub fn backend(&self) -> Backend {
        match self {
            Self::Rasterlottie(_) => Backend::Rasterlottie,
            #[cfg(feature = "thorvg")]
            Self::ThorVg(_) => Backend::ThorVg,
        }
    }

    /// Draws absolute source `frame` at `scale` into a `width × height` raster (dimensions from [`raster_size`], so the backends cannot disagree).
    #[cfg_attr(not(feature = "thorvg"), allow(unused_variables))]
    pub fn render(&mut self, frame: f32, scale: f32, width: u32, height: u32) -> Option<Raster> {
        match self {
            Self::Rasterlottie(inner) => inner.render(frame, scale),
            #[cfg(feature = "thorvg")]
            Self::ThorVg(inner) => inner.render(frame, scale, width, height),
        }
    }

    /// Draws absolute source `frame` filling a `width × height` raster: straight-alpha BGRA, rows packed, borrowed until the next call.
    #[cfg_attr(not(feature = "thorvg"), allow(unused_variables))]
    pub fn render_straight(
        &mut self,
        frame: f32,
        intrinsic: (u32, u32),
        width: u32,
        height: u32,
    ) -> Option<&[u8]> {
        match self {
            Self::Rasterlottie(inner) => inner.render_straight(frame, intrinsic, width, height),
            #[cfg(feature = "thorvg")]
            Self::ThorVg(inner) => inner.render_straight(frame, width, height),
        }
    }

    pub fn buffer_bytes(&self) -> usize {
        match self {
            Self::Rasterlottie(inner) => inner.buffer_bytes(),
            #[cfg(feature = "thorvg")]
            Self::ThorVg(inner) => inner.target_bytes(),
        }
    }
}

/// One premultiplied BGRA pixel as straight alpha (`c · 255 / a`, rounded; fully transparent is all zeros).
pub(crate) fn unpremultiply(px: [u8; 4]) -> [u8; 4] {
    let a = px[3] as u32;
    match a {
        0 => [0; 4],
        255 => px,
        _ => {
            let c = |v: u8| ((v as u32 * 255 + a / 2) / a).min(255) as u8;
            [c(px[0]), c(px[1]), c(px[2]), px[3]]
        }
    }
}

/// The raster dimensions of an animation of `(width, height)` at `scale`: `ceil`, as `rasterlottie`
/// does, so the backends agree on the texture size.
pub(crate) fn raster_size(intrinsic: (u32, u32), scale: f32) -> (u32, u32) {
    let width = ((intrinsic.0 as f32) * scale).ceil().max(1.0) as u32;
    let height = ((intrinsic.1 as f32) * scale).ceil().max(1.0) as u32;
    (width, height)
}
