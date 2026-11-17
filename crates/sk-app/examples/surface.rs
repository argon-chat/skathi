//! An embedder that shows native frames in `<sk-surface>` elements. The app asks for them through
//! `appHost.call`:
//!
//! - `present(name, mode, [b, g, r, a], width, height)` presents one solid frame from a thread of
//!   ours. `mode` is `"bgra"` (CPU), `"texture"` (a texture on the renderer's device) or `"shared"`
//!   (a texture on a device of our own, handed over as a shared NT handle with a keyed mutex),
//!   `"d3d11-fence"` (the same with a shared `ID3D11Fence`), `"d3d12"` (a resource on the
//!   renderer's D3D12 device) or `"d3d12-shared"` (a D3D12 device of our own, shared resource and
//!   fence).
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
    let ui = builder.spawn().unwrap_or_else(|e| {
        eprintln!("surface: {e}");
        std::process::exit(2)
    });
    let exit = ui.wait();
    std::thread::sleep(Duration::from_millis(10));
    std::process::exit(if exit.clean { 0 } else { 1 });
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

#[cfg(not(windows))]
mod gpu {
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

    pub fn animate(_: SurfaceHandle) {}
}
