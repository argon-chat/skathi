//! The Windows backend: `IMFMediaEngine` in frame-server mode.
//!
//! # Why this object and not a decoder
//!
//! `IMFMediaEngine` is Microsoft's implementation of `HTMLMediaElement`. Its
//! event enumeration is documented as *"values below 1000 correspond to events
//! defined in HTML 5 for media elements"*, and its methods are
//! `Play`/`Pause`/`SetCurrentTime`/`GetDuration`/`SetVolume`/`SetMuted`/
//! `SetLoop`/`SetPlaybackRate`/`GetBuffered`/`GetSeekable`/`GetReadyState`/
//! `CanPlayType` — a 1:1 map onto the spec. It renders the audio itself, so
//! **A/V sync, the audio clock, the endpoint and rate control are the OS's
//! problem, not ours**. Choosing the layer below (`IMFSourceReader`) would mean
//! hand-writing `readyState` transitions, the seek algorithm, `TimeRanges`, an
//! audio renderer and a master clock — weeks of work to reimplement, less
//! correctly, something `mfmediaengine.dll` already does.
//!
//! Frame-server mode is selected by *not* giving the engine a window or a
//! DirectComposition visual: it then decodes into textures and hands them over
//! on request through `OnVideoStreamTick` + `TransferVideoFrame`, which keeps
//! the video inside our scene graph (so `border-radius`, `overflow: hidden`,
//! `opacity` and z-order all work on it) instead of on a sibling visual that
//! escapes CSS entirely.
//!
//! # The device
//!
//! The engine decodes onto whatever `ID3D11Device` we give it through an
//! `IMFDXGIDeviceManager`. Given the renderer's own device — which
//! `gpui_windows::renderer_d3d11_device()` now publishes — decoded samples come
//! back as `ID3D11Texture2D` **on that device**, so the eventual zero-copy path
//! is a pure-VRAM blit. Given none, we create a private one; the v1 present
//! path reads frames back to system memory anyway (§5 of CONTRACT.md), so a
//! private device is a correct, slightly slower fallback rather than a failure.
//!
//! Asking for `DXGI_FORMAT_B8G8R8A8_UNORM` output makes MF insert its D3D11
//! video processor, which crops to the display aperture (1080, not the coded
//! 1088), converts colour on the GPU's fixed-function block and hands back
//! exactly the format gpui's atlas uses. No NV12 shader, no colour matrix, no
//! BT.601-vs-709 decision.

mod codecs;
mod notify;
mod present;

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};

use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaEngine, IMFMediaEngineClassFactory, IMFMediaEngineNotify,
    IMFDXGIDeviceManager, MFCreateAttributes, MFCreateDXGIDeviceManager, MFStartup,
    CLSID_MFMediaEngineClassFactory, MFSTARTUP_FULL, MF_MEDIA_ENGINE_CALLBACK,
    MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC,
    MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
    MF_VERSION,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED};

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaErrorCode, MediaEvent, MediaSource, NetworkState,
    ReadyState, VideoBackend, VideoFrame,
};

pub use notify::EngineNotify;

// ---------------------------------------------------------------------------
// the shared device
// ---------------------------------------------------------------------------

static SHARED_DEVICE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SHARED_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Publishes the renderer's `ID3D11Device` so decoded frames land on it.
///
/// The host calls this once at boot with what
/// `gpui_windows::renderer_d3d11_device()` returns. The pointer is BORROWED —
/// this crate clones the interface (which AddRefs) when it builds an engine.
///
/// Raw rather than `ID3D11Device` on purpose: `gpui_windows` compiles against
/// `windows` 0.61 and this crate against 0.62, and a COM interface pointer is
/// an ABI, not a type. Coupling the two versions would mean this crate's
/// Media Foundation coverage was hostage to whatever version the fork happens
/// to be on.
///
/// # Safety
/// `ptr` must be a valid `ID3D11Device` COM pointer that outlives the process's
/// use of `<video>`, or null to clear.
pub unsafe fn set_shared_device(ptr: *mut c_void, generation: u64) {
    SHARED_DEVICE.store(ptr, Ordering::Release);
    SHARED_GENERATION.store(generation, Ordering::Release);
}

/// The renderer's device if one was published, else a private one built the
/// same way. `bool` = "this is the renderer's own device".
fn device() -> Result<(ID3D11Device, bool), MediaError> {
    // `VN_VIDEO_DEVICE=private` forces the fallback. An escape hatch worth
    // keeping: sharing the renderer's device is the whole zero-copy argument,
    // so when a machine misbehaves the first question is always "is it the
    // sharing?", and answering it should not need a rebuild.
    if std::env::var("VN_VIDEO_DEVICE").is_ok_and(|v| v == "private") {
        return Ok((private_device()?, false));
    }
    let raw = SHARED_DEVICE.load(Ordering::Acquire);
    if !raw.is_null() {
        // SAFETY: the host publishes a device it owns for the process lifetime.
        if let Some(borrowed) = unsafe { ID3D11Device::from_raw_borrowed(&raw) } {
            return Ok((borrowed.clone(), true));
        }
    }
    Ok((private_device()?, false))
}

/// The fallback device: same flags the renderer uses, so the two paths differ
/// only in *whose* VRAM the frame lands in.
fn private_device() -> Result<ID3D11Device, MediaError> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| {
            MediaError::new(
                MediaErrorCode::Decode,
                format!("could not create a Direct3D 11 video device: {e}"),
            )
            .with_advisory(
                "the GPU driver refused a device with D3D11_CREATE_DEVICE_VIDEO_SUPPORT — \
                 <video> needs hardware video support. Update the display driver, or run with \
                 VN_ADAPTER=<name> to pick a different adapter.",
            )
        })?;
    }
    let device = device.expect("D3D11CreateDevice succeeded without a device");
    if let Ok(mt) = device.cast::<ID3D11Multithread>() {
        // MF's decoder threads share this context with us.
        let _ = unsafe { mt.SetMultithreadProtected(true) };
    }
    Ok(device)
}

/// `MFStartup` + `CoInitializeEx`, once per process.
fn ensure_platform() -> Result<(), MediaError> {
    static ONCE: Once = Once::new();
    static mut RESULT: Option<MediaError> = None;
    ONCE.call_once(|| {
        unsafe {
            // The host thread may already be initialised, in either apartment.
            // `RPC_E_CHANGED_MODE` means "already in the other one", which is
            // fine — the Media Engine works from both.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                RESULT = Some(
                    MediaError::new(
                        MediaErrorCode::Decode,
                        format!("MFStartup failed: {e}"),
                    )
                    .with_advisory(
                        "Media Foundation is not available on this system — <video> needs it. \
                         On Windows N/KN editions install the Media Feature Pack.",
                    ),
                );
            }
        }
    });
    // SAFETY: written exactly once inside `call_once`, read only after it.
    match unsafe { (*std::ptr::addr_of!(RESULT)).clone() } {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// the backend
// ---------------------------------------------------------------------------

pub struct MediaEngineBackend {
    engine: IMFMediaEngine,
    /// Kept alive for the engine's lifetime: MF holds a weak-ish reference and
    /// dropping this would leave it calling into freed memory.
    _notify: IMFMediaEngineNotify,
    queue: Arc<Mutex<Vec<u32>>>,
    presenter: present::Presenter,
    /// The last PTS `OnVideoStreamTick` reported, so an unchanged one is
    /// recognised as "no new frame" (`windows-rs` collapses `S_FALSE` into
    /// `Ok(())`, so the HRESULT cannot be used for that).
    last_pts: i64,
    shared_device: bool,
    /// The `src` last handed to the engine, for the error advisories.
    current_src: String,
}

impl MediaEngineBackend {
    pub fn new() -> Result<Self, MediaError> {
        ensure_platform()?;
        let (device, shared_device) = device()?;

        // The DXGI device manager is how MF is told "decode onto THIS device".
        let mut token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::Decode,
                format!("MFCreateDXGIDeviceManager failed: {e}"),
            )
        })?;
        let manager = manager.expect("MFCreateDXGIDeviceManager succeeded without a manager");
        unsafe { manager.ResetDevice(&device, token) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::Decode,
                format!("IMFDXGIDeviceManager::ResetDevice failed: {e}"),
            )
            .with_advisory(
                "the renderer's Direct3D device was refused by Media Foundation — it must be \
                 created with D3D11_CREATE_DEVICE_VIDEO_SUPPORT (gpui_windows does this since \
                 the m12 fork commit).",
            )
        })?;

        let queue: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let notify: IMFMediaEngineNotify = EngineNotify::new(queue.clone()).into();

        let mut attributes: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attributes, 4) }
            .map_err(|e| MediaError::new(MediaErrorCode::Decode, format!("MFCreateAttributes: {e}")))?;
        let attributes = attributes.expect("MFCreateAttributes succeeded without attributes");
        unsafe {
            attributes
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)
                .and_then(|()| attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager))
                .and_then(|()| {
                    attributes.SetUINT32(
                        &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                        DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
                    )
                })
        }
        .map_err(|e| {
            MediaError::new(MediaErrorCode::Decode, format!("media engine attributes: {e}"))
        })?;

        let factory: IMFMediaEngineClassFactory =
            unsafe { CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| {
                    MediaError::new(
                        MediaErrorCode::Decode,
                        format!("MFMediaEngineClassFactory: {e}"),
                    )
                    .with_advisory(
                        "mfmediaengine.dll could not be created — <video> needs the Windows \
                         media stack. On N/KN editions install the Media Feature Pack.",
                    )
                })?;

        // No playback HWND and no DirectComposition visual => frame-server
        // mode, which is what keeps the video inside our scene graph.
        let engine = unsafe { factory.CreateInstance(0, &attributes) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::Decode,
                format!("IMFMediaEngineClassFactory::CreateInstance: {e}"),
            )
        })?;

        Ok(Self {
            engine,
            _notify: notify,
            queue,
            presenter: present::Presenter::new(device),
            last_pts: i64::MIN,
            shared_device,
            current_src: String::new(),
        })
    }

    /// True when frames are being decoded onto the renderer's own device — the
    /// precondition for the eventual zero-copy present path. Reported by the
    /// element layer so a machine where it is false says so.
    pub fn uses_renderer_device(&self) -> bool {
        self.shared_device
    }

    /// The engine's `IMFMediaError`, translated, with the advisory that turns
    /// "MEDIA_ERR_SRC_NOT_SUPPORTED" into something actionable.
    fn engine_error(&self) -> Option<MediaError> {
        let error = unsafe { self.engine.GetError() }.ok()?;
        let code = unsafe { error.GetErrorCode() };
        let extended = match unsafe { error.GetExtendedErrorCode() } {
            Ok(()) => String::new(),
            Err(e) => format!(" [{}]", e.code().0),
        };
        let src = &self.current_src;
        Some(match code {
            1 => MediaError::new(MediaErrorCode::Aborted, "playback was aborted"),
            2 => MediaError::new(
                MediaErrorCode::Network,
                format!("a network error stopped playback{extended}"),
            ),
            3 => MediaError::new(
                MediaErrorCode::Decode,
                format!("the media could not be decoded{extended}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: the file was recognised but a frame could not be \
                 decoded. This is usually a corrupt file or a codec profile the installed \
                 decoder does not cover (10-bit HEVC, AV1 Professional). Re-encode to H.264 \
                 High profile 8-bit, which is inbox on every Windows install."
            )),
            5 => MediaError::new(MediaErrorCode::Decode, "the media is encrypted")
                .with_advisory(
                    "<video>: encrypted media (EME / PlayReady / Widevine) is not supported and \
                     is not planned — see crates/vn-video/CONTRACT.md §7.",
                ),
            // 4 = MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED, and everything unknown.
            _ => MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("the source is not supported{extended}"),
            )
            .with_advisory(self.unsupported_advisory()),
        })
    }

    /// The advisory for "this machine cannot play this file" — the one place
    /// the owner constraint "no reliance on Store codec extensions" turns into
    /// something a developer can act on.
    ///
    /// Built from a LIVE registry probe at the moment of failure, never a
    /// build-time table: what is decodable here is a property of this machine's
    /// installed Store packages, and one of them can appear after we shipped.
    fn unsupported_advisory(&self) -> String {
        let src = &self.current_src;
        let (installed, missing) = codecs::installed_summary();
        let mut out = format!(
            "<video src=\"{src}\"> could not be played. Media Foundation refused the file: either \
             its CONTAINER has no source registered on this machine, its codec has no decoder, \
             or the file is damaged/truncated. (An extension that lies about the contents is the \
             single most common cause — check what the file really is.)"
        );
        out.push_str(&format!(
            " Video decoders installed on this machine: {}.",
            if installed.is_empty() { "none".to_string() } else { installed.join(", ") }
        ));
        if !missing.is_empty() {
            out.push_str(&format!(" NOT installed: {}.", missing.join(", ")));
        }
        if codecs::have_decoder(codecs::Codec::H264) {
            out.push_str(
                " The always-safe fix is to re-encode to the inbox baseline: `ffmpeg -i in \
                 -c:v libx264 -profile:v high -pix_fmt yuv420p -c:a aac out.mp4`, which needs no \
                 Store extension on any Windows install.",
            );
        } else {
            out.push_str(
                " H.264 itself is missing, which means the Windows media stack is not fully \
                 installed — an N/KN edition without the Media Feature Pack.",
            );
        }
        out
    }

    fn time_ranges(&self, seekable: bool) -> Vec<(f64, f64)> {
        let ranges = if seekable {
            unsafe { self.engine.GetSeekable() }
        } else {
            unsafe { self.engine.GetBuffered() }
        };
        let Ok(ranges) = ranges else { return Vec::new() };
        let count = unsafe { ranges.GetLength() };
        (0..count)
            .filter_map(|i| {
                let start = unsafe { ranges.GetStart(i) }.ok()?;
                let end = unsafe { ranges.GetEnd(i) }.ok()?;
                Some((start, end))
            })
            .collect()
    }
}

/// `VN_VIDEO_LOG=1` — a trace of the load handshake, on stderr.
///
/// Kept because the failure it exists for is invisible otherwise: `SetSource`
/// and `Load` can both return `S_OK` and leave `networkState` at EMPTY, and
/// there is no error anywhere to notice. Two lines of trace is the difference
/// between "the video does not play" and "the URL was malformed".
fn trace(args: std::fmt::Arguments<'_>) {
    use std::sync::atomic::{AtomicU8, Ordering};
    static ON: AtomicU8 = AtomicU8::new(2);
    let on = match ON.load(Ordering::Relaxed) {
        2 => {
            let on = std::env::var("VN_VIDEO_LOG").is_ok_and(|v| v != "0") as u8;
            ON.store(on, Ordering::Relaxed);
            on
        }
        v => v,
    };
    if on == 1 {
        eprintln!("[vn-video] {args}");
    }
}

impl VideoBackend for MediaEngineBackend {
    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError> {
        let MediaSource::Path(path) = source;
        let url = file_url(path);
        trace(format_args!("load {url}  (shared device: {})", self.shared_device));
        self.current_src = path.display().to_string();
        self.last_pts = i64::MIN;
        self.presenter.reset();
        unsafe { self.engine.SetSource(&BSTR::from(url.as_str())) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("could not set the media source: {e}"),
            )
            .with_advisory(format!(
                "<video src=\"{}\">: Media Foundation refused the URL. Local file paths and \
                 file:// URLs are what this build supports.",
                path.display()
            ))
        })?;
        // `SetSource` alone leaves `networkState` at EMPTY: unlike the DOM,
        // where assigning `src` runs the media element load algorithm, the
        // Media Engine wants to be told. `SetPreload(AUTOMATIC)` then says
        // "fetch the whole thing", which is what a local file wants.
        // [measured: without the Load() below, readyState and networkState both
        // stay 0 forever and no event is ever raised.]
        let _ = unsafe { self.engine.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC) };
        unsafe { self.engine.Load() }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("the media could not be loaded: {e}"),
            )
        })?;
        trace(format_args!(
            "  after Load(): networkState={} readyState={}",
            unsafe { self.engine.GetNetworkState() },
            unsafe { self.engine.GetReadyState() },
        ));
        Ok(())
    }

    fn play(&mut self) {
        let _ = unsafe { self.engine.Play() };
    }

    fn pause(&mut self) {
        let _ = unsafe { self.engine.Pause() };
    }

    fn seek(&mut self, seconds: f64) {
        let _ = unsafe { self.engine.SetCurrentTime(seconds) };
    }

    fn set_volume(&mut self, volume: f64) {
        let _ = unsafe { self.engine.SetVolume(volume) };
    }

    fn set_muted(&mut self, muted: bool) {
        let _ = unsafe { self.engine.SetMuted(muted) };
    }

    fn set_rate(&mut self, rate: f64) {
        let _ = unsafe { self.engine.SetPlaybackRate(rate) };
    }

    fn set_looping(&mut self, looping: bool) {
        let _ = unsafe { self.engine.SetLoop(looping) };
    }

    fn current_time(&self) -> f64 {
        unsafe { self.engine.GetCurrentTime() }
    }

    fn duration(&self) -> f64 {
        unsafe { self.engine.GetDuration() }
    }

    fn paused(&self) -> bool {
        unsafe { self.engine.IsPaused() }.as_bool()
    }

    fn ended(&self) -> bool {
        unsafe { self.engine.IsEnded() }.as_bool()
    }

    fn seeking(&self) -> bool {
        unsafe { self.engine.IsSeeking() }.as_bool()
    }

    fn ready_state(&self) -> ReadyState {
        ReadyState::from_u8(unsafe { self.engine.GetReadyState() } as u8)
    }

    fn network_state(&self) -> NetworkState {
        NetworkState::from_u8(unsafe { self.engine.GetNetworkState() } as u8)
    }

    fn native_size(&self) -> Option<(u32, u32)> {
        let (mut cx, mut cy) = (0u32, 0u32);
        unsafe { self.engine.GetNativeVideoSize(Some(&mut cx), Some(&mut cy)) }.ok()?;
        (cx > 0 && cy > 0).then_some((cx, cy))
    }

    fn buffered(&self) -> Vec<(f64, f64)> {
        self.time_ranges(false)
    }

    fn seekable(&self) -> Vec<(f64, f64)> {
        self.time_ranges(true)
    }

    fn error(&self) -> Option<MediaError> {
        self.engine_error()
    }

    /// **Not `IMFMediaEngine::CanPlayType`.** [measured] that method answers
    /// `"probably"` for `video/ogg; codecs="theora"` on this machine — a
    /// container Media Foundation has no source for, carrying a codec with no
    /// decoder installed. It appears to answer on the shape of the MIME string
    /// rather than on the registry, which makes it worse than useless for the
    /// one question that matters: can THIS machine play THIS file. `codecs.rs`
    /// asks `MFTEnumEx` instead.
    fn can_play_type(&self, mime_type: &str) -> CanPlay {
        codecs::can_play_type(mime_type)
    }

    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame> {
        let native = self.native_size()?;
        let size = crate::fit_within(native, request.max_size);
        if size.0 == 0 || size.1 == 0 {
            return None;
        }

        // `OnVideoStreamTick` answers "is there a new frame" with S_OK vs
        // S_FALSE, and `windows-rs` maps BOTH to `Ok` (S_FALSE is a success
        // code) — leaving the PTS at its zeroed default on S_FALSE. So the PTS
        // is what actually distinguishes them, and an unchanged one means the
        // engine has nothing new.
        //
        // Two cases must transfer regardless: the very first frame (a video
        // seeked to 0 has PTS 0, which is also what S_FALSE leaves behind) and
        // a resize, whose new texture is empty until something is blitted into
        // it.
        let pts = unsafe { self.engine.OnVideoStreamTick() }.ok()?;
        let forced = self.last_pts == i64::MIN || self.presenter.size() != size;
        if !forced && pts == self.last_pts {
            return None;
        }
        self.last_pts = pts;

        // Pipeline only while PLAYING. A paused element produces one frame per
        // seek with nothing behind it, so a frame of latency there is not
        // latency at all — it is the wrong picture, permanently. See
        // `Presenter::transfer`.
        let pipeline = !self.paused();
        let (image, frame_pts) = self.presenter.transfer(&self.engine, size, pts, pipeline)?;
        Some(VideoFrame {
            image,
            // MF reports the presentation time in 100 ns units.
            pts: frame_pts as f64 / 10_000_000.0,
            size,
        })
    }

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        let raw = {
            let Ok(mut queue) = self.queue.lock() else { return };
            std::mem::take(&mut *queue)
        };
        for code in raw {
            if let Some(event) = notify::translate(code) {
                sink(event);
            }
        }
    }

    fn suspend(&mut self) {
        self.pause();
        // Our own textures go back — two staging surfaces plus the transfer
        // target, i.e. 3 x (painted area x 4 bytes).
        self.presenter.reset();
        self.last_pts = i64::MIN;
        // NOT `SetSource("")`. Tearing the engine's pipeline down here and
        // rebuilding it in `resume` is what a browser effectively does with a
        // detached video, and it does release ~20 MiB of the ~28 MiB a 1080p
        // pipeline holds — but [measured] it also grows the process by ~5 MiB
        // per park/unpark cycle and takes it down after about ten. A bounded
        // resident cost beats an unbounded one, so the decoder stays. See
        // CONTRACT.md section 5 for the diagnosis this needs before it can come
        // back.
    }
}

/// `C:\x\a.mp4` → `file:///C:/x/a.mp4`, percent-escaping what a URL must.
///
/// Media Foundation accepts a bare path too, but only sometimes — a path with
/// a `#` or a `%` in it is read as a URL fragment/escape and the open fails
/// with a "file not found" that names a file the user can see is there.
fn file_url(path: &Path) -> String {
    let raw = path.to_string_lossy();
    // Strip Windows extended-length prefixes first. `Path::canonicalize`
    // produces them, and they are FILESYSTEM syntax, not URL syntax: Media
    // Foundation fails such a URL with ERROR_INVALID_NAME (0x8007007B), which
    // surfaces as "the source is not supported" for a file that plainly exists
    // and plainly decodes. [measured — this one cost an hour.]
    let stripped = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| raw.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| raw.to_string());
    let text = stripped.replace('\\', "/");
    let mut out = String::from("file:///");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_become_well_formed_file_urls() {
        assert_eq!(
            file_url(Path::new(r"C:\apps\hello\assets\clip.mp4")),
            "file:///C:/apps/hello/assets/clip.mp4"
        );
        // The characters that make a bare path fail are escaped, not passed on.
        assert_eq!(
            file_url(Path::new(r"C:\a b\c#d%e.mp4")),
            "file:///C:/a%20b/c%23d%25e.mp4"
        );
    }
}
