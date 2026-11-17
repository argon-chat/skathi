//! The Windows GPU half of a surface: each surface owns one BGRA8 texture on the renderer's D3D11
//! device (the D3D11On12 bridge under the D3D12 renderer). A present copies the producer's frame
//! into it on the GPU; the element's tick copies it into the atlas (`LiveTile::write_gpu`).

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11Device1, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIKeyedMutex;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE};

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
    handed: u64,
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

/// Copies `source` into the surface's texture and makes it the frame. Runs with `state` locked so
/// two producers cannot interleave; the context calls are serialised with the UI thread's.
fn copy_in(
    gpu: &Gpu,
    state: &mut State,
    source: &ID3D11Texture2D,
    desc: &D3D11_TEXTURE2D_DESC,
    sync: Option<(&IDXGIKeyedMutex, u64, u64, u32)>,
) -> Result<(), SurfaceError> {
    let size = (desc.Width, desc.Height);
    let target = state.gpu.ensure(gpu, size)?.clone();
    if let Some((mutex, acquire, _, timeout)) = sync {
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
            if desc.SampleDesc.Count > 1 {
                gpu.context
                    .ResolveSubresource(&target, 0, source, 0, DXGI_FORMAT_B8G8R8A8_UNORM);
            } else {
                gpu.context
                    .CopySubresourceRegion(&target, 0, 0, 0, 0, source, 0, None);
            }
            gpu.context.Flush();
        }
    }
    if let Some((mutex, _, release, _)) = sync {
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
    copy_in(&gpu, &mut state, source, &desc, None)
}

pub(crate) struct Opened {
    texture: ID3D11Texture2D,
    desc: D3D11_TEXTURE2D_DESC,
    mutex: Option<IDXGIKeyedMutex>,
    generation: u64,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) keys: Option<(u64, u64)>,
    pub(super) timeout_ms: u32,
}

// SAFETY: see `Gpu`.
unsafe impl Send for Opened {}

pub(super) fn open(handle: u64) -> Result<Opened, SurfaceError> {
    let gpu = gpu()?;
    let handle = HANDLE(handle as usize as *mut c_void);
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
        generation: gpu.generation,
        width: desc.Width,
        height: desc.Height,
        keys: None,
        timeout_ms: 100,
    })
}

pub(super) fn open_from_process(pid: u32, handle: u64) -> Result<Opened, SurfaceError> {
    let process = unsafe { OpenProcess(PROCESS_DUP_HANDLE, false, pid) }
        .map_err(|e| platform(&format!("opening process {pid} to duplicate from"), e))?;
    let mut ours = HANDLE::default();
    let duplicated = unsafe {
        DuplicateHandle(
            process,
            HANDLE(handle as usize as *mut c_void),
            GetCurrentProcess(),
            &mut ours,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    duplicated
        .map_err(|e| platform(&format!("duplicating handle {handle} of process {pid}"), e))?;
    let opened = open(ours.0 as usize as u64);
    unsafe {
        let _ = CloseHandle(ours);
    }
    opened
}

pub(super) fn present_shared(state: &Mutex<State>, opened: &Opened) -> Result<(), SurfaceError> {
    let gpu = gpu()?;
    if gpu.generation != opened.generation {
        return Err(SurfaceError::DeviceChanged);
    }
    let sync = match (opened.keys, opened.mutex.as_ref()) {
        (Some((acquire, release)), Some(mutex)) => {
            Some((mutex, acquire, release, opened.timeout_ms))
        }
        (Some(_), None) => {
            return Err(SurfaceError::Format(
                "keyed-mutex keys were given for a texture made without SHARED_KEYEDMUTEX".into(),
            ))
        }
        (None, _) => None,
    };
    let mut state = lock(state);
    copy_in(&gpu, &mut state, &opened.texture, &opened.desc, sync)
}
