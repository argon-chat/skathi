//! Turning "the engine has a frame" into "gpui has an image".
//!
//! Default (zero-copy): `TransferVideoFrame` blits into a BGRA, cropped texture we own on the
//! renderer's device, and [`Presenter::transfer_gpu`] hands it to the host to copy into the atlas
//! tile. The remaining `RenderImage` is one stable, zero-filled placeholder per size that gives the
//! atlas a key.
//!
//! `SK_VIDEO_READBACK=1` ([`Presenter::transfer`]) reads the frame back through two staging
//! surfaces; it is the fallback when the atlas refuses the copy.
//!
//! Either way the frame is produced at the size it will be painted: MF's video processor scales
//! anyway, which cuts 8.3 MB/frame to 1.4 MB/frame (1080p into 800x450).

use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{IMFMediaEngine, MFARGB};

use crate::backend::GpuFrame;

struct Slot {
    texture: ID3D11Texture2D,
    pts: i64,
    filled: bool,
}

pub struct Presenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    size: (u32, u32),
    /// What `TransferVideoFrame` writes into: BGRA on the renderer's device, a render target (the
    /// video processor needs that); on the GPU path the atlas copies from it.
    target: Option<ID3D11Texture2D>,
    slots: Vec<Slot>,
    write: usize,
    /// The GPU path's stable, zero-filled `RenderImage`, one per size: gpui keys atlas tiles by
    /// `RenderImage::id`, so one id means one tile updated in place. Its bytes are never read.
    placeholder: Option<Arc<RenderImage>>,
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
            // `SK_VIDEO_READBACK=1` forces the v1 path, to tell whether the zero-copy blit is the problem without a rebuild.
            gpu: !std::env::var("SK_VIDEO_READBACK").is_ok_and(|v| v == "1"),
        }
    }

    pub fn is_gpu(&self) -> bool {
        self.gpu
    }

    pub fn has_pending_readback(&self) -> bool {
        self.slots.iter().any(|slot| slot.filled)
    }

    #[cfg(feature = "webgpu")]
    pub fn snapshot(
        &self,
        engine: &IMFMediaEngine,
        size: (u32, u32),
        timestamp: i64,
    ) -> Option<redjs_webgpu::video::Frame> {
        super::webgpu::capture(&self.device, engine, size, timestamp).ok()
    }

    /// Called when `Window::upload_image_from_gpu` returned `false`: falls back to readback permanently.
    pub fn demote_to_readback(&mut self) {
        if !self.gpu {
            return;
        }
        eprintln!(
            "[skathi] <video>: the renderer's atlas refused the zero-copy blit — falling \
             back to the CPU readback path for this element. SK_LOG=info names the reason."
        );
        self.gpu = false;
        self.reset();
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn reset(&mut self) {
        self.target = None;
        self.slots.clear();
        self.write = 0;
        self.size = (0, 0);
        self.placeholder = None;
    }

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
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    ..Default::default()
                },
                None,
                Some(&mut target),
            )
        }
        .ok()?;
        let pixels = vec![0u8; size.0 as usize * size.1 as usize * 4];
        let buffer = RgbaImage::from_raw(size.0, size.1, pixels)?;
        // `dynamic`: the GPU fills its tile, so its zeroed pixels must never be resampled into a
        // smaller copy (gpui `ImageResampler`).
        self.placeholder = Some(Arc::new(
            RenderImage::new(vec![Frame::new(buffer)]).dynamic(),
        ));
        self.target = target;
        self.slots.clear();
        self.write = 0;
        self.size = size;
        Some(())
    }

    /// The zero-copy present: transfer into our own texture and hand the host a borrow. A
    /// `CopySubresourceRegion` on the same context is ordered behind the transfer by the driver, so a
    /// paused element's single seek frame is correct.
    pub fn transfer_gpu(
        &mut self,
        engine: &IMFMediaEngine,
        size: (u32, u32),
        pts: i64,
    ) -> Option<(Arc<RenderImage>, i64, GpuFrame)> {
        self.ensure_gpu(size)?;
        let target = self.target.as_ref()?.clone();

        let dst = RECT {
            left: 0,
            top: 0,
            right: size.0 as i32,
            bottom: size.1 as i32,
        };
        let border = MFARGB {
            rgbAlpha: 0,
            rgbRed: 0,
            rgbGreen: 0,
            rgbBlue: 0,
        };
        unsafe { engine.TransferVideoFrame(&target, None, &dst, Some(&border)) }.ok()?;

        let image = self.placeholder.as_ref()?.clone();
        let gpu = GpuFrame {
            texture: target.as_raw(),
            size,
        };
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
            // gpui's `RENDER_TARGET_FORMAT` and Polychrome atlas format; asking the engine for it makes MF insert its video processor.
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
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
            slots.push(Slot {
                texture: staging?,
                pts: 0,
                filled: false,
            });
        }
        self.target = target;
        self.slots = slots;
        self.write = 0;
        self.size = size;
        Some(())
    }

    /// Pulls the engine's current frame at `size` and returns a gpui image of the frame before it,
    /// with that frame's PTS.
    ///
    /// Reading the staging texture just written makes `Map` block the immediate context (also gpui's)
    /// on the GPU. With two staging surfaces the copy queued this frame is read next frame, at the
    /// cost of one frame of latency; `D3D11_MAP_FLAG_DO_NOT_WAIT` guards a busy GPU.
    ///
    /// `pipeline: false` reads back the frame just written, blocking. A PAUSED element must do this:
    /// it produces one frame per seek, so with the pipeline on every seek would display the previous one.
    pub fn transfer(
        &mut self,
        engine: &IMFMediaEngine,
        size: (u32, u32),
        pts: i64,
        pipeline: bool,
    ) -> Option<(Arc<RenderImage>, i64)> {
        self.ensure(size)?;
        let target = self.target.as_ref()?.clone();

        let dst = RECT {
            left: 0,
            top: 0,
            right: size.0 as i32,
            bottom: size.1 as i32,
        };
        let border = MFARGB {
            rgbAlpha: 0,
            rgbRed: 0,
            rgbGreen: 0,
            rgbBlue: 0,
        };
        unsafe { engine.TransferVideoFrame(&target, None, &dst, Some(&border)) }.ok()?;

        let write = self.write;
        unsafe {
            self.context
                .CopyResource(&self.slots[write].texture, &target)
        };
        self.slots[write].pts = pts;
        self.slots[write].filled = true;

        let read = if pipeline {
            self.write = 1 - write;
            1 - write
        } else {
            self.slots[1 - write].filled = false;
            write
        };

        if !self.slots[read].filled {
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
            unsafe {
                self.context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            }
            .ok()?;
        }
        let row_bytes = size.0 as usize * 4;
        let pitch = mapped.RowPitch as usize;
        let mut pixels: Vec<u8> = Vec::with_capacity(row_bytes * size.1 as usize);
        // SAFETY: `Map` succeeded, so `pData` is `RowPitch * Height` readable
        // bytes of the staging texture we just created at exactly this size,
        // and `pixels` has capacity for every byte written below.
        unsafe {
            let src = mapped.pData as *const u8;
            let dst = pixels.as_mut_ptr();
            for y in 0..size.1 as usize {
                std::ptr::copy_nonoverlapping(
                    src.add(y * pitch),
                    dst.add(y * row_bytes),
                    row_bytes,
                );
            }
            pixels.set_len(row_bytes * size.1 as usize);
        }
        unsafe { self.context.Unmap(&staging, 0) };

        let buffer = RgbaImage::from_raw(size.0, size.1, pixels)?;
        Some((
            Arc::new(RenderImage::new(vec![Frame::new(buffer)]).dynamic()),
            frame_pts,
        ))
    }
}
