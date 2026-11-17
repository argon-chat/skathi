---
title: Showing native GPU content
label: Native GPU content
description: Show what a native renderer draws (a video player such as libmpv, a game engine, another process) inside a Skathi element, with Vue or Svelte UI over and around it.
section: Guides
order: 8
---

Some content is drawn by native code: a video player built on libmpv, a game engine's viewport, a
renderer in another process. [`<sk-surface>`](/web-platform/sk-surface/) shows it inside the page.
The program that embeds Skathi presents frames to a surface by name, and every `<sk-surface>` with
that `name` shows the newest one. The element is an ordinary box: `object-fit`, `border-radius`,
`overflow` clipping, transforms, opacity and z-order apply, and the app's controls sit over it like
over any other element.

New in 0.4.0. **Windows first:** on Windows a frame can be a Direct3D 11 texture and never leaves the
GPU. On macOS and Linux the element and the API exist, but frames are CPU BGRA; the GPU calls return
`SurfaceError::NotSupported`.

## The element

```vue
<template>
  <div class="player">
    <sk-surface name="player" class="video" @surfaceresize="resized" @surfacelost="lost" />
    <PlayerControls class="overlay" />
  </div>
</template>
```

| Event           | When                                                 | Carries                                             |
| --------------- | ---------------------------------------------------- | --------------------------------------------------- |
| `surfaceresize` | the painted content box changes, and on first layout | `width`, `height` (physical px), `devicePixelRatio` |
| `surfaceready`  | the first frame shows                                | `width`, `height` of that frame                     |
| `surfacelost`   | the frame goes                                       | `reason`: `"cleared"` or `"device"`                 |

The box is 300 × 150 unless CSS sizes it, and `object-fit` defaults to `contain`, as on `<video>`. Until
the first frame it paints nothing, so a background or a placeholder under it shows.

## The producer

The Rust side is a `SurfaceHandle`: `skathi::surface(name)` from anywhere, or `HostContext::surface`
in `Builder::setup`. It is `Send + Sync` and cheap to clone; present from the thread that renders.

```rust
let surface = skathi::surface("player");
let info = surface.info(); // width × height in physical px, scale, visible
let _watch = surface.on_change(|surface, event| { /* a new size, or DeviceLost */ });
```

Presenting never blocks on the UI. The newest frame wins, and the element takes it on its next paint,
only while it is visible: a frame presented under `display: none`, outside a scroll clip or in a hidden
window costs no upload. `surface.stats()` counts frames presented and frames shown. A producer that
renders continuously can skip frames while `info().visible` is false and render at `info().width` ×
`info().height` to match the painted size.

| Call                                 | Frame                                                           | Platforms |
| ------------------------------------ | --------------------------------------------------------------- | --------- |
| `present_d3d11_texture(texture)`     | an `ID3D11Texture2D` on the renderer's device, `d3d11_device()` | Windows   |
| `present_shared(&SharedTexture)`     | a DXGI shared texture from another device or process            | Windows   |
| `present_bgra(pixels, w, h, stride)` | CPU BGRA, straight alpha                                        | all       |
| `present_bgrx(pixels, w, h, stride)` | CPU BGRX, opaque                                                | all       |
| `clear()`                            | none: the element paints nothing, `surfacelost` fires           | all       |

Textures are `B8G8R8A8_UNORM` (or its `_UNORM_SRGB` and `_TYPELESS` forms), any sample count; mip 0
is shown. Alpha is straight. A present copies the texture into the surface's own on the GPU before it
returns, so the producer can start the next frame in the same texture at once.

### Rendering on the renderer's device

`d3d11_device()` is the device Skathi composites with (the D3D11On12 device of its D3D12 renderer). It
is multithread-protected: create textures on it and use its immediate context from your thread. The
raw pointer works with any version of the `windows` crate. This producer clears a texture to a moving
colour at the element's size:

```rust
use std::time::{Duration, Instant};

use skathi::{SurfaceError, SurfaceHandle};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

fn main() {
    let builder = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .setup(|host| {
            let surface = host.surface("preview");
            std::thread::spawn(move || produce(surface));
            Ok(())
        });
    let ui = builder.spawn().expect("start Skathi");
    std::process::exit(if ui.wait().clean { 0 } else { 1 });
}

fn produce(surface: SurfaceHandle) {
    let start = Instant::now();
    let mut gpu: Option<(ID3D11Device, ID3D11DeviceContext)> = None;
    let mut target: Option<((u32, u32), ID3D11Texture2D, ID3D11RenderTargetView)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(16));
        let info = surface.info();
        if !info.visible || info.width == 0 {
            continue;
        }
        if gpu.is_none() {
            // `NoDevice` until the window has drawn once.
            let Ok(device) = surface.d3d11_device() else { continue };
            let raw = device.as_raw();
            let device = unsafe { ID3D11Device::from_raw_borrowed(&raw) }.unwrap().clone();
            let context = unsafe { device.GetImmediateContext() }.unwrap();
            gpu = Some((device, context));
        }
        let (device, context) = gpu.as_ref().unwrap();
        let size = (info.width, info.height);
        if target.as_ref().is_none_or(|(s, ..)| *s != size) {
            target = Some(render_target(device, size));
        }
        let (_, texture, view) = target.as_ref().unwrap();
        let t = start.elapsed().as_secs_f32();
        let rgba = [0.5 + 0.5 * t.sin(), 0.5 + 0.5 * (t * 1.3).sin(), 0.6, 1.0];
        unsafe { context.ClearRenderTargetView(view, &rgba) };
        match unsafe { surface.present_d3d11_texture(texture.as_raw()) } {
            Err(SurfaceError::DeviceChanged | SurfaceError::WrongDevice) => (gpu, target) = (None, None),
            _ => {}
        }
    }
}

fn render_target(device: &ID3D11Device, (w, h): (u32, u32)) -> ((u32, u32), ID3D11Texture2D, ID3D11RenderTargetView) {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        ..Default::default()
    };
    let (mut texture, mut view) = (None, None);
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut texture)).unwrap();
        device.CreateRenderTargetView(texture.as_ref().unwrap(), None, Some(&mut view)).unwrap();
    }
    ((w, h), texture.unwrap(), view.unwrap())
}
```

`crates/sk-app/examples/surface.rs` in the Skathi repository is a complete program with all three
paths.

### Textures from another device or process

A renderer on a device of its own (another adapter queue, another API through DXGI, another process)
shares its texture instead: create it with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE |
D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX`, get an NT handle with `IDXGIResource1::CreateSharedHandle`,
and hand the number and your process id to the embedder (an `appHost` call or your own IPC).

```rust
// Once per texture; again after SurfaceError::DeviceChanged.
let shared = skathi::SharedTexture::open_from_process(producer_pid, handle)?
    .with_keyed_mutex(1, 0); // acquire key 1 (the producer released it), release key 0
// Per frame, after the producer released key 1:
surface.present_shared(&shared)?;
```

`open_from_process` duplicates the handle into this process (it needs `PROCESS_DUP_HANDLE` on the
producer, which a child process or one of the same user grants). A handle already in this process
opens with `SharedTexture::open(handle)`, and so does a legacy handle from
`IDXGIResource::GetSharedHandle`. Without a keyed mutex the producer must finish its writes (and flush)
before the present.

### Device loss

When the GPU driver resets, Skathi recreates its device. Every surface drops its GPU frame, the
elements fire `surfacelost` with `reason: "device"`, `on_change` hears `SurfaceEvent::DeviceLost`, and
a present with an old texture returns `SurfaceError::DeviceChanged`. Get `d3d11_device()` again, then
recreate your textures (or reopen shared ones) and present.

## Recipe: libmpv

A libmpv player renders each video frame into a texture of the surface and leaves the controls to the
app:

1. Create the mpv handle with `vo=libmpv`, and `hwdec=d3d11va` when the render API runs on the
   surface's device, so decoding stays on the GPU.
2. Create the render context on `surface.d3d11_device()`. Upstream libmpv's render API has OpenGL
   and software backends; a libmpv build with a D3D11 render API (`MPV_RENDER_API_TYPE_D3D11` in some
   builds) takes the device directly. With upstream libmpv, use OpenGL through ANGLE: its D3D11 backend
   renders into a shared BGRA texture wrapped as an EGL pbuffer (`EGL_ANGLE_d3d_texture_client_buffer`)
   and bound as mpv's FBO, presented with `present_shared`. The software API
   (`MPV_RENDER_API_TYPE_SW`, format `bgr0`) with `present_bgrx` is the simplest, at a CPU copy per
   frame.
3. In the update callback (any thread), when `mpv_render_context_update` reports
   `MPV_RENDER_UPDATE_FRAME`, render a frame of `surface.info()`'s size and present it. Skip the render
   while `info().visible` is false.
4. Forward `surfaceresize` (or `on_change`) to resize that texture, and the app's controls to
   `mpv_command` through `Builder::function`.

## Platform support

| Platform | `<sk-surface>` | CPU frames | D3D11 textures | Shared textures |
| -------- | -------------- | ---------- | -------------- | --------------- |
| Windows  | yes            | yes        | yes            | yes             |
| macOS    | yes            | yes        | no             | no              |
| Linux    | yes            | yes        | no             | no              |
