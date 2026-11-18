---
title: "Inline <svg>"
label: "<svg>"
metaTitle: "Inline <svg>: how Skathi draws SVG in a template"
description: Inline <svg> is one leaf element whose subtree is serialized and drawn by resvg. Its two paint paths, what CSS reaches the shapes, what a reactive change costs, sizing, events and the limits against a browser.
section: Elements
order: 8
---

An `<svg>` written in a template is drawn by [resvg](https://github.com/linebender/resvg), not laid out as elements. Skathi serializes the `<svg>` and its children into an SVG document, rasterizes it at the size of the `<svg>`'s box in device pixels, and paints the result. To layout and to input, the whole `<svg>` is one box: its children are not boxes of their own.

That makes icons cheap and sharp at any scale. It also means the children do not behave like DOM elements in a browser: there is no hit testing on a single shape, and state selectors do not restyle them.

## A minimal example

```vue
<template>
  <button class="icon-button" aria-label="Close">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  </button>
</template>

<style scoped>
.icon-button svg {
  width: 16px;
  height: 16px;
}
.icon-button {
  color: #888;
}
.icon-button:hover {
  color: white;
}
</style>
```

Everything the icon paints is `currentColor`, so the button's `:hover` color recolors it without drawing it again.

## Two ways to paint

Skathi picks one of two paths for each `<svg>`:

- **One color.** When everything the SVG paints is `currentColor`, Skathi rasterizes it once as a shape and tints it with the element's CSS `color`. A color change, such as a `:hover` rule on the `<svg>` or an ancestor, costs no new rasterization.
- **Full color.** Anything else is rasterized in full color and cached by its source, its size in device pixels and its `currentColor`. These force it: a concrete fill or stroke color, more than one color, a gradient or pattern (`url(#…)`), an `<image>`, a filter, a `<style>` element, or a missing `viewBox`.

SVG's default fill is black, which is a concrete color: a `<path>` with no `fill` takes the full-color path. Write `fill="currentColor"`, or `fill="none"` with `stroke="currentColor"`, for an icon that follows the text color.

Without a `viewBox`, the SVG's user units are the CSS pixels of its box.

## What a change costs

Each time the `<svg>` is built, Skathi hashes its subtree. It serializes the document again only when the hash changes, and a full-color SVG is rasterized again, during paint, when its source, size or `currentColor` changes. A change of `currentColor` alone redraws a full-color SVG at most every 50 ms.

So an attribute bound to reactive state inside an `<svg>` (a `d` that moves, a `width` that grows) rasterizes the whole SVG on every change.

Only a simple one-color icon, made of `<g>`, `<path>`, `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>` and `<polygon>`, can be replayed from the render cache when nothing in it changed. Any other `<svg>` keeps the subtree that holds it out of the render cache, as an `<img>` or a `<canvas>` does.

## CSS on the shapes

CSS rules match the children, and the computed `fill`, `stroke`, `fill-opacity`, `fill-rule`, `stroke-opacity`, `stroke-width`, `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`, `stroke-dasharray`, `stroke-dashoffset` and `shape-rendering` are written into the document, along with the children's `opacity` and `display: none`. The `style` attribute is kept.

Rules with `:hover`, `:focus`, `:active` or a group selector do not set [`fill`](/css/fill/), [`stroke`](/css/stroke/) or the other properties above on the children; an advisory says so. Recolor through `currentColor` instead: put the state rule on the `<svg>` or an ancestor and set `color`. A transition or animation of `fill` or `stroke` interpolates when the paint is `currentColor`, because it animates `color`; between two concrete colors it jumps.

A paint server from CSS, such as `fill: url(#gradient)`, or `context-fill`, is drawn in `currentColor` with an advisory. Write `fill="url(#gradient)"` as an attribute instead; that works.

SMIL (`<animate>`, `<set>`, `<animateTransform>`) is serialized but never runs, so the SVG shows its first state. Animate with CSS on the `<svg>` element itself, or rebuild the attribute from state.

## Size

An `<svg>` has no natural size. Give it a width and a height in CSS, or unitless `width` and `height` attributes, which CSS overrides. A browser makes an unsized `<svg>` 300 × 150; Skathi gives it no size of its own. On the full-color path the root's own `width` and `height` are replaced by the laid-out size, so the `viewBox` and `preserveAspectRatio` fit the box.

`filter: drop-shadow()` on an `<svg>` follows its shapes, not its box.

## Events

Listeners and focus work on the `<svg>` element. There is no hit testing inside it: a click anywhere in the box reaches the `<svg>`, and a listener on a `<path>` or `<circle>` does not see it as the target. Put a separate element over each part that needs its own handler, or compute the part from the pointer's position. `getBBox()`, `getTotalLength()` and `getScreenCTM()` do not exist.

## Text and fonts

`<text>` is drawn with system fonts. The app's `@font-face` fonts are not available inside an SVG.

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
