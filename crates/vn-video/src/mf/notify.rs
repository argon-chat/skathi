//! `IMFMediaEngineNotify` — the OS's event callback, and the queue it feeds.
//!
//! **The one non-negotiable rule here: never touch anything but the queue.**
//! `EventNotify` is called from a Media Foundation worker thread. A JS handler
//! can mutate the retained tree and gpui's `Window` is not `Send`, so anything
//! dispatched from here would be a data race at best. Events go into a mutex'd
//! `Vec` and are drained on the UI thread inside the frame pump — the same
//! collect-first / dispatch-after discipline `vn_tree::lottie::tick`
//! documents.

use std::sync::{Arc, Mutex};

use windows::core::implement;
use windows::Win32::Media::MediaFoundation::{
    IMFMediaEngineNotify, IMFMediaEngineNotify_Impl,
};

use crate::backend::MediaEvent;

#[implement(IMFMediaEngineNotify)]
pub struct EngineNotify {
    queue: Arc<Mutex<Vec<u32>>>,
}

impl EngineNotify {
    pub fn new(queue: Arc<Mutex<Vec<u32>>>) -> Self {
        Self { queue }
    }
}

impl IMFMediaEngineNotify_Impl for EngineNotify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        if let Ok(mut queue) = self.queue.lock() {
            // A stalled UI thread must not turn into unbounded growth. 4096
            // events is minutes of the busiest stream; past that the oldest go,
            // because the newest are the ones that describe the current state.
            if queue.len() >= 4096 {
                queue.remove(0);
            }
            queue.push(event);
        }
        Ok(())
    }
}

/// `MF_MEDIA_ENGINE_EVENT` → [`MediaEvent`].
///
/// Microsoft documents every value below 1000 as "events defined in HTML 5 for
/// media elements", which is why this function is a rename rather than a state
/// machine. The values above 1000 are MF's own (`MF_MEDIA_ENGINE_EVENT_
/// FORMATCHANGE`, `..._PURGEQUEUEDEVENTS`, the frame-server and audio-endpoint
/// notifications) and have no HTML counterpart, so they are dropped here — with
/// the two exceptions noted below.
pub fn translate(code: u32) -> Option<MediaEvent> {
    Some(match code {
        1 => MediaEvent::LoadStart,
        // 2 progress / 3 suspend / 4 abort — tier 2, see CONTRACT.md §4.
        5 => MediaEvent::Error,
        6 => MediaEvent::Emptied,
        // 7 stalled — folded into `waiting`, which is the one the spec makes
        // load-bearing for a buffering UI.
        7 => MediaEvent::Waiting,
        8 => MediaEvent::Play,
        9 => MediaEvent::Pause,
        10 => MediaEvent::LoadedMetadata,
        11 => MediaEvent::LoadedData,
        12 => MediaEvent::Waiting,
        13 => MediaEvent::Playing,
        14 => MediaEvent::CanPlay,
        15 => MediaEvent::CanPlayThrough,
        16 => MediaEvent::Seeking,
        17 => MediaEvent::Seeked,
        18 => MediaEvent::TimeUpdate,
        19 => MediaEvent::Ended,
        20 => MediaEvent::RateChange,
        21 => MediaEvent::DurationChange,
        22 => MediaEvent::VolumeChange,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The v1 event set, code by code. This is the table that would silently
    /// rot if MF's enum and ours ever drifted.
    #[test]
    fn the_html5_range_maps_one_for_one() {
        assert_eq!(translate(1), Some(MediaEvent::LoadStart));
        assert_eq!(translate(5), Some(MediaEvent::Error));
        assert_eq!(translate(8), Some(MediaEvent::Play));
        assert_eq!(translate(9), Some(MediaEvent::Pause));
        assert_eq!(translate(10), Some(MediaEvent::LoadedMetadata));
        assert_eq!(translate(13), Some(MediaEvent::Playing));
        assert_eq!(translate(17), Some(MediaEvent::Seeked));
        assert_eq!(translate(19), Some(MediaEvent::Ended));
        assert_eq!(translate(22), Some(MediaEvent::VolumeChange));
        // MF's own events carry no HTML meaning and must not leak out.
        assert_eq!(translate(1000), None);
        assert_eq!(translate(1002), None);
    }
}
