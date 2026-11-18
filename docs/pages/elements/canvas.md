---
title: "<canvas>"
label: "<canvas>"
metaTitle: "<canvas>: 2D and WebGPU drawing in Skathi"
description: How <canvas> draws, with a 2D context rasterized on the CPU or a WebGPU context on Direct3D 12, Metal or Vulkan, how its bitmap reaches the screen, sizing and device pixels, CSS, limits and what differs from a browser.
section: Elements
order: 9
---

`<canvas>` is a bitmap your script draws into, with a `"2d"` or a `"webgpu"` context. The element shows the bitmap like an image: CSS sizes, clips and transforms it. WebGL is not available; `getContext("webgl")` and `getContext("webgl2")` return `null` with an advisory.

The APIs themselves are in the web-platform reference: [Canvas 2D](/web-platform/canvas-2d/), [OffscreenCanvas](/web-platform/offscreen-canvas/) and [WebGPU](/web-platform/webgpu/), with examples and edge cases. This page is the element: how its pixels get to the screen, and what that means for size and CSS.

## A minimal example

```vue
<script setup lang="ts">
import { onMounted, ref } from "vue";

const W = 320;
const H = 80;
const dpr = devicePixelRatio;
const canvas = ref<HTMLCanvasElement | null>(null);

onMounted(() => {
  const ctx = canvas.value!.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.fillStyle = "#4f8cff";
  ctx.fillRect(0, 0, W / 2, H);
});
</script>

<template>
  <canvas ref="canvas" class="chart" :width="W * dpr" :height="H * dpr" />
</template>

<style scoped>
.chart {
  width: 320px;
  height: 80px;
}
</style>
```

```svelte
<script lang="ts">
  const W = 320;
  const H = 80;
  const dpr = devicePixelRatio;
  let canvas: HTMLCanvasElement;

  $effect(() => {
    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);
    ctx.fillStyle = "#4f8cff";
    ctx.fillRect(0, 0, W / 2, H);
  });
</script>

<canvas bind:this={canvas} class="chart" width={W * dpr} height={H * dpr}></canvas>
```

## The element

| Attribute or property     | What it does                                                                                                                                                    |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `width`, `height`         | The bitmap's size in pixels, 300 × 150 by default. Setting either, even to the same value, clears the bitmap and resets the context.                            |
| `getContext(id)`          | `"2d"` or `"webgpu"`. A canvas keeps the first kind it was given: asking for the other returns `null`. Any other id returns `null` and prints an advisory once. |
| `toDataURL()`, `toBlob()` | Encode the bitmap as an image.                                                                                                                                  |

That is the element's whole API. `transferControlToOffscreen()` and `captureStream()` do not exist. An `OffscreenCanvas` works, on the JavaScript thread: there are no workers to send it to.

The element's children, the fallback content a browser shows when canvas is unsupported, are never drawn. The canvas takes events, `pointer-events`, `tabindex` and `:focus` styles like any element.

## 2D: how a frame gets drawn

The 2D context records each call as a command; nothing is drawn at the call. The commands are rasterized on the CPU, by vello_cpu, the same on every platform, when the screen needs the bitmap, when script reads pixels back (`getImageData`, `toDataURL`, `toBlob`), when the canvas is drawn into another one, or when 2048 commands are queued. A `clearRect` or `putImageData` over the whole canvas drops the commands before it, and clearing a canvas that is already blank changes nothing.

Rasterizing for the screen runs on the UI thread, in the frame. A frame with no changed canvas costs one check. A changed canvas is copied whole to the GPU: the bitmap is converted to the renderer's format on the CPU and uploaded, not patched.

A canvas that changes again within 250 ms counts as animating and is written into its GPU tile in place, without rebuilding the page. Once it has been still for 300 ms, a canvas shown smaller than its bitmap is drawn from a filtered copy.

Text uses the app's `@font-face` fonts.

## WebGPU: how a frame gets drawn

| Platform | Device                          | To the screen                       |
| -------- | ------------------------------- | ----------------------------------- |
| Windows  | Direct3D 12, the renderer's own | Copied on the GPU                   |
| macOS    | Metal, its own                  | Read back to the CPU, then uploaded |
| Linux    | Vulkan, its own                 | Read back to the CPU, then uploaded |

On Windows, WebGPU shares the renderer's Direct3D 12 device, and each frame is converted and copied into the renderer's texture without leaving the GPU. On macOS and Linux, WebGPU has a device of its own; each frame is read back to the CPU asynchronously and uploaded like a 2D bitmap.

The texture from `getCurrentTexture()` is presented after the frame's `requestAnimationFrame` callbacks and their microtasks have run. On Windows, if the GPU copy is refused, Skathi falls back to the readback with a warning. During a readback, when every staging buffer is busy, the screen keeps the previous frame.

Before its first frame, an opaque WebGPU canvas paints a black box, and a `premultiplied` one paints nothing.

`toDataURL()` and `toBlob()` on a WebGPU canvas read the texture back synchronously on the JavaScript thread.

WebGPU needs the host's `webgpu` Cargo feature, on by default. Without it `navigator.gpu` is undefined and `getContext("webgpu")` returns `null`; see [Features and size](/embedding/features/).

## Size and device pixels

The bitmap size and the box size are separate, as in browsers. Without CSS, the box is the bitmap's size in CSS pixels. With CSS, the bitmap is scaled into the box.

One bitmap pixel is one CSS pixel unless you say otherwise, so on a 2× display an unscaled canvas is blurry. For a sharp canvas, make the bitmap `devicePixelRatio` times the CSS size and scale the context by the same factor, as the example does.

[`aspect-ratio`](/css/aspect-ratio/) in CSS wins over the bitmap's ratio.

A side above 32767 pixels, or an area above 2²⁸ pixels, makes the bitmap 0 × 0 with an advisory. A side above 16384 is drawn and can be read back, but is not shown, with a warning.

## CSS

- [`object-fit`](/css/object-fit/) defaults to `fill`, as in browsers; [`object-position`](/css/object-position/) applies.
- [`border-radius`](/css/border-radius/) and [`corner-shape`](/css/corner-shape/) clip the canvas.
- `transform`, `opacity` and `z-index` apply as to any element.
- [`image-rendering`](/css/image-rendering/) is not supported: the bitmap is always scaled with linear filtering, so pixel art blurs when scaled up.
- The canvas is sRGB only. A `colorSpace` option is ignored, and there is no HDR.

## Performance

A canvas is never skipped when it is off screen, and a subtree that contains one is not replayed from the render cache. Every canvas in a long scrolling list is laid out and painted, in view or not; render only the ones in view, as in [Virtualize long lists](/guides/best-practices/#virtualize-long-lists).

## Compared with the browser

- No WebGL; use WebGPU.
- No `transferControlToOffscreen()`, `captureStream()` or workers.
- `drawImage(video)` draws nothing yet.
- A canvas is never tainted: pixels from cross-origin images can be read back.
- 2D `filter` supports `blur()` only.
- No `image-rendering`, no wide gamut or HDR.

## See also

- [Canvas 2D](/web-platform/canvas-2d/), [OffscreenCanvas](/web-platform/offscreen-canvas/) and [WebGPU](/web-platform/webgpu/) in the web-platform reference.
- [`<sk-surface>`](/elements/sk-surface/): frames from native code.
- [`<img>`](/elements/img/): what `drawImage(img)` can draw.
