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
    MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
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
    /// Written from a LIVE probe, never a table: what is decodable here is a
    /// property of this machine's installed packages, not of the build.
    fn unsupported_advisory(&self) -> String {
        let src = &self.current_src;
        let probe = |mime: &str| self.can_play_type(mime) != CanPlay::No;
        let mut missing: Vec<&str> = Vec::new();
        if !probe("video/mp4; codecs=\"hev1.1.6.L93.B0\"") {
            missing.push("HEVC/H.265 (Microsoft.HEVCVideoExtension)");
        }
        if !probe("video/mp4; codecs=\"av01.0.05M.08\"") {
            missing.push("AV1 (Microsoft.AV1VideoExtension)");
        }
        if !probe("video/webm; codecs=\"vp9\"") {
            missing.push("VP9/WebM (Microsoft.VP9VideoExtensions + Microsoft.WebMediaExtensions)");
        }
        let baseline = if probe("video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"") {
            "H.264 + AAC in MP4 does work on this machine"
        } else {
            "even H.264 + AAC in MP4 is unavailable here, which means the Windows media stack \
             itself is missing (N/KN edition without the Media Feature Pack)"
        };
        let mut out = format!(
            "<video src=\"{src}\"> could not be played: no decoder on THIS machine handles it. \
             {baseline}."
        );
        if !missing.is_empty() {
            out.push_str(&format!(
                " Not installed here: {}. Either install the extension or re-encode the clip to \
                 H.264 (`ffmpeg -i in -c:v libx264 -profile:v high -pix_fmt yuv420p -c:a aac \
                 out.mp4`), which needs nothing extra anywhere.",
                missing.join(", ")
            ));
        } else {
            out.push_str(
                " Every codec probed IS installed, so the file itself is likely damaged or uses \
                 a container Media Foundation does not read (MP4/MOV, MKV/WebM, AVI, ASF are the \
                 ones it does).",
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

impl VideoBackend for MediaEngineBackend {
    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError> {
        let MediaSource::Path(path) = source;
        let url = file_url(path);
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
        // `SetSource` starts the resource-selection algorithm by itself; `Load`
        // is what re-runs it, and calling it here would fire a second
        // `loadstart`.
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

    fn can_play_type(&self, mime_type: &str) -> CanPlay {
        let Ok(answer) = (unsafe { self.engine.CanPlayType(&BSTR::from(mime_type)) }) else {
            return CanPlay::No;
        };
        match answer.0 {
            2 => CanPlay::Probably,
            1 => CanPlay::Maybe,
            _ => CanPlay::No,
        }
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

        let image = self.presenter.transfer(&self.engine, size)?;
        Some(VideoFrame {
            image,
            // MF reports the presentation time in 100 ns units.
            pts: pts as f64 / 10_000_000.0,
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
        self.presenter.reset();
        // `SetSource("")` tears the pipeline down (the spec's "media element
        // load algorithm" with no source), releasing the decoder, the DPB and
        // the video-processor pool — ~28 MiB of RAM and ~68 MiB of VRAM per
        // 1080p stream. `load` puts it back.
        let _ = unsafe { self.engine.SetSource(&BSTR::new()) };
    }
}

/// `C:\x\a.mp4` → `file:///C:/x/a.mp4`, percent-escaping what a URL must.
///
/// Media Foundation accepts a bare path too, but only sometimes — a path with
/// a `#` or a `%` in it is read as a URL fragment/escape and the open fails
/// with a "file not found" that names a file the user can see is there.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
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
