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

unsafe extern "C" fn handoff(identity: Ptr, buffer: *mut ffi::Buffer, user: Ptr) {
    let capture = unsafe { &*user.cast::<Capture>() };
    let Some(output) = capture
        .output
        .try_lock()
        .ok()
        .and_then(|s| s.as_ref()?.upgrade())
    else {
        return;
    };
    let Ok(api) = ffi::api() else { return };
    let core = &api.core;
    // SAFETY: identity passes a borrowed buffer with the format negotiated on its sink pad.
    unsafe {
        let pad = (core.gst_element_get_static_pad)(identity, c"sink".as_ptr());
        if pad.is_null() {
            return;
        }
        let caps = (core.gst_pad_get_current_caps)(pad);
        (core.gst_object_unref)(pad);
        if caps.is_null() {
            return;
        }
        let s = (core.gst_caps_get_structure)(caps, 0);
        let (mut channels, mut rate) = (0, 0);
        if !s.is_null() {
            (core.gst_structure_get_int)(s, c"channels".as_ptr(), &mut channels);
            (core.gst_structure_get_int)(s, c"rate".as_ptr(), &mut rate);
        }
        (core.gst_mini_object_unref)(caps);
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
                f32::from_bits(capture.gain.load(Ordering::Relaxed)),
            );
        }
        (core.gst_buffer_unmap)(buffer, &mut map);
    }
}

unsafe extern "C" fn release(data: Ptr, _: Ptr) {
    drop(unsafe { Arc::from_raw(data.cast::<Capture>()) });
}

pub(super) struct Filter {
    api: &'static Api,
    identity: Ptr,
    direct: Ptr,
}

impl Filter {
    pub fn install(api: &'static Api, playbin: Ptr, capture: &Arc<Capture>) -> Option<Self> {
        let format = if cfg!(target_endian = "little") {
            "F32LE"
        } else {
            "F32BE"
        };
        let bin = api.parse_bin(&format!("audioconvert ! audio/x-raw,format={format},layout=interleaved ! identity name=tap signal-handoffs=false ! volume name=direct")).ok()?;
        // SAFETY: the named elements belong to this bin. Each lookup takes a reference.
        unsafe {
            let identity = (api.core.gst_bin_get_by_name)(bin, c"tap".as_ptr());
            let direct = (api.core.gst_bin_get_by_name)(bin, c"direct".as_ptr());
            if identity.is_null() || direct.is_null() {
                for object in [identity, direct, bin].into_iter().filter(|p| !p.is_null()) {
                    (api.core.gst_object_unref)(object);
                }
                return None;
            }
            (api.gobject.g_signal_connect_data)(
                identity,
                c"handoff".as_ptr(),
                handoff as Ptr,
                Arc::into_raw(capture.clone()) as Ptr,
                Some(release),
                0,
            );
            api.set_ptr(playbin, "audio-filter", bin);
            (api.core.gst_object_unref)(bin);
            let filter = Self {
                api,
                identity,
                direct,
            };
            if capture.routed() {
                filter.route()
            }
            Some(filter)
        }
    }

    pub fn route(&self) {
        self.api.set_bool(self.direct, "mute", true);
        self.api.set_bool(self.identity, "sync", true);
        self.api.set_bool(self.identity, "signal-handoffs", true);
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        unsafe {
            (self.api.core.gst_object_unref)(self.identity);
            (self.api.core.gst_object_unref)(self.direct);
        }
    }
}
