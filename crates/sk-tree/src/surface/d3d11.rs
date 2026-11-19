//! The Windows GPU half of a surface: each surface owns one texture on the renderer's D3D11
//! device (the D3D11On12 bridge under the D3D12 renderer). A present copies the producer's frame
//! into it on the GPU; the element's tick copies it into the atlas (`LiveTile::write_gpu`), a
//! half-float one through an 8-bit texture it is drawn into first.

use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11DepthStencilState, ID3D11DepthStencilView, ID3D11Device, ID3D11Device1,
    ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4, ID3D11Fence, ID3D11GeometryShader,
    ID3D11Multithread, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_FORMAT_R16G16B16A16_TYPELESS, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIKeyedMutex;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::color::{self, Shader};
use super::{
    check_format, check_size, lock, Frame, State, SurfaceError, SurfaceFormat, REMOVED_DEVICE,
};

const WAIT_TIMEOUT: i32 = 0x102;

struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    multithread: ID3D11Multithread,
    generation: u64,
    shaders: OnceLock<Result<Shaders, String>>,
    _lease: Arc<dyn std::any::Any + Send + Sync>,
}

/// The half-float mapping, as a fullscreen triangle.
struct Shaders {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
}

impl Gpu {
    fn shaders(&self) -> Result<&Shaders, SurfaceError> {
        self.shaders
            .get_or_init(|| {
                let vs = color::bytecode(Shader::Vertex4)?;
                let ps = color::bytecode(Shader::Pixel4)?;
                let (mut vertex, mut pixel) = (None, None);
                unsafe {
                    self.device
                        .CreateVertexShader(vs, None, Some(&mut vertex))
                        .map_err(|e| format!("CreateVertexShader: {e}"))?;
                    self.device
                        .CreatePixelShader(ps, None, Some(&mut pixel))
                        .map_err(|e| format!("CreatePixelShader: {e}"))?;
                }
                Ok(Shaders {
                    vertex: vertex.ok_or("CreateVertexShader returned nothing")?,
                    pixel: pixel.ok_or("CreatePixelShader returned nothing")?,
                })
            })
            .as_ref()
            .map_err(|e| SurfaceError::Platform(e.clone()))
    }
}

/// What a texture format is to a surface: `None` for one it cannot take.
pub(super) fn surface_format(format: DXGI_FORMAT) -> Option<SurfaceFormat> {
    match format {
        DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
        | DXGI_FORMAT_B8G8R8A8_TYPELESS => Some(SurfaceFormat::Bgra8),
        DXGI_FORMAT_R16G16B16A16_FLOAT | DXGI_FORMAT_R16G16B16A16_TYPELESS => {
            Some(SurfaceFormat::Rgba16Float)
        }
        _ => None,
    }
}

pub(super) fn dxgi_format(format: SurfaceFormat) -> DXGI_FORMAT {
    match format {
        SurfaceFormat::Bgra8 => DXGI_FORMAT_B8G8R8A8_UNORM,
        SurfaceFormat::Rgba16Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
    }
}

// SAFETY: D3D11 devices are free-threaded, and the renderer's has multithread protection on, which
// serialises its immediate context; a sequence of context calls is bracketed by `Locked`.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);

fn platform(what: &str, e: windows::core::Error) -> SurfaceError {
    SurfaceError::Platform(format!("{what}: {e}"))
}

/// The renderer's device; `Err(None)` while it is removed.
fn current() -> Result<Arc<Gpu>, Option<SurfaceError>> {
    if super::removed() {
        return Err(None);
    }
    let lease = crate::video_device::renderer_device()
        .map_err(SurfaceError::NoDevice)?
        .ok_or_else(|| SurfaceError::NoDevice("no window has drawn yet".into()))?;
    let mut cached = lock(&GPU);
    if let Some(gpu) = cached.as_ref().filter(|g| g.generation == lease.generation) {
        if unsafe { gpu.device.GetDeviceRemovedReason() }.is_err() {
            // The host notices within a refresh, and has every surface let go of it.
            sk_runtime::frames::wake();
            return Err(None);
        }
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
        shaders: OnceLock::new(),
        _lease: lease.keep_alive,
    });
    *cached = Some(gpu.clone());
    Ok(gpu)
}

fn gpu() -> Result<Arc<Gpu>, SurfaceError> {
    current().map_err(|e| e.unwrap_or_else(|| SurfaceError::NoDevice(REMOVED_DEVICE.into())))
}

/// For a present: a frame made while the device is removed was made on the old one.
fn gpu_for_frame() -> Result<Arc<Gpu>, SurfaceError> {
    current().map_err(|e| e.unwrap_or(SurfaceError::DeviceChanged))
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
    format: SurfaceFormat,
    gpu: Arc<Gpu>,
    mapped: Option<Mapped>,
}

/// A half-float slot's 8-bit picture and the views that draw it; `drawn` once it shows the frame.
struct Mapped {
    texture: ID3D11Texture2D,
    target: ID3D11RenderTargetView,
    source: ID3D11ShaderResourceView,
    drawn: bool,
}

// SAFETY: see `Gpu`; the slot is only touched under the surface's state mutex.
unsafe impl Send for SurfaceGpu {}

fn create_texture(
    gpu: &Gpu,
    (width, height): (u32, u32),
    format: DXGI_FORMAT,
    bind: u32,
) -> Result<ID3D11Texture2D, SurfaceError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(|e| platform("creating the surface texture", e))?;
    texture.ok_or_else(|| SurfaceError::Platform("CreateTexture2D returned no texture".into()))
}

impl Mapped {
    fn new(gpu: &Gpu, frame: &ID3D11Texture2D, size: (u32, u32)) -> Result<Self, SurfaceError> {
        let bind = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
        let texture = create_texture(gpu, size, DXGI_FORMAT_B8G8R8A8_UNORM, bind)?;
        let (mut target, mut source) = (None, None);
        unsafe {
            gpu.device
                .CreateRenderTargetView(&texture, None, Some(&mut target))
                .map_err(|e| platform("CreateRenderTargetView", e))?;
            gpu.device
                .CreateShaderResourceView(frame, None, Some(&mut source))
                .map_err(|e| platform("CreateShaderResourceView", e))?;
        }
        Ok(Mapped {
            texture,
            target: target.ok_or_else(|| SurfaceError::Platform("no render target view".into()))?,
            source: source
                .ok_or_else(|| SurfaceError::Platform("no shader resource view".into()))?,
            drawn: false,
        })
    }

    /// Draws the frame into the 8-bit texture. The immediate context is the renderer's: what this
    /// binds is put back.
    fn draw(&mut self, gpu: &Gpu, (width, height): (u32, u32)) -> Result<(), SurfaceError> {
        let shaders = gpu.shaders()?;
        let _locked = Locked::enter(gpu);
        let cx = &gpu.context;
        unsafe {
            let topology = cx.IAGetPrimitiveTopology();
            let layout = cx.IAGetInputLayout().ok();
            let (mut vs, mut gs, mut ps) = (None, None, None);
            cx.VSGetShader(&mut vs, None, None);
            cx.GSGetShader(&mut gs, None, None);
            cx.PSGetShader(&mut ps, None, None);
            let mut resources: [Option<ID3D11ShaderResourceView>; 1] = [None];
            cx.PSGetShaderResources(0, Some(&mut resources));
            let mut viewport_count = 1u32;
            let mut viewport = D3D11_VIEWPORT::default();
            cx.RSGetViewports(&mut viewport_count, Some(&mut viewport));
            let raster = cx.RSGetState().ok();
            let mut targets: [Option<ID3D11RenderTargetView>; 1] = [None];
            let mut depth: Option<ID3D11DepthStencilView> = None;
            cx.OMGetRenderTargets(Some(&mut targets), Some(&mut depth));
            let (mut blend, mut factor, mut mask): (Option<ID3D11BlendState>, [f32; 4], u32) =
                (None, [0.0; 4], 0);
            cx.OMGetBlendState(Some(&mut blend), Some(&mut factor), Some(&mut mask));
            let (mut stencil, mut stencil_ref): (Option<ID3D11DepthStencilState>, u32) = (None, 0);
            cx.OMGetDepthStencilState(Some(&mut stencil), Some(&mut stencil_ref));

            cx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            cx.IASetInputLayout(None);
            cx.VSSetShader(&shaders.vertex, None);
            cx.GSSetShader(None::<&ID3D11GeometryShader>, None);
            cx.PSSetShader(&shaders.pixel, None);
            cx.PSSetShaderResources(0, Some(&[Some(self.source.clone())]));
            cx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            cx.RSSetState(None);
            cx.OMSetRenderTargets(Some(&[Some(self.target.clone())]), None);
            cx.OMSetBlendState(None, None, u32::MAX);
            cx.OMSetDepthStencilState(None, 0);
            cx.Draw(3, 0);

            cx.IASetPrimitiveTopology(topology);
            cx.IASetInputLayout(layout.as_ref());
            cx.VSSetShader(vs.as_ref(), None);
            cx.GSSetShader(gs.as_ref(), None);
            cx.PSSetShader(ps.as_ref(), None);
            cx.PSSetShaderResources(0, Some(&resources));
            cx.RSSetViewports(Some(&[viewport][..viewport_count.min(1) as usize]));
            cx.RSSetState(raster.as_ref());
            cx.OMSetRenderTargets(Some(&targets), depth.as_ref());
            cx.OMSetBlendState(blend.as_ref(), Some(&factor), mask);
            cx.OMSetDepthStencilState(stencil.as_ref(), stencil_ref);
        }
        self.drawn = true;
        Ok(())
    }
}

impl SurfaceGpu {
    /// The texture the atlas copies: the frame's, or a half-float frame's 8-bit mapping, drawn
    /// the first time it is asked for. With `hdr` the half-float texture itself.
    pub(crate) fn texture(&mut self, hdr: bool) -> Result<Option<Texture>, SurfaceError> {
        let Some(slot) = self.slot.as_mut() else {
            return Ok(None);
        };
        match &mut slot.mapped {
            None => Ok(Some(Texture(slot.texture.clone()))),
            Some(_) if hdr => Ok(Some(Texture(slot.texture.clone()))),
            Some(mapped) => {
                if !mapped.drawn {
                    mapped.draw(&slot.gpu, slot.size)?;
                }
                Ok(Some(Texture(mapped.texture.clone())))
            }
        }
    }

    pub(crate) fn forget(&mut self) {
        self.slot = None;
    }

    fn ensure(
        &mut self,
        gpu: &Arc<Gpu>,
        size: (u32, u32),
        format: SurfaceFormat,
    ) -> Result<&mut Slot, SurfaceError> {
        let fits = self.slot.as_ref().is_some_and(|s| {
            s.size == size && s.format == format && s.gpu.generation == gpu.generation
        });
        if !fits {
            self.slot = None;
            let texture = create_texture(
                gpu,
                size,
                dxgi_format(format),
                D3D11_BIND_SHADER_RESOURCE.0 as u32,
            )?;
            let mapped = match format {
                SurfaceFormat::Bgra8 => None,
                SurfaceFormat::Rgba16Float => {
                    gpu.shaders()?;
                    Some(Mapped::new(gpu, &texture, size)?)
                }
            };
            self.slot = Some(Slot {
                texture,
                size,
                format,
                gpu: gpu.clone(),
                mapped,
            });
        }
        Ok(self.slot.as_mut().expect("just made"))
    }
}

fn describe(
    texture: &ID3D11Texture2D,
) -> Result<(D3D11_TEXTURE2D_DESC, SurfaceFormat), SurfaceError> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let format = surface_format(desc.Format).ok_or_else(|| {
        SurfaceError::Format(format!(
            "a texture in DXGI format {}: a surface takes B8G8R8A8_UNORM (or its _UNORM_SRGB and \
             _TYPELESS forms), or R16G16B16A16_FLOAT on a surface set to Rgba16Float",
            desc.Format.0
        ))
    })?;
    check_size(desc.Width, desc.Height)?;
    Ok((desc, format))
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
    gpu: &Arc<Gpu>,
    state: &mut State,
    source: &ID3D11Texture2D,
    (desc, format): &(D3D11_TEXTURE2D_DESC, SurfaceFormat),
    sync: Guard<'_>,
) -> Result<u64, SurfaceError> {
    state.accept_frame()?;
    // Removed since `gpu()`: the surface has let go of its texture, and must not make another.
    if super::removed() {
        return Err(SurfaceError::DeviceChanged);
    }
    check_format(state.format, *format)?;
    let size = (desc.Width, desc.Height);
    let target = state.gpu.ensure(gpu, size, *format)?.texture.clone();
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
                    .ResolveSubresource(&target, 0, source, 0, dxgi_format(*format));
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
    if let Some(mapped) = state.gpu.slot.as_mut().and_then(|s| s.mapped.as_mut()) {
        mapped.drawn = false;
    }
    Ok(state.put(Frame::Gpu {
        width: size.0,
        height: size.1,
    }))
}

/// # Safety
/// `raw` is null or a live `ID3D11Texture2D`.
pub(super) unsafe fn present_texture(
    state: &Mutex<State>,
    raw: *mut c_void,
) -> Result<u64, SurfaceError> {
    // SAFETY: the caller's.
    let source = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
        .ok_or_else(|| SurfaceError::Size("a null texture".into()))?;
    let desc = describe(source)?;
    let gpu = gpu_for_frame()?;
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
    desc: (D3D11_TEXTURE2D_DESC, SurfaceFormat),
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
        (self.desc.0.Width, self.desc.0.Height)
    }

    pub(super) fn format(&self) -> SurfaceFormat {
        self.desc.1
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
) -> Result<u64, SurfaceError> {
    let gpu = gpu_for_frame()?;
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

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
        D3D11_USAGE_STAGING,
    };

    use super::*;
    use crate::surface::color::tests::{f32_to_half, ramp};
    use crate::surface::{surface, SurfaceHandle};

    /// A WARP device published as the renderer's, the way the host publishes its own.
    fn renderer() -> ID3D11Device {
        struct Kept(ID3D11Device);
        // SAFETY: multithread-protected below.
        unsafe impl Send for Kept {}
        unsafe impl Sync for Kept {}
        static DEVICE: OnceLock<Kept> = OnceLock::new();
        DEVICE
            .get_or_init(|| {
                let mut device = None;
                unsafe {
                    D3D11CreateDevice(
                        None,
                        D3D_DRIVER_TYPE_WARP,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        None,
                        D3D11_SDK_VERSION,
                        Some(&mut device),
                        None,
                        None,
                    )
                }
                .unwrap();
                let device: ID3D11Device = device.unwrap();
                let _ = unsafe {
                    device
                        .cast::<ID3D11Multithread>()
                        .unwrap()
                        .SetMultithreadProtected(true)
                };
                unsafe { crate::set_video_device(device.as_raw(), 1) };
                Kept(device)
            })
            .0
            .clone()
    }

    fn texture(
        device: &ID3D11Device,
        format: DXGI_FORMAT,
        width: u32,
        texels: &[u8],
    ) -> ID3D11Texture2D {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: texels.as_ptr().cast(),
            SysMemPitch: texels.len() as u32,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)) }.unwrap();
        texture.unwrap()
    }

    /// The BGRA the atlas would copy for the surface's frame.
    fn shown(device: &ID3D11Device, surface: &SurfaceHandle) -> Vec<[u8; 4]> {
        let shown = surface.lock_state().gpu_texture(false).unwrap().unwrap();
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { shown.0.GetDesc(&mut desc) };
        assert_eq!(desc.Format, DXGI_FORMAT_B8G8R8A8_UNORM);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }.unwrap();
        let staging = staging.unwrap();
        let context = unsafe { device.GetImmediateContext() }.unwrap();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context.CopyResource(&staging, &shown.0);
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
        }
        let row = unsafe {
            std::slice::from_raw_parts(mapped.pData.cast::<u8>(), desc.Width as usize * 4)
        };
        let pixels = row
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        unsafe { context.Unmap(&staging, 0) };
        pixels
    }

    fn halves(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn a_half_float_texture_shows_every_srgb_code_unchanged_and_clamps_above_one() {
        let device = renderer();
        let s = surface("tests:d3d11-rgba16f");
        s.set_format(SurfaceFormat::Rgba16Float);
        let h = f32_to_half;
        let mut texels = ramp();
        texels.extend([h(8.0), h(1.5), h(-0.25), h(2.0)]);
        let source = texture(
            &device,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            257,
            &halves(&texels),
        );
        unsafe { s.present_d3d11_texture(source.as_raw()) }.unwrap();
        let pixels = shown(&device, &s);
        for (code, px) in pixels[..256].iter().enumerate() {
            assert_eq!(px, &[code as u8; 4], "code {code}");
        }
        assert_eq!(pixels[256], [0, 255, 255, 255]);
    }

    #[test]
    fn a_texture_of_the_other_format_is_refused() {
        let device = renderer();
        let half = surface("tests:d3d11-mismatch-half");
        half.set_format(SurfaceFormat::Rgba16Float);
        let bgra = texture(&device, DXGI_FORMAT_B8G8R8A8_UNORM, 1, &[1, 2, 3, 4]);
        assert_eq!(
            unsafe { half.present_d3d11_texture(bgra.as_raw()) },
            Err(SurfaceError::FormatMismatch {
                surface: SurfaceFormat::Rgba16Float,
                frame: SurfaceFormat::Bgra8
            })
        );
        let float = texture(&device, DXGI_FORMAT_R16G16B16A16_FLOAT, 1, &[0; 8]);
        let plain = surface("tests:d3d11-mismatch-bgra");
        assert!(matches!(
            unsafe { plain.present_d3d11_texture(float.as_raw()) },
            Err(SurfaceError::FormatMismatch { .. })
        ));
        assert_eq!(half.stats().presented + plain.stats().presented, 0);
    }
}
