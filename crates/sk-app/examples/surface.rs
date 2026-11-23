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
//!
//!   A mode ending in `-rgba16f` (and `"cv-rgha"`) sets the surface to `SurfaceFormat::Rgba16Float`
//!   and presents half floats instead: the colour is then linear `[r, g, b, a]`, and may go
//!   above 1.0. `"cpu-rgba16f"` is the CPU path.
//! - `animate(name)` clears a texture to a moving colour at 60 Hz, at the element's painted size.
//! - `layerLog()` answers and clears what the `<sk-layer name="probe">` layer was told:
//!   `attach:<window>`, `place:x,y,w,h` in CSS px, `hide` and `presented`.
//! - `<sk-layer name="plasma">` (Windows) shows a layer that draws: a DirectComposition visual
//!   with a swap chain of its own, rendered with D3D11 at the display's rate.
//! - `stats(name)` answers `{ presented, shown, width, height, visible, dynamicRangeLimit }`.
//! - `feedbackStart(name, fps, paced, source, ring, width, height, pauseEvery)` and `feedbackStop(name)`: frames at `fps`
//!   (`0` runs unthrottled, `-1` presents once per measured refresh) with `on_frame` listening.
//!   The stop prints each frame's event and the cadence, and answers a summary.
//!   An optional positive `pauseEvery` inserts 250 ms pauses that many frames apart and reports
//!   exact present-to-shown times for the first frame after each pause.
//!   `source` is `"bgra"` (CPU frames, the default), `"d3d12"` or
//!   `"d3d12-rgba16f"` (a ring of `ring` textures on the renderer's D3D12 device, each presented
//!   behind its fence). With `"d3d12-shared"` or `"d3d12-shared-rgba16f"` the ring belongs to a
//!   separate device and uses a shared fence per texture. D3D12 frames default to 64×64.
//!   `"shared"` is a player with a device of its own: a shared texture and fence, as libmpv's.
//! - `removeDevice(name)` removes the renderer's D3D12 device, as a driver reset does, and answers
//!   when (`monotonic_ns`).
//! - `phaseStart(name)` and `phaseStop(name)`: where in the refresh the window's tick writes the
//!   surface's frames, from a thread that watches `stats().shown` against the display clock; the
//!   stop answers `{ count, p05, p50, p95 }` in refreshes.
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

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use sk_app::embed::{
    Builder, Call, Layer, LayerHost, LayerPlacement, SurfaceFormat, SurfaceHandle,
};

static LAYER_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct ProbeLayer;

impl Layer for ProbeLayer {
    fn attach(&mut self, host: &LayerHost) {
        LAYER_LOG
            .lock()
            .unwrap()
            .push(format!("attach:{}", host.window));
    }

    fn place(&mut self, placement: Option<LayerPlacement>) {
        let told = match placement {
            Some(p) => {
                let css = p.bounds.map(|v| (v / p.scale).round().to_string());
                format!("place:{}", css.join(","))
            }
            None => "hide".into(),
        };
        LAYER_LOG.lock().unwrap().push(told);
    }

    fn presented(&mut self) {
        LAYER_LOG.lock().unwrap().push("presented".into());
    }
}

/// What a present fills its frame with.
#[derive(Clone, Copy)]
enum Fill {
    /// `[b, g, r, a]`, 8-bit sRGB.
    Bgra([u8; 4]),
    /// `[r, g, b, a]`, linear half floats on an `Rgba16Float` surface.
    Half([f32; 4]),
}

impl Fill {
    /// The colour as a render-target clear takes it: `[r, g, b, a]`.
    #[allow(dead_code)]
    fn clear(self) -> [f32; 4] {
        match self {
            Fill::Bgra(c) => {
                let [b, g, r, a] = c.map(|c| c as f32 / 255.0);
                [r, g, b, a]
            }
            Fill::Half(c) => c,
        }
    }

    #[allow(dead_code)]
    fn half(self) -> bool {
        matches!(self, Fill::Half(_))
    }
}

/// `f32` to IEEE binary16 bits, rounding to nearest; normal values only.
fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 112;
    if value == 0.0 || exponent <= 0 {
        return sign;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    (sign | ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16) + ((bits >> 12) & 1) as u16
}

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
        .setup(|host| {
            host.layer("probe", || Box::new(ProbeLayer));
            #[cfg(windows)]
            host.layer("plasma", || Box::new(plasma::Plasma::new()));
            Ok(())
        })
        .function("layerLog", |call| {
            let told = std::mem::take(&mut *LAYER_LOG.lock().unwrap());
            call.resolve(json!(told).to_string());
        })
        .function("present", |call| {
            let args = args_of(&call);
            let surface = surface_of(&args);
            let mode = args.get(1).and_then(Value::as_str).unwrap_or("bgra");
            let halves = mode.ends_with("-rgba16f") || mode == "cv-rgha";
            let mode = mode.trim_end_matches("-rgba16f").to_string();
            let channel = |i: usize| {
                let v = args.get(2).and_then(|c| c.get(i)).and_then(Value::as_f64);
                v.unwrap_or(if i == 3 { 255.0 } else { 0.0 })
            };
            let color = if halves {
                Fill::Half([0, 1, 2, 3].map(|i| channel(i) as f32))
            } else {
                Fill::Bgra([0, 1, 2, 3].map(|i| channel(i) as u8))
            };
            surface.set_format(if halves {
                SurfaceFormat::Rgba16Float
            } else {
                SurfaceFormat::Bgra8
            });
            let width = args.get(3).and_then(Value::as_u64).unwrap_or(64) as u32;
            let height = args.get(4).and_then(Value::as_u64).unwrap_or(64) as u32;
            let reply = call.into_reply();
            std::thread::spawn(move || {
                let presented = match (mode.as_str(), color) {
                    ("pixels", _) => serde_json::from_value::<Vec<u8>>(
                        args.get(2).cloned().unwrap_or(Value::Null),
                    )
                    .map_err(|e| e.to_string())
                    .and_then(|pixels| {
                        surface
                            .present_bgra(&pixels, width, height, width as usize * 4)
                            .map(drop)
                            .map_err(|e| e.to_string())
                    }),
                    ("texture", _) => gpu::present_texture(&surface, color, width, height),
                    ("shared", _) => gpu::present_shared(&surface, color, width, height),
                    ("d3d11-fence", _) => gpu::present_d3d11_fenced(&surface, color, width, height),
                    ("d3d12", _) => gpu::d3d12::present(&surface, color, width, height),
                    ("d3d12-shared", _) => {
                        gpu::d3d12::present_shared(&surface, color, width, height)
                    }
                    (
                        "metal" | "iosurface" | "mach-port" | "cv-bgra" | "cv-nv12" | "cv-rgha",
                        _,
                    ) => metal::present(&surface, &mode, color, width, height),
                    (_, Fill::Half(rgba)) => {
                        let frame = rgba.map(half).repeat((width * height) as usize);
                        surface
                            .present_rgba16f(&frame, width, height, width as usize * 4)
                            .map(drop)
                            .map_err(|e| e.to_string())
                    }
                    (_, Fill::Bgra(bgra)) => {
                        let frame = bgra.repeat((width * height) as usize);
                        surface
                            .present_bgra(&frame, width, height, width as usize * 4)
                            .map(drop)
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
        .function("feedbackStart", |call| {
            let args = args_of(&call);
            let fps = args.get(1).and_then(Value::as_f64).unwrap_or(24.0);
            let paced = args.get(2).and_then(Value::as_bool).unwrap_or(false);
            let source = args
                .get(3)
                .and_then(Value::as_str)
                .unwrap_or("bgra")
                .to_string();
            let ring = args.get(4).and_then(Value::as_u64).unwrap_or(3) as usize;
            let width = args.get(5).and_then(Value::as_u64).unwrap_or(64) as u32;
            let height = args.get(6).and_then(Value::as_u64).unwrap_or(64) as u32;
            let pause_every = args.get(7).and_then(Value::as_u64).unwrap_or(0);
            feedback::start(
                surface_of(&args),
                fps,
                paced,
                source,
                ring,
                (width, height),
                pause_every,
            );
            call.resolve("true");
        })
        .function("phaseStart", |call| {
            phase::start(surface_of(&args_of(&call)));
            call.resolve("true");
        })
        .function("phaseStop", |call| {
            let name = args_of(&call)
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let reply = call.into_reply();
            std::thread::spawn(move || reply.resolve(&phase::stop(&name).to_string()));
        })
        .function("feedbackStop", |call| {
            let name = args_of(&call)
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let reply = call.into_reply();
            std::thread::spawn(move || reply.resolve(&feedback::stop(&name).to_string()));
        })
        .function("clear", |call| {
            surface_of(&args_of(&call)).clear();
            call.resolve("true");
        })
        .function("removeDevice", |call| {
            let surface = surface_of(&args_of(&call));
            let reply = call.into_reply();
            std::thread::spawn(move || match gpu::d3d12::remove_device(&surface) {
                Ok(at) => reply.resolve(&at.to_string()),
                Err(e) => reply.reject(e),
            });
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
                    "dynamicRangeLimit": info.dynamic_range_limit.to_css(),
                    "contentHeadroom": info.content_headroom(),
                    "display": {
                        "known": info.display.known,
                        "hdr": info.display.hdr,
                        "headroom": info.display.headroom,
                        "potentialHeadroom": info.display.potential_headroom,
                        "outputHeadroom": info.display.output_headroom,
                        "outputHdr": info.display.output_hdr,
                        "refreshHz": info.display.refresh_hz,
                    },
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
mod plasma {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Instant;

    use sk_app::embed::{Layer, LayerHost, LayerPlacement};
    use windows::core::{s, Interface, Result, PCSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE};
    use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
    use windows::Win32::Graphics::Direct3D::{
        ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_HARDWARE,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
        ID3D11RenderTargetView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
        D3D11_BUFFER_DESC, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
        D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
    };
    use windows::Win32::Graphics::DirectComposition::{
        IDCompositionDevice, IDCompositionEffectGroup, IDCompositionRectangleClip,
        IDCompositionScaleTransform, IDCompositionVisual,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1, IDXGISwapChain2, DXGI_PRESENT,
        DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
        DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        DXGI_USAGE_RENDER_TARGET_OUTPUT,
    };
    use windows::Win32::System::Threading::WaitForSingleObject;

    /// The swap chain keeps this size and the visual scales it to the element, so a move or a
    /// resize reaches the screen with the page's frame instead of waiting for a redraw.
    const SIZE: [u32; 2] = [1280, 800];
    /// Seconds before the picture repeats.
    const PERIOD: f32 = 4.0;

    const SHADER: &str = r#"
cbuffer Frame : register(b0) { float phase; float aspect; float2 size; };

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}

float4 ps(float4 at : SV_Position) : SV_Target {
    float2 uv = at.xy / size;
    float2 p = float2((uv.x - 0.5) * aspect, uv.y - 0.5) * 3.2;
    float t = phase * 6.2831853;
    float v = sin(p.x * 1.7 + t) + sin(p.y * 2.3 - t)
            + sin((p.x + p.y) * 1.3 + 2.0 * t)
            + sin(length(p + 1.2 * float2(sin(t), cos(t))) * 3.0 - t);
    float k = saturate(v * 0.125 + 0.5);
    float3 deep = float3(0.13, 0.08, 0.24), lilac = float3(0.62, 0.45, 0.93);
    float3 pink = float3(0.96, 0.52, 0.76), blue = float3(0.50, 0.76, 0.98);
    float3 c = lerp(deep, lilac, smoothstep(0.15, 0.45, k));
    c = lerp(c, pink, smoothstep(0.45, 0.7, k));
    c = lerp(c, blue, smoothstep(0.72, 0.95, k));
    for (int i = 0; i < 5; i++) {
        float y = 0.22 * sin(p.x * 0.9 + t + i * 1.3) + 0.12 * sin(p.x * 2.1 - 2.0 * t + i);
        float d = abs(p.y - (i - 2) * 0.32 - y);
        c += (0.9 - i * 0.12) * smoothstep(0.035, 0.0, d) * float3(1.0, 0.92, 0.98);
        c += 0.18 * smoothstep(0.25, 0.0, d) * pink;
    }
    return float4(c, 1);
}
"#;

    #[derive(Default)]
    struct Shared {
        stop: AtomicBool,
        shown: AtomicBool,
        aspect: AtomicU32,
    }

    pub struct Plasma {
        swap_chain: Option<IDXGISwapChain1>,
        visuals: Option<Visuals>,
        shared: Arc<Shared>,
        thread: Option<JoinHandle<()>>,
    }

    impl Plasma {
        pub fn new() -> Self {
            let shared = Arc::<Shared>::default();
            match Renderer::new() {
                Ok(renderer) => {
                    let swap_chain = renderer.swap_chain.clone();
                    let state = shared.clone();
                    let thread = std::thread::spawn(move || {
                        if let Err(e) = renderer.run(&state) {
                            eprintln!("plasma: {e}");
                        }
                    });
                    Self {
                        swap_chain: Some(swap_chain),
                        visuals: None,
                        shared,
                        thread: Some(thread),
                    }
                }
                Err(e) => {
                    eprintln!("plasma: no device: {e}");
                    Self {
                        swap_chain: None,
                        visuals: None,
                        shared,
                        thread: None,
                    }
                }
            }
        }
    }

    impl Layer for Plasma {
        fn attach(&mut self, host: &LayerHost) {
            if let Some(old) = self.visuals.take() {
                old.remove();
            }
            let (Some(composition), Some(swap_chain)) = (host.composition, &self.swap_chain) else {
                return;
            };
            // SAFETY: Skathi hands over live interfaces for this call; cloning keeps them.
            let (device, underlay) = unsafe {
                (
                    IDCompositionDevice::from_raw_borrowed(&composition.device.as_ptr())
                        .unwrap()
                        .clone(),
                    IDCompositionVisual::from_raw_borrowed(&composition.underlay.as_ptr())
                        .unwrap()
                        .clone(),
                )
            };
            match Visuals::new(device, underlay, swap_chain) {
                Ok(visuals) => self.visuals = Some(visuals),
                Err(e) => eprintln!("plasma: {e}"),
            }
        }

        fn place(&mut self, placement: Option<LayerPlacement>) {
            self.shared
                .shown
                .store(placement.is_some(), Ordering::Relaxed);
            if let Some(p) = placement {
                let aspect = p.bounds[2] / p.bounds[3].max(1.0);
                self.shared
                    .aspect
                    .store(aspect.to_bits(), Ordering::Relaxed);
            }
            if let Some(visuals) = self.visuals.as_mut() {
                if let Err(e) = visuals.place(placement) {
                    eprintln!("plasma: {e}");
                }
            }
        }
    }

    impl Drop for Plasma {
        fn drop(&mut self) {
            self.shared.stop.store(true, Ordering::Relaxed);
            if let Some(visuals) = self.visuals.take() {
                visuals.remove();
                // Not from `attach` or `place`: nothing of Skathi's commits a layer that is gone.
                let _ = unsafe { visuals.device.Commit() };
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// On the window's device: `root` takes the clip of the element's ancestors, `frame` sits at
    /// the element's box with its rounded corners and opacity, and `content` scales the swap chain
    /// to that box.
    struct Visuals {
        device: IDCompositionDevice,
        underlay: IDCompositionVisual,
        root: IDCompositionVisual,
        clip: IDCompositionRectangleClip,
        frame: IDCompositionVisual,
        rounded: IDCompositionRectangleClip,
        fade: IDCompositionEffectGroup,
        scale: IDCompositionScaleTransform,
        added: bool,
    }

    impl Visuals {
        fn new(
            device: IDCompositionDevice,
            underlay: IDCompositionVisual,
            swap_chain: &IDXGISwapChain1,
        ) -> Result<Self> {
            unsafe {
                let root = device.CreateVisual()?;
                let clip = device.CreateRectangleClip()?;
                root.SetClip(&clip)?;
                let frame = device.CreateVisual()?;
                let rounded = device.CreateRectangleClip()?;
                frame.SetClip(&rounded)?;
                let fade = device.CreateEffectGroup()?;
                frame.SetEffect(&fade)?;
                let content = device.CreateVisual()?;
                let scale = device.CreateScaleTransform()?;
                content.SetTransform(&scale)?;
                content.SetContent(swap_chain)?;
                frame.AddVisual(&content, true, None::<&IDCompositionVisual>)?;
                root.AddVisual(&frame, true, None::<&IDCompositionVisual>)?;
                Ok(Self {
                    device,
                    underlay,
                    root,
                    clip,
                    frame,
                    rounded,
                    fade,
                    scale,
                    added: false,
                })
            }
        }

        fn place(&mut self, placement: Option<LayerPlacement>) -> Result<()> {
            let Some(p) = placement else {
                if self.added {
                    unsafe { self.underlay.RemoveVisual(&self.root)? };
                    self.added = false;
                }
                return Ok(());
            };
            let [x, y, w, h] = p.bounds;
            let [cx, cy, cw, ch] = p.clip;
            let [tl, tr, br, bl] = p.radii;
            unsafe {
                self.clip.SetLeft2(cx)?;
                self.clip.SetTop2(cy)?;
                self.clip.SetRight2(cx + cw)?;
                self.clip.SetBottom2(cy + ch)?;
                self.frame.SetOffsetX2(x)?;
                self.frame.SetOffsetY2(y)?;
                self.rounded.SetLeft2(0.0)?;
                self.rounded.SetTop2(0.0)?;
                self.rounded.SetRight2(w)?;
                self.rounded.SetBottom2(h)?;
                self.rounded.SetTopLeftRadiusX2(tl)?;
                self.rounded.SetTopLeftRadiusY2(tl)?;
                self.rounded.SetTopRightRadiusX2(tr)?;
                self.rounded.SetTopRightRadiusY2(tr)?;
                self.rounded.SetBottomRightRadiusX2(br)?;
                self.rounded.SetBottomRightRadiusY2(br)?;
                self.rounded.SetBottomLeftRadiusX2(bl)?;
                self.rounded.SetBottomLeftRadiusY2(bl)?;
                self.fade.SetOpacity2(p.opacity)?;
                self.scale.SetScaleX2(w / SIZE[0] as f32)?;
                self.scale.SetScaleY2(h / SIZE[1] as f32)?;
                if !self.added {
                    self.underlay
                        .AddVisual(&self.root, true, None::<&IDCompositionVisual>)?;
                    self.added = true;
                }
            }
            Ok(())
        }

        fn remove(&self) {
            if self.added {
                let _ = unsafe { self.underlay.RemoveVisual(&self.root) };
            }
        }
    }

    /// A device of our own and a swap chain for composition, drawn on a thread of ours.
    struct Renderer {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        swap_chain: IDXGISwapChain1,
    }

    impl Renderer {
        fn new() -> Result<Self> {
            unsafe {
                let mut device = None;
                let mut context = None;
                D3D11CreateDevice(
                    None::<&IDXGIAdapter>,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )?;
                let device: ID3D11Device = device.unwrap();
                let factory: IDXGIFactory2 =
                    device.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
                let desc = DXGI_SWAP_CHAIN_DESC1 {
                    Width: SIZE[0],
                    Height: SIZE[1],
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                    Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                    ..Default::default()
                };
                let swap_chain = factory.CreateSwapChainForComposition(&device, &desc, None)?;
                Ok(Self {
                    device,
                    context: context.unwrap(),
                    swap_chain,
                })
            }
        }

        fn run(self, shared: &Shared) -> Result<()> {
            unsafe {
                let vs_code = compile(s!("vs"), s!("vs_5_0"))?;
                let ps_code = compile(s!("ps"), s!("ps_5_0"))?;
                let mut vs: Option<ID3D11VertexShader> = None;
                self.device
                    .CreateVertexShader(bytes(&vs_code), None, Some(&mut vs))?;
                let mut ps: Option<ID3D11PixelShader> = None;
                self.device
                    .CreatePixelShader(bytes(&ps_code), None, Some(&mut ps))?;
                let mut constants: Option<ID3D11Buffer> = None;
                let desc = D3D11_BUFFER_DESC {
                    ByteWidth: 16,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                };
                self.device
                    .CreateBuffer(&desc, None, Some(&mut constants))?;
                let back: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
                let mut view: Option<ID3D11RenderTargetView> = None;
                self.device
                    .CreateRenderTargetView(&back, None, Some(&mut view))?;

                let chain: IDXGISwapChain2 = self.swap_chain.cast()?;
                chain.SetMaximumFrameLatency(1)?;
                let waitable = Waitable(chain.GetFrameLatencyWaitableObject());

                let ctx = &self.context;
                ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                ctx.VSSetShader(vs.as_ref(), None);
                ctx.PSSetShader(ps.as_ref(), None);
                ctx.PSSetConstantBuffers(0, Some(&[constants.clone()]));
                ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    Width: SIZE[0] as f32,
                    Height: SIZE[1] as f32,
                    MaxDepth: 1.0,
                    ..Default::default()
                }]));
                let start = Instant::now();
                while !shared.stop.load(Ordering::Relaxed) {
                    WaitForSingleObject(waitable.0, 100);
                    if !shared.shown.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(30));
                        continue;
                    }
                    let aspect = match f32::from_bits(shared.aspect.load(Ordering::Relaxed)) {
                        a if a > 0.0 => a,
                        _ => SIZE[0] as f32 / SIZE[1] as f32,
                    };
                    let phase = (start.elapsed().as_secs_f32() / PERIOD).fract();
                    let frame = [phase, aspect, SIZE[0] as f32, SIZE[1] as f32];
                    ctx.UpdateSubresource(
                        constants.as_ref().unwrap(),
                        0,
                        None,
                        frame.as_ptr().cast(),
                        0,
                        0,
                    );
                    // Flip-model presents unbind the target.
                    ctx.OMSetRenderTargets(Some(&[view.clone()]), None);
                    ctx.Draw(3, 0);
                    self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
                }
            }
            Ok(())
        }
    }

    struct Waitable(HANDLE);

    impl Drop for Waitable {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
        let mut code = None;
        let mut errors = None;
        let compiled = unsafe {
            D3DCompile(
                SHADER.as_ptr().cast(),
                SHADER.len(),
                s!("plasma.hlsl"),
                None,
                None,
                entry,
                target,
                0,
                0,
                &mut code,
                Some(&mut errors),
            )
        };
        if let (Err(e), Some(errors)) = (&compiled, &errors) {
            eprintln!(
                "plasma: {}",
                String::from_utf8_lossy(unsafe { bytes(errors) })
            );
            return Err(e.clone());
        }
        compiled?;
        Ok(code.unwrap())
    }

    unsafe fn bytes(blob: &ID3DBlob) -> &[u8] {
        unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
    }
}

mod phase {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;

    use serde_json::{json, Value};
    use sk_app::embed::{monotonic_ns, SurfaceHandle};

    struct Probe {
        stop: Arc<AtomicBool>,
        thread: JoinHandle<Vec<f64>>,
    }

    fn probes() -> &'static Mutex<HashMap<String, Probe>> {
        static PROBES: OnceLock<Mutex<HashMap<String, Probe>>> = OnceLock::new();
        PROBES.get_or_init(Default::default)
    }

    pub fn start(surface: SurfaceHandle) {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let name = surface.name().to_string();
        let thread = std::thread::spawn(move || {
            let mut phases = Vec::new();
            let mut last = surface.stats().shown;
            while !stopping.load(Ordering::Relaxed) {
                let shown = surface.stats().shown;
                if shown != last {
                    let at = monotonic_ns();
                    let display = surface.info().display;
                    if let (Some(vblank), Some(period)) =
                        (display.last_vblank, display.refresh_period)
                    {
                        let since = at.saturating_sub(vblank) as f64;
                        phases.push((since / period.as_nanos() as f64).fract());
                    }
                    last = shown;
                }
                std::thread::yield_now();
            }
            phases
        });
        probes()
            .lock()
            .unwrap()
            .insert(name, Probe { stop, thread });
    }

    pub fn stop(name: &str) -> Value {
        let Some(probe) = probes().lock().unwrap().remove(name) else {
            return Value::Null;
        };
        probe.stop.store(true, Ordering::Relaxed);
        let mut phases = probe.thread.join().unwrap_or_default();
        // Phases wrap at the vblank: centre them on their circular mean before taking quantiles.
        let tau = std::f64::consts::TAU;
        let (sin, cos) = phases.iter().fold((0.0, 0.0), |(s, c), p| {
            (s + (p * tau).sin(), c + (p * tau).cos())
        });
        let mean = f64::atan2(sin, cos) / tau;
        for p in &mut phases {
            *p = mean + (*p - mean + 0.5).rem_euclid(1.0) - 0.5;
        }
        phases.sort_by(f64::total_cmp);
        let at = |q: f64| {
            let i = ((phases.len() as f64 - 1.0) * q).round() as usize;
            phases.get(i).copied().unwrap_or(f64::NAN)
        };
        json!({ "count": phases.len(), "p05": at(0.05), "p50": at(0.5), "p95": at(0.95) })
    }
}

/// `feedbackStart(name, fps)` presents CPU frames at `fps` from a thread of ours and records what
/// became of each; `feedbackStop(name)` stops it, waits for the last frames to resolve, prints
/// every event and the cadence (refreshes each frame was held), and answers a summary.
mod feedback {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use serde_json::{json, Value};
    use sk_app::embed::{
        monotonic_ns, FrameDropReason, FrameEvent, FramePrecision, FrameWatch, SurfaceError,
        SurfaceEvent, SurfaceFormat, SurfaceHandle, SurfaceWatch,
    };

    /// What the producer thread answers.
    #[derive(Default)]
    struct Produced {
        ids: Vec<u64>,
        resumed: Vec<u64>,
        /// `info().visible` as each of them was presented, and when.
        visible: Vec<(bool, u64)>,
        /// When each present began, and whether it was taken.
        attempts: Vec<(u64, bool)>,
        /// Presents refused `Busy`.
        busy: usize,
        /// Why it could not start.
        error: Option<String>,
    }

    fn failed(error: String) -> Produced {
        Produced {
            error: Some(error),
            ..Default::default()
        }
    }

    struct Run {
        stop: Arc<AtomicBool>,
        producer: JoinHandle<Produced>,
        events: Arc<Mutex<Vec<FrameEvent>>>,
        /// When the first frame since the last one presented was reported hidden.
        hidden_at: Arc<AtomicU64>,
        surface: SurfaceHandle,
        shown: u64,
        /// When each `SurfaceEvent::DeviceLost` arrived.
        lost: Arc<Mutex<Vec<u64>>>,
        _watch: FrameWatch,
        _changes: SurfaceWatch,
    }

    fn runs() -> &'static Mutex<HashMap<String, Run>> {
        static RUNS: OnceLock<Mutex<HashMap<String, Run>>> = OnceLock::new();
        RUNS.get_or_init(Default::default)
    }

    /// With `paced`, each frame is presented a refresh and a half before the vblank its time
    /// falls on (the display clock in `info().display`), as a display-synced player would, so
    /// jitter in this thread's wakeups or the window's draw cannot move it to another refresh.
    pub fn start(
        surface: SurfaceHandle,
        fps: f64,
        paced: bool,
        source: String,
        ring: usize,
        size: (u32, u32),
        pause_every: u64,
    ) {
        let half = source.ends_with("-rgba16f");
        surface.set_format(if half {
            SurfaceFormat::Rgba16Float
        } else {
            SurfaceFormat::Bgra8
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let hidden_at = Arc::new(AtomicU64::new(0));
        let first_hidden = hidden_at.clone();
        let watch = surface.on_frame(move |e| {
            match e {
                FrameEvent::Presented { .. } => first_hidden.store(0, Ordering::Relaxed),
                FrameEvent::Dropped {
                    reason: FrameDropReason::Hidden,
                    ..
                } => {
                    let now = monotonic_ns();
                    let _ =
                        first_hidden.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
                }
                FrameEvent::Dropped { .. } => {}
            }
            sink.lock().unwrap().push(e)
        });
        let lost = Arc::new(Mutex::new(Vec::new()));
        let losing = lost.clone();
        let changes = surface.on_change(move |_, e| {
            if matches!(e, SurfaceEvent::DeviceLost) {
                losing.lock().unwrap().push(monotonic_ns());
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let name = surface.name().to_string();
        let shown = surface.stats().shown;
        let producing = surface.clone();
        let producer = std::thread::spawn(move || {
            let surface = producing;
            let mut present: Box<dyn FnMut(usize) -> Result<u64, SurfaceError>> =
                if source.starts_with("d3d12") {
                    #[cfg(windows)]
                    match super::gpu::d3d12::Ring::new(
                        &surface,
                        ring,
                        half,
                        source.starts_with("d3d12-shared"),
                        size,
                    ) {
                        Ok(mut ring) => {
                            let surface = surface.clone();
                            Box::new(move |n| ring.present(&surface, n))
                        }
                        Err(e) => return failed(e),
                    }
                    #[cfg(not(windows))]
                    return failed(format!(
                        "{ring} D3D12 textures of size {size:?} off Windows"
                    ));
                } else if source == "shared" {
                    #[cfg(windows)]
                    match super::gpu::Shared::new() {
                        Ok(mut shared) => {
                            let surface = surface.clone();
                            Box::new(move |n| shared.present(&surface, n))
                        }
                        Err(e) => return failed(e),
                    }
                    #[cfg(not(windows))]
                    return failed("shared D3D11 textures off Windows".into());
                } else {
                    let surface = surface.clone();
                    Box::new(move |n| {
                        let shade = (n * 37 % 256) as u8;
                        let frame = [shade, 255 - shade, 128, 255].repeat(64 * 64);
                        surface.present_bgra(&frame, 64, 64, 64 * 4)
                    })
                };
            let interval = if fps > 0.0 { (1e9 / fps) as u64 } else { 0 };
            let start = monotonic_ns();
            let mut produced = Produced::default();
            let mut last_began = None;
            for n in 0_u64.. {
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                let pauses = if fps > 0.0 {
                    n.checked_div(pause_every).unwrap_or(0)
                } else {
                    0
                };
                let due = start + n * interval + pauses * 250_000_000;
                let info = surface.info();
                let display = info.display;
                let at = match (display.last_vblank, display.refresh_period) {
                    (Some(vblank), Some(period)) if fps < 0.0 => {
                        let p = period.as_nanos() as u64;
                        let now = monotonic_ns();
                        let start = vblank + now.saturating_sub(vblank) / p * p;
                        let half = start + p / 2;
                        if now > half || last_began.is_some_and(|at| at >= start) {
                            half + p
                        } else {
                            half
                        }
                    }
                    _ if fps < 0.0 => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    (Some(vblank), Some(period)) if paced => {
                        let p = period.as_nanos() as u64;
                        let k = due.saturating_sub(vblank).div_ceil(p);
                        (vblank + k * p).saturating_sub(p + p / 2)
                    }
                    _ => due,
                };
                let wait = at.saturating_sub(monotonic_ns());
                if wait > 0 {
                    std::thread::sleep(Duration::from_nanos(wait));
                }
                let began = monotonic_ns();
                last_began = Some(began);
                let presented = present(n as usize);
                produced.attempts.push((began, presented.is_ok()));
                match presented {
                    Ok(id) => {
                        produced.ids.push(id);
                        if pauses > 0 && n % pause_every == 0 {
                            produced.resumed.push(id);
                        }
                        produced.visible.push((info.visible, monotonic_ns()));
                    }
                    Err(SurfaceError::Busy) => produced.busy += 1,
                    Err(_) => {}
                }
            }
            produced
        });
        let run = Run {
            stop,
            producer,
            events,
            hidden_at,
            surface,
            shown,
            lost,
            _watch: watch,
            _changes: changes,
        };
        if let Some(old) = runs().lock().unwrap().insert(name, run) {
            old.stop.store(true, Ordering::Release);
        }
    }

    pub fn stop(name: &str) -> Value {
        let Some(run) = runs().lock().unwrap().remove(name) else {
            return Value::Null;
        };
        run.stop.store(true, Ordering::Release);
        let produced = run.producer.join().unwrap_or_default();
        let ids = &produced.ids;
        let shown = run.surface.stats().shown - run.shown;
        let deadline = Instant::now() + Duration::from_secs(3);
        while run.events.lock().unwrap().len() < ids.len() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let events = run.events.lock().unwrap().clone();
        let mut summary = summary(name, ids, &events, &produced.resumed);
        // Frames presented while `info().visible` still said so, after the frames since the last
        // one shown began to drop as hidden.
        let hidden_at = run.hidden_at.load(Ordering::Relaxed);
        let visible_late = produced
            .visible
            .iter()
            .filter(|&&(visible, at)| hidden_at != 0 && at > hidden_at && visible)
            .count();
        summary["busy"] = produced.busy.into();
        summary["shown"] = shown.into();
        summary["error"] = produced.error.into();
        summary["visibleLate"] = visible_late.into();
        summary["lostAt"] = json!(*run.lost.lock().unwrap());
        let began: HashMap<u64, u64> = produced
            .attempts
            .iter()
            .filter(|(_, taken)| *taken)
            .map(|(at, _)| *at)
            .zip(ids)
            .map(|(at, id)| (*id, at))
            .collect();
        let hidden_at: Vec<u64> = events
            .iter()
            .filter_map(|e| match *e {
                FrameEvent::Dropped {
                    frame,
                    reason: FrameDropReason::Hidden,
                } => began.get(&frame).copied(),
                _ => None,
            })
            .collect();
        summary["hiddenAt"] = json!(hidden_at);
        let pause_to_shown: Vec<_> = events
            .iter()
            .filter_map(|event| match *event {
                FrameEvent::Presented {
                    frame,
                    shown_at,
                    precision: FramePrecision::Exact,
                    ..
                } if produced.resumed.binary_search(&frame).is_ok() => {
                    Some(shown_at.saturating_sub(*began.get(&frame)?) as f64 / 1e6)
                }
                _ => None,
            })
            .collect();
        summary["pauseToShownMs"] = json!(pause_to_shown);
        summary["attempts"] = json!(produced.attempts);
        summary
    }

    fn summary(name: &str, ids: &[u64], events: &[FrameEvent], resumed: &[u64]) -> Value {
        let mut count: HashMap<&str, usize> = HashMap::new();
        let mut shown: Vec<(u64, u64, Duration)> = Vec::new();
        let mut exact = 0;
        for event in events {
            let line = match *event {
                FrameEvent::Presented {
                    frame,
                    shown_at,
                    refresh_interval,
                    precision,
                } => {
                    exact += usize::from(precision == FramePrecision::Exact);
                    shown.push((frame, shown_at, refresh_interval));
                    *count.entry("presented").or_default() += 1;
                    format!(
                        "frame {frame}: presented at {:.3} ms, refresh {:.3} ms, {precision:?}",
                        shown_at as f64 / 1e6,
                        refresh_interval.as_secs_f64() * 1e3
                    )
                }
                FrameEvent::Dropped { frame, reason } => {
                    let key = match reason {
                        FrameDropReason::Superseded => "superseded",
                        FrameDropReason::Hidden => "hidden",
                        FrameDropReason::DeviceLost => "deviceLost",
                        FrameDropReason::Closed => "closed",
                    };
                    *count.entry(key).or_default() += 1;
                    format!("frame {frame}: dropped, {reason:?}")
                }
            };
            println!("[feedback {name}] {line}");
        }
        // Refreshes each frame stayed on screen: the next one's shown_at minus its own.
        let cadence: Vec<u64> = shown
            .windows(2)
            .map(|w| {
                let (_, at, refresh) = w[0];
                let held = w[1].1.saturating_sub(at) as f64;
                (held / (refresh.as_nanos().max(1) as f64)).round() as u64
            })
            .collect();
        let line: Vec<String> = cadence.iter().map(u64::to_string).collect();
        let playback: Vec<_> = shown
            .windows(2)
            .zip(&cadence)
            .filter_map(|(pair, &held)| {
                let next_resume = resumed.partition_point(|&id| id <= pair[0].0);
                resumed
                    .get(next_resume)
                    .is_none_or(|&id| id > pair[1].0)
                    .then_some(held)
            })
            .collect();
        println!("[feedback {name}] cadence: {}", line.join(" "));
        let frames: Vec<u64> = events.iter().map(FrameEvent::frame).collect();
        let mut sorted = frames.clone();
        sorted.sort_unstable();
        sorted.dedup();
        json!({
            "presents": ids.len(),
            "events": events.len(),
            "inOrder": frames.windows(2).all(|w| w[0] < w[1]),
            "exactlyOnce": sorted == ids && frames.len() == ids.len(),
            "presented": count.get("presented").copied().unwrap_or(0),
            "superseded": count.get("superseded").copied().unwrap_or(0),
            "hidden": count.get("hidden").copied().unwrap_or(0),
            "deviceLost": count.get("deviceLost").copied().unwrap_or(0),
            "closed": count.get("closed").copied().unwrap_or(0),
            "exact": exact,
            "cadence": cadence,
            "playbackCadence": playback,
        })
    }
}

#[cfg(windows)]
mod gpu {
    use std::time::{Duration, Instant};

    use sk_app::embed::{SharedTexture, SurfaceError, SurfaceHandle};
    use windows::core::Interface;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE, WAIT_OBJECT_0};

    use super::Fill;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4,
        ID3D11Fence, ID3D11RenderTargetView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
        D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FENCE_FLAG_SHARED,
        D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
        D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIKeyedMutex, IDXGIResource1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    type Result<T> = std::result::Result<T, String>;

    fn err(e: impl std::fmt::Display) -> String {
        e.to_string()
    }

    fn format(fill: Fill) -> DXGI_FORMAT {
        if fill.half() {
            DXGI_FORMAT_R16G16B16A16_FLOAT
        } else {
            DXGI_FORMAT_B8G8R8A8_UNORM
        }
    }

    fn target(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        misc: u32,
        format: DXGI_FORMAT,
    ) -> Result<(ID3D11Texture2D, ID3D11RenderTargetView)> {
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

    pub fn present_texture(surface: &SurfaceHandle, color: Fill, w: u32, h: u32) -> Result<()> {
        let (device, context) = renderer(surface)?;
        let (texture, view) = target(&device, w, h, 0, format(color))?;
        unsafe {
            context.ClearRenderTargetView(&view, &color.clear());
            surface.present_d3d11_texture(texture.as_raw())
        }
        .map(drop)
        .map_err(err)
    }

    /// What another process would do: its own device, a shared NT handle, a keyed mutex.
    pub fn present_shared(surface: &SurfaceHandle, color: Fill, w: u32, h: u32) -> Result<()> {
        let (device, context) = own_device()?;
        let misc = (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0)
            as u32;
        let (texture, view) = target(&device, w, h, misc, format(color))?;
        let mutex: IDXGIKeyedMutex = texture.cast().map_err(err)?;
        unsafe {
            mutex.AcquireSync(0, 1000).map_err(err)?;
            context.ClearRenderTargetView(&view, &color.clear());
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
        color: Fill,
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
        let (texture, view) = target(&device, w, h, misc, format(color))?;
        unsafe {
            context.ClearRenderTargetView(&view, &color.clear());
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

    /// A player with a device of its own, as libmpv is: one shared texture and a shared fence. After
    /// the renderer's device went, a new fence, and the texture opened again with it.
    pub struct Shared {
        device5: ID3D11Device5,
        context: ID3D11DeviceContext,
        context4: ID3D11DeviceContext4,
        fence: ID3D11Fence,
        view: ID3D11RenderTargetView,
        _texture: ID3D11Texture2D,
        handles: (HANDLE, HANDLE),
        opened: Option<SharedTexture>,
        value: u64,
    }

    // SAFETY: D3D11 objects of a device used from this one thread only.
    unsafe impl Send for Shared {}

    impl Drop for Shared {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handles.0);
                let _ = CloseHandle(self.handles.1);
            }
        }
    }

    impl Shared {
        pub fn new() -> Result<Self> {
            let (device, context) = own_device()?;
            let device5: ID3D11Device5 = device.cast().map_err(err)?;
            let context4: ID3D11DeviceContext4 = context.cast().map_err(err)?;
            let (fence, fence_handle) = shared_fence(&device5)?;
            let misc =
                (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32;
            let (texture, view) = target(&device, 64, 64, misc, DXGI_FORMAT_B8G8R8A8_UNORM)?;
            let resource: IDXGIResource1 = texture.cast().map_err(err)?;
            let access = (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0;
            let texture_handle =
                unsafe { resource.CreateSharedHandle(None, access, None) }.map_err(err)?;
            Ok(Shared {
                device5,
                context,
                context4,
                fence,
                view,
                _texture: texture,
                handles: (texture_handle, fence_handle),
                opened: None,
                value: 0,
            })
        }

        pub fn present(
            &mut self,
            surface: &SurfaceHandle,
            n: usize,
        ) -> std::result::Result<u64, SurfaceError> {
            let shade = (n * 37 % 256) as f32 / 255.0;
            let platform = |e: windows::core::Error| SurfaceError::Platform(e.to_string());
            unsafe {
                self.context
                    .ClearRenderTargetView(&self.view, &[shade, 1.0 - shade, 0.5, 1.0]);
                self.context4
                    .Signal(&self.fence, self.value + 1)
                    .map_err(platform)?;
                self.context.Flush();
            }
            if self.opened.is_none() {
                let (texture, fence) = self.handles;
                self.opened = Some(
                    SharedTexture::open(texture.0 as usize as u64)
                        .and_then(|t| t.with_fence(fence.0 as usize as u64))?,
                );
            }
            let opened = self.opened.as_ref().expect("just opened");
            let presented = surface.present_shared_fenced(opened, self.value + 1, self.value + 2);
            self.value += 2;
            match presented {
                // A fence the removed device opened can read u64::MAX from then on.
                Err(SurfaceError::DeviceChanged | SurfaceError::NoDevice(_)) => {
                    if self.opened.take().is_some() {
                        let (fence, handle) =
                            shared_fence(&self.device5).map_err(SurfaceError::Platform)?;
                        unsafe {
                            let _ = CloseHandle(self.handles.1);
                        }
                        (self.fence, self.handles.1, self.value) = (fence, handle, 0);
                    }
                }
                // A removed renderer never signals the release value: a player times out and goes on.
                Ok(_) => {
                    self.reached(self.value, 100);
                }
                Err(_) => {}
            }
            presented
        }

        fn reached(&self, value: u64, ms: u32) -> bool {
            if unsafe { self.fence.GetCompletedValue() } >= value {
                return true;
            }
            let Ok(event) = (unsafe { CreateEventW(None, false, false, None) }) else {
                return false;
            };
            let reached = unsafe { self.fence.SetEventOnCompletion(value, event) }.is_ok()
                && unsafe { WaitForSingleObject(event, ms) } == WAIT_OBJECT_0;
            unsafe {
                let _ = CloseHandle(event);
            }
            reached
        }
    }

    fn shared_fence(device: &ID3D11Device5) -> Result<(ID3D11Fence, HANDLE)> {
        let mut fence: Option<ID3D11Fence> = None;
        unsafe { device.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence) }.map_err(err)?;
        let fence = fence.ok_or("no fence")?;
        let handle = unsafe { fence.CreateSharedHandle(None, GENERIC_ALL.0, None) }.map_err(err)?;
        Ok((fence, handle))
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
                held = target(device, size.0, size.1, 0, DXGI_FORMAT_B8G8R8A8_UNORM)
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
            ID3D12DescriptorHeap, ID3D12Device, ID3D12Device5, ID3D12DeviceChild, ID3D12Fence,
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
        use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory4};
        use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

        use super::{err, format, Fill, Result};

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

            fn texture(
                &self,
                w: u32,
                h: u32,
                heap: D3D12_HEAP_FLAGS,
                fill: Fill,
            ) -> Result<ID3D12Resource> {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: u64::from(w),
                    Height: h,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: format(fill),
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
                color: Fill,
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
                unsafe { list.ClearRenderTargetView(rtv, &color.clear(), None) };
                if after != D3D12_RESOURCE_STATE_RENDER_TARGET {
                    transition(&list, resource, D3D12_RESOURCE_STATE_RENDER_TARGET, after);
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

        /// Removes the renderer's D3D12 device, as a driver reset (TDR) does; answers when.
        pub fn remove_device(surface: &SurfaceHandle) -> Result<u64> {
            let renderer = surface.d3d12_device().map_err(err)?;
            let raw = renderer.as_raw();
            let device: ID3D12Device5 = unsafe { ID3D12Device::from_raw_borrowed(&raw) }
                .ok_or("null device")?
                .cast()
                .map_err(err)?;
            let at = sk_app::embed::monotonic_ns();
            unsafe { device.RemoveDevice() };
            Ok(at)
        }

        /// A producer on the renderer's device: its own queue, a GPU-side handoff, the release fence.
        pub fn present(surface: &SurfaceHandle, color: Fill, w: u32, h: u32) -> Result<()> {
            let renderer = surface.d3d12_device().map_err(err)?;
            let raw = renderer.as_raw();
            let device = unsafe { ID3D12Device::from_raw_borrowed(&raw) }
                .ok_or("null device")?
                .clone();
            let producer = Producer::new(device, false)?;
            let resource = producer.texture(w, h, D3D12_HEAP_FLAG_NONE, color)?;
            producer.clear(&resource, color, D3D12_RESOURCE_STATE_RENDER_TARGET, 1)?;
            let released = unsafe {
                surface.present_d3d12(
                    resource.as_raw(),
                    D3D12_RESOURCE_STATE_RENDER_TARGET.0,
                    Some((producer.fence.as_raw(), 1)),
                )
            }
            .map_err(err)?
            .release;
            let release = surface.release_fence().map_err(err)?;
            if release.wait(released, std::time::Duration::from_secs(2)) {
                Ok(())
            } else {
                Err("the release fence did not reach the returned value".into())
            }
        }

        struct RingTexture {
            texture: ID3D12Resource,
            allocator: ID3D12CommandAllocator,
            list: ID3D12GraphicsCommandList,
            heap: ID3D12DescriptorHeap,
            cleared: u64,
            released: u64,
            shared: Option<(SharedTexture, ID3D12Fence)>,
        }

        /// A video player's pipeline on the renderer's device: a ring of textures, each cleared on
        /// the producer's queue and presented behind its fence at once; a texture is reused once
        /// the surface has released it.
        pub struct Ring {
            producer: Producer,
            textures: Vec<RingTexture>,
            release: sk_app::embed::D3D12Fence,
            value: u64,
            half: bool,
        }

        impl Ring {
            pub fn new(
                surface: &SurfaceHandle,
                n: usize,
                half: bool,
                shared: bool,
                size: (u32, u32),
            ) -> Result<Self> {
                let renderer = surface.d3d12_device().map_err(err)?;
                let raw = renderer.as_raw();
                let renderer =
                    unsafe { ID3D12Device::from_raw_borrowed(&raw) }.ok_or("null device")?;
                let device = if shared {
                    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.map_err(err)?;
                    let adapter: IDXGIAdapter1 =
                        unsafe { factory.EnumAdapterByLuid(renderer.GetAdapterLuid()) }
                            .map_err(err)?;
                    let mut device = None;
                    unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) }
                        .map_err(err)?;
                    device.ok_or("no producer device")?
                } else {
                    renderer.clone()
                };
                let producer = Producer::new(device, false)?;
                let fill = if half {
                    Fill::Half([0.0; 4])
                } else {
                    Fill::Bgra([0; 4])
                };
                let mut textures = Vec::new();
                for _ in 0..n.max(1) {
                    let flags = if shared {
                        D3D12_HEAP_FLAG_SHARED
                    } else {
                        D3D12_HEAP_FLAG_NONE
                    };
                    let texture = producer.texture(size.0, size.1, flags, fill)?;
                    let handoff = if shared {
                        let fence =
                            unsafe { producer.device.CreateFence(0, D3D12_FENCE_FLAG_SHARED) }
                                .map_err(err)?;
                        let opened = open_shared(&producer.device, &texture, &fence)?;
                        Some((opened, fence))
                    } else {
                        None
                    };
                    let heap: ID3D12DescriptorHeap = unsafe {
                        producer
                            .device
                            .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                                NumDescriptors: 1,
                                ..Default::default()
                            })
                    }
                    .map_err(err)?;
                    unsafe {
                        producer.device.CreateRenderTargetView(
                            &texture,
                            None,
                            heap.GetCPUDescriptorHandleForHeapStart(),
                        )
                    };
                    let allocator: ID3D12CommandAllocator = unsafe {
                        producer
                            .device
                            .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                    }
                    .map_err(err)?;
                    let list: ID3D12GraphicsCommandList = unsafe {
                        producer.device.CreateCommandList(
                            0,
                            D3D12_COMMAND_LIST_TYPE_DIRECT,
                            &allocator,
                            None,
                        )
                    }
                    .map_err(err)?;
                    unsafe { list.Close() }.map_err(err)?;
                    textures.push(RingTexture {
                        texture,
                        allocator,
                        list,
                        heap,
                        cleared: 0,
                        released: 0,
                        shared: handoff,
                    });
                }
                Ok(Ring {
                    producer,
                    textures,
                    release: surface.release_fence().map_err(err)?,
                    value: 0,
                    half,
                })
            }

            /// Clears the `n`th frame's texture to a shade of its own and presents it.
            pub fn present(
                &mut self,
                surface: &SurfaceHandle,
                n: usize,
            ) -> std::result::Result<u64, sk_app::embed::SurfaceError> {
                use sk_app::embed::SurfaceError;
                let count = self.textures.len();
                let t = &mut self.textures[n % count];
                let timeout = std::time::Duration::from_secs(2);
                let reusable = if let Some((_, fence)) = &t.shared {
                    wait(fence, t.released.max(t.cleared)).is_ok()
                } else {
                    self.release.wait(t.released, timeout)
                        && wait(&self.producer.fence, t.cleared).is_ok()
                };
                if !reusable {
                    return Err(SurfaceError::Timeout);
                }
                let shade = (n * 37 % 256) as f32 / 255.0;
                let color = if self.half {
                    [4.0 * shade, 0.5, 1.0 - shade, 1.0]
                } else {
                    [shade, 1.0 - shade, 0.5, 1.0]
                };
                let platform = |e: windows::core::Error| SurfaceError::Platform(e.to_string());
                self.value += 1;
                let (fence, ready) = match &t.shared {
                    Some((_, fence)) => (fence, t.released.max(t.cleared) + 1),
                    None => (&self.producer.fence, self.value),
                };
                unsafe {
                    t.allocator.Reset().map_err(platform)?;
                    t.list.Reset(&t.allocator, None).map_err(platform)?;
                    let rtv = t.heap.GetCPUDescriptorHandleForHeapStart();
                    if t.shared.is_some() && t.cleared > 0 {
                        transition(
                            &t.list,
                            &t.texture,
                            D3D12_RESOURCE_STATE_COMMON,
                            D3D12_RESOURCE_STATE_RENDER_TARGET,
                        );
                    }
                    t.list.ClearRenderTargetView(rtv, &color, None);
                    if t.shared.is_some() {
                        transition(
                            &t.list,
                            &t.texture,
                            D3D12_RESOURCE_STATE_RENDER_TARGET,
                            D3D12_RESOURCE_STATE_COMMON,
                        );
                    }
                    t.list.Close().map_err(platform)?;
                    let list: ID3D12CommandList = t.list.cast().map_err(platform)?;
                    self.producer.queue.ExecuteCommandLists(&[Some(list)]);
                    self.producer.queue.Signal(fence, ready).map_err(platform)?;
                }
                t.cleared = ready;
                if let Some((opened, _)) = &t.shared {
                    let frame = surface.present_shared_fenced(opened, ready, ready + 1)?;
                    t.released = ready + 1;
                    return Ok(frame);
                }
                let presented = unsafe {
                    surface.present_d3d12(
                        t.texture.as_raw(),
                        D3D12_RESOURCE_STATE_RENDER_TARGET.0,
                        Some((self.producer.fence.as_raw(), self.value)),
                    )
                }?;
                t.released = presented.release;
                Ok(presented.frame)
            }
        }

        /// What a producer with its own D3D12 device does (a Vulkan one imports these two objects):
        /// share the resource and a fence.
        pub fn present_shared(surface: &SurfaceHandle, color: Fill, w: u32, h: u32) -> Result<()> {
            let mut device: Option<ID3D12Device> = None;
            unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_11_0, &mut device) }.map_err(err)?;
            let producer = Producer::new(device.ok_or("no device")?, true)?;
            let resource = producer.texture(w, h, D3D12_HEAP_FLAG_SHARED, color)?;
            producer.clear(&resource, color, D3D12_RESOURCE_STATE_COMMON, 1)?;
            let opened = open_shared(&producer.device, &resource, &producer.fence)?;
            surface.present_shared_fenced(&opened, 1, 2).map_err(err)?;
            wait(&producer.fence, 2)
        }

        fn open_shared(
            device: &ID3D12Device,
            resource: &ID3D12Resource,
            fence: &ID3D12Fence,
        ) -> Result<SharedTexture> {
            let share = |object: &ID3D12DeviceChild| unsafe {
                device.CreateSharedHandle(object, None, GENERIC_ALL.0, None)
            };
            let texture = share(&resource.cast().map_err(err)?).map_err(err)?;
            let pid = std::process::id();
            let opened = SharedTexture::open_from_process(pid, texture.0 as usize as u64);
            unsafe {
                let _ = CloseHandle(texture);
            }
            let opened = opened.map_err(err)?;
            let fence = share(&fence.cast().map_err(err)?).map_err(err)?;
            let opened = opened.with_fence_from_process(pid, fence.0 as usize as u64);
            unsafe {
                let _ = CloseHandle(fence);
            }
            opened.map_err(err)
        }

        fn transition(
            list: &ID3D12GraphicsCommandList,
            resource: &ID3D12Resource,
            before: D3D12_RESOURCE_STATES,
            after: D3D12_RESOURCE_STATES,
        ) {
            let mut barrier = D3D12_RESOURCE_BARRIER {
                Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                Anonymous: D3D12_RESOURCE_BARRIER_0 {
                    Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                        pResource: ManuallyDrop::new(Some(resource.clone())),
                        Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                        StateBefore: before,
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
    }
}

#[cfg(target_os = "macos")]
mod gpu {
    use sk_app::embed::SurfaceHandle;

    pub use super::metal::animate;
    use super::Fill;

    pub fn present_texture(s: &SurfaceHandle, c: Fill, w: u32, h: u32) -> Result<(), String> {
        super::metal::present(s, "metal", c, w, h)
    }

    pub fn present_shared(s: &SurfaceHandle, c: Fill, w: u32, h: u32) -> Result<(), String> {
        super::metal::present(s, "iosurface", c, w, h)
    }

    pub fn present_d3d11_fenced(s: &SurfaceHandle, _: Fill, _: u32, _: u32) -> Result<(), String> {
        s.d3d11_device().map(|_| ()).map_err(|e| e.to_string())
    }

    pub mod d3d12 {
        pub use super::present_d3d11_fenced as present;
        pub use super::present_d3d11_fenced as present_shared;

        pub fn remove_device(_: &sk_app::embed::SurfaceHandle) -> Result<u64, String> {
            Err("no D3D12 device here".into())
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod gpu {
    use std::time::{Duration, Instant};

    use sk_app::embed::SurfaceHandle;

    use super::Fill;

    pub fn present_texture(s: &SurfaceHandle, _: Fill, _: u32, _: u32) -> Result<(), String> {
        s.d3d11_device().map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn present_shared(s: &SurfaceHandle, c: Fill, w: u32, h: u32) -> Result<(), String> {
        present_texture(s, c, w, h)
    }

    pub fn present_d3d11_fenced(s: &SurfaceHandle, c: Fill, w: u32, h: u32) -> Result<(), String> {
        present_texture(s, c, w, h)
    }

    pub mod d3d12 {
        pub use super::{present_shared, present_texture as present};

        pub fn remove_device(_: &sk_app::embed::SurfaceHandle) -> Result<u64, String> {
            Err("no D3D12 device here".into())
        }
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

    use super::Fill;

    pub fn present(s: &SurfaceHandle, _: &str, _: Fill, _: u32, _: u32) -> Result<(), String> {
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
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_64RGBAHalf,
        kCVReturnSuccess, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane,
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

    use super::{half, Fill};

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
        fill: Fill,
        on: Option<&IOSurfaceRef>,
    ) -> Result<Retained<Texture>> {
        let desc = MTLTextureDescriptor::new();
        desc.setPixelFormat(if fill.half() {
            MTLPixelFormat::RGBA16Float
        } else {
            MTLPixelFormat::BGRA8Unorm
        });
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
        fill: Fill,
        signal: Option<(&Event, u64)>,
    ) -> Result<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let queue = device.newCommandQueue().ok_or("no queue")?;
        let command = queue.commandBuffer().ok_or("no command buffer")?;
        let pass = MTLRenderPassDescriptor::new();
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(target));
        color.setLoadAction(MTLLoadAction::Clear);
        color.setStoreAction(MTLStoreAction::Store);
        let [red, green, blue, alpha] = fill.clear().map(f64::from);
        color.setClearColor(MTLClearColor {
            red,
            green,
            blue,
            alpha,
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

    fn pixel_format(fill: Fill) -> u32 {
        if fill.half() {
            kCVPixelFormatType_64RGBAHalf
        } else {
            kCVPixelFormatType_32BGRA
        }
    }

    pub fn present(surface: &SurfaceHandle, mode: &str, color: Fill, w: u32, h: u32) -> Result<()> {
        match mode {
            "metal" => {
                // A producer on the renderer's device: its own queue, a GPU-side handoff.
                let device = renderer(surface)?;
                let event = device.newSharedEvent().ok_or("no event")?;
                let target = texture(&device, w, h, color, None)?;
                clear(&device, &target, color, Some((&event, 1)))?;
                let released = unsafe {
                    surface.present_metal_texture(raw(&*target), Some((raw(&*event), 1)))
                }
                .map_err(err)?
                .release;
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
                let buffer = pixel_buffer(w, h, pixel_format(color))?;
                let iosurface = CVPixelBufferGetIOSurface(Some(&buffer)).ok_or("no IOSurface")?;
                let target = texture(&device, w, h, color, Some(&iosurface))?;
                if mode == "mach-port" {
                    clear(&device, &target, color, None)?.waitUntilCompleted();
                    let port = iosurface.create_mach_port();
                    let shared = SharedTexture::from_mach_port(port);
                    unsafe {
                        mach2::mach_port::mach_port_deallocate(mach2::traps::mach_task_self(), port)
                    };
                    return surface
                        .present_shared(&shared.map_err(err)?)
                        .map(drop)
                        .map_err(err);
                }
                let event = device.newSharedEvent().ok_or("no event")?;
                clear(&device, &target, color, Some((&event, 1)))?;
                let shared = unsafe { SharedTexture::from_iosurface(raw(&*iosurface)) }
                    .and_then(|t| unsafe { t.with_shared_event(raw(&*event)) })
                    .map_err(err)?;
                surface.present_shared_fenced(&shared, 1, 2).map_err(err)?;
                reached(&event, 2)
            }
            "cv-bgra" | "cv-rgha" => {
                let buffer = pixel_buffer(w, h, pixel_format(color))?;
                let texel: Vec<u8> = match color {
                    Fill::Bgra(bgra) => bgra.to_vec(),
                    Fill::Half(rgba) => rgba.iter().flat_map(|c| half(*c).to_ne_bytes()).collect(),
                };
                fill(&buffer, 0, &texel);
                unsafe { surface.present_cv_pixel_buffer(raw(&*buffer)) }
                    .map(drop)
                    .map_err(err)
            }
            _ => {
                let Fill::Bgra(color) = color else {
                    return Err("4:2:0 video is 8-bit".into());
                };
                let buffer = pixel_buffer(w, h, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)?;
                let [y, cb, cr] = ycbcr(color);
                fill(&buffer, 0, &[y]);
                fill(&buffer, 1, &[cb, cr]);
                unsafe { surface.present_cv_pixel_buffer(raw(&*buffer)) }
                    .map(drop)
                    .map_err(err)
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
                    let target = texture(&device, size.0, size.1, Fill::Bgra([0; 4]), None).ok()?;
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
                Fill::Bgra([c(0.7), c(1.3), c(1.0), 255]),
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
                        release.wait(released.release, Duration::from_millis(100));
                    }
                }
                Err(SurfaceError::DeviceChanged | SurfaceError::WrongDevice) => held = None,
                Err(_) => {}
            }
        }
    }
}
