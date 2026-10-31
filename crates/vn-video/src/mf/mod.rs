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

use windows::core::{Interface, BSTR, HSTRING};
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
    MF_BYTESTREAM_CONTENT_TYPE,
    CLSID_MFMediaEngineClassFactory, IMFMediaEngineEx, MFCreateMFByteStreamOnStream,
    MFSTARTUP_FULL, MF_MEDIA_ENGINE_CALLBACK,
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
    /// The bytes of a `data:` source that was handed to the engine IN MEMORY
    /// and has not yet been proved to work.
    ///
    /// `SetSourceFromByteStream` reports its verdict asynchronously, like every
    /// other load: it returns `S_OK` and the engine raises `MF_MEDIA_ENGINE_
    /// EVENT_ERROR` a moment later if the resolver could not make sense of the
    /// stream. So the temp-file fallback cannot be a `?` on the call — it has
    /// to hang off the error, which is what [`Self::retry_bytes_from_file`]
    /// does. Cleared the moment the load succeeds far enough to raise
    /// `loadedmetadata`, and after one retry either way.
    bytes_retry: Option<(String, Arc<Vec<u8>>)>,
    /// Has `play()` been called more recently than `pause()`? The engine's own
    /// `GetPaused` cannot answer this across a reload, and the reload in
    /// [`Self::retry_bytes_from_file`] has to put the transport back.
    wants_playing: bool,
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
            bytes_retry: None,
            wants_playing: false,
            current_src: String::new(),
        })
    }

    /// `SetSource(url)` — the whole of what `Path` and `Uri` need.
    fn set_source_url(&self, url: &str) -> Result<(), MediaError> {
        unsafe { self.engine.SetSource(&BSTR::from(url)) }.map_err(|e| {
            MediaError::new(
                MediaErrorCode::SrcNotSupported,
                format!("could not set the media source: {e}"),
            )
        })
    }

    /// `data:` — the bytes without a file.
    ///
    /// # The in-memory path, and why it is not the default
    ///
    /// `IMFMediaEngineEx::SetSourceFromByteStream(stream, url)` looks like the
    /// answer: `MFCreateMFByteStreamOnStream` wraps any `IStream` (and
    /// `CreateStreamOnHGlobal` makes one over a block of memory), so the whole
    /// path would be HGLOBAL → `IStream` → `IMFByteStream` → engine, with the
    /// bytes copied once and never touching the filesystem. It is what m12's
    /// report named as the shape of this feature.
    ///
    /// It does not work, and the reason is the second argument. [measured, this
    /// machine, a 6 KB baseline-H.264 MP4 that plays fine from a file]
    ///
    /// | `url` passed | result |
    /// | --- | --- |
    /// | `"video/mp4"` (the MIME type — the obvious misreading) | `MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED` |
    /// | `"file:///vn-media.mp4"` (synthesised, right extension) | **`0x80070002 ERROR_FILE_NOT_FOUND`** |
    /// | `""` | `0x80004003 E_POINTER`, synchronously |
    ///
    /// The middle row is the finding: MF **opens the URL** rather than reading
    /// the stream it was just handed, so the parameter is not a hint — it is
    /// the source. A byte stream that has no URL therefore has nothing to put
    /// there, and `SetSourceFromByteStream` cannot serve one.
    ///
    /// So the default is the temp file, which always works. `MF_BYTESTREAM_
    /// CONTENT_TYPE` and the extension logic are kept because they are correct
    /// and cost nothing, and the byte-stream attempt is kept behind
    /// `VN_VIDEO_DATA_BYTESTREAM=1` so a future Windows build (or a media type
    /// whose handler behaves differently) can be re-tested in one environment
    /// variable rather than one rebuild.
    ///
    /// The temp file is named by the content hash, so re-mounting the same
    /// `data:` URI writes once, and it is left in the OS temp directory rather
    /// than deleted — Media Foundation reads it lazily for as long as the
    /// element lives, so deleting it on return would break playback rather than
    /// tidy up.
    fn set_source_bytes(&mut self, mime: &str, data: &Arc<Vec<u8>>) -> Result<(), MediaError> {
        if std::env::var("VN_VIDEO_DATA_BYTESTREAM").is_ok_and(|v| v == "1") {
            match self.set_source_from_memory(mime, data) {
                Ok(()) => {
                    // It returned S_OK; the engine's verdict arrives later, and
                    // `retry_bytes_from_file` is what catches a refusal.
                    self.bytes_retry = Some((mime.to_string(), data.clone()));
                    return Ok(());
                }
                Err(e) => trace(format_args!(
                    "  SetSourceFromByteStream failed synchronously ({e}); staging a temp file"
                )),
            }
        }
        // Straight to the file: there is no later attempt to make.
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

    /// Second attempt at a `data:` source that went out as a byte stream and
    /// was refused ASYNCHRONOUSLY: stage the bytes as a file and load that.
    /// Returns `true` when a retry actually started, so the error that
    /// triggered it should be swallowed rather than raised at the element.
    ///
    /// Only reachable under `VN_VIDEO_DATA_BYTESTREAM=1` — the default path
    /// stages the file up front, because the byte-stream API cannot serve a
    /// stream with no URL (see [`Self::set_source_bytes`]).
    fn retry_bytes_from_file(&mut self) -> bool {
        let Some((mime, data)) = self.bytes_retry.take() else {
            return false;
        };
        // The reload runs the load algorithm again, which puts the engine back
        // at paused/0. Whatever the element already asked for has to be
        // re-asserted afterwards, or an `autoplay` clip sits on its first frame.
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
        self.last_pts = i64::MIN;
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
        use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

        unsafe {
            // `fDeleteOnRelease: TRUE` hands ownership of the HGLOBAL to the
            // stream, so the block lives exactly as long as the engine's
            // reference to it — which is longer than this function.
            let global = GlobalAlloc(GMEM_MOVEABLE, data.len())?;
            let locked = GlobalLock(global);
            if locked.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), locked as *mut u8, data.len());
            let _ = GlobalUnlock(global);
            let stream = CreateStreamOnHGlobal(global, true)?;
            let byte_stream = MFCreateMFByteStreamOnStream(&stream)?;

            // The MIME type goes where it belongs: `MF_BYTESTREAM_CONTENT_TYPE`
            // on the stream, which is the same attribute MF's own network
            // source sets from a `Content-Type` header.
            if !mime.is_empty() {
                if let Ok(attributes) = byte_stream.cast::<IMFAttributes>() {
                    let _ =
                        attributes.SetString(&MF_BYTESTREAM_CONTENT_TYPE, &HSTRING::from(mime));
                }
            }
            // The best of three bad options — see `set_source_bytes` for the
            // measurements. Never `BSTR::new()`: that is a NULL BSTR, and
            // handing one to `mfmediaengine.dll` takes the process down with
            // no panic and no message (m12 §6.6 found that the hard way).
            let url = format!("file:///vn-media{}", extension_for(mime));
            let engine_ex: IMFMediaEngineEx = self.engine.cast()?;
            engine_ex.SetSourceFromByteStream(&byte_stream, &BSTR::from(url.as_str()))
        }
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
            // 4 = MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED, and everything
            // unknown.
            //
            // A NETWORK source that never arrived lands here too — MF reports
            // "not supported" for a URL it could not open, which is technically
            // true and completely unhelpful. [measured: a `<video>` pointed at
            // a dead port reported code 4 with the codec advisory, blaming this
            // machine's decoder set for a connection that was refused.] A
            // browser answers MEDIA_ERR_NETWORK there, so we do too, and the
            // advisory says what actually happened.
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

/// `IMFMediaEngine::Shutdown` before the last reference goes, as the API
/// requires ("call this method when you are done using the Media Engine,
/// before calling Release").
///
/// Without it, releasing the engine frees almost nothing. Its pipeline (the
/// media source, the decoder MFTs, their work-queue threads and buffers) holds
/// references back to it, so the cycle outlives every reference we drop. In
/// the Argon client each channel switch unmounts and mounts `<video>`s, and
/// every one left behind ~3.5 MB of process heap and two to three threads:
/// 145 → 283 threads and +176 MB of C/C++ heap over 40 switches, and then
/// the process exited (research/reports/m26/perf-mem-3.md §1).
impl Drop for MediaEngineBackend {
    fn drop(&mut self) {
        if let Err(e) = unsafe { self.engine.Shutdown() } {
            trace(format_args!("Shutdown failed: {e}"));
        }
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
        self.last_pts = i64::MIN;
        self.presenter.reset();
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
            // Handed to MF verbatim. Its scheme handler reads ahead and seeks
            // by HTTP range request, so a long clip starts playing while it is
            // still arriving and never becomes a file or a `Vec<u8>` — the same
            // reason `Path` is a path and not bytes.
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
            // A `data:` URI. The bytes are already in memory, so they are
            // wrapped as an `IMFByteStream` over an `IStream` rather than
            // written out — see `set_source_bytes`.
            MediaSource::Bytes { mime, data } => {
                trace(format_args!(
                    "load data: {} bytes, mime {mime:?}",
                    data.len()
                ));
                self.current_src = format!("data:{mime} ({} bytes)", data.len());
                self.set_source_bytes(mime, data)?;
            }
        }
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
        self.wants_playing = true;
        let _ = unsafe { self.engine.Play() };
    }

    fn pause(&mut self) {
        self.wants_playing = false;
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

        if self.presenter.is_gpu() {
            let (image, frame_pts, gpu) = self.presenter.transfer_gpu(&self.engine, size, pts)?;
            return Some(VideoFrame {
                image,
                // MF reports the presentation time in 100 ns units.
                pts: frame_pts as f64 / 10_000_000.0,
                size,
                gpu: Some(gpu),
            });
        }

        // Pipeline only while PLAYING. A paused element produces one frame per
        // seek with nothing behind it, so a frame of latency there is not
        // latency at all — it is the wrong picture, permanently. See
        // `Presenter::transfer`.
        let pipeline = !self.paused();
        let (image, frame_pts) = self.presenter.transfer(&self.engine, size, pts, pipeline)?;
        Some(VideoFrame {
            image,
            pts: frame_pts as f64 / 10_000_000.0,
            size,
            gpu: None,
        })
    }

    /// The renderer's atlas refused the zero-copy blit; fall back permanently.
    fn gpu_upload_failed(&mut self) {
        self.presenter.demote_to_readback();
        // The next `present` must produce a frame even though the PTS has not
        // moved, or a paused element would sit on the transparent placeholder
        // for ever.
        self.last_pts = i64::MIN;
    }

    fn drain_events(&mut self, sink: &mut dyn FnMut(MediaEvent)) {
        let raw = {
            let Ok(mut queue) = self.queue.lock() else { return };
            std::mem::take(&mut *queue)
        };
        for code in raw {
            let Some(event) = notify::translate(code) else { continue };
            // A `data:` source that the in-memory byte stream could not open
            // gets exactly one more try, from a temp file. The error is
            // SWALLOWED for that attempt: the element is not broken yet, and
            // raising `error` here would fire a JS handler and set
            // `MediaError` for a source that is about to start playing.
            if event == MediaEvent::Error && self.bytes_retry.is_some() {
                if self.retry_bytes_from_file() {
                    continue;
                }
            }
            // Anything that means the load got somewhere retires the retry:
            // whatever path we are on is working.
            if event == MediaEvent::LoadedMetadata {
                self.bytes_retry = None;
            }
            sink(event);
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

/// `C:\x\a.mp4` → `file:/// The filename extension Media Foundation's source resolver should see for a
/// `data:` URI's media type.
///
/// Not a codec table and not a claim about what plays — it is a HINT that picks
/// a byte-stream handler, and the handler then reads the actual bytes. An
/// unknown type gets no extension, which leaves `MF_BYTESTREAM_CONTENT_TYPE`
/// and content sniffing to do the work, exactly as an unknown `Content-Type`
/// would over http.
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

/// Stages `data:` bytes as a file, named by their content hash so the same URI
/// mounted twice writes once.
///
/// Deliberately NOT deleted on return: Media Foundation opens the file lazily
/// and keeps reading it for as long as the element lives, so removing it here
/// would break playback rather than tidy up. It lands in the OS temp directory,
/// which is what that directory is for, and is reused across process runs (the
/// name is a pure function of the bytes).
fn temp_file_for(mime: &str, data: &[u8]) -> std::io::Result<std::path::PathBuf> {
    // FNV-1a. A collision here costs the wrong video, not memory safety, and at
    // 64 bits with a handful of clips per app it is not a risk anyone meets.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    // The extension is a real hint here, not decoration: the source resolver
    // picks a byte-stream handler from it before anything reads the bytes.
    let ext = match extension_for(mime) {
        "" => ".bin",
        other => other,
    };
    let path = std::env::temp_dir().join(format!("vn-video-{hash:016x}{ext}"));
    let already_there = path.metadata().is_ok_and(|m| m.len() as usize == data.len());
    if !already_there {
        std::fs::write(&path, data)?;
    }
    Ok(path)
}

///C:/x/a.mp4`, percent-escaping what a URL must.
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
