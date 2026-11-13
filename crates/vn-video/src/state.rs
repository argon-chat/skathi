//! The portable core of `<video>`: everything that is not the platform.
//!
//! [`VideoState`] is the object the host keeps per node, ticked once per frame; it owns a
//! [`VideoBackend`] and adds what a backend must not own:
//!
//! 1. Idempotent transport: the element pushes every prop every frame, and this layer turns that
//!    into edges.
//! 2. The spec's `timeupdate` cadence (no more often than every 250 ms): the platform's own
//!    `timeupdate` is dropped and replaced by one on a 250 ms clock, plus one at every seek and at
//!    `ended`.
//! 3. Atlas hygiene: gpui keys atlas tiles by `RenderImage::id` and every frame mints a new id, so
//!    retired images must be handed back with `Window::drop_image` (two-generation retire, as in `vn-lottie`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::RenderImage;

use crate::backend::{
    CanPlay, FrameRequest, GpuFrame, MediaError, MediaEvent, MediaSource, NetworkState, ReadyState,
    VideoBackend, VideoFrame,
};

/// The spec's floor for `timeupdate`: no more often than every 250 ms.
const TIMEUPDATE_PERIOD: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct Advance {
    pub events: Vec<MediaEvent>,
    pub new_frame: bool,
}

struct Cached {
    image: Arc<RenderImage>,
    size: (u32, u32),
    pts: f64,
}

/// The transport values last pushed into the backend, so an unchanged prop costs a comparison, not
/// a platform call or a re-raised `volumechange` / `ratechange`.
struct Pushed {
    volume: f64,
    muted: bool,
    rate: f64,
    looping: bool,
    playing: bool,
}

impl Default for Pushed {
    fn default() -> Self {
        Self { volume: 1.0, muted: false, rate: 1.0, looping: false, playing: false }
    }
}

pub struct VideoState {
    backend: Box<dyn VideoBackend>,
    pushed: Pushed,
    source: Option<MediaSource>,
    /// A load that failed outright, kept so the failure costs one advisory, not one per frame.
    load_error: Option<MediaError>,

    cached: Option<Cached>,
    /// The frame before the current one, retired a generation late so the scene that painted it has
    /// been presented. Almost always `None` on the zero-copy path (stable image handle per size).
    previous: Option<Arc<RenderImage>>,
    retired: Vec<Arc<RenderImage>>,
    /// The GPU texture the last [`Self::advance`] produced, awaiting [`Self::take_gpu_upload`].
    pending_gpu: Option<(Arc<RenderImage>, GpuFrame)>,

    last_timeupdate: Option<Instant>,
    /// `currentTime` at the last emitted `timeupdate`, so a paused video does not tick forever.
    last_reported_time: f64,
    metadata: bool,
    suspended: bool,
}

impl VideoState {
    pub fn new(backend: Box<dyn VideoBackend>) -> Self {
        Self {
            backend,
            pushed: Pushed::default(),
            source: None,
            load_error: None,
            cached: None,
            previous: None,
            retired: Vec::new(),
            pending_gpu: None,
            last_timeupdate: None,
            last_reported_time: f64::NAN,
            metadata: false,
            suspended: false,
        }
    }

    /// Loads `source` if it is not the one already loaded. `Err` only for a failure the platform
    /// reported synchronously; an asynchronous one arrives as [`MediaEvent::Error`] plus [`Self::error`].
    pub fn set_source(&mut self, source: MediaSource) -> Result<(), MediaError> {
        if self.source.as_ref() == Some(&source) {
            return self.load_error.clone().map_or(Ok(()), Err);
        }
        self.source = Some(source.clone());
        self.load_error = None;
        self.metadata = false;
        self.last_reported_time = f64::NAN;
        self.retire_all_frames();
        match self.backend.load(&source) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.load_error = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn source(&self) -> Option<&MediaSource> {
        self.source.as_ref()
    }

    /// `play()` / `pause()` as the state the element wants, edge-triggered against what was last
    /// pushed so `:autoplay="isPlaying"` is a transport control rather than a per-frame restart.
    pub fn set_playing(&mut self, playing: bool) {
        if self.pushed.playing == playing {
            return;
        }
        self.pushed.playing = playing;
        if playing {
            self.suspended = false;
            self.backend.play();
        } else {
            self.backend.pause();
        }
    }

    /// True when playback is requested, whether or not the platform has started; not `!paused`.
    pub fn wants_playing(&self) -> bool {
        self.pushed.playing
    }

    pub fn play(&mut self) {
        self.set_playing(true);
    }

    pub fn pause(&mut self) {
        self.set_playing(false);
    }

    /// `currentTime = seconds`. Always forwarded, not deduped: seeking to the current position
    /// restarts a finished video in the DOM.
    pub fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() || seconds < 0.0 {
            return;
        }
        self.backend.seek(seconds);
    }

    pub fn set_volume(&mut self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        if (self.pushed.volume - volume).abs() < f64::EPSILON {
            return;
        }
        self.pushed.volume = volume;
        self.backend.set_volume(volume);
    }

    pub fn set_muted(&mut self, muted: bool) {
        if self.pushed.muted == muted {
            return;
        }
        self.pushed.muted = muted;
        self.backend.set_muted(muted);
    }

    pub fn set_rate(&mut self, rate: f64) {
        // A zero or negative rate is a pause request in disguise; browsers reject it.
        if !(rate.is_finite() && rate > 0.0) {
            return;
        }
        if (self.pushed.rate - rate).abs() < f64::EPSILON {
            return;
        }
        self.pushed.rate = rate;
        self.backend.set_rate(rate);
    }

    pub fn set_looping(&mut self, looping: bool) {
        if self.pushed.looping == looping {
            return;
        }
        self.pushed.looping = looping;
        self.backend.set_looping(looping);
    }

    /// Releases the decode pipeline while keeping the element (a parked `<KeepAlive>` page). Unlike
    /// `<lottie>`, a video costs ~28 MiB of RAM and ~68 MiB of VRAM while parked.
    pub fn suspend(&mut self) {
        if self.suspended {
            return;
        }
        self.suspended = true;
        self.backend.suspend();
    }

    /// Puts back what [`Self::suspend`] released and restores the transport. Position and source
    /// survive a suspend (`VideoBackend::suspend`), so this is a resume, not a reload.
    pub fn resume(&mut self) {
        if !self.suspended {
            return;
        }
        self.suspended = false;
        self.backend.resume();
        if self.pushed.playing {
            self.backend.play();
        }
    }

    pub fn suspended(&self) -> bool {
        self.suspended
    }

    /// Is this element's sound playing out now (`VideoBackend::audible`)? A suspended one has none.
    pub fn audible(&self) -> bool {
        !self.suspended && self.backend.audible()
    }

    pub fn current_time(&self) -> f64 {
        self.backend.current_time()
    }
    pub fn duration(&self) -> f64 {
        self.backend.duration()
    }
    pub fn paused(&self) -> bool {
        self.backend.paused()
    }
    pub fn ended(&self) -> bool {
        self.backend.ended()
    }
    pub fn seeking(&self) -> bool {
        self.backend.seeking()
    }
    pub fn ready_state(&self) -> ReadyState {
        self.backend.ready_state()
    }
    pub fn network_state(&self) -> NetworkState {
        self.backend.network_state()
    }
    pub fn volume(&self) -> f64 {
        self.pushed.volume
    }
    pub fn muted(&self) -> bool {
        self.pushed.muted
    }
    pub fn playback_rate(&self) -> f64 {
        self.pushed.rate
    }
    pub fn looping(&self) -> bool {
        self.pushed.looping
    }
    pub fn buffered(&self) -> Vec<(f64, f64)> {
        self.backend.buffered()
    }
    pub fn seekable(&self) -> Vec<(f64, f64)> {
        self.backend.seekable()
    }

    pub fn error(&self) -> Option<MediaError> {
        self.load_error.clone().or_else(|| self.backend.error())
    }

    pub fn can_play_type(&self, mime_type: &str) -> CanPlay {
        self.backend.can_play_type(mime_type)
    }

    /// `videoWidth` / `videoHeight`; the box falls back to this when CSS declares no size.
    pub fn native_size(&self) -> Option<(u32, u32)> {
        self.backend.native_size()
    }

    /// True while this element still needs the frame pump running.
    pub fn needs_frames(&self) -> bool {
        if self.suspended {
            return false;
        }
        // Not `wants_playing`: a seek on a paused video, a load's first frame and a load in progress
        // all need repaints, or the events that end them are collected and never drawn.
        !self.backend.paused()
            || self.backend.seeking()
            || self.backend.network_state() == NetworkState::Loading
            || (self.backend.ready_state() >= ReadyState::HaveMetadata && self.cached.is_none())
    }

    /// One frame's work: pull a frame at `request`'s size, collect the platform's events, add the
    /// throttled `timeupdate`. Call from the frame pump on the UI thread BEFORE the draw that builds
    /// the element.
    pub fn advance(&mut self, now: Instant, request: &FrameRequest) -> Advance {
        let mut out = Advance::default();
        if self.suspended {
            return out;
        }

        self.backend.drain_events(&mut |event| {
            // The platform's own cadence is replaced, not forwarded (MF raises TIMEUPDATE on its decoder cadence).
            if event != MediaEvent::TimeUpdate {
                out.events.push(event);
            }
        });

        for event in &out.events {
            match event {
                MediaEvent::LoadedMetadata => self.metadata = true,
                MediaEvent::Emptied => self.metadata = false,
                _ => {}
            }
        }

        if let Some(frame) = self.backend.present(request) {
            self.accept_frame(frame);
            out.new_frame = true;
        }

        // `timeupdate`: on the 250 ms clock while the position moves, and at every boundary (a seek
        // that landed, the end), so a bound UI is never left showing a stale position.
        let time = self.backend.current_time();
        let boundary = out
            .events
            .iter()
            .any(|e| matches!(e, MediaEvent::Seeked | MediaEvent::Ended | MediaEvent::LoadedData));
        let due = self
            .last_timeupdate
            .is_none_or(|last| now.saturating_duration_since(last) >= TIMEUPDATE_PERIOD);
        let moved = self.last_reported_time.is_nan() || self.last_reported_time != time;
        if boundary || (due && moved) {
            self.last_timeupdate = Some(now);
            self.last_reported_time = time;
            out.events.push(MediaEvent::TimeUpdate);
        }

        out
    }

    pub fn frame(&self) -> Option<Arc<RenderImage>> {
        self.cached.as_ref().map(|c| c.image.clone())
    }

    pub fn frame_info(&self) -> Option<((u32, u32), f64)> {
        self.cached.as_ref().map(|c| (c.size, c.pts))
    }

    fn accept_frame(&mut self, frame: VideoFrame) {
        if let Some(gpu) = frame.gpu {
            self.pending_gpu = Some((frame.image.clone(), gpu));
        }
        // The zero-copy path returns the SAME image every frame (one atlas tile, updated in place), so
        // retire only on identity change: `drop_image` on the tile being painted would blank the video.
        let same_image = self
            .cached
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(&c.image, &frame.image));
        if !same_image {
            if let Some(previous) = self.previous.take() {
                self.retired.push(previous);
            }
            self.previous = self.cached.take().map(|c| c.image);
        }
        self.cached = Some(Cached { image: frame.image, size: frame.size, pts: frame.pts });
    }

    /// The GPU texture this tick produced and the image whose atlas tile it must be blitted into,
    /// if the backend took the zero-copy path. The host must drain this every tick and pass it to
    /// `Window::upload_image_from_gpu` before the draw; the texture is reused by the next `present`.
    #[must_use = "the video paints transparent unless this reaches Window::upload_image_from_gpu"]
    pub fn take_gpu_upload(&mut self) -> Option<(Arc<RenderImage>, GpuFrame)> {
        self.pending_gpu.take()
    }

    /// Tells the backend the host could not upload the texture, so it fills images with real bytes.
    pub fn gpu_upload_failed(&mut self) {
        self.pending_gpu = None;
        self.backend.gpu_upload_failed();
    }

    fn retire_all_frames(&mut self) {
        self.retired.extend(self.previous.take());
        self.retired.extend(self.cached.take().map(|c| c.image));
    }

    /// Frames no longer painted whose atlas tiles the caller must release with `Window::drop_image`.
    /// The host must drain this every frame, or a 60 fps video leaks one tile per frame.
    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_retired(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.retired)
    }

    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_all_images(&mut self) -> Vec<Arc<RenderImage>> {
        self.retire_all_frames();
        std::mem::take(&mut self.retired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::StubBackend;

    fn state() -> VideoState {
        VideoState::new(Box::new(StubBackend::new(4.0, (320, 180))))
    }

    /// The element pushes every prop every frame, so a setter that is not idempotent causes an event
    /// storm; stated as "n pushes cost exactly what 1 push costs".
    #[test]
    fn repeated_prop_pushes_do_not_retrigger_the_backend() {
        fn events_after(rounds: usize) -> Vec<MediaEvent> {
            let mut s = state();
            s.set_source(MediaSource::Path("synthetic".into())).unwrap();
            for _ in 0..rounds {
                s.set_volume(0.5);
                s.set_muted(true);
                s.set_rate(2.0);
                s.set_looping(true);
                s.set_playing(true);
            }
            let mut events = Vec::new();
            s.backend.drain_events(&mut |e| events.push(e));
            events
        }
        assert_eq!(events_after(10), events_after(1));
        let once = events_after(1);
        assert!(once.contains(&MediaEvent::Play));
        assert!(once.contains(&MediaEvent::RateChange));
        assert!(once.contains(&MediaEvent::VolumeChange));
    }

    #[test]
    fn timeupdate_runs_at_the_spec_cadence_not_the_frame_rate() {
        let mut s = state();
        s.set_source(MediaSource::Path("synthetic".into())).unwrap();
        s.set_playing(true);
        let request = FrameRequest { max_size: (320, 180) };
        let start = Instant::now();
        let mut ticks = 0;
        for i in 0..60u32 {
            let now = start + Duration::from_millis(16 * i as u64);
            let advance = s.advance(now, &request);
            ticks += advance
                .events
                .iter()
                .filter(|e| **e == MediaEvent::TimeUpdate)
                .count();
        }
        assert!((3..=6).contains(&ticks), "timeupdate fired {ticks} times in ~1s");
    }

    #[test]
    fn every_retired_frame_is_handed_back() {
        let mut s = state();
        s.set_source(MediaSource::Path("synthetic".into())).unwrap();
        s.set_playing(true);
        let request = FrameRequest { max_size: (320, 180) };
        let start = Instant::now();
        let mut produced = 0;
        let mut retired = 0;
        for i in 0..30u32 {
            let now = start + Duration::from_millis(16 * i as u64);
            if s.advance(now, &request).new_frame {
                produced += 1;
            }
            retired += s.take_retired().len();
        }
        retired += s.take_all_images().len();
        assert!(produced > 0, "the stub produced no frames at all");
        assert_eq!(produced, retired, "{produced} frames produced, {retired} released");
    }

    /// A parked `<KeepAlive>` page must stop costing a decoder, and come back at its position and
    /// transport state when reactivated.
    #[test]
    fn parking_releases_the_decoder_and_unparking_restores_it() {
        let mut s = state();
        s.set_source(MediaSource::Path("synthetic".into())).unwrap();
        s.set_playing(true);
        let request = FrameRequest { max_size: (320, 180) };
        let start = Instant::now();
        for i in 0..30u32 {
            s.advance(start + Duration::from_millis(16 * i as u64), &request);
        }
        let left_at = s.current_time();
        assert!(left_at > 0.0, "the clock did not move, so the test proves nothing");
        assert!(s.needs_frames());

        s.suspend();
        assert!(!s.needs_frames(), "a parked element must stop asking for frames");
        assert!(s.suspended());

        s.resume();
        assert!(!s.suspended());
        assert!(s.wants_playing(), "it was playing when parked");
        s.advance(start + Duration::from_secs(1), &request);
        assert!(
            (s.current_time() - left_at).abs() < 0.1,
            "resumed at {} instead of {left_at}",
            s.current_time()
        );
    }
}
