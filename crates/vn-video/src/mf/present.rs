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
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{IMFMediaEngine, MFARGB};

pub struct Presenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    size: (u32, u32),
    /// What `TransferVideoFrame` writes into: BGRA, on the renderer's device,
    /// a render target because that is what the video processor needs.
    target: Option<ID3D11Texture2D>,
    /// CPU-readable mirror of `target`.
    staging: Option<ID3D11Texture2D>,
    /// Reused between frames so a 60 fps video does not allocate 60 buffers a
    /// second for the allocator to hand straight back.
    scratch: Vec<u8>,
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
            staging: None,
            scratch: Vec::new(),
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
        self.staging = None;
        self.size = (0, 0);
        self.scratch = Vec::new();
    }

    fn ensure(&mut self, size: (u32, u32)) -> Option<()> {
        if self.size == size && self.target.is_some() && self.staging.is_some() {
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
        self.target = target;
        self.staging = staging;
        self.size = size;
        self.scratch = vec![0u8; (size.0 as usize) * (size.1 as usize) * 4];
        Some(())
    }

    /// Pulls the engine's current frame at `size` and returns it as a gpui
    /// image. `None` on any device failure — a dropped frame, never a panic.
    pub fn transfer(
        &mut self,
        engine: &IMFMediaEngine,
        size: (u32, u32),
    ) -> Option<Arc<RenderImage>> {
        self.ensure(size)?;
        let target = self.target.as_ref()?.clone();
        let staging = self.staging.as_ref()?.clone();

        let dst = RECT { left: 0, top: 0, right: size.0 as i32, bottom: size.1 as i32 };
        // Transparent border: the destination is exactly the video's aspect
        // ratio (`fit_within`), so nothing should letterbox — but if a driver
        // rounds differently, transparent edges are invisible against whatever
        // CSS put behind the element, and opaque black ones would not be.
        let border = MFARGB { rgbAlpha: 0, rgbRed: 0, rgbGreen: 0, rgbBlue: 0 };
        unsafe { engine.TransferVideoFrame(&target, None, &dst, Some(&border)) }.ok()?;

        unsafe { self.context.CopyResource(&staging, &target) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.ok()?;
        let row_bytes = size.0 as usize * 4;
        let pitch = mapped.RowPitch as usize;
        // SAFETY: `Map` succeeded, so `pData` is `RowPitch * Height` readable
        // bytes of the staging texture we just created at exactly this size.
        unsafe {
            let src = mapped.pData as *const u8;
            for y in 0..size.1 as usize {
                std::ptr::copy_nonoverlapping(
                    src.add(y * pitch),
                    self.scratch.as_mut_ptr().add(y * row_bytes),
                    row_bytes,
                );
            }
        }
        unsafe { self.context.Unmap(&staging, 0) };

        // `RgbaImage` is a byte container here, not a colour-space claim: the
        // bytes are BGRA, which is what gpui's atlas uploads.
        let buffer = RgbaImage::from_raw(size.0, size.1, self.scratch.clone())?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
    }
}
