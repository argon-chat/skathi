//! The Windows backend: `IMFMediaEngine` in frame-server mode.
//!
//! `IMFMediaEngine` is Microsoft's `HTMLMediaElement` (events below 1000 are the HTML 5 set) and
//! renders the audio itself, so A/V sync and the master clock are the OS's problem. Frame-server
//! mode (no window, no DirectComposition visual) hands decoded textures over through
//! `OnVideoStreamTick` + `TransferVideoFrame`, which keeps the video in our scene graph so
//! `border-radius`, `overflow`, `opacity` and z-order work.
//!
//! The engine decodes onto the `ID3D11Device` it is given through an `IMFDXGIDeviceManager`: the
//! renderer's own (`gpui_windows::renderer_d3d11_device()`) when published, else a private one.
//! Asking for `DXGI_FORMAT_B8G8R8A8_UNORM` makes MF insert its D3D11 video processor, which crops,
//! converts colour on the GPU and returns the format gpui's atlas uses.

mod codecs;
mod notify;
mod present;
mod source;
#[cfg(feature = "webgpu")]
mod webgpu;

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};

use windows::core::{Interface, BSTR, HSTRING};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine,
    IMFMediaEngineClassFactory, IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineWebSupport,
    MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateMFByteStreamOnStream, MFStartup,
    MFSTARTUP_FULL, MF_BYTESTREAM_CONTENT_TYPE, MF_MEDIA_ENGINE_CALLBACK,
    MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_EXTENSION, MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC,
    MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

use crate::backend::{
    CanPlay, FrameRequest, MediaError, MediaErrorCode, MediaEvent, MediaSource, NetworkState,
    ReadyState, VideoBackend, VideoFrame,
};
use crate::trace;

pub use notify::EngineNotify;

static SHARED_DEVICE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SHARED_GENERATION: AtomicU64 = AtomicU64::new(0);

/// An owned lease on a renderer generation, held only while cloning the D3D11 interface so a
/// concurrent device replacement cannot invalidate the raw pointer before AddRef.
pub struct SharedVideoDevice {
    pub device: *mut c_void,
    pub generation: u64,
    pub keep_alive: Arc<dyn std::any::Any + Send + Sync>,
}

pub type SharedDeviceProvider = fn() -> Result<Option<SharedVideoDevice>, String>;
static SHARED_DEVICE_PROVIDER: OnceLock<SharedDeviceProvider> = OnceLock::new();

/// Installs a device factory without opening a decoder or initializing D3D11; the provider runs
/// only when a backend needs a device.
///
/// # Safety
/// A successful provider lease must own a live ID3D11Device through keep_alive.
pub unsafe fn install_shared_device_provider(
    provider: SharedDeviceProvider,
) -> Result<(), &'static str> {
    SHARED_DEVICE_PROVIDER
        .set(provider)
        .map_err(|_| "video device provider already installed")
}

/// Publishes the renderer's `ID3D11Device` so decoded frames land on it. The pointer is borrowed and
/// cloned (AddRef) when an engine is built. Raw because `gpui_windows` uses `windows` 0.61 and this
/// crate 0.62: a COM pointer is an ABI, not a type.
///
/// # Safety
/// `ptr` must be a valid `ID3D11Device` COM pointer that outlives the process's
/// use of `<video>`, or null to clear.
pub unsafe fn set_shared_device(ptr: *mut c_void, generation: u64) {
    SHARED_DEVICE.store(ptr, Ordering::Release);
    SHARED_GENERATION.store(generation, Ordering::Release);
}

/// The renderer's D3D11 device and its generation, `None` before a renderer exists. The lease keeps
/// a native D3D12 renderer's D3D11On12 bridge alive; atlas uploads need that bridge to exist.
pub fn renderer_device() -> Result<Option<SharedVideoDevice>, String> {
    if let Some(provider) = SHARED_DEVICE_PROVIDER.get() {
        return provider();
    }
    let raw = SHARED_DEVICE.load(Ordering::Acquire);
    if raw.is_null() {
        return Ok(None);
    }
    // SAFETY: the host publishes a device it owns for the renderer's life.
    let Some(device) = (unsafe { ID3D11Device::from_raw_borrowed(&raw) }) else {
        return Ok(None);
    };
    let device = device.clone();
    let pointer = device.as_raw();
    Ok(Some(SharedVideoDevice {
        device: pointer,
        generation: SHARED_GENERATION.load(Ordering::Acquire),
        keep_alive: Arc::new(SendDevice(device)),
    }))
}

struct SendDevice(#[allow(dead_code)] ID3D11Device);
// SAFETY: the renderer's device is multithread-protected; this only holds a reference.
unsafe impl Send for SendDevice {}
unsafe impl Sync for SendDevice {}

///
/// # Safety
/// `texture` is null or a live COM pointer.
pub(crate) unsafe fn retain_texture(texture: *mut c_void) {
    // SAFETY: the caller's.
    if let Some(unknown) = unsafe { windows::core::IUnknown::from_raw_borrowed(&texture) } {
        std::mem::forget(unknown.clone());
    }
}

///
/// # Safety
/// `texture` is null or a pointer `retain_texture` was given.
pub(crate) unsafe fn release_texture(texture: *mut c_void) {
    if !texture.is_null() {
        // SAFETY: the caller's: it owns the reference this drops.
        drop(unsafe { windows::core::IUnknown::from_raw(texture) });
    }
}

/// The renderer's device if one was published (`bool` = true), else a private one built the same way.
fn device() -> Result<
    (
        ID3D11Device,
        bool,
        Option<Arc<dyn std::any::Any + Send + Sync>>,
    ),
    MediaError,
> {
    // `SK_VIDEO_DEVICE=private` forces the fallback, to tell whether device sharing is the problem without a rebuild.
    if std::env::var("SK_VIDEO_DEVICE").is_ok_and(|v| v == "private") {
        return Ok((private_device()?, false, None));
    }
    if let Some(provider) = SHARED_DEVICE_PROVIDER.get() {
        let lease =
            provider().map_err(|message| MediaError::new(MediaErrorCode::Decode, message))?;
        if let Some(lease) = lease {
            let borrowed =
                unsafe { ID3D11Device::from_raw_borrowed(&lease.device) }.ok_or_else(|| {
                    MediaError::new(
                        MediaErrorCode::Decode,
                        "renderer video provider returned a null D3D11 device",
                    )
                })?;
            return Ok((borrowed.clone(), true, Some(lease.keep_alive)));
        }
        // The provider owns its generations; the legacy cache's pointer may already be dead.
        return Ok((private_device()?, false, None));
    }
    let raw = SHARED_DEVICE.load(Ordering::Acquire);
    if !raw.is_null() {
        // SAFETY: the host publishes a device it owns for the process lifetime.
        if let Some(borrowed) = unsafe { ID3D11Device::from_raw_borrowed(&raw) } {
            return Ok((borrowed.clone(), true, None));
        }
    }
    Ok((private_device()?, false, None))
}

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
                 SK_ADAPTER=<name> to pick a different adapter.",
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

fn ensure_platform() -> Result<(), MediaError> {
    static ONCE: Once = Once::new();
    static mut RESULT: Option<MediaError> = None;
    ONCE.call_once(|| {
        unsafe {
            // The host thread may already be initialised in either apartment; `RPC_E_CHANGED_MODE` is fine.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                RESULT = Some(
                    MediaError::new(MediaErrorCode::Decode, format!("MFStartup failed: {e}"))
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

struct WebAudio {
    web: IMFMediaEngineWebSupport,
    provider: windows::Win32::Media::MediaFoundation::IAudioSourceProvider,
    output: std::sync::Weak<dyn crate::AudioOutput>,
    rate: u32,
    channels: u32,
    last: std::time::Instant,
    remainder: f64,
    samples: Vec<f32>,
}

impl WebAudio {
    // The provider has no COM proxy. Pull on the media engine's apartment.
    fn pump(&mut self, playing: bool) {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        let Some(output) = self.output.upgrade().filter(|_| playing) else {
            self.remainder = 0.0;
            return;
        };
        let due = elapsed.min(0.2) * f64::from(self.rate) + self.remainder;
        self.remainder = due.fract();
        let mut remaining = due as usize;
        while remaining > 0 {
            let frames = remaining.min(512);
            let mut channels = self.channels;
            self.samples.resize(frames * channels as usize, 0.0);
            let mut result = unsafe {
                self.provider.ProvideInput(
                    frames as u32,
                    &mut channels,
                    Some(self.samples.as_mut_ptr()),
                )
            };
            // A layout change returns the required width, even when the supplied buffer was wider.
            if result.is_err() && channels != self.channels && (1..=32).contains(&channels) {
                self.channels = channels;
                self.samples.resize(frames * channels as usize, 0.0);
                result = unsafe {
                    self.provider.ProvideInput(
                        frames as u32,
                        &mut channels,
                        Some(self.samples.as_mut_ptr()),
                    )
                };
            }
            if result.is_ok() && channels > 0 && channels <= self.channels {
                output.push(
                    &self.samples[..frames * channels as usize],
                    channels as usize,
                    self.rate,
                    1.0,
                );
            }
            remaining -= frames;
        }
    }
}

pub struct MediaEngineBackend {
    engine: IMFMediaEngine,
    web_audio: Option<WebAudio>,
    _notify: IMFMediaEngineNotify,
    queue: Arc<Mutex<Vec<u32>>>,
    presenter: present::Presenter,
    /// Timestamp last sent to the atlas, in 100 ns units.
    last_pts: i64,
    current_pts: Option<i64>,
    #[cfg(feature = "webgpu")]
    webgpu_snapshot: Option<redjs_webgpu::video::Frame>,
    shared_device: bool,
    /// The bytes of a `data:` source handed to the engine in memory and not yet proved to work:
    /// `SetSourceFromByteStream` reports its verdict asynchronously, so [`Self::retry_bytes_from_file`]
    /// hangs off the error. Cleared at `loadedmetadata` and after one retry.
    bytes_retry: Option<(String, Arc<Vec<u8>>)>,
    /// Whether `play()` was called more recently than `pause()` (`GetPaused` cannot answer across a reload).
    wants_playing: bool,
    current_src: String,
    _source_extension: windows::Win32::Media::MediaFoundation::IMFMediaEngineExtension,
    source_control: source::Control,
    /// Last field: drops the host video facade only after the engine and texture COM references are gone.
    _renderer_owner: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

impl MediaEngineBackend {
    pub fn new() -> Result<Self, MediaError> {
        ensure_platform()?;
        let (device, shared_device, renderer_owner) = device()?;

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
                 created with D3D11_CREATE_DEVICE_VIDEO_SUPPORT.",
            )
        })?;

        let queue: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let notify: IMFMediaEngineNotify = EngineNotify::new(queue.clone()).into();

        let mut attributes: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attributes, 4) }.map_err(|e| {
            MediaError::new(MediaErrorCode::Decode, format!("MFCreateAttributes: {e}"))
        })?;
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
            MediaError::new(
                MediaErrorCode::Decode,
                format!("media engine attributes: {e}"),
            )
        })?;

        let factory: IMFMediaEngineClassFactory = unsafe {
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
        }
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

        let (source_extension, source_control) = source::extension();
        unsafe { attributes.SetUnknown(&MF_MEDIA_ENGINE_EXTENSION, &source_extension) }.map_err(
            |error| {
                MediaError::new(
                    MediaErrorCode::Decode,
                    format!("media source extension: {error}"),
                )
            },
        )?;
        let engine = unsafe { factory.CreateInstance(0, &attributes) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::Decode,
                format!("IMFMediaEngineClassFactory::CreateInstance: {e}"),
            )
        })?;

        Ok(Self {
            web_audio: None,
            engine,
            _notify: notify,
            queue,
            presenter: present::Presenter::new(device),
            last_pts: i64::MIN,
            current_pts: None,
            #[cfg(feature = "webgpu")]
            webgpu_snapshot: None,
            shared_device,
            bytes_retry: None,
            wants_playing: false,
            current_src: String::new(),
            _source_extension: source_extension,
            source_control,
            _renderer_owner: renderer_owner,
        })
    }

    fn set_source_url(&self, url: &str) -> Result<(), MediaError> {
        unsafe { self.engine.SetSource(&BSTR::from(url)) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("could not set the media source: {e}"),
            )
        })
    }

    /// `data:` bytes. `SetSourceFromByteStream` is not the default: MF opens the `url` argument
    /// instead of reading the stream, so a stream with no URL cannot be served. The default stages a
    /// temp file named by content hash, not deleted because MF reads it lazily while the element
    /// lives. `SK_VIDEO_DATA_BYTESTREAM=1` re-enables the byte-stream attempt.
    fn set_source_bytes(&mut self, mime: &str, data: &Arc<Vec<u8>>) -> Result<(), MediaError> {
        if std::env::var("SK_VIDEO_DATA_BYTESTREAM").is_ok_and(|v| v == "1") {
            match self.set_source_from_memory(mime, data) {
                Ok(()) => {
                    self.bytes_retry = Some((mime.to_string(), data.clone()));
                    return Ok(());
                }
                Err(e) => trace(format_args!(
                    "  SetSourceFromByteStream failed synchronously ({e}); staging a temp file"
                )),
            }
        }
        self.bytes_retry = None;
        let path = temp_file_for(mime, data).map_err(|e| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("a data: URI could not be staged to a temp file: {e}"),
            )
            .with_advisory(
                "<video src=\"data:…\">: the bytes could not be written to the temp directory, \
                 and Media Foundation cannot take them any other way (see `set_source_bytes`). \
                 Use a file path.",
            )
        })?;
        trace(format_args!("  staged to {}", path.display()));
        self.set_source_url(&file_url(&path))
    }

    /// Second attempt at a `data:` source the byte stream refused asynchronously: stage the bytes as
    /// a file and load that. `true` when a retry started, so the triggering error is swallowed.
    fn retry_bytes_from_file(&mut self) -> bool {
        let Some((mime, data)) = self.bytes_retry.take() else {
            return false;
        };
        // The reload resets the engine to paused/0, so re-assert what the element asked for or an
        // `autoplay` clip sits on its first frame.
        let was_playing = self.wants_playing;
        trace(format_args!(
            "  in-memory byte stream refused: {:?}",
            self.engine_error().map(|e| e.message)
        ));
        let Ok(path) = temp_file_for(&mime, &data) else {
            trace(format_args!("  data: retry could not write a temp file"));
            return false;
        };
        trace(format_args!("  retrying from {}", path.display()));
        self.reset_picture();
        self.presenter.reset();
        if self.set_source_url(&file_url(&path)).is_err() {
            return false;
        }
        let _ = unsafe { self.engine.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC) };
        if unsafe { self.engine.Load() }.is_err() {
            return false;
        }
        if was_playing {
            let _ = unsafe { self.engine.Play() };
        }
        true
    }

    fn set_source_from_memory(&self, mime: &str, data: &[u8]) -> windows::core::Result<()> {
        use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
        use windows::Win32::System::Memory::{
            GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
        };

        unsafe {
            // `fDeleteOnRelease: TRUE` gives the HGLOBAL to the stream, so it lives as long as the engine's reference.
            let global = GlobalAlloc(GMEM_MOVEABLE, data.len())?;
            let locked = GlobalLock(global);
            if locked.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), locked as *mut u8, data.len());
            let _ = GlobalUnlock(global);
            let stream = CreateStreamOnHGlobal(global, true)?;
            let byte_stream = MFCreateMFByteStreamOnStream(&stream)?;

            if !mime.is_empty() {
                if let Ok(attributes) = byte_stream.cast::<IMFAttributes>() {
                    let _ = attributes.SetString(&MF_BYTESTREAM_CONTENT_TYPE, &HSTRING::from(mime));
                }
            }
            // Never `BSTR::new()`: a NULL BSTR crashes `mfmediaengine.dll` with no panic or message.
            let url = format!("file:///sk-media{}", extension_for(mime));
            let engine_ex: IMFMediaEngineEx = self.engine.cast()?;
            engine_ex.SetSourceFromByteStream(&byte_stream, &BSTR::from(url.as_str()))
        }
    }

    pub fn uses_renderer_device(&self) -> bool {
        self.shared_device
    }

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
            5 => MediaError::new(MediaErrorCode::Decode, "the media is encrypted").with_advisory(
                "<video>: encrypted media (EME / PlayReady / Widevine) is not supported and \
                     is not planned.",
            ),
            // 4 = SRC_NOT_SUPPORTED and everything unknown. MF also reports that for a network source
            // that never arrived (measured: a dead port), so answer MEDIA_ERR_NETWORK as a browser does.
            _ if src.starts_with("http://") || src.starts_with("https://") => MediaError::new(
                MediaErrorCode::Network,
                format!("the source could not be opened{extended}"),
            )
            .with_advisory(format!(
                "<video src=\"{src}\">: Media Foundation could not open the URL. It reports \
                 \"not supported\" for every network failure, so this is most likely the server \
                 rather than the codec: an unreachable host, a 404, a redirect to a scheme it \
                 does not speak, or an HTML error page served with 200. A file that IS \
                 reachable but undecodable reports the same code — load it from disk once to \
                 tell the two apart."
            )),
            _ => MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("the source is not supported{extended}"),
            )
            .with_advisory(self.unsupported_advisory()),
        })
    }

    /// The advisory for "this machine cannot play this file", from a live registry probe (a Store codec package can appear after shipping).
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
            if installed.is_empty() {
                "none".to_string()
            } else {
                installed.join(", ")
            }
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
        let Ok(ranges) = ranges else {
            return Vec::new();
        };
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

/// `IMFMediaEngine::Shutdown` before the last reference goes, as the API requires: otherwise the
/// pipeline (source, decoder MFTs, work-queue threads) outlives every reference we drop, ~3.5 MB of
/// heap and 2-3 threads per unmounted `<video>`.
impl Drop for MediaEngineBackend {
    fn drop(&mut self) {
        if let Some(web) = self.web_audio.take() {
            let _ = unsafe { web.web.DisconnectWebAudio() };
        }
        if let Err(e) = unsafe { self.engine.Shutdown() } {
            trace(format_args!("Shutdown failed: {e}"));
        }
    }
}

impl MediaEngineBackend {
    fn reset_picture(&mut self) {
        self.last_pts = i64::MIN;
        self.current_pts = None;
        #[cfg(feature = "webgpu")]
        {
            self.webgpu_snapshot = None;
        }
    }

    fn picture_time(&mut self) -> Option<i64> {
        let mut pts = 0;
        // The generated wrapper maps S_FALSE to Ok(0). Only S_OK identifies an actual
        // decoded picture, including the first legitimate presentation timestamp of zero.
        let status =
            unsafe { (self.engine.vtable().OnVideoStreamTick)(self.engine.as_raw(), &mut pts) };
        if status == windows::Win32::Foundation::S_OK {
            self.current_pts = Some(pts);
        }
        status.ok().ok()?;
        self.current_pts
    }
}

impl VideoBackend for MediaEngineBackend {
    fn route_audio(&mut self, output: &Arc<dyn crate::AudioOutput>) -> Result<(), MediaError> {
        let failed = |e| {
            MediaError::new(
                MediaErrorCode::Decode,
                format!("Media Foundation could not route audio to Web Audio: {e}"),
            )
        };
        let web: IMFMediaEngineWebSupport = self.engine.cast().map_err(failed)?;
        let provider = unsafe { web.ConnectWebAudio(output.sample_rate()) }.map_err(failed)?;
        self.web_audio = Some(WebAudio {
            web,
            provider,
            output: Arc::downgrade(output),
            rate: output.sample_rate(),
            channels: 1,
            last: std::time::Instant::now(),
            remainder: 0.0,
            samples: Vec::new(),
        });
        Ok(())
    }

    fn load(&mut self, source: &MediaSource) -> Result<(), MediaError> {
        self.reset_picture();
        self.presenter.reset();
        if !matches!(source, MediaSource::StaticBytes { .. }) {
            self.source_control.clear();
        }
        match source {
            MediaSource::Path(path) => {
                let url = file_url(path);
                trace(format_args!(
                    "load {url}  (shared device: {})",
                    self.shared_device
                ));
                self.current_src = path.display().to_string();
                self.set_source_url(&url).map_err(|e| {
                    e.with_advisory(format!(
                        "<video src=\"{}\">: Media Foundation refused the URL. Local file \
                         paths and file:// URLs are what this build supports.",
                        path.display()
                    ))
                })?;
            }
            // Handed to MF verbatim: its scheme handler reads ahead and seeks by range request.
            MediaSource::Uri(url) => {
                trace(format_args!("load {url}  (MF network source)"));
                self.current_src = url.clone();
                self.set_source_url(url).map_err(|e| {
                    e.with_advisory(format!(
                        "<video src=\"{url}\">: Media Foundation refused the URL. It handles \
                         http(s) through its own network source, so this is the server's answer \
                         rather than a missing feature — check the URL, and note that a redirect \
                         to a scheme MF does not know, or an HTML error page served with 200, \
                         fails here rather than at the socket."
                    ))
                })?;
            }
            MediaSource::StaticBytes { mime, data } => {
                trace(format_args!(
                    "load package: {} bytes, mapped native source",
                    data.len()
                ));
                self.bytes_retry = None;
                self.current_src = format!("package:{mime} ({} bytes)", data.len());
                let url = self.source_control.select(mime, data);
                self.set_source_url(&url)?;
            }
            MediaSource::Bytes { mime, data } => {
                trace(format_args!(
                    "load data: {} bytes, mime {mime:?}",
                    data.len()
                ));
                self.current_src = format!("data:{mime} ({} bytes)", data.len());
                self.set_source_bytes(mime, data)?;
            }
        }
        // Unlike the DOM, `SetSource` alone leaves `networkState` at EMPTY: the engine must be told
        // to `Load()`. `SetPreload(AUTOMATIC)` fetches the whole file.
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
        self.wants_playing = true;
        let _ = unsafe { self.engine.Play() };
    }

    fn pause(&mut self) {
        self.wants_playing = false;
        let _ = unsafe { self.engine.Pause() };
    }

    fn seek(&mut self, seconds: f64) {
        if unsafe { self.engine.SetCurrentTime(seconds) }.is_ok() {
            self.reset_picture();
        }
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

    fn audible(&self) -> bool {
        unsafe {
            self.web_audio.is_none()
                && self.engine.HasAudio().as_bool()
                && !self.engine.IsPaused().as_bool()
                && !self.engine.IsEnded().as_bool()
                && !self.engine.GetMuted().as_bool()
                && self.engine.GetVolume() > 0.0
        }
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

    /// Not `IMFMediaEngine::CanPlayType`, which answers on the MIME string's shape ("probably" with no
    /// source or decoder); `codecs.rs` asks `MFTEnumEx` instead.
    fn can_play_type(&self, mime_type: &str) -> CanPlay {
        codecs::can_play_type(mime_type)
    }

    #[cfg(feature = "webgpu")]
    fn webgpu_frame(&mut self) -> Option<redjs_webgpu::video::Frame> {
        let size = self.native_size()?;
        let timestamp = self.picture_time()? / 10;
        if let Some(frame) = &self.webgpu_snapshot {
            if (frame.width, frame.height, frame.timestamp) == (size.0, size.1, timestamp) {
                return Some(frame.clone());
            }
        }
        let frame = self.presenter.snapshot(&self.engine, size, timestamp)?;
        self.webgpu_snapshot = Some(frame.clone());
        Some(frame)
    }

    fn present(&mut self, request: &FrameRequest) -> Option<VideoFrame> {
        let native = self.native_size()?;
        let size = crate::fit_within(native, request.max_size);
        if size.0 == 0 || size.1 == 0 {
            return None;
        }

        let pts = self.picture_time()?;
        let forced = self.last_pts == i64::MIN || self.presenter.size() != size;
        if !forced && pts == self.last_pts {
            return None;
        }
        self.last_pts = pts;

        if self.presenter.is_gpu() {
            let (image, frame_pts, gpu) = self.presenter.transfer_gpu(&self.engine, size, pts)?;
            return Some(VideoFrame {
                image,
                pts: frame_pts as f64 / 10_000_000.0,
                size,
                gpu: Some(gpu),
            });
        }

        // Pipeline only while PLAYING: a paused element's seek frame must be the right picture.
        let pipeline = !self.paused();
        let (image, frame_pts) = self.presenter.transfer(&self.engine, size, pts, pipeline)?;
        Some(VideoFrame {
            image,
            pts: frame_pts as f64 / 10_000_000.0,
            size,
            gpu: None,
        })
    }

    fn gpu_upload_failed(&mut self) {
        self.presenter.demote_to_readback();
        // The next `present` must produce a frame even if the PTS has not moved.
        self.last_pts = i64::MIN;
    }

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        let playing = !self.paused() && !self.ended();
        if let Some(audio) = &mut self.web_audio {
            audio.pump(playing)
        }
        let raw = {
            let Ok(mut queue) = self.queue.lock() else {
                return;
            };
            std::mem::take(&mut *queue)
        };
        for code in raw {
            let Some(event) = notify::translate(code) else {
                continue;
            };
            // A `data:` source the byte stream could not open gets one more try from a temp file.
            if event == MediaEvent::Error && self.bytes_retry.is_some() {
                if self.retry_bytes_from_file() {
                    continue;
                }
            }
            if event == MediaEvent::LoadedMetadata {
                self.bytes_retry = None;
            }
            sink(event);
        }
    }

    fn suspend(&mut self) {
        self.pause();
        self.presenter.reset();
        self.reset_picture();
        // Not `SetSource("")`: rebuilding the pipeline in `resume` grows the process ~5 MiB per
        // park/unpark cycle. A bounded resident cost beats an unbounded one, so the decoder stays.
    }
}

/// The file extension MF's source resolver should see for a `data:` URI's media type (picks a byte-stream handler).
fn extension_for(mime: &str) -> &'static str {
    match mime.trim() {
        "video/mp4" | "video/x-m4v" | "audio/mp4" => ".mp4",
        "video/quicktime" => ".mov",
        "video/webm" | "audio/webm" => ".webm",
        "video/ogg" | "application/ogg" => ".ogv",
        "audio/ogg" => ".ogg",
        "video/x-matroska" => ".mkv",
        "video/x-ms-wmv" | "video/x-ms-asf" => ".wmv",
        "video/mpeg" => ".mpg",
        "video/avi" | "video/x-msvideo" => ".avi",
        "audio/mpeg" | "audio/mp3" => ".mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => ".wav",
        "audio/flac" | "audio/x-flac" => ".flac",
        "audio/aac" | "audio/aacp" => ".aac",
        _ => "",
    }
}

/// Stages `data:` bytes as a file named by their content hash, so the same URI mounted twice writes
/// once. Not deleted: MF reads it lazily for as long as the element lives.
fn temp_file_for(mime: &str, data: &[u8]) -> std::io::Result<std::path::PathBuf> {
    // FNV-1a: a collision costs the wrong video, not memory safety.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    let ext = match extension_for(mime) {
        "" => ".bin",
        other => other,
    };
    let path = std::env::temp_dir().join(format!("sk-video-{hash:016x}{ext}"));
    let already_there = path
        .metadata()
        .is_ok_and(|m| m.len() as usize == data.len());
    if !already_there {
        std::fs::write(&path, data)?;
    }
    Ok(path)
}

/// `C:\x\a.mp4` → `file:///C:/x/a.mp4`, percent-escaping what a URL must (a bare path fails with `#` or `%`).
fn file_url(path: &Path) -> String {
    let raw = path.to_string_lossy();
    // Strip extended-length prefixes (from `Path::canonicalize`): MF fails such URLs with `ERROR_INVALID_NAME`.
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
        assert_eq!(
            file_url(Path::new(r"C:\a b\c#d%e.mp4")),
            "file:///C:/a%20b/c%23d%25e.mp4"
        );
    }
}
