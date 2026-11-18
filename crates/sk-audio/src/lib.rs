//! Spatial audio output for Web Audio. On Windows, a context with a `PannerNode` whose
//! `panningModel` is `"spatial"` renders through `ISpatialAudioClient`: the destination plays in
//! the static bed (up to 7.1.4) and each such panner is a dynamic object. Without a spatial format
//! on the device, or elsewhere, the backend refuses and redjs plays the context through cpal with
//! HRTF.

use std::sync::Arc;

use redjs::webapi::audio::{OutputBackend, OutputOpen, OutputRequest, Renderer};

pub mod layout;
#[cfg(windows)]
mod wasapi;

/// The `PannerNode.panningModel` that asks for an object.
pub const MODEL: &str = "spatial";

pub struct SpatialBackend;

impl OutputBackend for SpatialBackend {
    fn object_model(&self) -> &'static str {
        MODEL
    }

    fn open(&self, request: &OutputRequest, renderer: Renderer) -> Result<OutputOpen, String> {
        if !request.objects {
            return Err("no PannerNode asked for spatial output".into());
        }
        #[cfg(windows)]
        {
            wasapi::open(request, renderer)
        }
        #[cfg(not(windows))]
        {
            drop(renderer);
            Err("spatial audio output is only on Windows".into())
        }
    }
}

/// Installs the backend for every context of the process; once, before the first page runs.
pub fn install() -> bool {
    redjs::webapi::audio::set_output_backend(Arc::new(SpatialBackend))
}
