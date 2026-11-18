---
title: GPU frames on Windows
description: Present Direct3D 11 and Direct3D 12 textures to <sk-surface> without a CPU copy, share textures across devices, processes and Vulkan, and synchronise with keyed mutexes or fences.
section: Embedding
order: 5
since: "0.4"
---

On Windows, a frame for [`<sk-surface>`](/embedding/native-frames/) can stay on the GPU from the producer's render to the screen. There are three ways in, by where the producer's texture lives.

Skathi renders with Direct3D 12 (the default build), or with Direct3D 11 in a build without the `native-dx12` feature. That decides what is cheapest:

| Producer                           | Path                                                                                           | Under the D3D11 renderer                    |
| ---------------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------- |
| D3D11, can use Skathi's device     | `present_d3d11_texture` on `d3d11_device()`                                                    | same                                        |
| D3D11 on its own device or process | `SharedTexture` with a keyed mutex, or with an `ID3D11Fence`                                   | same                                        |
| D3D12, can use Skathi's device     | `present_d3d12` on `d3d12_device()`, with the producer's fence                                 | `NotSupported`: share the texture instead   |
| D3D12 on its own device or process | `SharedTexture::with_fence`, `present_shared_fenced`                                           | same (an `ID3D11Fence` opens the D3D12 one) |
| Vulkan                             | external memory and a semaphore shared as a D3D12 fence: `with_fence`, `present_shared_fenced` | same                                        |

Under the D3D12 renderer the D3D12 and fence paths never touch Direct3D 11: the copy runs on a queue of the surface's own, after a GPU-side wait on the producer's fence, and the window takes the frame once that copy has finished, so a slow producer never stalls the UI. Since 0.5 each surface has its own queue and release fence: a fence that is never signalled holds back only the surface it was presented to. The D3D11 paths go through the renderer's D3D11On12 device, created on first use.

Textures are `B8G8R8A8_UNORM` (or its `_UNORM_SRGB` and `_TYPELESS` forms), or `R16G16B16A16_FLOAT` on a [half-float surface](#half-float-textures); mip 0 is shown. Alpha is straight.

## Direct3D 11 on the renderer's device

`d3d11_device()` is the device Skathi composites with (the D3D11On12 device of its D3D12 renderer). It is multithread-protected: create textures on it and use its immediate context from your thread. The raw pointer works with any version of the `windows` crate. The copy is queued on that immediate context before `present_d3d11_texture` returns, so the next frame can go into the same texture at once; a multisampled texture is resolved.

This producer clears a texture to a moving colour at the element's size:

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

## Direct3D 12 on the renderer's device

`d3d12_device()` is the renderer's `ID3D12Device` and its direct queue. Render on a queue of your own on that device, signal a fence of yours, and present the resource with that fence and value. Nothing blocks: the surface's queue waits for your value on the GPU, copies, and signals the surface's release fence. `present_d3d12` returns a `FramePresent`: the frame's id ([when it reached the screen](/embedding/native-frames/#knowing-when-a-frame-is-on-screen)) and that release value. Write the resource again once `release_fence()` reaches it, or make your queue wait on it (`ID3D12CommandQueue::Wait`), or keep a few resources in rotation.

```rust
let gpu = surface.d3d12_device()?; // NotSupported under the D3D11 renderer
let device = unsafe { ID3D12Device::from_raw_borrowed(&gpu.as_raw()) }.unwrap().clone();
// … record into `resource` (BGRA8, single-sampled) on your queue, then:
unsafe { queue.Signal(&fence, frame)? };
let presented = unsafe {
    surface.present_d3d12(
        resource.as_raw(),
        D3D12_RESOURCE_STATE_RENDER_TARGET.0, // its state now; it is left in it
        Some((fence.as_raw(), frame)),
    )?
};
let release = surface.release_fence()?;
release.wait(presented.release, Duration::from_millis(100)); // or queue.Wait on release.as_raw()
```

`d3d12_device().queue()` is the renderer's own queue: work submitted there delays the next frame, so a producer keeps its own. A resource made on another D3D12 device returns `SurfaceError::WrongDevice`; share it as below. If the surface's textures are all still in flight (the producer outruns the window, or its fence is never signalled), a present to it returns `SurfaceError::Busy` and the frame can be dropped. Other surfaces keep showing their frames. Since 0.5 a texture is free again as soon as its copy has finished; before, in an HDR window, four copies queued at once could leave every later present `Busy`.

## Textures from another device, API or process

A renderer on a device of its own (another adapter queue, another API, another process) shares its texture instead and hands the NT handle number and its process id to the embedder, through a [native module](/embedding/native-modules/) function, a [native API](/embedding/native-api/) or your own IPC:

- **D3D11:** create the texture with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE` and either `D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX`, or `D3D11_RESOURCE_MISC_SHARED` with an `ID3D11Fence` (`D3D11_FENCE_FLAG_SHARED`); `IDXGIResource1::CreateSharedHandle`.
- **D3D12:** a committed resource with `D3D12_HEAP_FLAG_SHARED` and a fence with `D3D12_FENCE_FLAG_SHARED`; `ID3D12Device::CreateSharedHandle` for both. Leave the resource in `D3D12_RESOURCE_STATE_COMMON` when you signal.
- **Vulkan:** Vulkan imports D3D12 objects more widely than it exports them, so create the texture and fence on a D3D12 device as above and import them: the memory with `VkImportMemoryWin32HandleInfoKHR` (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_RESOURCE_BIT`, `VK_KHR_external_memory_win32`), the fence as a timeline semaphore with `VkImportSemaphoreWin32HandleInfoKHR` (`VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_D3D12_FENCE_BIT`, `VK_KHR_external_semaphore_win32`). Submit the render with a signal of the frame value, and release the image to the external queue family.

### With a keyed mutex

```rust
// Once per texture; again after SurfaceError::DeviceChanged.
let shared = skathi::SharedTexture::open_from_process(producer_pid, handle)?
    .with_keyed_mutex(1, 0); // acquire key 1 (the producer released it), release key 0
// Per frame, after the producer released key 1:
surface.present_shared(&shared)?;
```

### With a fence

The producer picks the values per frame: the copy waits until the fence reaches `wait`, then signals it to `release`, the point after which the producer may write the texture again.

```rust
let shared = skathi::SharedTexture::open_from_process(producer_pid, texture_handle)?
    .with_fence_from_process(producer_pid, fence_handle)?;
// Per frame, after the producer signalled 2n + 1:
surface.present_shared_fenced(&shared, 2 * n + 1, 2 * n + 2)?;
// The producer waits for 2n + 2 (on its queue or the CPU) before rendering into it again.
```

`open_from_process` duplicates the handle into this process (it needs `PROCESS_DUP_HANDLE` on the producer, which a child process or one of the same user grants). A handle already in this process opens with `SharedTexture::open(handle)` and `with_fence(handle)`; a legacy handle from `IDXGIResource::GetSharedHandle` opens too, without a fence. Without a keyed mutex or a fence the producer must finish its writes before the present.

Under the D3D12 renderer a fenced present returns at once and the wait happens on the GPU. Under the D3D11 renderer the present waits on the CPU for `wait`, up to `with_timeout` (100 ms by default, then `SurfaceError::Timeout`), so a fence that is never signalled cannot stall the window.

## Half-float textures

A surface set to `SurfaceFormat::Rgba16Float` takes `R16G16B16A16_FLOAT` (or `R16G16B16A16_TYPELESS`) on every path above: a D3D11 texture on `d3d11_device()`, a D3D12 resource on `d3d12_device()`, and shared textures with a keyed mutex or a fence. Vulkan renders into a `VK_FORMAT_R16G16B16A16_SFLOAT` image bound to the imported memory of an `R16G16B16A16_FLOAT` D3D12 resource. Nothing else changes: the same fences, the same release values, the same rotation.

```rust
surface.set_format(skathi::SurfaceFormat::Rgba16Float);
// … render into an R16G16B16A16_FLOAT texture on d3d11_device(), then:
unsafe { surface.present_d3d11_texture(texture.as_raw())? };
```

The values are scRGB, with 1.0 as SDR white: what Windows shows for 1.0 on an SDR display. A B8G8R8A8 texture on a half-float surface, or a half-float one on an 8-bit surface, returns `SurfaceError::FormatMismatch`.

The frame stays half float in the surface's textures. A window on a display in HDR mode draws HDR while it shows a visible half-float surface whose [`dynamic-range-limit`](/css/dynamic-range-limit/) allows more than 1.0 (see [HDR windows](/embedding/native-frames/#hdr-windows)): the element then shows the frame's own values, up to that limit and the display's headroom. In a standard window, when an element draws the frame, the surface first renders it into an 8-bit texture of its own (clamped to 0…1, then the exact sRGB curve; see [half-float frames](/embedding/native-frames/#half-float-frames)): under the D3D12 renderer on the surface's queue, with the window's copy waiting on the GPU; under Direct3D 11 on the renderer's immediate context. A frame that is never shown is never mapped.

## Choosing, in short

- **One process, D3D11 renderer of your own?** Render on `d3d11_device()`, present the texture. Fewest moving parts.
- **One process, D3D12?** Render on `d3d12_device()` with your own queue and fence, present with `present_d3d12`, rotate two or three textures.
- **Another process, or a device you cannot change?** Share the texture with a fence; a keyed mutex if the producer is D3D11 and already uses one.
- **Vulkan?** Create on D3D12, import into Vulkan, present with a fence.
- **Not sure yet?** Start with `present_bgra`. It works everywhere, and moving to a GPU path later changes only the producer.

Next: [GPU frames on macOS](/embedding/gpu-frames-macos/).
