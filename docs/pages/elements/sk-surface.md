---
title: "<sk-surface>"
label: "<sk-surface>"
metaTitle: "<sk-surface>: show frames native code renders"
description: The element that shows frames native code renders, such as a video player, a game engine's viewport or another process, composited like any other element. Its attributes, events, sizing and CSS.
section: Elements
order: 2
since: "0.4"
---

`<sk-surface>` shows frames that native code renders: a video player built on libmpv, a game engine's viewport, a renderer in another process. The program that embeds Skathi presents frames to a surface by name, and every `<sk-surface>` with that `name` shows the newest one. In the page it is an ordinary box: CSS sizes it, clips it and moves it, and the app's own controls sit over it.

This page is the element. [Native frames](/embedding/native-frames/) is the producer: the Rust `SurfaceHandle`, CPU and GPU frames, half floats, frame feedback and device loss.

## A minimal example

```vue
<script setup lang="ts">
import { ref } from "vue";

const size = ref("");

function resized(e: SkSurfaceEvent) {
  size.value = `${e.width} × ${e.height}`;
}
</script>

<template>
  <div class="player">
    <sk-surface name="player" class="video" @surfaceresize="resized" />
    <div class="overlay">{{ size }}</div>
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

```svelte
<script lang="ts">
  let size = $state("");
</script>

<div class="player">
  <sk-surface name="player" class="video" onsurfaceresize={(e) => (size = `${e.width} × ${e.height}`)} />
  <div class="overlay">{size}</div>
</div>
```

Vue compiles every `sk-*` tag as a custom element, so no component registration is needed. In Svelte, `@skathi-native/runtime/svelte` types the element's `name` and its four events. In Vue, the events are typed as the global `SkSurfaceEvent` and `SkSurfaceDisplayEvent`.

## Attributes

| Attribute         | What it does                                                                                                   |
| ----------------- | -------------------------------------------------------------------------------------------------------------- |
| `name`            | The surface to show, trimmed. Changing it binds the element to the other surface. An empty name shows nothing. |
| `width`, `height` | Size hints in CSS pixels, unitless, as on `<img>`. CSS wins.                                                   |

That is all the element reads. The pixel format, the color space and the frames themselves are set from Rust. There is no `name` property on the element object: from plain JavaScript, use `setAttribute("name", …)`. Vue's `:name` and Svelte's `name={…}` set the attribute.

The element draws no children. Put controls and overlays next to it, positioned over it, as in the example.

## Events

| Event                  | When                                                                                   | Carries                                                                                  |
| ---------------------- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `surfaceresize`        | the painted size changes, and on first layout                                          | `width`, `height` in physical pixels, `devicePixelRatio`                                 |
| `surfaceready`         | a frame shows for the first time, and again after a lost frame is replaced             | `width`, `height` of that frame                                                          |
| `surfacelost`          | the frame shown goes away                                                              | `reason`: `"cleared"` or `"device"`                                                      |
| `surfacedisplaychange` | the display is first known, its properties change, or the element moves to another one | `headroom`, `potentialHeadroom`, `refreshRate`, `sdrWhiteNits`, `hdr`, `variableRefresh` |

The values are on the event itself and on `detail`. The events do not bubble and cannot be cancelled, and they are only dispatched to an element that has a listener for them.

- `surfaceresize` reports the size in physical pixels: the CSS size times the window's scale, rounded. A producer that renders at that size is 1:1 with the screen. It fires while the element is in the document with a non-zero width, visible or not.
- `surfaceready` carries the frame's size, not the box's, and no `devicePixelRatio`.
- `surfacelost` fires only for an element that has shown a frame. `"cleared"` means the producer called `clear()`; `"device"` means the GPU device was lost and the frame with it. A CPU frame survives a device loss.
- `surfacedisplaychange` fires on Windows and macOS, on a change above 1 %. `refreshRate`, `sdrWhiteNits` and `variableRefresh` are `null` where the platform does not say. [The display](/embedding/native-frames/#the-display) explains the values.

## Size and CSS

The box is 300 × 150 CSS pixels unless CSS or the `width` and `height` attributes size it. Size it as you would a `<video>`: a width and an `aspect-ratio`, or both sides.

The frame is scaled into the box:

- [`object-fit`](/css/object-fit/) defaults to `contain`, as on `<video>`; `cover`, `fill`, `none` and `scale-down` apply too.
- [`object-position`](/css/object-position/) places the frame in the box.
- [`border-radius`](/css/border-radius/) and an ancestor's `overflow` clip it.
- `transform`, `opacity` and `z-index` apply to the picture like to any other element, because the frame is drawn in the page, not in a window of its own.
- [`dynamic-range-limit`](/css/dynamic-range-limit/) caps how bright the element may get on an HDR display. The producer reads the result as its content headroom; see [HDR windows](/embedding/native-frames/#hdr-windows).

Until the first frame arrives the element paints nothing over its own background, so a `background` or a poster image set in CSS shows through. After `surfacelost` it paints nothing again.

## What counts as visible

The element takes a frame on its next paint, and only while it is visible: in the document, in a shown window, not in a hidden page, with a non-zero painted size, and with its painted rectangle inside the viewport and every clipping ancestor. `display: none` counts as hidden. `opacity: 0` and an element covered by another one count as visible.

A frame presented while no element is visible is not uploaded. The newest one shows when an element becomes visible again.

## Several elements, one surface

Any number of elements can name the same surface. Each shows the same newest frame at its own size, a thumbnail and a full view of one stream, for example. The size the producer sees in `info()` is the largest visible element's. A CPU frame is uploaded once for each visible element that shows it.

## Input and focus

The element is an ordinary hit target. Clicks, pointer events and the wheel go to the element and to your listeners, as on a `<div>`; nothing is forwarded to the producer. `pointer-events: none` lets them through to what is under it. With a `tabindex` it takes focus.

## Platforms

| Platform | Frames the element can show                                                                   | `surfacedisplaychange` |
| -------- | --------------------------------------------------------------------------------------------- | ---------------------- |
| Windows  | CPU BGRA; Direct3D 11 and 12 textures; shared textures from another device, process or Vulkan | yes                    |
| macOS    | CPU BGRA; Metal textures, IOSurfaces, CVPixelBuffers                                          | yes                    |
| Linux    | CPU BGRA only                                                                                 | no                     |

On Windows and macOS a half-float surface on an HDR display switches the window to HDR output while it shows. Linux windows stay SDR.

## Compared with the browser

There is no browser element like it. The closest is a `<video>` whose frames come from your own code, or a `<canvas>` you draw into from another thread; unlike both, the producer is native code outside JavaScript, and a present never waits for the UI. For a video file the OS can play, [`<video>`](/elements/video/) needs no Rust at all.

## See also

- [Native frames](/embedding/native-frames/): presenting frames from Rust.
- [GPU frames on Windows](/embedding/gpu-frames/) and [on macOS](/embedding/gpu-frames-macos/).
- [`<sk-surface>` in the web-platform reference](/web-platform/sk-surface/).
- [Native modules](/embedding/native-modules/): giving the app functions to call the producer, such as play and resize.
