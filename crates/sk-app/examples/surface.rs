//! An embedder that shows native frames in `<sk-surface>` elements. The app asks for them through
//! `appHost.call`:
//!
//! - `present(name, mode, [b, g, r, a], width, height)` presents one solid frame from a thread of
//!   ours. `mode` is `"bgra"` (CPU), `"texture"` (a texture on the renderer's device) or `"shared"`
//!   (a texture on a device of our own, handed over as a shared NT handle with a keyed mutex).
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
    use windows::Win32::Foundation::{CloseHandle, HMODULE};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
        ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
        D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        IDXGIKeyedMutex, IDXGIResource1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
    };

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
        let (device, context): (ID3D11Device, ID3D11DeviceContext) =
            (device.ok_or("no device")?, context.ok_or("no context")?);
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

    pub fn animate(_: SurfaceHandle) {}
}
