//! Turning "the engine has a frame" into "gpui has an image".
//!
//! # Two paths, and why the default changed
//!
//! `TransferVideoFrame` blits the current frame into a texture we own, on the
//! renderer's device, already BGRA and already cropped. That texture is *one
//! `CopySubresourceRegion` away* from being a gpui atlas tile.
//!
//! **v2 (the default) takes that copy.** `gpui::PlatformAtlas` grew
//! `upload_from_gpu`, so [`Presenter::transfer_gpu`] hands the transfer target
//! straight to the host, which blits it into the tile. The frame never touches
//! system memory: no staging surface, no `Map`, no `Vec<u8>`, no per-frame
//! `RenderImage`. The `RenderImage` that remains is one stable, zero-filled
//! placeholder per size, alive only so the atlas has a key.
//!
//! **v1 (`VN_VIDEO_READBACK=1`) is still here**, in [`Presenter::transfer`],
//! because it is the fallback for a machine where the atlas refuses the copy
//! (a private decode device, `VN_VIDEO_DEVICE=private`), and because it is the
//! measurement baseline the m13 report compares against. It reads the frame
//! back through two staging surfaces and costs [measured] 1.18 ms p50 at
//! 800×450 and 2.87 ms p50 at 1080p.
//!
//! One thing is common to both and is load-bearing on either:
//!
//! * **The frame is produced at the size it will be PAINTED**, not the size it
//!   was encoded at. MF's video processor is scaling anyway, so asking it for
//!   800×450 instead of 1920×1080 turns 8.3 MB/frame into 1.4 MB/frame. On the
//!   readback path that was the difference between viable and not; on the GPU
//!   path it is still a smaller atlas tile and less VRAM traffic.
//!
//! CONTRACT.md §5 records the measured cost of both.

use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{IMFMediaEngine, MFARGB};

use crate::backend::GpuFrame;

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
    /// a render target because that is what the video processor needs. On the
    /// GPU path this is also what the atlas copies FROM, so it is the whole
    /// pipeline.
    target: Option<ID3D11Texture2D>,
    /// TWO staging surfaces, not one. See [`Presenter::transfer`]. Empty on
    /// the GPU path — nothing is read back, so nothing is staged.
    slots: Vec<Slot>,
    /// The slot this frame's copy goes into; the other one is read.
    write: usize,
    /// The GPU path's stable, zero-filled `RenderImage`: one per size, kept
    /// for the life of the element.
    ///
    /// gpui keys atlas tiles by `RenderImage::id`, so a fresh image per frame
    /// is a fresh tile per frame — the leak `<lottie>`'s two-generation retire
    /// exists to plug. Keeping ONE id means one tile, updated in place, and
    /// the retire machinery only ever runs on a resize.
    ///
    /// Its bytes are never read on this path: `paint_image` finds the tile
    /// already populated and never runs its `build` closure. The buffer exists
    /// because `RenderImage::size()` reads the image's dimensions, and the
    /// atlas needs the tile to be the right size.
    placeholder: Option<Arc<RenderImage>>,
    /// `false` once the atlas has refused a GPU upload — see
    /// [`Presenter::demote_to_readback`].
    gpu: bool,
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
            placeholder: None,
            // `VN_VIDEO_READBACK=1` forces the v1 path. Kept as an escape
            // hatch for the same reason `VN_VIDEO_DEVICE=private` is: when a
            // machine misbehaves, "is it the zero-copy blit?" should be one
            // environment variable away, not a rebuild.
            gpu: !std::env::var("VN_VIDEO_READBACK").is_ok_and(|v| v == "1"),
        }
    }

    /// Is this presenter producing GPU frames?
    pub fn is_gpu(&self) -> bool {
        self.gpu
    }

    /// Called by the host when `Window::upload_image_from_gpu` returned
    /// `false` — the atlas refused the texture (wrong device, wrong format).
    /// Falls back to the readback path permanently rather than painting
    /// nothing, and forces the next `present` to re-allocate.
    pub fn demote_to_readback(&mut self) {
        if !self.gpu {
            return;
        }
        eprintln!(
            "[vue-native] <video>: the renderer's atlas refused the zero-copy blit — falling \
             back to the CPU readback path for this element. VN_LOG=info names the reason."
        );
        self.gpu = false;
        self.reset();
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
        self.placeholder = None;
    }

    /// The GPU path's allocation: the transfer target and nothing else.
    fn ensure_gpu(&mut self, size: (u32, u32)) -> Option<()> {
        if self.size == size && self.target.is_some() && self.placeholder.is_some() {
            return Some(());
        }
        let mut target = None;
        unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.0,
                    Height: size.1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    // RENDER_TARGET because the video processor writes here;
                    // SHADER_RESOURCE is not needed for a copy source but
                    // costs nothing and keeps the desc identical to v1's.
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    ..Default::default()
                },
                None,
                Some(&mut target),
            )
        }
        .ok()?;
        // One zeroed buffer per size. `RgbaImage` is a byte container here, not
        // a colour-space claim — same as on the readback path.
        let pixels = vec![0u8; size.0 as usize * size.1 as usize * 4];
        let buffer = RgbaImage::from_raw(size.0, size.1, pixels)?;
        self.placeholder = Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])));
        self.target = target;
        self.slots.clear();
        self.write = 0;
        self.size = size;
        Some(())
    }

    /// The zero-copy present: transfer into our own texture and hand the host
    /// a borrow of it. Nothing is read back, nothing is allocated per frame,
    /// and there is no pipelining question — a `CopySubresourceRegion` on the
    /// same context is ordered behind the transfer by the driver, so a paused
    /// element's single seek frame is as correct as a playing one's.
    ///
    /// That last point is worth stating plainly, because it is the bug the v1
    /// path had to work around: the readback needed two staging surfaces to
    /// avoid blocking on `Map`, and two staging surfaces showed the *previous*
    /// seek's frame on a paused video. GPU→GPU has neither problem.
    pub fn transfer_gpu(
        &mut self,
        engine: &IMFMediaEngine,
        size: (u32, u32),
        pts: i64,
    ) -> Option<(Arc<RenderImage>, i64, GpuFrame)> {
        self.ensure_gpu(size)?;
        let target = self.target.as_ref()?.clone();

        let dst = RECT { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 };
        let border = MFARGB { rgbAlpha: 0, rgbRed: 0, rgbGreen: 0, rgbBlue: 0 };
        unsafe { engine.TransferVideoFrame(&target, None, &dst, Some(&border)) }.ok()?;

        let image = self.placeholder.as_ref()?.clone();
        let gpu = GpuFrame { texture: target.as_raw(), size };
        Some((image, pts, gpu))
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
