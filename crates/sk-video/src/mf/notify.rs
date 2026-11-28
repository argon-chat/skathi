//! `IMFMediaEngineNotify`: the OS's event callback and the queue it feeds.
//!
//! Never touch anything but the queue here: `EventNotify` runs on a Media Foundation worker thread
//! and a JS handler can mutate the retained tree. Events go into a mutex'd `Vec` drained on the UI
//! thread in the frame pump.

use std::sync::{Arc, Mutex};

use sk_windows::core::implement;
use sk_windows::win32::{IMFMediaEngineNotify, IMFMediaEngineNotify_Impl};

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
    fn EventNotify(
        &self,
        event: u32,
        _param1: usize,
        _param2: u32,
    ) -> sk_windows::core::Result<()> {
        if let Ok(mut queue) = self.queue.lock() {
            // Bounds a stalled UI thread's backlog; the newest events describe the current state, so the oldest go.
            if queue.len() >= 4096 {
                queue.remove(0);
            }
            queue.push(event);
        }
        Ok(())
    }
}

/// `MF_MEDIA_ENGINE_EVENT` → [`MediaEvent`]: values below 1000 are the HTML 5 set; MF's own events above 1000 are dropped, with two exceptions below.
pub fn translate(code: u32) -> Option<MediaEvent> {
    Some(match code {
        1 => MediaEvent::LoadStart,
        5 => MediaEvent::Error,
        6 => MediaEvent::Emptied,
        // 7 stalled: folded into `waiting`, which a buffering UI relies on.
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
