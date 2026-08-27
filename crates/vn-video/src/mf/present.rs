//! Turning "the engine has a frame" into "gpui has an image".
//!
//! # Why there is a readback here at all
//!
//! `TransferVideoFrame` blits the current frame into a texture we own, on the
//! renderer's device, already BGRA and already cropped — the frame is *one
//! `CopySubresourceRegion` away* from being a gpui atlas tile, and that copy is
//! measured at 0.11–0.18 ms of pure VRAM traffic. Taking it would need a new
//! method on `DirectXAtlas` (or filling in the `PrimitiveBatch::Surfaces` stub
//! that is already dispatched and does nothing), i.e. a change to
//! `directx_atlas.rs` / `directx_renderer.rs` in the fork.
//!
//! This round is scoped to `directx_devices.rs`, so v1 goes through the one
//! currency the renderer already accepts from outside: `RenderImage`. That
//! means GPU → system memory → atlas, and it is honestly the expensive path.
//! Two things keep it viable, and both are in the code below:
//!
//! * **The frame is produced at the size it will be PAINTED**, not the size it
//!   was encoded at. MF's video processor is scaling anyway, so asking it for
//!   800×450 instead of 1920×1080 turns 8.3 MB/frame into 1.4 MB/frame — a
//!   5.8× cut for free. See [`crate::FrameRequest`].
//! * **One texture pair, reused forever.** The target and the staging surface
//!   are allocated once per size and overwritten in place, so a playing video
//!   does not churn VRAM.
//!
//! CONTRACT.md §5 records the measured cost and what replaces this.

use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{IMFMediaEngine, MFARGB};

/// One CPU-readable copy of a decoded frame, in flight.
struct Slot {
    texture: ID3D11Texture2D,
    /// The PTS of the frame copied into it, in 100 ns units.
    pts: i64,
    /// Is there a frame in it that has not been read yet?
    filled: bool,
}

pub struct Presenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    size: (u32, u32),
    /// What `TransferVideoFrame` writes into: BGRA, on the renderer's device,
    /// a render target because that is what the video processor needs.
    target: Option<ID3D11Texture2D>,
    /// TWO staging surfaces, not one. See [`Presenter::transfer`].
    slots: Vec<Slot>,
    /// The slot this frame's copy goes into; the other one is read.
    write: usize,
}

impl Presenter {
    pub fn new(device: ID3D11Device) -> Self {
        let context = unsafe { device.GetImmediateContext() }
            .expect("a D3D11 device always has an immediate context");
        Self {
            device,
            context,
            size: (0, 0),
            target: None,
            slots: Vec::new(),
            write: 0,
        }
    }

    /// The size the textures are currently allocated at.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Drops the textures. Called on `src` change and on suspend, so a parked
    /// video gives its VRAM back.
    pub fn reset(&mut self) {
        self.target = None;
        self.slots.clear();
        self.write = 0;
        self.size = (0, 0);
    }

    fn ensure(&mut self, size: (u32, u32)) -> Option<()> {
        if self.size == size && self.target.is_some() && self.slots.len() == 2 {
            return Some(());
        }
        let common = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            // gpui's `RENDER_TARGET_FORMAT` and its Polychrome atlas format.
            // Asking the media engine for this is what makes MF insert its
            // video processor and hand back exactly what we can paint.
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };
        let mut target = None;
        unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    ..common
                },
                None,
                Some(&mut target),
            )
        }
        .ok()?;
        let mut slots = Vec::with_capacity(2);
        for _ in 0..2 {
            let mut staging = None;
            unsafe {
                self.device.CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        ..common
                    },
                    None,
                    Some(&mut staging),
                )
            }
            .ok()?;
            slots.push(Slot { texture: staging?, pts: 0, filled: false });
        }
        self.target = target;
        self.slots = slots;
        self.write = 0;
        self.size = size;
        Some(())
    }

    /// Pulls the engine's current frame at `size` and returns a gpui image —
    /// **of the frame before it**, together with that frame's PTS.
    ///
    /// # Why one frame of latency buys 2.5 ms
    ///
    /// The naive shape is `TransferVideoFrame` → `CopyResource` → `Map` on the
    /// same staging texture. `Map(D3D11_MAP_READ)` then has to wait for the GPU
    /// to finish the copy it was queued microseconds ago, so it *blocks the
    /// immediate context* — which is also gpui's context. [measured] that costs
    /// **2.57 ms mean / 4.5 ms p99** per 800×450 frame, identically in debug and
    /// release, i.e. it is pure GPU sync and no amount of optimising our own
    /// code touches it.
    ///
    /// With two staging surfaces the copy queued *this* frame is read *next*
    /// frame, by which time the GPU finished it long ago and `Map` returns
    /// immediately. The cost is 16 ms of extra display latency on a medium that
    /// is not interactive, and one more surface of VRAM.
    ///
    /// `D3D11_MAP_FLAG_DO_NOT_WAIT` guards the case where it is somehow still
    /// drawing (a GPU under load from something else — this machine games): we
    /// fall back to a blocking map rather than dropping the frame, because a
    /// dropped frame is visible and 2 ms is not.
    ///
    /// # …except when there is no next frame
    ///
    /// `pipeline: false` reads back the frame just written, blocking. It is
    /// what a PAUSED element must do, and getting this wrong is visible rather
    /// than subtle: a paused video produces exactly one frame per seek, so with
    /// the pipeline on, every seek displays the frame from the *previous* seek
    /// and there is never a following frame to flush it out. (Seeking to 2 s
    /// showed 4.333 s; seeking to 8 s then showed 2.000 s. Caught by reading the
    /// timecode burnt into the test clip — which is exactly why it is burnt in.)
    ///
    /// The trade is the right way round: latency matters at 30–60 fps and costs
    /// nothing to hide, correctness matters for a single still frame and the
    /// 2.5 ms it costs there is invisible.
    pub fn transfer(
        &mut self,
        engine: &IMFMediaEngine,
        size: (u32, u32),
        pts: i64,
        pipeline: bool,
    ) -> Option<(Arc<RenderImage>, i64)> {
        self.ensure(size)?;
        let target = self.target.as_ref()?.clone();

        let dst = RECT { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 };
        // Transparent border: the destination is exactly the video's aspect
        // ratio (`fit_within`), so nothing should letterbox — but if a driver
        // rounds differently, transparent edges are invisible against whatever
        // CSS put behind the element, and opaque black ones would not be.
        let border = MFARGB { rgbAlpha: 0, rgbRed: 0, rgbGreen: 0, rgbBlue: 0 };
        unsafe { engine.TransferVideoFrame(&target, None, &dst, Some(&border)) }.ok()?;

        let write = self.write;
        unsafe { self.context.CopyResource(&self.slots[write].texture, &target) };
        self.slots[write].pts = pts;
        self.slots[write].filled = true;

        let read = if pipeline {
            self.write = 1 - write;
            1 - write
        } else {
            // Read what was just written, and drop anything queued behind it —
            // it is older than what the caller is about to be shown.
            self.slots[1 - write].filled = false;
            write
        };

        if !self.slots[read].filled {
            // The very first frame after a load or a resize: nothing to read
            // yet. One tick of the pump later there is.
            return None;
        }
        self.slots[read].filled = false;
        let staging = self.slots[read].texture.clone();
        let frame_pts = self.slots[read].pts;

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let ok = pipeline
            && unsafe {
                self.context.Map(
                    &staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                )
            }
            .is_ok();
        if !ok {
            // Still drawing. Block rather than drop: a missing frame is visible.
            unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.ok()?;
        }
        let row_bytes = size.0 as usize * 4;
        let pitch = mapped.RowPitch as usize;
        // ONE copy, straight into the buffer the `RenderImage` will own. The
        // obvious shape — read into a reusable scratch buffer, then `clone()`
        // it for the image — copies 1.4 MB twice per frame for no benefit,
        // because `RenderImage` needs its own `Vec` anyway.
        let mut pixels: Vec<u8> = Vec::with_capacity(row_bytes * size.1 as usize);
        // SAFETY: `Map` succeeded, so `pData` is `RowPitch * Height` readable
        // bytes of the staging texture we just created at exactly this size,
        // and `pixels` has capacity for every byte written below.
        unsafe {
            let src = mapped.pData as *const u8;
            let dst = pixels.as_mut_ptr();
            for y in 0..size.1 as usize {
                std::ptr::copy_nonoverlapping(src.add(y * pitch), dst.add(y * row_bytes), row_bytes);
            }
            pixels.set_len(row_bytes * size.1 as usize);
        }
        unsafe { self.context.Unmap(&staging, 0) };

        // `RgbaImage` is a byte container here, not a colour-space claim: the
        // bytes are BGRA, which is what gpui's atlas uploads.
        let buffer = RgbaImage::from_raw(size.0, size.1, pixels)?;
        Some((Arc::new(RenderImage::new(vec![Frame::new(buffer)])), frame_pts))
    }
}
