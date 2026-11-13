//! The portable half: the types a `<video>` element is made of and the [`VideoBackend`] trait the
//! platform implements.
//!
//! Nothing here may name a platform type. The trait sits at `HTMLMediaElement` altitude, where
//! `IMFMediaEngine` and `AVPlayer` already agree: OS objects that own the demuxer, decoder, audio
//! renderer and master clock.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::RenderImage;

/// `HTMLMediaElement.readyState`; the numbers are the spec's.
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

/// `MediaError.code`: the four the spec defines. A platform error that maps onto none is a
/// `Decode` with the platform's text in [`MediaError::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MediaErrorCode {
    Aborted = 1,
    Network = 2,
    Decode = 3,
    SrcNotSupported = 4,
}

/// `HTMLMediaElement.error`, plus an **advisory** naming what is missing and what to do: codec
/// availability on Windows is a runtime fact (HEVC, VP9, AV1 and WebM/Ogg are separately installable
/// Store packages), and a bare "SRC_NOT_SUPPORTED" cannot tell a corrupt file from a missing decoder.
#[derive(Debug, Clone)]
pub struct MediaError {
    pub code: MediaErrorCode,
    pub message: String,
    /// What the developer should do; `None` when the message says it all.
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

/// `HTMLMediaElement.canPlayType()`'s three-valued answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanPlay {
    No,
    Maybe,
    Probably,
}

impl CanPlay {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::No => "",
            Self::Maybe => "maybe",
            Self::Probably => "probably",
        }
    }
}

/// The media events this crate emits, named after the HTML ones. Each is an `MF_MEDIA_ENGINE_EVENT`
/// below 1000 (the HTML 5 range) on Windows, so that backend is a rename, not a state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaEvent {
    LoadStart,
    LoadedMetadata,
    LoadedData,
    CanPlay,
    CanPlayThrough,
    Play,
    Playing,
    Pause,
    Seeking,
    Seeked,
    TimeUpdate,
    DurationChange,
    RateChange,
    VolumeChange,
    Waiting,
    Ended,
    Error,
    Emptied,
}

impl MediaEvent {
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

/// Where the media comes from. Resolved by the caller, so a backend never parses a URL.
#[derive(Debug, Clone)]
pub enum MediaSource {
    /// A local file.
    Path(PathBuf),
    /// An absolute `http(s)://` URL handed to the platform verbatim: both platforms' network sources
    /// read ahead and seek by range, so downloading first would only add a wait.
    Uri(String),
    /// Bytes the caller already has (a `data:` URI). `mime` is the URI's media type, the platform's
    /// only hint about the bytes; empty means none declared, so the platform sniffs.
    Bytes {
        mime: String,
        data: Arc<Vec<u8>>,
    },
    /// Immutable mapped package bytes. A backend reads/seeks ranges directly; it must not copy the
    /// asset or stage it to a temporary file.
    StaticBytes { mime: String, data: &'static [u8] },
}

/// Answers "is this the source already loaded?", so [`MediaSource::Bytes`] compares by identity:
/// the element layer memoises the decode per `src`, so the same `data:` URI is the same `Arc`.
impl PartialEq for MediaSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Path(a), Self::Path(b)) => a == b,
            (Self::Uri(a), Self::Uri(b)) => a == b,
            (Self::Bytes { data: a, .. }, Self::Bytes { data: b, .. }) => Arc::ptr_eq(a, b),
            (Self::StaticBytes { data: a, .. }, Self::StaticBytes { data: b, .. }) =>
                a.as_ptr() == b.as_ptr() && a.len() == b.len(),
            _ => false,
        }
    }
}

impl Eq for MediaSource {}

/// What the element wants of the next frame. `max_size` is in physical pixels, the box the element
/// is painted at: a cap, never larger than the video's native size, at the video's own aspect ratio.
/// It bounds the readback cost (`width * height * 4` bytes per frame) to the drawn size rather than
/// the encoded one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRequest {
    pub max_size: (u32, u32),
}

/// One decoded, presentable frame, in one of two shapes: zero-copy (`gpu: Some`, the Windows path;
/// `image` is a zero-filled placeholder that only gives gpui's atlas a key and size) or CPU
/// (`gpu: None`; `image` carries real BGRA bytes).
pub struct VideoFrame {
    pub image: Arc<RenderImage>,
    /// Presentation timestamp in seconds from the media clock, not the wall clock: the OS owns the
    /// A/V master clock, so wall-clock extrapolation would drift against the audio.
    pub pts: f64,
    pub size: (u32, u32),
    /// Set when the pixels are NOT in `image`. The host must pass it to `Window::upload_image_from_gpu`
    /// before the draw that paints `image`, or the element paints transparent.
    pub gpu: Option<GpuFrame>,
}

/// A decoded frame still in the renderer's own GPU memory. The handle is opaque (a raw COM/Metal
/// pointer), so this file still names no platform type. The texture is borrowed: the backend reuses
/// it in place, so it is valid only until the next [`VideoBackend::present`], and the host must
/// upload it in the tick it was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuFrame {
    pub texture: *mut std::ffi::c_void,
    pub size: (u32, u32),
}

/// A [`GpuFrame`] holding a reference of its own to its texture, for a host that cannot upload in
/// the tick that made it (a hidden pop-out draws later, by which time the backend may have released
/// the borrowed texture: uploading from it read freed memory). A late upload shows an old picture at worst.
#[derive(Debug)]
pub struct HeldGpuFrame {
    frame: GpuFrame,
}

impl HeldGpuFrame {
    pub fn new(frame: GpuFrame) -> Self {
        #[cfg(windows)]
        // SAFETY: a `GpuFrame` is valid in the tick it was produced, which is when the host wraps it.
        unsafe {
            crate::mf::retain_texture(frame.texture)
        };
        Self { frame }
    }

    pub fn frame(&self) -> GpuFrame {
        self.frame
    }
}

impl Drop for HeldGpuFrame {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: the reference `new` took.
        unsafe {
            crate::mf::release_texture(self.frame.texture)
        };
    }
}

/// One platform's media player, at `HTMLMediaElement` altitude.
///
/// * The backend owns the clock and the audio.
/// * Getters are cheap and may be called per frame.
/// * Setters are idempotent: the element pushes the whole prop set every frame, and a setter that
///   re-fired on an unchanged value would cause a `volumechange` storm.
/// * [`present`](VideoBackend::present) is called only from the UI thread, inside the frame pump.
/// * [`drain_events`](VideoBackend::drain_events) never re-enters: platform callbacks are queued, not
///   dispatched, since a JS handler can mutate the tree.
pub trait VideoBackend {
    /// Begin loading (`src` + `load()`); must be asynchronous and must not block.
    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError>;

    fn play(&mut self);
    fn pause(&mut self);
    fn seek(&mut self, seconds: f64);
    fn set_volume(&mut self, volume: f64);
    fn set_muted(&mut self, muted: bool);
    fn set_rate(&mut self, rate: f64);
    fn set_looping(&mut self, looping: bool);

    fn current_time(&self) -> f64;
    /// `f64::NAN` while unknown, `f64::INFINITY` for a live stream.
    fn duration(&self) -> f64;
    fn paused(&self) -> bool;
    fn ended(&self) -> bool;
    fn seeking(&self) -> bool;
    fn ready_state(&self) -> ReadyState;
    fn network_state(&self) -> NetworkState;
    fn native_size(&self) -> Option<(u32, u32)>;
    fn buffered(&self) -> Vec<(f64, f64)>;
    fn seekable(&self) -> Vec<(f64, f64)>;
    fn error(&self) -> Option<MediaError>;

    /// Is this element's sound going to the audio renderer right now (has audio, plays, not muted,
    /// volume above zero), read off the platform. For tests; the default is no sound.
    fn audible(&self) -> bool {
        false
    }

    /// `canPlayType(type)`: a live probe, never a table, since what a machine can decode is not a
    /// build-time fact.
    fn can_play_type(&self, mime_type: &str) -> CanPlay;

    /// The current frame, if a new one has been produced since the last call; `None` keeps the
    /// previous frame painted.
    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame>;

    /// Hands over everything the platform has raised since the last call, in order.
    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent));

    /// Park: stop decoding and give back what can be given back cheaply, while keeping enough state
    /// that [`resume`](VideoBackend::resume) is not a reload. Must be reversible without re-running
    /// the load algorithm: tearing the pipeline down leaked ~5 MiB per park/resume cycle. The
    /// default is a pause.
    fn suspend(&mut self) {
        self.pause();
    }

    /// Unpark. Position and source survive [`suspend`]; the default resumes playing if wanted.
    fn resume(&mut self) {}

    /// The host could not get the last [`VideoFrame::gpu`] texture into the renderer's atlas. A
    /// backend producing [`GpuFrame`]s should stop and fill `VideoFrame::image` with real bytes; the
    /// default ignores it. One-way: the causes are properties of the machine, so retrying every
    /// frame would only repeat the rejection.
    fn gpu_upload_failed(&mut self) {}
}
