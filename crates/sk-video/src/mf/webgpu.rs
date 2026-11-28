//! Take a private media-resolution snapshot. The atlas target is mutable and can be paint-sized.
use redjs_webgpu::{
    backend::Context,
    video::{wgpu, Frame, NativeFrame, PixelFormat},
};
use std::sync::{Arc, Mutex};
use windows::{
    core::{IUnknown, Interface, Owned},
    Win32::{
        Foundation::{E_FAIL, GENERIC_ALL, RECT},
        Graphics::{
            Direct3D11::*,
            Direct3D11on12::ID3D11On12Device2,
            Direct3D12::*,
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                IDXGIResource1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
            },
        },
        Media::MediaFoundation::{IMFMediaEngine, MFARGB},
    },
};
type Result<T> = windows::core::Result<T>;

struct Image {
    device: ID3D11Device,
    frame: ID3D11Texture2D,
    desc: D3D11_TEXTURE2D_DESC,
    checked_out: Mutex<bool>,
}

unsafe fn texture(device: &ID3D11Device, desc: &D3D11_TEXTURE2D_DESC) -> Result<ID3D11Texture2D> {
    unsafe {
        let mut target = None;
        device.CreateTexture2D(desc, None, Some(&mut target))?;
        target.ok_or_else(|| E_FAIL.into())
    }
}

pub(super) fn capture(
    device: &sk_windows::win32::ID3D11Device,
    engine: &sk_windows::win32::IMFMediaEngine,
    (width, height): (u32, u32),
    timestamp: i64,
) -> Result<Frame> {
    // wgpu exposes its own D3D projection. Both borrowed interfaces keep the same IID and owner.
    let device_raw = sk_windows::core::Interface::as_raw(device);
    let engine_raw = sk_windows::core::Interface::as_raw(engine);
    let device = unsafe { ID3D11Device::from_raw_borrowed(&device_raw) }.ok_or(E_FAIL)?;
    let engine = unsafe { IMFMediaEngine::from_raw_borrowed(&engine_raw) }.ok_or(E_FAIL)?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
        ..Default::default()
    };
    let frame = unsafe { texture(device, &desc)? };
    let rect = RECT {
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
    };
    unsafe { engine.TransferVideoFrame(&frame, None, &rect, Some(&MFARGB::default()))? };
    Ok(Frame::native(
        width,
        height,
        timestamp,
        PixelFormat::Bgra,
        Arc::new(Image {
            checked_out: Mutex::new(false),
            device: device.clone(),
            frame,
            desc,
        }),
    ))
}

impl Image {
    fn descriptor(&self, usage: wgpu::TextureUsages) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("Media Foundation video"),
            size: wgpu::Extent3d {
                width: self.desc.Width,
                height: self.desc.Height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage,
            view_formats: &[],
        }
    }
    unsafe fn wrap(
        &self,
        device: &wgpu::Device,
        resource: ID3D12Resource,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        unsafe {
            let desc = self.descriptor(usage);
            let hal = wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                desc.format,
                desc.dimension,
                desc.size,
                1,
                1,
            );
            // In D3D12 PRESENT is COMMON. The decoder has already initialized all texels.
            device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal,
                &desc,
                wgpu::TextureUses::PRESENT,
            )
        }
    }

    unsafe fn native(&self, context: &Context, checked_out: &mut bool) -> Result<wgpu::Texture> {
        unsafe {
            let (device, queue) = {
                let dx = context
                    .device
                    .as_hal::<wgpu::hal::api::Dx12>()
                    .ok_or(E_FAIL)?;
                (dx.raw_device().clone(), dx.raw_queue().clone())
            };
            let immediate = self.device.GetImmediateContext()?;
            if let Ok(on12) = self.device.cast::<ID3D11On12Device2>() {
                let producer: ID3D12Device = on12.GetD3D12Device()?;
                if producer.cast::<IUnknown>()? != device.cast::<IUnknown>()? {
                    return Err(E_FAIL.into());
                }
                let done: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
                let result = context.device.create_texture(&self.descriptor(
                    wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                ));
                let mut encoder = context.device.create_command_encoder(&Default::default());
                let raw: ID3D12Resource = on12.UnwrapUnderlyingResource(&self.frame, &queue)?;
                *checked_out = true;
                immediate.Flush();
                let source = self.wrap(&context.device, raw, wgpu::TextureUsages::COPY_SRC);
                encoder.copy_texture_to_texture(
                    source.as_image_copy(),
                    result.as_image_copy(),
                    result.size(),
                );
                encoder.transition_resources(
                    std::iter::empty(),
                    std::iter::once(wgpu::TextureTransition {
                        texture: &source,
                        selector: None,
                        state: wgpu::TextureUses::PRESENT,
                    }),
                );
                context.queue.submit([encoder.finish()]);
                // Return ownership with a GPU fence before the decoder or another logical device can use it.
                queue.Signal(&done, 1)?;
                on12.ReturnUnderlyingResource(&self.frame, 1, &1, [Some(done)].as_ptr())?;
                *checked_out = false;
                crate::trace(format_args!("WebGPU video copied on the D3D12 queue"));
                return Ok(result);
            }

            // Native D3D11 produces into its own queue. Export a private copy and its completion fence.
            let shared = texture(
                &self.device,
                &D3D11_TEXTURE2D_DESC {
                    MiscFlags: (D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE).0
                        as u32,
                    ..self.desc
                },
            )?;
            let resource = shared.cast::<IDXGIResource1>()?;
            let handle = Owned::new(resource.CreateSharedHandle(
                None,
                (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                None,
            )?);
            let mut dst = None;
            device.OpenSharedHandle(*handle, &mut dst)?;
            let mut done: Option<ID3D11Fence> = None;
            self.device.cast::<ID3D11Device5>()?.CreateFence(
                0,
                D3D11_FENCE_FLAG_SHARED,
                &mut done,
            )?;
            let done = done.ok_or(E_FAIL)?;
            let handle = Owned::new(done.CreateSharedHandle(None, GENERIC_ALL.0, None)?);
            let mut ready: Option<ID3D12Fence> = None;
            device.OpenSharedHandle(*handle, &mut ready)?;
            immediate.CopyResource(&shared, &self.frame);
            immediate.cast::<ID3D11DeviceContext4>()?.Signal(&done, 1)?;
            immediate.Flush();
            queue.Wait(&ready.ok_or(E_FAIL)?, 1)?;
            crate::trace(format_args!("WebGPU video imported through DXGI"));
            Ok(self.wrap(
                &context.device,
                dst.ok_or(E_FAIL)?,
                wgpu::TextureUsages::TEXTURE_BINDING,
            ))
        }
    }

    unsafe fn readback(&self) -> Result<Vec<u8>> {
        unsafe {
            let staging = texture(
                &self.device,
                &D3D11_TEXTURE2D_DESC {
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..self.desc
                },
            )?;
            let row = self.desc.Width as usize * 4;
            let height = self.desc.Height as usize;
            let mut data = Vec::with_capacity(row * height);
            let immediate = self.device.GetImmediateContext()?;
            immediate.CopyResource(&staging, &self.frame);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            immediate.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            for y in 0..height {
                let ptr = mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize);
                data.extend_from_slice(std::slice::from_raw_parts(ptr, row));
            }
            immediate.Unmap(&staging, 0);
            Ok(data)
        }
    }
}
impl NativeFrame for Image {
    fn texture(&self, context: &Context) -> std::result::Result<wgpu::Texture, String> {
        // Clones may use different queues. Serialize handoff of this one immutable source.
        let mut checked_out = self
            .checked_out
            .lock()
            .map_err(|_| "D3D11 video handoff failed")?;
        if *checked_out {
            return Err("D3D11 video ownership was lost".into());
        }
        match unsafe { self.native(context, &mut checked_out) } {
            Ok(texture) => return Ok(texture),
            // The translation layer cannot access a resource still checked out to D3D12.
            Err(error) if *checked_out => return Err(error.to_string()),
            Err(error) => crate::trace(format_args!("WebGPU video readback fallback: {error}")),
        }
        // A fallback adapter can be on a different physical GPU.
        let bytes = unsafe { self.readback() }.map_err(|e| e.to_string())?;
        Frame::packed(
            self.desc.Width,
            self.desc.Height,
            0,
            bytes,
            PixelFormat::Bgra,
        )
        .ok_or("Invalid Media Foundation frame")?
        .texture(context)
    }
}
