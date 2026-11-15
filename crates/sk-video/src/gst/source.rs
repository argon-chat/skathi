//! In-memory sources (`data:` bytes, mapped package bytes) through playbin's `appsrc://` in
//! random-access mode: the demuxer seeks by byte offset and each request is answered with a buffer
//! that wraps the bytes in place. No temp file and no copy.

use std::ffi::c_uint;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::ffi::{self, Api, Ptr};

pub(super) enum Bytes {
    Mapped(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl Bytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mapped(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

pub(super) struct Feed {
    bytes: Bytes,
    offset: AtomicU64,
}

const CHUNK: usize = 256 << 10;

impl Feed {
    pub fn new(bytes: Bytes) -> Arc<Self> {
        Arc::new(Self {
            bytes,
            offset: AtomicU64::new(0),
        })
    }

    fn len(&self) -> usize {
        self.bytes.as_slice().len()
    }
}

unsafe extern "C" fn release_feed(data: Ptr) {
    // SAFETY: a reference leaked by `configure` or `wrap`.
    drop(unsafe { Arc::from_raw(data as *const Feed) });
}

unsafe extern "C" fn release_feed_closure(data: Ptr, _closure: Ptr) {
    unsafe { release_feed(data) };
}

unsafe extern "C" fn need_data(src: Ptr, length: c_uint, data: Ptr) {
    let Ok(api) = ffi::api() else { return };
    // SAFETY: `data` is the feed registered with these callbacks, alive until their notify.
    let feed = unsafe { &*(data as *const Feed) };
    let len = feed.len();
    let start = (feed.offset.load(Ordering::SeqCst) as usize).min(len);
    if start >= len {
        unsafe { (api.app.gst_app_src_end_of_stream)(src) };
        return;
    }
    let want = match length as usize {
        0 => CHUNK,
        n if n == c_uint::MAX as usize => CHUNK,
        n => n,
    };
    let end = start.saturating_add(want).min(len);
    let Some(buffer) = (unsafe { wrap(api, data as *const Feed, start, end) }) else {
        return;
    };
    feed.offset.store(end as u64, Ordering::SeqCst);
    // SAFETY: `push_buffer` takes the buffer's reference.
    unsafe { (api.app.gst_app_src_push_buffer)(src, buffer) };
}

unsafe extern "C" fn seek_data(_src: Ptr, offset: u64, data: Ptr) -> ffi::GBool {
    // SAFETY: as in `need_data`.
    let feed = unsafe { &*(data as *const Feed) };
    if offset as usize > feed.len() {
        return ffi::FALSE;
    }
    feed.offset.store(offset, Ordering::SeqCst);
    ffi::TRUE
}

/// A read-only buffer over `start..end` of the feed's bytes, holding a reference to the feed so the
/// bytes outlive the buffer.
unsafe fn wrap(api: &Api, feed: *const Feed, start: usize, end: usize) -> Option<*mut ffi::Buffer> {
    let bytes = unsafe { &*feed }.bytes.as_slice();
    let base = bytes.as_ptr() as Ptr;
    // SAFETY: the feed is alive (the caller holds it); the new reference is the buffer's.
    unsafe { Arc::increment_strong_count(feed) };
    let buffer = unsafe {
        (api.core.gst_buffer_new_wrapped_full)(
            ffi::MEMORY_FLAG_READONLY,
            base,
            bytes.len(),
            start,
            end - start,
            feed as Ptr,
            Some(release_feed),
        )
    };
    (!buffer.is_null()).then_some(buffer)
}

/// playbin's `source-setup`: configures the `appsrc` it made for `appsrc://`.
unsafe extern "C" fn source_setup(_playbin: Ptr, source: Ptr, data: Ptr) {
    let Ok(api) = ffi::api() else { return };
    // SAFETY: `source` is the element playbin just created; `data` is the connected feed.
    unsafe {
        let factory = (api.core.gst_element_get_factory)(source);
        if factory.is_null() || api.owned((api.core.gst_object_get_name)(factory)) != "appsrc" {
            return;
        }
        let feed = &*(data as *const Feed);
        api.set_i32(source, "stream-type", ffi::APP_STREAM_RANDOM_ACCESS);
        api.set_i32(source, "format", ffi::FORMAT_BYTES);
        api.set_i64(source, "size", feed.len() as i64);
        let callbacks = ffi::AppSrcCallbacks {
            need_data: Some(need_data),
            enough_data: None,
            seek_data: Some(seek_data),
            reserved: [std::ptr::null_mut(); 4],
        };
        Arc::increment_strong_count(data as *const Feed);
        (api.app.gst_app_src_set_callbacks)(source, &callbacks, data, Some(release_feed));
    }
}

/// Serves `feed` to the `appsrc` that `playbin` creates for an `appsrc://` URI.
pub(super) fn connect(api: &Api, playbin: Ptr, feed: &Arc<Feed>) {
    let signal = ffi::c("source-setup");
    let data = Arc::into_raw(feed.clone()) as Ptr;
    // SAFETY: the handler's signature matches `source-setup`; the closure notify frees `data`.
    unsafe {
        (api.gobject.g_signal_connect_data)(
            playbin,
            signal.as_ptr(),
            source_setup as Ptr,
            data,
            Some(release_feed_closure),
            0,
        )
    };
}
