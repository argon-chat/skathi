---
title: "Inline <svg>"
label: "<svg>"
metaTitle: "Inline <svg>: how Skathi draws SVG in a template"
description: Inline <svg> is one leaf element whose subtree is serialized and drawn by resvg. Its two paint paths, what CSS reaches the shapes, what a reactive change costs, sizing, events and the limits against a browser.
section: Elements
order: 8
---

Icons are the everyday use of inline `<svg>`: the close button, the toolbar, the chevron in a menu, a logo, a small chart or progress ring. Paste the SVG into a template and it draws, sharp at any scale and on any display.

Skathi does not lay the SVG out as elements. It serializes the `<svg>` and its children into an SVG document, rasterizes it with [resvg](https://github.com/linebender/resvg) at the size of the `<svg>`'s box in device pixels, and paints the result. An icon drawn only in `currentColor` is rasterized once as a shape and tinted with the CSS `color`, so a hover color change costs no new rasterization. To layout and to input, the whole `<svg>` is one box, and that is the one thing to keep in mind: there is no hit testing on a single shape, and state selectors do not restyle the children.

## An icon that follows the text color

Write the icon's paint as `currentColor` and put the state rules on the button, not on the paths. The icon then takes the button's color, in every state and theme.

```vue title="CloseButton.vue" framework="vue"
<template>
  <button class="icon-button" aria-label="Close">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  </button>
</template>

<style scoped>
.icon-button {
  color: #888;
}
.icon-button:hover {
  color: white;
}
.icon-button svg {
  width: 16px;
  height: 16px;
}
</style>
```

```svelte title="CloseButton.svelte" framework="svelte"
<button class="icon-button" aria-label="Close">
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <path d="M6 6l12 12M18 6 6 18" />
  </svg>
</button>

<style>
  .icon-button {
    color: #888;
  }
  .icon-button:hover {
    color: white;
  }
  .icon-button svg {
    width: 16px;
    height: 16px;
  }
</style>
```

SVG's default fill is black, which is a concrete color: a `<path>` with no `fill` takes the full-color path described [below](#two-ways-to-paint). Write `fill="currentColor"`, or `fill="none"` with `stroke="currentColor"` as here, for an icon that follows the text color. A `color` transition on the button animates the icon too.

## A progress ring bound to state

An attribute bound to reactive state works: change it and the SVG is drawn again. A ring that shows upload progress is two circles and a `stroke-dashoffset`:

```vue title="ProgressRing.vue" framework="vue"
<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ progress: number }>();
const length = 2 * Math.PI * 16;
const offset = computed(() => length * (1 - props.progress));
</script>

<template>
  <svg class="ring" viewBox="0 0 40 40" fill="none" stroke="currentColor" stroke-width="4">
    <circle cx="20" cy="20" r="16" opacity="0.2" />
    <circle cx="20" cy="20" r="16" :stroke-dasharray="length" :stroke-dashoffset="offset" />
  </svg>
</template>

<style scoped>
.ring {
  width: 40px;
  height: 40px;
  color: #30a46c;
}
</style>
```

```svelte title="ProgressRing.svelte" framework="svelte"
<script lang="ts">
  let { progress }: { progress: number } = $props();
  const length = 2 * Math.PI * 16;
  const offset = $derived(length * (1 - progress));
</script>

<svg class="ring" viewBox="0 0 40 40" fill="none" stroke="currentColor" stroke-width="4">
  <circle cx="20" cy="20" r="16" opacity="0.2" />
  <circle cx="20" cy="20" r="16" stroke-dasharray={length} stroke-dashoffset={offset} />
</svg>

<style>
  .ring {
    width: 40px;
    height: 40px;
    color: #30a46c;
  }
</style>
```

Each change rasterizes the whole SVG again. For a value that moves a few times a second that is fine. For something that changes every frame, draw it on a [`<canvas>`](/elements/canvas/), or animate the `<svg>` element itself with CSS, as a spinner that rotates, instead of its attributes.

## Things to watch for

- **Clicks land on the `<svg>`, not on a shape.** A listener on a `<path>` or `<circle>` does not see itself as the target. Put a separate element over each part that needs its own handler, or compute the part from the pointer's position.
- **`:hover` on a child does not recolor it.** Put the state rule on the `<svg>` or an ancestor and set `color`.
- **Give it a size.** An unsized `<svg>` has no size of its own, not the browser's 300 × 150.
- **SMIL does not run.** `<animate>` and friends are kept but the SVG shows its first state.
- **App fonts do not reach `<text>`**, which is drawn with system fonts.

## Two ways to paint

Skathi picks one of two paths for each `<svg>`:

- **One color.** When everything the SVG paints is `currentColor`, Skathi rasterizes it once as a shape and tints it with the element's CSS `color`. A color change, such as a `:hover` rule on the `<svg>` or an ancestor, costs no new rasterization.
- **Full color.** Anything else is rasterized in full color and cached by its source, its size in device pixels and its `currentColor`. These force it: a concrete fill or stroke color, more than one color, a gradient or pattern (`url(#…)`), an `<image>`, a filter, a `<style>` element, or a missing `viewBox`.

Without a `viewBox`, the SVG's user units are the CSS pixels of its box.

## What a change costs

Each time the `<svg>` is built, Skathi hashes its subtree. It serializes the document again only when the hash changes, and a full-color SVG is rasterized again, during paint, when its source, size or `currentColor` changes. A change of `currentColor` alone redraws a full-color SVG at most every 50 ms.

So an attribute bound to reactive state inside an `<svg>` (a `d` that moves, a `width` that grows) rasterizes the whole SVG on every change.

Only a simple one-color icon, made of `<g>`, `<path>`, `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>` and `<polygon>`, can be replayed from the render cache when nothing in it changed. Any other `<svg>` keeps the subtree that holds it out of the render cache, as an `<img>` or a `<canvas>` does.

## CSS on the shapes

CSS rules match the children, and the computed `fill`, `stroke`, `fill-opacity`, `fill-rule`, `stroke-opacity`, `stroke-width`, `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`, `stroke-dasharray`, `stroke-dashoffset` and `shape-rendering` are written into the document, along with the children's `opacity` and `display: none`.

Shape geometry (`x`, `y`, `width`, `height`, `rx`, `ry`, `cx`, `cy`, `r`) and gradient `stop-color` and `stop-opacity` can come from CSS (since 0.6). A child's inline style resolves `var()` using its own cascade. For example, `rect { x: 0.5px; width: calc(100% - 1px) }` follows the nearest SVG viewport as it resizes. A percentage circle radius uses the viewport's normalized diagonal.

The SVG renderer also reads styles written directly on a child or in a `<style>` inside the SVG. For example, `<text style="text-anchor: middle">` centers the label on its x coordinate, and `<path style="filter: grayscale(1)">` grays out the path. Both declarations are ignored in an app stylesheet. For path geometry, set the `d` attribute directly. A scaled stroke keeps scaling even with `vector-effect="non-scaling-stroke"`.

A custom property can carry hover state into SVG geometry and gradient paint (since 0.6). For example, a button with `--accent: red` and `.button:hover { --accent: blue }` can use `<stop style="stop-color: var(--accent)" />`. A rule such as `.gradient-stop { stop-color: var(--accent) }` reads the same inherited value.

Setting [`fill`](/css/fill/), [`stroke`](/css/stroke/) or the other properties above directly in a `:hover`, `:focus`, `:active` or group-state rule on an SVG child emits an advisory. Set `color` on the button and use `currentColor`, or change a custom property that a base rule reads. Animating `color` interpolates `currentColor` paint. A change between two concrete `fill` or `stroke` colors jumps.

A paint server from CSS, such as `fill: url(#gradient)`, or `context-fill`, is drawn in `currentColor` with an advisory. Write `fill="url(#gradient)"` as an attribute instead; that works.

SMIL (`<animate>`, `<set>`, `<animateTransform>`) is serialized but never runs. Animate with CSS on the `<svg>` element itself, or rebuild the attribute from state.

## Size

An `<svg>` has no natural size. Give it a width and a height in CSS, or unitless `width` and `height` attributes, which CSS overrides. On the full-color path the root's own `width` and `height` are replaced by the laid-out size, so the `viewBox` and `preserveAspectRatio` fit the box.

`filter: drop-shadow()` on an `<svg>` follows its shapes, not its box.

## Events

Listeners and focus work on the `<svg>` element. There is no hit testing inside it: a click anywhere in the box reaches the `<svg>`. `getBBox()`, `getTotalLength()` and `getScreenCTM()` do not exist.

## Compared with the browser

- The `<svg>` is one box; its children are not elements to layout or input.
- No per-shape hit testing, no SVG geometry methods.
- State selectors do not set `fill` and `stroke` on the children.
- No SMIL animation.
- No default 300 × 150 size.
- App fonts do not reach `<text>`.

## See also

- [`<img>`](/elements/img/): an SVG file in `<img>` is a fixed-size raster.
- [`fill`](/css/fill/) and [`stroke`](/css/stroke/) in the CSS reference.
- [Best practices](/guides/best-practices/).
