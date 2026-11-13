//! Native canvas conversion with GPU-queue fences, no D3D11On12 bridge.
use std::{
    ffi::c_void,
    sync::{Arc, Mutex, Weak},
};

use anyhow::{Context as _, Result};
use vn_runtime::redjs::webapi::webgpu::{
    backend::Context,
    canvas::{NativeFrame, NativePresenter, NativeSource},
};
use windows::{core::Interface, Win32::Graphics::Direct3D12::*};

use super::{ensure_generation, HostState, Lifetime, CONVERTERS, FRAMES};

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

pub(super) struct Presenter {
    state: Arc<HostState>,
    conversions: Mutex<Vec<Weak<CachedConversion>>>,
}

struct CachedConversion {
    context: Weak<Context>,
    generation: u64,
    pipeline: Conversion,
    fence: ID3D12Fence,
    native_queue: ID3D12CommandQueue,
    submission: Mutex<u64>,
    _lifetime: Lifetime,
}

impl CachedConversion {
    fn new(context: &Arc<Context>, generation: u64) -> Result<Self> {
        let (fence, native_queue) = {
            let native = unsafe { context.device.as_hal::<wgpu::hal::api::Dx12>() }
                .context("Native canvas context is not D3D12")?;
            let fence = unsafe { native.raw_device().CreateFence(0, D3D12_FENCE_FLAG_NONE)? };
            (fence, native.raw_queue().clone())
        };
        Ok(Self {
            context: Arc::downgrade(context),
            generation,
            pipeline: Conversion::new(&context.device),
            fence,
            native_queue,
            submission: Mutex::new(0),
            _lifetime: Lifetime::new(&CONVERTERS),
        })
    }
}

impl Presenter {
    pub(super) fn new(state: Arc<HostState>) -> Self {
        Self {
            state,
            conversions: Mutex::new(Vec::new()),
        }
    }

    fn conversion(&self, context: &Arc<Context>, generation: u64) -> Result<Arc<CachedConversion>> {
        let mut conversions = self
            .conversions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        conversions.retain(|cached| cached.strong_count() != 0);
        let wanted = Arc::downgrade(context);
        if let Some(cached) = conversions
            .iter()
            .filter_map(Weak::upgrade)
            .find(|cached| cached.generation == generation && cached.context.ptr_eq(&wanted))
        {
            return Ok(cached);
        }
        let cached = Arc::new(CachedConversion::new(context, generation)?);
        conversions.push(Arc::downgrade(&cached));
        Ok(cached)
    }

    fn prepare_frame(&self, source: NativeSource<'_>, generation: u64) -> Result<Frame> {
        let context = &source.client.context;
        let conversion = self.conversion(context, generation)?;
        let output = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("native D3D12 canvas presentation"),
            size: wgpu::Extent3d {
                width: source.width,
                height: source.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let input = source.texture.create_view(&Default::default());
        let view = output.create_view(&Default::default());
        let group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("native canvas alpha conversion input"),
                layout: &conversion.pipeline.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input),
                }],
            });
        let mut encoder = context.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native canvas alpha conversion"),
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
                &conversion.pipeline.opaque
            } else {
                &conversion.pipeline.premultiplied
            });
            pass.set_bind_group(0, Some(&group), &[]);
            pass.draw(0..3, 0..1);
        }
        let ready_value = {
            let mut value = conversion
                .submission
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let next = value
                .checked_add(1)
                .context("Canvas readiness fence exhausted")?;
            context.queue.submit([encoder.finish()]);
            // submit() has enqueued the render pass on this raw queue. The
            // renderer waits for this signal on its own GPU queue, never CPU.
            unsafe {
                conversion.native_queue.Signal(&conversion.fence, next)?;
            }
            *value = next;
            next
        };
        let resource = unsafe {
            output
                .as_hal::<wgpu::hal::api::Dx12>()
                .context("Native canvas output is not D3D12")?
                .raw_resource()
                .clone()
        };
        ensure_generation(generation, context)?;
        Ok(Frame {
            output,
            resource,
            context: context.clone(),
            conversion,
            generation,
            ready_value,
            _lifetime: Lifetime::new(&FRAMES),
        })
    }
}

impl NativePresenter for Presenter {
    fn prepare(&self, source: NativeSource<'_>) -> Result<Option<Arc<dyn NativeFrame>>, String> {
        if let Some(loss) = source.client.context.loss() {
            return Err(format!("Shared GPU generation is lost: {}", loss.message));
        }
        let Some(generation) = self.state.current_generation(&source.client.context) else {
            if let Some(loss) = source.client.context.loss() {
                return Err(format!("Shared GPU generation is lost: {}", loss.message));
            }
            return Ok(None);
        };
        self.prepare_frame(source, generation)
            .map(|frame| Some(Arc::new(frame) as Arc<dyn NativeFrame>))
            .map_err(|error| format!("{error:#}"))
    }
}

struct Frame {
    output: wgpu::Texture,
    resource: ID3D12Resource,
    // The presenter only keeps a Weak conversion cache. A published frame
    // retains its pipeline/fence until the atlas has finished consuming it.
    conversion: Arc<CachedConversion>,
    generation: u64,
    ready_value: u64,
    _lifetime: Lifetime,
    // Drop the last conversion/fence before releasing its physical owner.
    context: Arc<Context>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeD3D12CanvasFrame")
            .field("width", &self.output.width())
            .field("height", &self.output.height())
            .field("generation", &self.generation)
            .finish()
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        self.output.destroy();
    }
}

// SAFETY: resource, readiness fence and physical device remain owned by this
// frame. The consumer synchronously records a GPU wait/copy, restores the
// RENDER_TARGET state, and retains the frame until its own fence completes.
// Repeated copies only read this immutable output; no JSC values cross here.
unsafe impl NativeFrame for Frame {
    fn copy_d3d11(&self, _copy: &mut dyn FnMut(*mut c_void) -> bool) -> Result<bool, String> {
        Ok(false)
    }

    fn copy_d3d12(
        &self,
        copy: &mut dyn FnMut(*mut c_void, *mut c_void, u64) -> bool,
    ) -> Result<bool, String> {
        if let Some(loss) = self.context.loss() {
            return Err(format!("Shared GPU generation is lost: {}", loss.message));
        }
        let active = gpui_windows::renderer_d3d12_device();
        if !active.is_some_and(|renderer| renderer.generation == self.generation) {
            self.context
                .notify_lost("unknown", "Canvas belongs to a retired renderer generation");
            return Err("Canvas belongs to a retired renderer generation".into());
        }
        Ok(copy(
            self.resource.as_raw(),
            self.conversion.fence.as_raw(),
            self.ready_value,
        ))
    }
}
