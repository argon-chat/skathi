use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};

use crate::{AudioOutput, MediaError, MediaErrorCode};
use objc2_av_foundation::AVPlayerItem;

pub(super) struct Capture {
    output: Weak<dyn AudioOutput>,
    gain: AtomicU32,
}

impl Capture {
    pub fn new(output: &Arc<dyn AudioOutput>) -> Arc<Self> {
        Arc::new(Self {
            output: Arc::downgrade(output),
            gain: AtomicU32::new(1.0f32.to_bits()),
        })
    }
    pub fn gain(&self, volume: f64, muted: bool) {
        self.gain.store(
            (if muted { 0.0 } else { volume as f32 }).to_bits(),
            Ordering::Relaxed,
        );
    }
    pub fn install(self: &Arc<Self>, item: &AVPlayerItem) -> Result<(), MediaError> {
        unsafe extern "C" {
            fn sk_install_audio_tap(
                item: *const AVPlayerItem,
                context: *const c_void,
                push: unsafe extern "C" fn(*const c_void, *const f32, usize, u32, u32),
                release: unsafe extern "C" fn(*const c_void),
            ) -> bool;
        }
        unsafe extern "C" fn push(
            data: *const c_void,
            pcm: *const f32,
            len: usize,
            channels: u32,
            rate: u32,
        ) {
            let capture = unsafe { &*data.cast::<Capture>() };
            if let Some(out) = capture.output.upgrade() {
                out.push(
                    unsafe { std::slice::from_raw_parts(pcm, len) },
                    channels as usize,
                    rate,
                    f32::from_bits(capture.gain.load(Ordering::Relaxed)),
                );
            }
        }
        unsafe extern "C" fn release(data: *const c_void) {
            drop(unsafe { Arc::from_raw(data.cast::<Capture>()) });
        }
        // SAFETY: the tap takes ownership of this reference on every return path.
        if unsafe { sk_install_audio_tap(item, Arc::into_raw(self.clone()).cast(), push, release) }
        {
            Ok(())
        } else {
            Err(MediaError::new(MediaErrorCode::Decode, "AVFoundation could not attach an audio tap. Use a file-based audio or video source for Web Audio on macOS."))
        }
    }
}
