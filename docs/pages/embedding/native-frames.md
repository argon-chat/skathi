---
title: "Native frames: <sk-surface>"
label: Native frames
description: Show what a native renderer draws (a video player such as libmpv, a game engine, another process) inside a Skathi element, with Vue or Svelte UI over and around it. The element, its events, and presenting frames from Rust.
section: Embedding
order: 4
since: "0.4"
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

| Event                  | When                                                                                                                            | Carries                                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `surfaceresize`        | the painted content box changes, and on first layout                                                                            | `width`, `height` (physical px), `devicePixelRatio`                                                   |
| `surfaceready`         | the first frame shows                                                                                                           | `width`, `height` of that frame                                                                       |
| `surfacelost`          | the frame goes                                                                                                                  | `reason`: `"cleared"` or `"device"`                                                                   |
| `surfacedisplaychange` | the [display](#the-display) is first known, changes, the window switches to or from HDR output, or the element moves to another | `headroom`, `potentialHeadroom`, `refreshRate`, `sdrWhiteNits`, `hdr`, `outputHdr`, `variableRefresh` |

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
    skathi::SurfaceEvent::DisplayChanged(_) => retarget(&surface.info()),
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

Nothing waits for the UI. A present stores the frame and returns; the element takes the newest one on its next paint, and only while it is visible: a frame presented under `display: none`, outside a scroll clip or in a hidden window costs no upload. A producer that presents faster than the screen refreshes only loses frames, never time. `stats().shown` against `stats().presented` tells how many reach the screen; [`on_frame`](#knowing-when-a-frame-is-on-screen) tells which, and when.

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

### What shows

On a standard window every frame is mapped into the window's 8-bit sRGB: each channel is clamped to 0…1 and then encoded with the exact sRGB curve, the one gradients and video already go through. Values from 0 to 1 are never touched; there is no tone-curve shoulder below 1.0, and anything brighter than SDR white is clipped to it. Two things follow:

- **SDR content looks exactly as on the 8-bit path.** Linearise your 8-bit sRGB frames with the sRGB EOTF and they look identical to the BGRA8 path: Skathi's tests put all 256 sRGB code values through half floats and get every one back unchanged, on Direct3D 11, Direct3D 12 and Metal.
- **Highlights clip.** A producer that has highlights above 1.0 and wants them rolled off rather than clipped tone-maps them itself, to the element's [`content_headroom()`](#a-video-player).

GPU frames stay half float in the surface's own textures; an element maps a frame into an 8-bit texture when it first draws it, on the GPU, so a frame replaced before it is shown is never mapped. CPU frames are mapped when presented, on the producer's thread, through a table. A half-float frame takes twice the memory of an 8-bit one, and the mapping costs one small GPU pass per frame shown.

### HDR windows

New in 0.4.0. On Windows (Direct3D 12 and Direct3D 11) and macOS, a window whose display is in HDR draws HDR while it shows a half-float surface that may be brighter than SDR white: a visible element whose [`dynamic-range-limit`](/css/dynamic-range-limit/) allows more than 1.0 on that display (on macOS, at the screen's `potential_headroom`). The element then shows the frame's own values, clamped to that limit and to the display's headroom, instead of the 8-bit mapping. Two seconds after no such element is visible, the window goes back to standard output.

- **Windows** switches the swap chain to `R16G16B16A16_FLOAT` in scRGB (`DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709`).
- **macOS** switches the window's layer to `RGBA16Float`, in the screen's colour space extended past 1.0 (extended Display P3 for a window that asked for Display P3), and asks for EDR.

macOS raises a screen's headroom above 1.0 only while a window on it asks for EDR, and decides how far. On macOS 26 and later the window sets its layer's `preferredDynamicRange` to high and tags the layer's `contentsHeadroom` with the most that any such element's `dynamic-range-limit` allows at the screen's `potential_headroom`: 2.0 under `constrained`, the screen's maximum under `no-limit`, and in between for a `dynamic-range-limit-mix()`. The system then raises the headroom no further than that tag, so a window that only needs 2.0 does not push the screen to its maximum. The tag follows the elements while the window stays in HDR, and when the window goes back to standard output it sets the layer to standard range and clears the tag. Before macOS 26 the window sets `wantsExtendedDynamicRangeContent` instead, and the system may then raise the headroom up to the screen's maximum whatever the limit is. The element is still clamped to its limit. Which of the two the window uses is decided when the app runs, so one build covers both.

Everything else in the window looks as it did. The page is still blended in sRGB-encoded values, as on an 8-bit target, now into a half-float one, and only a last pass makes it linear, with 1.0 at the SDR white Windows composes SDR content at (`sdr_white_nits / 80` in scRGB); on macOS EDR's 1.0 already is SDR white. Skathi's renderer tests draw text, translucent fills, antialiased corners and half-float tiles both ways on Direct3D 11, Direct3D 12, wgpu and Metal: every pixel of the HDR page is within one 8-bit code value of the standard one, and the output at SDR white encodes back to it within one code value. A tile of 4.0 limited to 2.5 shows at 2.5 times SDR white, and under `standard` at SDR white.

On an SDR display, with HDR off, on Linux, and while no element needs it, a window draws exactly as before. [`output_headroom`](#the-display) and the `(dynamic-range: high)` media query say whether a window can show HDR; [`output_hdr`](#the-display) says whether it does now. A switch either way fires `SurfaceEvent::DisplayChanged` and `surfacedisplaychange` (`outputHdr`), and `SK_LOG=info` logs it. A player that sets `dynamic-range-limit: standard` on SDR content to keep the window out of HDR sees `output_hdr` turn false two seconds later. The wgpu renderer can switch where its surface offers extended linear sRGB, but Linux windows stay SDR.

A switch recreates the swap chain or reconfigures the layer, once each way; on macOS a new headroom tag only retags the layer. In HDR a window keeps a half-float copy of the page, and its swap chain is half float too, so it takes about three times the memory of an 8-bit one. At 3840×2160 with 96 full-window translucent layers, a Direct3D 12 frame took 23.4 ms in HDR against 23.2 ms standard on an RTX 5070.

## The display

`info().display` is a `SurfaceDisplay`: the screen the largest element showing the surface is on. `on_change` hears `SurfaceEvent::DisplayChanged` when it is first known, when it changes, and when that element moves to another screen; each element fires `surfacedisplaychange` for its own. While no element shows the surface, the last one known stays.

Brightness is in the surface's unit, where 1.0 is SDR white (paper white), on every platform:

| Field                          | Means                                                                                                        |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| `headroom`                     | the brightest value the screen shows now; 1.0 on an SDR screen                                               |
| `potential_headroom`           | the brightest it can show; on macOS at full brightness, on Windows the same as `headroom`                    |
| `output_headroom`              | the brightest a frame reaches the screen with: `headroom` where the window can draw HDR, else 1.0            |
| `output_hdr`                   | whether the element's window draws HDR now (half-float swap chain or layer); see [HDR windows](#hdr-windows) |
| `sdr_white_nits`               | SDR white in nits, on Windows in HDR mode (the "SDR content brightness" slider); `None` elsewhere            |
| `peak_nits`, `full_frame_nits` | the screen's luminance, where Windows knows it                                                               |
| `hdr`                          | Windows' HDR mode is on, or the macOS screen has EDR headroom                                                |
| `refresh_hz`                   | Windows: the current mode's rate; macOS: the screen's maximum (`maximumFramesPerSecond`)                     |
| `variable_refresh`             | macOS: whether the screen's refresh interval has a range (ProMotion, Adaptive-Sync); `None` on Windows       |
| `refresh_period`               | the nominal refresh period: the compositor's on Windows, else from `refresh_hz`                              |
| `last_vblank`                  | when the latest vblank was, in [`monotonic_ns`](#the-clock)'s clock, read as `info()` is called              |
| `known`                        | `false` on Linux, and before the window first draws                                                          |

1.0 as SDR white is what macOS EDR values already are, and how Windows composes SDR content in HDR mode: at the white level the user picked. On macOS `headroom` is 1.0 until a window on the screen asks for EDR, then what the system grants, which follows the brightness and, from macOS 26, goes no higher than the window's tag unless something else on the screen asks for more (see [HDR windows](#hdr-windows)). On Windows, `headroom` is the screen's peak over that white (`MaxLuminance` / SDR white), and a scRGB value, where 1.0 is 80 nits, is a surface value times `sdr_white_nits / 80`. In SDR mode it is 1.0.

macOS sends nothing when the EDR headroom moves with the brightness, and Windows nothing when the SDR slider moves, so Skathi reads the display as it draws: on macOS every frame, on Windows the monitor and DXGI's state every frame and the display configuration once a second (at once on a move to another monitor or a display change). While a surface is visible and nothing else draws, it looks four times a second. A change smaller than 1 % is not reported. Windows does not say whether a screen refreshes at a variable rate, and Linux reports nothing yet.

`refresh_period` and `last_vblank` are the display clock, so a player can aim at a vblank without a thread of its own: the next ones are `last_vblank + k × refresh_period`. On Windows both are the compositor's (`DwmGetCompositionTimingInfo`), used while its rate matches the surface's screen. Elsewhere, and on a screen of another rate, `last_vblank` is carried forward from the last frame [`on_frame`](#knowing-when-a-frame-is-on-screen) saw shown, so it is `None` until one was.

### A video player

A player that tone-maps HDR video itself sizes its output to `info.content_headroom()`: `output_headroom` as the element's [`dynamic-range-limit`](/css/dynamic-range-limit/) allows it. With libmpv, whose linear output has 1.0 at its 203-nit reference white, that is `target-peak`; `display-fps-override` feeds its display-synced playback:

```rust
fn retarget(player: &mpv::Handle, info: &skathi::SurfaceInfo) {
    // 1.0 in the surface is SDR white, and mpv's reference white is 203 nits.
    let peak = 203.0 * info.content_headroom();
    player.set_property("target-peak", format!("{peak:.0}"));
    if let Some(hz) = info.display.refresh_hz {
        player.set_property("display-fps-override", format!("{hz:.3}"));
    }
}
```

On an SDR display `output_headroom` is 1.0, so mpv tone-maps HDR video down to SDR white, which is all a frame can show. On an HDR display it follows `headroom`, and the same code gives highlights the room the screen has, no more than `dynamic-range-limit` allows: `standard` keeps them at SDR white, `constrained` at twice it, and a `dynamic-range-limit-mix()` in between, mixed in stops. Render with `target-trc=linear` and `target-prim=bt.709` into an `RGBA16F` target to hand the frames over as [half floats](#half-float-frames).

`info.dynamic_range_limit` is the computed value itself, as weights of the three keywords, for a producer that applies it some other way: `dynamic_range_limit.headroom(display.headroom)` is the limit on the screen's own headroom. The value is the largest element's, and a change to it fires `SurfaceEvent::Changed`.

## Knowing when a frame is on screen

A present returning `Ok` means the frame is queued, not that anyone saw it. A player that keeps its own clock against the display (mpv's display-sync, a game engine's frame pacing) needs to know when each frame actually reached the screen, and which never did. Every present returns the frame's id, and `on_frame` reports what became of it:

```rust
use skathi::{FrameDropReason, FrameEvent, FramePrecision};

let surface = skathi::surface("player");
let _frames = surface.on_frame(|event| match event {
    FrameEvent::Presented { frame, shown_at, refresh_interval, precision } => {
        // `frame` was first on screen on the refresh that began at `shown_at` (ns).
    }
    FrameEvent::Dropped { frame, reason } => {
        // Superseded, Hidden, DeviceLost or Closed: it never reached the screen.
    }
});
let frame = surface.present_bgra(&pixels, w, h, stride)?; // the id that event carries
```

`present_bgra`, `present_bgrx`, `present_rgba16f`, `present_d3d11_texture`, `present_shared`, `present_shared_fenced` and `present_cv_pixel_buffer` return the id; `present_d3d12` and `present_metal_texture` return a `FramePresent` with the id and the release value. Ids increase per surface. The callback is `FnMut + Send`; reports go on until the `FrameWatch` it returns is dropped, and a surface nobody listens to costs nothing.

### What is guaranteed

- **One event per frame.** Each frame presented while a `FrameWatch` is held gets exactly one `FrameEvent`, `Presented` or `Dropped`, never none and never two. Frames presented before `on_frame` or after the last watch dropped are not reported.
- **In frame order.** A surface's events arrive in the order of their ids. A frame that is settled early (superseded at once, say) waits for the ones before it.
- **`Presented` once, on first display.** `shown_at` is the start of the refresh the frame first showed on; a frame left on screen is not reported again. It stayed until the next frame's `shown_at`, so the difference is how long it was held. `refresh_interval` is that refresh's actual distance from the one before, which is not the nominal period on a variable-refresh screen; `info().display.refresh_period` keeps the nominal one.
- **`Dropped` says why:**

| `FrameDropReason` | When                                                                                                                                                                                                 |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Superseded`      | a newer frame replaced it before it reached the screen: presented twice within a refresh, or before the window drew it. Reported as soon as that is known                                            |
| `Hidden`          | nothing showing the surface could be on screen (`info().visible` is false, or the window is minimised, covered or hidden): what was waiting drops then, and every present while hidden drops at once |
| `DeviceLost`      | the renderer lost its device: everything waiting drops, before `surfacelost`                                                                                                                         |
| `Closed`          | the window showing it closed, or the app quit                                                                                                                                                        |

- **No timeout is needed.** A frame resolves when the display shows a window present carrying it, or when one of the above makes that impossible. Only a stalled UI thread, which draws no window, can hold a frame unresolved; a newer present still supersedes it at once.

### Precision

`precision` says how `shown_at` was known:

| Platform               | `Exact`                                                                                                     | `Estimated`                                                                                                                                                                                                                      |
| ---------------------- | ----------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows                | the swap chain's frame statistics named the window present that showed it, and the refresh, at its QPC time | the statistics lagged (DWM refreshes a composed window's only now and then) or a poll was missed: placed on the refresh after the first vblank following its GPU work, ±1 refresh; `refresh_interval` is then the nominal period |
| Windows, no statistics |                                                                                                             | a swap chain whose statistics stay empty: every frame, the same way                                                                                                                                                              |
| macOS                  | the drawable's `presentedTime`; `refresh_interval` is the nominal period                                    |                                                                                                                                                                                                                                  |
| Linux                  |                                                                                                             | about a refresh after the window's tick                                                                                                                                                                                          |

On Windows, gpui presents with `Present(0)` on a flip-model swap chain and paces itself with `DwmFlush`; DWM silently discards a present that a newer one replaced before its vblank. Skathi records which frame of each surface every window present carries, and its vsync thread reads `GetFrameStatistics` (`PresentCount`, `PresentRefreshCount`, `SyncQPCTime`) every refresh while any frame waits, and once more after each present. A frame is `Presented` by the first window present the display showed that carried it; a present the statistics skipped over was discarded, unless a refresh was free for it between its neighbours, in which case it is placed there as `Estimated`. While no frame waits, the vsync thread sleeps as before, so an idle window costs nothing.

### Threading

The callback runs on Skathi's vsync thread on Windows and on a thread of Skathi's own on macOS and Linux, never on the UI thread, and the UI's queue is never in the way: it comes at most a refresh after the frame was shown. The vsync thread paces every window, so keep the callback short: record, signal, return. It may present, or drop its own watch. `Closed` events at quit are delivered on the thread that quits.

### The clock

`shown_at` and `info().display.last_vblank` are in `skathi::monotonic_ns()`'s clock, the one mpv's `mp_raw_time_ns` reads: QPC on Windows and `mach_absolute_time` on macOS, both in nanoseconds, and `CLOCK_MONOTONIC` on Linux. libmpv's own `mpv_get_time_ns` counts from an arbitrary start; take the offset between the two once.

### mpv: display-sync and report_swap

With the render API, libmpv draws each frame when told to and paces playback itself. `video-sync=display-resample` times frames to the display: it needs the refresh rate, and the moments frames actually flipped. `mpv_render_context_report_swap` is how it hears the second: libmpv's VO waits after each frame until the swap is reported, and times the display from when that happens. Called right after a present it would report queueing; called from `on_frame` it reports the flip, within a refresh of it.

```rust
use std::sync::Arc;

use skathi::{FrameDropReason, FrameEvent, SurfaceEvent};

let surface = skathi::surface("player");

mpv.set_property("video-sync", "display-resample");
let display = surface.info().display;
if let Some(hz) = display.refresh_hz {
    mpv.set_property("display-fps-override", format!("{hz:.3}"));
}
let _display = surface.on_change({
    let mpv = mpv.clone();
    move |_, event| {
        if let SurfaceEvent::DisplayChanged(display) = event {
            if let Some(hz) = display.refresh_hz {
                mpv.set_property("display-fps-override", format!("{hz:.3}"));
            }
        }
    }
});

let render: Arc<RenderContext> = render.clone(); // wraps mpv_render_context
let _frames = surface.on_frame(move |event| {
    // One report per frame rendered, shown or not, so libmpv's count of flips stays in step.
    render.report_swap();
    match event {
        FrameEvent::Presented { shown_at, refresh_interval, .. } => {
            stats.shown(shown_at, refresh_interval); // e.g. hold times, for an OSD
        }
        FrameEvent::Dropped { reason: FrameDropReason::Superseded, .. } => stats.late(),
        FrameEvent::Dropped { .. } => {}
    }
});

// The render thread: present mid-refresh, as far from either vblank as it gets, so jitter on
// either side cannot move a frame to another refresh.
loop {
    render.wait_for_frame(); // MPV_RENDER_UPDATE_FRAME
    let display = surface.info().display;
    if let (Some(vblank), Some(period)) = (display.last_vblank, display.refresh_period) {
        let p = period.as_nanos() as u64;
        let now = skathi::monotonic_ns();
        let mid = vblank + p / 2 + now.saturating_sub(vblank + p / 2).div_ceil(p) * p;
        std::thread::sleep(std::time::Duration::from_nanos(mid - now));
    }
    render.render_into(&texture); // mpv_render_context_render on the shared device
    unsafe { surface.present_d3d11_texture(texture.as_raw()) }?;
}
```

Without `report_swap` libmpv assumes each frame flipped when its render returned, which is when the texture was handed over, not when it was seen; with display-sync that turns the window's draw jitter into resampling error. `display-resample` with flips reported from `on_frame` keeps audio and video against the display's real cadence, and a `Superseded` count that grows says frames are being presented faster than the screen takes them. `crates/sk-app/examples/surface.rs` (`feedbackStart`, `feedbackStop`) presents at a fixed rate paced to `last_vblank` this way and prints every event and the cadence: 24 fps on a 165 Hz screen holds frames for 7, 7, 7, 7, 7, 7, 7 and 6 refreshes, over and over.

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
| `Busy`           | Every texture of this surface is still in flight; drop the frame or wait                           |
| `Platform`       | A system call failed; the message carries its error                                                |

## Platform support

| Platform | `<sk-surface>` | CPU frames | GPU textures on Skathi's device       | Shared textures                           | Video frames                     | Half-float frames                |
| -------- | -------------- | ---------- | ------------------------------------- | ----------------------------------------- | -------------------------------- | -------------------------------- |
| Windows  | yes            | yes        | D3D11; D3D12 under the D3D12 renderer | NT handles, with a keyed mutex or a fence | no                               | CPU, every GPU path              |
| macOS    | yes            | yes        | Metal, with an `MTLSharedEvent`       | IOSurfaces, with an `MTLSharedEvent`      | CVPixelBuffer: BGRA, 4:2:0, RGhA | CPU, every GPU path except 4:2:0 |
| Linux    | yes            | yes        | no                                    | no                                        | no                               | CPU                              |

Next: [GPU frames on Windows](/embedding/gpu-frames/), [GPU frames on macOS](/embedding/gpu-frames-macos/).
