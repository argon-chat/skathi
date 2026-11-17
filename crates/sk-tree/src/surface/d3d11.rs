//! The Windows GPU half of a surface: each surface owns one BGRA8 texture on the renderer's D3D11
//! device (the D3D11On12 bridge under the D3D12 renderer). A present copies the producer's frame
//! into it on the GPU; the element's tick copies it into the atlas (`LiveTile::write_gpu`).

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11Device1, ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4,
    ID3D11Fence, ID3D11Multithread, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIKeyedMutex;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::{check_size, lock, Frame, State, SurfaceError};

const WAIT_TIMEOUT: i32 = 0x102;

struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    multithread: ID3D11Multithread,
    generation: u64,
    _lease: Arc<dyn std::any::Any + Send + Sync>,
}

// SAFETY: D3D11 devices are free-threaded, and the renderer's has multithread protection on, which
// serialises its immediate context; a sequence of context calls is bracketed by `Locked`.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);

fn platform(what: &str, e: windows::core::Error) -> SurfaceError {
    SurfaceError::Platform(format!("{what}: {e}"))
}

fn gpu() -> Result<Arc<Gpu>, SurfaceError> {
    let lease = sk_video::mf::renderer_device()
        .map_err(SurfaceError::NoDevice)?
        .ok_or_else(|| SurfaceError::NoDevice("no window has drawn yet".into()))?;
    let mut cached = lock(&GPU);
    if let Some(gpu) = cached.as_ref().filter(|g| g.generation == lease.generation) {
        return Ok(gpu.clone());
    }
    // SAFETY: the lease keeps the device alive.
    let device = unsafe { ID3D11Device::from_raw_borrowed(&lease.device) }
        .ok_or_else(|| SurfaceError::NoDevice("the renderer returned a null device".into()))?
        .clone();
    let context = unsafe { device.GetImmediateContext() }
        .map_err(|e| platform("ID3D11Device::GetImmediateContext", e))?;
    let multithread: ID3D11Multithread = device
        .cast()
        .map_err(|e| platform("ID3D11Multithread", e))?;
    let gpu = Arc::new(Gpu {
        device,
        context,
        multithread,
        generation: lease.generation,
        _lease: lease.keep_alive,
    });
    *cached = Some(gpu.clone());
    Ok(gpu)
}

pub(super) fn forget_device() {
    lock(&GPU).take();
}

struct Locked<'a>(&'a ID3D11Multithread);

impl<'a> Locked<'a> {
    fn enter(gpu: &'a Gpu) -> Self {
        unsafe { gpu.multithread.Enter() };
        Locked(&gpu.multithread)
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        unsafe { self.0.Leave() };
    }
}

pub(crate) struct Device(Arc<Gpu>);

impl Device {
    pub(crate) fn as_raw(&self) -> *mut c_void {
        self.0.device.as_raw()
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

/// A reference to a surface's texture, for the UI thread's atlas copy.
pub(crate) struct Texture(ID3D11Texture2D);

impl Texture {
    pub(crate) fn as_raw(&self) -> *mut c_void {
        self.0.as_raw()
    }
}

#[derive(Default)]
pub(crate) struct SurfaceGpu {
    slot: Option<Slot>,
    /// The device generation last handed to the producer: a texture from an older one is
    /// [`SurfaceError::DeviceChanged`], not [`SurfaceError::WrongDevice`].
    pub(super) handed: u64,
}

struct Slot {
    texture: ID3D11Texture2D,
    size: (u32, u32),
    generation: u64,
}

// SAFETY: see `Gpu`; the slot is only touched under the surface's state mutex.
unsafe impl Send for SurfaceGpu {}

impl SurfaceGpu {
    pub(crate) fn texture(&self) -> Option<Texture> {
        self.slot.as_ref().map(|s| Texture(s.texture.clone()))
    }

    pub(crate) fn forget(&mut self) {
        self.slot = None;
    }

    fn ensure(&mut self, gpu: &Gpu, size: (u32, u32)) -> Result<&ID3D11Texture2D, SurfaceError> {
        let fits = self
            .slot
            .as_ref()
            .is_some_and(|s| s.size == size && s.generation == gpu.generation);
        if !fits {
            self.slot = None;
            let desc = D3D11_TEXTURE2D_DESC {
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
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture = None;
            unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)) }
                .map_err(|e| platform("creating the surface texture", e))?;
            let texture = texture.ok_or_else(|| {
                SurfaceError::Platform("CreateTexture2D returned no texture".into())
            })?;
            self.slot = Some(Slot {
                texture,
                size,
                generation: gpu.generation,
            });
        }
        Ok(&self.slot.as_ref().expect("just made").texture)
    }
}

fn describe(texture: &ID3D11Texture2D) -> Result<D3D11_TEXTURE2D_DESC, SurfaceError> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    if ![
        DXGI_FORMAT_B8G8R8A8_UNORM,
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        DXGI_FORMAT_B8G8R8A8_TYPELESS,
    ]
    .contains(&desc.Format)
    {
        return Err(SurfaceError::Format(format!(
            "a texture in DXGI format {}: a surface takes B8G8R8A8_UNORM (or its _UNORM_SRGB and \
             _TYPELESS forms); render into a BGRA8 texture",
            desc.Format.0
        )));
    }
    check_size(desc.Width, desc.Height)?;
    Ok(desc)
}

fn same(a: &ID3D11Device, b: &ID3D11Device) -> bool {
    match (a.cast::<IUnknown>(), b.cast::<IUnknown>()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Guard<'a> {
    None,
    Keyed(&'a IDXGIKeyedMutex, u64, u64, u32),
    Fence(&'a ID3D11Fence, u64, u64, u32),
}

fn reached(fence: &ID3D11Fence, value: u64, timeout_ms: u32) -> bool {
    if unsafe { fence.GetCompletedValue() } >= value {
        return true;
    }
    let Ok(event) = (unsafe { CreateEventW(None, false, false, None) }) else {
        return false;
    };
    let waited = unsafe { fence.SetEventOnCompletion(value, event) }.is_ok()
        && unsafe { WaitForSingleObject(event, timeout_ms) } == WAIT_OBJECT_0;
    unsafe {
        let _ = CloseHandle(event);
    }
    waited || unsafe { fence.GetCompletedValue() } >= value
}

/// Copies `source` into the surface's texture and makes it the frame. Runs with `state` locked so
/// two producers cannot interleave; the context calls are serialised with the UI thread's.
fn copy_in(
    gpu: &Gpu,
    state: &mut State,
    source: &ID3D11Texture2D,
    desc: &D3D11_TEXTURE2D_DESC,
    sync: Guard<'_>,
) -> Result<(), SurfaceError> {
    let size = (desc.Width, desc.Height);
    let target = state.gpu.ensure(gpu, size)?.clone();
    let fenced = match sync {
        // The immediate context is the renderer's too: a GPU wait on a fence that is never
        // signalled would stall every window, so the CPU waits first, with the timeout.
        Guard::Fence(fence, wait, _, timeout) if !reached(fence, wait, timeout) => {
            return Err(SurfaceError::Timeout)
        }
        Guard::Fence(fence, wait, release, _) => Some((
            gpu.context
                .cast::<ID3D11DeviceContext4>()
                .map_err(|e| platform("ID3D11DeviceContext4", e))?,
            fence,
            wait,
            release,
        )),
        _ => None,
    };
    if let Guard::Keyed(mutex, acquire, _, timeout) = sync {
        // Raw: `WAIT_TIMEOUT` is a success code the wrapper would turn into `Ok`.
        let hr =
            unsafe { (Interface::vtable(mutex).AcquireSync)(mutex.as_raw(), acquire, timeout) };
        if hr.0 == WAIT_TIMEOUT {
            return Err(SurfaceError::Timeout);
        }
        hr.ok()
            .map_err(|e| platform("IDXGIKeyedMutex::AcquireSync", e))?;
    }
    {
        let _locked = Locked::enter(gpu);
        unsafe {
            if let Some((context, fence, wait, _)) = &fenced {
                context
                    .Wait(*fence, *wait)
                    .map_err(|e| platform("waiting on the producer's fence", e))?;
            }
            if desc.SampleDesc.Count > 1 {
                gpu.context
                    .ResolveSubresource(&target, 0, source, 0, DXGI_FORMAT_B8G8R8A8_UNORM);
            } else {
                gpu.context
                    .CopySubresourceRegion(&target, 0, 0, 0, 0, source, 0, None);
            }
            if let Some((context, fence, _, release)) = &fenced {
                context
                    .Signal(*fence, *release)
                    .map_err(|e| platform("signalling the producer's fence", e))?;
            }
            gpu.context.Flush();
        }
    }
    if let Guard::Keyed(mutex, _, release, _) = sync {
        unsafe { mutex.ReleaseSync(release) }
            .map_err(|e| platform("IDXGIKeyedMutex::ReleaseSync", e))?;
    }
    state.put(Frame::Gpu {
        width: size.0,
        height: size.1,
    });
    Ok(())
}

/// # Safety
/// `raw` is null or a live `ID3D11Texture2D`.
pub(super) unsafe fn present_texture(
    state: &Mutex<State>,
    raw: *mut c_void,
) -> Result<(), SurfaceError> {
    // SAFETY: the caller's.
    let source = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
        .ok_or_else(|| SurfaceError::Size("a null texture".into()))?;
    let desc = describe(source)?;
    let gpu = gpu()?;
    let producer = unsafe { source.GetDevice() }.map_err(|e| platform("GetDevice", e))?;
    let mut state = lock(state);
    if !same(&producer, &gpu.device) {
        let handed = state.gpu.handed;
        return Err(if handed != 0 && handed != gpu.generation {
            SurfaceError::DeviceChanged
        } else {
            SurfaceError::WrongDevice
        });
    }
    copy_in(&gpu, &mut state, source, &desc, Guard::None)
}

pub(crate) struct Opened {
    texture: ID3D11Texture2D,
    desc: D3D11_TEXTURE2D_DESC,
    mutex: Option<IDXGIKeyedMutex>,
    fence: Option<ID3D11Fence>,
    generation: u64,
}

// SAFETY: see `Gpu`.
unsafe impl Send for Opened {}

pub(super) fn open(handle: HANDLE) -> Result<Opened, SurfaceError> {
    let gpu = gpu()?;
    let legacy = || {
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe { gpu.device.OpenSharedResource(handle, &mut texture) }?;
        texture.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER))
    };
    // An NT handle opens only through `OpenSharedResource1`, a legacy one only through the other.
    let texture = match gpu.device.cast::<ID3D11Device1>() {
        Ok(device1) => {
            unsafe { device1.OpenSharedResource1::<ID3D11Texture2D>(handle) }.or_else(|_| legacy())
        }
        Err(_) => legacy(),
    }
    .map_err(|e| platform("opening the shared texture", e))?;
    let desc = describe(&texture)?;
    Ok(Opened {
        mutex: texture.cast().ok(),
        texture,
        desc,
        fence: None,
        generation: gpu.generation,
    })
}

impl Opened {
    pub(super) fn size(&self) -> (u32, u32) {
        (self.desc.Width, self.desc.Height)
    }

    pub(super) fn open_fence(&mut self, handle: HANDLE) -> Result<(), SurfaceError> {
        let gpu = gpu()?;
        let device: ID3D11Device5 = gpu.device.cast().map_err(|_| {
            SurfaceError::NotSupported(
                "the renderer's Direct3D 11 device has no shared fences (ID3D11Device5 needs \
                 Windows 10 1703); synchronise with a keyed mutex",
            )
        })?;
        let mut fence: Option<ID3D11Fence> = None;
        unsafe { device.OpenSharedFence(handle, &mut fence) }
            .map_err(|e| platform("opening the shared fence", e))?;
        self.fence = fence;
        Ok(())
    }
}

/// How a shared present synchronises with its producer.
#[derive(Clone, Copy)]
pub(super) enum SharedSync {
    None,
    Keyed(u64, u64),
    Fence(u64, u64),
}

pub(super) fn present_shared(
    state: &Mutex<State>,
    opened: &Opened,
    sync: SharedSync,
    timeout_ms: u32,
) -> Result<(), SurfaceError> {
    let gpu = gpu()?;
    if gpu.generation != opened.generation {
        return Err(SurfaceError::DeviceChanged);
    }
    let guard = match sync {
        SharedSync::None => Guard::None,
        SharedSync::Keyed(acquire, release) => {
            let mutex = opened.mutex.as_ref().ok_or_else(|| {
                SurfaceError::Format(
                    "keyed-mutex keys were given for a texture made without SHARED_KEYEDMUTEX"
                        .into(),
                )
            })?;
            Guard::Keyed(mutex, acquire, release, timeout_ms)
        }
        SharedSync::Fence(wait, release) => {
            let fence = opened.fence.as_ref().ok_or_else(|| {
                SurfaceError::Format("the shared texture was opened without a fence".into())
            })?;
            Guard::Fence(fence, wait, release, timeout_ms)
        }
    };
    let mut state = lock(state);
    copy_in(&gpu, &mut state, &opened.texture, &opened.desc, guard)
}
