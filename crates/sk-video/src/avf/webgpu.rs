use super::frame::{bgra, size_of};
use objc2::rc::Retained;
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetIOSurface};
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use redjs_webgpu::{
    backend::Context,
    video::{wgpu, Frame, NativeFrame},
};

#[derive(Clone)]
pub(super) struct Image(pub Retained<CVPixelBuffer>);
// The decoder publishes an immutable buffer. Retaining it prevents pool reuse, and
// CoreVideo permits read-only access and retain/release from other threads.
unsafe impl Send for Image {}
unsafe impl Sync for Image {}

impl NativeFrame for Image {
    fn texture(&self, context: &Context) -> Result<wgpu::Texture, String> {
        let (width, height) = size_of(&self.0);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let device = &context.device;
        let native = unsafe { device.as_hal::<wgpu::hal::api::Metal>() };
        let surface = CVPixelBufferGetIOSurface(Some(&self.0));
        if let (Some(hal), Some(surface)) = (native, surface) {
            let desc = MTLTextureDescriptor::new();
            desc.setTextureType(MTLTextureType::Type2D);
            desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            unsafe {
                desc.setWidth(width as usize);
                desc.setHeight(height as usize);
            }
            desc.setUsage(MTLTextureUsage::ShaderRead);
            let view = hal
                .raw_device()
                .newTextureWithDescriptor_iosurface_plane(&desc, &surface, 0);
            drop(hal);
            if let Some(view) = view {
                let keep = self.clone();
                // Keep the pool buffer through the last GPU use, including work already submitted.
                let raw = unsafe {
                    wgpu::hal::metal::Device::texture_from_raw(
                        view,
                        wgpu::TextureFormat::Bgra8Unorm,
                        MTLTextureType::Type2D,
                        1,
                        1,
                        size.into(),
                        Some(Box::new(move || drop(keep))),
                    )
                };
                return Ok(unsafe {
                    device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                        raw,
                        &wgpu::TextureDescriptor {
                            label: Some("AVFoundation video"),
                            size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Bgra8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        },
                        wgpu::TextureUses::RESOURCE,
                    )
                });
            }
        }
        let bytes = bgra(&self.0, (width, height), (width, height))
            .ok_or("AVFoundation frame is unreadable")?;
        Frame::packed(width, height, 0, bytes, true)
            .ok_or("Invalid AVFoundation frame")?
            .texture(context)
    }
}
