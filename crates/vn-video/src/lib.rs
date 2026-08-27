//! vn-video — `<video>` playback for gpui, without shipping a codec.
//!
//! # The thesis
//!
//! We do not write a player. Every desktop OS we target already has one, it is
//! hardware-accelerated, it carries its own codec licences, and its API is
//! *already shaped like `HTMLMediaElement`* — because both were designed for
//! the same job. So this crate is a thin, portable element on top of a
//! [`VideoBackend`] that the platform implements:
//!
//! | | Windows | macOS (next) |
//! |---|---|---|
//! | player | `IMFMediaEngine` (frame-server mode) | `AVPlayer` + `AVPlayerItem` |
//! | frame pull | `OnVideoStreamTick` + `TransferVideoFrame` | `AVPlayerItemVideoOutput.copyPixelBuffer` |
//! | audio + A/V clock | the OS | the OS |
//! | events | `MF_MEDIA_ENGINE_EVENT` (< 1000 = the HTML 5 set, documented) | KVO + `AVPlayerItemDidPlayToEndTime` |
//!
//! `research/reports/m11/video.md` is the measurement behind that choice; the
//! short version is that a bundled ffmpeg costs 3–70 MB of binary, 60–160 MiB
//! of RAM, a per-frame colour conversion and an 8.3 MB/frame PCIe upload, in
//! exchange for codecs Windows already decodes at 27–31× realtime without the
//! frame ever leaving VRAM.
//!
//! # Shape of the API
//!
//! It follows `vn-lottie`'s split, so wiring it into the renderer is
//! mechanical (`CONTRACT.md` is the wiring spec):
//!
//! * [`VideoState`] — everything that must survive a frame: the backend, the
//!   transport, the frame cache. The host keeps one per node in a side map
//!   keyed by node id.
//! * [`VideoOptions`] — what the CSS engine recomputes per frame: the content
//!   box, the display scale, `object-fit`.
//! * [`video`] — builds the element for one frame out of the two.
//!
//! ```ignore
//! // once per <video> node, in the host's side map
//! let mut state = VideoState::new(backend::for_this_platform()?);
//! state.set_source(MediaSource::Path(path))?;
//!
//! // every frame, from the pump
//! let advance = state.advance(Instant::now(), &FrameRequest { max_size });
//! for event in advance.events { /* dispatch to JS */ }
//!
//! // every frame, from the element build
//! let element = video(&mut state, &options, window, cx);
//! ```
//!
//! # Two owner constraints this crate is built around
//!
//! 1. **No reliance on Store codec extensions.** The guaranteed baseline is
//!    what Windows ships inbox: H.264 + AAC in MP4. Everything else — HEVC,
//!    VP9, AV1, VP8/Opus/Vorbis — is a *runtime* fact, so
//!    [`VideoState::can_play_type`] is a live probe and a failed load produces
//!    an advisory that names the missing decoder and what to do, never a
//!    static table and never a silent black box. See [`MediaError::advisory`].
//! 2. **Portability is a design requirement.** [`VideoBackend`] is sized so
//!    `IMFMediaEngine` and `AVPlayerItemVideoOutput` both fit naturally, and no
//!    platform type appears in `backend.rs`, `state.rs` or `element.rs`. The MF
//!    implementation is `#[cfg(windows)]`; [`StubBackend`] keeps the portable
//!    half testable everywhere.

pub mod backend;
pub mod element;
pub mod state;
pub mod stub;

#[cfg(windows)]
pub mod mf;

pub use backend::{
    CanPlay, FrameRequest, GpuFrame, MediaError, MediaErrorCode, MediaEvent, MediaSource,
    NetworkState, ReadyState, VideoBackend, VideoFrame,
};
pub use element::{video, VideoFit, VideoOptions};
pub use state::{Advance, VideoState};
pub use stub::StubBackend;

/// Fits `natural` inside `bound` at the natural aspect ratio, never upscaling.
///
/// This is what turns the readback cost into a function of the *painted* size
/// rather than the *encoded* size — see [`FrameRequest`].
pub fn fit_within(natural: (u32, u32), bound: (u32, u32)) -> (u32, u32) {
    let (nw, nh) = (natural.0.max(1), natural.1.max(1));
    let (bw, bh) = bound;
    if bw == 0 || bh == 0 {
        return (0, 0);
    }
    if bw >= nw && bh >= nh {
        return (nw, nh);
    }
    let scale = (bw as f64 / nw as f64).min(bh as f64 / nh as f64);
    (
        ((nw as f64 * scale).round() as u32).max(1),
        ((nh as f64 * scale).round() as u32).max(1),
    )
}

/// Builds the backend this platform has, or explains what is missing.
///
/// The advisory path is the point: a machine with no usable media stack must
/// produce one clear line, not a silent black rectangle.
pub fn backend_for_this_platform() -> Result<Box<dyn VideoBackend>, MediaError> {
    #[cfg(windows)]
    {
        mf::MediaEngineBackend::new().map(|b| Box::new(b) as Box<dyn VideoBackend>)
    }
    #[cfg(not(windows))]
    {
        Err(MediaError::new(
            backend::MediaErrorCode::SrcNotSupported,
            "no <video> backend is compiled in for this platform",
        )
        .with_advisory(
            "vn-video ships a Media Foundation backend for Windows; the macOS \
             (AVFoundation) one is designed but not written yet — see CONTRACT.md §8.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_never_asked_for_larger_than_it_is_encoded() {
        // The box is bigger than the video: stay at native, do not upscale a
        // readback we would then hand the GPU to upscale again.
        assert_eq!(fit_within((1920, 1080), (3840, 2160)), (1920, 1080));
        // The box is smaller: fit the aspect ratio inside it. 1080p in an
        // 800x450 box is 1.4 MB/frame instead of 8.3.
        assert_eq!(fit_within((1920, 1080), (800, 450)), (800, 450));
        // A box of the wrong shape is bounded by its tighter axis.
        assert_eq!(fit_within((1920, 1080), (800, 2000)), (800, 450));
        assert_eq!(fit_within((1920, 1080), (4000, 450)), (800, 450));
        // Degenerate boxes produce no frame at all rather than a 1x1 one.
        assert_eq!(fit_within((1920, 1080), (0, 100)), (0, 0));
    }
}
