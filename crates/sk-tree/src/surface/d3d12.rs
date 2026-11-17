//! The native D3D12 half of a surface, under the D3D12 renderer: frames are copied on a queue of
//! the surface's own on the renderer's device, after a GPU wait on the producer's fence, into one
//! of a few surface textures. The element's tick hands a finished one to the atlas
//! (`write_image_tile_d3d12`), so the renderer's queue never waits on a producer. A half-float
//! texture is first drawn into an 8-bit one of its own, on the same queue, when the tick asks.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, HANDLE, RECT, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D12::{
    D3D12SerializeRootSignature, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
    ID3D12DescriptorHeap, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList,
    ID3D12PipelineState, ID3D12Resource, ID3D12RootSignature, D3D12_BLEND_DESC, D3D12_BLEND_ONE,
    D3D12_BLEND_OP_ADD, D3D12_BLEND_ZERO, D3D12_BOX, D3D12_COLOR_WRITE_ENABLE_ALL,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_CULL_MODE_NONE,
    D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_DESCRIPTOR_RANGE,
    D3D12_DESCRIPTOR_RANGE_TYPE_SRV, D3D12_FENCE_FLAG_SHARED, D3D12_FILL_MODE_SOLID,
    D3D12_GRAPHICS_PIPELINE_STATE_DESC, D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES,
    D3D12_HEAP_TYPE_DEFAULT, D3D12_LOGIC_OP_NOOP, D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
    D3D12_RASTERIZER_DESC, D3D12_RENDER_TARGET_BLEND_DESC, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    D3D12_RESOURCE_STATES, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST,
    D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
    D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_TRANSITION_BARRIER,
    D3D12_ROOT_DESCRIPTOR_TABLE, D3D12_ROOT_PARAMETER, D3D12_ROOT_PARAMETER_0,
    D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE, D3D12_ROOT_SIGNATURE_DESC, D3D12_SHADER_BYTECODE,
    D3D12_SHADER_VISIBILITY_PIXEL, D3D12_TEXTURE_COPY_LOCATION, D3D12_TEXTURE_COPY_LOCATION_0,
    D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX, D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D12_VIEWPORT,
    D3D_ROOT_SIGNATURE_VERSION_1,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::color::{self, Shader};
use super::d3d11::{dxgi_format, surface_format};
use super::{check_format, check_size, lock, Frame, State, SurfaceError, SurfaceFormat};

/// The renderer's D3D12 device and direct queue, retained by `keep_alive`.
pub struct D3D12Lease {
    pub device: *mut c_void,
    pub queue: *mut c_void,
    pub generation: u64,
    pub keep_alive: Arc<dyn std::any::Any + Send + Sync>,
}

pub type D3D12Provider = fn() -> Option<D3D12Lease>;

static PROVIDER: OnceLock<D3D12Provider> = OnceLock::new();

pub(super) fn install(provider: D3D12Provider) {
    let _ = PROVIDER.set(provider);
}

/// Whether the renderer is D3D12: the host installed a provider.
pub(super) fn native() -> bool {
    PROVIDER.get().is_some()
}

const D3D11_RENDERER: &str =
    "this host renders with Direct3D 11 (built without the native-dx12 feature): present a D3D11 \
     texture, or a shared texture synchronised with a keyed mutex or a fence";

/// More in flight than this means the atlas is not keeping up; the present is refused.
const MAX_SLOTS: usize = 4;

struct Gpu {
    device: ID3D12Device,
    renderer_queue: ID3D12CommandQueue,
    queue: ID3D12CommandQueue,
    /// Signalled after each copy: the release fence the producer waits on.
    fence: ID3D12Fence,
    generation: u64,
    submit: Mutex<Submit>,
    pipeline: OnceLock<Result<Pipeline, String>>,
    _lease: Arc<dyn std::any::Any + Send + Sync>,
}

// SAFETY: D3D12 devices, queues and fences are free-threaded; command lists are recorded under
// `submit`'s mutex.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

/// The half-float mapping: a fullscreen triangle reading one texture.
struct Pipeline {
    root: ID3D12RootSignature,
    state: ID3D12PipelineState,
}

fn bytes(blob: &windows::Win32::Graphics::Direct3D::ID3DBlob) -> &[u8] {
    // SAFETY: a blob's pointer covers its size for its life.
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
}

fn build_pipeline(device: &ID3D12Device) -> Result<Pipeline, String> {
    let vs = color::bytecode(Shader::Vertex5)?;
    let ps = color::bytecode(Shader::Pixel5)?;
    let range = D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
        NumDescriptors: 1,
        ..Default::default()
    };
    let parameter = D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        Anonymous: D3D12_ROOT_PARAMETER_0 {
            DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                NumDescriptorRanges: 1,
                pDescriptorRanges: &range,
            },
        },
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
    };
    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 1,
        pParameters: &parameter,
        ..Default::default()
    };
    let (mut serialized, mut diagnostic) = (None, None);
    unsafe {
        D3D12SerializeRootSignature(
            &desc,
            D3D_ROOT_SIGNATURE_VERSION_1,
            &mut serialized,
            Some(&mut diagnostic),
        )
    }
    .map_err(|e| {
        let why = diagnostic.as_ref().map(bytes).unwrap_or_default();
        format!(
            "serializing the root signature: {e} {}",
            String::from_utf8_lossy(why)
        )
    })?;
    let serialized = serialized.ok_or("the root signature serialized to nothing")?;
    let root: ID3D12RootSignature = unsafe { device.CreateRootSignature(0, bytes(&serialized)) }
        .map_err(|e| format!("CreateRootSignature: {e}"))?;
    let shader = |code: &[u8]| D3D12_SHADER_BYTECODE {
        pShaderBytecode: code.as_ptr().cast(),
        BytecodeLength: code.len(),
    };
    let target = D3D12_RENDER_TARGET_BLEND_DESC {
        SrcBlend: D3D12_BLEND_ONE,
        DestBlend: D3D12_BLEND_ZERO,
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: D3D12_BLEND_ONE,
        DestBlendAlpha: D3D12_BLEND_ZERO,
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        ..Default::default()
    };
    let mut formats = [DXGI_FORMAT_UNKNOWN; 8];
    formats[0] = DXGI_FORMAT_B8G8R8A8_UNORM;
    let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root.clone())),
        VS: shader(vs),
        PS: shader(ps),
        BlendState: D3D12_BLEND_DESC {
            RenderTarget: [target; 8],
            ..Default::default()
        },
        SampleMask: u32::MAX,
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        },
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        RTVFormats: formats,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        ..Default::default()
    };
    let state = unsafe { device.CreateGraphicsPipelineState(&desc) };
    unsafe { ManuallyDrop::drop(&mut desc.pRootSignature) };
    Ok(Pipeline {
        root,
        state: state.map_err(|e| format!("CreateGraphicsPipelineState: {e}"))?,
    })
}

impl Gpu {
    fn pipeline(&self) -> Result<&Pipeline, SurfaceError> {
        self.pipeline
            .get_or_init(|| build_pipeline(&self.device))
            .as_ref()
            .map_err(|e| SurfaceError::Platform(e.clone()))
    }

    /// A closed command list to reset, once the work that last used it is done.
    fn commands(
        &self,
        submit: &mut Submit,
    ) -> Result<(ID3D12CommandAllocator, ID3D12GraphicsCommandList), SurfaceError> {
        let completed = unsafe { self.fence.GetCompletedValue() };
        while submit.pending.front().is_some_and(|p| p.value <= completed) {
            let done = submit.pending.pop_front().expect("checked");
            submit.free.push(done.commands);
        }
        let (allocator, commands) = match submit.free.pop() {
            Some(pair) => pair,
            None => {
                let allocator: ID3D12CommandAllocator = unsafe {
                    self.device
                        .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                }
                .map_err(|e| platform("CreateCommandAllocator", e))?;
                let commands: ID3D12GraphicsCommandList = unsafe {
                    self.device.CreateCommandList(
                        0,
                        D3D12_COMMAND_LIST_TYPE_DIRECT,
                        &allocator,
                        None,
                    )
                }
                .map_err(|e| platform("CreateCommandList", e))?;
                unsafe { commands.Close() }.map_err(|e| platform("Close", e))?;
                (allocator, commands)
            }
        };
        unsafe {
            allocator
                .Reset()
                .map_err(|e| platform("resetting a command allocator", e))?;
            commands
                .Reset(&allocator, None)
                .map_err(|e| platform("resetting a command list", e))?;
        }
        Ok((allocator, commands))
    }
}

#[derive(Default)]
struct Submit {
    next: u64,
    free: Vec<(ID3D12CommandAllocator, ID3D12GraphicsCommandList)>,
    pending: VecDeque<Pending>,
}

struct Pending {
    value: u64,
    commands: (ID3D12CommandAllocator, ID3D12GraphicsCommandList),
    _held: Vec<windows::core::IUnknown>,
    _slot: Arc<SlotTexture>,
}

static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);

fn platform(what: &str, e: windows::core::Error) -> SurfaceError {
    SurfaceError::Platform(format!("{what}: {e}"))
}

fn gpu() -> Result<Arc<Gpu>, SurfaceError> {
    let Some(provider) = PROVIDER.get() else {
        // The host installs the provider on its first frame.
        return Err(match crate::video_device::renderer_device() {
            Ok(Some(_)) => SurfaceError::NotSupported(D3D11_RENDERER),
            _ => SurfaceError::NoDevice("no window has drawn yet".into()),
        });
    };
    let lease =
        provider().ok_or_else(|| SurfaceError::NoDevice("no window has drawn yet".into()))?;
    let mut cached = lock(&GPU);
    if let Some(gpu) = cached.as_ref().filter(|g| g.generation == lease.generation) {
        return Ok(gpu.clone());
    }
    let borrowed = |raw: &*mut c_void| {
        // SAFETY: the lease keeps both alive.
        unsafe { windows::core::IUnknown::from_raw_borrowed(raw) }
            .cloned()
            .ok_or_else(|| SurfaceError::NoDevice("the renderer returned a null device".into()))
    };
    let device: ID3D12Device = borrowed(&lease.device)?
        .cast()
        .map_err(|e| platform("ID3D12Device", e))?;
    let renderer_queue: ID3D12CommandQueue = borrowed(&lease.queue)?
        .cast()
        .map_err(|e| platform("ID3D12CommandQueue", e))?;
    let queue: ID3D12CommandQueue = unsafe {
        device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        })
    }
    .map_err(|e| platform("creating the surface queue", e))?;
    let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_SHARED) }
        .map_err(|e| platform("creating the release fence", e))?;
    let gpu = Arc::new(Gpu {
        device,
        renderer_queue,
        queue,
        fence,
        generation: lease.generation,
        submit: Mutex::new(Submit::default()),
        pipeline: OnceLock::new(),
        _lease: lease.keep_alive,
    });
    *cached = Some(gpu.clone());
    Ok(gpu)
}

pub(super) fn forget_device() {
    lock(&GPU).take();
}

pub(crate) struct Device(Arc<Gpu>);

impl Device {
    pub(crate) fn as_raw(&self) -> *mut c_void {
        self.0.device.as_raw()
    }

    pub(crate) fn queue(&self) -> *mut c_void {
        self.0.renderer_queue.as_raw()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.0.generation
    }
}

pub(super) fn device(state: &Mutex<State>) -> Result<Device, SurfaceError> {
    let gpu = gpu()?;
    lock(state).gpu.handed = gpu.generation;
    Ok(Device(gpu))
}

pub(crate) struct Fence(Arc<Gpu>);

impl Fence {
    pub(crate) fn as_raw(&self) -> *mut c_void {
        self.0.fence.as_raw()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.0.generation
    }

    pub(crate) fn completed(&self) -> u64 {
        unsafe { self.0.fence.GetCompletedValue() }
    }

    pub(crate) fn wait(&self, value: u64, timeout: Duration) -> bool {
        if self.completed() >= value {
            return true;
        }
        let Ok(event) = (unsafe { CreateEventW(None, false, false, None) }) else {
            return false;
        };
        let reached = unsafe { self.0.fence.SetEventOnCompletion(value, event) }.is_ok()
            && unsafe {
                WaitForSingleObject(
                    event,
                    timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32,
                )
            } == WAIT_OBJECT_0;
        unsafe {
            let _ = CloseHandle(event);
        }
        reached || self.completed() >= value
    }
}

pub(super) fn release_fence() -> Result<Fence, SurfaceError> {
    gpu().map(Fence)
}

pub(crate) struct SlotTexture {
    texture: ID3D12Resource,
    mapped: Option<Mapped>,
}

/// A half-float slot's 8-bit picture, the descriptors that draw it, and the release-fence value
/// its last draw signals (0 until it is drawn for the frame in the slot).
struct Mapped {
    texture: ID3D12Resource,
    targets: ID3D12DescriptorHeap,
    sources: ID3D12DescriptorHeap,
    drawn: Mutex<u64>,
}

// SAFETY: D3D12 resources and descriptor heaps are free-threaded; `drawn` is a mutex.
unsafe impl Send for SlotTexture {}
unsafe impl Sync for SlotTexture {}

impl SlotTexture {
    fn new(gpu: &Gpu, size: (u32, u32), format: SurfaceFormat) -> Result<Self, SurfaceError> {
        let texture = create_texture(gpu, size, dxgi_format(format))?;
        let mapped = match format {
            SurfaceFormat::Bgra8 => None,
            SurfaceFormat::Rgba16Float => {
                gpu.pipeline()?;
                Some(Mapped::new(gpu, &texture, size)?)
            }
        };
        Ok(Self { texture, mapped })
    }

    fn drawn(&self) -> Option<MutexGuard<'_, u64>> {
        self.mapped.as_ref().map(|m| lock(&m.drawn))
    }
}

impl Mapped {
    fn new(gpu: &Gpu, frame: &ID3D12Resource, size: (u32, u32)) -> Result<Self, SurfaceError> {
        let texture = create_texture(gpu, size, DXGI_FORMAT_B8G8R8A8_UNORM)?;
        let heap = |kind, flags| -> Result<ID3D12DescriptorHeap, SurfaceError> {
            unsafe {
                gpu.device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: kind,
                        NumDescriptors: 1,
                        Flags: flags,
                        NodeMask: 0,
                    })
            }
            .map_err(|e| platform("CreateDescriptorHeap", e))
        };
        let targets = heap(D3D12_DESCRIPTOR_HEAP_TYPE_RTV, Default::default())?;
        let sources = heap(
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
        )?;
        unsafe {
            gpu.device.CreateRenderTargetView(
                &texture,
                None,
                targets.GetCPUDescriptorHandleForHeapStart(),
            );
            gpu.device.CreateShaderResourceView(
                frame,
                None,
                sources.GetCPUDescriptorHandleForHeapStart(),
            );
        }
        Ok(Self {
            texture,
            targets,
            sources,
            drawn: Mutex::new(0),
        })
    }
}

struct Slot {
    texture: Arc<SlotTexture>,
    size: (u32, u32),
    format: SurfaceFormat,
    generation: u64,
    written: u64,
}

#[derive(Default)]
pub(crate) struct SurfaceGpu {
    slots: Vec<Slot>,
    current: Option<usize>,
    fence: Option<Arc<Gpu>>,
}

// SAFETY: see `Gpu`.
unsafe impl Send for SurfaceGpu {}

/// A frame for the atlas: the texture, the fence and the value its copy (or its mapping, once
/// [`Ready::for_atlas`] drew it) signals.
pub(crate) struct Ready {
    texture: Arc<SlotTexture>,
    fence: Arc<Gpu>,
    value: u64,
    size: (u32, u32),
    mapped: bool,
}

impl Ready {
    /// Whether the copy into the texture has run; until then the atlas must not read it.
    pub(crate) fn done(&self) -> bool {
        unsafe { self.fence.fence.GetCompletedValue() >= self.value }
    }

    /// What the atlas copies: the texture, or for a half-float frame its 8-bit mapping, drawn on
    /// the surface's queue the first time it is asked for. Call once [`Self::done`].
    pub(crate) fn for_atlas(self) -> Result<Ready, SurfaceError> {
        let Some(mut drawn) = self.texture.drawn() else {
            return Ok(self);
        };
        if *drawn == 0 {
            *drawn = draw_mapped(&self.fence, &self.texture, self.size)?;
        }
        let value = *drawn;
        drop(drawn);
        Ok(Ready {
            value,
            mapped: true,
            ..self
        })
    }

    pub(crate) fn handle(&self) -> gpui::D3D12TextureHandle {
        let resource = match (&self.texture.mapped, self.mapped) {
            (Some(mapped), true) => &mapped.texture,
            _ => &self.texture.texture,
        };
        gpui::D3D12TextureHandle {
            resource: resource.as_raw(),
            ready_fence: self.fence.fence.as_raw(),
            ready_value: self.value,
        }
    }
}

impl SurfaceGpu {
    pub(crate) fn ready(&self) -> Option<Ready> {
        let slot = &self.slots[self.current?];
        Some(Ready {
            texture: slot.texture.clone(),
            fence: self.fence.clone()?,
            value: slot.written,
            size: slot.size,
            mapped: false,
        })
    }

    pub(crate) fn forget(&mut self) {
        self.slots.clear();
        self.current = None;
        self.fence = None;
    }

    /// A slot neither shown as the frame nor still read by the atlas (whose copy holds its `Arc`).
    fn free_slot(
        &mut self,
        gpu: &Gpu,
        size: (u32, u32),
        format: SurfaceFormat,
    ) -> Result<usize, SurfaceError> {
        if self.slots.iter().any(|s| s.generation != gpu.generation) {
            self.forget();
        }
        let free = |(i, s): &(usize, &Slot)| {
            Some(*i) != self.current && Arc::strong_count(&s.texture) == 1
        };
        let fits = |s: &Slot| s.size == size && s.format == format;
        let pick = self
            .slots
            .iter()
            .enumerate()
            .filter(free)
            .min_by_key(|(_, s)| !fits(s))
            .map(|(i, _)| i);
        let slot = || -> Result<Slot, SurfaceError> {
            Ok(Slot {
                texture: Arc::new(SlotTexture::new(gpu, size, format)?),
                size,
                format,
                generation: gpu.generation,
                written: 0,
            })
        };
        match pick {
            Some(i) if fits(&self.slots[i]) => Ok(i),
            Some(i) => {
                self.slots[i] = slot()?;
                Ok(i)
            }
            None if self.slots.len() < MAX_SLOTS => {
                self.slots.push(slot()?);
                Ok(self.slots.len() - 1)
            }
            None => Err(SurfaceError::Busy),
        }
    }
}

fn create_texture(
    gpu: &Gpu,
    (width, height): (u32, u32),
    format: DXGI_FORMAT,
) -> Result<ID3D12Resource, SurfaceError> {
    let heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        ..Default::default()
    };
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: u64::from(width),
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
        ..Default::default()
    };
    let mut texture: Option<ID3D12Resource> = None;
    unsafe {
        gpu.device.CreateCommittedResource(
            &heap,
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            None,
            &mut texture,
        )
    }
    .map_err(|e| platform("creating the surface texture", e))?;
    texture.ok_or_else(|| SurfaceError::Platform("CreateCommittedResource returned nothing".into()))
}

fn describe(resource: &ID3D12Resource) -> Result<((u32, u32), SurfaceFormat), SurfaceError> {
    let desc = unsafe { resource.GetDesc() };
    let format = surface_format(desc.Format)
        .filter(|_| desc.Dimension == D3D12_RESOURCE_DIMENSION_TEXTURE2D)
        .ok_or_else(|| {
            SurfaceError::Format(format!(
                "a resource in DXGI format {}: a surface takes a 2D B8G8R8A8_UNORM texture (or \
                 its _UNORM_SRGB and _TYPELESS forms), or R16G16B16A16_FLOAT on a surface set \
                 to Rgba16Float",
                desc.Format.0
            ))
        })?;
    if desc.SampleDesc.Count != 1 {
        return Err(SurfaceError::Format(
            "a multisampled resource: resolve it to a single-sampled texture first".into(),
        ));
    }
    let width = u32::try_from(desc.Width).unwrap_or(u32::MAX);
    check_size(width, desc.Height)?;
    Ok(((width, desc.Height), format))
}

/// Draws a half-float slot into its 8-bit texture on the surface's queue, after the copy into it.
/// Returns the release-fence value the draw signals.
fn draw_mapped(
    gpu: &Arc<Gpu>,
    slot: &Arc<SlotTexture>,
    size: (u32, u32),
) -> Result<u64, SurfaceError> {
    let mapped = slot.mapped.as_ref().expect("a half-float slot");
    let pipeline = gpu.pipeline()?;
    let mut submit = lock(&gpu.submit);
    let (allocator, commands) = gpu.commands(&mut submit)?;
    transition(
        &commands,
        &slot.texture,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
    );
    unsafe {
        commands.SetGraphicsRootSignature(&pipeline.root);
        commands.SetPipelineState(&pipeline.state);
        commands.SetDescriptorHeaps(&[Some(mapped.sources.clone())]);
        commands
            .SetGraphicsRootDescriptorTable(0, mapped.sources.GetGPUDescriptorHandleForHeapStart());
        commands.RSSetViewports(&[D3D12_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size.0 as f32,
            Height: size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]);
        commands.RSSetScissorRects(&[RECT {
            left: 0,
            top: 0,
            right: size.0 as i32,
            bottom: size.1 as i32,
        }]);
        let target = mapped.targets.GetCPUDescriptorHandleForHeapStart();
        commands.OMSetRenderTargets(1, Some(&target), false, None);
        commands.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        commands.DrawInstanced(3, 1, 0, 0);
    }
    transition(
        &commands,
        &slot.texture,
        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
    );
    unsafe { commands.Close() }.map_err(|e| platform("closing the mapping", e))?;
    let list: ID3D12CommandList = commands.cast().map_err(|e| platform("cast", e))?;
    let value = submit.next + 1;
    unsafe {
        gpu.queue.ExecuteCommandLists(&[Some(list)]);
        gpu.queue
            .Signal(&gpu.fence, value)
            .map_err(|e| platform("signalling the release fence", e))?;
    }
    submit.next = value;
    submit.pending.push_back(Pending {
        value,
        commands: (allocator, commands),
        _held: Vec::new(),
        _slot: slot.clone(),
    });
    Ok(value)
}

fn transition(
    commands: &ID3D12GraphicsCommandList,
    resource: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    if before == after {
        return;
    }
    let mut barrier = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: ManuallyDrop::new(Some(resource.clone())),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    };
    unsafe {
        commands.ResourceBarrier(std::slice::from_ref(&barrier));
        ManuallyDrop::drop(&mut (*barrier.Anonymous.Transition).pResource);
    }
}

fn location(resource: &ID3D12Resource) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(resource.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            SubresourceIndex: 0,
        },
    }
}

/// Copies `source` (in `state`, left in it) into a free surface texture after `wait`, signals the
/// release fence (and `signal`), and makes the texture the frame. Returns the release value.
fn copy_in(
    gpu: &Arc<Gpu>,
    state: &mut State,
    source: &ID3D12Resource,
    source_state: D3D12_RESOURCE_STATES,
    wait: Option<(&ID3D12Fence, u64)>,
    signal: Option<(&ID3D12Fence, u64)>,
) -> Result<(u64, u64), SurfaceError> {
    let (size, format) = describe(source)?;
    check_format(state.format, format)?;
    let index = state.gpu12.free_slot(gpu, size, format)?;
    let target = state.gpu12.slots[index].texture.clone();
    if let Some(mut drawn) = target.drawn() {
        *drawn = 0;
    }
    let mut submit = lock(&gpu.submit);
    let (allocator, commands) = gpu.commands(&mut submit)?;
    transition(
        &commands,
        source,
        source_state,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
    );
    transition(
        &commands,
        &target.texture,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
        D3D12_RESOURCE_STATE_COPY_DEST,
    );
    let mut to = location(&target.texture);
    let mut from = location(source);
    let region = D3D12_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: size.0,
        bottom: size.1,
        back: 1,
    };
    unsafe {
        commands.CopyTextureRegion(&to, 0, 0, 0, &from, Some(&region));
        ManuallyDrop::drop(&mut to.pResource);
        ManuallyDrop::drop(&mut from.pResource);
    }
    transition(
        &commands,
        source,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
        source_state,
    );
    transition(
        &commands,
        &target.texture,
        D3D12_RESOURCE_STATE_COPY_DEST,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
    );
    unsafe { commands.Close() }.map_err(|e| platform("closing the copy", e))?;
    let list: ID3D12CommandList = commands.cast().map_err(|e| platform("cast", e))?;
    let value = submit.next + 1;
    unsafe {
        if let Some((fence, at)) = wait {
            gpu.queue
                .Wait(fence, at)
                .map_err(|e| platform("waiting on the producer's fence", e))?;
        }
        gpu.queue.ExecuteCommandLists(&[Some(list)]);
        gpu.queue
            .Signal(&gpu.fence, value)
            .map_err(|e| platform("signalling the release fence", e))?;
        if let Some((fence, at)) = signal {
            gpu.queue
                .Signal(fence, at)
                .map_err(|e| platform("signalling the producer's fence", e))?;
        }
    }
    submit.next = value;
    let mut held: Vec<windows::core::IUnknown> =
        vec![source.cast().map_err(|e| platform("cast", e))?];
    for (fence, _) in wait.into_iter().chain(signal) {
        held.push(fence.cast().map_err(|e| platform("cast", e))?);
    }
    submit.pending.push_back(Pending {
        value,
        commands: (allocator, commands),
        _held: held,
        _slot: target,
    });
    drop(submit);
    state.gpu12.slots[index].written = value;
    state.gpu12.current = Some(index);
    state.gpu12.fence = Some(gpu.clone());
    let frame = state.put(Frame::D3D12 {
        width: size.0,
        height: size.1,
    });
    Ok((frame, value))
}

/// # Safety
/// `raw` is null or a live `ID3D12Resource`; `ready` is a live `ID3D12Fence` and a value.
pub(super) unsafe fn present(
    state: &std::sync::Mutex<State>,
    raw: *mut c_void,
    resource_state: i32,
    ready: Option<(*mut c_void, u64)>,
) -> Result<(u64, u64), SurfaceError> {
    // SAFETY: the caller's.
    let source = unsafe { ID3D12Resource::from_raw_borrowed(&raw) }
        .ok_or_else(|| SurfaceError::Size("a null resource".into()))?;
    let fence_raw = ready.map(|(raw, _)| raw);
    let fence = match (&fence_raw, ready) {
        (Some(raw), Some((_, value))) => Some((
            // SAFETY: the caller's.
            unsafe { ID3D12Fence::from_raw_borrowed(raw) }
                .ok_or_else(|| SurfaceError::Size("a null fence".into()))?,
            value,
        )),
        _ => None,
    };
    let gpu = gpu()?;
    let mut owner: Option<ID3D12Device> = None;
    unsafe { source.GetDevice(&mut owner) }.map_err(|e| platform("GetDevice", e))?;
    let same = owner
        .and_then(|o| o.cast::<windows::core::IUnknown>().ok())
        .zip(gpu.device.cast::<windows::core::IUnknown>().ok())
        .is_some_and(|(a, b)| a == b);
    let mut state = lock(state);
    if !same {
        let handed = state.gpu.handed;
        return Err(if handed != 0 && handed != gpu.generation {
            SurfaceError::DeviceChanged
        } else {
            SurfaceError::WrongDevice
        });
    }
    copy_in(
        &gpu,
        &mut state,
        source,
        D3D12_RESOURCE_STATES(resource_state),
        fence,
        None,
    )
}

pub(crate) struct Opened {
    resource: ID3D12Resource,
    fence: Option<ID3D12Fence>,
    generation: u64,
    pub(super) size: (u32, u32),
    pub(super) format: SurfaceFormat,
}

// SAFETY: see `Gpu`.
unsafe impl Send for Opened {}

pub(super) fn open(handle: HANDLE) -> Result<Opened, SurfaceError> {
    let gpu = gpu()?;
    let mut resource: Option<ID3D12Resource> = None;
    unsafe { gpu.device.OpenSharedHandle(handle, &mut resource) }
        .map_err(|e| platform("opening the shared texture on D3D12", e))?;
    let resource = resource
        .ok_or_else(|| SurfaceError::Platform("OpenSharedHandle returned nothing".into()))?;
    let (size, format) = describe(&resource)?;
    Ok(Opened {
        resource,
        fence: None,
        generation: gpu.generation,
        size,
        format,
    })
}

impl Opened {
    pub(super) fn open_fence(&mut self, handle: HANDLE) -> Result<(), SurfaceError> {
        let gpu = gpu()?;
        let mut fence: Option<ID3D12Fence> = None;
        unsafe { gpu.device.OpenSharedHandle(handle, &mut fence) }
            .map_err(|e| platform("opening the shared fence", e))?;
        self.fence = fence;
        Ok(())
    }
}

/// Shared resources cross devices in the common state.
pub(super) fn present_shared(
    state: &Mutex<State>,
    opened: &Opened,
    values: Option<(u64, u64)>,
) -> Result<u64, SurfaceError> {
    let gpu = gpu()?;
    if gpu.generation != opened.generation {
        return Err(SurfaceError::DeviceChanged);
    }
    let (wait, signal) = match (values, opened.fence.as_ref()) {
        (Some((wait, release)), Some(fence)) => (Some((fence, wait)), Some((fence, release))),
        (Some(_), None) => {
            return Err(SurfaceError::Format(
                "fence values were given for a shared texture opened without a fence".into(),
            ))
        }
        (None, _) => (None, None),
    };
    let mut state = lock(state);
    copy_in(
        &gpu,
        &mut state,
        &opened.resource,
        D3D12_RESOURCE_STATE_COMMON,
        wait,
        signal,
    )
    .map(|(frame, _)| frame)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12CreateDevice, D3D12_CPU_PAGE_PROPERTY_WRITE_BACK, D3D12_FENCE_FLAG_NONE,
        D3D12_HEAP_TYPE_CUSTOM, D3D12_HEAP_TYPE_READBACK, D3D12_MEMORY_POOL_L0,
        D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_RESOURCE_DIMENSION_BUFFER,
        D3D12_RESOURCE_FLAG_NONE, D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4};

    use super::*;
    use crate::surface::color::tests::{f32_to_half, ramp};
    use crate::surface::{install_d3d12_provider, surface};

    struct Renderer {
        device: ID3D12Device,
        queue: ID3D12CommandQueue,
    }

    // SAFETY: free-threaded D3D12 objects.
    unsafe impl Send for Renderer {}
    unsafe impl Sync for Renderer {}

    static RENDERER: OnceLock<Arc<Renderer>> = OnceLock::new();

    /// A WARP device and queue installed as the renderer's, the way the host installs its own.
    fn renderer() -> Arc<Renderer> {
        RENDERER
            .get_or_init(|| {
                let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.unwrap();
                let adapter: IDXGIAdapter = unsafe { factory.EnumWarpAdapter() }.unwrap();
                let mut device: Option<ID3D12Device> = None;
                unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) }
                    .unwrap();
                let device = device.unwrap();
                let queue = unsafe {
                    device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                        Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                        ..Default::default()
                    })
                }
                .unwrap();
                install_d3d12_provider(|| {
                    let r = RENDERER.get()?.clone();
                    Some(D3D12Lease {
                        device: r.device.as_raw(),
                        queue: r.queue.as_raw(),
                        generation: 1,
                        keep_alive: r,
                    })
                });
                Arc::new(Renderer { device, queue })
            })
            .clone()
    }

    /// A texture the CPU writes, as a producer's finished frame.
    fn texture(r: &Renderer, format: DXGI_FORMAT, width: u32, texels: &[u8]) -> ID3D12Resource {
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_CUSTOM,
            CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
            MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
            ..Default::default()
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: u64::from(width),
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            ..Default::default()
        };
        let mut resource: Option<ID3D12Resource> = None;
        unsafe {
            r.device.CreateCommittedResource(
                &heap,
                D3D12_HEAP_FLAG_NONE,
                &desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut resource,
            )
        }
        .unwrap();
        let resource = resource.unwrap();
        unsafe {
            resource.Map(0, None, None).unwrap();
            resource
                .WriteToSubresource(
                    0,
                    None,
                    texels.as_ptr().cast(),
                    texels.len() as u32,
                    texels.len() as u32,
                )
                .unwrap();
            resource.Unmap(0, None);
        }
        resource
    }

    fn halves(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Copies what the atlas would take for `ready` into a readback buffer, after its fence.
    fn shown(r: &Renderer, ready: &Ready) -> Vec<[u8; 4]> {
        let handle = ready.handle();
        let source = unsafe { ID3D12Resource::from_raw_borrowed(&handle.resource) }.unwrap();
        let fence = unsafe { ID3D12Fence::from_raw_borrowed(&handle.ready_fence) }.unwrap();
        let desc = unsafe { source.GetDesc() };
        assert_eq!(desc.Format, DXGI_FORMAT_B8G8R8A8_UNORM);
        let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut total = 0u64;
        unsafe {
            r.device.GetCopyableFootprints(
                &desc,
                0,
                1,
                0,
                Some(&mut footprint),
                None,
                None,
                Some(&mut total),
            )
        };
        let buffer_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: total,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_NONE,
            ..Default::default()
        };
        let mut buffer: Option<ID3D12Resource> = None;
        unsafe {
            r.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_READBACK,
                    ..Default::default()
                },
                D3D12_HEAP_FLAG_NONE,
                &buffer_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
                &mut buffer,
            )
        }
        .unwrap();
        let buffer = buffer.unwrap();
        let allocator: ID3D12CommandAllocator = unsafe {
            r.device
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
        }
        .unwrap();
        let commands: ID3D12GraphicsCommandList = unsafe {
            r.device
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
        }
        .unwrap();
        transition(
            &commands,
            source,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        let mut to = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(buffer.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: footprint,
            },
        };
        let mut from = location(source);
        unsafe {
            commands.CopyTextureRegion(&to, 0, 0, 0, &from, None);
            ManuallyDrop::drop(&mut to.pResource);
            ManuallyDrop::drop(&mut from.pResource);
        }
        transition(
            &commands,
            source,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        unsafe { commands.Close() }.unwrap();
        let done: ID3D12Fence = unsafe { r.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.unwrap();
        unsafe {
            r.queue.Wait(fence, handle.ready_value).unwrap();
            r.queue
                .ExecuteCommandLists(&[Some(commands.cast::<ID3D12CommandList>().unwrap())]);
            r.queue.Signal(&done, 1).unwrap();
        }
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while unsafe { done.GetCompletedValue() } < 1 {
            assert!(std::time::Instant::now() < until, "the readback never ran");
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut data = std::ptr::null_mut();
        unsafe { buffer.Map(0, None, Some(&mut data)) }.unwrap();
        let row = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), desc.Width as usize * 4) };
        let pixels = row
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        unsafe { buffer.Unmap(0, None) };
        pixels
    }

    #[test]
    fn a_half_float_resource_shows_every_srgb_code_unchanged_and_clamps_above_one() {
        let r = renderer();
        let s = surface("tests:d3d12-rgba16f");
        s.set_format(SurfaceFormat::Rgba16Float);
        let h = f32_to_half;
        let mut texels = ramp();
        texels.extend([h(8.0), h(1.5), h(-0.25), h(2.0)]);
        let source = texture(&r, DXGI_FORMAT_R16G16B16A16_FLOAT, 257, &halves(&texels));
        unsafe { s.present_d3d12(source.as_raw(), D3D12_RESOURCE_STATE_COMMON.0, None) }.unwrap();
        let ready = s.lock_state().gpu12.ready().unwrap();
        let ready = ready.for_atlas().unwrap();
        let pixels = shown(&r, &ready);
        for (code, px) in pixels[..256].iter().enumerate() {
            assert_eq!(px, &[code as u8; 4], "code {code}");
        }
        assert_eq!(pixels[256], [0, 255, 255, 255]);
        let again = s.lock_state().gpu12.ready().unwrap().for_atlas().unwrap();
        assert_eq!(again.value, ready.value, "a frame is mapped once");
    }

    #[test]
    fn a_resource_of_the_other_format_is_refused() {
        let r = renderer();
        let half = surface("tests:d3d12-mismatch-half");
        half.set_format(SurfaceFormat::Rgba16Float);
        let bgra = texture(&r, DXGI_FORMAT_B8G8R8A8_UNORM, 1, &[1, 2, 3, 4]);
        let state = D3D12_RESOURCE_STATE_COMMON.0;
        assert_eq!(
            unsafe { half.present_d3d12(bgra.as_raw(), state, None) },
            Err(SurfaceError::FormatMismatch {
                surface: SurfaceFormat::Rgba16Float,
                frame: SurfaceFormat::Bgra8
            })
        );
        let float = texture(&r, DXGI_FORMAT_R16G16B16A16_FLOAT, 1, &[0; 8]);
        let plain = surface("tests:d3d12-mismatch-bgra");
        assert!(matches!(
            unsafe { plain.present_d3d12(float.as_raw(), state, None) },
            Err(SurfaceError::FormatMismatch { .. })
        ));
        assert_eq!(half.stats().presented + plain.stats().presented, 0);
    }
}
