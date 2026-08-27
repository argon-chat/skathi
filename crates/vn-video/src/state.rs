//! The portable core: everything about a `<video>` that is not the platform.
//!
//! [`VideoState`] is to `<video>` what `LottieState` is to `<lottie>` — the
//! object the host keeps in a side map keyed by node id, ticked once per frame,
//! and the only thing that survives the per-frame element rebuild. It owns a
//! [`VideoBackend`] and adds the three things a backend must not own:
//!
//! 1. **Idempotent transport.** The element layer pushes every prop every
//!    frame; this layer turns that into edges.
//! 2. **The event cadence the spec asks for.** `timeupdate` is specified as
//!    "about 4 Hz to 66 Hz, and no more often than every 250 ms" — a per-frame
//!    event would be 165 Hz on this machine and would drag a JS handler onto
//!    every frame. The platform's own `timeupdate` is dropped and replaced by
//!    one on a 250 ms clock, plus one at every seek and at `ended`, so the
//!    stream is the same everywhere.
//! 3. **Atlas hygiene.** gpui keys atlas tiles by `RenderImage::id` and a new
//!    frame mints a new id, so a 60 fps video leaks one tile per frame unless
//!    the retired images are handed back with `Window::drop_image`. The
//!    two-generation retire below is `vn-lottie`'s, for the same reason.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::RenderImage;

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaEvent, MediaSource, NetworkState, ReadyState,
    VideoBackend, VideoFrame,
};

/// The spec's floor for `timeupdate`: "no more often than every 250 ms".
/// Browsers sit at exactly this; so do we, so a `@timeupdate` readout ticks at
/// the same 4 Hz it does in Chrome.
const TIMEUPDATE_PERIOD: Duration = Duration::from_millis(250);

/// What one [`VideoState::advance`] produced.
#[derive(Default)]
pub struct Advance {
    /// Events to dispatch, in order.
    pub events: Vec<MediaEvent>,
    /// A new frame was transferred; the window must repaint.
    pub new_frame: bool,
}

struct Cached {
    image: Arc<RenderImage>,
    size: (u32, u32),
    pts: f64,
}

/// The transport values last pushed into the backend, so an unchanged prop
/// costs a comparison rather than a platform call (and, more importantly, does
/// not re-raise `volumechange` / `ratechange` sixty times a second).
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
    /// `None` until `load` has been called at all.
    source: Option<MediaSource>,
    /// A load that failed outright (a `src` the platform will not open). Kept
    /// so the failure costs one advisory, not one per frame.
    load_error: Option<MediaError>,

    cached: Option<Cached>,
    /// The frame before the current one. Retired a generation late: the scene
    /// that painted it has been presented by then, so its atlas tile is safe to
    /// release.
    previous: Option<Arc<RenderImage>>,
    retired: Vec<Arc<RenderImage>>,

    last_timeupdate: Option<Instant>,
    /// `currentTime` at the last emitted `timeupdate`, so a paused video does
    /// not tick forever.
    last_reported_time: f64,
    /// Has `loadedmetadata` been seen? Gates the intrinsic size.
    metadata: bool,
    suspended: bool,
}

impl VideoState {
    /// Wraps a backend. Nothing is loaded yet — call [`VideoState::set_source`].
    pub fn new(backend: Box<dyn VideoBackend>) -> Self {
        Self {
            backend,
            pushed: Pushed::default(),
            source: None,
            load_error: None,
            cached: None,
            previous: None,
            retired: Vec::new(),
            last_timeupdate: None,
            last_reported_time: f64::NAN,
            metadata: false,
            suspended: false,
        }
    }

    // -- source --------------------------------------------------------------

    /// Loads `source` if it is not the one already loaded. Returns `Err` only
    /// for a failure the platform reported synchronously; an asynchronous
    /// failure arrives later as [`MediaEvent::Error`] plus [`Self::error`].
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

    // -- transport (idempotent; see the module docs) --------------------------

    /// `play()` / `pause()`, expressed as the state the element wants to be in.
    /// Edge-triggered against what was last pushed, so binding
    /// `:autoplay="isPlaying"` is a transport control rather than a per-frame
    /// restart.
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

    /// `true` when the element has asked for playback, whether or not the
    /// platform has managed to start yet. This is *not* `!paused` — that one
    /// reads the platform (see [`Self::paused`]).
    pub fn wants_playing(&self) -> bool {
        self.pushed.playing
    }

    pub fn play(&mut self) {
        self.set_playing(true);
    }

    pub fn pause(&mut self) {
        self.set_playing(false);
    }

    /// `currentTime = seconds`. Always forwarded — a seek to the position we
    /// are already at is a legal, meaningful operation in the DOM (it restarts
    /// a finished video), so this one is deliberately not deduped.
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
        // The spec allows any positive rate; a zero or negative one is a pause
        // request in disguise and every browser rejects it.
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

    /// Releases the decode pipeline while keeping the element. What a parked
    /// `<KeepAlive>` page must do: a browser does not keep decoding a video in
    /// a detached subtree, and neither does this.
    ///
    /// Deliberately different from `<lottie>`, whose clock keeps running while
    /// parked — an animation with no soundtrack costs arithmetic, a video costs
    /// ~28 MiB of RAM and ~68 MiB of VRAM.
    pub fn suspend(&mut self) {
        if self.suspended {
            return;
        }
        self.suspended = true;
        self.backend.suspend();
    }

    /// Puts back what [`Self::suspend`] released and restores the transport.
    ///
    /// The position and the loaded source SURVIVE a suspend — the backend
    /// contract requires it (`VideoBackend::suspend`) — so this is a resume,
    /// not a reload. The first version did reload, and that leaked ~5 MiB per
    /// park/unpark cycle and killed the process after about ten of them.
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

    // -- the property surface ------------------------------------------------

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

    /// The element's error, from either half: a `src` the platform refused up
    /// front, or a failure it reported later.
    pub fn error(&self) -> Option<MediaError> {
        self.load_error.clone().or_else(|| self.backend.error())
    }

    pub fn can_play_type(&self, mime_type: &str) -> CanPlay {
        self.backend.can_play_type(mime_type)
    }

    /// `videoWidth` / `videoHeight` — the box falls back to this when CSS
    /// declares no size, which is the `<img>` rule.
    pub fn native_size(&self) -> Option<(u32, u32)> {
        self.backend.native_size()
    }

    /// True while this element still needs the frame pump running. Folded into
    /// the host's `animating()` OR-chain.
    pub fn needs_frames(&self) -> bool {
        if self.suspended {
            return false;
        }
        // Not `wants_playing`: a seek on a paused video still has to produce
        // the frame at the new position, a load still has to produce its first
        // frame, and a load still IN PROGRESS has to keep the host repainting
        // or the events that end it are collected and never drawn.
        !self.backend.paused()
            || self.backend.seeking()
            || self.backend.network_state() == NetworkState::Loading
            || (self.backend.ready_state() >= ReadyState::HaveMetadata && self.cached.is_none())
    }

    // -- the tick ------------------------------------------------------------

    /// One frame's work: pull a frame at `request`'s size, collect the events
    /// the platform raised, and add the throttled `timeupdate`.
    ///
    /// Call from the host's frame pump, on the UI thread, BEFORE the draw that
    /// builds the element — so the frame a node paints is the one this tick
    /// produced.
    pub fn advance(&mut self, now: Instant, request: &FrameRequest) -> Advance {
        let mut out = Advance::default();
        if self.suspended {
            return out;
        }

        self.backend.drain_events(&mut |event| {
            // The platform's own cadence is replaced, not forwarded (module
            // docs) — MF raises TIMEUPDATE on its decoder cadence.
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

        // `timeupdate`: on the 250 ms clock while the position is moving, and
        // once more at every boundary the spec calls out (a seek that landed,
        // the end of the media), so a UI bound to it is never left showing a
        // stale position.
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

    // -- frames --------------------------------------------------------------

    /// The frame to paint this build, if any. Cheap — the tick already produced
    /// it; this is the cached handle.
    pub fn frame(&self) -> Option<Arc<RenderImage>> {
        self.cached.as_ref().map(|c| c.image.clone())
    }

    /// The size of the cached frame in physical pixels, and its PTS.
    pub fn frame_info(&self) -> Option<((u32, u32), f64)> {
        self.cached.as_ref().map(|c| (c.size, c.pts))
    }

    fn accept_frame(&mut self, frame: VideoFrame) {
        if let Some(previous) = self.previous.take() {
            self.retired.push(previous);
        }
        self.previous = self.cached.take().map(|c| c.image);
        self.cached = Some(Cached { image: frame.image, size: frame.size, pts: frame.pts });
    }

    fn retire_all_frames(&mut self) {
        self.retired.extend(self.previous.take());
        self.retired.extend(self.cached.take().map(|c| c.image));
    }

    /// Frames that are no longer painted and whose atlas tiles the caller must
    /// release with `Window::drop_image`.
    ///
    /// **The host must drain this every frame.** gpui keys atlas tiles by
    /// `RenderImage::id` and every decoded frame mints a fresh id, so a 60 fps
    /// video leaks one tile per frame if these are dropped on the floor — the
    /// exact leak `crates/vn-lottie/CONTRACT.md` §3 documents.
    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_retired(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.retired)
    }

    /// Every image still held, for teardown when the node leaves the tree.
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

    /// The element layer pushes every prop every frame — that is how the tree
    /// works. A setter that is not idempotent turns that into an event storm:
    /// 165 `volumechange`s a second, each one a JS dispatch.
    ///
    /// Stated as "n pushes cost exactly what 1 push costs", which is the
    /// property that actually matters and does not have to be re-derived every
    /// time the event set changes.
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
        // …and one round really does produce the edges, so the assertion above
        // is not comparing two empty vectors.
        let once = events_after(1);
        assert!(once.contains(&MediaEvent::Play));
        assert!(once.contains(&MediaEvent::RateChange));
        assert!(once.contains(&MediaEvent::VolumeChange));
    }

    /// The spec's floor: no more often than every 250 ms.
    #[test]
    fn timeupdate_runs_at_the_spec_cadence_not_the_frame_rate() {
        let mut s = state();
        s.set_source(MediaSource::Path("synthetic".into())).unwrap();
        s.set_playing(true);
        let request = FrameRequest { max_size: (320, 180) };
        let start = Instant::now();
        let mut ticks = 0;
        // 60 frames at a simulated 16 ms apart = ~1 s of playback.
        for i in 0..60u32 {
            let now = start + Duration::from_millis(16 * i as u64);
            let advance = s.advance(now, &request);
            ticks += advance
                .events
                .iter()
                .filter(|e| **e == MediaEvent::TimeUpdate)
                .count();
        }
        // ~1 s at 4 Hz. Generous bounds: the point is that it is not 60.
        assert!((3..=6).contains(&ticks), "timeupdate fired {ticks} times in ~1s");
    }

    /// Every frame mints a new `RenderImage`; every one but the two most
    /// recent has to come back out for `Window::drop_image`.
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

    /// A parked `<KeepAlive>` page must stop costing a decoder — and must come
    /// BACK when it is reactivated, at the position it was left at and in the
    /// transport state it was left in. (It did not: `suspend` tore the pipeline
    /// down and nothing ever put it back, so returning to the page showed
    /// `--:--` and a dead element.)
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
        // The position is restored at `loadedmetadata`, which the reload emits.
        s.advance(start + Duration::from_secs(1), &request);
        assert!(
            (s.current_time() - left_at).abs() < 0.1,
            "resumed at {} instead of {left_at}",
            s.current_time()
        );
    }
}
