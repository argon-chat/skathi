//! The video output's BGRA `CVPixelBuffer`s as gpui images.
//!
//! The output is asked for frames at the size they will be painted, so VideoToolbox scales on the
//! GPU and only the painted pixels are copied (1080p into 800x450: ~1 ms instead of ~15 ms for a
//! CPU downscale). A new size means a new output; until it delivers, the last buffer is scaled on
//! the CPU once.

use std::borrow::Cow;
use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_av_foundation::{AVPlayerItem, AVPlayerItemVideoOutput};
use objc2_core_foundation::CFString;
use objc2_core_media::{kCMTimeInvalid, CMTime};
use objc2_core_video::{
    kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey,
    kCVPixelFormatType_32BGRA, kCVReturnSuccess, CVPixelBuffer, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use crate::backend::{FrameRequest, VideoFrame};

extern "C" {
    fn mach_absolute_time() -> u64;
}

#[derive(Default)]
pub(super) struct Frames {
    output: Option<Retained<AVPlayerItemVideoOutput>>,
    output_size: (u32, u32),
    /// Kept so a resize, or a paused element's single seek frame, can be produced again.
    last: Option<(Retained<CVPixelBuffer>, f64)>,
    /// The frame box the last presented image was made for.
    presented_for: (u32, u32),
}

pub(super) struct Presented {
    pub frame: Option<VideoFrame>,
    /// A new output replaced the old one: it has no frame until the player decodes one, which a
    /// paused player does only on a seek.
    pub output_replaced: bool,
}

impl Frames {
    pub fn detach(&mut self, item: Option<&AVPlayerItem>) {
        if let (Some(item), Some(output)) = (item, self.output.take()) {
            unsafe { item.removeOutput(&output) };
        }
        self.output = None;
        self.output_size = (0, 0);
        self.last = None;
        self.presented_for = (0, 0);
    }

    /// Forgets the frame but keeps the output.
    pub fn forget(&mut self) {
        self.last = None;
        self.presented_for = (0, 0);
    }

    pub fn has_frame(&self) -> bool {
        self.last.is_some()
    }

    pub fn last_size(&self) -> Option<(u32, u32)> {
        self.last.as_ref().map(|(buffer, _)| size_of(buffer))
    }

    fn attach(&mut self, item: &AVPlayerItem, size: (u32, u32)) {
        let number = |n: u32| -> Retained<AnyObject> {
            Retained::into_super(Retained::into_super(Retained::into_super(
                NSNumber::new_u32(n),
            )))
        };
        // SAFETY: CFString and NSString are toll-free bridged.
        let key = |k: &'static CFString| -> &'static NSString {
            unsafe { &*(k as *const CFString).cast() }
        };
        let attributes = unsafe {
            NSDictionary::<NSString, AnyObject>::from_retained_objects(
                &[
                    key(kCVPixelBufferPixelFormatTypeKey),
                    key(kCVPixelBufferWidthKey),
                    key(kCVPixelBufferHeightKey),
                ],
                &[
                    number(kCVPixelFormatType_32BGRA),
                    number(size.0),
                    number(size.1),
                ],
            )
        };
        let output = unsafe {
            AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                AVPlayerItemVideoOutput::alloc(),
                Some(&attributes),
            )
        };
        unsafe {
            item.addOutput(&output);
            if let Some(old) = self.output.replace(output) {
                item.removeOutput(&old);
            }
        }
        self.output_size = size;
    }

    /// The frame due now on the player's clock (so the picture follows the audio), at `request`'s size.
    pub fn present(
        &mut self,
        item: &AVPlayerItem,
        native: (u32, u32),
        request: &FrameRequest,
    ) -> Presented {
        let target = crate::fit_within(native, request.max_size);
        let mut out = Presented {
            frame: None,
            output_replaced: false,
        };
        if target.0 == 0 || target.1 == 0 {
            return out;
        }
        if self.output_size != target {
            self.attach(item, target);
            out.output_replaced = true;
        }
        let Some(output) = &self.output else {
            return out;
        };
        // SAFETY: plain queries on a live output; the out-pointer is a local.
        let fresh = unsafe {
            let now = output.itemTimeForMachAbsoluteTime(mach_absolute_time() as i64);
            let mut shown = kCMTimeInvalid;
            output.hasNewPixelBufferForItemTime(now)
                && output
                    .copyPixelBufferForItemTime_itemTimeForDisplay(now, &mut shown)
                    .map(|buffer| {
                        let pts = seconds(shown).or_else(|| seconds(now)).unwrap_or(0.0);
                        self.last = Some((buffer, pts));
                    })
                    .is_some()
        };
        if !fresh && self.presented_for == target {
            return out;
        }
        let Some((buffer, pts)) = &self.last else {
            return out;
        };
        let have = size_of(buffer);
        let size = crate::fit_within(have, target);
        let Some(bytes) = bgra(buffer, have, size) else {
            return out;
        };
        let Some(image) = RgbaImage::from_raw(size.0, size.1, bytes) else {
            return out;
        };
        self.presented_for = target;
        out.frame = Some(VideoFrame {
            image: Arc::new(RenderImage::new(vec![Frame::new(image)]).dynamic()),
            pts: *pts,
            size,
            gpu: None,
        });
        out
    }
}

fn size_of(buffer: &CVPixelBuffer) -> (u32, u32) {
    (
        CVPixelBufferGetWidth(buffer) as u32,
        CVPixelBufferGetHeight(buffer) as u32,
    )
}

fn seconds(time: CMTime) -> Option<f64> {
    // SAFETY: `CMTimeGetSeconds` is a pure function of its argument.
    let s = unsafe { time.seconds() };
    s.is_finite().then_some(s)
}

/// Packed BGRA at `size`, downscaled on the CPU when the buffer is larger.
fn bgra(buffer: &CVPixelBuffer, have: (u32, u32), size: (u32, u32)) -> Option<Vec<u8>> {
    if CVPixelBufferGetPixelFormatType(buffer) != kCVPixelFormatType_32BGRA {
        return None;
    }
    let flags = CVPixelBufferLockFlags::ReadOnly;
    // SAFETY: a live pixel buffer, unlocked below with the same flags.
    if unsafe { CVPixelBufferLockBaseAddress(buffer, flags) } != kCVReturnSuccess {
        return None;
    }
    let base = CVPixelBufferGetBaseAddress(buffer) as *const u8;
    let pitch = CVPixelBufferGetBytesPerRow(buffer);
    let (w, h) = (have.0 as usize, have.1 as usize);
    let row = w * 4;
    let out = (!base.is_null() && pitch >= row && h > 0).then(|| {
        // SAFETY: while locked the base address is readable for `pitch` bytes per row over `h`
        // rows, the last of which is at least `row` long.
        let src = unsafe { std::slice::from_raw_parts(base, pitch * (h - 1) + row) };
        let packed: Cow<[u8]> = if pitch == row {
            Cow::Borrowed(src)
        } else {
            let mut out = Vec::with_capacity(row * h);
            for y in 0..h {
                out.extend_from_slice(&src[y * pitch..y * pitch + row]);
            }
            Cow::Owned(out)
        };
        if size == have {
            packed.into_owned()
        } else {
            sk_resample::downscale(&packed, have.0, have.1, size.0, size.1, false)
        }
    });
    // SAFETY: the lock taken above.
    unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
    out
}
