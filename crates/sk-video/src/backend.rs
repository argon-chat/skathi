//! The portable half: the types a `<video>` element is made of and the [`VideoBackend`] trait the
//! platform implements. Nothing here may name a platform type. The trait sits at `HTMLMediaElement`
//! altitude: OS objects that own the demuxer, decoder, audio renderer and master clock.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::RenderImage;

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

/// `MediaError.code`: the four the spec defines. A platform error that maps onto none is a `Decode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MediaErrorCode {
    Aborted = 1,
    Network = 2,
    Decode = 3,
    SrcNotSupported = 4,
}

/// `HTMLMediaElement.error`, plus an **advisory** naming what is missing and what to do: codec
/// availability on Windows is a runtime fact, and a bare "SRC_NOT_SUPPORTED" cannot tell a corrupt
/// file from a missing decoder.
#[derive(Debug, Clone)]
pub struct MediaError {
    pub code: MediaErrorCode,
    pub message: String,
    pub advisory: Option<String>,
}

impl MediaError {
    pub fn new(code: MediaErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            advisory: None,
        }
    }

    pub fn with_advisory(mut self, advisory: impl Into<String>) -> Self {
        self.advisory = Some(advisory.into());
        self
    }
}

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

/// The media events this crate emits, named after the HTML ones (each an `MF_MEDIA_ENGINE_EVENT` below 1000 on Windows).
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

#[derive(Debug, Clone)]
pub enum MediaSource {
    Path(PathBuf),
    /// An absolute `http(s)://` URL handed to the platform verbatim: its network source reads ahead and seeks by range.
    Uri(String),
    /// Bytes the caller already has (a `data:` URI). `mime` is the URI's media type, the platform's
    /// only hint; empty means none declared, so the platform sniffs.
    Bytes {
        mime: String,
        data: Arc<Vec<u8>>,
    },
    /// Immutable mapped package bytes. A backend reads/seeks ranges directly; it must not copy them.
    StaticBytes {
        mime: String,
        data: &'static [u8],
    },
}

/// Answers "is this the source already loaded?", so [`MediaSource::Bytes`] compares by identity (the
/// element layer memoises the decode per `src`).
impl PartialEq for MediaSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Path(a), Self::Path(b)) => a == b,
            (Self::Uri(a), Self::Uri(b)) => a == b,
            (Self::Bytes { data: a, .. }, Self::Bytes { data: b, .. }) => Arc::ptr_eq(a, b),
            (Self::StaticBytes { data: a, .. }, Self::StaticBytes { data: b, .. }) => {
                a.as_ptr() == b.as_ptr() && a.len() == b.len()
            }
            _ => false,
        }
    }
}

impl Eq for MediaSource {}

/// What the element wants of the next frame. `max_size` is in physical pixels: a cap on the painted
/// box, never larger than the video's native size, at its aspect ratio. It bounds the readback cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRequest {
    pub max_size: (u32, u32),
}

/// One decoded, presentable frame: zero-copy (`gpu: Some`; `image` is a zero-filled placeholder that
/// only gives gpui's atlas a key and size) or CPU (`gpu: None`; `image` carries real BGRA bytes).
pub struct VideoFrame {
    pub image: Arc<RenderImage>,
    /// Presentation timestamp in seconds from the media clock: the OS owns the A/V master clock, so
    /// wall-clock extrapolation would drift against the audio.
    pub pts: f64,
    pub size: (u32, u32),
    /// Set when the pixels are NOT in `image`: the host must pass it to `Window::upload_image_from_gpu`
    /// before the draw that paints `image`, or the element paints transparent.
    pub gpu: Option<GpuFrame>,
}

/// A decoded frame still in the renderer's GPU memory. The handle is opaque (a raw COM/Metal
/// pointer). The texture is borrowed: valid only until the next [`VideoBackend::present`], and the
/// host must upload it in the tick it was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuFrame {
    pub texture: *mut std::ffi::c_void,
    pub size: (u32, u32),
}

/// A [`GpuFrame`] holding a reference of its own to its texture, for a host that cannot upload in
/// the tick that made it (the backend may have released the borrowed texture by a later draw).
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
/// * The backend owns the clock and the audio.
/// * Getters are cheap and may be called per frame.
/// * Setters are idempotent: the element pushes the whole prop set every frame, and a setter that
///   re-fired on an unchanged value would cause a `volumechange` storm.
/// * [`present`](VideoBackend::present) is called only from the UI thread, inside the frame pump.
/// * [`drain_events`](VideoBackend::drain_events) never re-enters: platform callbacks are queued, not
///   dispatched, since a JS handler can mutate the tree.
pub trait VideoBackend {
    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError>;

    fn play(&mut self);
    fn pause(&mut self);
    fn seek(&mut self, seconds: f64);
    fn set_volume(&mut self, volume: f64);
    fn set_muted(&mut self, muted: bool);
    fn set_rate(&mut self, rate: f64);
    fn set_looping(&mut self, looping: bool);

    fn current_time(&self) -> f64;
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

    /// Is this element's sound going to the audio renderer right now, read off the platform. For tests.
    fn audible(&self) -> bool {
        false
    }

    /// `canPlayType(type)`: a live probe, never a table, since what a machine can decode is not a build-time fact.
    fn can_play_type(&self, mime_type: &str) -> CanPlay;

    /// The current frame, if a new one has been produced since the last call.
    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame>;

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent));

    /// Park: stop decoding and give back what can be given back cheaply, keeping enough state that
    /// [`resume`](VideoBackend::resume) is not a reload (tearing the pipeline down leaked ~5 MiB per
    /// park/resume cycle). The default is a pause.
    fn suspend(&mut self) {
        self.pause();
    }

    fn resume(&mut self) {}

    /// The host could not get the last [`VideoFrame::gpu`] texture into the renderer's atlas. A
    /// backend producing [`GpuFrame`]s should stop and fill `VideoFrame::image` with real bytes.
    /// One-way: the causes are properties of the machine. The default ignores it.
    fn gpu_upload_failed(&mut self) {}
}
