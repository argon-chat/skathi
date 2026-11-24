use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::ffi::{self, Api, Ptr};
use crate::AudioOutput;

#[derive(Default)]
pub(super) struct Capture {
    output: Mutex<Option<Weak<dyn AudioOutput>>>,
    gain: AtomicU32,
}

impl Capture {
    pub fn gain(&self, volume: f64, muted: bool) {
        self.gain.store(
            (if muted { 0.0 } else { volume as f32 }).to_bits(),
            Ordering::Relaxed,
        );
    }
    pub fn attach(&self, output: &Arc<dyn AudioOutput>) {
        *self.output.lock().unwrap() = Some(Arc::downgrade(output));
    }
    pub fn routed(&self) -> bool {
        self.output.lock().unwrap().is_some()
    }
}

unsafe fn forward(api: &Api, sample: Ptr, output: &dyn AudioOutput, gain: f32) {
    let core = &api.core;
    // SAFETY: the sample owns its caps and buffer for the duration of this call.
    unsafe {
        let caps = (core.gst_sample_get_caps)(sample);
        let buffer = (core.gst_sample_get_buffer)(sample);
        if caps.is_null() || buffer.is_null() {
            return;
        }
        let s = (core.gst_caps_get_structure)(caps, 0);
        let (mut channels, mut rate) = (0, 0);
        if !s.is_null() {
            (core.gst_structure_get_int)(s, c"channels".as_ptr(), &mut channels);
            (core.gst_structure_get_int)(s, c"rate".as_ptr(), &mut rate);
        }
        if channels <= 0 || rate <= 0 {
            return;
        }
        let mut map = ffi::MapInfo::zeroed();
        if (core.gst_buffer_map)(buffer, &mut map, ffi::MAP_READ) == ffi::FALSE {
            return;
        }
        if !map.data.is_null() && map.data.cast::<f32>().is_aligned() {
            output.push(
                std::slice::from_raw_parts(map.data.cast::<f32>(), map.size / 4),
                channels as usize,
                rate as u32,
                gain,
            );
        }
        (core.gst_buffer_unmap)(buffer, &mut map);
    }
}

unsafe extern "C" fn on_sample(sink: Ptr, data: Ptr) -> i32 {
    let Ok(api) = ffi::api() else {
        return ffi::FLOW_OK;
    };
    // SAFETY: appsink owns the callback's Capture and hands us one sample reference.
    unsafe {
        let capture = &*data.cast::<Capture>();
        let sample = (api.app.gst_app_sink_pull_sample)(sink);
        if !sample.is_null() {
            if let Some(output) = capture
                .output
                .try_lock()
                .ok()
                .and_then(|s| s.as_ref()?.upgrade())
            {
                forward(
                    api,
                    sample,
                    output.as_ref(),
                    f32::from_bits(capture.gain.load(Ordering::Relaxed)),
                );
            }
            (api.core.gst_mini_object_unref)(sample);
        }
    }
    ffi::FLOW_OK
}

unsafe extern "C" fn release(data: Ptr) {
    drop(unsafe { Arc::from_raw(data.cast::<Capture>()) });
}

pub(super) fn install(api: &Api, playbin: Ptr, capture: &Arc<Capture>) -> bool {
    let format = if cfg!(target_endian = "little") {
        "F32LE"
    } else {
        "F32BE"
    };
    // Only new_sample delivers PCM, after clock sync. The main output controls paused preroll.
    let desc = format!(
        "tee name=split ! queue split. ! queue ! audioconvert ! \
        audio/x-raw,format={format},layout=interleaved ! appsink name=tap sync=true \
        async=false max-buffers=1 drop=true enable-last-sample=false"
    );
    let Ok(bin) = api.parse_bin(&desc) else {
        return false;
    };
    unsafe {
        let tap = (api.core.gst_bin_get_by_name)(bin, c"tap".as_ptr());
        if tap.is_null() {
            (api.core.gst_object_unref)(bin);
            return false;
        }
        let callbacks = ffi::AppSinkCallbacks {
            eos: None,
            new_preroll: None,
            new_sample: Some(on_sample),
            new_event: std::ptr::null_mut(),
            propose_allocation: std::ptr::null_mut(),
            reserved: [std::ptr::null_mut(); 2],
        };
        (api.app.gst_app_sink_set_callbacks)(
            tap,
            &callbacks,
            Arc::into_raw(capture.clone()) as Ptr,
            Some(release),
        );
        api.set_ptr(playbin, "audio-filter", bin);
        (api.core.gst_object_unref)(tap);
        (api.core.gst_object_unref)(bin);
    }
    true
}
