//! The macOS backend: `AVPlayer` with an `AVPlayerItemVideoOutput`.
//!
//! AVPlayer decodes, renders the audio to the system output and owns the master clock. Frames are
//! pulled from the video output at the item time for the current host time, so the picture follows
//! the audio. The output yields BGRA `CVPixelBuffer`s, converted and scaled by VideoToolbox, that
//! are copied into a `RenderImage`: gpui's Metal atlas has no `upload_from_gpu` to hand a texture to.
//!
//! AVPlayer is main-thread-only and publishes its state changes through the main queue, so the
//! backend lives on the UI (main) thread and polls that state once per tick, turning changes into
//! events. The callbacks that arrive elsewhere (end of item, a failure while playing, seek
//! completion) touch nothing but [`Shared`].

mod frame;
mod loader;

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{MainThreadMarker, Message};
use objc2_av_foundation::{
    AVAsset, AVMediaTypeAudio, AVMediaTypeVideo, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem,
    AVPlayerItemDidPlayToEndTimeNotification, AVPlayerItemFailedToPlayToEndTimeErrorKey,
    AVPlayerItemFailedToPlayToEndTimeNotification, AVPlayerItemStatus, AVPlayerTimeControlStatus,
    AVPlayerWaitingToMinimizeStallsReason, AVURLAsset, NSValueAVFoundationExtensions,
};
use objc2_core_media::{kCMTimeZero, CMTime, CMTimeFlags, CMTimeRange};
use objc2_foundation::{
    NSArray, NSError, NSNotification, NSNotificationCenter, NSObjectProtocol, NSString,
    NSUnderlyingErrorKey, NSValue, NSURL,
};

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaErrorCode, MediaEvent, MediaSource, NetworkState,
    ReadyState, VideoBackend, VideoFrame,
};
use crate::trace;

/// What the platform's callbacks leave for the next tick.
#[derive(Default)]
struct Shared {
    events: Mutex<Vec<MediaEvent>>,
    seek_issued: AtomicU64,
    seek_done: AtomicU64,
    reached_end: AtomicBool,
    failed: Mutex<Option<Failure>>,
}

impl Shared {
    fn push(&self, event: MediaEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
    }
}

/// An `NSError` reduced to plain data, so it can cross threads.
#[derive(Clone, Debug)]
struct Failure {
    domain: String,
    code: isize,
    text: String,
    /// The first `NSURLErrorDomain` code in the underlying-error chain: a network failure that
    /// AVFoundation reports under its own domain.
    url_code: Option<isize>,
}

impl Failure {
    fn of(error: &NSError) -> Self {
        let mut url_code = None;
        let mut cursor: Option<Retained<NSError>> = Some(error.retain());
        for _ in 0..4 {
            let Some(e) = cursor else { break };
            if e.domain().to_string() == "NSURLErrorDomain" {
                url_code = Some(e.code());
                break;
            }
            cursor = e
                .userInfo()
                .objectForKey(unsafe { NSUnderlyingErrorKey })
                .and_then(|u| u.downcast::<NSError>().ok());
        }
        Self {
            domain: error.domain().to_string(),
            code: error.code(),
            text: error.localizedDescription().to_string(),
            url_code,
        }
    }
}

pub struct AvPlayerBackend {
    mtm: MainThreadMarker,
    player: Retained<AVPlayer>,
    item: Option<Retained<AVPlayerItem>>,
    served: Option<loader::Served>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    shared: Arc<Shared>,
    frames: frame::Frames,
    events: Vec<MediaEvent>,

    src: String,
    remote: bool,
    loaded: bool,
    /// The DOM's `paused`: true until `play()`, and again after `pause()` or the end.
    paused: bool,
    ended: bool,
    looping: bool,
    rate: f64,
    volume: f64,
    muted: bool,
    seek_target: f64,
    /// A seek asked for before the item could take one, applied at `loadedmetadata`.
    pending_seek: Option<f64>,

    reported_ready: ReadyState,
    reported_duration: f64,
    loaded_data_fired: bool,
    playing_reported: bool,
    waiting_reported: bool,
    error: Option<MediaError>,
    has_audio: Option<bool>,
    has_video: Option<bool>,
}

impl AvPlayerBackend {
    pub fn new() -> Result<Self, MediaError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::Decode,
                "the macOS <video> backend was created off the main thread",
            )
            .with_advisory(
                "AVPlayer is main-thread-only: create and tick <video> state on the thread that \
                 runs the application's main run loop.",
            )
        })?;
        let player = unsafe { AVPlayer::new(mtm) };
        // The end is handled in `poll` (pause or loop), not by AVPlayer's own end action.
        unsafe { player.setActionAtItemEnd(AVPlayerActionAtItemEnd::None) };
        Ok(Self {
            mtm,
            player,
            item: None,
            served: None,
            observers: Vec::new(),
            shared: Arc::default(),
            frames: frame::Frames::default(),
            events: Vec::new(),
            src: String::new(),
            remote: false,
            loaded: false,
            paused: true,
            ended: false,
            looping: false,
            rate: 1.0,
            volume: 1.0,
            muted: false,
            seek_target: 0.0,
            pending_seek: None,
            reported_ready: ReadyState::HaveNothing,
            reported_duration: f64::NAN,
            loaded_data_fired: false,
            playing_reported: false,
            waiting_reported: false,
            error: None,
            has_audio: None,
            has_video: None,
        })
    }

    fn unload(&mut self) {
        let center = NSNotificationCenter::defaultCenter();
        for observer in self.observers.drain(..) {
            unsafe { center.removeObserver(observer.as_ref()) };
        }
        self.frames.detach(self.item.as_deref());
        unsafe {
            self.player.pause();
            self.player.replaceCurrentItemWithPlayerItem(None);
        }
        self.item = None;
        self.served = None;
        // A completion still in flight belongs to the old item.
        let issued = self.shared.seek_issued.load(Ordering::SeqCst);
        self.shared.seek_done.store(issued, Ordering::SeqCst);
        self.shared.reached_end.store(false, Ordering::SeqCst);
        *self.shared.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.shared
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.ended = false;
        self.pending_seek = None;
        self.seek_target = 0.0;
        self.reported_ready = ReadyState::HaveNothing;
        self.reported_duration = f64::NAN;
        self.loaded_data_fired = false;
        self.playing_reported = false;
        self.waiting_reported = false;
        self.error = None;
        self.has_audio = None;
        self.has_video = None;
    }

    fn asset_for(&mut self, source: &MediaSource) -> Result<Retained<AVAsset>, MediaError> {
        self.remote = matches!(source, MediaSource::Uri(_));
        let url = match source {
            MediaSource::Path(path) => {
                self.src = path.display().to_string();
                NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
            }
            MediaSource::Uri(url) => {
                self.src = url.clone();
                NSURL::URLWithString(&NSString::from_str(url)).ok_or_else(|| {
                    MediaError::new(
                        MediaErrorCode::SrcNotSupported,
                        format!("\"{url}\" is not a well-formed URL"),
                    )
                    .with_advisory(format!(
                        "<video src=\"{url}\">: the URL could not be parsed. Percent-encode \
                         spaces and other reserved characters."
                    ))
                })?
            }
            MediaSource::Bytes { mime, data } => {
                self.src = format!("data:{mime} ({} bytes)", data.len());
                return self.serve(mime, loader::Bytes::Shared(data.clone()));
            }
            MediaSource::StaticBytes { mime, data } => {
                self.src = format!("package:{mime} ({} bytes)", data.len());
                return self.serve(mime, loader::Bytes::Mapped(data));
            }
        };
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        Ok(Retained::into_super(asset))
    }

    fn serve(&mut self, mime: &str, bytes: loader::Bytes) -> Result<Retained<AVAsset>, MediaError> {
        let served = loader::asset(mime, bytes).ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                "the in-memory media source could not be set up",
            )
        })?;
        let asset = Retained::into_super(served.asset.clone());
        self.served = Some(served);
        Ok(asset)
    }

    fn observe(&mut self, item: &AVPlayerItem) {
        let center = NSNotificationCenter::defaultCenter();
        let shared = self.shared.clone();
        let ended = RcBlock::new(move |_: NonNull<NSNotification>| {
            shared.reached_end.store(true, Ordering::SeqCst);
        });
        let shared = self.shared.clone();
        let failed = RcBlock::new(move |note: NonNull<NSNotification>| {
            // SAFETY: the notification center passes a live notification.
            let note = unsafe { note.as_ref() };
            let error = note
                .userInfo()
                .and_then(|info| unsafe {
                    info.objectForKey(AVPlayerItemFailedToPlayToEndTimeErrorKey)
                })
                .and_then(|e| e.downcast::<NSError>().ok());
            let failure = error.as_deref().map(Failure::of).unwrap_or(Failure {
                domain: String::new(),
                code: 0,
                text: "playback stopped before the end".into(),
                url_code: None,
            });
            *shared.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(failure);
        });
        let object: &AnyObject = item.as_ref();
        unsafe {
            self.observers
                .push(center.addObserverForName_object_queue_usingBlock(
                    Some(AVPlayerItemDidPlayToEndTimeNotification),
                    Some(object),
                    None,
                    &ended,
                ));
            self.observers
                .push(center.addObserverForName_object_queue_usingBlock(
                    Some(AVPlayerItemFailedToPlayToEndTimeNotification),
                    Some(object),
                    None,
                    &failed,
                ));
        }
    }

    fn start_seek(&mut self, seconds: f64) {
        let generation = self.shared.seek_issued.fetch_add(1, Ordering::SeqCst) + 1;
        self.seek_target = seconds;
        self.ended = false;
        self.events.push(MediaEvent::Seeking);
        let shared = self.shared.clone();
        let done = RcBlock::new(move |finished: Bool| {
            if finished.as_bool() && shared.seek_issued.load(Ordering::SeqCst) == generation {
                shared.seek_done.store(generation, Ordering::SeqCst);
                shared.push(MediaEvent::Seeked);
            }
        });
        unsafe {
            let time = CMTime::with_seconds(seconds, 90_000);
            self.player
                .seekToTime_toleranceBefore_toleranceAfter_completionHandler(
                    time,
                    kCMTimeZero,
                    kCMTimeZero,
                    &done,
                );
        }
    }

    fn is_seeking(&self) -> bool {
        self.shared.seek_done.load(Ordering::SeqCst)
            < self.shared.seek_issued.load(Ordering::SeqCst)
    }

    fn item_ready(&self) -> bool {
        self.item
            .as_ref()
            .is_some_and(|item| unsafe { item.status() } == AVPlayerItemStatus::ReadyToPlay)
    }

    fn track_kinds(&mut self) {
        if self.has_video.is_some() || !self.item_ready() {
            return;
        }
        let Some(item) = &self.item else { return };
        let (mut audio, mut video) = (false, false);
        unsafe {
            for track in item.tracks().iter() {
                let Some(asset_track) = track.assetTrack() else {
                    continue;
                };
                let kind = asset_track.mediaType();
                audio |= AVMediaTypeAudio.is_some_and(|a| *kind == *a);
                video |= AVMediaTypeVideo.is_some_and(|v| *kind == *v);
            }
        }
        self.has_audio = Some(audio);
        self.has_video = Some(video);
    }

    fn compute_ready_state(&self) -> ReadyState {
        if self.error.is_some() || !self.item_ready() {
            return ReadyState::HaveNothing;
        }
        let Some(item) = &self.item else {
            return ReadyState::HaveNothing;
        };
        if self.is_seeking() {
            return ReadyState::HaveMetadata;
        }
        let current = self.has_video == Some(false) || self.frames.has_frame();
        let keeping_up = unsafe { item.isPlaybackLikelyToKeepUp() || item.isPlaybackBufferFull() };
        if keeping_up {
            return ReadyState::HaveEnoughData;
        }
        let now = self.current_time();
        let ahead = self
            .buffered()
            .iter()
            .any(|&(start, end)| start <= now + 0.01 && end > now + 0.1);
        match (current, ahead) {
            (_, true) => ReadyState::HaveFutureData,
            (true, false) => ReadyState::HaveCurrentData,
            _ => ReadyState::HaveMetadata,
        }
    }

    /// The tick's poll: platform state to events, in the order a browser fires them.
    fn poll(&mut self) {
        let Some(item) = self.item.clone() else {
            return;
        };
        if self.error.is_none() {
            let failure = match unsafe { item.status() } {
                AVPlayerItemStatus::Failed => Some(
                    unsafe { item.error() }
                        .as_deref()
                        .map(Failure::of)
                        .unwrap_or(Failure {
                            domain: String::new(),
                            code: 0,
                            text: "the media could not be opened".into(),
                            url_code: None,
                        }),
                ),
                _ => self
                    .shared
                    .failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take(),
            };
            if let Some(failure) = failure {
                trace(format_args!("  failed: {failure:?}"));
                self.error = Some(self.media_error(&failure));
                self.reported_ready = ReadyState::HaveNothing;
                unsafe { self.player.pause() };
                self.events.push(MediaEvent::Error);
                return;
            }
        } else {
            return;
        }

        // `seeked` before the `canplay` the landed seek brings.
        self.events
            .append(&mut self.shared.events.lock().unwrap_or_else(|e| e.into_inner()));

        self.track_kinds();
        let ready = self.compute_ready_state();
        while self.reported_ready < ready {
            let next = ReadyState::from_u8(self.reported_ready as u8 + 1);
            match next {
                ReadyState::HaveMetadata => {
                    self.reported_duration = self.duration();
                    self.events
                        .extend([MediaEvent::DurationChange, MediaEvent::LoadedMetadata]);
                    if let Some(at) = self.pending_seek.take() {
                        self.start_seek(at);
                    }
                }
                ReadyState::HaveCurrentData if !self.loaded_data_fired => {
                    self.loaded_data_fired = true;
                    self.events.push(MediaEvent::LoadedData);
                }
                ReadyState::HaveFutureData => self.events.push(MediaEvent::CanPlay),
                ReadyState::HaveEnoughData => self.events.push(MediaEvent::CanPlayThrough),
                _ => {}
            }
            self.reported_ready = next;
        }
        // A seek drops the state to metadata: it climbs back up with fresh `canplay`s, as in a browser.
        if ready < self.reported_ready && ready >= ReadyState::HaveMetadata {
            self.reported_ready = ready;
        }

        if self.reported_ready >= ReadyState::HaveMetadata {
            let duration = self.duration();
            let same = duration == self.reported_duration
                || (duration.is_nan() && self.reported_duration.is_nan());
            if !same {
                self.reported_duration = duration;
                self.events.push(MediaEvent::DurationChange);
            }
        }

        if self.shared.reached_end.swap(false, Ordering::SeqCst) {
            if self.looping {
                self.start_seek(0.0);
            } else {
                self.ended = true;
                unsafe { self.player.pause() };
                if !self.paused {
                    self.paused = true;
                    self.events.push(MediaEvent::Pause);
                }
                self.playing_reported = false;
                self.events.push(MediaEvent::Ended);
            }
        }

        let status = unsafe { self.player.timeControlStatus() };
        let playing = !self.paused && status == AVPlayerTimeControlStatus::Playing;
        if playing && !self.playing_reported {
            self.playing_reported = true;
            self.waiting_reported = false;
            self.events.push(MediaEvent::Playing);
        }
        let stalled = !self.paused
            && status == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate
            && unsafe { self.player.reasonForWaitingToPlay() }
                .is_some_and(|reason| &*reason == unsafe { AVPlayerWaitingToMinimizeStallsReason });
        if stalled && !self.waiting_reported {
            self.waiting_reported = true;
            self.playing_reported = false;
            self.events.push(MediaEvent::Waiting);
        }
        if self.paused {
            self.playing_reported = false;
            self.waiting_reported = false;
        }
    }

    fn media_error(&self, failure: &Failure) -> MediaError {
        use objc2_av_foundation::AVError;
        let src = &self.src;
        let detail = format!("{} ({} {})", failure.text, failure.domain, failure.code);
        let network = src.starts_with("http://") || src.starts_with("https://");
        if let Some(code) = failure.url_code {
            // -1022: App Transport Security refused a cleartext http:// load.
            let ats = if code == -1022 {
                " The app's App Transport Security policy refuses plain http://: serve the media \
                 over https, or allow the host under NSAppTransportSecurity in the app's \
                 Info.plist."
            } else {
                ""
            };
            return MediaError::new(
                MediaErrorCode::Network,
                format!("a network error stopped playback: {detail}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: the media could not be fetched ({}). Check that the URL \
                 is reachable and serves the media itself rather than an error page.{ats}",
                failure.text
            ));
        }
        let av = failure.domain == "AVFoundationErrorDomain";
        let code = failure.code;
        if av
            && [
                AVError::DecodeFailed.0,
                AVError::DecoderNotFound.0,
                AVError::DecoderTemporarilyUnavailable.0,
            ]
            .contains(&code)
        {
            return MediaError::new(
                MediaErrorCode::Decode,
                format!("the media could not be decoded: {detail}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: the file was recognised but its video or audio could not \
                 be decoded on this Mac. Re-encode to H.264 (or HEVC) with AAC audio in an MP4: \
                 `ffmpeg -i in -c:v libx264 -pix_fmt yuv420p -c:a aac out.mp4`."
            ));
        }
        if av
            && [
                AVError::ContentIsProtected.0,
                AVError::ContentIsNotAuthorized.0,
            ]
            .contains(&code)
        {
            return MediaError::new(MediaErrorCode::Decode, "the media is encrypted")
                .with_advisory(
                "<video>: encrypted media (EME / FairPlay) is not supported and is not planned.",
            );
        }
        if network {
            return MediaError::new(
                MediaErrorCode::Network,
                format!("the source could not be opened: {detail}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: AVFoundation could not open the URL ({}). An unreachable \
                 host, a 404, or an HTML page served with 200 fails here, as does a container \
                 macOS cannot play; load the file from disk once to tell the two apart.",
                failure.text
            ));
        }
        MediaError::new(
            MediaErrorCode::SrcNotSupported,
            format!("the source is not supported: {detail}"),
        )
        .with_advisory(format!(
            "<video src=\"{src}\"> could not be played: AVFoundation refused the file ({}). macOS \
             plays MP4, MOV and M4V with H.264, HEVC or AV1 (on hardware with an AV1 decoder) \
             video and AAC, MP3 or ALAC audio; WebM, Matroska and Ogg are not supported. The \
             always-safe fix is to re-encode: `ffmpeg -i in -c:v libx264 -pix_fmt yuv420p -c:a \
             aac out.mp4`.",
            failure.text
        ))
    }

    fn time_ranges(ranges: Retained<NSArray<NSValue>>) -> Vec<(f64, f64)> {
        ranges
            .iter()
            .filter_map(|value| {
                let range: CMTimeRange = unsafe { value.CMTimeRangeValue() };
                let start = unsafe { range.start.seconds() };
                let length = unsafe { range.duration.seconds() };
                (start.is_finite() && length.is_finite()).then_some((start, start + length))
            })
            .collect()
    }
}

impl Drop for AvPlayerBackend {
    fn drop(&mut self) {
        self.unload();
    }
}

impl VideoBackend for AvPlayerBackend {
    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError> {
        let had_source = self.loaded;
        self.unload();
        if had_source {
            self.events.push(MediaEvent::Emptied);
        }
        self.events.push(MediaEvent::LoadStart);
        self.loaded = true;
        let asset = self.asset_for(source)?;
        trace(format_args!("load {} (AVPlayer)", self.src));
        let item = unsafe { AVPlayerItem::playerItemWithAsset(&asset, self.mtm) };
        unsafe {
            // AVFoundation's stall prediction does not work for data served by a resource loader.
            self.player
                .setAutomaticallyWaitsToMinimizeStalling(self.served.is_none());
            self.player.replaceCurrentItemWithPlayerItem(Some(&item));
            self.player.setVolume(self.volume as f32);
            self.player.setMuted(self.muted);
        }
        self.observe(&item);
        self.item = Some(item);
        if !self.paused {
            unsafe { self.player.setRate(self.rate as f32) };
        }
        Ok(())
    }

    fn play(&mut self) {
        if self.ended || (self.item_ready() && self.current_time() >= self.duration()) {
            self.start_seek(0.0);
        }
        if self.paused {
            self.paused = false;
            self.events.push(MediaEvent::Play);
        }
        if self.item.is_some() && self.error.is_none() {
            unsafe { self.player.setRate(self.rate as f32) };
        }
    }

    fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.events.push(MediaEvent::Pause);
        }
        unsafe { self.player.pause() };
    }

    fn seek(&mut self, seconds: f64) {
        if self.item.is_none() {
            return;
        }
        if self.reported_ready < ReadyState::HaveMetadata {
            self.pending_seek = Some(seconds);
            return;
        }
        let duration = self.duration();
        let seconds = if duration.is_finite() {
            seconds.min(duration)
        } else {
            seconds
        };
        self.start_seek(seconds);
    }

    fn set_volume(&mut self, volume: f64) {
        self.volume = volume;
        unsafe { self.player.setVolume(volume as f32) };
        self.events.push(MediaEvent::VolumeChange);
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        unsafe { self.player.setMuted(muted) };
        self.events.push(MediaEvent::VolumeChange);
    }

    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        if !self.paused && self.item.is_some() {
            unsafe { self.player.setRate(rate as f32) };
        }
        self.events.push(MediaEvent::RateChange);
    }

    fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    fn current_time(&self) -> f64 {
        if self.is_seeking() {
            return self.seek_target;
        }
        if self.item.is_none() {
            return 0.0;
        }
        let duration = self.duration();
        if self.ended && duration.is_finite() {
            return duration;
        }
        // With no action at the item's end the player's clock runs on past it until `poll` sees
        // the end notification and pauses.
        let t = unsafe { self.player.currentTime().seconds() };
        if !t.is_finite() {
            return 0.0;
        }
        if duration.is_finite() {
            t.clamp(0.0, duration)
        } else {
            t.max(0.0)
        }
    }

    fn duration(&self) -> f64 {
        let Some(item) = &self.item else {
            return f64::NAN;
        };
        if !self.item_ready() {
            return f64::NAN;
        }
        let duration = unsafe { item.duration() };
        if duration.flags.contains(CMTimeFlags::Indefinite)
            || duration.flags.contains(CMTimeFlags::PositiveInfinity)
        {
            return f64::INFINITY;
        }
        unsafe { duration.seconds() }
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn ended(&self) -> bool {
        self.ended
    }

    fn seeking(&self) -> bool {
        self.is_seeking()
    }

    fn audible(&self) -> bool {
        self.has_audio == Some(true)
            && !self.paused
            && !self.ended
            && !self.muted
            && self.volume > 0.0
            && unsafe { self.player.timeControlStatus() } == AVPlayerTimeControlStatus::Playing
    }

    fn ready_state(&self) -> ReadyState {
        self.reported_ready
    }

    fn network_state(&self) -> NetworkState {
        if !self.loaded {
            return NetworkState::Empty;
        }
        if self
            .error
            .as_ref()
            .is_some_and(|e| e.code == MediaErrorCode::SrcNotSupported)
        {
            return NetworkState::NoSource;
        }
        if self.error.is_some() || self.item.is_none() {
            return NetworkState::Idle;
        }
        if !self.item_ready() {
            return NetworkState::Loading;
        }
        let duration = self.duration();
        let complete = !self.remote
            || self.buffered().iter().any(|&(start, end)| {
                start <= 0.01 && duration.is_finite() && end >= duration - 0.01
            });
        if complete {
            NetworkState::Idle
        } else {
            NetworkState::Loading
        }
    }

    fn native_size(&self) -> Option<(u32, u32)> {
        if let Some(item) = &self.item {
            let size = unsafe { item.presentationSize() };
            if size.width >= 1.0 && size.height >= 1.0 {
                return Some((size.width.round() as u32, size.height.round() as u32));
            }
        }
        self.frames.last_size()
    }

    fn buffered(&self) -> Vec<(f64, f64)> {
        match &self.item {
            Some(item) => Self::time_ranges(unsafe { item.loadedTimeRanges() }),
            None => Vec::new(),
        }
    }

    fn seekable(&self) -> Vec<(f64, f64)> {
        match &self.item {
            Some(item) if self.item_ready() => {
                Self::time_ranges(unsafe { item.seekableTimeRanges() })
            }
            _ => Vec::new(),
        }
    }

    fn error(&self) -> Option<MediaError> {
        self.error.clone()
    }

    /// `+[AVURLAsset isPlayableExtendedMIMEType:]` asks the installed decoders, codecs parameter included.
    fn can_play_type(&self, mime_type: &str) -> CanPlay {
        let mime = mime_type.trim();
        if mime.is_empty() {
            return CanPlay::No;
        }
        let playable = unsafe { AVURLAsset::isPlayableExtendedMIMEType(&NSString::from_str(mime)) };
        match (playable, mime.contains("codecs")) {
            (false, _) => CanPlay::No,
            (true, true) => CanPlay::Probably,
            (true, false) => CanPlay::Maybe,
        }
    }

    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame> {
        if self.error.is_some() {
            return None;
        }
        if self.has_video == Some(false) {
            return None;
        }
        let item = self.item.clone()?;
        let native = self.native_size()?;
        let presented = self.frames.present(&item, native, request);
        if presented.output_replaced && self.paused && !self.is_seeking() {
            // An exact seek to where the player already is decodes the picture again for the new output.
            unsafe {
                let now = self.player.currentTime();
                self.player.seekToTime_toleranceBefore_toleranceAfter(
                    now,
                    kCMTimeZero,
                    kCMTimeZero,
                );
            }
        }
        presented.frame
    }

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        self.poll();
        for event in self.events.drain(..) {
            sink(event);
        }
    }

    fn suspend(&mut self) {
        self.pause();
        self.frames.forget();
    }
}
