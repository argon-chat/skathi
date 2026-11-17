//! The native D3D12 half of a surface, under the D3D12 renderer: frames are copied on a queue of
//! the surface's own on the renderer's device, after a GPU wait on the producer's fence, into one
//! of a few surface textures. The element's tick hands a finished one to the atlas
//! (`write_image_tile_d3d12`), so the renderer's queue never waits on a producer.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue, ID3D12Device, ID3D12Fence,
    ID3D12GraphicsCommandList, ID3D12Resource, D3D12_BOX, D3D12_COMMAND_LIST_TYPE_DIRECT,
    D3D12_COMMAND_QUEUE_DESC, D3D12_FENCE_FLAG_SHARED, D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES,
    D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0,
    D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE,
    D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    D3D12_RESOURCE_STATES, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST,
    D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET,
    D3D12_RESOURCE_TRANSITION_BARRIER, D3D12_TEXTURE_COPY_LOCATION, D3D12_TEXTURE_COPY_LOCATION_0,
    D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX, D3D12_TEXTURE_LAYOUT_UNKNOWN,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::{check_size, lock, Frame, State, SurfaceError};

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
    _lease: Arc<dyn std::any::Any + Send + Sync>,
}

// SAFETY: D3D12 devices, queues and fences are free-threaded; command lists are recorded under
// `submit`'s mutex.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

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
        return Err(match sk_video::mf::renderer_device() {
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

pub(crate) struct SlotTexture(ID3D12Resource);

// SAFETY: a D3D12 resource is free-threaded.
unsafe impl Send for SlotTexture {}
unsafe impl Sync for SlotTexture {}

struct Slot {
    texture: Arc<SlotTexture>,
    size: (u32, u32),
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

/// A frame for the atlas: the texture, the fence and the value its copy signals.
pub(crate) struct Ready {
    texture: Arc<SlotTexture>,
    fence: Arc<Gpu>,
    value: u64,
}

impl Ready {
    /// Whether the copy into the texture has run; until then the atlas must not read it.
    pub(crate) fn done(&self) -> bool {
        unsafe { self.fence.fence.GetCompletedValue() >= self.value }
    }

    pub(crate) fn handle(&self) -> gpui::D3D12TextureHandle {
        gpui::D3D12TextureHandle {
            resource: self.texture.0.as_raw(),
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
        })
    }

    pub(crate) fn forget(&mut self) {
        self.slots.clear();
        self.current = None;
        self.fence = None;
    }

    /// A slot neither shown as the frame nor still read by the atlas (whose copy holds its `Arc`).
    fn free_slot(&mut self, gpu: &Gpu, size: (u32, u32)) -> Result<usize, SurfaceError> {
        if self.slots.iter().any(|s| s.generation != gpu.generation) {
            self.forget();
        }
        let free = |(i, s): &(usize, &Slot)| {
            Some(*i) != self.current && Arc::strong_count(&s.texture) == 1
        };
        let pick = self
            .slots
            .iter()
            .enumerate()
            .filter(free)
            .min_by_key(|(_, s)| s.size != size)
            .map(|(i, _)| i);
        let index = match pick {
            Some(i) if self.slots[i].size == size => return Ok(i),
            Some(i) => i,
            None if self.slots.len() < MAX_SLOTS => {
                self.slots.push(Slot {
                    texture: Arc::new(SlotTexture(create_texture(gpu, size)?)),
                    size,
                    generation: gpu.generation,
                    written: 0,
                });
                return Ok(self.slots.len() - 1);
            }
            None => return Err(SurfaceError::Busy),
        };
        self.slots[index] = Slot {
            texture: Arc::new(SlotTexture(create_texture(gpu, size)?)),
            size,
            generation: gpu.generation,
            written: 0,
        };
        Ok(index)
    }
}

fn create_texture(gpu: &Gpu, (width, height): (u32, u32)) -> Result<ID3D12Resource, SurfaceError> {
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
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
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

fn describe(resource: &ID3D12Resource) -> Result<(u32, u32), SurfaceError> {
    let desc = unsafe { resource.GetDesc() };
    if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
        || ![
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            DXGI_FORMAT_B8G8R8A8_TYPELESS,
        ]
        .contains(&desc.Format)
    {
        return Err(SurfaceError::Format(format!(
            "a resource in DXGI format {}: a surface takes a 2D B8G8R8A8_UNORM texture (or its \
             _UNORM_SRGB and _TYPELESS forms)",
            desc.Format.0
        )));
    }
    if desc.SampleDesc.Count != 1 {
        return Err(SurfaceError::Format(
            "a multisampled resource: resolve it to a single-sampled texture first".into(),
        ));
    }
    let width = u32::try_from(desc.Width).unwrap_or(u32::MAX);
    check_size(width, desc.Height)?;
    Ok((width, desc.Height))
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
) -> Result<u64, SurfaceError> {
    let size = describe(source)?;
    let mut submit = lock(&gpu.submit);
    let completed = unsafe { gpu.fence.GetCompletedValue() };
    while submit.pending.front().is_some_and(|p| p.value <= completed) {
        let done = submit.pending.pop_front().expect("checked");
        submit.free.push(done.commands);
    }
    let index = state.gpu12.free_slot(gpu, size)?;
    let target = state.gpu12.slots[index].texture.clone();
    let (allocator, commands) = match submit.free.pop() {
        Some(pair) => pair,
        None => {
            let allocator: ID3D12CommandAllocator = unsafe {
                gpu.device
                    .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
            }
            .map_err(|e| platform("CreateCommandAllocator", e))?;
            let commands: ID3D12GraphicsCommandList = unsafe {
                gpu.device
                    .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
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
    transition(
        &commands,
        source,
        source_state,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
    );
    transition(
        &commands,
        &target.0,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
        D3D12_RESOURCE_STATE_COPY_DEST,
    );
    let mut to = location(&target.0);
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
        &target.0,
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
    state.put(Frame::D3D12 {
        width: size.0,
        height: size.1,
    });
    Ok(value)
}

/// # Safety
/// `raw` is null or a live `ID3D12Resource`; `ready` is a live `ID3D12Fence` and a value.
pub(super) unsafe fn present(
    state: &std::sync::Mutex<State>,
    raw: *mut c_void,
    resource_state: i32,
    ready: Option<(*mut c_void, u64)>,
) -> Result<u64, SurfaceError> {
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
    let size = describe(&resource)?;
    Ok(Opened {
        resource,
        fence: None,
        generation: gpu.generation,
        size,
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
) -> Result<(), SurfaceError> {
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
    .map(|_| ())
}
