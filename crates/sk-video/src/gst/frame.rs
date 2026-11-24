//! The appsink end of the pipeline: BGRA samples at the painted size, as gpui images.
//!
//! The sink syncs to the pipeline clock, so the sample it last handed over is the frame due now.
//! The streaming thread only parks that sample here; the UI thread copies it into a `RenderImage`.
//! The caps filter in front of the sink is set to the painted size, so `videoconvertscale` scales on
//! the streaming thread; until a renegotiated sample arrives, the held one is scaled on the CPU.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use super::ffi::{self, Api, Ptr};
use crate::backend::VideoFrame;

/// An owned `GstSample` reference.
pub(super) struct Sample(Ptr);

// SAFETY: GstSample is an immutable, atomically refcounted mini object.
unsafe impl Send for Sample {}

impl Drop for Sample {
    fn drop(&mut self) {
        if let Ok(api) = ffi::api() {
            // SAFETY: the reference this wrapper owns.
            unsafe { (api.core.gst_mini_object_unref)(self.0) };
        }
    }
}

/// What the appsink's streaming thread leaves for the UI thread.
#[derive(Default)]
pub(super) struct Latest {
    sample: Mutex<Option<Sample>>,
    generation: AtomicU64,
}

impl Latest {
    fn put(&self, sample: Ptr) {
        if sample.is_null() {
            return;
        }
        *self.sample.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sample(sample));
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe extern "C" fn on_preroll(sink: Ptr, data: Ptr) -> i32 {
    // SAFETY: `data` is the `Latest` registered with these callbacks, alive until their notify.
    let latest = unsafe { &*(data as *const Latest) };
    if let Ok(api) = ffi::api() {
        latest.put(unsafe { (api.app.gst_app_sink_pull_preroll)(sink) });
    }
    ffi::FLOW_OK
}

unsafe extern "C" fn on_sample(sink: Ptr, data: Ptr) -> i32 {
    // SAFETY: as in `on_preroll`.
    let latest = unsafe { &*(data as *const Latest) };
    if let Ok(api) = ffi::api() {
        latest.put(unsafe { (api.app.gst_app_sink_pull_sample)(sink) });
    }
    ffi::FLOW_OK
}

unsafe extern "C" fn release_latest(data: Ptr) {
    // SAFETY: the reference `attach` leaked for the callbacks.
    drop(unsafe { Arc::from_raw(data as *const Latest) });
}

/// Hands the appsink's samples to `latest`, from the streaming thread.
pub(super) fn attach(api: &Api, appsink: Ptr, latest: &Arc<Latest>) {
    let callbacks = ffi::AppSinkCallbacks {
        eos: None,
        new_preroll: Some(on_preroll),
        new_sample: Some(on_sample),
        new_event: std::ptr::null_mut(),
        propose_allocation: std::ptr::null_mut(),
        reserved: [std::ptr::null_mut(); 2],
    };
    let data = Arc::into_raw(latest.clone()) as Ptr;
    // SAFETY: appsink copies the callback table; `release_latest` frees `data` when it is replaced.
    unsafe {
        (api.app.gst_app_sink_set_callbacks)(appsink, &callbacks, data, Some(release_latest))
    };
}

/// The UI thread's side: the sample being shown and the size it was shown at.
pub(super) struct Frames {
    pub latest: Arc<Latest>,
    seen: u64,
    held: Option<Held>,
    #[cfg(feature = "webgpu")]
    media: Option<(Sample, redjs_webgpu::video::Frame)>,
    presented_for: (u32, u32),
    /// A frame has been decoded since the load; survives [`Frames::forget`].
    decoded: bool,
}

struct Held {
    sample: Sample,
    size: (u32, u32),
    pts: f64,
}

impl Frames {
    pub fn new() -> Self {
        Self {
            latest: Arc::default(),
            seen: 0,
            held: None,
            #[cfg(feature = "webgpu")]
            media: None,
            presented_for: (0, 0),
            decoded: false,
        }
    }

    /// Forgets the frame shown, keeping the sink.
    pub fn forget(&mut self) {
        #[cfg(feature = "webgpu")]
        {
            self.media = None;
        }
        self.held = None;
        self.presented_for = (0, 0);
    }

    pub fn has_frame(&self) -> bool {
        self.decoded || self.latest.generation.load(Ordering::SeqCst) != self.seen
    }

    pub fn held_size(&self) -> Option<(u32, u32)> {
        self.held.as_ref().map(|h| h.size)
    }

    #[cfg(feature = "webgpu")]
    pub fn snapshot(&mut self, api: &Api, video_bin: Ptr) -> Option<redjs_webgpu::video::Frame> {
        use redjs_webgpu::video::{Frame, PixelFormat};
        let video = api.video.as_ref()?;
        // SAFETY: the bin owns the sink; the boxed last-sample property returns an owned
        // reference, including when the sink is paused on its preroll frame.
        let sample = unsafe {
            let sink = (api.core.gst_bin_get_by_name)(video_bin, c"media".as_ptr());
            if sink.is_null() {
                return None;
            }
            let mut raw: Ptr = std::ptr::null_mut();
            (api.gobject.g_object_get)(
                sink,
                c"last-sample".as_ptr(),
                &mut raw as *mut Ptr,
                std::ptr::null::<std::ffi::c_char>(),
            );
            (api.core.gst_object_unref)(sink);
            if raw.is_null() {
                return None;
            }
            Sample(raw)
        };
        // SAFETY: both buffer pointers are borrowed from live sample references.
        if let Some((previous, frame)) = &self.media {
            if unsafe {
                (api.core.gst_sample_get_buffer)(previous.0)
                    == (api.core.gst_sample_get_buffer)(sample.0)
            } {
                return Some(frame.clone());
            }
        }
        let source = describe(api, sample)?;
        // SAFETY: convert_sample borrows the sample and caps, returning a new sample reference.
        let converted = unsafe {
            let caps = (api.core.gst_caps_from_string)(
                c"video/x-raw,format=BGRA,pixel-aspect-ratio=1/1".as_ptr(),
            );
            if caps.is_null() {
                return None;
            }
            let raw = (video.gst_video_convert_sample)(
                source.sample.0,
                caps,
                1_000_000_000,
                std::ptr::null_mut(),
            );
            (api.core.gst_mini_object_unref)(caps);
            if raw.is_null() {
                return None;
            }
            describe(api, Sample(raw))?
        };
        let frame = Frame::packed(
            converted.size.0,
            converted.size.1,
            (source.pts * 1e6) as i64,
            bgra(api, &converted.sample, converted.size, converted.size)?,
            PixelFormat::Bgra,
        )?;
        self.media = Some((source.sample, frame.clone()));
        Some(frame)
    }

    /// The newest sample the sink delivered, at `target` (already fitted to the request).
    pub fn present(&mut self, api: &Api, target: (u32, u32)) -> Option<VideoFrame> {
        if target.0 == 0 || target.1 == 0 {
            return None;
        }
        let generation = self.latest.generation.load(Ordering::SeqCst);
        let mut fresh = false;
        if generation != self.seen {
            self.seen = generation;
            let sample = self
                .latest
                .sample
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(held) = sample.and_then(|s| describe(api, s)) {
                // Starting playback from a paused seek hands the preroll frame over a second time.
                let repeat = self
                    .held
                    .as_ref()
                    .is_some_and(|h| h.pts == held.pts && h.size == held.size);
                self.held = Some(held);
                self.decoded = true;
                fresh = !repeat;
            }
        }
        if !fresh && self.presented_for == target {
            return None;
        }
        let held = self.held.as_ref()?;
        let size = crate::fit_within(held.size, target);
        let bytes = bgra(api, &held.sample, held.size, size)?;
        let image = RgbaImage::from_raw(size.0, size.1, bytes)?;
        self.presented_for = target;
        Some(VideoFrame {
            image: Arc::new(RenderImage::new(vec![Frame::new(image)]).dynamic()),
            pts: held.pts,
            size,
            gpu: None,
        })
    }
}

fn describe(api: &Api, sample: Sample) -> Option<Held> {
    // SAFETY: the sample is live; caps, buffer and segment are borrowed from it.
    unsafe {
        let caps = (api.core.gst_sample_get_caps)(sample.0);
        let (w, h, _) = api.video_caps(caps)?;
        let buffer = (api.core.gst_sample_get_buffer)(sample.0);
        if buffer.is_null() {
            return None;
        }
        let raw = (*buffer).pts;
        let segment = (api.core.gst_sample_get_segment)(sample.0);
        let stream = if raw == ffi::CLOCK_TIME_NONE || segment.is_null() {
            raw
        } else {
            (api.core.gst_segment_to_stream_time)(segment, ffi::FORMAT_TIME, raw)
        };
        let pts = if stream == ffi::CLOCK_TIME_NONE {
            0.0
        } else {
            stream as f64 / 1e9
        };
        Some(Held {
            sample,
            size: (w, h),
            pts,
        })
    }
}

/// Packed BGRA at `size`, downscaled on the CPU when the sample is larger.
fn bgra(api: &Api, sample: &Sample, have: (u32, u32), size: (u32, u32)) -> Option<Vec<u8>> {
    let (w, h) = (have.0 as usize, have.1 as usize);
    let row = w * 4;
    // SAFETY: the buffer is borrowed from the live sample and mapped read-only until the unmap below.
    unsafe {
        let buffer = (api.core.gst_sample_get_buffer)(sample.0);
        if buffer.is_null() {
            return None;
        }
        let (mut offset, mut pitch) = (0usize, row);
        if let Some(video) = &api.video {
            let meta = (video.gst_buffer_get_video_meta)(buffer);
            if !meta.is_null() && (*meta).stride[0] > 0 {
                offset = (*meta).offset[0];
                pitch = (*meta).stride[0] as usize;
            }
        }
        let mut map = ffi::MapInfo::zeroed();
        if (api.core.gst_buffer_map)(buffer, &mut map, ffi::MAP_READ) == ffi::FALSE {
            return None;
        }
        let need = offset + pitch * (h.max(1) - 1) + row;
        let out = (pitch >= row && h > 0 && !map.data.is_null() && map.size >= need).then(|| {
            let src = std::slice::from_raw_parts(map.data.add(offset), need - offset);
            let mut packed = Vec::with_capacity(row * h);
            if pitch == row {
                packed.extend_from_slice(&src[..row * h]);
            } else {
                for y in 0..h {
                    packed.extend_from_slice(&src[y * pitch..y * pitch + row]);
                }
            }
            if size == have {
                packed
            } else {
                sk_resample::downscale(&packed, have.0, have.1, size.0, size.1, false)
            }
        });
        (api.core.gst_buffer_unmap)(buffer, &mut map);
        out
    }
}
