---
title: "Native frames: <sk-surface>"
label: Native frames
description: Show what a native renderer draws (a video player such as libmpv, a game engine, another process) inside a Skathi element, with Vue or Svelte UI over and around it. The element, its events, and presenting frames from Rust.
section: Embedding
order: 4
---

This is the part a webview cannot do. Some content is drawn by native code: a video player built on libmpv, a game engine's viewport, a renderer in another process. [`<sk-surface>`](/web-platform/sk-surface/) shows it inside the page as an ordinary box: `object-fit`, `border-radius`, `overflow` clipping, transforms, opacity and z-order apply, and the app's controls sit over it like over any other element. No second window glued on top, no airspace problems.

The program that embeds Skathi presents frames to a surface by name, and every `<sk-surface>` with that `name` shows the newest one.

New in 0.4.0. **Windows first:** on Windows a frame can be a Direct3D 11 or 12 texture, or one shared from another device, process or Vulkan, and never leaves the GPU. On macOS and Linux the element and the API exist, and frames are CPU BGRA.

## The element

```vue
<template>
  <div class="player">
    <sk-surface name="player" class="video" @surfaceresize="resized" @surfacelost="lost" />
    <PlayerControls class="overlay" />
  </div>
</template>

<style scoped>
.player {
  position: relative;
}
.video {
  width: 100%;
  aspect-ratio: 16 / 9;
  border-radius: 12px;
  background: #000;
}
.overlay {
  position: absolute;
  inset: auto 0 0 0;
}
</style>
```

| Event           | When                                                 | Carries                                             |
| --------------- | ---------------------------------------------------- | --------------------------------------------------- |
| `surfaceresize` | the painted content box changes, and on first layout | `width`, `height` (physical px), `devicePixelRatio` |
| `surfaceready`  | the first frame shows                                | `width`, `height` of that frame                     |
| `surfacelost`   | the frame goes                                       | `reason`: `"cleared"` or `"device"`                 |

The box is 300 × 150 unless CSS sizes it, and `object-fit` defaults to `contain`, as on `<video>`. Until the first frame it paints nothing, so a background or a placeholder under it shows. Several elements may name the same surface; each shows the same frame at its own size.

## The producer

The Rust side is a `SurfaceHandle`: `skathi::surface(name)` from anywhere, or `HostContext::surface` in `setup`. It is `Send + Sync` and cheap to clone; present from the thread that renders.

```rust
let surface = skathi::surface("player");

let info = surface.info();        // width × height in physical px, scale, visible
let stats = surface.stats();      // frames presented, frames shown
let _watch = surface.on_change(|surface, event| match event {
    skathi::SurfaceEvent::Changed(info) => resize_render_target(info.width, info.height),
    skathi::SurfaceEvent::DeviceLost => recreate_gpu_resources(),
});
```

`info().width` and `height` are the largest element's content box in physical pixels, 0 while no element shows the surface. Render at that size and the picture is 1:1 with the screen. `on_change` keeps reporting until the `SurfaceWatch` it returns is dropped.

### CPU frames, every platform

The simplest path, and the one to start with: BGRA pixels in memory.

```rust
use std::time::Duration;

let surface = skathi::surface("preview");
std::thread::spawn(move || {
    let mut pixels = Vec::new();
    let mut t = 0u32;
    loop {
        std::thread::sleep(Duration::from_millis(16));
        let info = surface.info();
        if !info.visible || info.width == 0 {
            continue; // nothing on screen shows it: render nothing
        }
        let (w, h) = (info.width, info.height);
        pixels.resize((w * h * 4) as usize, 0);
        for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            px.copy_from_slice(&[(x + t) as u8, (y + t) as u8, 160, 255]); // B, G, R, A
        }
        surface.present_bgra(&pixels, w, h, (w * 4) as usize).ok();
        t += 2;
    }
});
```

| Call                                 | Frame                                                 |
| ------------------------------------ | ----------------------------------------------------- |
| `present_bgra(pixels, w, h, stride)` | BGRA, straight alpha                                  |
| `present_bgrx(pixels, w, h, stride)` | BGRX: the fourth byte is ignored, the frame is opaque |
| `clear()`                            | none: the element paints nothing, `surfacelost` fires |

`stride` is the length of one row in bytes, so a frame cut out of a bigger buffer needs no copy. A CPU frame is uploaded to the GPU once per frame shown; at 4K and 60 fps that is about 2 GB/s, which is when the GPU paths below pay off.

### What a present costs the UI

Nothing waits for the UI. A present stores the frame and returns; the element takes the newest one on its next paint, and only while it is visible: a frame presented under `display: none`, outside a scroll clip or in a hidden window costs no upload. A producer that presents faster than the screen refreshes only loses frames, never time. `stats().shown` against `stats().presented` tells how many reach the screen.

## GPU frames, Windows

On Windows the frame can stay on the GPU. Pick the path by what the producer renders with:

| Producer                                                | Path                                                   |
| ------------------------------------------------------- | ------------------------------------------------------ |
| D3D11, can use Skathi's device                          | `present_d3d11_texture` on `d3d11_device()`            |
| D3D12, can use Skathi's device                          | `present_d3d12` on `d3d12_device()`, with a fence      |
| D3D11 or D3D12 on its own device, or in another process | `SharedTexture` with a keyed mutex or a fence          |
| Vulkan                                                  | a D3D12 shared texture and fence, imported into Vulkan |
| CPU, or anything else                                   | `present_bgra`, `present_bgrx`                         |

[GPU frames on Windows](/embedding/gpu-frames/) walks through each, with code. Every GPU present copies the frame into a texture the surface owns, so the producer gets its texture back right away.

## Device loss

When the GPU driver resets, Skathi recreates its device. Every surface drops its GPU frame, the elements fire `surfacelost` with `reason: "device"`, `on_change` hears `SurfaceEvent::DeviceLost`, and a present with an old texture returns `SurfaceError::DeviceChanged`. Get the device again, recreate your textures (or reopen shared ones), and present. CPU frames are not affected.

## Errors

| `SurfaceError`   | Means                                                                               |
| ---------------- | ----------------------------------------------------------------------------------- |
| `NotSupported`   | This platform or renderer has no such path; the message says which one to use       |
| `NoDevice`       | No window has drawn yet, so there is no GPU device; try again after the first frame |
| `DeviceChanged`  | The device was recreated; recreate your GPU resources                               |
| `WrongDevice`    | The texture is from another device; share it instead                                |
| `Format`, `Size` | The frame is not BGRA8, is zero or oversized, or the buffer is too short            |
| `Timeout`        | A keyed mutex or fence was not released in time; nothing was presented              |
| `Busy`           | Every surface texture is still in flight; drop the frame or wait                    |
| `Platform`       | A system call failed; the message carries its error                                 |

## Platform support

| Platform | `<sk-surface>` | CPU frames | D3D11 textures | D3D12 textures      | Shared textures (keyed mutex, fence) |
| -------- | -------------- | ---------- | -------------- | ------------------- | ------------------------------------ |
| Windows  | yes            | yes        | yes            | D3D12 renderer only | yes                                  |
| macOS    | yes            | yes        | no             | no                  | no                                   |
| Linux    | yes            | yes        | no             | no                  | no                                   |

Next: [GPU frames on Windows](/embedding/gpu-frames/).
