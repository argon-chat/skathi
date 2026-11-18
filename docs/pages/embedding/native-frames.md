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

Nothing waits for the UI. A present stores the frame and returns; the element takes the newest one on its next paint, and only while it is visible: a frame presented under `display: none`, outside a scroll clip or in a hidden window costs no upload. A producer that presents faster than the screen refreshes only loses frames, never time: since 0.5, while the newest GPU frame is still being copied, the element shows the newest frame whose copy has finished, so a producer at or above the refresh rate drops frames instead of freezing. `stats().shown` against `stats().presented` tells how many reach the screen; [`on_frame`](#knowing-when-a-frame-is-on-screen) tells which, and when.

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
- **macOS** switches the window's layer to `RGBA16Float`, tagged extended sRGB (extended Display P3 for a window that asked for Display P3), and asks for EDR. Going back restores the colour space the layer had. Since 0.5 the tag is what the layer holds; before, it was the screen's own colour space, which over-saturated colours on a P3 panel, and the layer stayed untagged after it left EDR.

Everything else in the window looks as it did. The page is still blended in sRGB-encoded values, as on an 8-bit target, now into a half-float one. On Windows a last pass makes it linear, with 1.0 at the SDR white Windows composes SDR content at (`sdr_white_nits / 80` in scRGB); on macOS the layer takes the encoded values as they are, extended sRGB being sRGB's curve continued past 1.0, and EDR's 1.0 already is SDR white. Skathi's renderer tests draw text, translucent fills, antialiased corners and half-float tiles both ways on Direct3D 11, Direct3D 12, wgpu and Metal: every pixel of the HDR page is within one 8-bit code value of the standard one, and the output at SDR white encodes back to it within one code value. A tile of 4.0 limited to 2.5 shows at 2.5 times SDR white, and under `standard` at SDR white.

A switch recreates the swap chain or reconfigures the layer, once each way; on macOS a new headroom tag only retags the layer. On macOS 14 and later the frame that switches was handed a drawable of the old format; since 0.5 it is drawn in that format, so neither side of a switch shows a black or garbled frame. In HDR a window keeps a half-float copy of the page, and its swap chain is half float too, so it takes about three times the memory of an 8-bit one. At 3840×2160 with 96 full-window translucent layers, a Direct3D 12 frame took 23.4 ms in HDR against 23.2 ms standard on an RTX 5070.

On an SDR display, with HDR off, on Linux, and while no element needs it, a window draws exactly as before. The wgpu renderer can switch where its surface offers extended linear sRGB, but Linux windows stay SDR.

### Whether a window draws HDR (since 0.5)

[`output_headroom`](#the-display) and the `(dynamic-range: high)` media query say whether a window can show HDR; [`output_hdr`](#the-display) says whether it does now. A switch either way fires `SurfaceEvent::DisplayChanged` and `surfacedisplaychange` (`outputHdr`), and `SK_LOG=info` logs it. A player that sets `dynamic-range-limit: standard` on SDR content to keep the window out of HDR sees `output_hdr` turn false two seconds later.

On macOS a screen's `headroom` stays 1.0 until some window on it asks for EDR, so a window decides by the screen's `potential_headroom` instead: it asks for EDR as soon as a visible half-float element's `dynamic-range-limit` allows more than 1.0 there, whatever else the screen shows. The same holds after a pause: when nothing has shown EDR for a while the system lets the headroom fall back to 1.0, and an element that needs HDR again, such as one whose limit goes back to `no-limit`, brings the window back into EDR. `output_headroom` is the current `headroom` and follows it as the system raises it, from 1.0 at the switch. Since 0.5 the element is painted with the headroom read on each frame, so highlights already in the frame brighten with the system's ramp, not in the steps it is reported in.

### How far macOS raises the headroom (since 0.5)

macOS raises a screen's headroom above 1.0 only while a window on it asks for EDR, and decides how far. On macOS 26 and later the window sets its layer's `preferredDynamicRange` to high and tags the layer's `contentsHeadroom` with the most that any such element's `dynamic-range-limit` allows at the screen's `potential_headroom`: 2.0 under `constrained`, the screen's maximum under `no-limit`, and in between for a `dynamic-range-limit-mix()`. The system then raises the headroom no further than that tag, so a window that only needs 2.0 does not push the screen to its maximum. The tag follows the elements while the window stays in HDR, and when the window goes back to standard output it sets the layer to standard range and clears the tag. Before macOS 26 the window sets `wantsExtendedDynamicRangeContent` instead, and the system may then raise the headroom up to the screen's maximum whatever the limit is. The element is still clamped to its limit. Which of the two the window uses is decided when the app runs, so one build covers both.

## The display

`info().display` is a `SurfaceDisplay`: the screen the largest element showing the surface is on. `on_change` hears `SurfaceEvent::DisplayChanged` when it is first known, when it changes, and when that element moves to another screen; each element fires `surfacedisplaychange` for its own. While no element shows the surface, the last one known stays.

Brightness is in the surface's unit, where 1.0 is SDR white (paper white), on every platform:

| Field                          | Means                                                                                                                                                                                                                                        |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `headroom`                     | the brightest value the screen shows now; 1.0 on an SDR screen                                                                                                                                                                               |
| `potential_headroom`           | the brightest it can show; on macOS at full brightness, on Windows the same as `headroom`                                                                                                                                                    |
| `output_headroom`              | the brightest a frame reaches the screen with: `headroom` where the window can draw HDR, else 1.0                                                                                                                                            |
| `output_hdr`                   | whether the element's window draws HDR now (half-float swap chain or layer); see [HDR windows](#hdr-windows)                                                                                                                                 |
| `sdr_white_nits`               | SDR white in nits, on Windows in HDR mode (the "SDR content brightness" slider); `None` elsewhere                                                                                                                                            |
| `peak_nits`, `full_frame_nits` | the screen's luminance, where Windows knows it                                                                                                                                                                                               |
| `hdr`                          | Windows' HDR mode is on, or the macOS screen has EDR headroom                                                                                                                                                                                |
| `refresh_hz`                   | Windows: the current mode's rate; macOS: the screen's maximum (`maximumFramesPerSecond`)                                                                                                                                                     |
| `variable_refresh`             | macOS: whether the screen's refresh interval has a range (ProMotion, Adaptive-Sync); `None` on Windows                                                                                                                                       |
| `refresh_period`               | the refresh period: since 0.5 on Windows, measured from Skathi's own vblank waits on the window's display; elsewhere, and until then, the nominal one from `refresh_hz`                                                                      |
| `last_vblank`                  | when the latest vblank was, in [`monotonic_ns`](#the-clock)'s clock, read as `info()` is called: never ahead of that moment, and at most `refresh_period` before it. Since 0.5 it stays known once it was, through a refresh-rate change too |
| `known`                        | `false` on Linux, and before the window first draws                                                                                                                                                                                          |

1.0 as SDR white is what macOS EDR values already are, and how Windows composes SDR content in HDR mode: at the white level the user picked. On macOS `headroom` is 1.0 until a window on the screen asks for EDR, then what the system grants, which follows the brightness and, from macOS 26, goes no higher than the window's tag unless something else on the screen asks for more (see [HDR windows](#hdr-windows)). On Windows, `headroom` is the screen's peak over that white (`MaxLuminance` / SDR white), and a scRGB value, where 1.0 is 80 nits, is a surface value times `sdr_white_nits / 80`. In SDR mode it is 1.0.

macOS sends nothing when the EDR headroom moves with the brightness, and Windows nothing when the SDR slider moves, so Skathi reads the display as it draws: on macOS every frame, on Windows the monitor and DXGI's state every frame and the display configuration once a second (at once on a move to another monitor or a display change). While a surface is visible and nothing else draws, it looks four times a second. A change smaller than 1 % is not reported. Since 0.5 a level that keeps moving, as macOS's headroom does for a second or two after a window enters or leaves EDR, is reported at most every 200 ms and once more where it settles, so a producer that re-derives its tone mapping on each event is not run on every frame of the ramp. Only the reports are spaced: since 0.5 the clamp an HDR window paints a surface with follows the headroom every frame, while `content_headroom()`, like `info().display`, moves with the reports. `known`, `hdr` and `output_hdr` are reported at once, and a `variable_refresh` change once it has held for 100 ms: macOS's refresh range can blink for a few milliseconds. Windows does not say whether a screen refreshes at a variable rate, and Linux reports nothing yet.

`refresh_period` and `last_vblank` are the display clock, so a player can aim at a vblank without a thread of its own: the next ones are `last_vblank + k × refresh_period`, for k ≥ 1. Since 0.5 on Windows both come from the vblank waits of Skathi's vsync thread (`IDXGIOutput::WaitForVBlank`) on the window's display, which return within about 0.1 ms of its vblank, and are used while their rate matches the surface's screen. DWM's clock (`DwmGetCompositionTimingInfo`), which they came from before, follows one display only, and against another display of nearly the same rate it drifts through the whole refresh; it is now the fallback while the waits have not measured the display. Since 0.5 `last_vblank` is the latest vblank already past on Windows too; before, it was usually the coming one. Elsewhere, and on a screen of another rate, `last_vblank` is carried forward from the last frame [`on_frame`](#knowing-when-a-frame-is-on-screen) saw shown. Since 0.5 a refresh-rate change does not make it `None`: the last vblank known is carried forward at the new rate until the display is measured again. It is `None` only until the first vblank is known.

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
- **`Presented` once, on first display.** `shown_at` is the start of the refresh the frame first showed on; a frame left on screen is not reported again. It stayed until the next frame's `shown_at`, so the difference is how long it was held. `refresh_interval` is that refresh's actual distance from the one before, which is not the nominal period on a variable-refresh screen; `info().display.refresh_period` keeps the display's steady one.
- **`Dropped` says why:**

| `FrameDropReason` | When                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Superseded`      | a newer frame replaced it before it reached the screen: presented twice within a refresh, or before the window drew it. Reported as soon as that is known; since 0.5 a GPU frame counts as replaced once a newer one has finished copying, since until then the window may still show it. Since 0.5 on macOS, a frame whose window present came back without a `presentedTime` counts as replaced only when a later present was aimed at the same refresh |
| `Hidden`          | nothing showing the surface could be on screen (`info().visible` is false, or the window is minimised, covered or hidden): what was waiting drops then, and every present while hidden drops at once. Since 0.5 on Windows this includes a covered window that has the focus, and every window while the system has turned the display off, where a present would otherwise come back as shown; `info().visible` turns false with the first such frame    |
| `DeviceLost`      | the renderer lost its device: everything waiting drops, before `surfacelost`. Since 0.5 on Windows a removed device drops them as soon as the removal is seen, before a new device exists                                                                                                                                                                                                                                                                 |
| `Closed`          | the window showing it closed, or the app quit                                                                                                                                                                                                                                                                                                                                                                                                             |

- **No timeout is needed.** A frame resolves when the display shows a window present carrying it, or when one of the above makes that impossible. Only a stalled UI thread, which draws no window, can hold a frame unresolved; a newer present still supersedes it at once.

### Precision

`precision` says how `shown_at` was known:

| Platform               | `Exact`                                                                                                     | `Estimated`                                                                                                                                                                                                                                                                                                          |
| ---------------------- | ----------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows                | the swap chain's frame statistics named the window present that showed it, and the refresh, at its QPC time | the statistics lagged (DWM refreshes a composed window's only now and then) or a poll was missed: placed as many refreshes after the first vblank following its GPU work as the last `Exact` frames took (since 0.5; one before, and until there are any), ±1 refresh; `refresh_interval` is then the nominal period |
| Windows, no statistics |                                                                                                             | a swap chain whose statistics stay empty: every frame, the same way                                                                                                                                                                                                                                                  |
| macOS                  | the drawable's `presentedTime`; `refresh_interval` is the nominal period                                    | since 0.5, a drawable macOS hands back without a `presentedTime`, as it can at sparse presents and during a live resize, that no later present was aimed at the same refresh: placed on the refresh it was aimed at, the display link's target or, while the window is resized, a refresh after its present          |
| Linux                  |                                                                                                             | about a refresh after the window's tick                                                                                                                                                                                                                                                                              |

On Windows, gpui presents with `Present(0)` on a flip-model swap chain. Its vsync thread waited for each refresh in `DwmFlush`, which returns when DWM is done composing, anywhere in the refresh; since 0.5 it waits on the vblank of the display showing the first shown window (`IDXGIOutput::WaitForVBlank`), with `DwmFlush` as the fallback, so the tick keeps its place in the refresh, 0.08 to 0.17 of it after the vblank, at low present rates too. DWM silently discards a present that a newer one replaced before its vblank. Skathi records which frame of each surface every window present carries, and its vsync thread reads `GetFrameStatistics` (`PresentCount`, `PresentRefreshCount`, `SyncQPCTime`) every refresh while any frame waits, and once more after each present. A frame is `Presented` by the first window present the display showed that carried it; a present the statistics skipped over was discarded, unless a refresh was free for it between its neighbours, in which case it is placed there as `Estimated`. While no frame waits, the vsync thread sleeps as before, so an idle window costs nothing.

### Threading

The callback runs on Skathi's vsync thread on Windows and on a thread of Skathi's own on macOS and Linux, never on the UI thread, and the UI's queue is never in the way: it comes at most a refresh after the frame was shown. The vsync thread paces every window, so keep the callback short: record, signal, return. It may present, or drop its own watch. `Closed` events at quit are delivered on the thread that quits.

### The clock

`shown_at` and `info().display.last_vblank` are in `skathi::monotonic_ns()`'s clock, the one mpv's `mp_raw_time_ns` reads: QPC on Windows and `mach_absolute_time` on macOS, both in nanoseconds, and `CLOCK_MONOTONIC` on Linux. libmpv's own `mpv_get_time_ns` counts from an arbitrary start; take the offset between the two once.

### mpv: display-sync and report_swap

With the render API, libmpv draws each frame when told to and paces playback itself. `video-sync=display-resample` times frames to the display: it needs the refresh rate, and one iteration of its loop per refresh. After each frame it waits until `mpv_render_context_report_swap` is called, and it measures the display from when those calls come. So call it from the render thread, right after each render, and drive that thread from the display clock:

1. Set `display-fps-override` to `info().display.refresh_hz`, and again on `DisplayChanged`.
2. Once per refresh, wait for `MPV_RENDER_UPDATE_FRAME`, then sleep until the render should start, computed from `info().display.last_vblank` and `refresh_period`: early enough that the present lands near the middle of the refresh (see [when to render](#when-to-render)).
3. Ask libmpv what comes next with `MPV_RENDER_PARAM_NEXT_FRAME_INFO`. A repeat (`MPV_RENDER_FRAME_INFO_REPEAT` without `MPV_RENDER_FRAME_INFO_REDRAW`) is the frame already on screen: render with `MPV_RENDER_PARAM_SKIP_RENDERING` and present nothing. Anything else is rendered into the next texture and presented.
4. Call `report_swap` right after each render call, skipped or not. Use `on_frame` for statistics only.

```rust
use std::time::{Duration, Instant};
use skathi::{FrameDropReason, FrameEvent, SurfaceEvent};

let surface = skathi::surface("player");

mpv.set_property("video-sync", "display-resample");
if let Some(hz) = surface.info().display.refresh_hz {
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

// Statistics only: what reached the screen, and when.
let _frames = surface.on_frame(move |event| match event {
    FrameEvent::Presented { shown_at, refresh_interval, .. } => {
        stats.shown(shown_at, refresh_interval)
    }
    FrameEvent::Dropped { reason: FrameDropReason::Superseded, .. } => stats.late(),
    FrameEvent::Dropped { .. } => {}
});

// The render thread (`render` wraps mpv_render_context).
let mut render_time = Duration::from_millis(2); // measured below
loop {
    render.wait_for_frame(); // MPV_RENDER_UPDATE_FRAME
    let display = surface.info().display;
    // `None` only before the first vblank is known: render at once.
    if let (Some(vblank), Some(period)) = (display.last_vblank, display.refresh_period) {
        let p = period.as_nanos() as u64;
        let start = (0.5 - render_time.as_secs_f64() / period.as_secs_f64()).max(0.0);
        let first = vblank + (p as f64 * start) as u64;
        let now = skathi::monotonic_ns();
        let at = first + now.saturating_sub(first).div_ceil(p) * p;
        std::thread::sleep(Duration::from_nanos(at - now));
    }
    let next = render.next_frame_info(); // MPV_RENDER_PARAM_NEXT_FRAME_INFO
    if next.repeat && !next.redraw {
        render.skip(); // mpv_render_context_render with MPV_RENDER_PARAM_SKIP_RENDERING = 1
    } else {
        let began = Instant::now();
        let texture = ring.next();
        render.render_into(texture); // mpv_render_context_render on the shared device
        unsafe { surface.present_d3d11_texture(texture.as_raw()) }?;
        render_time = (render_time * 7 + began.elapsed()) / 8;
    }
    render.report_swap();
}
```

Calling `report_swap` from `on_frame`, as this page used to, does not work: libmpv waits for each report before its next iteration, and a frame's event comes when it is on screen, about 3 refreshes after its present on Windows and about 5 on macOS. libmpv's loop then runs at a fraction of the refresh rate, it estimates a display of 30 to 170 Hz, and plays slowly or keeps delaying frames. A report from `on_frame` only works where a present reaches the screen within a refresh. Presenting the repeats costs a copy and a window draw each refresh for nothing new on screen.

`last_vblank` is the latest vblank already past since 0.5; on Windows in 0.4 it was usually the coming one. The `saturating_sub` above lands on a refresh either way.

Reported this way, libmpv takes a frame to flip when its render returns, a constant few refreshes before it is seen. `shown_at` minus the time of the report says how much on a given machine; `audio-delay` can make up for it. One user measured about 2.4 refreshes on Windows, 10 ms at 240 Hz, and about 45 ms on a 120 Hz Mac; that has not been checked against real audio output. `crates/sk-app/examples/surface.rs` (`feedbackStart`, `feedbackStop`) presents at a fixed rate paced to `last_vblank` and prints every event and the cadence: 24 fps on a 165 Hz screen holds frames for 7, 7, 7, 7, 7, 7, 7 and 6 refreshes, over and over.

### When to render (since 0.5)

A present is taken into the window by the window's next tick. On Windows the tick runs 0.08 to 0.17 of a refresh after the vblank, and a present that lands near it races it: some frames make that tick and some the next, and frames that should be held evenly are held a refresh more or less. A present that lands near the middle of the refresh is clear of the tick on both sides, so start the render that much earlier, at `0.5 − render_time / refresh_period` of the refresh after the vblank.

Measured on a 165 Hz screen, the share of frames shown on the refresh they were meant for, by where in the refresh the render started:

| Render starts at | On the intended refresh |
| ---------------- | ----------------------- |
| 0.05             | 83–99 %                 |
| 0.3              | 95–97 %                 |
| 0.6              | 99.7–100 %              |

The rule also accounts for what has worked elsewhere: 0.05 for a 3–4 ms render on a 120 Hz Mac, and 0.3 for 2160p HDR frames at 240 Hz. Measure your render time on the machine it runs on, from the start of the render until the present returns, as the loop above does; a fixed start that suits one machine or one video can land on the tick on another.

## Device loss

When the GPU driver resets on Windows, Skathi recreates its device; on macOS the same happens if a window comes up on another GPU. Every surface drops its GPU frame, the elements fire `surfacelost` with `reason: "device"`, `on_change` hears `SurfaceEvent::DeviceLost`, and a present with an old texture returns `SurfaceError::DeviceChanged`. Get the device again, recreate your textures (or reopen shared ones), and present. CPU frames are not affected.

### A removed device on Windows (since 0.5)

A driver reset (TDR), a driver update or a GPU that goes away removes the renderer's device. D3D12 makes no new device for that GPU while anything still references the removed one, so Skathi has everything let go of it before it makes the next:

1. At the next refresh, or the next GPU present if the app is idle, every frame still waiting drops as `DeviceLost`, the elements fire `surfacelost`, and `on_change` hears `SurfaceEvent::DeviceLost`.
2. Until there is a new device, a GPU present returns `SurfaceError::DeviceChanged`, and `d3d11_device()`, `d3d12_device()`, `release_fence()` and opening a `SharedTexture` return `SurfaceError::NoDevice`. CPU frames still present.
3. Drop everything you have from the old device: `D3D11Device`, `D3D12Device`, `D3D12Fence`, and the textures and fences you made on it. Each `SharedTexture` already let go of what Skathi opened, so a present with it returns `DeviceChanged`; open it again later.
4. When the new device is ready, `on_change` hears `SurfaceEvent::DeviceLost` a second time. Get the device again and present as after any device loss.

A producer with a device of its own (libmpv, another process) keeps its device; only the `SharedTexture` is opened again. A release fence value returned for a frame that dropped is never signalled: wait on it with a timeout.

Skathi keeps trying for about a minute. If no device can be made by then, the app says so (a message box when it runs on its own) and quits: the exe with exit code 1, and an embedded app's `Exit` has `clean: false`.

## Errors

| `SurfaceError`   | Means                                                                                                                                                                                    |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `NotSupported`   | This platform or renderer has no such path; the message says which one to use                                                                                                            |
| `NoDevice`       | No window has drawn yet, so there is no GPU device; try again after the first frame. Since 0.5 also while a removed device is being replaced; try again after `SurfaceEvent::DeviceLost` |
| `DeviceChanged`  | The device was recreated; recreate your GPU resources. Since 0.5 also for a GPU present while the device is removed                                                                      |
| `WrongDevice`    | The texture is from another device; share it instead                                                                                                                                     |
| `Format`, `Size` | The frame's format is not one a surface takes, it is zero or oversized, or the buffer is too short                                                                                       |
| `FormatMismatch` | The frame is BGRA8 and the surface is set to `Rgba16Float`, or the other way round                                                                                                       |
| `Timeout`        | A keyed mutex or fence was not released in time; nothing was presented                                                                                                                   |
| `Busy`           | Every texture of this surface is still in flight; drop the frame or wait                                                                                                                 |
| `Platform`       | A system call failed; the message carries its error                                                                                                                                      |

## Platform support

| Platform | `<sk-surface>` | CPU frames | GPU textures on Skathi's device       | Shared textures                           | Video frames                     | Half-float frames                |
| -------- | -------------- | ---------- | ------------------------------------- | ----------------------------------------- | -------------------------------- | -------------------------------- |
| Windows  | yes            | yes        | D3D11; D3D12 under the D3D12 renderer | NT handles, with a keyed mutex or a fence | no                               | CPU, every GPU path              |
| macOS    | yes            | yes        | Metal, with an `MTLSharedEvent`       | IOSurfaces, with an `MTLSharedEvent`      | CVPixelBuffer: BGRA, 4:2:0, RGhA | CPU, every GPU path except 4:2:0 |
| Linux    | yes            | yes        | no                                    | no                                        | no                               | CPU                              |

Next: [GPU frames on Windows](/embedding/gpu-frames/), [GPU frames on macOS](/embedding/gpu-frames-macos/).
