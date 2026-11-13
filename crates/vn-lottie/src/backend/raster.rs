//! The [`rasterlottie`] backend: pure Rust, `tiny-skia`. It validates: an unsupported feature
//! surfaces as [`LottieError::Unsupported`] at load, one advisory per feature, rather than a
//! silently wrong frame. See `crate::normalize`.

use rasterlottie::{Animation, PreparedAnimation, RenderConfig, Renderer, Rgba8};
use serde_json::Value;

use super::Raster;
use crate::state::LottieError;

pub(crate) struct Rasterlottie {
    prepared: PreparedAnimation,
    straight: Vec<u8>,
}

// SAFETY: `PreparedAnimation` is `!Send` only because its static path cache holds
// `Rc<tiny_skia::Path>`s (rasterlottie 0.2.2). Every clone lives inside this one graph
// (`render_frame`'s clones drop before it returns) and rasterlottie keeps no thread-local or global
// state that could hold one, so the graph moves between threads intact and is never shared
// (`!Sync`). Re-check on any rasterlottie upgrade.
unsafe impl Send for Rasterlottie {}

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
        Ok(Self {
            prepared,
            straight: Vec::new(),
        })
    }

    /// The sticker engine's raster: straight-alpha BGRA. rasterlottie scales uniformly only, so the
    /// animation is drawn at the fitting scale, top-left, leaving at most a pixel of transparent edge.
    pub fn render_straight(&mut self, frame: f32, intrinsic: (u32, u32), width: u32, height: u32) -> Option<&[u8]> {
        if width == 0 || height == 0 {
            return None;
        }
        let scale = (width as f32 / intrinsic.0.max(1) as f32).min(height as f32 / intrinsic.1.max(1) as f32);
        let raster = self.render(frame, scale)?;
        let out = &mut self.straight;
        out.clear();
        out.resize(width as usize * height as usize * 4, 0);
        let rows = raster.height.min(height) as usize;
        let cols = raster.width.min(width) as usize;
        for y in 0..rows {
            let src = &raster.bgra[y * raster.width as usize * 4..][..cols * 4];
            let dst = &mut out[y * width as usize * 4..][..cols * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                d.copy_from_slice(&super::unpremultiply([s[0], s[1], s[2], s[3]]));
            }
        }
        Some(out)
    }

    pub fn buffer_bytes(&self) -> usize {
        self.straight.capacity()
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
