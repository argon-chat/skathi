//! A synthetic backend with no OS, codec or GPU, so the portable half of the crate is testable
//! anywhere. Its clock is driven by [`present`](StubBackend::present), not the wall clock, so tests
//! never sleep.

use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaEvent, MediaSource, NetworkState, ReadyState,
    VideoBackend, VideoFrame,
};

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
        if self.ended {
            self.time = 0.0;
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
        self.events
            .extend([MediaEvent::Seeking, MediaEvent::Seeked]);
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
        if self.loaded {
            self.duration
        } else {
            f64::NAN
        }
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
        if self.loaded {
            ReadyState::HaveEnoughData
        } else {
            ReadyState::HaveNothing
        }
    }
    fn network_state(&self) -> NetworkState {
        if self.loaded {
            NetworkState::Idle
        } else {
            NetworkState::Empty
        }
    }
    fn native_size(&self) -> Option<(u32, u32)> {
        self.loaded.then_some(self.native)
    }
    fn buffered(&self) -> Vec<(f64, f64)> {
        if self.loaded {
            vec![(0.0, self.duration)]
        } else {
            Vec::new()
        }
    }
    fn seekable(&self) -> Vec<(f64, f64)> {
        self.buffered()
    }
    fn error(&self) -> Option<MediaError> {
        None
    }

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
        let progress = if self.duration > 0.0 {
            self.time / self.duration
        } else {
            0.0
        };
        let bar = (progress * w as f64) as u32;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let lit = x.abs_diff(bar) < 4;
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
            gpu: None,
        })
    }

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        for event in std::mem::take(&mut self.events) {
            sink(event);
        }
    }
}
