//! `<video>` playback for gpui, without shipping a codec.
//!
//! A thin portable element over a [`VideoBackend`] that the platform implements (Windows:
//! `IMFMediaEngine` in frame-server mode; macOS: `AVPlayer`): the OS player is hardware-accelerated,
//! carries its own codec licences and has an `HTMLMediaElement`-shaped API.
//!
//! Split as in `sk-lottie`: [`VideoState`] survives a frame (backend, transport, frame cache; the
//! host keeps one per node), [`VideoOptions`] is recomputed per frame (content box, display scale,
//! `object-fit`), and [`video`] builds the element.
//! ```ignore
//! let mut state = VideoState::new(backend::for_this_platform()?);
//! state.set_source(MediaSource::Path(path))?;
//! let advance = state.advance(Instant::now(), &FrameRequest { max_size });
//! for event in advance.events { /* dispatch to JS */ }
//! let element = video(&mut state, &options, window, cx);
//! ```
//! Codec availability is a runtime fact: [`VideoState::can_play_type`] is a live probe and a failed
//! load produces an advisory naming the missing decoder ([`MediaError::advisory`]). The MF
//! implementation is `#[cfg(windows)]`; [`StubBackend`] keeps the portable half testable everywhere.

pub mod backend;
pub mod element;
pub mod state;
pub mod stub;
pub mod stream;

#[cfg(windows)]
pub mod mf;

pub use backend::{
    CanPlay, FrameRequest, GpuFrame, HeldGpuFrame, MediaError, MediaErrorCode, MediaEvent,
    MediaSource, NetworkState, ReadyState, VideoBackend, VideoFrame,
};
pub use element::{video, VideoFit, VideoOptions};
pub use state::{Advance, VideoState};
pub use stub::StubBackend;

/// Fits `natural` inside `bound` at the natural aspect ratio, never upscaling (see [`FrameRequest`]).
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
            "sk-video ships a Media Foundation backend for Windows; the macOS \
             (AVFoundation) one is designed but not written yet.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_never_asked_for_larger_than_it_is_encoded() {
        // The box is bigger than the video: stay at native.
        assert_eq!(fit_within((1920, 1080), (3840, 2160)), (1920, 1080));
        assert_eq!(fit_within((1920, 1080), (800, 450)), (800, 450));
        assert_eq!(fit_within((1920, 1080), (800, 2000)), (800, 450));
        assert_eq!(fit_within((1920, 1080), (4000, 450)), (800, 450));
        assert_eq!(fit_within((1920, 1080), (0, 100)), (0, 0));
    }
}
