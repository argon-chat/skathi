---
title: "Native frames: <sk-surface>"
label: Native frames
description: Show what a native renderer draws (a video player such as libmpv, a game engine, another process) inside a Skathi element, with Vue or Svelte UI over and around it. The element, its events, and presenting frames from Rust.
section: Embedding
order: 4
---

This is the part a webview cannot do. Some content is drawn by native code: a video player built on libmpv, a game engine's viewport, a renderer in another process. [`<sk-surface>`](/web-platform/sk-surface/) shows it inside the page as an ordinary box: `object-fit`, `border-radius`, `overflow` clipping, transforms, opacity and z-order apply, and the app's controls sit over it like over any other element. No second window glued on top, no airspace problems.

The program that embeds Skathi presents frames to a surface by name, and every `<sk-surface>` with that `name` shows the newest one.

New in 0.4.0. On Windows a frame can be a Direct3D 11 or 12 texture, or one shared from another device, process or Vulkan; on macOS a Metal texture, an IOSurface or a CVPixelBuffer. Either way it never leaves the GPU. On Linux the element and the API exist, and frames are CPU BGRA.

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

| Event                  | When                                                                                 | Carries                                                                                  |
| ---------------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| `surfaceresize`        | the painted content box changes, and on first layout                                 | `width`, `height` (physical px), `devicePixelRatio`                                      |
| `surfaceready`         | the first frame shows                                                                | `width`, `height` of that frame                                                          |
| `surfacelost`          | the frame goes                                                                       | `reason`: `"cleared"` or `"device"`                                                      |
| `surfacedisplaychange` | the [display](#the-display) is first known, changes, or the element moves to another | `headroom`, `potentialHeadroom`, `refreshRate`, `sdrWhiteNits`, `hdr`, `variableRefresh` |

The box is 300 × 150 unless CSS sizes it, and `object-fit` defaults to `contain`, as on `<video>`. Until the first frame it paints nothing, so a background or a placeholder under it shows. Several elements may name the same surface; each shows the same frame at its own size.

## The producer

The Rust side is a `SurfaceHandle`: `skathi::surface(name)` from anywhere, or `HostContext::surface` in `setup`. It is `Send + Sync` and cheap to clone; present from the thread that renders.

```rust
let surface = skathi::surface("player");

let info = surface.info();        // width × height in physical px, scale, visible, display
let stats = surface.stats();      // frames presented, frames shown
let _watch = surface.on_change(|surface, event| match event {
    skathi::SurfaceEvent::Changed(info) => resize_render_target(info.width, info.height),
    skathi::SurfaceEvent::DeviceLost => recreate_gpu_resources(),
    skathi::SurfaceEvent::DisplayChanged(display) => retarget(display),
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

## GPU frames, Windows and macOS

On Windows and macOS the frame can stay on the GPU. Pick the path by what the producer renders with:

| Producer                                                         | Path                                                       |
| ---------------------------------------------------------------- | ---------------------------------------------------------- |
| D3D11, can use Skathi's device                                   | `present_d3d11_texture` on `d3d11_device()`                |
| D3D12, can use Skathi's device                                   | `present_d3d12` on `d3d12_device()`, with a fence          |
| D3D11 or D3D12 on its own device, or in another process          | `SharedTexture` with a keyed mutex or a fence              |
| Vulkan on Windows                                                | a D3D12 shared texture and fence, imported into Vulkan     |
| Metal, can use Skathi's device                                   | `present_metal_texture` on `metal_device()`, with an event |
| Metal on another device, another process, anything on IOSurfaces | `SharedTexture::from_iosurface` or `from_mach_port`        |
| VideoToolbox, AVFoundation, ScreenCaptureKit                     | `present_cv_pixel_buffer`                                  |
| CPU, or anything else                                            | `present_bgra`, `present_bgrx`                             |

[GPU frames on Windows](/embedding/gpu-frames/) and [GPU frames on macOS](/embedding/gpu-frames-macos/) walk through each, with code. Every GPU present copies the frame into a texture the surface owns, so the producer gets its texture back as soon as that copy has run.

## Half-float frames

A surface takes 8-bit BGRA unless it is told otherwise. A producer that renders in linear light, such as a video player tone-mapping HDR video or a renderer with an FP16 framebuffer, can hand over half floats instead. Set the surface's format once, from native code, before presenting:

```rust
use skathi::SurfaceFormat;

let surface = skathi::surface("player");
surface.set_format(SurfaceFormat::Rgba16Float);
```

From then on the surface takes half-float RGBA and refuses 8-bit frames, and the other way round for a `Bgra8` surface: a present of the wrong kind returns `SurfaceError::FormatMismatch` and shows nothing. Every path that makes sense takes them:

| Path                         | Half-float frame                                                                        |
| ---------------------------- | --------------------------------------------------------------------------------------- |
| CPU                          | `present_rgba16f(halves, w, h, stride)`: `&[u16]` of IEEE half bits, `stride` in values |
| D3D11, D3D12, shared, Vulkan | `R16G16B16A16_FLOAT` (or `_TYPELESS`) textures, resources and shared handles            |
| Metal, IOSurface             | `RGBA16Float` textures; IOSurfaces with pixel format `'RGhA'`                           |
| CVPixelBuffer                | `kCVPixelFormatType_64RGBAHalf` (`'RGhA'`)                                              |

The values are linear extended sRGB: sRGB (BT.709) primaries, no transfer curve, 1.0 is SDR white, and values may go below 0 and above 1. That is what CoreVideo's `'RGhA'` holds in `kCGColorSpaceExtendedLinearSRGB`, and Windows' scRGB. Alpha is straight, as in the 8-bit paths.

### What shows today

The window itself still draws 8-bit sRGB, so the element maps each frame into it: every channel is clamped to 0…1 and then encoded with the exact sRGB curve, the one gradients and video already go through. Values from 0 to 1 are never touched; there is no tone-curve shoulder below 1.0, and anything brighter than SDR white is clipped to it. Two things follow:

- **SDR content looks exactly as on the 8-bit path.** Linearise your 8-bit sRGB frames with the sRGB EOTF and they look identical to the BGRA8 path: Skathi's tests put all 256 sRGB code values through half floats and get every one back unchanged, on Direct3D 11, Direct3D 12 and Metal.
- **Highlights clip.** A producer that has highlights above 1.0 and wants them rolled off rather than clipped tone-maps them itself, to a peak of 1.0.

Clipping is what Windows does with scRGB content and macOS with EDR content on a screen that has no headroom, and it keeps the mapping exact for everything an SDR frame can hold. The window does not yet tag its output as HDR (no FP16 swapchain on Windows, no EDR on macOS), so values above 1.0 are clipped even on an HDR display; [`output_headroom`](#the-display) says so.

GPU frames stay half float in the surface's own textures; an element maps a frame into an 8-bit texture when it first draws it, on the GPU, so a frame replaced before it is shown is never mapped. CPU frames are mapped when presented, on the producer's thread, through a table. A half-float frame takes twice the memory of an 8-bit one, and the mapping costs one small GPU pass per frame shown.

## The display

`info().display` is a `SurfaceDisplay`: the screen the largest element showing the surface is on. `on_change` hears `SurfaceEvent::DisplayChanged` when it is first known, when it changes, and when that element moves to another screen; each element fires `surfacedisplaychange` for its own. While no element shows the surface, the last one known stays.

Brightness is in the surface's unit, where 1.0 is SDR white (paper white), on every platform:

| Field                          | Means                                                                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------ |
| `headroom`                     | the brightest value the screen shows now; 1.0 on an SDR screen                                         |
| `potential_headroom`           | the brightest it can show; on macOS at full brightness, on Windows the same as `headroom`              |
| `output_headroom`              | the brightest value a frame reaches the screen with: 1.0, because windows draw 8-bit sRGB              |
| `sdr_white_nits`               | SDR white in nits, on Windows in HDR mode (the "SDR content brightness" slider); `None` elsewhere      |
| `peak_nits`, `full_frame_nits` | the screen's luminance, where Windows knows it                                                         |
| `hdr`                          | Windows' HDR mode is on, or the macOS screen has EDR headroom                                          |
| `refresh_hz`                   | Windows: the current mode's rate; macOS: the screen's maximum (`maximumFramesPerSecond`)               |
| `variable_refresh`             | macOS: whether the screen's refresh interval has a range (ProMotion, Adaptive-Sync); `None` on Windows |
| `known`                        | `false` on Linux, and before the window first draws                                                    |

1.0 as SDR white is what macOS EDR values already are, and how Windows composes SDR content in HDR mode: at the white level the user picked. On Windows, `headroom` is the screen's peak over that white (`MaxLuminance` / SDR white), and a scRGB value, where 1.0 is 80 nits, is a surface value times `sdr_white_nits / 80`. In SDR mode it is 1.0.

macOS sends nothing when the EDR headroom moves with the brightness, and Windows nothing when the SDR slider moves, so Skathi reads the display as it draws: on macOS every frame, on Windows the monitor and DXGI's state every frame and the display configuration once a second (at once on a move to another monitor or a display change). While a surface is visible and nothing else draws, it looks four times a second. A change smaller than 1 % is not reported. Windows does not say whether a screen refreshes at a variable rate, Linux reports nothing yet, and there is no vsync timestamp.

### A video player

A player that tone-maps HDR video itself sizes its output to `output_headroom`. With libmpv, whose linear output has 1.0 at its 203-nit reference white, that is `target-peak`; `display-fps-override` feeds its display-synced playback:

```rust
fn retarget(player: &mpv::Handle, display: skathi::SurfaceDisplay) {
    // 1.0 in the surface is SDR white, and mpv's reference white is 203 nits.
    let peak = 203.0 * display.output_headroom;
    player.set_property("target-peak", format!("{peak:.0}"));
    if let Some(hz) = display.refresh_hz {
        player.set_property("display-fps-override", format!("{hz:.3}"));
    }
}
```

Today `output_headroom` is 1.0, so mpv tone-maps HDR video down to SDR white, which is all a frame can show. Once windows output HDR it follows `headroom`, and the same code gives highlights the room the screen has. Render with `target-trc=linear` and `target-prim=bt.709` into an `RGBA16F` target to hand the frames over as [half floats](#half-float-frames).

## Device loss

When the GPU driver resets on Windows, Skathi recreates its device; on macOS the same happens if a window comes up on another GPU. Every surface drops its GPU frame, the elements fire `surfacelost` with `reason: "device"`, `on_change` hears `SurfaceEvent::DeviceLost`, and a present with an old texture returns `SurfaceError::DeviceChanged`. Get the device again, recreate your textures (or reopen shared ones), and present. CPU frames are not affected.

## Errors

| `SurfaceError`   | Means                                                                                              |
| ---------------- | -------------------------------------------------------------------------------------------------- |
| `NotSupported`   | This platform or renderer has no such path; the message says which one to use                      |
| `NoDevice`       | No window has drawn yet, so there is no GPU device; try again after the first frame                |
| `DeviceChanged`  | The device was recreated; recreate your GPU resources                                              |
| `WrongDevice`    | The texture is from another device; share it instead                                               |
| `Format`, `Size` | The frame's format is not one a surface takes, it is zero or oversized, or the buffer is too short |
| `FormatMismatch` | The frame is BGRA8 and the surface is set to `Rgba16Float`, or the other way round                 |
| `Timeout`        | A keyed mutex or fence was not released in time; nothing was presented                             |
| `Busy`           | Every surface texture is still in flight; drop the frame or wait                                   |
| `Platform`       | A system call failed; the message carries its error                                                |

## Platform support

| Platform | `<sk-surface>` | CPU frames | GPU textures on Skathi's device       | Shared textures                           | Video frames                     | Half-float frames                |
| -------- | -------------- | ---------- | ------------------------------------- | ----------------------------------------- | -------------------------------- | -------------------------------- |
| Windows  | yes            | yes        | D3D11; D3D12 under the D3D12 renderer | NT handles, with a keyed mutex or a fence | no                               | CPU, every GPU path              |
| macOS    | yes            | yes        | Metal, with an `MTLSharedEvent`       | IOSurfaces, with an `MTLSharedEvent`      | CVPixelBuffer: BGRA, 4:2:0, RGhA | CPU, every GPU path except 4:2:0 |
| Linux    | yes            | yes        | no                                    | no                                        | no                               | CPU                              |

Next: [GPU frames on Windows](/embedding/gpu-frames/), [GPU frames on macOS](/embedding/gpu-frames-macos/).
