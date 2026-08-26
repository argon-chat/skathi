//! The [`rasterlottie`] backend — pure Rust, `tiny-skia`, no build step.
//!
//! Its distinguishing behaviour is that it *validates*:
//! `analyze_animation` walks the model and refuses anything it cannot draw
//! exactly, so an unsupported feature surfaces as
//! [`LottieError::Unsupported`] at load with one advisory line per feature,
//! rather than as a silently wrong frame. See `crate::normalize` for the two
//! Bodymovin idioms that have to be rewritten before that check passes.

use rasterlottie::{Animation, PreparedAnimation, RenderConfig, Renderer, Rgba8};
use serde_json::Value;

use super::Raster;
use crate::state::LottieError;

pub(crate) struct Rasterlottie {
    prepared: PreparedAnimation,
}

impl Rasterlottie {
    pub fn load(document: Value) -> Result<Self, LottieError> {
        let animation: Animation = serde_json::from_value(document)
            .map_err(|error| LottieError::Parse(error.to_string()))?;

        let report = rasterlottie::analyze_animation(&animation);
        if !report.is_supported() {
            return Err(LottieError::Unsupported(
                report
                    .issues
                    .iter()
                    .map(|issue| format!("{} ({:?}): {}", issue.path, issue.kind, issue.detail))
                    .collect(),
            ));
        }

        let prepared = Renderer::default()
            .prepare(&animation)
            .map_err(|error| LottieError::Parse(error.to_string()))?;
        Ok(Self { prepared })
    }

    pub fn render(&mut self, frame: f32, scale: f32) -> Option<Raster> {
        let raster = self
            .prepared
            .render_frame(frame, RenderConfig::new(Rgba8::TRANSPARENT, scale))
            .ok()?;

        // rasterlottie hands back premultiplied RGBA; gpui's atlas wants BGRA.
        let mut bgra = raster.pixels;
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Some(Raster {
            width: raster.width,
            height: raster.height,
            bgra,
        })
    }
}
