//! An embedder that shows native frames in `<sk-surface>` elements. The app asks for them through
//! `appHost.call`:
//!
//! - `present(name, mode, [b, g, r, a], width, height)` presents one solid frame from a thread of
//!   ours. `mode` is `"bgra"` (CPU), `"texture"` (a texture on the renderer's device) or `"shared"`
//!   (a texture on a device of our own, handed over as a shared NT handle with a keyed mutex, or
//!   an IOSurface with an `MTLSharedEvent` on macOS), `"d3d11-fence"` (the same with a shared
//!   `ID3D11Fence`), `"d3d12"` (a resource on the renderer's D3D12 device) or `"d3d12-shared"` (a
//!   D3D12 device of our own, shared resource and fence). On macOS also `"metal"` (a texture on
//!   the renderer's `MTLDevice`), `"iosurface"`, `"mach-port"` (an IOSurface looked up from a
//!   mach port), `"cv-bgra"` and `"cv-nv12"` (CVPixelBuffers).
//! - `animate(name)` clears a texture to a moving colour at 60 Hz, at the element's painted size.
//! - `stats(name)` answers `{ presented, shown, width, height, visible }`.
//!
//! ```text
//! cargo build -p sk-app --example surface
//! set SK_DIST=<a dev dist of packages/test/fixtures/surface>
//! target\x86_64-pc-windows-msvc\debug\examples\surface.exe
//! ```
//!
//! packages/test/specs/surface.spec.ts drives it.

// The surface spec and its fixture still speak `appHost.call`.
#![allow(deprecated)]

use std::time::Duration;

use serde_json::{json, Value};
use sk_app::embed::{Builder, Call, SurfaceHandle};

fn args_of(call: &Call) -> Vec<Value> {
    serde_json::from_str::<Vec<Value>>(call.args()).unwrap_or_default()
}

fn surface_of(args: &[Value]) -> SurfaceHandle {
    let name = args.first().and_then(Value::as_str).unwrap_or("");
    sk_app::embed::surface(name)
}

fn main() {
    let builder = Builder::new()
        .activate(false)
        .function("present", |call| {
            let args = args_of(&call);
            let surface = surface_of(&args);
            let mode = args
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or("bgra")
                .to_string();
            let color: Vec<u8> = args
                .get(2)
                .and_then(Value::as_array)
                .map(|c| c.iter().map(|v| v.as_u64().unwrap_or(0) as u8).collect())
                .unwrap_or_else(|| vec![0, 0, 0, 255]);
            let color = [color[0], color[1], color[2], color[3]];
            let width = args.get(3).and_then(Value::as_u64).unwrap_or(64) as u32;
            let height = args.get(4).and_then(Value::as_u64).unwrap_or(64) as u32;
            let reply = call.into_reply();
            std::thread::spawn(move || {
                let presented = match mode.as_str() {
                    "texture" => gpu::present_texture(&surface, color, width, height),
                    "shared" => gpu::present_shared(&surface, color, width, height),
                    "d3d11-fence" => gpu::present_d3d11_fenced(&surface, color, width, height),
                    "d3d12" => gpu::d3d12::present(&surface, color, width, height),
                    "d3d12-shared" => gpu::d3d12::present_shared(&surface, color, width, height),
                    "metal" | "iosurface" | "mach-port" | "cv-bgra" | "cv-nv12" => {
                        metal::present(&surface, &mode, color, width, height)
                    }
                    _ => {
                        let frame = color.repeat((width * height) as usize);
                        surface
                            .present_bgra(&frame, width, height, width as usize * 4)
                            .map_err(|e| e.to_string())
                    }
                };
                match presented {
                    Ok(()) => reply.resolve("true"),
                    Err(e) => reply.reject(e),
                }
            });
        })
        .function("animate", |call| {
            let surface = surface_of(&args_of(&call));
            std::thread::spawn(move || gpu::animate(surface));
            call.resolve("true");
        })
        .function("clear", |call| {
            surface_of(&args_of(&call)).clear();
            call.resolve("true");
        })
        .function("stats", |call| {
            let surface = surface_of(&args_of(&call));
            let (stats, info) = (surface.stats(), surface.info());
            call.resolve(
                json!({
                    "presented": stats.presented,
                    "shown": stats.shown,
                    "width": info.width,
                    "height": info.height,
                    "visible": info.visible,
                })
                .to_string(),
            );
        });
    #[cfg(target_os = "macos")]
    if let Err(e) = builder.run_on_main_thread(|exit| i32::from(!exit.clean)) {
        eprintln!("surface: {e}");
        std::process::exit(2)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let ui = builder.spawn().unwrap_or_else(|e| {
            eprintln!("surface: {e}");
            std::process::exit(2)
        });
        let exit = ui.wait();
        std::thread::sleep(Duration::from_millis(10));
        std::process::exit(if exit.clean { 0 } else { 1 });
    }
}

#[cfg(windows)]
mod gpu {
    use std::time::{Duration, Instant};

    use sk_app::embed::{SharedTexture, SurfaceError, SurfaceHandle};
    use windows::core::Interface;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HMODULE, WAIT_OBJECT_0};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4,
        ID3D11Fence, ID3D11RenderTargetView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
        D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FENCE_FLAG_SHARED,
        D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
        D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        IDXGIKeyedMutex, IDXGIResource1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    type Result<T> = std::result::Result<T, String>;

    fn err(e: impl std::fmt::Display) -> String {
        e.to_string()
    }

    fn target(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        misc: u32,
    ) -> Result<(ID3D11Texture2D, ID3D11RenderTargetView)> {
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
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: misc,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }.map_err(err)?;
        let texture = texture.ok_or("no texture")?;
        let mut view = None;
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut view)) }.map_err(err)?;
        Ok((texture, view.ok_or("no view")?))
    }

    fn rgba(color: [u8; 4]) -> [f32; 4] {
        let [b, g, r, a] = color.map(|c| c as f32 / 255.0);
        [r, g, b, a]
    }

    /// The renderer's device, borrowed as this crate's `windows` version sees it.
    fn renderer(surface: &SurfaceHandle) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
        let device = surface.d3d11_device().map_err(err)?;
        let raw = device.as_raw();
        let device = unsafe { ID3D11Device::from_raw_borrowed(&raw) }
            .ok_or("null device")?
            .clone();
        let context = unsafe { device.GetImmediateContext() }.map_err(err)?;
        Ok((device, context))
    }

    pub fn present_texture(surface: &SurfaceHandle, color: [u8; 4], w: u32, h: u32) -> Result<()> {
        let (device, context) = renderer(surface)?;
        let (texture, view) = target(&device, w, h, 0)?;
        unsafe {
            context.ClearRenderTargetView(&view, &rgba(color));
            surface.present_d3d11_texture(texture.as_raw())
        }
        .map_err(err)
    }

    /// What another process would do: its own device, a shared NT handle, a keyed mutex.
    pub fn present_shared(surface: &SurfaceHandle, color: [u8; 4], w: u32, h: u32) -> Result<()> {
        let (device, context) = own_device()?;
        let misc = (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0)
            as u32;
        let (texture, view) = target(&device, w, h, misc)?;
        let mutex: IDXGIKeyedMutex = texture.cast().map_err(err)?;
        unsafe {
            mutex.AcquireSync(0, 1000).map_err(err)?;
            context.ClearRenderTargetView(&view, &rgba(color));
            mutex.ReleaseSync(1).map_err(err)?;
        }
        let resource: IDXGIResource1 = texture.cast().map_err(err)?;
        let handle = unsafe {
            resource.CreateSharedHandle(
                None,
                (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                None,
            )
        }
        .map_err(err)?;
        // The handle number is what would cross the process boundary, with this pid.
        let opened = SharedTexture::open_from_process(std::process::id(), handle.0 as usize as u64);
        unsafe {
            let _ = CloseHandle(handle);
        }
        let shared = opened.map_err(err)?.with_keyed_mutex(1, 0);
        surface.present_shared(&shared).map_err(err)?;
        // The producer can take its texture back once the consumer released key 0.
        unsafe { mutex.AcquireSync(0, 1000) }.map_err(err)?;
        unsafe { mutex.ReleaseSync(0) }.map_err(err)?;
        Ok(())
    }

    fn own_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(err)?;
        Ok((device.ok_or("no device")?, context.ok_or("no context")?))
    }

    /// A D3D11 producer synchronised by a shared `ID3D11Fence` instead of a keyed mutex.
    pub fn present_d3d11_fenced(
        surface: &SurfaceHandle,
        color: [u8; 4],
        w: u32,
        h: u32,
    ) -> Result<()> {
        let (device, context) = own_device()?;
        let device5: ID3D11Device5 = device.cast().map_err(err)?;
        let context4: ID3D11DeviceContext4 = context.cast().map_err(err)?;
        let mut fence: Option<ID3D11Fence> = None;
        unsafe { device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence) }.map_err(err)?;
        let fence = fence.ok_or("no fence")?;
        let misc = (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32;
        let (texture, view) = target(&device, w, h, misc)?;
        unsafe {
            context.ClearRenderTargetView(&view, &rgba(color));
            context4.Signal(&fence, 1).map_err(err)?;
            context.Flush();
        }
        let resource: IDXGIResource1 = texture.cast().map_err(err)?;
        let texture_handle = unsafe {
            resource.CreateSharedHandle(
                None,
                (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                None,
            )
        }
        .map_err(err)?;
        let fence_handle =
            unsafe { fence.CreateSharedHandle(None, GENERIC_ALL.0, None) }.map_err(err)?;
        let presented = SharedTexture::open(texture_handle.0 as usize as u64)
            .and_then(|t| t.with_fence(fence_handle.0 as usize as u64))
            .and_then(|t| surface.present_shared_fenced(&t, 1, 2));
        unsafe {
            let _ = CloseHandle(texture_handle);
            let _ = CloseHandle(fence_handle);
        }
        presented.map_err(err)?;
        let event = unsafe { CreateEventW(None, false, false, None) }.map_err(err)?;
        let reached = unsafe { fence.SetEventOnCompletion(2, event) }.is_ok()
            && unsafe { WaitForSingleObject(event, 2000) } == WAIT_OBJECT_0;
        unsafe {
            let _ = CloseHandle(event);
        }
        if reached {
            Ok(())
        } else {
            Err("the surface did not signal the release value".into())
        }
    }

    /// Renders at the element's painted size, recreating its texture after a resize or a device loss.
    pub fn animate(surface: SurfaceHandle) {
        let start = Instant::now();
        let mut held: Option<((u32, u32), ID3D11Texture2D, ID3D11RenderTargetView)> = None;
        let mut renderer_dc: Option<(ID3D11Device, ID3D11DeviceContext)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(16));
            let info = surface.info();
            if !info.visible || info.width == 0 {
                continue;
            }
            if renderer_dc.is_none() {
                renderer_dc = renderer(&surface).ok();
            }
            let Some((device, context)) = renderer_dc.as_ref() else {
                continue;
            };
            let size = (info.width, info.height);
            if held.as_ref().is_none_or(|(s, ..)| *s != size) {
                held = target(device, size.0, size.1, 0)
                    .ok()
                    .map(|(t, v)| (size, t, v));
            }
            let Some((_, texture, view)) = held.as_ref() else {
                continue;
            };
            let t = start.elapsed().as_secs_f32();
            let color = [
                0.5 + 0.5 * t.sin(),
                0.5 + 0.5 * (t * 1.3).sin(),
                0.5 + 0.5 * (t * 0.7).sin(),
                1.0,
            ];
            unsafe { context.ClearRenderTargetView(view, &color) };
            match unsafe { surface.present_d3d11_texture(texture.as_raw()) } {
                Err(SurfaceError::DeviceChanged | SurfaceError::WrongDevice) => {
                    held = None;
                    renderer_dc = None;
                }
                _ => {}
            }
        }
    }

    pub mod d3d12 {
        use std::mem::ManuallyDrop;

        use sk_app::embed::{SharedTexture, SurfaceHandle};
        use windows::core::Interface;
        use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, WAIT_OBJECT_0};
        use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
        use windows::Win32::Graphics::Direct3D12::{
            D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
            ID3D12DescriptorHeap, ID3D12Device, ID3D12DeviceChild, ID3D12Fence,
            ID3D12GraphicsCommandList, ID3D12Resource, D3D12_COMMAND_LIST_TYPE_DIRECT,
            D3D12_COMMAND_QUEUE_DESC, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            D3D12_FENCE_FLAG_NONE, D3D12_FENCE_FLAG_SHARED, D3D12_HEAP_FLAGS, D3D12_HEAP_FLAG_NONE,
            D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT,
            D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0,
            D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_STATES,
            D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_TRANSITION_BARRIER,
        };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };
        use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

        use super::{err, rgba, Result};

        struct Producer {
            device: ID3D12Device,
            queue: ID3D12CommandQueue,
            fence: ID3D12Fence,
        }

        impl Producer {
            fn new(device: ID3D12Device, fence_flags: bool) -> Result<Self> {
                let queue = unsafe {
                    device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                        Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                        ..Default::default()
                    })
                }
                .map_err(err)?;
                let flags = if fence_flags {
                    D3D12_FENCE_FLAG_SHARED
                } else {
                    D3D12_FENCE_FLAG_NONE
                };
                let fence = unsafe { device.CreateFence(0, flags) }.map_err(err)?;
                Ok(Producer {
                    device,
                    queue,
                    fence,
                })
            }

            fn texture(&self, w: u32, h: u32, heap: D3D12_HEAP_FLAGS) -> Result<ID3D12Resource> {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: u64::from(w),
                    Height: h,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                    ..Default::default()
                };
                let props = D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_DEFAULT,
                    ..Default::default()
                };
                let mut resource: Option<ID3D12Resource> = None;
                unsafe {
                    self.device.CreateCommittedResource(
                        &props,
                        heap,
                        &desc,
                        D3D12_RESOURCE_STATE_RENDER_TARGET,
                        None,
                        &mut resource,
                    )
                }
                .map_err(err)?;
                resource.ok_or_else(|| "no resource".into())
            }

            /// Clears `resource` (in RENDER_TARGET), leaves it in `after` and signals `value`.
            fn clear(
                &self,
                resource: &ID3D12Resource,
                color: [u8; 4],
                after: D3D12_RESOURCE_STATES,
                value: u64,
            ) -> Result<()> {
                let heap: ID3D12DescriptorHeap = unsafe {
                    self.device
                        .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                            NumDescriptors: 1,
                            ..Default::default()
                        })
                }
                .map_err(err)?;
                let rtv = unsafe { heap.GetCPUDescriptorHandleForHeapStart() };
                unsafe { self.device.CreateRenderTargetView(resource, None, rtv) };
                let allocator: ID3D12CommandAllocator = unsafe {
                    self.device
                        .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                }
                .map_err(err)?;
                let list: ID3D12GraphicsCommandList = unsafe {
                    self.device.CreateCommandList(
                        0,
                        D3D12_COMMAND_LIST_TYPE_DIRECT,
                        &allocator,
                        None,
                    )
                }
                .map_err(err)?;
                unsafe { list.ClearRenderTargetView(rtv, &rgba(color), None) };
                if after != D3D12_RESOURCE_STATE_RENDER_TARGET {
                    let mut barrier = D3D12_RESOURCE_BARRIER {
                        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                        Anonymous: D3D12_RESOURCE_BARRIER_0 {
                            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                                pResource: ManuallyDrop::new(Some(resource.clone())),
                                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                                StateBefore: D3D12_RESOURCE_STATE_RENDER_TARGET,
                                StateAfter: after,
                            }),
                        },
                        ..Default::default()
                    };
                    unsafe {
                        list.ResourceBarrier(std::slice::from_ref(&barrier));
                        ManuallyDrop::drop(&mut (*barrier.Anonymous.Transition).pResource);
                    }
                }
                unsafe { list.Close() }.map_err(err)?;
                let list: ID3D12CommandList = list.cast().map_err(err)?;
                unsafe {
                    self.queue.ExecuteCommandLists(&[Some(list)]);
                    self.queue.Signal(&self.fence, value).map_err(err)?;
                }
                // The allocator and heap must outlive the GPU work.
                wait(&self.fence, value)
            }
        }

        fn wait(fence: &ID3D12Fence, value: u64) -> Result<()> {
            if unsafe { fence.GetCompletedValue() } >= value {
                return Ok(());
            }
            let event = unsafe { CreateEventW(None, false, false, None) }.map_err(err)?;
            let reached = unsafe { fence.SetEventOnCompletion(value, event) }.is_ok()
                && unsafe { WaitForSingleObject(event, 2000) } == WAIT_OBJECT_0;
            unsafe {
                let _ = CloseHandle(event);
            }
            if reached {
                Ok(())
            } else {
                Err(format!("fence value {value} was not reached"))
            }
        }

        /// A producer on the renderer's device: its own queue, a GPU-side handoff, the release fence.
        pub fn present(surface: &SurfaceHandle, color: [u8; 4], w: u32, h: u32) -> Result<()> {
            let renderer = surface.d3d12_device().map_err(err)?;
            let raw = renderer.as_raw();
            let device = unsafe { ID3D12Device::from_raw_borrowed(&raw) }
                .ok_or("null device")?
                .clone();
            let producer = Producer::new(device, false)?;
            let resource = producer.texture(w, h, D3D12_HEAP_FLAG_NONE)?;
            producer.clear(&resource, color, D3D12_RESOURCE_STATE_RENDER_TARGET, 1)?;
            let released = unsafe {
                surface.present_d3d12(
                    resource.as_raw(),
                    D3D12_RESOURCE_STATE_RENDER_TARGET.0,
                    Some((producer.fence.as_raw(), 1)),
                )
            }
            .map_err(err)?;
            let release = surface.release_fence().map_err(err)?;
            if release.wait(released, std::time::Duration::from_secs(2)) {
                Ok(())
            } else {
                Err("the release fence did not reach the returned value".into())
            }
        }

        /// What a producer with its own D3D12 device does (a Vulkan one imports these two objects):
        /// share the resource and a fence.
        pub fn present_shared(
            surface: &SurfaceHandle,
            color: [u8; 4],
            w: u32,
            h: u32,
        ) -> Result<()> {
            let mut device: Option<ID3D12Device> = None;
            unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_11_0, &mut device) }.map_err(err)?;
            let producer = Producer::new(device.ok_or("no device")?, true)?;
            let resource = producer.texture(w, h, D3D12_HEAP_FLAG_SHARED)?;
            producer.clear(&resource, color, D3D12_RESOURCE_STATE_COMMON, 1)?;
            let share = |object: &ID3D12DeviceChild| unsafe {
                producer
                    .device
                    .CreateSharedHandle(object, None, GENERIC_ALL.0, None)
            };
            let texture_handle = share(&resource.cast().map_err(err)?).map_err(err)?;
            let fence_handle = share(&producer.fence.cast().map_err(err)?).map_err(err)?;
            let pid = std::process::id();
            let presented = SharedTexture::open_from_process(pid, texture_handle.0 as usize as u64)
                .and_then(|t| t.with_fence_from_process(pid, fence_handle.0 as usize as u64))
                .and_then(|t| surface.present_shared_fenced(&t, 1, 2));
            unsafe {
                let _ = CloseHandle(texture_handle);
                let _ = CloseHandle(fence_handle);
            }
            presented.map_err(err)?;
            wait(&producer.fence, 2)
        }
    }
}

#[cfg(target_os = "macos")]
mod gpu {
    use sk_app::embed::SurfaceHandle;

    pub use super::metal::animate;

    pub fn present_texture(s: &SurfaceHandle, c: [u8; 4], w: u32, h: u32) -> Result<(), String> {
        super::metal::present(s, "metal", c, w, h)
    }

    pub fn present_shared(s: &SurfaceHandle, c: [u8; 4], w: u32, h: u32) -> Result<(), String> {
        super::metal::present(s, "iosurface", c, w, h)
    }

    pub fn present_d3d11_fenced(
        s: &SurfaceHandle,
        _: [u8; 4],
        _: u32,
        _: u32,
    ) -> Result<(), String> {
        s.d3d11_device().map(|_| ()).map_err(|e| e.to_string())
    }

    pub mod d3d12 {
        pub use super::present_d3d11_fenced as present;
        pub use super::present_d3d11_fenced as present_shared;
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod gpu {
    use std::time::{Duration, Instant};

    use sk_app::embed::SurfaceHandle;

    pub fn present_texture(s: &SurfaceHandle, _: [u8; 4], _: u32, _: u32) -> Result<(), String> {
        s.d3d11_device().map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn present_shared(s: &SurfaceHandle, c: [u8; 4], w: u32, h: u32) -> Result<(), String> {
        present_texture(s, c, w, h)
    }

    pub fn present_d3d11_fenced(
        s: &SurfaceHandle,
        c: [u8; 4],
        w: u32,
        h: u32,
    ) -> Result<(), String> {
        present_texture(s, c, w, h)
    }

    pub mod d3d12 {
        pub use super::{present_shared, present_texture as present};
    }

    /// CPU frames at the painted size: no GPU path here.
    pub fn animate(surface: SurfaceHandle) {
        let start = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(16));
            let info = surface.info();
            if !info.visible || info.width == 0 {
                continue;
            }
            let t = start.elapsed().as_secs_f32();
            let c = |k: f32| ((0.5 + 0.5 * (t * k).sin()) * 255.0) as u8;
            let frame = [c(0.7), c(1.3), c(1.0), 255].repeat((info.width * info.height) as usize);
            let _ = surface.present_bgra(&frame, info.width, info.height, info.width as usize * 4);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod metal {
    use sk_app::embed::SurfaceHandle;

    pub fn present(s: &SurfaceHandle, _: &str, _: [u8; 4], _: u32, _: u32) -> Result<(), String> {
        s.metal_device().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
mod metal {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::time::{Duration, Instant};

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_core_foundation::{CFDictionary, CFRetained, CFType};
    use objc2_core_video::{
        kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess, CVPixelBuffer,
        CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane,
        CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeightOfPlane,
        CVPixelBufferGetIOSurface, CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress,
        CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    };
    use objc2_io_surface::IOSurfaceRef;
    use objc2_metal::{
        MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
        MTLCreateSystemDefaultDevice, MTLDevice, MTLLoadAction, MTLPixelFormat,
        MTLRenderPassDescriptor, MTLSharedEvent, MTLStorageMode, MTLStoreAction, MTLTexture,
        MTLTextureDescriptor, MTLTextureUsage,
    };
    use sk_app::embed::{SharedTexture, SurfaceError, SurfaceHandle};

    type Result<T> = std::result::Result<T, String>;
    type Device = ProtocolObject<dyn MTLDevice>;
    type Texture = ProtocolObject<dyn MTLTexture>;
    type Event = ProtocolObject<dyn MTLSharedEvent>;

    fn err(e: impl std::fmt::Display) -> String {
        e.to_string()
    }

    fn raw<T: ?Sized>(object: &T) -> *mut c_void {
        (object as *const T).cast_mut().cast()
    }

    /// The renderer's device, retained as this crate's `objc2` sees it.
    fn renderer(surface: &SurfaceHandle) -> Result<Retained<Device>> {
        let device = surface.metal_device().map_err(err)?;
        unsafe { Retained::retain(device.as_raw().cast::<Device>()) }.ok_or("null device".into())
    }

    fn texture(
        device: &Device,
        w: u32,
        h: u32,
        on: Option<&IOSurfaceRef>,
    ) -> Result<Retained<Texture>> {
        let desc = MTLTextureDescriptor::new();
        desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        unsafe {
            desc.setWidth(w as usize);
            desc.setHeight(h as usize);
        }
        desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        let texture = match on {
            Some(surface) => device.newTextureWithDescriptor_iosurface_plane(&desc, surface, 0),
            None => {
                desc.setStorageMode(MTLStorageMode::Private);
                device.newTextureWithDescriptor(&desc)
            }
        };
        texture.ok_or("no texture".into())
    }

    /// Clears `target` on `device`'s new queue, then signals `event` to `value`.
    fn clear(
        device: &Device,
        target: &Texture,
        [b, g, r, a]: [u8; 4],
        signal: Option<(&Event, u64)>,
    ) -> Result<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let queue = device.newCommandQueue().ok_or("no queue")?;
        let command = queue.commandBuffer().ok_or("no command buffer")?;
        let pass = MTLRenderPassDescriptor::new();
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(target));
        color.setLoadAction(MTLLoadAction::Clear);
        color.setStoreAction(MTLStoreAction::Store);
        let unit = |c: u8| f64::from(c) / 255.0;
        color.setClearColor(MTLClearColor {
            red: unit(r),
            green: unit(g),
            blue: unit(b),
            alpha: unit(a),
        });
        command
            .renderCommandEncoderWithDescriptor(&pass)
            .ok_or("no encoder")?
            .endEncoding();
        if let Some((event, value)) = signal {
            command.encodeSignalEvent_value(ProtocolObject::from_ref(event), value);
        }
        command.commit();
        Ok(command)
    }

    fn reached(event: &Event, value: u64) -> Result<()> {
        let until = Instant::now() + Duration::from_secs(2);
        while event.signaledValue() < value {
            if Instant::now() > until {
                return Err(format!("the shared event did not reach {value}"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    fn pixel_buffer(w: u32, h: u32, format: u32) -> Result<CFRetained<CVPixelBuffer>> {
        let key = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
        let attributes =
            CFDictionary::from_slices(&[key], &[&*CFDictionary::<CFType, CFType>::empty()]);
        let mut buffer = std::ptr::null_mut();
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                w as usize,
                h as usize,
                format,
                Some(attributes.as_opaque()),
                NonNull::from(&mut buffer),
            )
        };
        if status != kCVReturnSuccess {
            return Err(format!("CVPixelBufferCreate: {status}"));
        }
        let buffer = NonNull::new(buffer).ok_or("no pixel buffer")?;
        Ok(unsafe { CFRetained::from_raw(buffer) })
    }

    fn fill(buffer: &CVPixelBuffer, plane: usize, texel: &[u8]) {
        unsafe {
            CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
            let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane);
            let width = CVPixelBufferGetWidthOfPlane(buffer, plane);
            for y in 0..CVPixelBufferGetHeightOfPlane(buffer, plane) {
                let row = std::slice::from_raw_parts_mut(base.add(y * stride), width * texel.len());
                for px in row.chunks_exact_mut(texel.len()) {
                    px.copy_from_slice(texel);
                }
            }
            CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
        }
    }

    /// BT.709 video-range Y, Cb, Cr of a BGRA colour.
    fn ycbcr([b, g, r, _]: [u8; 4]) -> [u8; 3] {
        let [r, g, b] = [r, g, b].map(|c| f32::from(c) / 255.0);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cb = (b - y) / 1.8556;
        let cr = (r - y) / 1.5748;
        [16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr].map(|v| v.round() as u8)
    }

    pub fn present(
        surface: &SurfaceHandle,
        mode: &str,
        color: [u8; 4],
        w: u32,
        h: u32,
    ) -> Result<()> {
        match mode {
            "metal" => {
                // A producer on the renderer's device: its own queue, a GPU-side handoff.
                let device = renderer(surface)?;
                let event = device.newSharedEvent().ok_or("no event")?;
                let target = texture(&device, w, h, None)?;
                clear(&device, &target, color, Some((&event, 1)))?;
                let released = unsafe {
                    surface.present_metal_texture(raw(&*target), Some((raw(&*event), 1)))
                }
                .map_err(err)?;
                let release = surface.release_event().map_err(err)?;
                if release.wait(released, Duration::from_secs(2)) {
                    Ok(())
                } else {
                    Err("the release event did not reach the returned value".into())
                }
            }
            "iosurface" | "mach-port" => {
                // What another device or process does: render into an IOSurface of its own.
                let device = MTLCreateSystemDefaultDevice().ok_or("no Metal device")?;
                let buffer = pixel_buffer(w, h, kCVPixelFormatType_32BGRA)?;
                let iosurface = CVPixelBufferGetIOSurface(Some(&buffer)).ok_or("no IOSurface")?;
                let target = texture(&device, w, h, Some(&iosurface))?;
                if mode == "mach-port" {
                    clear(&device, &target, color, None)?.waitUntilCompleted();
                    let port = iosurface.create_mach_port();
                    let shared = SharedTexture::from_mach_port(port);
                    unsafe {
                        mach2::mach_port::mach_port_deallocate(mach2::traps::mach_task_self(), port)
                    };
                    return surface.present_shared(&shared.map_err(err)?).map_err(err);
                }
                let event = device.newSharedEvent().ok_or("no event")?;
                clear(&device, &target, color, Some((&event, 1)))?;
                let shared = unsafe { SharedTexture::from_iosurface(raw(&*iosurface)) }
                    .and_then(|t| unsafe { t.with_shared_event(raw(&*event)) })
                    .map_err(err)?;
                surface.present_shared_fenced(&shared, 1, 2).map_err(err)?;
                reached(&event, 2)
            }
            "cv-bgra" => {
                let buffer = pixel_buffer(w, h, kCVPixelFormatType_32BGRA)?;
                fill(&buffer, 0, &color);
                unsafe { surface.present_cv_pixel_buffer(raw(&*buffer)) }.map_err(err)
            }
            _ => {
                let buffer = pixel_buffer(w, h, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)?;
                let [y, cb, cr] = ycbcr(color);
                fill(&buffer, 0, &[y]);
                fill(&buffer, 1, &[cb, cr]);
                unsafe { surface.present_cv_pixel_buffer(raw(&*buffer)) }.map_err(err)
            }
        }
    }

    /// Renders at the element's painted size, recreating its texture after a resize or a device
    /// change, and reuses it once the surface released it.
    pub fn animate(surface: SurfaceHandle) {
        let start = Instant::now();
        let mut held: Option<(
            (u32, u32),
            Retained<Device>,
            Retained<Texture>,
            Retained<Event>,
        )> = None;
        let mut value = 0;
        loop {
            std::thread::sleep(Duration::from_millis(16));
            let info = surface.info();
            if !info.visible || info.width == 0 {
                continue;
            }
            let size = (info.width, info.height);
            if held.as_ref().is_none_or(|(s, ..)| *s != size) {
                held = renderer(&surface).ok().and_then(|device| {
                    let target = texture(&device, size.0, size.1, None).ok()?;
                    let event = device.newSharedEvent()?;
                    Some((size, device, target, event))
                });
                value = 0;
            }
            let Some((_, device, target, event)) = held.as_ref() else {
                continue;
            };
            let t = start.elapsed().as_secs_f32();
            let c = |k: f32| ((0.5 + 0.5 * (t * k).sin()) * 255.0) as u8;
            value += 1;
            if clear(
                device,
                target,
                [c(0.7), c(1.3), c(1.0), 255],
                Some((event, value)),
            )
            .is_err()
            {
                continue;
            }
            let presented = unsafe {
                surface.present_metal_texture(raw(&**target), Some((raw(&**event), value)))
            };
            match presented {
                Ok(released) => {
                    if let Ok(release) = surface.release_event() {
                        release.wait(released, Duration::from_millis(100));
                    }
                }
                Err(SurfaceError::DeviceChanged | SurfaceError::WrongDevice) => held = None,
                Err(_) => {}
            }
        }
    }
}
