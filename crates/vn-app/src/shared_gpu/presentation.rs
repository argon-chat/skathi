//! GPU-only canvas display: convert premultiplied RGBA/BGRA into the atlas's
//! straight BGRA on wgpu, then expose an acquired D3D11On12 texture to GPUI.
use super::*;
use vn_runtime::redjs::webapi::webgpu::canvas::{NativeFrame, NativePresenter, NativeSource};
use windows::Win32::Graphics::{Direct3D11::*, Direct3D11on12::*, Direct3D12::*};

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) i:u32) -> @builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
    return vec4(p[i],0.,1.);
}
fn pixel(p:vec4<f32>, opaque:bool) -> vec4<f32> {
    let v=vec4<u32>(round(textureLoad(source,vec2<i32>(p.xy),0)*255.));
    if opaque { return vec4<f32>(vec3<f32>(v.rgb)/255.,1.); }
    if v.a==0u || v.a==255u { return vec4<f32>(v)/255.; }
    let rgb=min((v.rgb*vec3<u32>(255u)+vec3<u32>(v.a/2u))/vec3<u32>(v.a),vec3<u32>(255u));
    return vec4<f32>(vec3<f32>(rgb),f32(v.a))/255.;
}
@fragment fn premultiplied(@builtin(position) p:vec4<f32>) -> @location(0) vec4<f32> { return pixel(p,false); }
@fragment fn opaque(@builtin(position) p:vec4<f32>) -> @location(0) vec4<f32> { return pixel(p,true); }
"#;

pub(super) struct Presenter {
    context: Arc<Context>,
    device: ID3D11Device,
    immediate: ID3D11DeviceContext,
    on12: ID3D11On12Device,
    conversion: Mutex<Option<Conversion>>,
}
struct Conversion {
    layout: wgpu::BindGroupLayout,
    opaque: wgpu::RenderPipeline,
    premultiplied: wgpu::RenderPipeline,
}
impl Conversion {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas atlas alpha conversion"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("GPU canvas -> straight BGRA atlas"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = pipeline("opaque");
        let premultiplied = pipeline("premultiplied");
        Self {
            layout,
            opaque,
            premultiplied,
        }
    }
}
impl Presenter {
    pub(super) fn new(
        context: Arc<Context>,
        device: ID3D11Device,
        immediate: ID3D11DeviceContext,
    ) -> Result<Self> {
        let on12 = device.cast()?;
        Ok(Self {
            context,
            device,
            immediate,
            on12,
            conversion: Mutex::new(None),
        })
    }
}
struct Frame {
    output: wgpu::Texture,
    texture: Option<ID3D11Texture2D>,
    resource: Option<ID3D11Resource>,
    on12: ID3D11On12Device,
    immediate: ID3D11DeviceContext,
    _context: Arc<Context>,
    copy_lock: Mutex<()>,
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D11On12CanvasFrame")
            .field("width", &self.output.width())
            .field("height", &self.output.height())
            .finish()
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        drop(self.texture.take());
        drop(self.resource.take());
        self.output.destroy();
        // D3D11 defers destruction: release both views before this flush.
        unsafe {
            self.immediate.Flush();
        }
    }
}
struct Acquired<'a>(&'a Frame);
impl Drop for Acquired<'_> {
    fn drop(&mut self) {
        unsafe {
            self.0
                .on12
                .ReleaseWrappedResources(&[Some(self.0.resource.as_ref().unwrap().clone())]);
            self.0.immediate.Flush();
        }
    }
}
// SAFETY: all interfaces are cloned COM owners, output stays alive, and the
// guard releases/flushes even if the synchronous atlas callback unwinds.
unsafe impl NativeFrame for Frame {
    fn copy_d3d11(
        &self,
        copy: &mut dyn FnMut(*mut std::ffi::c_void) -> bool,
    ) -> std::result::Result<bool, String> {
        let _copy = self.copy_lock.lock().unwrap_or_else(|e| e.into_inner());
        if self._context.loss().is_some() {
            return Err("Shared GPU generation is lost".into());
        }
        unsafe {
            self.on12
                .AcquireWrappedResources(&[Some(self.resource.as_ref().unwrap().clone())]);
        }
        let _acquired = Acquired(self);
        Ok(copy(self.texture.as_ref().unwrap().as_raw()))
    }
}
impl NativePresenter for Presenter {
    fn prepare(
        &self,
        source: NativeSource<'_>,
    ) -> std::result::Result<Option<Arc<dyn NativeFrame>>, String> {
        if !Arc::ptr_eq(&self.context, &source.client.context) {
            return Ok(None);
        }
        self.prepare_frame(source)
            .map(|frame| Some(Arc::new(frame) as Arc<dyn NativeFrame>))
            .map_err(|e| e.to_string())
    }
}
impl Presenter {
    fn prepare_frame(&self, source: NativeSource<'_>) -> Result<Frame> {
        let mut conversion = self.conversion.lock().unwrap_or_else(|e| e.into_inner());
        let conversion = conversion.get_or_insert_with(|| Conversion::new(&self.context.device));
        let output = self
            .context
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("D3D11On12 canvas presentation"),
                size: wgpu::Extent3d {
                    width: source.width,
                    height: source.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
        let input = source.texture.create_view(&Default::default());
        let view = output.create_view(&Default::default());
        let group = self
            .context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &conversion.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input),
                }],
            });
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas alpha conversion"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(if source.opaque {
                &conversion.opaque
            } else {
                &conversion.premultiplied
            });
            pass.set_bind_group(0, Some(&group), &[]);
            pass.draw(0..3, 0..1);
        }
        self.context.queue.submit([encoder.finish()]);
        let raw = unsafe {
            output
                .as_hal::<wgpu::hal::api::Dx12>()
                .context("Native canvas output is not D3D12")?
                .raw_resource()
                .clone()
        };
        let resource: ID3D11Resource = unsafe {
            let flags = D3D11_RESOURCE_FLAGS {
                BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
                ..Default::default()
            };
            let mut wrapped = None;
            self.on12.CreateWrappedResource(
                &raw,
                &flags,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                &mut wrapped,
            )?;
            wrapped.context("No D3D11 wrapped canvas resource")?
        };
        let texture = resource.cast::<ID3D11Texture2D>()?;
        // New wrapped resources start acquired. Hand ownership back immediately;
        // a painting window acquires it only around its own atlas GPU copy.
        unsafe {
            self.on12.ReleaseWrappedResources(&[Some(resource.clone())]);
            self.immediate.Flush();
        }
        // Require the exact renderer device, not merely the same GPU adapter.
        let actual = unsafe { texture.GetDevice()? };
        ensure!(
            actual.cast::<IUnknown>()? == self.device.cast::<IUnknown>()?,
            "Wrapped canvas belongs to another D3D11 device"
        );
        Ok(Frame {
            output,
            texture: Some(texture),
            resource: Some(resource),
            on12: self.on12.clone(),
            immediate: self.immediate.clone(),
            _context: self.context.clone(),
            copy_lock: Mutex::new(()),
        })
    }
}
