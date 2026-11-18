//! `<sk-surface name="…">`: a picture native code produces and a Skathi element shows, composited
//! like any other content (object-fit, clipping, radii, transforms, z-order).
//!
//! A producer gets a [`SurfaceHandle`] by name ([`surface`]) and presents frames from any thread.
//! The latest frame wins; an element picks it up on its next paint, and only while it is visible.
//! On Windows a frame can be a D3D11 or D3D12 texture on the renderer's own device (GPU copy, no
//! readback) or a shared texture from another device, API or process, synchronised by a keyed
//! mutex or a fence. On macOS it can be a Metal texture on the renderer's device, an IOSurface
//! (synchronised by an `MTLSharedEvent`) or a CVPixelBuffer. Everywhere it can be CPU BGRA.
//!
//! A surface set to [`SurfaceFormat::Rgba16Float`] takes half-float frames instead; they stay
//! half-float until an element composites them: an HDR window (Windows, macOS, on an HDR display)
//! shows them up to the element's `dynamic-range-limit`, a standard one maps them to 8-bit sRGB.

mod color;
#[cfg(windows)]
mod d3d11;
#[cfg(windows)]
mod d3d12;
mod display;
pub(crate) mod feedback;
#[cfg(target_os = "macos")]
mod metal;
pub(crate) mod node;
#[cfg(windows)]
mod shared;

#[cfg(windows)]
pub use d3d12::{D3D12Lease, D3D12Provider};
pub use feedback::monotonic_ns;

/// Under the D3D12 renderer, how surfaces reach its device and queue. Set once by the host.
#[cfg(windows)]
pub fn install_d3d12_provider(provider: D3D12Provider) {
    d3d12::install(provider);
}

#[cfg(target_os = "macos")]
const NO_D3D: &str = "Direct3D is Windows-only; on macOS present a Metal texture \
                      (present_metal_texture), an IOSurface (SharedTexture::from_iosurface) or a \
                      CVPixelBuffer (present_cv_pixel_buffer)";
#[cfg(windows)]
const NO_METAL: &str = "Metal is macOS-only; on Windows present a D3D11 or D3D12 texture, or a \
                        SharedTexture opened from an NT handle";
#[cfg(not(any(windows, target_os = "macos")))]
const NO_D3D: &str =
    "GPU surfaces are Windows and macOS only for now; present CPU frames with present_bgra";
#[cfg(not(any(windows, target_os = "macos")))]
const NO_METAL: &str = NO_D3D;

use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

/// The largest frame side accepted, the renderer's texture limit.
const MAX_SIDE: u32 = 16384;

// Producers present from their own threads.
const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    const fn send<T: Send>() {}
    send_sync::<SurfaceHandle>();
    send_sync::<D3D11Device>();
    send_sync::<D3D12Device>();
    send_sync::<D3D12Fence>();
    send_sync::<MetalDevice>();
    send_sync::<MetalEvent>();
    send::<SharedTexture>();
};

/// The surface named `name`, created on first use. Every handle and element with that name shares it.
pub fn surface(name: &str) -> SurfaceHandle {
    let mut map = lock(registry());
    let shared = map
        .entry(name.to_string())
        .or_insert_with(|| {
            ANY.store(true, Ordering::Relaxed);
            let shared = Arc::new(Shared {
                name: name.into(),
                state: Mutex::new(State::default()),
                watchers: Mutex::new(Vec::new()),
                frames: Mutex::new(Vec::new()),
            });
            lock(&shared.state).key = feedback::key(&shared);
            shared
        })
        .clone();
    SurfaceHandle { shared }
}

/// Why a surface call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceError {
    /// This platform or renderer has no such path; the message says which one to use.
    /// [`SurfaceHandle::present_bgra`] works everywhere.
    NotSupported(&'static str),
    /// The renderer has no device yet (no window has drawn) or could not make one.
    NoDevice(String),
    /// The renderer recreated its device: ask for the device again and recreate textures (or
    /// reopen shared ones) on it.
    DeviceChanged,
    /// The texture was made on another device; open it as a [`SharedTexture`] instead.
    WrongDevice,
    /// A pixel format the renderer cannot take; the message names it.
    Format(String),
    /// A zero or oversized frame, or a buffer too short for it.
    Size(String),
    /// The keyed mutex was not released to the acquire key, or the fence not signalled, in time;
    /// nothing was presented.
    Timeout,
    /// Every texture of this surface is still being copied or shown; nothing was presented.
    Busy,
    /// The platform call failed; the message carries its error.
    Platform(String),
    /// The frame's format is not the one the surface is set to ([`SurfaceHandle::set_format`]).
    FormatMismatch {
        surface: SurfaceFormat,
        frame: SurfaceFormat,
    },
}

/// The pixels a surface takes, set with [`SurfaceHandle::set_format`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SurfaceFormat {
    /// 8-bit BGRA, sRGB-encoded, straight alpha: `B8G8R8A8_UNORM`, `BGRA8Unorm`, `'BGRA'`, and
    /// the 4:2:0 CVPixelBuffers.
    #[default]
    Bgra8,
    /// Half-float RGBA in linear extended sRGB (scRGB primaries, 1.0 = SDR white), straight alpha:
    /// `R16G16B16A16_FLOAT`, `RGBA16Float`, `'RGhA'`. Shown with values above 1.0 clamped.
    Rgba16Float,
}

impl fmt::Display for SurfaceFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bgra8 => "Bgra8",
            Self::Rgba16Float => "Rgba16Float",
        })
    }
}

fn check_format(surface: SurfaceFormat, frame: SurfaceFormat) -> Result<(), SurfaceError> {
    if surface == frame {
        Ok(())
    } else {
        Err(SurfaceError::FormatMismatch { surface, frame })
    }
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSupported(why) => f.write_str(why),
            Self::NoDevice(why) => write!(f, "the renderer has no GPU device: {why}"),
            Self::DeviceChanged => f.write_str(
                "the renderer recreated its device; get it again (d3d11_device, d3d12_device, \
                 metal_device) and recreate the textures",
            ),
            Self::WrongDevice => f.write_str(
                "the texture is not on the renderer's device (d3d11_device, d3d12_device, \
                 metal_device); share it (CreateSharedHandle, or an IOSurface on macOS) and \
                 present it as a SharedTexture",
            ),
            Self::Format(why) | Self::Size(why) => f.write_str(why),
            Self::Timeout => f.write_str(
                "the producer did not release the keyed mutex to the acquire key, or signal the \
                 fence, in time",
            ),
            Self::Busy => f.write_str(
                "every texture of this surface is still in use: the producer presents faster than \
                 the window draws, or a fence it waits on is never signalled; drop this frame",
            ),
            Self::Platform(why) => f.write_str(why),
            Self::FormatMismatch { surface, frame } => write!(
                f,
                "a {frame} frame on a surface set to {surface}: present {surface} frames, or call \
                 set_format(SurfaceFormat::{frame}) first"
            ),
        }
    }
}

impl std::error::Error for SurfaceError {}

/// Where the elements showing a surface are, for a producer that renders at the painted size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceInfo {
    /// The largest element's content box in physical pixels; 0 while no element shows the surface.
    pub width: u32,
    pub height: u32,
    /// That element's window scale factor.
    pub scale: f32,
    /// Whether any element showing it can be on screen. A hidden one takes no frames.
    pub visible: bool,
    /// The display that element is on; the last one known while no element shows the surface.
    pub display: SurfaceDisplay,
    /// That element's computed `dynamic-range-limit`.
    pub dynamic_range_limit: sk_css::DynamicRangeLimit,
}

impl SurfaceInfo {
    /// The brightest value a frame should reach, in [`SurfaceDisplay`]'s unit (1.0 is SDR white):
    /// [`SurfaceDisplay::output_headroom`] as `dynamic-range-limit` allows it, for a producer that
    /// tone-maps itself, such as a video player's target peak.
    pub fn content_headroom(&self) -> f32 {
        self.dynamic_range_limit
            .headroom(self.display.output_headroom)
    }
}

/// A display, in a surface's unit: 1.0 is SDR (paper) white, as in macOS EDR and as Windows
/// composes SDR content, so scRGB is this scaled by [`Self::sdr_white_nits`] / 80.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceDisplay {
    /// Whether the platform reported it: not on Linux, and not before the window first drew.
    pub known: bool,
    /// The display is in HDR: Windows' HDR (PQ) mode, or a macOS screen with EDR headroom.
    pub hdr: bool,
    /// The brightest value it shows now; 1.0 on an SDR display. On macOS it follows the brightness,
    /// and is 1.0 until a window on the screen asks for EDR.
    pub headroom: f32,
    /// The brightest it can show (macOS at full brightness); `headroom` on Windows.
    pub potential_headroom: f32,
    /// The brightest value a surface's frame reaches the display with: `headroom` where the
    /// window can draw HDR (Windows, macOS), else 1.0, where frames are clamped.
    pub output_headroom: f32,
    /// Whether that element's window draws HDR now (a half-float swap chain or layer): while it
    /// shows a half-float surface that may be brighter than SDR white, and two seconds after.
    pub output_hdr: bool,
    /// SDR white in nits: Windows in HDR mode (its SDR content brightness setting).
    pub sdr_white_nits: Option<f32>,
    /// The display's peak and full-frame luminance in nits, where Windows knows them.
    pub peak_nits: Option<f32>,
    pub full_frame_nits: Option<f32>,
    /// The refresh rate in Hz: Windows' current mode, the screen's maximum on macOS.
    pub refresh_hz: Option<f32>,
    /// Whether the screen refreshes at a variable rate (macOS: ProMotion, Adaptive-Sync).
    pub variable_refresh: Option<bool>,
    /// The display's nominal refresh period: the compositor's on Windows, else from `refresh_hz`.
    pub refresh_period: Option<Duration>,
    /// When its latest vblank was, in [`monotonic_ns`]'s clock, as `info()` was called: never after
    /// that, and at most `refresh_period` before. Off Windows it is extrapolated from the last
    /// frame shown, so it is known once one was.
    pub last_vblank: Option<u64>,
}

impl Default for SurfaceDisplay {
    fn default() -> Self {
        Self {
            known: false,
            hdr: false,
            headroom: 1.0,
            potential_headroom: 1.0,
            output_headroom: 1.0,
            output_hdr: false,
            sdr_white_nits: None,
            peak_nits: None,
            full_frame_nits: None,
            refresh_hz: None,
            variable_refresh: None,
            refresh_period: None,
            last_vblank: None,
        }
    }
}

impl SurfaceDisplay {
    /// A surface value in nits, where the SDR white level is known.
    pub fn nits(&self, value: f32) -> Option<f32> {
        self.sdr_white_nits.map(|white| value * white)
    }

    /// Whether `other` differs enough to report: a flag, or a level by more than 1 %.
    fn differs(&self, other: &SurfaceDisplay) -> bool {
        let level = |a: f32, b: f32| (a - b).abs() > 0.01 * a.abs().max(b.abs()).max(1.0);
        let maybe = |a: Option<f32>, b: Option<f32>| match (a, b) {
            (Some(a), Some(b)) => level(a, b),
            (a, b) => a.is_some() != b.is_some(),
        };
        self.known != other.known
            || self.hdr != other.hdr
            || self.output_hdr != other.output_hdr
            || self.variable_refresh != other.variable_refresh
            || level(self.headroom, other.headroom)
            || level(self.potential_headroom, other.potential_headroom)
            || level(self.output_headroom, other.output_headroom)
            || maybe(self.sdr_white_nits, other.sdr_white_nits)
            || maybe(self.peak_nits, other.peak_nits)
            || maybe(self.full_frame_nits, other.full_frame_nits)
            || maybe(self.refresh_hz, other.refresh_hz)
    }
}

/// What [`SurfaceHandle::on_change`] reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceEvent {
    /// The size, scale or visibility changed.
    Changed(SurfaceInfo),
    /// The renderer recreated its device; the frame is gone and GPU resources must be recreated.
    DeviceLost,
    /// The display became known, changed (headroom, SDR white, refresh rate), the window switched
    /// to or from HDR output, or the largest element moved to another one.
    DisplayChanged(SurfaceDisplay),
}

/// Frame counters, for a producer that wants to know whether its frames are seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceStats {
    pub presented: u64,
    /// Frames that reached at least one element. A frame replaced before a visible element painted
    /// it is not counted.
    pub shown: u64,
}

/// What became of a presented frame, reported once per frame in frame order
/// ([`SurfaceHandle::on_frame`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameEvent {
    /// The frame reached the screen, on the refresh that began at `shown_at` ([`monotonic_ns`]'s
    /// clock). `refresh_interval` is that refresh's distance from the one before.
    Presented {
        frame: u64,
        shown_at: u64,
        refresh_interval: Duration,
        precision: FramePrecision,
    },
    /// The frame never reached the screen.
    Dropped { frame: u64, reason: FrameDropReason },
}

impl FrameEvent {
    /// The frame id its present returned.
    pub fn frame(&self) -> u64 {
        match *self {
            Self::Presented { frame, .. } | Self::Dropped { frame, .. } => frame,
        }
    }

    fn dropped(frame: u64, reason: FrameDropReason) -> Self {
        Self::Dropped { frame, reason }
    }
}

/// How [`FrameEvent::Presented`]'s `shown_at` was known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePrecision {
    /// Read from the display's own timing.
    Exact,
    /// Inferred, within a refresh either way.
    Estimated,
}

/// Why a frame never reached the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameDropReason {
    /// A newer frame replaced it first.
    Superseded,
    /// No element showing the surface could be on screen (`info().visible` false, or the window
    /// minimised, covered or hidden).
    Hidden,
    /// The renderer lost its device.
    DeviceLost,
    /// The window showing it closed, or the app quit.
    Closed,
}

/// What [`SurfaceHandle::present_d3d12`] and [`SurfaceHandle::present_metal_texture`] return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePresent {
    /// The frame's id, as [`FrameEvent`] names it.
    pub frame: u64,
    /// The value the release fence or event reaches once the producer's texture is free.
    pub release: u64,
}

type FrameCallback = Arc<Mutex<Box<dyn FnMut(FrameEvent) + Send>>>;

/// A named surface. Cheap to clone, `Send + Sync`; present from any thread.
#[derive(Clone)]
pub struct SurfaceHandle {
    shared: Arc<Shared>,
}

impl fmt::Debug for SurfaceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SurfaceHandle")
            .field("name", &self.shared.name)
            .finish()
    }
}

type Watcher = Arc<dyn Fn(&SurfaceHandle, SurfaceEvent) + Send + Sync>;

pub(crate) struct Shared {
    name: Arc<str>,
    state: Mutex<State>,
    watchers: Mutex<Vec<(u64, Watcher)>>,
    frames: Mutex<Vec<(u64, FrameCallback)>>,
}

impl Shared {
    fn frame_event(&self, event: FrameEvent) {
        let callbacks: Vec<FrameCallback> =
            lock(&self.frames).iter().map(|(_, f)| f.clone()).collect();
        for callback in callbacks {
            (lock(&callback))(event);
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) seq: u64,
    pub(crate) frame: Option<Frame>,
    /// The id of `frame`, and of the last frame presented.
    pub(crate) frame_id: u64,
    key: feedback::Key,
    /// Whether anything listens to [`SurfaceHandle::on_frame`].
    watched: bool,
    /// Why the frame went, when it did: the element's `surfacelost` reason.
    pub(crate) lost: &'static str,
    presented: u64,
    shown: u64,
    shown_seq: u64,
    /// The newest frame present feedback was told is ready to be shown.
    settled: u64,
    info: SurfaceInfo,
    pub(crate) format: SurfaceFormat,
    #[cfg(windows)]
    gpu: d3d11::SurfaceGpu,
    #[cfg(windows)]
    gpu12: d3d12::SurfaceGpu,
    #[cfg(target_os = "macos")]
    metal: metal::SurfaceGpu,
}

#[derive(Clone)]
pub(crate) enum Frame {
    /// BGRA, straight alpha, rows packed; a half-float frame's own values too while a window can
    /// draw HDR.
    Cpu {
        width: u32,
        height: u32,
        bgra: Arc<[u8]>,
        halves: Option<Arc<[u16]>>,
    },
    /// In the surface's own texture on the renderer's device.
    #[cfg(windows)]
    Gpu { width: u32, height: u32 },
    /// In one of the surface's D3D12 textures, once the copy into it has run.
    #[cfg(windows)]
    D3D12 { width: u32, height: u32 },
    /// In one of the surface's Metal textures, once the copy into it has run.
    #[cfg(target_os = "macos")]
    Metal { width: u32, height: u32 },
}

impl Frame {
    pub(crate) fn size(&self) -> (u32, u32) {
        match self {
            Frame::Cpu { width, height, .. } => (*width, *height),
            #[cfg(windows)]
            Frame::Gpu { width, height } | Frame::D3D12 { width, height } => (*width, *height),
            #[cfg(target_os = "macos")]
            Frame::Metal { width, height } => (*width, *height),
        }
    }
}

impl State {
    fn put(&mut self, frame: Frame) -> u64 {
        self.seq += 1;
        self.presented += 1;
        self.frame = Some(frame);
        self.lost = "";
        self.frame_id += 1;
        if self.watched {
            let (key, id, visible) = (self.key, self.frame_id, self.info.visible);
            feedback::with(|t| t.put(key, id, visible));
        }
        let id = self.frame_id;
        let copied_later = match &self.frame {
            #[cfg(windows)]
            Some(Frame::D3D12 { .. }) => true,
            #[cfg(target_os = "macos")]
            Some(Frame::Metal { .. }) => true,
            _ => false,
        };
        if !copied_later {
            self.settle(id);
        }
        id
    }

    pub(crate) fn note_shown(&mut self, seq: u64) {
        if seq > self.shown_seq {
            self.shown_seq = seq;
            self.shown += 1;
        }
    }

    /// Frame `frame` is ready to be shown, so no older one will be.
    pub(crate) fn settle(&mut self, frame: u64) {
        if frame > self.settled {
            self.settled = frame;
            if self.watched {
                let key = self.key;
                feedback::with(|t| t.settle(key, frame));
            }
        }
    }

    /// The tick is writing `frame`; called under the lock it read the frame with.
    pub(crate) fn pin(&mut self, frame: u64) {
        if self.watched {
            let key = self.key;
            feedback::with(|t| t.pin(key, frame));
        }
    }

    /// The texture the atlas copies for a [`Frame::Gpu`], referenced for the caller; with `hdr`, a
    /// half-float frame's own texture instead of its 8-bit mapping.
    #[cfg(windows)]
    pub(crate) fn gpu_texture(
        &mut self,
        hdr: bool,
    ) -> Result<Option<d3d11::Texture>, SurfaceError> {
        self.gpu.texture(hdr)
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Shared>>> {
    static REGISTRY: std::sync::OnceLock<Mutex<HashMap<String, Arc<Shared>>>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

static ANY: AtomicBool = AtomicBool::new(false);

/// Whether `key`'s window can draw HDR on its display now: `dynamic-range: high`.
pub(crate) fn dynamic_range_high(key: crate::windows::WinKey) -> bool {
    display::dynamic_range_high(key)
}

pub(crate) fn any() -> bool {
    ANY.load(Ordering::Relaxed)
}

pub(crate) fn all() -> Vec<SurfaceHandle> {
    lock(registry())
        .values()
        .map(|shared| SurfaceHandle {
            shared: shared.clone(),
        })
        .collect()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl SurfaceHandle {
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    pub fn info(&self) -> SurfaceInfo {
        let mut info = self.state().info;
        info.display = self.with_clock(info.display);
        info
    }

    fn with_clock(&self, mut display: SurfaceDisplay) -> SurfaceDisplay {
        if display.known {
            (display.last_vblank, display.refresh_period) =
                feedback::display_clock(feedback::key(&self.shared), display.refresh_hz);
        }
        display
    }

    /// Calls `f` with what became of each frame presented from now on: exactly one
    /// [`FrameEvent`] per frame id, in frame order, until the returned guard drops. `f` runs on
    /// Skathi's vsync thread (on macOS a thread of its own), not the UI thread; keep it short.
    pub fn on_frame(&self, f: impl FnMut(FrameEvent) + Send + 'static) -> FrameWatch {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        lock(&self.shared.frames).push((id, Arc::new(Mutex::new(Box::new(f)))));
        let mut st = self.state();
        if !st.watched {
            st.watched = true;
            feedback::watch(&self.shared);
        }
        FrameWatch {
            shared: Arc::downgrade(&self.shared),
            id,
        }
    }

    pub fn stats(&self) -> SurfaceStats {
        let st = self.state();
        SurfaceStats {
            presented: st.presented,
            shown: st.shown,
        }
    }

    /// Calls `f` on the UI thread when [`SurfaceInfo`] changes or the device is lost, until the
    /// returned guard drops. Keep it short; it may present.
    pub fn on_change(
        &self,
        f: impl Fn(&SurfaceHandle, SurfaceEvent) + Send + Sync + 'static,
    ) -> SurfaceWatch {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        lock(&self.shared.watchers).push((id, Arc::new(f)));
        SurfaceWatch {
            shared: Arc::downgrade(&self.shared),
            id,
        }
    }

    /// Sets the pixel format presents must have; [`SurfaceFormat::Bgra8`] until set. The frame
    /// already shown stays until the next present.
    pub fn set_format(&self, format: SurfaceFormat) {
        self.state().format = format;
    }

    pub fn format(&self) -> SurfaceFormat {
        self.state().format
    }

    /// Presents a CPU frame: BGRA with straight alpha, `stride` bytes per row. Works on every
    /// platform. Every present returns the frame's id ([`Self::on_frame`]).
    pub fn present_bgra(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        stride: usize,
    ) -> Result<u64, SurfaceError> {
        self.present_cpu(pixels, width, height, stride, false)
    }

    /// Presents a CPU frame to a [`SurfaceFormat::Rgba16Float`] surface: RGBA half floats (IEEE
    /// binary16 bits), straight alpha, `stride` values per row. It is mapped to 8-bit sRGB here,
    /// and kept as it is too while a window can draw HDR.
    pub fn present_rgba16f(
        &self,
        pixels: &[u16],
        width: u32,
        height: u32,
        stride: usize,
    ) -> Result<u64, SurfaceError> {
        check_format(self.format(), SurfaceFormat::Rgba16Float)?;
        check_size(width, height)?;
        let row = width as usize * 4;
        let need = stride * (height as usize - 1) + row;
        if stride < row || pixels.len() < need {
            return Err(SurfaceError::Size(format!(
                "a {width}x{height} half-float frame with stride {stride} needs {need} values and \
                 a stride of at least {row}; got {}",
                pixels.len()
            )));
        }
        let bgra = color::rgba16f_to_bgra8(pixels, width, height, stride);
        let halves = display::any_capable().then(|| {
            (0..height as usize)
                .flat_map(|y| &pixels[y * stride..y * stride + row])
                .copied()
                .collect()
        });
        let frame = self.state().put(Frame::Cpu {
            width,
            height,
            bgra: bgra.into(),
            halves,
        });
        sk_runtime::frames::wake();
        Ok(frame)
    }

    /// [`Self::present_bgra`] for BGRX: the fourth byte is ignored and the frame is opaque.
    pub fn present_bgrx(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        stride: usize,
    ) -> Result<u64, SurfaceError> {
        self.present_cpu(pixels, width, height, stride, true)
    }

    fn present_cpu(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        stride: usize,
        opaque: bool,
    ) -> Result<u64, SurfaceError> {
        check_format(self.format(), SurfaceFormat::Bgra8)?;
        check_size(width, height)?;
        let row = width as usize * 4;
        let need = stride * (height as usize - 1) + row;
        if stride < row || pixels.len() < need {
            return Err(SurfaceError::Size(format!(
                "a {width}x{height} BGRA frame with stride {stride} needs {need} bytes and a stride \
                 of at least {row}; got {} bytes",
                pixels.len()
            )));
        }
        let mut packed = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            packed.extend_from_slice(&pixels[y * stride..y * stride + row]);
        }
        if opaque {
            for alpha in packed.iter_mut().skip(3).step_by(4) {
                *alpha = 255;
            }
        }
        let frame = self.state().put(Frame::Cpu {
            width,
            height,
            bgra: packed.into(),
            halves: None,
        });
        sk_runtime::frames::wake();
        Ok(frame)
    }

    /// Withdraws the frame: elements paint nothing and fire `surfacelost` with reason `"cleared"`.
    pub fn clear(&self) {
        let mut st = self.state();
        if st.frame.take().is_some() {
            st.seq += 1;
            st.lost = "cleared";
            #[cfg(windows)]
            st.gpu12.withdraw();
            #[cfg(target_os = "macos")]
            st.metal.withdraw();
            if st.watched {
                let key = st.key;
                feedback::with(|t| t.cleared(key));
            }
        }
        drop(st);
        sk_runtime::frames::wake();
    }

    /// The renderer's D3D11 device. Textures for [`Self::present_d3d11_texture`] are made on it.
    pub fn d3d11_device(&self) -> Result<D3D11Device, SurfaceError> {
        #[cfg(windows)]
        {
            d3d11::device(&self.shared.state).map(|inner| D3D11Device { inner })
        }
        #[cfg(not(windows))]
        {
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// Presents an `ID3D11Texture2D` made on [`Self::d3d11_device`]: `B8G8R8A8_UNORM`, `_UNORM_SRGB`
    /// or `_TYPELESS` (`R16G16B16A16_FLOAT` on a [`SurfaceFormat::Rgba16Float`] surface), straight
    /// alpha, mip 0, any sample count. The texture is copied on the GPU before this returns, so the
    /// producer may render its next frame into it at once.
    ///
    /// # Safety
    /// `texture` is a live `ID3D11Texture2D` COM pointer, or null.
    pub unsafe fn present_d3d11_texture(&self, texture: *mut c_void) -> Result<u64, SurfaceError> {
        #[cfg(windows)]
        {
            // SAFETY: the caller's.
            let frame = unsafe { d3d11::present_texture(&self.shared.state, texture) }?;
            sk_runtime::frames::wake();
            Ok(frame)
        }
        #[cfg(not(windows))]
        {
            let _ = texture;
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// The renderer's D3D12 device and direct queue, under the D3D12 renderer.
    pub fn d3d12_device(&self) -> Result<D3D12Device, SurfaceError> {
        #[cfg(windows)]
        {
            d3d12::device(&self.shared.state).map(|inner| D3D12Device { inner })
        }
        #[cfg(not(windows))]
        {
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// This surface's fence, which [`Self::present_d3d12`] signals once it has copied a frame out
    /// of the producer's resource.
    pub fn release_fence(&self) -> Result<D3D12Fence, SurfaceError> {
        #[cfg(windows)]
        {
            d3d12::release_fence(&self.shared.state).map(|inner| D3D12Fence { inner })
        }
        #[cfg(not(windows))]
        {
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// Presents an `ID3D12Resource` made on [`Self::d3d12_device`]: a 2D `B8G8R8A8_UNORM` (or
    /// `_UNORM_SRGB`, `_TYPELESS`; `R16G16B16A16_FLOAT` on a [`SurfaceFormat::Rgba16Float`] surface)
    /// texture, single-sampled, straight alpha, in `resource_state` (a
    /// `D3D12_RESOURCE_STATES`, left as it is). The copy runs on a queue of the surface's own after
    /// a GPU wait for `ready` (fence, value); without `ready` the resource must already be written.
    /// Nothing blocks. Returns the frame's id and the value [`Self::release_fence`] reaches once the
    /// resource is free to reuse.
    ///
    /// # Safety
    /// `resource` is null or a live `ID3D12Resource`; `ready`'s fence is a live `ID3D12Fence`.
    pub unsafe fn present_d3d12(
        &self,
        resource: *mut c_void,
        resource_state: i32,
        ready: Option<(*mut c_void, u64)>,
    ) -> Result<FramePresent, SurfaceError> {
        #[cfg(windows)]
        {
            // SAFETY: the caller's.
            let (frame, release) =
                unsafe { d3d12::present(&self.shared.state, resource, resource_state, ready) }?;
            sk_runtime::frames::wake();
            Ok(FramePresent { frame, release })
        }
        #[cfg(not(windows))]
        {
            let _ = (resource, resource_state, ready);
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// Presents a shared texture from another device or process (see [`SharedTexture`]); one
    /// opened with a fence takes [`Self::present_shared_fenced`] instead.
    pub fn present_shared(&self, texture: &SharedTexture) -> Result<u64, SurfaceError> {
        #[cfg(windows)]
        {
            if texture.inner.fenced {
                return Err(SurfaceError::Format(
                    "the shared texture has a fence: present it with present_shared_fenced".into(),
                ));
            }
            let sync = match texture.keys {
                Some((acquire, release)) => d3d11::SharedSync::Keyed(acquire, release),
                None => d3d11::SharedSync::None,
            };
            let frame = texture
                .inner
                .present(&self.shared.state, sync, texture.timeout_ms)?;
            sk_runtime::frames::wake();
            Ok(frame)
        }
        #[cfg(target_os = "macos")]
        {
            if texture.keyed {
                return Err(SurfaceError::NotSupported(
                    "a keyed mutex is a Direct3D object; on macOS synchronise with an \
                     MTLSharedEvent (SharedTexture::with_shared_event)",
                ));
            }
            if texture.inner.fenced() {
                return Err(SurfaceError::Format(
                    "the shared texture has a shared event: present it with present_shared_fenced"
                        .into(),
                ));
            }
            let frame = metal::present_shared(&self.shared.state, &texture.inner, None)?;
            sk_runtime::frames::wake();
            Ok(frame)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            match texture.never {}
        }
    }

    /// Presents a shared texture opened [`SharedTexture::with_fence`] (Windows) or
    /// [`SharedTexture::with_shared_event`] (macOS): the copy waits until the fence or event
    /// reaches `wait`, then it is signalled to `release` (the producer may write the texture again
    /// once it gets there).
    pub fn present_shared_fenced(
        &self,
        texture: &SharedTexture,
        wait: u64,
        release: u64,
    ) -> Result<u64, SurfaceError> {
        #[cfg(windows)]
        {
            if !texture.inner.fenced {
                return Err(SurfaceError::Format(
                    "the shared texture has no fence: open it with SharedTexture::with_fence"
                        .into(),
                ));
            }
            let frame = texture.inner.present(
                &self.shared.state,
                d3d11::SharedSync::Fence(wait, release),
                texture.timeout_ms,
            )?;
            sk_runtime::frames::wake();
            Ok(frame)
        }
        #[cfg(target_os = "macos")]
        {
            if !texture.inner.fenced() {
                return Err(SurfaceError::Format(
                    "the shared texture has no shared event: open it with \
                     SharedTexture::with_shared_event"
                        .into(),
                ));
            }
            let frame =
                metal::present_shared(&self.shared.state, &texture.inner, Some((wait, release)))?;
            sk_runtime::frames::wake();
            Ok(frame)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (wait, release);
            match texture.never {}
        }
    }

    /// The renderer's `MTLDevice`, on macOS. Textures for [`Self::present_metal_texture`] are made
    /// on it.
    pub fn metal_device(&self) -> Result<MetalDevice, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            metal::device(&self.shared.state).map(|inner| MetalDevice { inner })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// This surface's `MTLSharedEvent`, which [`Self::present_metal_texture`] signals once it has
    /// copied a frame out of the producer's texture.
    pub fn release_event(&self) -> Result<MetalEvent, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            metal::release_event(&self.shared.state).map(|inner| MetalEvent { inner })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// Presents an `id<MTLTexture>` made on [`Self::metal_device`]: 2D, `BGRA8Unorm` (or
    /// `BGRA8Unorm_sRGB`; `RGBA16Float` on a [`SurfaceFormat::Rgba16Float`] surface),
    /// single-sampled, straight alpha. The copy runs on a queue of the
    /// surface's own after a GPU wait for `ready` (an `id<MTLSharedEvent>` and the value the
    /// producer signals after its last write); without `ready` the texture must already be
    /// written. Nothing blocks. Returns the frame's id and the value [`Self::release_event`]
    /// reaches once the texture is free to reuse.
    ///
    /// # Safety
    /// `texture` is null or a live `id<MTLTexture>`; `ready`'s event is a live
    /// `id<MTLSharedEvent>`.
    pub unsafe fn present_metal_texture(
        &self,
        texture: *mut c_void,
        ready: Option<(*mut c_void, u64)>,
    ) -> Result<FramePresent, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: the caller's.
            let (frame, release) = unsafe { metal::present(&self.shared.state, texture, ready) }?;
            sk_runtime::frames::wake();
            Ok(FramePresent { frame, release })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (texture, ready);
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// Presents a `CVPixelBufferRef` (VideoToolbox, AVFoundation, ScreenCaptureKit): BGRA
    /// (`kCVPixelFormatType_32BGRA`) or 4:2:0 bi-planar (`420v`, `420f`, converted with the
    /// buffer's YCbCr matrix, BT.709 when it names none); `RGhA` (`kCVPixelFormatType_64RGBAHalf`)
    /// on a [`SurfaceFormat::Rgba16Float`] surface. The buffer is retained until the GPU copied it;
    /// a BGRA or RGhA one with no IOSurface is copied on the CPU. It must be fully written.
    ///
    /// # Safety
    /// `buffer` is null or a live `CVPixelBufferRef`.
    pub unsafe fn present_cv_pixel_buffer(&self, buffer: *mut c_void) -> Result<u64, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            let cpu = |cpu: metal::Cpu<'_>| match cpu.format {
                SurfaceFormat::Bgra8 => {
                    self.present_cpu(cpu.pixels, cpu.width, cpu.height, cpu.stride, false)
                }
                SurfaceFormat::Rgba16Float => {
                    if cpu.pixels.as_ptr().align_offset(2) != 0 || cpu.stride % 2 != 0 {
                        return Err(SurfaceError::Platform(
                            "a misaligned RGhA CVPixelBuffer".into(),
                        ));
                    }
                    // SAFETY: aligned above; the buffer stays locked while this runs.
                    let halves = unsafe {
                        std::slice::from_raw_parts(
                            cpu.pixels.as_ptr().cast::<u16>(),
                            cpu.pixels.len() / 2,
                        )
                    };
                    self.present_rgba16f(halves, cpu.width, cpu.height, cpu.stride / 2)
                }
            };
            // SAFETY: the caller's.
            let frame = unsafe { metal::present_pixel_buffer(&self.shared.state, buffer, cpu) }?;
            sk_runtime::frames::wake();
            Ok(frame)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = buffer;
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.shared.state)
    }

    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub(crate) fn lock_state(&self) -> MutexGuard<'_, State> {
        self.state()
    }

    /// Records what the elements report and tells the watchers when it changed. A display that is
    /// unknown, or within [`SurfaceDisplay::differs`]' margin of the last one, is not taken.
    pub(crate) fn set_info(&self, mut info: SurfaceInfo) {
        let (changed, display) = {
            let mut st = self.state();
            let old = st.info;
            let display = info.display.known && info.display.differs(&old.display);
            if !display {
                info.display = old.display;
            }
            st.info = info;
            if st.watched {
                let key = st.key;
                feedback::with(|t| t.visibility(key, info.visible));
            }
            (
                info != SurfaceInfo {
                    display: info.display,
                    ..old
                },
                display,
            )
        };
        if changed {
            self.notify(SurfaceEvent::Changed(info));
        }
        if display {
            self.notify(SurfaceEvent::DisplayChanged(self.with_clock(info.display)));
        }
    }

    pub(crate) fn notify(&self, event: SurfaceEvent) {
        let watchers: Vec<Watcher> = lock(&self.shared.watchers)
            .iter()
            .map(|(_, w)| w.clone())
            .collect();
        for watcher in watchers {
            watcher(self, event);
        }
    }

    /// The renderer replaced its device: a GPU frame and its texture are gone.
    pub(crate) fn device_lost(&self) {
        #[cfg(windows)]
        {
            let mut st = self.state();
            if matches!(st.frame, Some(Frame::Gpu { .. } | Frame::D3D12 { .. })) {
                st.frame = None;
                st.seq += 1;
                st.lost = "device";
            }
            st.gpu.forget();
            st.gpu12.forget();
        }
        #[cfg(target_os = "macos")]
        {
            let mut st = self.state();
            if matches!(st.frame, Some(Frame::Metal { .. })) {
                st.frame = None;
                st.seq += 1;
                st.lost = "device";
            }
            st.metal.forget();
        }
        self.notify(SurfaceEvent::DeviceLost);
    }
}

fn check_size(width: u32, height: u32) -> Result<(), SurfaceError> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(SurfaceError::Size(format!(
            "a {width}x{height} frame: each side must be 1 to {MAX_SIDE} pixels"
        )));
    }
    Ok(())
}

/// Ends an [`SurfaceHandle::on_change`] subscription when dropped.
#[must_use = "dropping the guard ends the subscription"]
pub struct SurfaceWatch {
    shared: Weak<Shared>,
    id: u64,
}

impl Drop for SurfaceWatch {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            lock(&shared.watchers).retain(|(id, _)| *id != self.id);
        }
    }
}

/// Ends an [`SurfaceHandle::on_frame`] subscription when dropped. Frames of a surface nobody
/// listens to are not tracked.
#[must_use = "dropping the guard ends the subscription"]
pub struct FrameWatch {
    shared: Weak<Shared>,
    id: u64,
}

impl Drop for FrameWatch {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let mut st = lock(&shared.state);
        let mut frames = lock(&shared.frames);
        frames.retain(|(id, _)| *id != self.id);
        if frames.is_empty() && st.watched {
            st.watched = false;
            feedback::unwatch(&shared);
        }
    }
}

/// The renderer's `ID3D11Device`, referenced while this lives. Multithread-protected: use it and its
/// immediate context from any thread. Raw, so any `windows` crate version can wrap it
/// (`ID3D11Device::from_raw_borrowed(&device.as_raw())`).
pub struct D3D11Device {
    #[cfg(windows)]
    inner: d3d11::Device,
    #[cfg(not(windows))]
    never: std::convert::Infallible,
}

impl D3D11Device {
    /// The `ID3D11Device*`, borrowed from this value.
    pub fn as_raw(&self) -> *mut c_void {
        #[cfg(windows)]
        {
            self.inner.as_raw()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    /// Changes when the renderer recreates its device.
    pub fn generation(&self) -> u64 {
        #[cfg(windows)]
        {
            self.inner.generation()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }
}

impl fmt::Debug for D3D11Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("D3D11Device")
            .field("raw", &self.as_raw())
            .field("generation", &self.generation())
            .finish()
    }
}

/// The renderer's `ID3D12Device` and its direct `ID3D12CommandQueue`, referenced while this lives.
/// Raw, so any `windows` crate version can wrap them.
pub struct D3D12Device {
    #[cfg(windows)]
    inner: d3d12::Device,
    #[cfg(not(windows))]
    never: std::convert::Infallible,
}

impl D3D12Device {
    /// The `ID3D12Device*`, borrowed from this value.
    pub fn as_raw(&self) -> *mut c_void {
        #[cfg(windows)]
        {
            self.inner.as_raw()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    /// The renderer's `ID3D12CommandQueue*`. Work submitted there delays the next frame; a producer
    /// with its own queue does not.
    pub fn queue(&self) -> *mut c_void {
        #[cfg(windows)]
        {
            self.inner.queue()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    /// Changes when the renderer recreates its device.
    pub fn generation(&self) -> u64 {
        #[cfg(windows)]
        {
            self.inner.generation()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }
}

impl fmt::Debug for D3D12Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("D3D12Device")
            .field("raw", &self.as_raw())
            .field("generation", &self.generation())
            .finish()
    }
}

/// The `ID3D12Fence` a surface signals when [`SurfaceHandle::present_d3d12`] is done with a
/// resource. Shared (`D3D12_FENCE_FLAG_SHARED`), so a queue on the same device can wait on it too.
pub struct D3D12Fence {
    #[cfg(windows)]
    inner: d3d12::Fence,
    #[cfg(not(windows))]
    never: std::convert::Infallible,
}

impl D3D12Fence {
    /// The `ID3D12Fence*`, borrowed from this value.
    pub fn as_raw(&self) -> *mut c_void {
        #[cfg(windows)]
        {
            self.inner.as_raw()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    pub fn completed_value(&self) -> u64 {
        #[cfg(windows)]
        {
            self.inner.completed()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    /// Blocks until the fence reaches `value` or `timeout` passes; whether it got there.
    pub fn wait(&self, value: u64, timeout: Duration) -> bool {
        #[cfg(windows)]
        {
            self.inner.wait(value, timeout)
        }
        #[cfg(not(windows))]
        {
            let _ = (value, timeout);
            match self.never {}
        }
    }

    /// Changes when the renderer recreates its device.
    pub fn generation(&self) -> u64 {
        #[cfg(windows)]
        {
            self.inner.generation()
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }
}

impl fmt::Debug for D3D12Fence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("D3D12Fence")
            .field("raw", &self.as_raw())
            .field("completed", &self.completed_value())
            .finish()
    }
}

/// The renderer's `id<MTLDevice>` and the `id<MTLCommandQueue>` of its first window, retained
/// while this lives. Raw, so any `objc2` or `metal` crate version can wrap them.
pub struct MetalDevice {
    #[cfg(target_os = "macos")]
    inner: metal::Device,
    #[cfg(not(target_os = "macos"))]
    never: std::convert::Infallible,
}

impl MetalDevice {
    /// The `id<MTLDevice>`, borrowed from this value.
    pub fn as_raw(&self) -> *mut c_void {
        #[cfg(target_os = "macos")]
        {
            self.inner.as_raw()
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self.never {}
        }
    }

    /// The renderer's `id<MTLCommandQueue>`. Work committed there delays the next frame; a
    /// producer with its own queue does not.
    pub fn queue(&self) -> *mut c_void {
        #[cfg(target_os = "macos")]
        {
            self.inner.queue()
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self.never {}
        }
    }

    /// Changes when the renderer moves to another device.
    pub fn generation(&self) -> u64 {
        #[cfg(target_os = "macos")]
        {
            self.inner.generation()
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self.never {}
        }
    }
}

impl fmt::Debug for MetalDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetalDevice")
            .field("raw", &self.as_raw())
            .field("generation", &self.generation())
            .finish()
    }
}

/// The `id<MTLSharedEvent>` a surface signals when [`SurfaceHandle::present_metal_texture`] is
/// done with a texture. A command buffer on any device can `encodeWaitForEvent` on it.
pub struct MetalEvent {
    #[cfg(target_os = "macos")]
    inner: metal::Event,
    #[cfg(not(target_os = "macos"))]
    never: std::convert::Infallible,
}

impl MetalEvent {
    /// The `id<MTLSharedEvent>`, borrowed from this value.
    pub fn as_raw(&self) -> *mut c_void {
        #[cfg(target_os = "macos")]
        {
            self.inner.as_raw()
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self.never {}
        }
    }

    pub fn signaled_value(&self) -> u64 {
        #[cfg(target_os = "macos")]
        {
            self.inner.signaled()
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self.never {}
        }
    }

    /// Blocks until the event reaches `value` or `timeout` passes; whether it got there.
    pub fn wait(&self, value: u64, timeout: Duration) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.inner.wait(value, timeout)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (value, timeout);
            match self.never {}
        }
    }

    /// Changes when the renderer moves to another device.
    pub fn generation(&self) -> u64 {
        #[cfg(target_os = "macos")]
        {
            self.inner.generation()
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self.never {}
        }
    }
}

impl fmt::Debug for MetalEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetalEvent")
            .field("raw", &self.as_raw())
            .field("signaled", &self.signaled_value())
            .finish()
    }
}

/// A shared BGRA8 or half-float texture opened on the renderer's device, from another device, API
/// or process.
/// On Windows: D3D11, D3D12, or Vulkan rendering into D3D12 memory it imports. On macOS: an
/// IOSurface from another `MTLDevice`, process or framework. Reopen it after
/// [`SurfaceError::DeviceChanged`].
pub struct SharedTexture {
    #[cfg(windows)]
    inner: shared::Opened,
    #[cfg(windows)]
    keys: Option<(u64, u64)>,
    #[cfg(windows)]
    timeout_ms: u32,
    #[cfg(target_os = "macos")]
    inner: metal::Opened,
    #[cfg(target_os = "macos")]
    keyed: bool,
    #[cfg(not(any(windows, target_os = "macos")))]
    never: std::convert::Infallible,
}

#[cfg(target_os = "macos")]
const NT_HANDLES: &str = "NT handles are Windows-only; on macOS open an IOSurface \
                          (SharedTexture::from_iosurface, SharedTexture::from_mach_port)";

impl SharedTexture {
    #[cfg(windows)]
    fn new(inner: shared::Opened) -> Self {
        Self {
            inner,
            keys: None,
            timeout_ms: 100,
        }
    }

    #[cfg(target_os = "macos")]
    fn new(inner: metal::Opened) -> Self {
        Self {
            inner,
            keyed: false,
        }
    }

    /// Opens a handle this process holds: an NT handle (`CreateSharedHandle` in D3D11 or D3D12)
    /// or a legacy one (`IDXGIResource::GetSharedHandle`). The handle stays the caller's to close.
    pub fn open(handle: u64) -> Result<Self, SurfaceError> {
        #[cfg(windows)]
        {
            shared::open(handle).map(Self::new)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = handle;
            Err(SurfaceError::NotSupported(NT_HANDLES))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = handle;
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// Duplicates the NT handle `handle` of process `pid` into this process and opens it. Needs
    /// `PROCESS_DUP_HANDLE` on that process, which a child or a same-user process grants.
    pub fn open_from_process(pid: u32, handle: u64) -> Result<Self, SurfaceError> {
        #[cfg(windows)]
        {
            shared::open_from_process(pid, handle).map(Self::new)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = (pid, handle);
            Err(SurfaceError::NotSupported(NT_HANDLES))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (pid, handle);
            Err(SurfaceError::NotSupported(NO_D3D))
        }
    }

    /// Opens an `IOSurfaceRef` this process holds (macOS): pixel format `'BGRA'` (or `'RGhA'`), made
    /// by any `MTLDevice` or framework. It is retained while this lives.
    ///
    /// # Safety
    /// `surface` is null or a live `IOSurfaceRef`.
    pub unsafe fn from_iosurface(surface: *mut c_void) -> Result<Self, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: the caller's.
            unsafe { metal::open_raw(surface) }.map(Self::new)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = surface;
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// Opens the IOSurface another process sent as a mach port (`IOSurfaceCreateMachPort`), a
    /// send right this process holds and stays the caller's to deallocate.
    pub fn from_mach_port(port: u32) -> Result<Self, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            metal::open_mach_port(port).map(Self::new)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = port;
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// Synchronises through the texture's `IDXGIKeyedMutex`: each present acquires `acquire` and
    /// releases `release`. Required for a texture made with `SHARED_KEYEDMUTEX`. Windows only; on
    /// macOS a present of it fails with [`SurfaceError::NotSupported`].
    pub fn with_keyed_mutex(self, acquire: u64, release: u64) -> Self {
        #[cfg(windows)]
        {
            let mut this = self;
            this.inner.need_d3d11();
            this.keys = Some((acquire, release));
            this
        }
        #[cfg(target_os = "macos")]
        {
            let _ = (acquire, release);
            Self {
                keyed: true,
                ..self
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (acquire, release);
            match self.never {}
        }
    }

    /// Synchronises through a shared fence, an NT handle this process holds: a D3D12 fence made
    /// `D3D12_FENCE_FLAG_SHARED`, an `ID3D11Fence` made `D3D11_FENCE_FLAG_SHARED`, or a Vulkan
    /// semaphore exported as `D3D12_FENCE`. Present it with
    /// [`SurfaceHandle::present_shared_fenced`]. The handle stays the caller's to close.
    pub fn with_fence(self, fence: u64) -> Result<Self, SurfaceError> {
        #[cfg(windows)]
        {
            let mut this = self;
            this.inner.open_fence(fence)?;
            Ok(this)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = (self, fence);
            Err(SurfaceError::NotSupported(NT_HANDLES))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = fence;
            match self.never {}
        }
    }

    /// [`Self::with_fence`] for a fence handle of process `pid`.
    pub fn with_fence_from_process(self, pid: u32, fence: u64) -> Result<Self, SurfaceError> {
        #[cfg(windows)]
        {
            let mut this = self;
            this.inner.open_fence_from_process(pid, fence)?;
            Ok(this)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = (self, pid, fence);
            Err(SurfaceError::NotSupported(NT_HANDLES))
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (pid, fence);
            match self.never {}
        }
    }

    /// Synchronises through an `id<MTLSharedEvent>` of this process (macOS), from any device:
    /// present it with [`SurfaceHandle::present_shared_fenced`]. It is retained while this lives.
    ///
    /// # Safety
    /// `event` is null or a live `id<MTLSharedEvent>`.
    pub unsafe fn with_shared_event(self, event: *mut c_void) -> Result<Self, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            let mut this = self;
            // SAFETY: the caller's.
            unsafe { this.inner.set_event(event) }?;
            Ok(this)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (self, event);
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// [`Self::with_shared_event`] for an event of another process: the `MTLSharedEventHandle`
    /// it made (`newSharedEventHandle`) and sent over XPC.
    ///
    /// # Safety
    /// `handle` is null or a live `MTLSharedEventHandle`.
    pub unsafe fn with_shared_event_handle(
        self,
        handle: *mut c_void,
    ) -> Result<Self, SurfaceError> {
        #[cfg(target_os = "macos")]
        {
            let mut this = self;
            // SAFETY: the caller's.
            unsafe { this.inner.set_event_handle(handle) }?;
            Ok(this)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (self, handle);
            Err(SurfaceError::NotSupported(NO_METAL))
        }
    }

    /// How long a present waits for the acquire key, or (under the D3D11 renderer) for the fence;
    /// 100 ms by default. Under the D3D12 renderer and on macOS the GPU waits instead.
    pub fn with_timeout(self, timeout: Duration) -> Self {
        #[cfg(windows)]
        {
            let mut this = self;
            this.timeout_ms = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
            this
        }
        #[cfg(target_os = "macos")]
        {
            let _ = timeout;
            self
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = timeout;
            match self.never {}
        }
    }

    pub fn size(&self) -> (u32, u32) {
        #[cfg(any(windows, target_os = "macos"))]
        {
            self.inner.size
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            match self.never {}
        }
    }

    /// The texture's pixels, which the surface it is presented to must be set to.
    pub fn format(&self) -> SurfaceFormat {
        #[cfg(any(windows, target_os = "macos"))]
        {
            self.inner.format
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            match self.never {}
        }
    }
}

impl fmt::Debug for SharedTexture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedTexture")
            .field("size", &self.size())
            .finish()
    }
}

/// The renderer recreated its device: every surface drops its GPU frame and texture.
pub(crate) fn device_changed() {
    #[cfg(windows)]
    {
        d3d11::forget_device();
        d3d12::forget_device();
    }
    #[cfg(target_os = "macos")]
    metal::forget_device();
    feedback::with(|t| t.device_lost());
    for surface in all() {
        surface.device_lost();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_name_is_one_surface() {
        let a = surface("tests:one-name");
        let b = surface("tests:one-name");
        a.present_bgra(&[1, 2, 3, 4], 1, 1, 4).unwrap();
        assert_eq!(b.stats().presented, 1);
        assert_eq!(surface("tests:other-name").stats().presented, 0);
    }

    #[test]
    fn a_padded_frame_is_packed_and_a_short_one_refused() {
        let s = surface("tests:stride");
        let mut pixels = vec![0u8; 12 * 2];
        pixels[..8].copy_from_slice(&[1, 1, 1, 1, 2, 2, 2, 2]);
        pixels[12..20].copy_from_slice(&[3, 3, 3, 3, 4, 4, 4, 4]);
        s.present_bgra(&pixels, 2, 2, 12).unwrap();
        match s.state().frame.clone() {
            Some(Frame::Cpu { bgra, .. }) => {
                assert_eq!(&*bgra, &[1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4])
            }
            _ => panic!("a CPU frame"),
        }
        assert!(matches!(
            s.present_bgra(&pixels[..19], 2, 2, 12),
            Err(SurfaceError::Size(_))
        ));
        assert!(matches!(
            s.present_bgra(&pixels, 4, 2, 12),
            Err(SurfaceError::Size(_))
        ));
        assert!(matches!(
            s.present_bgra(&[], 0, 0, 0),
            Err(SurfaceError::Size(_))
        ));
        assert_eq!(s.stats().presented, 1);
        s.present_bgrx(&pixels, 2, 2, 12).unwrap();
        let frame = s.state().frame.clone();
        match frame {
            Some(Frame::Cpu { bgra, .. }) => {
                assert_eq!(
                    &*bgra,
                    &[1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]
                )
            }
            _ => panic!("a CPU frame"),
        }
    }

    #[test]
    fn watchers_hear_changes_once_and_stop_when_dropped() {
        let s = surface("tests:watch");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let watch = s.on_change(move |_, e| lock(&sink).push(e));
        let info = SurfaceInfo {
            width: 640,
            height: 360,
            scale: 1.5,
            visible: true,
            ..Default::default()
        };
        s.set_info(info);
        s.set_info(info);
        s.device_lost();
        drop(watch);
        s.set_info(SurfaceInfo::default());
        assert_eq!(
            *lock(&heard),
            vec![SurfaceEvent::Changed(info), SurfaceEvent::DeviceLost]
        );
        assert_eq!(s.info(), SurfaceInfo::default());
    }

    #[test]
    fn a_display_is_reported_when_it_moves_past_the_margin_and_kept_when_unknown() {
        let s = surface("tests:display");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let _watch = s.on_change(move |_, e| lock(&sink).push(e));
        let hdr = SurfaceDisplay {
            known: true,
            hdr: true,
            headroom: 4.0,
            potential_headroom: 4.0,
            refresh_hz: Some(60.0),
            ..Default::default()
        };
        let with = |display| SurfaceInfo {
            display,
            ..Default::default()
        };
        s.set_info(with(hdr));
        s.set_info(with(SurfaceDisplay {
            headroom: 4.02,
            refresh_hz: Some(59.94),
            ..hdr
        }));
        s.set_info(with(SurfaceDisplay::default()));
        let faster = SurfaceDisplay {
            refresh_hz: Some(120.0),
            ..hdr
        };
        s.set_info(with(faster));
        let drawing_hdr = SurfaceDisplay {
            output_hdr: true,
            ..faster
        };
        s.set_info(with(drawing_hdr));
        // The display clock is read live; a known display always has a period.
        let unclocked = |d: SurfaceDisplay| {
            assert!(d.refresh_period.is_some());
            SurfaceDisplay {
                refresh_period: None,
                last_vblank: None,
                ..d
            }
        };
        let heard: Vec<SurfaceEvent> = lock(&heard)
            .iter()
            .map(|e| match *e {
                SurfaceEvent::DisplayChanged(d) => SurfaceEvent::DisplayChanged(unclocked(d)),
                other => other,
            })
            .collect();
        assert_eq!(
            heard,
            vec![
                SurfaceEvent::DisplayChanged(hdr),
                SurfaceEvent::DisplayChanged(faster),
                SurfaceEvent::DisplayChanged(drawing_hdr)
            ]
        );
        assert_eq!(unclocked(s.info().display), drawing_hdr);
    }

    #[test]
    fn a_frame_of_the_other_format_is_refused_and_half_floats_are_mapped() {
        let s = surface("tests:format");
        assert_eq!(s.format(), SurfaceFormat::Bgra8);
        assert_eq!(
            s.present_rgba16f(&[0; 4], 1, 1, 4),
            Err(SurfaceError::FormatMismatch {
                surface: SurfaceFormat::Bgra8,
                frame: SurfaceFormat::Rgba16Float
            })
        );
        s.set_format(SurfaceFormat::Rgba16Float);
        assert!(matches!(
            s.present_bgra(&[0; 4], 1, 1, 4),
            Err(SurfaceError::FormatMismatch { .. })
        ));
        assert_eq!(s.stats().presented, 0);
        let h = color::tests::f32_to_half;
        s.present_rgba16f(&[h(2.0), h(0.5), h(0.0), h(1.0)], 1, 1, 4)
            .unwrap();
        let frame = s.state().frame.clone();
        match frame {
            Some(Frame::Cpu { bgra, .. }) => assert_eq!(&*bgra, &[0, 188, 255, 255]),
            _ => panic!("a CPU frame"),
        }
    }

    #[test]
    fn each_present_returns_its_id_and_a_hidden_surface_drops_it_at_once() {
        let s = surface("tests:frame-ids");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let watch = s.on_frame(move |e| lock(&sink).push(e));
        let first = s.present_bgra(&[0; 4], 1, 1, 4).unwrap();
        let second = s.present_bgrx(&[0; 4], 1, 1, 4).unwrap();
        assert!(second > first);
        feedback::deliver();
        let hidden = |frame| FrameEvent::Dropped {
            frame,
            reason: FrameDropReason::Hidden,
        };
        assert_eq!(*lock(&heard), [hidden(first), hidden(second)]);
        drop(watch);
        s.present_bgra(&[0; 4], 1, 1, 4).unwrap();
        feedback::deliver();
        assert_eq!(lock(&heard).len(), 2, "nobody listens any more");
    }

    #[test]
    fn clearing_withdraws_the_frame_once() {
        let s = surface("tests:clear");
        s.present_bgra(&[0; 4], 1, 1, 4).unwrap();
        let seq = s.state().seq;
        s.clear();
        s.clear();
        let st = s.state();
        assert!(st.frame.is_none());
        assert_eq!(st.lost, "cleared");
        assert_eq!(st.seq, seq + 1);
    }
}
