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

New in 0.4.0. **Windows first:** on Windows a frame can be a Direct3D 11 or 12 texture, or one shared
from Vulkan, and never leaves the GPU. On macOS and Linux the element and the API exist, but frames
are CPU BGRA; the GPU calls return `SurfaceError::NotSupported`.

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

| Call                                          | Frame                                                           | Platforms |
| --------------------------------------------- | --------------------------------------------------------------- | --------- |
| `present_d3d11_texture(texture)`              | an `ID3D11Texture2D` on the renderer's device, `d3d11_device()` | Windows   |
| `present_d3d12(resource, state, ready)`       | an `ID3D12Resource` on the renderer's device, `d3d12_device()`  | Windows   |
| `present_shared(&SharedTexture)`              | a shared texture, with a keyed mutex or no synchronisation      | Windows   |
| `present_shared_fenced(&SharedTexture, w, r)` | a shared texture synchronised by a shared fence                 | Windows   |
| `present_bgra(pixels, w, h, stride)`          | CPU BGRA, straight alpha                                        | all       |
| `present_bgrx(pixels, w, h, stride)`          | CPU BGRX, opaque                                                | all       |
| `clear()`                                     | none: the element paints nothing, `surfacelost` fires           | all       |

Textures are `B8G8R8A8_UNORM` (or its `_UNORM_SRGB` and `_TYPELESS` forms); mip 0 is shown. Alpha is
straight. Every GPU present copies the frame into a texture of the surface's own, so the producer
gets its texture back as soon as that copy has run.

### Which path

Skathi renders with Direct3D 12 (the default build), or with Direct3D 11 in a host built without the
`native-dx12` feature. Pick the path by what the producer renders with:

| Producer                           | Path                                                                                           | Under the D3D11 renderer                    |
| ---------------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------- |
| D3D11, can use Skathi's device     | `present_d3d11_texture` on `d3d11_device()`                                                    | same                                        |
| D3D11 on its own device or process | `SharedTexture` with a keyed mutex, or with an `ID3D11Fence`                                   | same                                        |
| D3D12, can use Skathi's device     | `present_d3d12` on `d3d12_device()`, with the producer's fence                                 | `NotSupported`: share the texture instead   |
| D3D12 on its own device or process | `SharedTexture::with_fence`, `present_shared_fenced`                                           | same (an `ID3D11Fence` opens the D3D12 one) |
| Vulkan                             | external memory and a semaphore shared as a D3D12 fence: `with_fence`, `present_shared_fenced` | same                                        |
| CPU, or a GPU API not listed       | `present_bgra` / `present_bgrx`                                                                | same                                        |

Under the D3D12 renderer the D3D12 and fence paths never touch Direct3D 11: the copy runs on a queue of
the surface's own, after a GPU-side wait on the producer's fence, and the window takes the frame once
that copy has finished, so a slow producer never stalls the UI. The D3D11 paths go through the
renderer's D3D11On12 device, created on first use.

### Direct3D 11 on the renderer's device

`d3d11_device()` is the device Skathi composites with (the D3D11On12 device of its D3D12 renderer). It
is multithread-protected: create textures on it and use its immediate context from your thread. The
raw pointer works with any version of the `windows` crate. The copy is queued on that immediate
context before `present_d3d11_texture` returns, so the next frame can go into the same texture at
once; a multisampled texture is resolved. This producer clears a texture to a moving colour at the
element's size:

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

`crates/sk-app/examples/surface.rs` in the Skathi repository is a complete program with every path.

### Direct3D 12 on the renderer's device

`d3d12_device()` is the renderer's `ID3D12Device` and its direct queue. Render on a queue of your own
on that device, signal a fence of yours, and present the resource with that fence and value. Nothing
blocks: the surface's queue waits for your value on the GPU, copies, and signals its release fence,
whose value `present_d3d12` returns. Write the resource again once `release_fence()` reaches it, or
make your queue wait on it (`ID3D12CommandQueue::Wait`), or keep a few resources in rotation.

```rust
let gpu = surface.d3d12_device()?; // NotSupported under the D3D11 renderer
let device = unsafe { ID3D12Device::from_raw_borrowed(&gpu.as_raw()) }.unwrap().clone();
// … record into `resource` (BGRA8, single-sampled) on your queue, then:
unsafe { queue.Signal(&fence, frame)? };
let released = unsafe {
    surface.present_d3d12(
        resource.as_raw(),
        D3D12_RESOURCE_STATE_RENDER_TARGET.0, // its state now; it is left in it
        Some((fence.as_raw(), frame)),
    )?
};
let release = surface.release_fence()?;
release.wait(released, Duration::from_millis(100)); // or queue.Wait on release.as_raw()
```

`d3d12_device().queue()` is the renderer's own queue: work submitted there delays the next frame,
so a producer keeps its own. A resource made on another D3D12 device returns
`SurfaceError::WrongDevice`; share it as below. If the surface textures are all still in flight (the
producer outruns the window, or its fence is never signalled), a present returns
`SurfaceError::Busy` and the frame can be dropped.

### Textures from another device, API or process

A renderer on a device of its own (another adapter queue, another API, another process) shares its
texture instead and hands the NT handle number and its process id to the embedder (an `appHost` call
or your own IPC):

- D3D11: create the texture with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE` and either
  `D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX` or `D3D11_RESOURCE_MISC_SHARED` with an `ID3D11Fence`
  (`D3D11_FENCE_FLAG_SHARED`); `IDXGIResource1::CreateSharedHandle`.
- D3D12: a committed resource with `D3D12_HEAP_FLAG_SHARED` and a fence with
  `D3D12_FENCE_FLAG_SHARED`; `ID3D12Device::CreateSharedHandle` for both. Leave the resource in
  `D3D12_RESOURCE_STATE_COMMON` when you signal.
- Vulkan: Vulkan imports D3D12 objects more widely than it exports them, so create the texture and
  fence on a D3D12 device as above and import them: the memory with `VkImportMemoryWin32HandleInfoKHR`
  (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_RESOURCE_BIT`, `VK_KHR_external_memory_win32`), the fence as
  a timeline semaphore with `VkImportSemaphoreWin32HandleInfoKHR`
  (`VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_D3D12_FENCE_BIT`, `VK_KHR_external_semaphore_win32`). Submit the
  render with a signal of the frame value, and release the image to the external queue family.

```rust
// Once per texture; again after SurfaceError::DeviceChanged.
let shared = skathi::SharedTexture::open_from_process(producer_pid, handle)?
    .with_keyed_mutex(1, 0); // acquire key 1 (the producer released it), release key 0
// Per frame, after the producer released key 1:
surface.present_shared(&shared)?;
```

With a fence the producer picks the values per frame: the copy waits until the fence reaches `wait`,
then signals it to `release`, the point after which the producer may write the texture again.

```rust
let shared = skathi::SharedTexture::open_from_process(producer_pid, texture_handle)?
    .with_fence_from_process(producer_pid, fence_handle)?;
// Per frame, after the producer signalled 2n + 1:
surface.present_shared_fenced(&shared, 2 * n + 1, 2 * n + 2)?;
// The producer waits for 2n + 2 (on its queue or the CPU) before rendering into it again.
```

`open_from_process` duplicates the handle into this process (it needs `PROCESS_DUP_HANDLE` on the
producer, which a child process or one of the same user grants). A handle already in this process
opens with `SharedTexture::open(handle)` and `with_fence(handle)`; a legacy handle from
`IDXGIResource::GetSharedHandle` opens too, without a fence. Without a keyed mutex or a fence the
producer must finish its writes before the present.

Under the D3D12 renderer a fenced present returns at once and the wait happens on the GPU. Under the
D3D11 renderer the present waits on the CPU for `wait`, up to `with_timeout` (100 ms by default, then
`SurfaceError::Timeout`), so a fence that is never signalled cannot stall the window.

### Device loss

When the GPU driver resets, Skathi recreates its device. Every surface drops its GPU frame, the
elements fire `surfacelost` with `reason: "device"`, `on_change` hears `SurfaceEvent::DeviceLost`, and
a present with an old texture returns `SurfaceError::DeviceChanged`. Get `d3d11_device()` or
`d3d12_device()` again, then recreate your textures (or reopen shared ones) and present.

## Recipe: libmpv

A libmpv player renders each video frame into a texture the surface takes and leaves the controls to
the app. Upstream libmpv's render API has two backends, OpenGL and software; a libmpv build or fork
whose render API renders through libplacebo on Vulkan (the `gpu-next` renderer) fits the Vulkan path.

| How mpv renders                           | mpv options and render API                                                                                                                           | Skathi call                                                                                       |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Software                                  | `vo=libmpv`, `hwdec=auto-copy`; `MPV_RENDER_API_TYPE_SW` with `MPV_RENDER_PARAM_SW_FORMAT` `"bgr0"`                                                  | `present_bgrx`, one CPU copy per frame                                                            |
| OpenGL through ANGLE on Skathi's device   | `vo=libmpv`, `hwdec=d3d11va`; `MPV_RENDER_API_TYPE_OPENGL` on an ANGLE display made from `d3d11_device()` (`EGL_ANGLE_device_d3d11`)                 | `present_d3d11_texture` with the texture behind mpv's FBO (`EGL_ANGLE_d3d_texture_client_buffer`) |
| OpenGL through ANGLE on its own device    | the same on a device of its own; the FBO's texture made `SHARED_NTHANDLE \| SHARED_KEYEDMUTEX`                                                       | `SharedTexture::with_keyed_mutex`, `present_shared`                                               |
| Vulkan through libplacebo (build or fork) | `vo=libmpv`, `gpu-api=vulkan`, `hwdec=d3d11va` or `vulkan`; the target image imported from a D3D12 shared resource, its semaphore from a D3D12 fence | `SharedTexture::with_fence`, `present_shared_fenced`                                              |

Then, whatever the backend:

1. In the update callback (any thread), when `mpv_render_context_update` reports
   `MPV_RENDER_UPDATE_FRAME`, render a frame of `surface.info()`'s size and present it. Skip the render
   while `info().visible` is false.
2. With a fence, signal the frame's value after the render and wait for the release value before the
   next one renders into the same texture, or rotate two or three textures.
3. Forward `surfaceresize` (or `on_change`) to resize the target, and the app's controls to
   `mpv_command` through `Builder::function`.

## Platform support

| Platform | `<sk-surface>` | CPU frames | D3D11 textures | D3D12 textures      | Shared textures (keyed mutex, fence) |
| -------- | -------------- | ---------- | -------------- | ------------------- | ------------------------------------ |
| Windows  | yes            | yes        | yes            | D3D12 renderer only | yes                                  |
| macOS    | yes            | yes        | no             | no                  | no                                   |
| Linux    | yes            | yes        | no             | no                  | no                                   |
