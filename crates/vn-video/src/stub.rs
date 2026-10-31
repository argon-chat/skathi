//! A synthetic backend: no OS, no codec, no GPU.
//!
//! It exists so the portable half of this crate is testable on a machine that
//! has neither Media Foundation nor AVFoundation — `cargo test -p vn-video`
//! runs the cadence, idempotence and atlas-hygiene tests above on any platform,
//! and CI does not need a GPU. It is also the reference implementation of the
//! trait: everything [`VideoBackend`] asks of a platform is done here in eighty
//! lines, which is a useful sanity check that the trait is at the right
//! altitude.
//!
//! The clock is driven by [`present`](StubBackend::present) rather than by the
//! wall clock, deliberately: a test that has to sleep is a test that is flaky.

use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaEvent, MediaSource, NetworkState, ReadyState,
    VideoBackend, VideoFrame,
};

/// The nominal frame period the synthetic clock advances by per `present`.
const FRAME: f64 = 1.0 / 60.0;

pub struct StubBackend {
    duration: f64,
    native: (u32, u32),
    time: f64,
    paused: bool,
    ended: bool,
    looping: bool,
    rate: f64,
    volume: f64,
    muted: bool,
    loaded: bool,
    events: Vec<MediaEvent>,
    frames: u64,
}

impl StubBackend {
    pub fn new(duration_secs: f64, native_size: (u32, u32)) -> Self {
        Self {
            duration: duration_secs,
            native: native_size,
            time: 0.0,
            paused: true,
            ended: false,
            looping: false,
            rate: 1.0,
            volume: 1.0,
            muted: false,
            loaded: false,
            events: Vec::new(),
            frames: 0,
        }
    }

    /// How many frames this backend has produced — the handle a test uses to
    /// assert that motion actually happened.
    pub fn frames_produced(&self) -> u64 {
        self.frames
    }
}

impl VideoBackend for StubBackend {
    fn load(&mut self, _source: &MediaSource) -> Result<(), MediaError> {
        self.loaded = true;
        self.time = 0.0;
        self.ended = false;
        self.events.extend([
            MediaEvent::LoadStart,
            MediaEvent::DurationChange,
            MediaEvent::LoadedMetadata,
            MediaEvent::LoadedData,
            MediaEvent::CanPlay,
        ]);
        Ok(())
    }

    fn play(&mut self) {
        if !self.paused {
            return;
        }
        self.paused = false;
        self.ended = false;
        self.events.extend([MediaEvent::Play, MediaEvent::Playing]);
    }

    fn pause(&mut self) {
        if self.paused {
            return;
        }
        self.paused = true;
        self.events.push(MediaEvent::Pause);
    }

    fn seek(&mut self, seconds: f64) {
        self.time = seconds.clamp(0.0, self.duration);
        self.ended = false;
        self.events.extend([MediaEvent::Seeking, MediaEvent::Seeked]);
    }

    fn set_volume(&mut self, volume: f64) {
        self.volume = volume;
        self.events.push(MediaEvent::VolumeChange);
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.events.push(MediaEvent::VolumeChange);
    }

    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.events.push(MediaEvent::RateChange);
    }

    fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    fn current_time(&self) -> f64 {
        self.time
    }
    fn duration(&self) -> f64 {
        if self.loaded { self.duration } else { f64::NAN }
    }
    fn paused(&self) -> bool {
        self.paused
    }
    fn ended(&self) -> bool {
        self.ended
    }
    fn seeking(&self) -> bool {
        false
    }
    fn ready_state(&self) -> ReadyState {
        if self.loaded { ReadyState::HaveEnoughData } else { ReadyState::HaveNothing }
    }
    fn network_state(&self) -> NetworkState {
        if self.loaded { NetworkState::Idle } else { NetworkState::Empty }
    }
    fn native_size(&self) -> Option<(u32, u32)> {
        self.loaded.then_some(self.native)
    }
    fn buffered(&self) -> Vec<(f64, f64)> {
        if self.loaded { vec![(0.0, self.duration)] } else { Vec::new() }
    }
    fn seekable(&self) -> Vec<(f64, f64)> {
        self.buffered()
    }
    fn error(&self) -> Option<MediaError> {
        None
    }

    /// The stub speaks for nothing real, so it answers `Maybe` for anything
    /// that looks like a video type and `No` otherwise — enough to exercise the
    /// call site, honest about knowing nothing.
    fn can_play_type(&self, mime_type: &str) -> CanPlay {
        if mime_type.trim().to_ascii_lowercase().starts_with("video/") {
            CanPlay::Maybe
        } else {
            CanPlay::No
        }
    }

    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame> {
        if !self.loaded {
            return None;
        }
        if !self.paused && !self.ended {
            self.time += FRAME * self.rate;
            if self.time >= self.duration {
                if self.looping {
                    self.time = 0.0;
                } else {
                    self.time = self.duration;
                    self.ended = true;
                    self.paused = true;
                    self.events.push(MediaEvent::Ended);
                }
            }
        }
        let size = crate::fit_within(self.native, request.max_size);
        let (w, h) = size;
        if w == 0 || h == 0 {
            return None;
        }
        // A vertical bar sweeping left to right: two frames at different times
        // are never bit-identical, which is what a motion test asserts on.
        let progress = if self.duration > 0.0 { self.time / self.duration } else { 0.0 };
        let bar = (progress * w as f64) as u32;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let lit = x.abs_diff(bar) < 4;
                // BGRA, which is what gpui's atlas uploads.
                buf[i] = if lit { 255 } else { (x * 255 / w.max(1)) as u8 };
                buf[i + 1] = if lit { 255 } else { (y * 255 / h.max(1)) as u8 };
                buf[i + 2] = if lit { 255 } else { 64 };
                buf[i + 3] = 255;
            }
        }
        self.frames += 1;
        let buffer = RgbaImage::from_raw(w, h, buf)?;
        Some(VideoFrame {
            image: Arc::new(RenderImage::new(vec![Frame::new(buffer)]).dynamic()),
            pts: self.time,
            size,
            // No OS and no GPU: the synthetic backend is the CPU path by
            // definition, which is also what keeps it testable in CI.
            gpu: None,
        })
    }

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        for event in std::mem::take(&mut self.events) {
            sink(event);
        }
    }
}
