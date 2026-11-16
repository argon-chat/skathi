import type { Details } from "../../details";

export const details: Details = {
  "background-color": {
    examples: [
      {
        title: "Highlight the selected row in a list",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const selected = ref("inbox");
const folders = ["inbox", "drafts", "archive"];
</script>

<template>
  <div v-for="folder in folders" :key="folder" class="row" :class="{ active: folder === selected }" @click="selected = folder">
    {{ folder }}
  </div>
</template>

<style scoped>
.row {
  padding: 6px 12px;
  background-color: transparent;
  transition: background-color 150ms ease;
}
.row:hover {
  background-color: rgb(255 255 255 / 0.06);
}
.row.active {
  background-color: #2563eb;
}
</style>`,
      },
      {
        title: "Fill an icon box with the text color",
        lang: "css",
        code: `.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  color: #22c55e;
  background-color: currentColor;
}
.status-dot.away {
  color: #f59e0b;
}`,
        note: "`currentColor` resolves against the element's own `color`, so changing `color` recolors the fill.",
      },
    ],
    edgeCases: [
      "`background-color: currentColor` takes the element's computed `color`, including one it inherits. It reads the base `color`: a `:hover` rule that changes `color` does not recolor it.",
      "A color that does not resolve to a concrete value (`light-dark()`, a system color such as `Canvas`) is dropped with an advisory; switch colors with `@media (prefers-color-scheme: dark)` instead.",
    ],
  },
  background: {
    examples: [
      {
        title: "Cover a hero banner with an image",
        lang: "css",
        code: `.hero {
  height: 180px;
  border-radius: 12px;
  background: #1e293b url("images/hero.jpg") center / cover no-repeat;
}`,
      },
      {
        title: "Darken an image under a caption",
        lang: "css",
        code: `.album-cover {
  display: flex;
  align-items: flex-end;
  height: 220px;
  padding: 16px;
  color: white;
  background:
    linear-gradient(to top, rgb(0 0 0 / 0.7), transparent 60%),
    url("covers/album.jpg") center / cover no-repeat;
}`,
        note: "The first layer paints on top, so the gradient sits over the image.",
      },
    ],
    edgeCases: [
      "`transition: background` animates only the background color; image and gradient layers snap to the new value.",
      "Once any layer is a `url()`, the whole stack is painted by an extra child box under the element's content, so gradient layers keep their place in the stack.",
    ],
  },
  "background-image": {
    examples: [
      {
        title: "Show a placeholder pattern behind an empty canvas",
        lang: "css",
        code: `.board {
  height: 100%;
  background-color: #0f172a;
  background-image: radial-gradient(#334155 1px, transparent 1px);
  background-size: 16px 16px;
}`,
      },
      {
        title: "Use an SVG file as a decorative background",
        lang: "css",
        code: `.empty-state {
  height: 240px;
  background-image: url("illustrations/empty-inbox.svg");
  background-position: center;
  background-repeat: no-repeat;
  background-size: contain;
}`,
      },
    ],
    edgeCases: [
      "A relative `url()` resolves the way an `<img src>` does, against the app root, not against the stylesheet's location.",
      "An image that fails to load is not painted, and an advisory names the URL.",
      "The image paints in the padding box and scrolls with the content inside a scroll container.",
      "A layer that would need more than 4096 tiles to cover the box is not painted, with an advisory; give it a larger `background-size` or `no-repeat`.",
    ],
  },
  "function-linear-gradient": {
    examples: [
      {
        title: "Fade a header into the content below it",
        lang: "css",
        code: `.page-header {
  height: 96px;
  background: linear-gradient(to bottom, #111827, rgb(17 24 39 / 0));
}`,
      },
      {
        title: "Draw a two-tone progress track with hard stops",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ percent: number }>();
</script>

<template>
  <div class="track" :style="{ '--done': percent + '%' }" />
</template>

<style scoped>
.track {
  height: 6px;
  border-radius: 3px;
  background: linear-gradient(90deg, #3b82f6 var(--done), #e5e7eb var(--done));
}
</style>`,
        note: "Two stops at the same position make a sharp edge instead of a blend.",
      },
    ],
    edgeCases: [
      "A stop list longer than one GPU draw takes is split into bands, so there is no cap on the number of stops.",
      "A stop whose color does not resolve, such as `light-dark()` or a system color, leaves the whole gradient layer unpainted.",
    ],
  },
  "function-radial-gradient": {
    examples: [
      {
        title: "Add a soft spotlight behind a dialog",
        lang: "css",
        code: `.dialog-backdrop {
  position: fixed;
  inset: 0;
  background: radial-gradient(circle at 50% 40%, rgb(59 130 246 / 0.25), transparent 60%), #0b1120;
}`,
      },
      {
        title: "Draw a round avatar placeholder",
        lang: "css",
        code: `.avatar-placeholder {
  width: 40px;
  height: 40px;
  border-radius: 50%;
  background: radial-gradient(circle 14px at 50% 38%, #94a3b8 98%, transparent), #e2e8f0;
}`,
      },
    ],
    edgeCases: [
      "An ellipse size that mixes a percentage and a length (`radial-gradient(40% 60px, …)`) draws as `farthest-corner`.",
      "A center written as a far-edge offset with a length (`at right 10px center`) sits flush against that edge; a percentage offset is exact.",
    ],
  },
  "function-conic-gradient": {
    examples: [
      {
        title: "Draw a circular progress indicator",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ percent: number }>();
</script>

<template>
  <div class="ring" :style="{ '--done': percent + '%' }" />
</template>

<style scoped>
.ring {
  width: 48px;
  height: 48px;
  border-radius: 50%;
  background: conic-gradient(#22c55e var(--done), #27272a var(--done));
}
</style>`,
      },
      {
        title: "Paint a color wheel for a picker",
        lang: "css",
        code: `.hue-wheel {
  width: 160px;
  height: 160px;
  border-radius: 50%;
  background: conic-gradient(from 90deg, red, yellow, lime, cyan, blue, magenta, red);
}`,
      },
    ],
  },
  "function-repeating-linear-gradient": {
    examples: [
      {
        title: "Stripe a placeholder for a loading upload",
        lang: "css",
        code: `.upload-pending {
  height: 8px;
  border-radius: 4px;
  background: repeating-linear-gradient(45deg, #3b82f6 0 8px, #60a5fa 8px 16px);
}`,
      },
      {
        title: "Draw a ruler's tick marks",
        lang: "css",
        code: `.ruler {
  height: 12px;
  background: repeating-linear-gradient(90deg, #64748b 0 1px, transparent 1px 10px);
}`,
      },
    ],
  },
  "function-repeating-radial-gradient": {
    examples: [
      {
        title: "Draw concentric rings behind a call avatar",
        lang: "css",
        code: `.calling {
  width: 160px;
  height: 160px;
  border-radius: 50%;
  background: repeating-radial-gradient(circle, rgb(34 197 94 / 0.25) 0 6px, transparent 6px 18px);
}`,
      },
    ],
  },
  "function-repeating-conic-gradient": {
    examples: [
      {
        title: "Show a checkerboard behind a transparent image",
        lang: "css",
        code: `.image-preview {
  background: repeating-conic-gradient(#d4d4d8 0 25%, #ffffff 0 50%);
  background-size: 16px 16px;
}`,
        note: "The pattern tiles because `background-size` gives both axes.",
      },
    ],
  },
  "function-image-set": {
    examples: [
      {
        title: "Instead: Show a crisp logo at any display scale",
        lang: "css",
        code: `.logo {
  width: 120px;
  height: 32px;
  background: url("brand/logo.svg") center / contain no-repeat;
}`,
        note: "An SVG stays sharp at every scale factor, which is what `image-set()` picks resolutions for.",
      },
    ],
    edgeCases: [
      "`background-image: image-set(…)` is not painted, with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "function-cross-fade": {
    examples: [
      {
        title: "Instead: Tint a cover image",
        lang: "css",
        code: `.playlist-cover {
  height: 200px;
  background:
    linear-gradient(rgb(15 23 42 / 0.5), rgb(15 23 42 / 0.5)),
    url("covers/playlist.jpg") center / cover no-repeat;
}`,
      },
    ],
  },
  "function-element": {
    examples: [
      {
        title: "Instead: Paint a static placeholder",
        lang: "css",
        code: `.thumbnail-loading {
  aspect-ratio: 16 / 9;
  border-radius: 8px;
  background: linear-gradient(135deg, #27272a, #3f3f46);
}`,
      },
    ],
  },
  "function-webkit-gradient": {
    examples: [
      {
        title: "Instead: Rewrite a legacy gradient in standard syntax",
        lang: "css",
        code: `.toolbar {
  background-image: linear-gradient(to bottom, #ffffff, #e5e5e5);
}`,
        note: "This replaces `-webkit-gradient(linear, left top, left bottom, from(#fff), to(#e5e5e5))`.",
      },
    ],
    edgeCases: ["A `-webkit-gradient()` layer is not painted, with an advisory."],
  },
  "background-size": {
    examples: [
      {
        title: "Fit an album cover without cropping",
        lang: "css",
        code: `.cover {
  width: 200px;
  height: 200px;
  background: #111 url("covers/single.png") center / contain no-repeat;
}`,
      },
      {
        title: "Tile a dot grid behind a canvas",
        lang: "css",
        code: `.canvas {
  background-color: #fafafa;
  background-image: radial-gradient(#d4d4d8 1.5px, transparent 1.5px);
  background-size: 20px 20px;
}`,
      },
    ],
    edgeCases: [
      "A gradient tiles only when `background-size` gives both axes (`20px 20px`). With `auto`, `cover`, `contain` or a single value, the gradient fills the box.",
      "Percentages resolve against the padding box: `200% 100%` makes an image twice the box's width.",
    ],
  },
  "background-repeat": {
    examples: [
      {
        title: "Repeat a divider pattern along one axis",
        lang: "css",
        code: `.section-divider {
  height: 8px;
  background-image: url("patterns/wave.svg");
  background-size: 24px 8px;
  background-repeat: repeat-x;
}`,
      },
      {
        title: "Place one watermark in a corner",
        lang: "css",
        code: `.document {
  background-image: url("brand/watermark.svg");
  background-position: 100% 100%;
  background-size: 96px 96px;
  background-repeat: no-repeat;
}`,
      },
    ],
    edgeCases: [
      "On a gradient layer, `background-repeat` takes effect only when `background-size` gives both axes; otherwise the gradient fills the box once.",
    ],
  },
  "background-position": {
    examples: [
      {
        title: "Show one cell of a sprite sheet",
        lang: "css",
        code: `.flag {
  width: 24px;
  height: 16px;
  background-image: url("sprites/flags.png");
  background-size: 400% 100%;
  background-repeat: no-repeat;
}
.flag.de {
  background-position: 33.333% 0;
}`,
        note: "A percentage aligns the same point of the image and the box, so `33.333%` picks the second of four cells.",
      },
      {
        title: "Offset a pattern from the top-left corner",
        lang: "css",
        code: `.sprite-icon {
  width: 20px;
  height: 20px;
  background-image: url("sprites/toolbar.svg");
  background-size: 40px 20px;
  background-position: -20px 0;
}`,
      },
    ],
    edgeCases: [
      "`right 10%` and `bottom 10%` are exact: a percentage from the far edge is read as `90%`.",
      "`center` is `50%` on that axis.",
    ],
  },
  "background-position-x": {
    examples: [
      {
        title: "Step through sprite frames horizontally",
        lang: "css",
        code: `.loader-frame {
  width: 32px;
  height: 32px;
  background-image: url("sprites/loader.png");
  background-size: 256px 32px;
  background-position-x: 0;
}
.loader-frame.step-2 {
  background-position-x: -32px;
}`,
      },
    ],
  },
  "background-position-y": {
    examples: [
      {
        title: "Swap a toggle's icon from a vertical sprite",
        lang: "css",
        code: `.mute-toggle {
  width: 20px;
  height: 20px;
  background-image: url("sprites/mute.svg");
  background-size: 20px 40px;
  background-position-y: 0;
}
.mute-toggle.muted {
  background-position-y: -20px;
}`,
      },
    ],
  },
  "background-clip": {
    examples: [
      {
        title: "Fill a heading with a gradient",
        lang: "css",
        code: `.brand-title {
  font-size: 32px;
  font-weight: 700;
  background: linear-gradient(90deg, #7c3aed, #db2777);
  background-clip: text;
  color: transparent;
}`,
      },
    ],
    edgeCases: [
      "With `text`, the element's own box paints no background; the color and every layer move into the glyphs.",
      "A `text-shadow` still draws behind the gradient-filled glyphs.",
      "Any value other than `text` prints an advisory.",
    ],
  },
  "background-origin": {
    edgeCases: [
      'Accepted without an advisory and without effect, so `CSS.supports("background-origin", "content-box")` returns `true`.',
    ],
  },
  "background-attachment": {
    edgeCases: [
      '`CSS.supports("background-attachment", "fixed")` returns `true`, so feature detection does not catch it.',
    ],
  },
  "background-blend-mode": {
    edgeCases: [
      'A declaration prints an advisory and changes nothing; `CSS.supports("background-blend-mode", "multiply")` returns `false`.',
    ],
  },
  border: {
    examples: [
      {
        title: "Separate panes with a hairline",
        lang: "css",
        code: `.sidebar {
  width: 240px;
  border-right: 1px solid #27272a;
}
.sidebar .section + .section {
  border-top: 1px solid #27272a;
}`,
      },
      {
        title: "Ring the plan the user picks",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const plans = ["Free", "Pro", "Team"];
const picked = ref("Pro");
</script>

<template>
  <div v-for="plan in plans" :key="plan" class="plan" :class="{ picked: plan === picked }" @click="picked = plan">
    {{ plan }}
  </div>
</template>

<style scoped>
.plan {
  padding: 16px;
  border: 1px solid #3f3f46;
  border-radius: 12px;
}
.plan.picked {
  border: 2px solid #3b82f6;
}
</style>`,
      },
    ],
    edgeCases: [
      "Without a color, the border takes the element's `color`.",
      "`border-bottom: none` or `border-bottom: 0` removes only that side; the other sides keep their width.",
      "`transition: border` animates the color and the four widths.",
    ],
  },
  "border-width": {
    examples: [
      {
        title: "Thicken an input's bottom edge on focus",
        lang: "css",
        code: `.field {
  border: 0 solid #3f3f46;
  border-bottom-width: 1px;
  transition: border-bottom-width 120ms ease;
}
.field:focus-visible {
  border-bottom-width: 2px;
  border-color: #3b82f6;
}`,
      },
    ],
    edgeCases: ["`thin`, `medium` and `thick` are `1px`, `3px` and `5px`."],
  },
  "border-color": {
    examples: [
      {
        title: "Mark an invalid field",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";

const email = ref("");
const invalid = computed(() => email.value !== "" && !email.value.includes("@"));
</script>

<template>
  <input v-model="email" class="field" :class="{ invalid }" placeholder="you@example.com" />
</template>

<style scoped>
.field {
  border: 1px solid #3f3f46;
  transition: border-color 150ms ease;
}
.field.invalid {
  border-color: #ef4444;
}
</style>`,
      },
      {
        title: "Color one side as an active-tab marker",
        lang: "css",
        code: `.tab {
  border-bottom: 2px solid transparent;
}
.tab[aria-selected="true"] {
  border-bottom-color: #3b82f6;
}`,
      },
    ],
    edgeCases: [
      "`currentColor` resolves against the element's own `color`, so a child with its own `color` gets its own border color.",
    ],
  },
  "border-style": {
    examples: [
      {
        title: "Outline a placeholder slot with a dashed border",
        lang: "css",
        code: `.widget-slot {
  min-height: 80px;
  border-width: 1px;
  border-style: dashed;
  border-color: #52525b;
  border-radius: 8px;
}`,
      },
    ],
    edgeCases: [
      "`none` and `hidden` set that side's width to `0` and leave the other sides as they are.",
      "A drawn-solid value (`dotted`, `double`, `groove` and the rest) prints an advisory.",
    ],
  },
  "border-radius": {
    examples: [
      {
        title: "Round a card and clip its scrolling list",
        lang: "css",
        code: `.card {
  height: 320px;
  border-radius: 12px;
  overflow: auto;
  background: #18181b;
}`,
        note: "The rows clip at the card's curve before and after scrolling.",
      },
      {
        title: "Crop a profile picture to a circle",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ src: string; name: string }>();
</script>

<template>
  <div class="avatar">
    <img :src="src" :alt="name" />
  </div>
</template>

<style scoped>
.avatar {
  width: 40px;
  height: 40px;
  border-radius: 50%;
  overflow: hidden;
}
.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
}
</style>`,
      },
    ],
    edgeCases: [
      "`9999px` makes a pill: each corner is capped at half the box's shorter side.",
      "`border-radius: inherit` takes the parent's radii, including from an inline `style`.",
      "A bordered scroller clips its content to the inner radius, inside the border.",
    ],
  },
  "corner-shape": {
    edgeCases: ["A declaration prints an advisory and changes nothing; the corners stay round."],
  },
  "border-image": {
    examples: [
      {
        title: "Instead: Draw a gradient border",
        lang: "css",
        code: `.price-card {
  position: relative;
  border-radius: 12px;
}
.price-card .ring {
  position: absolute;
  inset: 0;
  padding: 2px;
  border-radius: inherit;
  background: linear-gradient(135deg, #8b5cf6, #06b6d4);
  mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
  mask-composite: exclude;
}`,
        note: "The ring is an empty overlay, so the card's text stays uncovered.",
      },
    ],
    edgeCases: ['A declaration prints an advisory, and `CSS.supports("border-image", …)` returns `false`.'],
  },
  outline: {
    examples: [
      {
        title: "Show a focus ring for keyboard users",
        lang: "css",
        code: `button:focus-visible {
  outline: 2px solid #3b82f6;
  outline-offset: 2px;
}`,
      },
      {
        title: "Keep a list item's focus ring inside its scroller",
        lang: "css",
        code: `.result-row:focus-visible {
  outline: 2px solid #3b82f6;
  outline-offset: -2px;
}`,
        note: "A negative offset pulls the ring inside the border box, where the scroller's clip does not cut it.",
      },
    ],
    edgeCases: [
      "A `:focus` rule with `outline: none` removes a ring declared on the base rule.",
      "`outline: auto` draws a solid ring.",
      "Without a color, the ring takes the element's `color`.",
      "An outline does not transition; it snaps to the new value.",
    ],
  },
  "outline-width": {
    examples: [
      {
        title: "Thicken the ring of a selected tile",
        lang: "css",
        code: `.tile {
  outline: 1px solid #3f3f46;
}
.tile.selected {
  outline-width: 3px;
  outline-color: #3b82f6;
}`,
      },
    ],
    edgeCases: ["`thin`, `medium` and `thick` are `1px`, `3px` and `5px`."],
  },
  "outline-color": {
    examples: [
      {
        title: "Recolor a focus ring for a destructive button",
        lang: "css",
        code: `button:focus-visible {
  outline: 2px solid #3b82f6;
}
button.danger:focus-visible {
  outline-color: #ef4444;
}`,
      },
    ],
    edgeCases: ["A state rule that sets only `outline-color` keeps the width and style of the ring on the base rule."],
  },
  "outline-style": {
    examples: [
      {
        title: "Mark a drop target with a dashed ring",
        lang: "css",
        code: `.folder.drop-target {
  outline-style: dashed;
  outline-color: #3b82f6;
  outline-offset: 2px;
}`,
      },
    ],
    edgeCases: [
      "`outline-style` without a width draws at `medium`, `3px`.",
      "`none` and `hidden` set the width to `0`.",
    ],
  },
  "outline-offset": {
    examples: [
      {
        title: "Push a focus ring off a rounded button",
        lang: "css",
        code: `.icon-button {
  border-radius: 8px;
}
.icon-button:focus-visible {
  outline: 2px solid #3b82f6;
  outline-offset: 3px;
}`,
      },
    ],
    edgeCases: [
      "Negative lengths are accepted and draw the ring inside the border box.",
      "A value that is not a single length, such as a percentage, prints an advisory and is ignored.",
      "A `:focus` rule that sets only `outline-offset` keeps the base ring's width, style and color.",
    ],
  },
  "box-decoration-break": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
};
