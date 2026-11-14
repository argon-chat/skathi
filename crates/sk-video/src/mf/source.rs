//! Per-engine, in-process packaged media source. The image owns the bytes;
//! Media Foundation asks a seekable IStream for only the ranges it needs.
//! No registry scheme, whole-asset allocation, temp file or server is involved.
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
use windows::core::{implement, IUnknown, Interface, OutRef, Ref, Result, BOOL, BSTR, HRESULT};
use windows::Win32::Foundation::{
    E_INVALIDARG, E_NOTIMPL, E_POINTER, STG_E_ACCESSDENIED, STG_E_INVALIDFUNCTION, S_FALSE, S_OK,
};
use windows::Win32::Media::MediaFoundation::{
    IMFAsyncCallback, IMFAsyncCallback_Impl, IMFAsyncResult, IMFAttributes, IMFByteStream,
    IMFMediaEngineExtension, IMFMediaEngineExtension_Impl, MFCreateAsyncResult,
    MFCreateMFByteStreamOnStream, MFInvokeCallback, MFPutWorkItem, MFASYNC_CALLBACK_QUEUE_STANDARD,
    MF_BYTESTREAM_CONTENT_TYPE, MF_E_INVALIDREQUEST, MF_E_OPERATION_CANCELLED,
    MF_E_UNSUPPORTED_SCHEME, MF_MEDIA_ENGINE_CANPLAY, MF_OBJECT_BYTESTREAM, MF_OBJECT_TYPE,
};
use windows::Win32::System::Com::{
    IAgileObject, IAgileObject_Impl, ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE,
    STATFLAG, STATSTG, STGC, STGM_READ, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR,
    STREAM_SEEK_END, STREAM_SEEK_SET,
};

#[derive(Default)]
struct ReadStats {
    calls: AtomicU64,
    bytes: AtomicU64,
    seeks: AtomicU64,
    largest: AtomicU64,
}
#[derive(Clone)]
struct Source {
    url: String,
    mime: String,
    bytes: &'static [u8],
    stats: Arc<ReadStats>,
}
#[derive(Clone, Default)]
pub(super) struct Control(Arc<Mutex<Option<Source>>>);
impl Control {
    pub(super) fn select(&self, mime: &str, bytes: &'static [u8]) -> String {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let url = format!(
            "sk-package://{}/asset{}",
            NEXT.fetch_add(1, Ordering::Relaxed),
            super::extension_for(mime)
        );
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(Source {
            url: url.clone(),
            mime: mime.into(),
            bytes,
            stats: Arc::default(),
        });
        url
    }
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
}

#[implement(IStream)]
struct MappedStream {
    bytes: &'static [u8],
    cursor: Mutex<u64>,
    stats: Arc<ReadStats>,
}
impl MappedStream {
    fn open(bytes: &'static [u8], stats: Arc<ReadStats>, cursor: u64) -> IStream {
        Self {
            bytes,
            cursor: Mutex::new(cursor),
            stats,
        }
        .into()
    }
}
impl ISequentialStream_Impl for MappedStream_Impl {
    fn Read(&self, destination: *mut c_void, requested: u32, read: *mut u32) -> HRESULT {
        unsafe {
            if !read.is_null() {
                *read = 0;
            }
        }
        if requested != 0 && destination.is_null() {
            return E_POINTER;
        }
        let mut position = self
            .cursor
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let available = self
            .bytes
            .len()
            .saturating_sub(usize::try_from(*position).unwrap_or(usize::MAX));
        let count = available.min(requested as usize);
        if count != 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.bytes.as_ptr().add(*position as usize),
                    destination.cast(),
                    count,
                );
            }
            *position += count as u64;
        }
        unsafe {
            if !read.is_null() {
                *read = count as u32;
            }
        }
        self.stats.calls.fetch_add(1, Ordering::Relaxed);
        self.stats.bytes.fetch_add(count as u64, Ordering::Relaxed);
        self.stats
            .largest
            .fetch_max(count as u64, Ordering::Relaxed);
        if count == requested as usize {
            S_OK
        } else {
            S_FALSE
        }
    }
    fn Write(&self, _source: *const c_void, _count: u32, written: *mut u32) -> HRESULT {
        unsafe {
            if !written.is_null() {
                *written = 0;
            }
        }
        STG_E_ACCESSDENIED
    }
}
impl IStream_Impl for MappedStream_Impl {
    fn Seek(&self, delta: i64, origin: STREAM_SEEK, new_position: *mut u64) -> Result<()> {
        let mut cursor = self
            .cursor
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let base = match origin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => *cursor as i128,
            STREAM_SEEK_END => self.bytes.len() as i128,
            _ => return Err(E_INVALIDARG.into()),
        };
        let position = base + delta as i128;
        if !(0..=u64::MAX as i128).contains(&position) {
            return Err(STG_E_INVALIDFUNCTION.into());
        }
        *cursor = position as u64;
        unsafe {
            if !new_position.is_null() {
                *new_position = *cursor;
            }
        }
        self.stats.seeks.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn SetSize(&self, _size: u64) -> Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }
    fn CopyTo(
        &self,
        target: Ref<IStream>,
        count: u64,
        read: *mut u64,
        written: *mut u64,
    ) -> Result<()> {
        unsafe {
            if !read.is_null() {
                *read = 0;
            }
            if !written.is_null() {
                *written = 0;
            }
        }
        let target = target.ok()?;
        let mut remaining = count;
        let mut buffer = [0u8; 16 * 1024];
        while remaining != 0 {
            let request = remaining.min(buffer.len() as u64) as u32;
            let mut got = 0;
            self.Read(buffer.as_mut_ptr().cast(), request, &mut got)
                .ok()?;
            if got == 0 {
                break;
            }
            unsafe {
                if !read.is_null() {
                    *read += got as u64;
                }
            }
            let mut at = 0;
            while at < got {
                let mut sent = 0;
                unsafe {
                    target
                        .Write(
                            buffer.as_ptr().add(at as usize).cast(),
                            got - at,
                            Some(&mut sent),
                        )
                        .ok()?;
                }
                if sent == 0 {
                    return Err(STG_E_ACCESSDENIED.into());
                }
                at += sent;
                unsafe {
                    if !written.is_null() {
                        *written += sent as u64;
                    }
                }
            }
            remaining -= got as u64;
        }
        Ok(())
    }
    fn Commit(&self, _flags: &STGC) -> Result<()> {
        Ok(())
    }
    fn Revert(&self) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn LockRegion(&self, _offset: u64, _count: u64, _kind: &LOCKTYPE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn UnlockRegion(&self, _offset: u64, _count: u64, _kind: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Stat(&self, out: *mut STATSTG, _flags: &STATFLAG) -> Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            out.write(STATSTG {
                r#type: STGTY_STREAM.0 as u32,
                cbSize: self.bytes.len() as u64,
                grfMode: STGM_READ,
                ..Default::default()
            });
        }
        Ok(())
    }
    fn Clone(&self) -> Result<IStream> {
        Ok(MappedStream::open(
            self.bytes,
            self.stats.clone(),
            *self
                .cursor
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        ))
    }
}

const PENDING: u8 = 0;
const CANCELLED: u8 = 1;
const ENDED: u8 = 2;

#[implement(IAgileObject)]
struct Cookie {
    stream: IMFByteStream,
    phase: AtomicU8,
}
impl IAgileObject_Impl for Cookie_Impl {}

#[implement(IMFAsyncCallback)]
struct Complete {
    result: IMFAsyncResult,
}
impl IMFAsyncCallback_Impl for Complete_Impl {
    fn GetParameters(&self, _flags: *mut u32, _queue: *mut u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Invoke(&self, _work: Ref<IMFAsyncResult>) -> Result<()> {
        unsafe {
            let object = self.result.GetObject()?;
            let cookie = object.cast_object_ref::<Cookie>()?;
            if cookie.phase.load(Ordering::Acquire) == CANCELLED {
                self.result.SetStatus(MF_E_OPERATION_CANCELLED)?;
            }
            MFInvokeCallback(&self.result)
        }
    }
}
#[implement(IMFMediaEngineExtension)]
struct Extension {
    control: Control,
}
impl IMFMediaEngineExtension_Impl for Extension_Impl {
    fn CanPlayType(&self, _audio_only: BOOL, _mime: &BSTR) -> Result<MF_MEDIA_ENGINE_CANPLAY> {
        Err(E_NOTIMPL.into())
    }
    fn BeginCreateObject(
        &self,
        url: &BSTR,
        _existing: Ref<IMFByteStream>,
        kind: MF_OBJECT_TYPE,
        cancel: OutRef<IUnknown>,
        callback: Ref<IMFAsyncCallback>,
        state: Ref<IUnknown>,
    ) -> Result<()> {
        if kind != MF_OBJECT_BYTESTREAM {
            return Err(MF_E_UNSUPPORTED_SCHEME.into());
        }
        let source = self
            .control
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .filter(|source| source.url == url.to_string())
            .cloned()
            .ok_or_else(|| windows::core::Error::from_hresult(MF_E_UNSUPPORTED_SCHEME))?;
        let stream = MappedStream::open(source.bytes, source.stats, 0);
        let bytes = unsafe { MFCreateMFByteStreamOnStream(&stream)? };
        if !source.mime.is_empty() {
            let attributes: IMFAttributes = bytes.cast()?;
            unsafe {
                attributes.SetString(
                    &MF_BYTESTREAM_CONTENT_TYPE,
                    &windows::core::HSTRING::from(source.mime),
                )?;
            }
        }
        // The private object owns the stream and linearizes Cancel versus End.
        // A successful Cancel cannot race a subsequent successful End.
        let cookie: IUnknown = Cookie {
            stream: bytes,
            phase: AtomicU8::new(PENDING),
        }
        .into();
        let result = unsafe { MFCreateAsyncResult(&cookie, callback.ok()?, state.as_ref())? };
        unsafe {
            result.SetStatus(S_OK)?;
        }
        let completion: IMFAsyncCallback = Complete { result }.into();
        unsafe {
            MFPutWorkItem(
                MFASYNC_CALLBACK_QUEUE_STANDARD,
                &completion,
                None::<&IUnknown>,
            )?;
        }
        // This output is optional. Publish only after submission succeeds, so
        // a failed Begin transfers no cookie reference to its caller.
        if !cancel.is_null() {
            cancel.write(Some(cookie))?;
        }
        Ok(())
    }
    fn CancelObjectCreation(&self, cookie: Ref<IUnknown>) -> Result<()> {
        let cookie = cookie.ok()?.cast_object_ref::<Cookie>()?;
        match cookie
            .phase
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(CANCELLED) => Ok(()),
            Err(_) => Err(MF_E_INVALIDREQUEST.into()),
        }
    }
    fn EndCreateObject(&self, result: Ref<IMFAsyncResult>) -> Result<IUnknown> {
        let result = result.ok()?;
        let object = unsafe { result.GetObject()? };
        let cookie = object.cast_object_ref::<Cookie>()?;
        match cookie.phase.swap(ENDED, Ordering::AcqRel) {
            PENDING => {
                unsafe {
                    result.GetStatus()?;
                }
                cookie.stream.cast()
            }
            CANCELLED => Err(MF_E_OPERATION_CANCELLED.into()),
            _ => Err(MF_E_INVALIDREQUEST.into()),
        }
    }
}
pub(super) fn extension() -> (IMFMediaEngineExtension, Control) {
    let control = Control::default();
    (
        Extension {
            control: control.clone(),
        }
        .into(),
        control,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{STATFLAG_NONAME, STGC_DEFAULT};

    #[test]
    fn mapped_stream_reads_requested_ranges_and_clone_has_an_independent_cursor() {
        unsafe {
            let stats = Arc::new(ReadStats::default());
            let stream = MappedStream::open(b"0123456789", stats.clone(), 0);
            let mut buffer = [0u8; 4];
            let mut got = 0;
            assert_eq!(
                stream.Read(buffer.as_mut_ptr().cast(), 4, Some(&mut got)),
                S_OK
            );
            assert_eq!((&buffer, got), (b"0123", 4));
            stream.Seek(-2, STREAM_SEEK_END, None).unwrap();
            let clone = stream.Clone().unwrap();
            assert_eq!(
                stream.Read(buffer.as_mut_ptr().cast(), 4, Some(&mut got)),
                S_FALSE
            );
            assert_eq!((&buffer[..2], got), (&b"89"[..], 2));
            assert_eq!(
                clone.Read(buffer.as_mut_ptr().cast(), 2, Some(&mut got)),
                S_OK
            );
            assert_eq!((&buffer[..2], got), (&b"89"[..], 2));
            assert_eq!(stats.bytes.load(Ordering::Relaxed), 8);
            assert_eq!(stats.largest.load(Ordering::Relaxed), 4);
            let mut stat = STATSTG::default();
            stream.Stat(&mut stat, STATFLAG_NONAME).unwrap();
            assert_eq!((stat.cbSize, stat.grfMode), (10, STGM_READ));
        }
    }
    #[test]
    fn read_only_stream_validates_pointers_seeks_and_eof_without_resizing() {
        unsafe {
            let stream = MappedStream::open(b"abc", Arc::default(), 0);
            let mut got = 99;
            assert_eq!(
                stream.Read(std::ptr::null_mut(), 1, Some(&mut got)),
                E_POINTER
            );
            assert_eq!(got, 0);
            assert_eq!(stream.Read(std::ptr::null_mut(), 0, None), S_OK);
            assert!(stream.Seek(-1, STREAM_SEEK_SET, None).is_err());
            assert!(stream.Seek(0, STREAM_SEEK(7), None).is_err());
            stream.Seek(i64::MAX, STREAM_SEEK_SET, None).unwrap();
            stream.Seek(i64::MAX, STREAM_SEEK_CUR, None).unwrap();
            assert!(stream.Seek(2, STREAM_SEEK_CUR, None).is_err());
            let mut byte = 0u8;
            assert_eq!(
                stream.Read((&mut byte as *mut u8).cast(), 1, Some(&mut got)),
                S_FALSE
            );
            assert_eq!(got, 0);
            assert_eq!(
                stream.Write(std::ptr::null(), 0, Some(&mut got)),
                STG_E_ACCESSDENIED
            );
            assert_eq!(got, 0);
            assert_eq!(stream.SetSize(0).unwrap_err().code(), STG_E_ACCESSDENIED);
            stream.Commit(STGC_DEFAULT).unwrap();
        }
    }

    #[test]
    #[ignore = "requires SK_PACKED_MEDIA_PROBE_MP4 mapped padded MP4 and the real Windows media stack"]
    fn native_packaged_mp4_plays_and_seeks_with_bounded_mapped_reads() {
        use crate::{FrameRequest, MediaSource, VideoBackend};
        use std::{
            fs::OpenOptions,
            os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
            time::{Duration, Instant},
        };
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Memory::{MapViewOfFile, FILE_MAP_READ};
        // Kept for process lifetime, exactly as an exe resource/payload is.
        // A fake 'static slice followed by unmapping would invalidate MF's
        // asynchronous readers after Shutdown, so the proof never does that.
        unsafe extern "system" {
            #[link_name = "CreateFileMappingW"]
            fn create_mapping(
                file: *mut c_void,
                security: *const c_void,
                protection: u32,
                high: u32,
                low: u32,
                name: *const u16,
            ) -> *mut c_void;
            #[link_name = "GetCurrentProcess"]
            fn process() -> *mut c_void;
            #[link_name = "K32GetProcessMemoryInfo"]
            fn memory(process: *mut c_void, out: *mut c_void, size: u32) -> i32;
        }
        #[repr(C)]
        #[derive(Default)]
        struct Memory {
            size: u32,
            faults: u32,
            counters: [usize; 9],
        }
        fn private_bytes() -> usize {
            unsafe {
                let mut out = Memory {
                    size: std::mem::size_of::<Memory>() as u32,
                    ..Default::default()
                };
                assert_ne!(
                    memory(process(), (&mut out as *mut Memory).cast(), out.size),
                    0
                );
                out.counters[8]
            }
        }
        let path =
            std::env::var("SK_PACKED_MEDIA_PROBE_MP4").expect("set isolated probe fixture path");
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1 | 4)
            .open(&path)
            .unwrap();
        let length = file.metadata().unwrap().len() as usize;
        assert!(
            length >= 128 * 1024 * 1024,
            "fixture must expose whole-asset allocation regressions"
        );
        let handle = unsafe {
            create_mapping(
                file.as_raw_handle(),
                std::ptr::null(),
                2,
                0,
                0,
                std::ptr::null(),
            )
        };
        assert!(!handle.is_null());
        let view = unsafe { MapViewOfFile(HANDLE(handle), FILE_MAP_READ, 0, 0, 0) };
        assert!(!view.Value.is_null());
        let data: &'static [u8] = unsafe { std::slice::from_raw_parts(view.Value.cast(), length) };
        let _keep = Box::leak(Box::new((file, handle, view)));
        let mut backend = super::super::MediaEngineBackend::new().unwrap();
        backend.set_muted(true);
        let before = private_bytes();
        backend
            .load(&MediaSource::StaticBytes {
                mime: "video/mp4".into(),
                data,
            })
            .unwrap();
        let stats = backend
            .source_control
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stats
            .clone();
        backend.play();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut painted = false;
        while Instant::now() < deadline {
            let mut events = Vec::new();
            backend.drain_events(&mut |event| events.push(event));
            if let Some(error) = backend.error() {
                panic!(
                    "mapped source failed: {error:?}; reads={}, bytes={}",
                    stats.calls.load(Ordering::Relaxed),
                    stats.bytes.load(Ordering::Relaxed)
                );
            }
            painted |= backend
                .present(&FrameRequest { max_size: (16, 16) })
                .is_some();
            if painted && backend.current_time() > 0.05 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            painted && backend.current_time() > 0.05,
            "real MF must produce a frame and advance playback"
        );
        backend.pause();
        let duration = backend.duration();
        assert!(duration.is_finite() && duration > 0.1);
        let destination = duration * 0.6;
        backend.seek(destination);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut sought = false;
        while Instant::now() < deadline {
            backend.drain_events(&mut |event| {
                if event == crate::MediaEvent::Seeked {
                    sought = true;
                }
            });
            assert!(backend.error().is_none());
            let _ = backend.present(&FrameRequest { max_size: (16, 16) });
            if sought && !backend.seeking() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(sought && !backend.seeking(), "MF must finish a real seek");
        assert!((backend.current_time() - destination).abs() < 0.25);
        assert!(backend.bytes_retry.is_none());
        let after = private_bytes();
        let bytes = stats.bytes.load(Ordering::Relaxed);
        let largest = stats.largest.load(Ordering::Relaxed);
        let seeks = stats.seeks.load(Ordering::Relaxed);
        assert!(
            bytes > 0 && bytes < (length / 8) as u64,
            "decoder must read ranges, not scan the padded whole asset: {bytes}/{length}"
        );
        assert!(seeks > 0 && largest < (length / 8) as u64);
        assert!(
            after.saturating_sub(before) < length / 2,
            "private allocation must not scale with the mapped video size"
        );
        eprintln!("MAPPED_MEDIA_PROOF {{\"assetBytes\":{length},\"readBytes\":{bytes},\"largestRead\":{largest},\"seeks\":{seeks},\"privateBefore\":{before},\"privateAfter\":{after},\"painted\":{painted},\"seeked\":{sought}}}");
        // Exercise the data-URI Bytes path (temp-file fallback) with the extension installed.
        let tiny = include_bytes!("../../../../apps/hello/assets/clip-tiny.mp4");
        backend
            .load(&MediaSource::Bytes {
                mime: "video/mp4".into(),
                data: Arc::new(tiny.to_vec()),
            })
            .unwrap();
        backend.play();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut legacy = false;
        while Instant::now() < deadline {
            backend.drain_events(&mut |_| {});
            assert!(backend.error().is_none(), "existing Bytes path failed");
            legacy |= backend
                .present(&FrameRequest { max_size: (16, 16) })
                .is_some();
            if legacy {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(legacy, "existing Bytes fallback must still produce a frame");
        eprintln!("LEGACY_BYTES_PROOF pass");
    }

    #[implement(IMFAsyncCallback)]
    struct Observe {
        result: Arc<(Mutex<Option<IMFAsyncResult>>, std::sync::Condvar)>,
        calls: Arc<AtomicU64>,
    }
    impl IMFAsyncCallback_Impl for Observe_Impl {
        fn GetParameters(&self, _flags: *mut u32, _queue: *mut u32) -> Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn Invoke(&self, result: Ref<IMFAsyncResult>) -> Result<()> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            *self.result.0.lock().unwrap() = Some(result.ok()?.clone());
            self.result.1.notify_one();
            Ok(())
        }
    }
    fn observer() -> (
        IMFAsyncCallback,
        Arc<(Mutex<Option<IMFAsyncResult>>, std::sync::Condvar)>,
        Arc<AtomicU64>,
    ) {
        let result = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        let calls = Arc::new(AtomicU64::new(0));
        (
            Observe {
                result: result.clone(),
                calls: calls.clone(),
            }
            .into(),
            result,
            calls,
        )
    }
    fn received(
        result: &Arc<(Mutex<Option<IMFAsyncResult>>, std::sync::Condvar)>,
    ) -> IMFAsyncResult {
        let slot = result.0.lock().unwrap();
        let (slot, timeout) = result
            .1
            .wait_timeout_while(slot, std::time::Duration::from_secs(5), |slot| {
                slot.is_none()
            })
            .unwrap();
        assert!(!timeout.timed_out(), "MF callback did not complete");
        slot.as_ref().unwrap().clone()
    }
    #[test]
    fn extension_completes_without_optional_cookie_and_defers_other_sources() {
        super::super::ensure_platform().unwrap();
        let (extension, control) = extension();
        let url = control.select("video/mp4", b"abc");
        let (callback, result, calls) = observer();
        unsafe {
            extension
                .BeginCreateObject(
                    &BSTR::from(url),
                    None::<&IMFByteStream>,
                    MF_OBJECT_BYTESTREAM,
                    std::ptr::null_mut(),
                    &callback,
                    None::<&IUnknown>,
                )
                .unwrap();
            let result = received(&result);
            let object = extension.EndCreateObject(&result).unwrap();
            assert_eq!(
                object.cast::<IMFByteStream>().unwrap().GetLength().unwrap(),
                3
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                extension.EndCreateObject(&result).unwrap_err().code(),
                MF_E_INVALIDREQUEST
            );
            let failure = extension
                .BeginCreateObject(
                    &BSTR::from("https://example.invalid/x.mp4"),
                    None::<&IMFByteStream>,
                    MF_OBJECT_BYTESTREAM,
                    std::ptr::null_mut(),
                    &callback,
                    None::<&IUnknown>,
                )
                .unwrap_err();
            assert_eq!(failure.code(), MF_E_UNSUPPORTED_SCHEME);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
    }
    #[test]
    fn cancellation_is_exactly_once_and_replacement_keeps_the_old_stream_owned() {
        super::super::ensure_platform().unwrap();
        let (extension, control) = extension();
        let old_url = control.select("video/mp4", b"old");
        let (callback, result, calls) = observer();
        let mut cancel = None;
        unsafe {
            extension
                .BeginCreateObject(
                    &BSTR::from(old_url.clone()),
                    None::<&IMFByteStream>,
                    MF_OBJECT_BYTESTREAM,
                    &mut cancel,
                    &callback,
                    None::<&IUnknown>,
                )
                .unwrap();
            extension
                .CancelObjectCreation(cancel.as_ref().unwrap())
                .unwrap();
            let result = received(&result);
            assert_eq!(
                extension.EndCreateObject(&result).unwrap_err().code(),
                MF_E_OPERATION_CANCELLED
            );
            assert_eq!(
                extension
                    .CancelObjectCreation(cancel.as_ref().unwrap())
                    .unwrap_err()
                    .code(),
                MF_E_INVALIDREQUEST
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            let (callback, old, calls) = observer();
            extension
                .BeginCreateObject(
                    &BSTR::from(old_url.clone()),
                    None::<&IMFByteStream>,
                    MF_OBJECT_BYTESTREAM,
                    std::ptr::null_mut(),
                    &callback,
                    None::<&IUnknown>,
                )
                .unwrap();
            let new_url = control.select("video/mp4", b"new bytes");
            let old = extension
                .EndCreateObject(&received(&old))
                .unwrap()
                .cast::<IMFByteStream>()
                .unwrap();
            let mut bytes = [0u8; 3];
            let mut got = 0;
            old.Read(&mut bytes, &mut got).unwrap();
            assert_eq!(got, 3);
            assert_eq!(&bytes, b"old");
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            let (callback, new, calls) = observer();
            assert_eq!(
                extension
                    .BeginCreateObject(
                        &BSTR::from(old_url),
                        None::<&IMFByteStream>,
                        MF_OBJECT_BYTESTREAM,
                        std::ptr::null_mut(),
                        &callback,
                        None::<&IUnknown>
                    )
                    .unwrap_err()
                    .code(),
                MF_E_UNSUPPORTED_SCHEME
            );
            extension
                .BeginCreateObject(
                    &BSTR::from(new_url),
                    None::<&IMFByteStream>,
                    MF_OBJECT_BYTESTREAM,
                    std::ptr::null_mut(),
                    &callback,
                    None::<&IUnknown>,
                )
                .unwrap();
            let stream = extension
                .EndCreateObject(&received(&new))
                .unwrap()
                .cast::<IMFByteStream>()
                .unwrap();
            assert_eq!(stream.GetLength().unwrap(), 9);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
    }
}
