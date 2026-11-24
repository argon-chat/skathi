//! The Linux backend: GStreamer's `playbin3` (`playbin` before 1.22) with an appsink.
//!
//! GStreamer is loaded with `dlopen` on first use ([`ffi`]), so the host neither links it nor needs
//! it installed to start. playbin demuxes, decodes, plays the audio through the default sink
//! (PipeWire or PulseAudio via `autoaudiosink`) and owns the pipeline clock; the video branch is
//! `videoconvertscale ! capsfilter ! appsink`, BGRA at the painted size, synced to that clock.
//!
//! The bus is popped on the UI thread once per tick, turning messages into events in the order a
//! browser fires them. The streaming threads touch nothing but the appsink's sample slot and the
//! appsrc's read offset.

mod audio;
mod codecs;
mod ffi;
mod frame;
mod source;

use std::ffi::c_int;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ffi::{Api, Ptr};

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaErrorCode, MediaEvent, MediaSource, NetworkState,
    ReadyState, VideoBackend, VideoFrame,
};
use crate::trace;

const BASE_CAPS: &str = "video/x-raw,format=BGRA,pixel-aspect-ratio=1/1";

/// A seek the backend has sent or will send. `user` seeks are the element's `currentTime` writes
/// and fire `seeking`/`seeked`; the others re-decode the picture or apply a rate, silently.
#[derive(Clone, Copy)]
struct Seek {
    target: f64,
    user: bool,
}

struct InFlight {
    seek: Seek,
    issued: Instant,
    async_done: bool,
}

/// A failure from the bus, reduced to plain data.
#[derive(Debug, Clone)]
struct Failure {
    domain: Domain,
    code: i32,
    text: String,
    debug: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Domain {
    Core,
    Resource,
    Stream,
    Other,
}

#[derive(Debug, Clone)]
struct Missing {
    description: String,
    detail: String,
}

impl Missing {
    fn video_decoder(&self) -> bool {
        self.detail.starts_with("decoder-video/")
    }

    fn packages(&self) -> Option<codecs::Packages> {
        if let Some(caps) = self.detail.strip_prefix("decoder-") {
            return codecs::kind_for_caps(caps).map(|k| k.packages);
        }
        if self.detail.starts_with("urisource-http") {
            return Some(codecs::Packages::Good);
        }
        None
    }
}

struct Pipeline {
    audio_filter: bool,
    playbin: Ptr,
    bus: Ptr,
    video_bin: Ptr,
    video_pad: Ptr,
    capsfilter: Ptr,
    audio_sink: Ptr,
    audio_pad: Ptr,
}

pub struct GstBackend {
    capture: Arc<audio::Capture>,
    api: &'static Api,
    pipeline: Option<Pipeline>,
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

    prerolled: bool,
    in_flight: Option<InFlight>,
    queued: Option<Seek>,
    /// A seek asked for before the pipeline could take one, applied at `loadedmetadata`.
    pending_seek: Option<f64>,
    buffering: bool,
    /// Where the current stretch of uninterrupted playback started: a remote source's buffered
    /// range when GStreamer reports none.
    contiguous_from: f64,
    duration: f64,
    native: Option<(u32, u32)>,
    caps_size: (u32, u32),
    has_audio: Option<bool>,
    has_video: Option<bool>,
    missing: Vec<Missing>,

    reported_ready: ReadyState,
    reported_duration: f64,
    loaded_data_fired: bool,
    playing_reported: bool,
    waiting_reported: bool,
    error: Option<MediaError>,
}

fn install_advice() -> &'static str {
    "Ubuntu/Debian: `sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
     gstreamer1.0-libav`; Fedora: `sudo dnf install gstreamer1-plugins-base \
     gstreamer1-plugins-good gstreamer1-plugin-openh264`; Arch: `sudo pacman -S gst-plugins-base \
     gst-plugins-good gst-libav`"
}

impl GstBackend {
    pub fn new() -> Result<Self, MediaError> {
        let api = ffi::api().map_err(|e| {
            trace(format_args!("GStreamer unavailable: {e:?}"));
            let (detail, state) = match &e {
                ffi::Unavailable::Missing(d) => (d, "is not installed on this machine"),
                ffi::Unavailable::Broken(d) => (d, "on this machine could not be used"),
            };
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("GStreamer could not be loaded ({detail})"),
            )
            .with_advisory(format!(
                "<video> plays through GStreamer on Linux, and GStreamer 1.x {state}. Install \
                 or update it with the base, good and libav plugins — {}.",
                install_advice()
            ))
        })?;
        let converter = api.has_factory("videoconvertscale")
            || (api.has_factory("videoconvert") && api.has_factory("videoscale"));
        let missing: Vec<&str> = ["playbin", "appsink"]
            .into_iter()
            .filter(|f| !api.has_factory(f))
            .chain((!converter).then_some("videoconvertscale"))
            .collect();
        if !missing.is_empty() {
            return Err(MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("GStreamer is missing {}", missing.join(", ")),
            )
            .with_advisory(format!(
                "<video> needs GStreamer's base plugins ({} not found). Install them — {}.",
                missing.join(", "),
                codecs::Packages::Base.advice()
            )));
        }
        Ok(Self {
            api,
            capture: Arc::default(),
            pipeline: None,
            frames: frame::Frames::new(),
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
            prerolled: false,
            in_flight: None,
            queued: None,
            pending_seek: None,
            buffering: false,
            contiguous_from: 0.0,
            duration: f64::NAN,
            native: None,
            caps_size: (0, 0),
            has_audio: None,
            has_video: None,
            missing: Vec::new(),
            reported_ready: ReadyState::HaveNothing,
            reported_duration: f64::NAN,
            loaded_data_fired: false,
            playing_reported: false,
            waiting_reported: false,
            error: None,
        })
    }

    fn unload(&mut self) {
        if let Some(p) = self.pipeline.take() {
            let core = &self.api.core;
            // SAFETY: `set_state(NULL)` joins the streaming threads, so no callback outlives this;
            // each pointer is a reference this backend owns.
            unsafe {
                (core.gst_element_set_state)(p.playbin, ffi::STATE_NULL);
                for object in [
                    p.bus,
                    p.video_pad,
                    p.capsfilter,
                    p.video_bin,
                    p.audio_pad,
                    p.audio_sink,
                    p.playbin,
                ] {
                    if !object.is_null() {
                        (core.gst_object_unref)(object);
                    }
                }
            }
        }
        self.frames = frame::Frames::new();
        self.ended = false;
        self.prerolled = false;
        self.in_flight = None;
        self.queued = None;
        self.pending_seek = None;
        self.buffering = false;
        self.contiguous_from = 0.0;
        self.duration = f64::NAN;
        self.native = None;
        self.caps_size = (0, 0);
        self.has_audio = None;
        self.has_video = None;
        self.missing.clear();
        self.reported_ready = ReadyState::HaveNothing;
        self.reported_duration = f64::NAN;
        self.loaded_data_fired = false;
        self.playing_reported = false;
        self.waiting_reported = false;
        self.error = None;
    }

    fn uri_for(
        &mut self,
        source: &MediaSource,
    ) -> Result<(String, Option<Arc<source::Feed>>), MediaError> {
        self.remote = matches!(source, MediaSource::Uri(_));
        Ok(match source {
            MediaSource::Path(path) => {
                self.src = path.display().to_string();
                let name = ffi::c(&path.to_string_lossy());
                let mut err = std::ptr::null_mut();
                // SAFETY: a NUL-terminated path and an out-pointer for the error.
                let uri = unsafe { (self.api.core.gst_filename_to_uri)(name.as_ptr(), &mut err) };
                let error = unsafe { ffi::take_error(&self.api.glib, err) };
                if uri.is_null() {
                    return Err(MediaError::new(
                        MediaErrorCode::SrcNotSupported,
                        format!(
                            "\"{}\" is not a usable path: {}",
                            self.src,
                            error.unwrap_or_default()
                        ),
                    ));
                }
                (unsafe { self.api.owned(uri) }, None)
            }
            MediaSource::Uri(url) => {
                self.src = url.clone();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(MediaError::new(
                        MediaErrorCode::SrcNotSupported,
                        format!("\"{url}\" is not an http(s) URL"),
                    )
                    .with_advisory(format!(
                        "<video src=\"{url}\">: only local files, http(s) URLs and data: URIs are \
                         supported."
                    )));
                }
                (url.clone(), None)
            }
            MediaSource::Bytes { mime, data } => {
                self.src = format!("data:{mime} ({} bytes)", data.len());
                let feed = source::Feed::new(source::Bytes::Shared(data.clone()));
                ("appsrc://".into(), Some(feed))
            }
            MediaSource::StaticBytes { mime, data } => {
                self.src = format!("package:{mime} ({} bytes)", data.len());
                let feed = source::Feed::new(source::Bytes::Mapped(data));
                ("appsrc://".into(), Some(feed))
            }
        })
    }

    fn build(
        &mut self,
        uri: &str,
        feed: Option<Arc<source::Feed>>,
    ) -> Result<Pipeline, MediaError> {
        let api = self.api;
        let core = &api.core;
        let setup = |what: String| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("the GStreamer pipeline could not be built: {what}"),
            )
            .with_advisory(format!(
                "<video> could not set up GStreamer ({what}). Reinstall its base plugins — {}.",
                codecs::Packages::Base.advice()
            ))
        };
        let factory = std::env::var("SK_VIDEO_GST_PLAYBIN").unwrap_or_else(|_| {
            if api.version >= (1, 22, 0) && api.has_factory("playbin3") {
                "playbin3".into()
            } else {
                "playbin".into()
            }
        });
        let name = ffi::c(&factory);
        // SAFETY: element creation by factory name; the floating reference becomes ours.
        let playbin = unsafe { (core.gst_element_factory_make)(name.as_ptr(), std::ptr::null()) };
        if playbin.is_null() {
            return Err(setup(format!("no {factory} element")));
        }
        let mut p = Pipeline {
            audio_filter: false,
            playbin: unsafe { (core.gst_object_ref_sink)(playbin) },
            bus: std::ptr::null_mut(),
            video_bin: std::ptr::null_mut(),
            video_pad: std::ptr::null_mut(),
            capsfilter: std::ptr::null_mut(),
            audio_sink: std::ptr::null_mut(),
            audio_pad: std::ptr::null_mut(),
        };
        trace(format_args!(
            "load {} ({factory}, GStreamer {:?})",
            self.src, api.version
        ));

        let convert = if api.has_factory("videoconvertscale") {
            "videoconvertscale add-borders=false"
        } else {
            "videoconvert ! videoscale add-borders=false"
        };
        let description = format!(
            "{convert} ! capsfilter name=size caps=\"{BASE_CAPS}\" ! appsink name=sink sync=true \
             max-buffers=1 drop=true enable-last-sample=false qos=true"
        );
        let bin = match api.parse_bin(&description) {
            Ok(bin) => bin,
            Err(e) => {
                // SAFETY: the reference taken above; nothing else holds the playbin yet.
                unsafe { (core.gst_object_unref)(p.playbin) };
                return Err(setup(e));
            }
        };
        p.video_bin = bin;
        let (size, sink, pad) = (ffi::c("size"), ffi::c("sink"), ffi::c("sink"));
        // SAFETY: lookups in the bin just built; each returns a new reference or NULL.
        unsafe {
            p.capsfilter = (core.gst_bin_get_by_name)(bin, size.as_ptr());
            let appsink = (core.gst_bin_get_by_name)(bin, sink.as_ptr());
            p.video_pad = (core.gst_element_get_static_pad)(bin, pad.as_ptr());
            if !appsink.is_null() {
                frame::attach(api, appsink, &self.frames.latest);
                (core.gst_object_unref)(appsink);
            }
        }
        api.set_ptr(p.playbin, "video-sink", bin);

        p.audio_filter = audio::install(api, p.playbin, &self.capture);
        if !p.audio_filter && self.capture.routed() {
            unsafe {
                for object in [p.video_pad, p.capsfilter, p.video_bin, p.playbin]
                    .into_iter()
                    .filter(|p| !p.is_null())
                {
                    (core.gst_object_unref)(object);
                }
            }
            return Err(setup(
                "the Web Audio filter needs tee, queue, audioconvert and appsink".into(),
            ));
        }

        let audio = match std::env::var("SK_VIDEO_GST_AUDIO_SINK") {
            Ok(description) if !description.trim().is_empty() => api.parse_bin(&description).ok(),
            _ => {
                let auto = ffi::c("autoaudiosink");
                // SAFETY: element creation by name; the floating reference becomes ours.
                unsafe {
                    let sink = (core.gst_element_factory_make)(auto.as_ptr(), std::ptr::null());
                    (!sink.is_null()).then(|| (core.gst_object_ref_sink)(sink))
                }
            }
        };
        if let Some(audio) = audio {
            p.audio_sink = audio;
            p.audio_pad = unsafe { (core.gst_element_get_static_pad)(audio, pad.as_ptr()) };
            api.set_ptr(p.playbin, "audio-sink", audio);
        }

        // Subtitles are left to the app: the DOM renders text tracks only through <track>.
        // `native-video` keeps playsink's own converter out, so the bin's sink pad carries the
        // decoder's caps (the native size) and the frame is converted and scaled once.
        let flags =
            (api.get_u32(p.playbin, "flags") & !ffi::PLAY_FLAG_TEXT) | ffi::PLAY_FLAG_NATIVE_VIDEO;
        api.set_i32(p.playbin, "flags", flags as c_int);
        api.set_str(p.playbin, "uri", uri);
        let routed = self.capture.routed();
        api.set_f64(p.playbin, "volume", if routed { 1.0 } else { self.volume });
        api.set_bool(p.playbin, "mute", routed || self.muted);
        if let Some(feed) = &feed {
            source::connect(api, p.playbin, feed);
        }
        p.bus = unsafe { (core.gst_element_get_bus)(p.playbin) };
        Ok(p)
    }

    fn playbin(&self) -> Ptr {
        self.pipeline
            .as_ref()
            .map_or(std::ptr::null_mut(), |p| p.playbin)
    }

    /// Plays when the element wants to and nothing holds it back; pauses otherwise.
    fn apply_state(&self) {
        let playbin = self.playbin();
        if playbin.is_null() {
            return;
        }
        let state = if !self.paused && !self.ended && !self.buffering && self.error.is_none() {
            ffi::STATE_PLAYING
        } else {
            ffi::STATE_PAUSED
        };
        // SAFETY: a live pipeline; the result is reported on the bus.
        unsafe { (self.api.core.gst_element_set_state)(playbin, state) };
    }

    /// `(result, current)` of the pipeline's state without waiting.
    fn state(&self) -> (c_int, c_int) {
        let (mut current, mut pending) = (0, 0);
        let playbin = self.playbin();
        if playbin.is_null() {
            return (ffi::STATE_CHANGE_FAILURE, 0);
        }
        // SAFETY: a live pipeline and out-pointers to locals.
        let result = unsafe {
            (self.api.core.gst_element_get_state)(playbin, &mut current, &mut pending, 0)
        };
        (result, current)
    }

    fn request_seek(&mut self, target: f64, user: bool) {
        if user {
            self.ended = false;
            self.contiguous_from = target;
            self.events.push(MediaEvent::Seeking);
        }
        let seek = Seek { target, user };
        if self.in_flight.is_some() {
            // A queued element seek also applies the current rate and decodes a fresh picture.
            if !(self.queued.is_some_and(|q| q.user) && !user) {
                self.queued = Some(seek);
            }
            return;
        }
        self.issue(seek);
    }

    fn issue(&mut self, seek: Seek) {
        let playbin = self.playbin();
        let ns = (seek.target.max(0.0) * 1e9) as i64;
        // SAFETY: a live pipeline. A flushing seek makes the sinks preroll again, which ends in ASYNC_DONE.
        let ok = !playbin.is_null()
            && unsafe {
                (self.api.core.gst_element_seek)(
                    playbin,
                    self.rate,
                    ffi::FORMAT_TIME,
                    ffi::SEEK_FLAG_FLUSH | ffi::SEEK_FLAG_ACCURATE,
                    ffi::SEEK_TYPE_SET,
                    ns,
                    ffi::SEEK_TYPE_NONE,
                    -1,
                )
            } != ffi::FALSE;
        trace(format_args!(
            "  seek to {:.3} at rate {} ({}): {ok}",
            seek.target,
            self.rate,
            if seek.user { "element" } else { "internal" }
        ));
        if ok {
            self.in_flight = Some(InFlight {
                seek,
                issued: Instant::now(),
                async_done: false,
            });
        } else {
            self.seek_finished(seek);
        }
    }

    /// A rate change without a flush (GStreamer 1.18+), so playback does not stall on a re-decode
    /// from the last keyframe. `false` when the pipeline refuses it or a seek is in flight.
    fn instant_rate(&self) -> bool {
        let playbin = self.playbin();
        if playbin.is_null() || self.in_flight.is_some() || self.api.version < (1, 18, 0) {
            return false;
        }
        // SAFETY: a live pipeline; position and stop are left as they are.
        let ok = unsafe {
            (self.api.core.gst_element_seek)(
                playbin,
                self.rate,
                ffi::FORMAT_TIME,
                ffi::SEEK_FLAG_INSTANT_RATE_CHANGE,
                ffi::SEEK_TYPE_NONE,
                -1,
                ffi::SEEK_TYPE_NONE,
                -1,
            )
        } != ffi::FALSE;
        trace(format_args!("  instant rate {}: {ok}", self.rate));
        ok
    }

    fn seek_finished(&mut self, done: Seek) {
        let next = self.queued.take();
        if done.user && !next.is_some_and(|n| n.user) {
            self.events.push(MediaEvent::Seeked);
        }
        if let Some(next) = next {
            self.issue(next);
        }
    }

    fn is_seeking(&self) -> bool {
        self.in_flight.as_ref().is_some_and(|f| f.seek.user) || self.queued.is_some_and(|q| q.user)
    }

    fn seek_target(&self) -> Option<f64> {
        self.queued
            .map(|q| q.target)
            .or_else(|| self.in_flight.as_ref().map(|f| f.seek.target))
    }

    fn query_position(&self) -> Option<f64> {
        let playbin = self.playbin();
        let mut ns = 0i64;
        // SAFETY: a live pipeline and an out-pointer to a local.
        let ok = !playbin.is_null()
            && unsafe {
                (self.api.core.gst_element_query_position)(playbin, ffi::FORMAT_TIME, &mut ns)
            } != ffi::FALSE;
        (ok && ns >= 0).then(|| ns as f64 / 1e9)
    }

    fn refresh_duration(&mut self) {
        let playbin = self.playbin();
        let mut ns = 0i64;
        // SAFETY: as in `query_position`.
        let ok = !playbin.is_null()
            && unsafe {
                (self.api.core.gst_element_query_duration)(playbin, ffi::FORMAT_TIME, &mut ns)
            } != ffi::FALSE;
        if ok && ns >= 0 {
            self.duration = ns as f64 / 1e9;
        } else if self.duration.is_nan() {
            self.duration = f64::INFINITY;
        }
    }

    /// The decoded video's display size and whether each kind of track reached its sink.
    fn discover_tracks(&mut self) {
        let Some(p) = &self.pipeline else { return };
        let api = self.api;
        let caps_of = |pad: Ptr| -> Option<Ptr> {
            if pad.is_null() {
                return None;
            }
            // SAFETY: a pad this backend holds a reference to; the caps reference is returned.
            let caps = unsafe { (api.core.gst_pad_get_current_caps)(pad) };
            (!caps.is_null()).then_some(caps)
        };
        let video = caps_of(p.video_pad);
        if let Some(caps) = video {
            // SAFETY: live caps, released right after.
            if let Some((w, h, (n, d))) = unsafe { api.video_caps(caps) } {
                let width = ((w as u64 * n as u64) as f64 / d as f64).round().max(1.0) as u32;
                self.native = Some((width, h));
            }
            unsafe { (api.core.gst_mini_object_unref)(caps) };
        }
        let audio = caps_of(p.audio_pad);
        if let Some(caps) = audio {
            unsafe { (api.core.gst_mini_object_unref)(caps) };
        }
        if self.prerolled {
            self.has_video = Some(video.is_some() || self.frames.has_frame());
            self.has_audio = Some(audio.is_some());
        }
    }

    fn compute_ready_state(&self) -> ReadyState {
        if self.error.is_some() || !self.prerolled {
            return ReadyState::HaveNothing;
        }
        if self.is_seeking() {
            return ReadyState::HaveMetadata;
        }
        let current = self.has_video == Some(false) || self.frames.has_frame();
        match (current, self.buffering) {
            (false, _) => ReadyState::HaveMetadata,
            (true, true) => ReadyState::HaveCurrentData,
            (true, false) => ReadyState::HaveEnoughData,
        }
    }

    /// Drains the bus. Returns `(eos, async_done)`, or the failure that ends playback.
    fn drain_bus(&mut self) -> Result<(bool, bool), Failure> {
        let Some(bus) = self.pipeline.as_ref().map(|p| p.bus) else {
            return Ok((false, false));
        };
        let api = self.api;
        let core = &api.core;
        let (mut eos, mut async_done) = (false, false);
        let mut failure = None;
        loop {
            // SAFETY: a live bus; each popped message is ours and is released below.
            let msg = unsafe { (core.gst_bus_pop)(bus) };
            if msg.is_null() {
                break;
            }
            let kind = unsafe { (*msg).type_ };
            match kind {
                ffi::MSG_ERROR | ffi::MSG_WARNING => {
                    let mut err = std::ptr::null_mut();
                    let mut debug = std::ptr::null_mut();
                    let parsed = unsafe {
                        if kind == ffi::MSG_ERROR {
                            (core.gst_message_parse_error)(msg, &mut err, &mut debug);
                        } else {
                            (core.gst_message_parse_warning)(msg, &mut err, &mut debug);
                        }
                        let (domain, code) = if err.is_null() {
                            (0, 0)
                        } else {
                            ((*err).domain, (*err).code)
                        };
                        let domain = if domain == (core.gst_core_error_quark)() {
                            Domain::Core
                        } else if domain == (core.gst_resource_error_quark)() {
                            Domain::Resource
                        } else if domain == (core.gst_stream_error_quark)() {
                            Domain::Stream
                        } else {
                            Domain::Other
                        };
                        Failure {
                            domain,
                            code,
                            text: ffi::take_error(&api.glib, err).unwrap_or_default(),
                            debug: api.owned(debug),
                        }
                    };
                    trace(format_args!(
                        "  {}: {parsed:?}",
                        if kind == ffi::MSG_ERROR {
                            "error"
                        } else {
                            "warning"
                        }
                    ));
                    if kind == ffi::MSG_ERROR && failure.is_none() {
                        failure = Some(parsed);
                    }
                }
                ffi::MSG_ELEMENT => {
                    if let Some(missing) = unsafe { self.missing_plugin(msg) } {
                        trace(format_args!("  missing plugin: {missing:?}"));
                        if !self.missing.iter().any(|m| m.detail == missing.detail) {
                            self.missing.push(missing);
                        }
                    }
                }
                ffi::MSG_EOS => eos = true,
                ffi::MSG_ASYNC_DONE => async_done = true,
                ffi::MSG_BUFFERING => {
                    let mut percent = 100;
                    unsafe { (core.gst_message_parse_buffering)(msg, &mut percent) };
                    let buffering = percent < 100;
                    if buffering != self.buffering {
                        trace(format_args!("  buffering {percent}%"));
                        self.buffering = buffering;
                        self.apply_state();
                    }
                }
                ffi::MSG_DURATION_CHANGED => self.duration = f64::NAN,
                ffi::MSG_CLOCK_LOST => unsafe {
                    (core.gst_element_set_state)(self.playbin(), ffi::STATE_PAUSED);
                    self.apply_state();
                },
                ffi::MSG_LATENCY => unsafe {
                    (core.gst_bin_recalculate_latency)(self.playbin());
                },
                _ => {}
            }
            unsafe { (core.gst_mini_object_unref)(msg.cast()) };
        }
        match failure {
            Some(f) => Err(f),
            None => Ok((eos, async_done)),
        }
    }

    /// # Safety
    /// `msg` is a live element message.
    unsafe fn missing_plugin(&self, msg: *mut ffi::Message) -> Option<Missing> {
        let api = self.api;
        let structure = unsafe { (api.core.gst_message_get_structure)(msg) };
        if structure.is_null()
            || unsafe { ffi::cstr((api.core.gst_structure_get_name)(structure)) }
                != "missing-plugin"
        {
            return None;
        }
        if let Some(pb) = &api.pbutils {
            if unsafe { (pb.gst_is_missing_plugin_message)(msg) } != ffi::FALSE {
                let description =
                    unsafe { api.owned((pb.gst_missing_plugin_message_get_description)(msg)) };
                let installer =
                    unsafe { api.owned((pb.gst_missing_plugin_message_get_installer_detail)(msg)) };
                let detail = installer.split('|').nth(4).unwrap_or("").to_string();
                return Some(Missing {
                    description,
                    detail,
                });
            }
        }
        let text = unsafe { api.owned((api.core.gst_structure_to_string)(structure)) };
        Some(Missing {
            description: text.clone(),
            detail: text,
        })
    }

    fn fail(&mut self, error: MediaError) {
        trace(format_args!("  failed: {:?} {}", error.code, error.message));
        self.error = Some(error);
        self.reported_ready = ReadyState::HaveNothing;
        self.in_flight = None;
        self.queued = None;
        self.apply_state();
        self.events.push(MediaEvent::Error);
    }

    /// The tick's poll: platform state to events, in the order a browser fires them.
    fn poll(&mut self) {
        if self.pipeline.is_none() || self.error.is_some() {
            return;
        }
        let (eos, async_done) = match self.drain_bus() {
            Ok(v) => v,
            Err(failure) => {
                let error = self.media_error(&failure);
                self.fail(error);
                return;
            }
        };
        if let Some(missing) = self.missing.iter().find(|m| m.video_decoder()).cloned() {
            let error = self.missing_error(&[missing]);
            self.fail(error);
            return;
        }

        let (result, current) = self.state();
        if !self.prerolled && result == ffi::STATE_CHANGE_SUCCESS && current >= ffi::STATE_PAUSED {
            self.prerolled = true;
            trace(format_args!("  prerolled"));
        }
        if self.prerolled {
            self.discover_tracks();
            if self.duration.is_nan() || !self.duration.is_finite() {
                self.refresh_duration();
            }
        }

        // `seeked` before the `canplay` the landed seek brings.
        if let Some(flight) = &mut self.in_flight {
            flight.async_done |= async_done;
            let settled = result != ffi::STATE_CHANGE_ASYNC;
            let overdue = flight.issued.elapsed() > Duration::from_secs(2);
            if settled && (flight.async_done || overdue) {
                let done = flight.seek;
                self.in_flight = None;
                self.seek_finished(done);
            }
        }

        let ready = self.compute_ready_state();
        while self.reported_ready < ready {
            let next = ReadyState::from_u8(self.reported_ready as u8 + 1);
            match next {
                ReadyState::HaveMetadata => {
                    self.reported_duration = self.duration;
                    self.events
                        .extend([MediaEvent::DurationChange, MediaEvent::LoadedMetadata]);
                    if let Some(at) = self.pending_seek.take() {
                        self.request_seek(at, true);
                    } else if self.rate != 1.0 {
                        let at = self.current_time();
                        self.request_seek(at, false);
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
            if self.is_seeking() {
                break;
            }
        }
        // A seek drops the state to metadata: it climbs back up with fresh `canplay`s, as in a browser.
        let ready = self.compute_ready_state();
        if ready < self.reported_ready && ready >= ReadyState::HaveMetadata {
            self.reported_ready = ready;
        }

        if self.reported_ready >= ReadyState::HaveMetadata {
            let same = self.duration == self.reported_duration
                || (self.duration.is_nan() && self.reported_duration.is_nan());
            if !same {
                self.reported_duration = self.duration;
                self.events.push(MediaEvent::DurationChange);
            }
        }

        if eos && self.in_flight.is_none() && !self.ended {
            if self.looping {
                self.request_seek(0.0, true);
            } else {
                self.ended = true;
                self.apply_state();
                if !self.paused {
                    self.paused = true;
                    self.events.push(MediaEvent::Pause);
                }
                self.playing_reported = false;
                self.events.push(MediaEvent::Ended);
            }
        }

        let (_, current) = self.state();
        let playing = !self.paused && !self.buffering && current == ffi::STATE_PLAYING;
        if playing && !self.playing_reported {
            self.playing_reported = true;
            self.waiting_reported = false;
            self.events.push(MediaEvent::Playing);
        }
        if !self.paused && self.buffering && !self.waiting_reported {
            self.waiting_reported = true;
            self.playing_reported = false;
            self.events.push(MediaEvent::Waiting);
        }
        if self.paused {
            self.playing_reported = false;
            self.waiting_reported = false;
        }
    }

    fn missing_error(&self, missing: &[Missing]) -> MediaError {
        let src = &self.src;
        let names: Vec<&str> = missing.iter().map(|m| m.description.as_str()).collect();
        let mut packages: Vec<codecs::Packages> =
            missing.iter().filter_map(Missing::packages).collect();
        packages.dedup();
        let install = if packages.is_empty() {
            format!(
                "Install GStreamer's plugin sets — {}; Ubuntu/Debian also have \
                 `gstreamer1.0-plugins-bad` and `gstreamer1.0-plugins-ugly`.",
                install_advice()
            )
        } else {
            let advice: Vec<&str> = packages.iter().map(|p| p.advice()).collect();
            format!("Install it — {}.", advice.join(". "))
        };
        let reencode = if codecs::have_decoder(self.api, codecs::H264)
            && codecs::have_decoder(self.api, codecs::AAC)
        {
            " Or re-encode to H.264 with AAC audio in an MP4, which this machine plays: `ffmpeg \
             -i in -c:v libx264 -pix_fmt yuv420p -c:a aac out.mp4`."
        } else {
            ""
        };
        MediaError::new(
            MediaErrorCode::SrcNotSupported,
            format!("GStreamer has no {}", names.join(", ")),
        )
        .with_advisory(format!(
            "<video src=\"{src}\"> needs a GStreamer plugin this machine does not have: {}. \
             {install}{reencode}",
            names.join(", ")
        ))
    }

    fn media_error(&self, failure: &Failure) -> MediaError {
        // GstCoreError, GstResourceError and GstStreamError codes.
        const CORE_MISSING_PLUGIN: i32 = 12;
        const STREAM_TYPE_NOT_FOUND: i32 = 4;
        const STREAM_WRONG_TYPE: i32 = 5;
        const STREAM_CODEC_NOT_FOUND: i32 = 6;
        const STREAM_DECODE: i32 = 7;
        const STREAM_DEMUX: i32 = 9;
        const STREAM_FORMAT: i32 = 11;
        const STREAM_DECRYPT: i32 = 12;
        const STREAM_DECRYPT_NOKEY: i32 = 13;

        let src = &self.src;
        let detail = format!("{} [{:?} {}]", failure.text, failure.domain, failure.code);
        let code = failure.code;
        let missing_plugin = (failure.domain == Domain::Stream && code == STREAM_CODEC_NOT_FOUND)
            || (failure.domain == Domain::Core && code == CORE_MISSING_PLUGIN);
        if missing_plugin && !self.missing.is_empty() {
            return self.missing_error(&self.missing);
        }
        if failure.domain == Domain::Resource && self.remote {
            return MediaError::new(
                MediaErrorCode::Network,
                format!("a network error stopped playback: {detail}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: the media could not be fetched ({}). Check that the URL \
                 is reachable and serves the media itself rather than an error page.",
                failure.text
            ));
        }
        if failure.domain == Domain::Stream
            && [STREAM_DECRYPT, STREAM_DECRYPT_NOKEY].contains(&code)
        {
            return MediaError::new(MediaErrorCode::Decode, "the media is encrypted").with_advisory(
                "<video>: encrypted media (EME / Widevine / PlayReady) is not supported and is not \
                 planned.",
            );
        }
        if failure.domain == Domain::Stream
            && [STREAM_DECODE, STREAM_DEMUX, STREAM_FORMAT].contains(&code)
            && self.prerolled
        {
            return MediaError::new(
                MediaErrorCode::Decode,
                format!("the media could not be decoded: {detail}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: the file was recognised but its video or audio could not \
                 be decoded ({}). It is likely damaged or truncated; re-encode it: `ffmpeg -i in \
                 -c:v libx264 -pix_fmt yuv420p -c:a aac out.mp4`.",
                failure.text
            ));
        }
        if self.remote && failure.domain == Domain::Stream && code == STREAM_TYPE_NOT_FOUND {
            return MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("the response is not media: {detail}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: the server answered, but not with media GStreamer \
                 recognises — often an HTML error or login page served with 200. Check the URL; \
                 if it is a real video, re-encode it: `ffmpeg -i in -c:v libx264 -pix_fmt yuv420p \
                 -c:a aac out.mp4`."
            ));
        }
        let what = match (failure.domain, code) {
            (Domain::Stream, STREAM_TYPE_NOT_FOUND | STREAM_WRONG_TYPE) => {
                "GStreamer does not recognise the file as media: it is not a video, or its \
                 extension does not match its contents"
                    .to_string()
            }
            (Domain::Resource, _) => format!("the file could not be read ({})", failure.text),
            _ => format!("GStreamer refused the file ({})", failure.text),
        };
        let (installed, missing) = codecs::summary(self.api);
        let mut advisory = format!(
            "<video src=\"{src}\"> could not be played: {what}. Video decoders on this machine: \
             {}.",
            if installed.is_empty() {
                "none".to_string()
            } else {
                installed.join(", ")
            }
        );
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|k| k.name).collect();
            advisory.push_str(&format!(" Not installed: {}.", names.join(", ")));
        }
        advisory.push_str(
            " The always-safe fix is to re-encode: `ffmpeg -i in -c:v libx264 -pix_fmt yuv420p \
             -c:a aac out.mp4`.",
        );
        if !failure.debug.is_empty() {
            trace(format_args!("  debug: {}", failure.debug));
        }
        MediaError::new(
            MediaErrorCode::SrcNotSupported,
            format!("the source is not supported: {detail}"),
        )
        .with_advisory(advisory)
    }

    /// The download buffer's ranges (`queue2` answers in percent of the resource).
    fn buffering_ranges(&self) -> Vec<(f64, f64)> {
        let api = self.api;
        let playbin = self.playbin();
        if playbin.is_null() || !self.duration.is_finite() {
            return Vec::new();
        }
        let format = ffi::FORMAT_PERCENT;
        let scale = |v: i64| v as f64 / ffi::FORMAT_PERCENT_MAX as f64 * self.duration;
        let mut out = Vec::new();
        // SAFETY: a query this function owns, released below.
        unsafe {
            let query = (api.core.gst_query_new_buffering)(format);
            if query.is_null() {
                return out;
            }
            if (api.core.gst_element_query)(playbin, query) != ffi::FALSE {
                let n = (api.core.gst_query_get_n_buffering_ranges)(query);
                for i in 0..n {
                    let (mut start, mut stop) = (0i64, 0i64);
                    if (api.core.gst_query_parse_nth_buffering_range)(
                        query, i, &mut start, &mut stop,
                    ) != ffi::FALSE
                        && stop > start
                    {
                        out.push((scale(start), scale(stop).min(self.duration)));
                    }
                }
                if out.is_empty() {
                    let (mut fmt, mut start, mut stop, mut total) = (0, 0i64, 0i64, 0i64);
                    (api.core.gst_query_parse_buffering_range)(
                        query, &mut fmt, &mut start, &mut stop, &mut total,
                    );
                    if fmt == format && start >= 0 && stop > start {
                        out.push((scale(start), scale(stop).min(self.duration)));
                    }
                }
            }
            (api.core.gst_mini_object_unref)(query);
        }
        out
    }
}

impl Drop for GstBackend {
    fn drop(&mut self) {
        self.unload();
    }
}

impl VideoBackend for GstBackend {
    fn route_audio(&mut self, output: &Arc<dyn crate::AudioOutput>) -> Result<(), MediaError> {
        if let Some(p) = &self.pipeline {
            if !p.audio_filter {
                return Err(MediaError::new(
                    MediaErrorCode::Decode,
                    "GStreamer could not install the Web Audio filter",
                ));
            }
            self.api.set_bool(p.playbin, "mute", true);
            self.api.set_f64(p.playbin, "volume", 1.0);
        }
        self.capture.gain(self.volume, self.muted);
        self.capture.attach(output);
        Ok(())
    }

    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError> {
        let had_source = self.loaded;
        self.unload();
        if had_source {
            self.events.push(MediaEvent::Emptied);
        }
        self.events.push(MediaEvent::LoadStart);
        self.loaded = true;
        let (uri, feed) = self.uri_for(source)?;
        let pipeline = self.build(&uri, feed)?;
        self.pipeline = Some(pipeline);
        self.apply_state();
        Ok(())
    }

    fn play(&mut self) {
        if self.ended || (self.prerolled && self.current_time() >= self.duration) {
            self.request_seek(0.0, true);
        }
        self.ended = false;
        if self.paused {
            self.paused = false;
            self.events.push(MediaEvent::Play);
        }
        self.apply_state();
    }

    fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.events.push(MediaEvent::Pause);
        }
        self.apply_state();
    }

    fn seek(&mut self, seconds: f64) {
        if self.pipeline.is_none() || self.error.is_some() {
            return;
        }
        if self.reported_ready < ReadyState::HaveMetadata {
            self.pending_seek = Some(seconds);
            return;
        }
        let seconds = if self.duration.is_finite() {
            seconds.min(self.duration)
        } else {
            seconds
        };
        self.request_seek(seconds, true);
    }

    fn set_volume(&mut self, volume: f64) {
        self.volume = volume;
        self.capture.gain(volume, self.muted);
        let playbin = self.playbin();
        if !playbin.is_null() && !self.capture.routed() {
            self.api.set_f64(playbin, "volume", volume);
        }
        self.events.push(MediaEvent::VolumeChange);
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.capture.gain(self.volume, muted);
        let playbin = self.playbin();
        if !playbin.is_null() && !self.capture.routed() {
            self.api.set_bool(playbin, "mute", muted);
        }
        self.events.push(MediaEvent::VolumeChange);
    }

    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        if self.prerolled && self.error.is_none() && !self.instant_rate() {
            let at = self.current_time();
            self.request_seek(at, false);
        }
        self.events.push(MediaEvent::RateChange);
    }

    fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    fn current_time(&self) -> f64 {
        if let Some(target) = self.seek_target() {
            return target;
        }
        if self.ended && self.duration.is_finite() {
            return self.duration;
        }
        if !self.prerolled {
            return 0.0;
        }
        let t = self.query_position().unwrap_or(0.0);
        if self.duration.is_finite() {
            t.min(self.duration)
        } else {
            t
        }
    }

    fn duration(&self) -> f64 {
        if self.prerolled {
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
        self.is_seeking()
    }

    fn audible(&self) -> bool {
        let playbin = self.playbin();
        !self.capture.routed()
            && self.has_audio == Some(true)
            && !self.paused
            && !self.ended
            && !playbin.is_null()
            && self.api.get_u32(playbin, "mute") == 0
            && self.api.get_f64(playbin, "volume") > 0.0
            && self.state().1 == ffi::STATE_PLAYING
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
        if self.error.is_some() || self.pipeline.is_none() {
            return NetworkState::Idle;
        }
        if !self.prerolled {
            return NetworkState::Loading;
        }
        let complete = !self.remote
            || self.paused
            || self.buffered().iter().any(|&(start, end)| {
                start <= 0.01 && self.duration.is_finite() && end >= self.duration - 0.01
            });
        if complete {
            NetworkState::Idle
        } else {
            NetworkState::Loading
        }
    }

    fn native_size(&self) -> Option<(u32, u32)> {
        self.native.or_else(|| self.frames.held_size())
    }

    fn buffered(&self) -> Vec<(f64, f64)> {
        if !self.prerolled || !self.duration.is_finite() {
            return Vec::new();
        }
        if !self.remote {
            return vec![(0.0, self.duration)];
        }
        let ranges = self.buffering_ranges();
        if !ranges.is_empty() {
            return ranges;
        }
        // playbin3 pulls an http(s) MP4 by range on demand and answers no buffering query: what
        // has been fetched is the stretch played since the last seek.
        let now = self.current_time();
        vec![(self.contiguous_from.min(now), now)]
    }

    fn seekable(&self) -> Vec<(f64, f64)> {
        let playbin = self.playbin();
        if !self.prerolled || playbin.is_null() {
            return Vec::new();
        }
        let api = self.api;
        let (mut format, mut seekable, mut start, mut end) = (0, ffi::FALSE, 0i64, -1i64);
        // SAFETY: a query this function owns, released below.
        unsafe {
            let query = (api.core.gst_query_new_seeking)(ffi::FORMAT_TIME);
            if query.is_null() {
                return Vec::new();
            }
            if (api.core.gst_element_query)(playbin, query) != ffi::FALSE {
                (api.core.gst_query_parse_seeking)(
                    query,
                    &mut format,
                    &mut seekable,
                    &mut start,
                    &mut end,
                );
            }
            (api.core.gst_mini_object_unref)(query);
        }
        if seekable == ffi::FALSE {
            return Vec::new();
        }
        let start = start.max(0) as f64 / 1e9;
        let end = if end > 0 {
            end as f64 / 1e9
        } else {
            self.duration
        };
        if end.is_finite() && end > start {
            vec![(start, end)]
        } else {
            Vec::new()
        }
    }

    fn error(&self) -> Option<MediaError> {
        self.error.clone()
    }

    /// Asks the GStreamer registry for a demuxer for the container and a decoder for each codec.
    fn can_play_type(&self, mime_type: &str) -> CanPlay {
        codecs::can_play_type(self.api, mime_type)
    }

    #[cfg(feature = "webgpu")]
    fn webgpu_frame(&mut self) -> Option<redjs_webgpu::video::Frame> {
        self.frames.snapshot(self.api)
    }

    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame> {
        if self.error.is_some() || self.has_video == Some(false) {
            return None;
        }
        let capsfilter = self.pipeline.as_ref()?.capsfilter;
        let native = self.native?;
        let target = crate::fit_within(native, request.max_size);
        if target.0 == 0 || target.1 == 0 {
            return None;
        }
        // WebGPU reads the media pixels independently of the element's CSS box.
        let decode = if cfg!(feature = "webgpu") {
            native
        } else {
            target
        };
        if decode != self.caps_size && !capsfilter.is_null() {
            self.caps_size = decode;
            let caps = ffi::c(&format!(
                "{BASE_CAPS},width={},height={}",
                decode.0, decode.1
            ));
            // SAFETY: the capsfilter takes its own reference to the caps.
            unsafe {
                let caps = (self.api.core.gst_caps_from_string)(caps.as_ptr());
                if !caps.is_null() {
                    self.api.set_ptr(capsfilter, "caps", caps);
                    (self.api.core.gst_mini_object_unref)(caps);
                }
            }
            // A paused pipeline makes no new frame by itself: decode the picture again at the new size.
            let smaller = self
                .frames
                .held_size()
                .is_some_and(|(w, h)| w < target.0 || h < target.1);
            trace(format_args!(
                "  frame size {target:?} (holding {:?})",
                self.frames.held_size()
            ));
            if self.paused && self.prerolled && self.in_flight.is_none() && smaller {
                let at = self.current_time();
                self.request_seek(at, false);
            }
        }
        self.frames.present(self.api, target)
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
