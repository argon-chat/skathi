import type { Details } from "../../details";

export const details: Details = {
  "box-shadow": {
    examples: [
      {
        title: "Lift a card off the page",
        lang: "css",
        code: `.card {
  border-radius: 12px;
  background: #ffffff;
  box-shadow:
    0 1px 2px rgb(0 0 0 / 0.06),
    0 8px 24px rgb(0 0 0 / 0.12);
}`,
      },
      {
        title: "Draw a two-color focus ring",
        lang: "css",
        code: `.search:focus-visible {
  box-shadow:
    0 0 0 2px #ffffff,
    0 0 0 4px #3b82f6;
}`,
        note: "A zero-blur shadow with spread draws a ring that follows `border-radius` and takes no layout space.",
      },
      {
        title: "Press a button in with an inset shadow",
        lang: "css",
        code: `.toggle {
  background: #27272a;
}
.toggle:active {
  box-shadow: inset 0 2px 4px rgb(0 0 0 / 0.5);
}`,
      },
    ],
    edgeCases: [
      "A shadow written without a color is a `currentColor` shadow, so it is dropped with an advisory; the other shadows in the list still draw.",
      "Inside a color function, `currentColor` resolves: `0 0 0 2px color-mix(in srgb, currentColor 40%, transparent)` draws.",
      "An outer shadow is cut out of the border box: under a transparent box, a spread shadow is a ring, not a filled rectangle.",
      "`box-shadow: none` in a later rule cancels every shadow from earlier rules.",
      "`box-shadow` does not transition; it snaps to the new value.",
    ],
  },
  opacity: {
    examples: [
      {
        title: "Fade a toast in and out",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const shown = ref(false);
function notify() {
  shown.value = true;
  setTimeout(() => (shown.value = false), 2500);
}
</script>

<template>
  <button @click="notify">Save</button>
  <div class="toast" :class="{ shown }">Draft saved</div>
</template>

<style scoped>
.toast {
  opacity: 0;
  transition: opacity 200ms ease;
}
.toast.shown {
  opacity: 1;
}
</style>`,
      },
      {
        title: "Dim a disabled control",
        lang: "css",
        code: `button:disabled {
  opacity: 0.45;
  cursor: default;
}`,
      },
    ],
    edgeCases: [
      "`opacity` below `1` makes a stacking context: a `z-index: 100` child stays inside it and paints under a later sibling with `z-index: 10`.",
    ],
  },
  filter: {
    examples: [
      {
        title: "Blur a solid placeholder while an image loads",
        lang: "css",
        code: `.thumb-placeholder {
  width: 120px;
  height: 80px;
  border-radius: 8px;
  background-color: #64748b;
  filter: blur(6px);
}`,
        note: "The box has no children and a solid background, which is what `blur()` needs.",
      },
      {
        title: "Gray out an offline contact's avatar",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ src: string; online: boolean }>();
</script>

<template>
  <svg style="display: none">
    <filter id="grayscale" color-interpolation-filters="sRGB">
      <feColorMatrix type="matrix" values="0.2126 0.7152 0.0722 0 0 0.2126 0.7152 0.0722 0 0 0.2126 0.7152 0.0722 0 0 0 0 0 1 0" />
    </filter>
  </svg>
  <img :src="src" alt="" class="avatar" :class="{ offline: !online }" />
</template>

<style scoped>
.offline {
  filter: url(#grayscale);
}
</style>`,
        note: "The app stylesheet can apply this matrix to the avatar. An SVG shape can also use `filter: grayscale(1)` in its own inline style.",
      },
      {
        title: "Drop a shadow under an opaque floating panel",
        lang: "css",
        code: `.popover {
  border-radius: 10px;
  background-color: #18181b;
  filter: drop-shadow(0 6px 12px rgb(0 0 0 / 0.4));
}`,
      },
    ],
    edgeCases: [
      "On a blurred box, the blur is drawn as the box's own shadow, so it replaces the element's `box-shadow`.",
      "`drop-shadow()` adds to `box-shadow`, so both draw. Without a color, it takes the element's `color`.",
      "Only the first `blur()` and the first `drop-shadow()` apply; a second one is dropped with an advisory.",
      '`url(#id)` applies only when it is the only function in the list. The `<filter>` must hold one `feColorMatrix` with `type="matrix"`, 20 numbers and the `SourceGraphic` input; without `color-interpolation-filters="sRGB"`, the matrix runs in linear RGB, as in browsers.',
      "`getComputedStyle(el).filter` reads `none` whatever you declare.",
    ],
  },
  "backdrop-filter": {
    examples: [
      {
        title: "Frost a header over scrolling content",
        lang: "css",
        code: `.app-header {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 48px;
  z-index: 10;
  background: rgb(24 24 27 / 0.6);
  backdrop-filter: blur(16px);
}`,
        note: "Use a translucent background, or the blur behind it does not show.",
      },
      {
        title: "Blur the app behind a modal",
        lang: "css",
        code: `.modal-overlay {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  background: rgb(0 0 0 / 0.3);
  backdrop-filter: blur(8px);
}`,
      },
    ],
    edgeCases: [
      "The blurred area follows the element's `border-radius`.",
      "Under a `transform`, the blur is drawn where the box is drawn.",
      "Only the first `blur()` applies; a second one is dropped with an advisory.",
      "`backdrop-filter` does not transition; it snaps to the new value.",
      '`getComputedStyle(el).getPropertyValue("backdrop-filter")` reads `blur(<n>px)` or `none`.',
    ],
  },
  isolation: {
    examples: [
      {
        title: "Keep a card's badge under an open menu",
        lang: "css",
        code: `.card {
  position: relative;
  isolation: isolate;
}
.card .badge {
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 10;
}
.menu {
  position: absolute;
  z-index: 5;
}`,
        note: "The badge's `z-index: 10` counts only inside the card, so the menu stays on top.",
      },
    ],
    edgeCases: [
      "A value other than `auto`, `isolate` or a CSS-wide keyword prints an advisory.",
      "`opacity` below `1`, a `transform`, `clip-path`, `filter`, `backdrop-filter` and `mask` make a stacking context the same way.",
    ],
  },
  "clip-path": {
    examples: [
      {
        title: "Cut a hexagonal avatar",
        lang: "css",
        code: `.hex-avatar {
  width: 64px;
  height: 64px;
  object-fit: cover;
  clip-path: polygon(25% 0, 75% 0, 100% 50%, 75% 100%, 25% 100%, 0 50%);
}`,
      },
      {
        title: "Reveal a panel by animating its inset",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const panel = ref<HTMLElement | null>(null);
function reveal() {
  const start = performance.now();
  const step = (now: number) => {
    const t = Math.min((now - start) / 300, 1);
    panel.value!.style.clipPath = "inset(0 " + (1 - t) * 100 + "% 0 0)";
    if (t < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}
</script>

<template>
  <button @click="reveal">Show details</button>
  <div ref="panel" class="details" style="clip-path: inset(0 100% 0 0)">Order #4821</div>
</template>`,
        note: "`clip-path` does not transition, so a frame loop rewrites it.",
      },
    ],
    edgeCases: [
      "The clip applies to hit testing too: a click outside the shape reaches what is behind.",
      "The border, `box-shadow` and positioned descendants outside the shape are cut.",
      "The shape moves with the element's `transform`.",
      "`inset(50%)` hides the element and it receives no clicks.",
      "`path()` takes `evenodd` (`path(evenodd, '…')`) to cut holes in overlapping subpaths.",
    ],
  },
  clip: {
    examples: [
      {
        title: "Hide a label visually",
        lang: "css",
        code: `.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
}`,
      },
    ],
    edgeCases: [
      "Any `rect()` whose top equals its bottom or whose left equals its right counts as empty, so `rect(1px, 1px, 1px, 1px)` works too.",
      "An empty rect hides the element as `visibility: hidden` does.",
    ],
  },
  mask: {
    examples: [
      {
        title: "Tint an SVG icon with the text color",
        lang: "css",
        code: `.icon-settings {
  width: 20px;
  height: 20px;
  background-color: currentColor;
  mask: url("icons/settings.svg") center / contain no-repeat;
}`,
        note: "The icon takes the element's `color`, so one SVG serves every color.",
      },
      {
        title: "Fade the edge of a scrolling list",
        lang: "css",
        code: `.chat-history {
  overflow-y: auto;
  mask: linear-gradient(to bottom, transparent, #000 24px);
}`,
      },
    ],
    edgeCases: [
      "The mask cuts the element's children too.",
      "A mask `url()` that fails to load counts as transparent, as in a browser, so the element is hidden there; an advisory names the URL.",
      "A masked element makes a stacking context.",
      "A layer that would need too many tiles to cover the box is left out with an advisory, which hides the element there.",
    ],
  },
  "mask-image": {
    examples: [
      {
        title: "Replace an icon font with masked SVGs",
        lang: "css",
        code: `.menu-item::before {
  content: "";
  display: block;
  width: 16px;
  height: 16px;
  background: currentColor;
  mask-image: url("icons/chevron.svg");
  mask-size: contain;
  mask-repeat: no-repeat;
}`,
      },
      {
        title: "Fade the bottom of a collapsed preview",
        lang: "css",
        code: `.note-preview {
  max-height: 120px;
  overflow: hidden;
  mask-image: linear-gradient(to bottom, #000 50%, transparent);
}`,
      },
    ],
    edgeCases: [
      "A masked SVG stays sharp under `transform: scale()`.",
      "A gradient mask tiles only when `mask-size` gives both axes; otherwise it covers the box once.",
    ],
  },
  "mask-position": {
    examples: [
      {
        title: "Center an icon in a larger button",
        lang: "css",
        code: `.icon-button {
  width: 32px;
  height: 32px;
  background-color: currentColor;
  mask-image: url("icons/close.svg");
  mask-size: 16px 16px;
  mask-repeat: no-repeat;
  mask-position: center;
}`,
      },
    ],
    edgeCases: [
      "An offset from the right or bottom edge with a length (`right 4px`) sits flush against that edge; use a percentage or a `left`/`top` offset.",
    ],
  },
  "mask-repeat": {
    examples: [
      {
        title: "Cut a row of stripes from a progress bar",
        lang: "css",
        code: `.segmented-bar {
  height: 8px;
  background: #22c55e;
  mask-image: url("patterns/segment.svg");
  mask-size: 20px 8px;
  mask-repeat: repeat-x;
  mask-position: 10px 0;
}`,
      },
    ],
  },
  "mask-size": {
    examples: [
      {
        title: "Fit a status glyph inside its badge",
        lang: "css",
        code: `.badge-check {
  width: 18px;
  height: 18px;
  background: #2563eb;
  mask: url("icons/check-circle.svg") center / 18px 18px no-repeat;
}`,
      },
    ],
  },
  "mask-mode": {
    examples: [
      {
        title: "Mask with a black-and-white image",
        lang: "css",
        code: `.split-preview {
  background: #ef4444;
  mask: linear-gradient(to right, #fff 50%, #000 50%) luminance;
}`,
        note: "With `luminance`, white keeps the element and black hides it.",
      },
    ],
    edgeCases: [
      "When layers declare different modes, the first layer's mode applies to all of them, with an advisory.",
    ],
  },
  "mask-clip": {
    examples: [
      {
        title: "Paint a background only in the border ring",
        lang: "css",
        code: `.gradient-border {
  position: absolute;
  inset: 0;
  padding: 3px;
  border-radius: inherit;
  background: linear-gradient(90deg, #f472b6, #818cf8);
  mask-image: linear-gradient(#fff 0 0), linear-gradient(#fff 0 0);
  mask-clip: content-box, border-box;
  mask-composite: exclude;
}`,
      },
    ],
    edgeCases: ["Outside the gradient-border ring, a value other than `border-box` prints an advisory."],
  },
  "mask-origin": {
    examples: [
      {
        title: "Cut a notch for a status dot out of a bordered avatar",
        lang: "css",
        code: `.avatar.with-status {
  border: 2px solid #18181b;
  border-radius: 50%;
  mask-image: radial-gradient(circle at 85% 85%, transparent 7px, #000 7.5px);
  mask-origin: border-box;
}`,
        note: "The notch's position counts from the outer edge of the border, which is the only box Skathi lays a mask out from.",
      },
    ],
    edgeCases: [
      "A value other than `border-box` prints an advisory and the mask is laid out at the border box, except in the gradient-border ring.",
    ],
  },
  "mask-composite": {
    examples: [
      {
        title: "Outline a selected tile with a gradient ring",
        lang: "css",
        code: `.tile.selected::after {
  content: "";
  position: absolute;
  inset: 0;
  padding: 2px;
  border-radius: inherit;
  background: conic-gradient(#22d3ee, #a78bfa, #22d3ee);
  mask:
    linear-gradient(#fff 0 0) content-box,
    linear-gradient(#fff 0 0);
  mask-composite: exclude;
}`,
        note: "The tile needs `position: relative`; the ring has no children, so nothing inside it is covered.",
      },
    ],
    edgeCases: [
      "`-webkit-mask-composite: xor` and `destination-out` read as `exclude`.",
      "In the ring, the element's own children paint whole, with an advisory; put the ring on an empty overlay.",
      "The ring is not applied over a `url()` background, with an advisory.",
    ],
  },
  "mask-type": {
    edgeCases: [
      'An app rule such as `mask { mask-type: alpha }` is dropped. Use `<mask style="mask-type: alpha">` inside the SVG.',
    ],
  },
  "mask-border": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
  "webkit-mask": {
    examples: [
      {
        title: "Instead: Write the unprefixed mask",
        lang: "css",
        code: `.icon-bell {
  width: 18px;
  height: 18px;
  background: currentColor;
  mask: url("icons/bell.svg") center / contain no-repeat;
}`,
      },
    ],
    edgeCases: [
      "In the gradient-border ring idiom (two opaque gradient layers, the top one at `content-box`), `-webkit-mask` is accepted without an advisory, so the usual prefixed and unprefixed pair stays quiet.",
    ],
  },
  "webkit-box-reflect": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
  "effect-quality": {
    examples: [
      {
        title: "Keep an animated texture inexpensive",
        lang: "css",
        code: `.grain {
  --shading: noise(0.25, 2px, 20);
  effect-quality: reduced;
}`,
        note: "The built-in noise effect uses one monochrome sample at reduced quality. Its animation clock advances up to 30 times per second.",
      },
    ],
    edgeCases: ["A cross-fade applies the element's quality to both shaders if they declare the slot."],
  },
  shading: {
    examples: [
      {
        title: "Frost a floating panel",
        lang: "css",
        code: `.command-palette {
  border-radius: 14px;
  background: #ffffff1a;
  border: 1px solid #ffffff55;
  --shading: frost(0.85, 14px, #cfe9ff1f);
}`,
      },
      {
        title: "Grow a glow when a card is picked",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const hot = ref(false);
</script>

<template>
  <div class="call-card" :class="{ hot }" @click="hot = !hot">Join call</div>
</template>

<style scoped>
.call-card {
  background: #1d2233;
  --shading: glow(#7fd4ff, 10px, 0.35);
  transition: --shading 420ms ease;
}
.call-card.hot {
  --shading: glow(#ff2d95, 34px, 1);
}
</style>`,
        note: "Keep the `transition` on the base rule: one declared on a state rule is ignored.",
      },
      {
        title: "Add film grain to a header",
        lang: "css",
        code: `.now-playing {
  background: #2b3350;
  --shading: noise(0.4, 2px, 18);
}`,
      },
      {
        title: "Declare how far a backdrop effect reads",
        lang: "slang",
        code: `import sk;

[SkEffect("ripple-glass")]
struct RippleGlass : IEffect
{
    // Every read below lands at most \`depth\` outside the element.
    [SkLength] [SkBackdropReach] [SkDefault("12")] [SkRange(0.0, 48.0)]
    float depth;

    float4 shade(EffectInput input)
    {
        float2 bend = float2(sin(input.px.y * 0.2), cos(input.px.x * 0.2));
        return input.backdropAtPx(input.px + bend * depth);
    }
};`,
        note: "A `[SkBackdrop]` effect may read the backdrop anywhere, so each frame it is drawn the renderer copies the whole window for it, and blurs the whole window when it has a `[SkBackdropRadius]`. With `[SkBackdropReach]` on a length parameter, only the drawn quad grown by that length is copied and blurred, so the cost follows the element instead of the window. A read beyond the reach returns the backdrop at the edge of that area. The attribute implies `[SkBackdrop]`, and an effect has at most one.",
      },
    ],
    edgeCases: [
      "Built in are `frost(<strength>, <radius>?, <tint>?)`, `noise(<amount>, <scale>?, <speed>?)` and `glow(<color>?, <radius>, <strength>)`; an app adds its own as `shaders/<name>.slang`, written against the [shader library](/shaders/) (`sk` and `sk.color`; see [Writing effects with the shader library](/guides/shader-library/)).",
      "A number outside the effect's range is clamped, with an advisory: `frost(1.4)` runs at strength `1`.",
      "Only one effect function applies; the others are dropped with an advisory.",
      "`var()` inside `--shading` is not resolved; the effect is skipped with an advisory.",
      "A transition between two values of the same effect interpolates its arguments; between different effects, or to and from `none`, it cross-fades.",
    ],
  },
};
