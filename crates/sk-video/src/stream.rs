//! Bounded native video handoff. Pixels never cross the JavaScript bridge.
//! Each producer replaces one latest frame; each attached element has its own
//! playback cursor, so a preview and a pop-out cannot consume each other's frames.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Instant;

use crate::backend::*;
use gpui::RenderImage;
use image::{Frame, RgbaImage};

pub const PREFIX: &str = "sk-stream://";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static STREAMS: OnceLock<Mutex<HashMap<String, Weak<StreamSlot>>>> = OnceLock::new();

fn streams() -> &'static Mutex<HashMap<String, Weak<StreamSlot>>> {
    STREAMS.get_or_init(Mutex::default)
}

struct Pixels {
    size: (u32, u32),
    bytes: Vec<u8>,
    generation: u64,
    pts: f64,
}

#[derive(Default)]
struct Latest {
    frame: Option<Arc<Pixels>>,
    generation: u64,
    ended: bool,
}

pub struct StreamSlot {
    latest: Mutex<Latest>,
    started: Instant,
}

impl StreamSlot {
    /// Packed, top-down, straight BGRA. Malformed frames are ignored, never
    /// handed to image constructors or allowed to replace the last good frame.
    pub fn publish(&self, width: u32, height: u32, bgra: Vec<u8>) {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || expected != Some(bgra.len())
        {
            return;
        }
        let mut latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        if latest.ended {
            return;
        }
        latest.generation += 1;
        latest.frame = Some(Arc::new(Pixels {
            size: (width, height),
            bytes: bgra,
            generation: latest.generation,
            pts: self.started.elapsed().as_secs_f64(),
        }));
    }

    pub fn end(&self) {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).ended = true;
    }
}

pub fn create() -> (String, Arc<StreamSlot>) {
    let url = format!("{PREFIX}{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let slot = Arc::new(StreamSlot {
        latest: Mutex::default(),
        started: Instant::now(),
    });
    let mut registry = streams().lock().unwrap_or_else(|e| e.into_inner());
    registry.retain(|_, slot| slot.strong_count() != 0);
    registry.insert(url.clone(), Arc::downgrade(&slot));
    (url, slot)
}

pub fn unregister(url: &str) {
    if let Some(slot) = streams()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(url)
        .and_then(|s| s.upgrade())
    {
        slot.end();
    }
}

pub fn backend(url: &str) -> Result<Box<dyn VideoBackend>, MediaError> {
    let slot = streams()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(url)
        .and_then(Weak::upgrade)
        .ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                "the native video track has stopped",
            )
        })?;
    Ok(Box::new(StreamBackend {
        slot,
        paused: true,
        loaded: false,
        metadata: false,
        ended_event: false,
        generation: 0,
        size: (0, 0),
        time: 0.0,
        events: Vec::new(),
    }))
}

struct StreamBackend {
    slot: Arc<StreamSlot>,
    paused: bool,
    loaded: bool,
    metadata: bool,
    ended_event: bool,
    generation: u64,
    size: (u32, u32),
    time: f64,
    events: Vec<MediaEvent>,
}

impl VideoBackend for StreamBackend {
    fn load(&mut self, _: &MediaSource) -> Result<(), MediaError> {
        self.loaded = true;
        self.events.push(MediaEvent::LoadStart);
        Ok(())
    }
    fn play(&mut self) {
        if self.paused {
            self.paused = false;
            self.events.push(MediaEvent::Play);
            if self.metadata {
                self.events.push(MediaEvent::Playing);
            }
        }
    }
    fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.events.push(MediaEvent::Pause);
        }
    }
    fn seek(&mut self, _: f64) {} // Live tracks have no seekable range.
    fn set_volume(&mut self, _: f64) {}
    fn set_muted(&mut self, _: bool) {}
    fn set_rate(&mut self, _: f64) {}
    fn set_looping(&mut self, _: bool) {}
    fn current_time(&self) -> f64 {
        self.time
    }
    fn duration(&self) -> f64 {
        f64::INFINITY
    }
    fn paused(&self) -> bool {
        self.paused || self.ended()
    }
    fn ended(&self) -> bool {
        self.slot
            .latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ended
    }
    fn seeking(&self) -> bool {
        false
    }
    fn ready_state(&self) -> ReadyState {
        let latest = self.slot.latest.lock().unwrap_or_else(|e| e.into_inner());
        if latest.frame.is_some() {
            ReadyState::HaveEnoughData
        } else {
            ReadyState::HaveNothing
        }
    }
    fn network_state(&self) -> NetworkState {
        if !self.loaded {
            NetworkState::Empty
        } else if self.ready_state() == ReadyState::HaveNothing && !self.ended() {
            NetworkState::Loading
        } else {
            NetworkState::Idle
        }
    }
    fn native_size(&self) -> Option<(u32, u32)> {
        self.slot
            .latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .frame
            .as_ref()
            .map(|f| f.size)
    }
    fn buffered(&self) -> Vec<(f64, f64)> {
        Vec::new()
    }
    fn seekable(&self) -> Vec<(f64, f64)> {
        Vec::new()
    }
    fn error(&self) -> Option<MediaError> {
        None
    }
    fn can_play_type(&self, _: &str) -> CanPlay {
        CanPlay::No
    }
    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame> {
        if !self.loaded || (self.paused && self.generation != 0) {
            return None;
        }
        let frame = self
            .slot
            .latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .frame
            .clone()?;
        let size = crate::fit_within(frame.size, request.max_size);
        if size.0 == 0 || size.1 == 0 || (frame.generation == self.generation && size == self.size)
        {
            return None;
        }
        let bytes = if size == frame.size {
            frame.bytes.clone()
        } else {
            sk_resample::downscale(
                &frame.bytes,
                frame.size.0,
                frame.size.1,
                size.0,
                size.1,
                false,
            )
        };
        self.generation = frame.generation;
        self.size = size;
        self.time = frame.pts;
        Some(VideoFrame {
            image: Arc::new(
                RenderImage::new(vec![Frame::new(RgbaImage::from_raw(
                    size.0, size.1, bytes,
                )?)])
                .dynamic(),
            ),
            pts: frame.pts,
            size,
            gpu: None,
        })
    }
    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        if !self.metadata && self.native_size().is_some() {
            self.metadata = true;
            self.events.extend([
                MediaEvent::DurationChange,
                MediaEvent::LoadedMetadata,
                MediaEvent::LoadedData,
                MediaEvent::CanPlay,
            ]);
            if !self.paused {
                self.events.push(MediaEvent::Playing);
            }
        }
        if self.ended() && !self.ended_event {
            self.ended_event = true;
            self.events.push(MediaEvent::Ended);
        }
        for event in self.events.drain(..) {
            sink(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn consumer(url: &str) -> Box<dyn VideoBackend> {
        let mut b = backend(url).unwrap();
        b.load(&MediaSource::Uri(url.into())).unwrap();
        b.play();
        b
    }
    #[test]
    fn latest_frame_is_bounded_and_each_attachment_reads_independently() {
        let (url, slot) = create();
        let mut a = consumer(&url);
        let mut b = consumer(&url);
        let request = FrameRequest { max_size: (2, 2) };
        assert!(a.present(&request).is_none());
        slot.publish(4, 2, vec![255; 32]);
        slot.publish(4, 2, vec![127; 32]);
        assert_eq!(a.present(&request).unwrap().size, (2, 1));
        assert!(a.present(&request).is_none());
        assert!(b.present(&request).is_some());
        slot.publish(4, 2, vec![0; 1]);
        assert!(a.present(&request).is_none());
        a.pause();
        slot.publish(4, 2, vec![0; 32]);
        assert!(a.present(&request).is_none());
        a.play();
        assert!(a.present(&request).is_some());
        unregister(&url);
        assert!(a.ended());
        assert!(backend(&url).is_err());
    }
    #[test]
    fn end_and_metadata_events_are_emitted_once() {
        let (url, slot) = create();
        let mut b = consumer(&url);
        slot.publish(1, 1, vec![255; 4]);
        slot.end();
        let mut events = Vec::new();
        b.drain_events(&mut |e| events.push(e));
        assert!(events.contains(&MediaEvent::LoadedMetadata));
        assert!(events.contains(&MediaEvent::Ended));
        b.drain_events(&mut |_| panic!("duplicate event"));
        unregister(&url);
    }
}
