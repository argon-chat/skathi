//! The portable half: the types a `<video>` element is made of, and the
//! [`VideoBackend`] trait the platform implements.
//!
//! **Nothing in this file may name a platform type.** That is the whole point:
//! the trait sits at `HTMLMediaElement` altitude, which is the altitude at
//! which Windows' `IMFMediaEngine` and macOS' `AVPlayer` + `AVPlayerItem`
//! *already agree* — both are OS objects that own the demuxer, the decoder, the
//! audio renderer and the master clock, and both expose
//! play/pause/seek/rate/volume/duration/readyState plus a
//! pull-the-current-frame call. A trait pitched any lower (samples, decoders,
//! surfaces) would fit exactly one of them.
//!
//! See CONTRACT.md §2 for the mapping table.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::RenderImage;

// ---------------------------------------------------------------------------
// the spec's enums
// ---------------------------------------------------------------------------

/// `HTMLMediaElement.readyState`. The numbers are the spec's and are what the
/// property reports, so a component that branches on `>= 2` behaves as it does
/// in a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum ReadyState {
    #[default]
    HaveNothing = 0,
    HaveMetadata = 1,
    HaveCurrentData = 2,
    HaveFutureData = 3,
    HaveEnoughData = 4,
}

impl ReadyState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::HaveMetadata,
            2 => Self::HaveCurrentData,
            3 => Self::HaveFutureData,
            4 => Self::HaveEnoughData,
            _ => Self::HaveNothing,
        }
    }
}

/// `HTMLMediaElement.networkState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum NetworkState {
    #[default]
    Empty = 0,
    Idle = 1,
    Loading = 2,
    NoSource = 3,
}

impl NetworkState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Idle,
            2 => Self::Loading,
            3 => Self::NoSource,
            _ => Self::Empty,
        }
    }
}

/// `MediaError.code`. The four the spec defines, and only those — a platform
/// error that does not map onto one of them is a `Decode` with the platform's
/// own text in [`MediaError::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MediaErrorCode {
    Aborted = 1,
    Network = 2,
    Decode = 3,
    SrcNotSupported = 4,
}

/// `HTMLMediaElement.error`, plus the thing a browser never gives you: an
/// **advisory** that names what is missing and what to do about it.
///
/// The advisory is not decoration. Codec availability on Windows is a runtime
/// fact (§1 of the research report): H.264/AAC in MP4 is inbox and always
/// there, while HEVC, VP9, AV1 and the WebM/Ogg family arrive as separately
/// installable Store packages that an enterprise image may have stripped. A
/// bare "MEDIA_ERR_SRC_NOT_SUPPORTED" leaves the developer guessing between
/// "my file is corrupt" and "this machine lacks a decoder"; the advisory says
/// which, and names the package or the re-encode.
#[derive(Debug, Clone)]
pub struct MediaError {
    pub code: MediaErrorCode,
    /// What went wrong, in the platform's own words where it has any.
    pub message: String,
    /// What the developer should DO. `None` when there is nothing useful to
    /// say beyond the message.
    pub advisory: Option<String>,
}

impl MediaError {
    pub fn new(code: MediaErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), advisory: None }
    }

    pub fn with_advisory(mut self, advisory: impl Into<String>) -> Self {
        self.advisory = Some(advisory.into());
        self
    }
}

/// `HTMLMediaElement.canPlayType()`'s three-valued answer. Deliberately the
/// same three values, because the reason the DOM has this odd tri-state — "I
/// will probably manage, ask me again when I have the bytes" — is the same
/// reason we do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanPlay {
    /// `""` — no.
    No,
    /// `"maybe"` — the container is recognised; the codecs were not stated.
    Maybe,
    /// `"probably"` — container and codecs are both supported.
    Probably,
}

impl CanPlay {
    /// The exact string `canPlayType()` returns.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::No => "",
            Self::Maybe => "maybe",
            Self::Probably => "probably",
        }
    }
}

/// The media events this crate emits, named after the HTML ones.
///
/// Every one of these is also an `MF_MEDIA_ENGINE_EVENT` below 1000 — Microsoft
/// documents that range as "events defined in HTML 5 for media elements", so on
/// Windows the OS raises them with the semantics the spec asks for and the
/// backend is a rename, not a state machine. On macOS the same set comes out of
/// KVO on `AVPlayerItem.status` plus `AVPlayerItemDidPlayToEndTime` plus a
/// periodic time observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaEvent {
    LoadStart,
    /// Metadata is known: `duration`, `videoWidth`/`videoHeight` are readable.
    LoadedMetadata,
    /// The first frame is available.
    LoadedData,
    CanPlay,
    CanPlayThrough,
    /// Playback was requested (`play()` / `autoplay`), before it actually runs.
    Play,
    /// Playback is actually running.
    Playing,
    Pause,
    Seeking,
    Seeked,
    /// Throttled to the spec's cadence — see [`crate::VideoState`].
    TimeUpdate,
    DurationChange,
    RateChange,
    VolumeChange,
    /// Stalled waiting for data.
    Waiting,
    Ended,
    Error,
    Emptied,
}

impl MediaEvent {
    /// The DOM event name, which is also the name the JS runtime registers.
    pub fn name(self) -> &'static str {
        match self {
            Self::LoadStart => "loadstart",
            Self::LoadedMetadata => "loadedmetadata",
            Self::LoadedData => "loadeddata",
            Self::CanPlay => "canplay",
            Self::CanPlayThrough => "canplaythrough",
            Self::Play => "play",
            Self::Playing => "playing",
            Self::Pause => "pause",
            Self::Seeking => "seeking",
            Self::Seeked => "seeked",
            Self::TimeUpdate => "timeupdate",
            Self::DurationChange => "durationchange",
            Self::RateChange => "ratechange",
            Self::VolumeChange => "volumechange",
            Self::Waiting => "waiting",
            Self::Ended => "ended",
            Self::Error => "error",
            Self::Emptied => "emptied",
        }
    }
}

// ---------------------------------------------------------------------------
// what to play, and what comes back
// ---------------------------------------------------------------------------

/// Where the media comes from. Resolved by the caller (the element layer runs
/// the `<img src>` URL grammar), so a backend never parses a URL.
#[derive(Debug, Clone)]
pub enum MediaSource {
    /// A local file. The backend turns it into whatever its platform wants —
    /// a `file://` URL for Media Foundation, an `AVURLAsset` on macOS.
    Path(PathBuf),
    /// An absolute `http(s)://` URL, handed to the platform verbatim.
    ///
    /// Both target platforms have a network source of their own that reads
    /// ahead, seeks by range request and never materialises the whole clip:
    /// Media Foundation's scheme handler and `AVURLAsset`. Downloading first
    /// would be strictly worse — it would turn a 30-minute stream into a
    /// 30-minute wait and a file the size of the clip.
    Uri(String),
    /// Bytes the caller already has — a `data:` URI.
    ///
    /// `mime` is the URI's media type, which is the only hint the platform gets
    /// about what the bytes are (there is no filename and no `Content-Type`).
    /// An empty string means the URI declared none, and the platform sniffs.
    Bytes {
        mime: String,
        data: Arc<Vec<u8>>,
    },
}

/// Compared to answer one question — "is this the source that is already
/// loaded?" — so [`MediaSource::Bytes`] compares by IDENTITY, not by content.
///
/// The element layer memoises the decode per `src` string, so two `Bytes` from
/// the same `data:` URI ARE the same `Arc`; two from different URIs are not,
/// whatever their contents. Comparing megabytes of payload to answer a question
/// the pointer already answers would be a per-load memcmp for nothing.
impl PartialEq for MediaSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Path(a), Self::Path(b)) => a == b,
            (Self::Uri(a), Self::Uri(b)) => a == b,
            (Self::Bytes { data: a, .. }, Self::Bytes { data: b, .. }) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for MediaSource {}

/// What the element wants of the next frame.
///
/// `max_size` is in **physical** pixels and is the box the element is actually
/// painted at. It is a cap, not a demand: the frame comes back at the video's
/// own aspect ratio, never larger than `max_size` and never larger than the
/// video's native size.
///
/// This is load-bearing rather than an optimisation. The v1 present path reads
/// the frame back to system memory (CONTRACT.md §5), so its cost is exactly
/// `width * height * 4` bytes per frame; asking the platform's video processor
/// — which is scaling anyway — for the size we are going to *draw* instead of
/// the size the file was *encoded* at is the difference between 8.3 MB and
/// 1.4 MB per frame for a 1080p clip in an 800x450 box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRequest {
    pub max_size: (u32, u32),
}

/// One decoded, presentable frame.
///
/// Two shapes, one type:
///
/// * **zero-copy** (`gpu: Some`) — the pixels are in VRAM and `image` is a
///   stable, zero-filled placeholder that exists only to give gpui's atlas a
///   key and a size. This is the Windows path.
/// * **CPU** (`gpu: None`) — `image` carries real BGRA bytes, which is what
///   `<img>`/`<lottie>` hand gpui. The fallback, the `stub` backend, and any
///   platform that has no GPU route yet.
pub struct VideoFrame {
    pub image: Arc<RenderImage>,
    /// Presentation timestamp in seconds, from the media clock — NOT the wall
    /// clock. The distinction matters: a `<video>` has a soundtrack and the
    /// OS owns the A/V master clock, so wall-clock extrapolation (which is
    /// right for `<lottie>`) would drift against the audio.
    pub pts: f64,
    pub size: (u32, u32),
    /// **Set when the pixels are NOT in `image`** — see the type docs. The
    /// host must hand it to `Window::upload_image_from_gpu` before the draw
    /// that paints `image`, or the element paints transparent.
    pub gpu: Option<GpuFrame>,
}

/// A decoded frame that is still in the renderer's own GPU memory.
///
/// The one place in this crate where the "name no platform type" rule bends,
/// and it bends the way gpui's own does: gpui carries a `CVPixelBuffer` in
/// `PaintSurface` under `#[cfg(target_os = "macos")]` for exactly this reason.
/// The handle is **opaque** here — a raw COM/Metal pointer, never a typed
/// platform object — so `backend.rs` still names no platform type, still
/// compiles with no platform crate in scope, and `AVPlayerItemVideoOutput`'s
/// `CVPixelBufferRef` fits the same field on macOS.
///
/// The texture is **borrowed**: it belongs to the backend, which reuses it in
/// place for the next frame. It is valid until the next
/// [`VideoBackend::present`] on the same backend, which is why the host has to
/// upload it in the tick it was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuFrame {
    /// An `ID3D11Texture2D*` on Windows. Opaque; see the type docs.
    pub texture: *mut std::ffi::c_void,
    /// The populated region, measured from the texture's origin.
    pub size: (u32, u32),
}

// ---------------------------------------------------------------------------
// the trait
// ---------------------------------------------------------------------------

/// One platform's media player, at `HTMLMediaElement` altitude.
///
/// # The contract
///
/// * **The backend owns the clock and the audio.** It is not asked to render
///   audio, to synchronise it, or to be ticked with a wall-clock delta. Both
///   target platforms already do this, correctly, in the OS.
/// * **Every getter is cheap and may be called per frame.** They are reads of
///   state the platform is maintaining anyway.
/// * **Every setter is idempotent.** The element layer pushes the whole prop
///   set every frame (that is how the tree works); a setter that re-fires an
///   event on an unchanged value would produce a `volumechange` storm.
/// * **[`present`](VideoBackend::present) is called only from the UI thread**,
///   inside the frame pump. It touches the graphics device.
/// * **[`drain_events`](VideoBackend::drain_events) never re-enters.** Platform
///   callbacks arrive on platform threads and must be queued, not dispatched:
///   a JS handler can mutate the tree, and the tree must not be borrowed from a
///   decoder thread. Collect first, dispatch after — the same discipline
///   `lottie::tick` documents.
pub trait VideoBackend {
    /// Begin loading. Corresponds to setting `src` + `load()`: it must be
    /// asynchronous, reporting progress through events, and must not block.
    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError>;

    fn play(&mut self);
    fn pause(&mut self);
    /// `currentTime = seconds`.
    fn seek(&mut self, seconds: f64);
    /// `0.0..=1.0`.
    fn set_volume(&mut self, volume: f64);
    fn set_muted(&mut self, muted: bool);
    fn set_rate(&mut self, rate: f64);
    fn set_looping(&mut self, looping: bool);

    fn current_time(&self) -> f64;
    /// `f64::NAN` while unknown, `f64::INFINITY` for a live stream — the
    /// spec's own values.
    fn duration(&self) -> f64;
    fn paused(&self) -> bool;
    fn ended(&self) -> bool;
    fn seeking(&self) -> bool;
    fn ready_state(&self) -> ReadyState;
    fn network_state(&self) -> NetworkState;
    /// `videoWidth` / `videoHeight`; `None` before metadata.
    fn native_size(&self) -> Option<(u32, u32)>;
    /// `buffered` as `(start, end)` pairs in seconds.
    fn buffered(&self) -> Vec<(f64, f64)>;
    /// `seekable` as `(start, end)` pairs in seconds.
    fn seekable(&self) -> Vec<(f64, f64)>;
    fn error(&self) -> Option<MediaError>;

    /// `canPlayType(type)` — a **live probe**, never a table.
    ///
    /// The owner constraint that produced this signature: what a machine can
    /// decode is not a build-time fact. H.264 + AAC in MP4 is the guaranteed
    /// inbox baseline; HEVC, VP9, AV1, VP8/Opus/Vorbis all arrive as Store
    /// packages that may or may not be installed, and macOS has a *different*
    /// set again. So the answer is asked of the platform, on this machine, now.
    fn can_play_type(&self, mime_type: &str) -> CanPlay;

    /// The current frame, if a new one has been produced since the last call.
    ///
    /// `None` means "nothing new" — the element keeps painting the previous
    /// frame, which is what a browser does between decoded frames.
    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame>;

    /// Hands over everything the platform has raised since the last call, in
    /// order. Must not call back into the tree.
    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent));

    /// Park: stop decoding and give back what can be given back cheaply, while
    /// keeping enough state that [`resume`](VideoBackend::resume) is not a
    /// reload. What a `<KeepAlive>`-parked page or an off-screen video does.
    ///
    /// **It must be reversible without re-running the load algorithm.** The
    /// first version of this tore the Media Foundation pipeline down entirely
    /// (`SetSource("")`) and rebuilt it on resume, which released ~20 MiB while
    /// parked and then leaked ~5 MiB per cycle and killed the process after
    /// about ten of them. Keeping the decoder is the cheaper trade: it costs
    /// resident memory that is bounded, instead of an unbounded one.
    ///
    /// The default is a pause, which is always correct.
    fn suspend(&mut self) {
        self.pause();
    }

    /// Unpark. The position and the loaded source survive [`suspend`], so the
    /// default — resume playing if the element wants to — is usually right.
    fn resume(&mut self) {}

    /// The host could not get the last [`VideoFrame::gpu`] texture into the
    /// renderer's atlas — wrong device, wrong format, or a renderer with no
    /// GPU upload path at all.
    ///
    /// A backend that produces [`GpuFrame`]s should stop producing them and
    /// fall back to filling `VideoFrame::image` with real bytes; one that never
    /// produced any ignores this, which is the default.
    ///
    /// It is a one-way door on purpose. The three reasons the atlas refuses are
    /// all properties of the machine or of a device generation, not of the
    /// frame, so retrying every frame would just log the same rejection 165
    /// times a second.
    fn gpu_upload_failed(&mut self) {}
}
