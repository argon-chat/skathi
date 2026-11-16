import type { Details } from "../../details";

const VIEWPORT_POPOUT =
  "Viewport lengths resolve against the main window in every window, popouts included, and `sk dev` warns when a popout is open. `@media` and `env()` answer per window.";
const VIEWPORT_RESIZE = "A resize re-resolves viewport lengths in rules, inline styles and `@keyframes` alike.";
const VAR_CONSTANT =
  "`var()` arguments count as constant: they are substituted before the function is evaluated, so the function can take a value set from script.";
const MIXED_UNITS =
  "Arguments in two units that only resolve per element, such as `1em` and `4px`, cannot be evaluated; in a size property the declaration is dropped with an advisory.";

export const details: Details = {
  "unit-px": {
    examples: [
      {
        title: "Draw a hairline divider",
        lang: "css",
        code: `.divider {
  height: 1px;
  background-color: #ffffff1f;
}`,
      },
    ],
    edgeCases: [
      "A CSS pixel is not a device pixel: on a display scaled to 150%, `1px` covers 1.5 device pixels and `(resolution: 1.5dppx)` matches.",
    ],
  },

  "unit-percent": {
    examples: [
      {
        title: "Split two panes by percentage",
        lang: "css",
        code: `.workspace {
  display: flex;
  height: 100%;
}
.file-tree {
  width: 28%;
}
.editor {
  width: 72%;
}`,
      },
      {
        title: "Center a popover under its anchor",
        lang: "css",
        code: `.popover {
  position: absolute;
  top: 100%;
  left: 50%;
  transform: translateX(-50%);
}`,
        note: "`left` takes a percentage of the containing block; `translateX()` takes one of the popover's own width.",
      },
    ],
    edgeCases: [
      "A `font-size` percentage is of the parent's font size, and so is a percentage inside a `font-size` `calc()`, such as `calc(50% + 2px)`.",
      "A `line-height` percentage resolves against the element's own font size, and children inherit the resulting px, not the percentage.",
      "In an `@media` width or height test, a percentage drops the whole block with an advisory.",
    ],
  },

  "unit-em": {
    examples: [
      {
        title: "Pad a badge in proportion to its text",
        lang: "css",
        code: `.badge {
  font-size: 11px;
  padding: 0.25em 0.75em;
  border-radius: 1em;
  gap: 0.5em;
}`,
        note: "Change the badge's `font-size` and the padding, radius and gap scale with it.",
      },
    ],
    edgeCases: [
      "In `font-size`, `em` is the parent's font size; in every other property it is the element's own, after its `font-size` applies.",
      "A `:hover` rule that changes `font-size` also moves the hovered element's `em`-based padding and margins.",
      "A custom property holding `2em` resolves on the element that reads it with `var()`, not on the one that declares it.",
      "In an `@media` width or height test, `em` drops the whole block with an advisory.",
    ],
  },

  "unit-rem": {
    examples: [
      {
        title: "Scale the whole interface from one root size",
        lang: "css",
        code: `:root {
  font-size: 15px;
}
.card {
  padding: 1rem;
  gap: 0.5rem;
  border-radius: 0.75rem;
}`,
      },
      {
        title: "Apply a text size preference",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watch } from "vue";

const size = ref(16);
watch(size, (px) => (document.documentElement.style.fontSize = px + "px"));
</script>

<template>
  <input v-model.number="size" type="range" min="12" max="22" class="text-size" />
</template>`,
        note: "Every `rem` length in your stylesheets follows the root element's font size.",
      },
    ],
    edgeCases: [
      "The root element's own `rem` lengths use 16px, since `1rem` is defined by its font size.",
      "Changing the root font size at runtime recompiles the stylesheet so every `rem` moves.",
      "A root font size outside 1px to 200px is refused, and `rem` keeps its previous size.",
    ],
  },

  "unit-ch": {
    examples: [
      {
        title: "Size a field to the value it holds",
        lang: "css",
        code: `.zip-code {
  width: calc(6ch + 16px);
  padding: 0 8px;
  font-variant-numeric: tabular-nums;
}`,
        note: "Padding sits inside the width, so the `16px` makes room for it.",
      },
    ],
    edgeCases: [
      "`ch` is measured from the `0` glyph of the element's own font at its own size, so it changes with `font-family` as well as `font-size`.",
      "When the font cannot be measured, `1ch` falls back to `0.5em`.",
    ],
  },

  "unit-ex": {
    examples: [
      {
        title: "Size an unread dot to the x-height",
        lang: "css",
        code: `.unread-dot {
  width: 1ex;
  height: 1ex;
  border-radius: 50%;
  background-color: #7aa2ff;
}`,
      },
    ],
    edgeCases: [
      "`ex` is measured from the x-height of the element's own font at its own size.",
      "When the font cannot be measured, `1ex` falls back to `0.5em`.",
    ],
  },

  "unit-rch": {
    examples: [
      {
        title: "Give code columns a root-relative width",
        lang: "css",
        code: `.line-number {
  width: 5rch;
  text-align: right;
}`,
      },
    ],
  },

  "unit-rex": {
    examples: [
      {
        title: "Space inline icons by a root-relative amount",
        lang: "css",
        code: `.inline-icon {
  width: 2rex;
  height: 2rex;
  margin-right: 1rex;
}`,
      },
    ],
  },

  "unit-vw": {
    examples: [
      {
        title: "Keep a dialog inside the window",
        lang: "css",
        code: `.dialog {
  width: 90vw;
  max-width: 560px;
}`,
      },
    ],
    edgeCases: [VIEWPORT_POPOUT, VIEWPORT_RESIZE],
  },

  "unit-vh": {
    examples: [
      {
        title: "Let a dialog scroll instead of overflowing the window",
        lang: "css",
        code: `.dialog-body {
  max-height: calc(85vh - 80px);
  overflow-y: auto;
}`,
      },
    ],
    edgeCases: [VIEWPORT_POPOUT, VIEWPORT_RESIZE],
  },

  "unit-vmin": {
    examples: [
      {
        title: "Size an empty-state illustration to fit any window",
        lang: "css",
        code: `.empty-state img {
  width: 40vmin;
  height: 40vmin;
}`,
      },
    ],
    edgeCases: [VIEWPORT_POPOUT],
  },

  "unit-vmax": {
    examples: [
      {
        title: "Cover the window with a background glow",
        lang: "css",
        code: `.backdrop-glow {
  position: absolute;
  width: 120vmax;
  height: 120vmax;
  border-radius: 50%;
  background: radial-gradient(#7aa2ff33, transparent 70%);
}`,
      },
    ],
    edgeCases: [VIEWPORT_POPOUT],
  },

  "unit-vi": {
    examples: [
      {
        title: "Size a sidebar to a share of the window",
        lang: "css",
        code: `.sidebar {
  width: 24vi;
  min-width: 200px;
}`,
      },
    ],
    edgeCases: [
      "`vi` always resolves against the window width, like `vw`, whatever the `writing-mode`.",
      VIEWPORT_POPOUT,
    ],
  },

  "unit-vb": {
    examples: [
      {
        title: "Cap a preview to a share of the window height",
        lang: "css",
        code: `.preview {
  max-height: 60vb;
  overflow-y: auto;
}`,
      },
    ],
    edgeCases: [
      "`vb` always resolves against the window height, like `vh`, whatever the `writing-mode`.",
      VIEWPORT_POPOUT,
    ],
  },

  "unit-in": {
    examples: [
      {
        title: "Instead: size a ruler tick in px",
        lang: "css",
        code: `.ruler-inch {
  width: 96px;
  border-right: 1px solid #ffffff33;
}`,
        note: "`1in` is `96px`.",
      },
    ],
    edgeCases: [
      "In an `@media` width or height test, `in` converts to px and works: `(min-width: 10in)` tests for 960px.",
    ],
  },

  "unit-cm": {
    examples: [
      {
        title: "Instead: size a print preview margin in px",
        lang: "css",
        code: `.page-preview {
  padding: 75.6px;
  background-color: #ffffff;
}`,
        note: "`2cm` is `75.6px`.",
      },
    ],
    edgeCases: ["In an `@media` width or height test, `cm` converts to px and works."],
  },

  "unit-mm": {
    examples: [
      {
        title: "Instead: space the ticks of a millimeter scale in px",
        lang: "css",
        code: `.scale-tick {
  width: 1px;
  margin-right: 2.78px;
}`,
        note: "`1mm` is about `3.78px`.",
      },
    ],
    edgeCases: ["In an `@media` width or height test, `mm` converts to px and works."],
  },

  "unit-pt": {
    examples: [
      {
        title: "Instead: set a 12pt body size in px",
        lang: "css",
        code: `.document {
  font-size: 16px;
  line-height: 1.5;
}`,
        note: "`12pt` is `16px`.",
      },
    ],
    edgeCases: ["In an `@media` width or height test, `pt` converts to px and works."],
  },

  "unit-pc": {
    examples: [
      {
        title: "Instead: indent a quote by one pica in px",
        lang: "css",
        code: `.quote {
  padding-left: 16px;
  border-left: 2px solid #ffffff33;
}`,
        note: "`1pc` is `16px`.",
      },
    ],
    edgeCases: ["In an `@media` width or height test, `pc` converts to px and works."],
  },

  "unit-q": {
    examples: [
      {
        title: "Instead: set a 20Q label size in px",
        lang: "css",
        code: `.label {
  font-size: 18.9px;
  font-weight: 600;
}`,
        note: "`1Q` is about `0.945px`.",
      },
    ],
    edgeCases: ["In an `@media` width or height test, `Q` converts to px and works."],
  },

  "function-calc": {
    examples: [
      {
        title: "Fill the space under a fixed header",
        lang: "css",
        code: `.content {
  height: calc(100% - 48px);
  overflow-y: auto;
}`,
      },
      {
        title: "Derive a radius scale from one token",
        lang: "css",
        code: `:root {
  --radius: 10px;
}
.card {
  border-radius: var(--radius);
}
.card .button {
  border-radius: calc(var(--radius) - 4px);
}`,
      },
    ],
    edgeCases: [
      "A `font-size` that computes below zero is clamped to `0`.",
      "Elsewhere, a `calc()` that cannot resolve drops its declaration as if it were invalid, with an advisory.",
    ],
  },

  "function-min": {
    examples: [
      {
        title: "Cap a dialog's size and keep it inside the window",
        lang: "css",
        code: `.dialog {
  width: min(560px, 90vw);
  max-height: min(640px, 85vh);
}`,
      },
    ],
    edgeCases: [
      "`em`, `rem`, `ch` and viewport units mix freely, because each resolves to px before the comparison: `min(2em, 24px)` works.",
    ],
  },

  "function-max": {
    examples: [
      {
        title: "Keep a target clickable at small text sizes",
        lang: "css",
        code: `.icon-button {
  min-height: max(28px, 2.25em);
  min-width: max(28px, 2.25em);
}`,
      },
    ],
    edgeCases: ["`em`, `rem`, `ch` and viewport units mix freely, because each resolves to px before the comparison."],
  },

  "function-clamp": {
    examples: [
      {
        title: "Scale a title with the window, within bounds",
        lang: "css",
        code: `.page-title {
  font-size: clamp(18px, 1.5vw + 10px, 30px);
  line-height: 1.2;
}`,
      },
    ],
    edgeCases: ["If the minimum is larger than the maximum, the minimum wins."],
  },

  "function-round": {
    examples: [
      {
        title: "Snap a panel width to an 8px grid",
        lang: "css",
        code: `.panel {
  --panel-width: 251px;
  width: round(var(--panel-width), 8px);
}
.panel.expanded {
  width: round(up, var(--panel-width), 8px);
}`,
      },
    ],
    edgeCases: [
      "The strategies are `nearest` (the default), `up`, `down` and `to-zero`.",
      "`nearest` rounds a half away from zero: `round(-10px, 4px)` gives `-12px`, where CSS specifies `-8px`.",
      VAR_CONSTANT,
      MIXED_UNITS,
    ],
  },

  "function-mod": {
    examples: [
      {
        title: "Loop a playhead within one beat",
        lang: "css",
        code: `.playhead {
  position: absolute;
  left: mod(var(--position), 48px);
}`,
        note: "Set `--position` from script as the elapsed px; the playhead wraps every 48px.",
      },
    ],
    edgeCases: [VAR_CONSTANT, MIXED_UNITS],
  },

  "function-abs": {
    examples: [
      {
        title: "Size a selection from a drag in either direction",
        lang: "css",
        code: `.selection {
  width: abs(var(--drag-x));
  height: abs(var(--drag-y));
}`,
        note: "Set `--drag-x` and `--drag-y` from pointer events; dragging left or up gives a negative delta.",
      },
    ],
    edgeCases: [VAR_CONSTANT],
  },

  "function-sign": {
    examples: [
      {
        title: "Point a trend arrow up or down",
        lang: "css",
        code: `.trend-arrow {
  transform: rotate(calc(sign(var(--change)) * -90deg));
  transition: transform 200ms ease;
}`,
        note: "For an arrow drawn pointing right and `--change` set to the day's change, a gain turns it up and a loss turns it down.",
      },
    ],
    edgeCases: [VAR_CONSTANT],
  },

  "function-pow": {
    examples: [
      {
        title: "Build a modular type scale",
        lang: "css",
        code: `.heading {
  font-size: calc(1rem * pow(1.25, var(--step)));
}
.heading.h1 {
  --step: 4;
}
.heading.h2 {
  --step: 3;
}`,
      },
    ],
    edgeCases: ["`pow()` takes plain numbers only; multiply the result by a unit, as above.", VAR_CONSTANT],
  },

  "function-sqrt": {
    examples: [
      {
        title: "Size a bubble so its area tracks a count",
        lang: "css",
        code: `.bubble {
  width: calc(sqrt(var(--count)) * 6px);
  height: calc(sqrt(var(--count)) * 6px);
  border-radius: 50%;
}`,
      },
    ],
    edgeCases: ["`sqrt()` takes a plain number only; multiply the result by a unit, as above.", VAR_CONSTANT],
  },

  "function-hypot": {
    examples: [
      {
        title: "Size a ripple to reach the farthest corner",
        lang: "css",
        code: `.card {
  --card-w: 320px;
  --card-h: 180px;
}
.card .ripple {
  width: calc(2 * hypot(var(--card-w), var(--card-h)));
  height: calc(2 * hypot(var(--card-w), var(--card-h)));
  border-radius: 50%;
}`,
      },
    ],
    edgeCases: [VAR_CONSTANT, MIXED_UNITS],
  },

  "function-exp": {
    examples: [
      {
        title: "Turn a zoom slider into a scale factor",
        lang: "css",
        code: `.canvas-content {
  transform-origin: top left;
  transform: scale(exp(var(--zoom)));
}`,
        note: "With `--zoom` from -1 to 1, each step of the slider multiplies the size by the same factor.",
      },
    ],
    edgeCases: ["`exp()` takes a plain number only.", VAR_CONSTANT],
  },

  "function-log": {
    examples: [
      {
        title: "Draw file sizes on a logarithmic scale",
        lang: "css",
        code: `.size-bar {
  width: calc(log(var(--bytes), 10) * 24px);
  height: 6px;
}`,
        note: "A 1 KB file draws 72px wide and a 1 MB file 144px.",
      },
    ],
    edgeCases: [
      "`log()` takes plain numbers only; the second argument is the base, and without it the base is e.",
      VAR_CONSTANT,
    ],
  },

  "function-sin": {
    examples: [
      {
        title: "Place hour marks around a clock face",
        lang: "vue",
        code: `<template>
  <div class="clock-face">
    <span v-for="hour in 12" :key="hour" class="hour-mark" :style="{ '--angle': hour * 30 + 'deg' }" />
  </div>
</template>

<style>
.clock-face {
  position: relative;
  width: 160px;
  height: 160px;
}
.hour-mark {
  position: absolute;
  left: calc(50% + sin(var(--angle)) * 70px);
  top: calc(50% - cos(var(--angle)) * 70px);
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background-color: currentColor;
}
</style>`,
      },
    ],
    edgeCases: ["A plain number argument is taken as radians: `sin(1.5708)` is `1`.", VAR_CONSTANT],
  },

  "function-cos": {
    examples: [
      {
        title: "Orbit a dot around a center",
        lang: "css",
        code: `.orbit-dot {
  position: absolute;
  left: calc(50% + cos(var(--angle)) * 60px);
  top: calc(50% + sin(var(--angle)) * 60px);
}`,
      },
    ],
    edgeCases: ["A plain number argument is taken as radians.", VAR_CONSTANT],
  },

  "function-tan": {
    examples: [
      {
        title: "Size the drop of a slanted ribbon",
        lang: "css",
        code: `.ribbon-fold {
  width: 48px;
  height: calc(tan(15deg) * 48px);
}`,
      },
    ],
    edgeCases: ["A plain number argument is taken as radians.", VAR_CONSTANT],
  },

  "function-asin": {
    examples: [
      {
        title: "Turn a gauge needle from a level",
        lang: "css",
        code: `.gauge-needle {
  transform-origin: bottom;
  transform: rotate(asin(var(--level)));
}`,
        note: "`--level` runs from -1 to 1 and the needle sweeps from -90deg to 90deg.",
      },
    ],
    edgeCases: [
      "An argument outside -1 to 1, such as `asin(2)`, has no angle and makes the declaration invalid.",
      VAR_CONSTANT,
    ],
  },

  "function-acos": {
    examples: [
      {
        title: "Open a hinge from a ratio",
        lang: "css",
        code: `.lid {
  transform-origin: left;
  transform: rotate(acos(var(--closed)));
}`,
        note: "`--closed: 1` gives `0deg`, a shut lid; `0` gives `90deg`.",
      },
    ],
    edgeCases: [
      "An argument outside -1 to 1, such as `acos(2)`, has no angle and makes the declaration invalid.",
      VAR_CONSTANT,
    ],
  },

  "function-atan": {
    examples: [
      {
        title: "Tilt a trend line to its slope",
        lang: "css",
        code: `.trend-line {
  transform-origin: left;
  transform: rotate(atan(var(--slope)));
}`,
      },
    ],
    edgeCases: [VAR_CONSTANT],
  },

  "function-atan2": {
    examples: [
      {
        title: "Aim an arrow at the pointer",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const delta = ref({ x: 1, y: 0 });

function track(event: PointerEvent) {
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
  delta.value = { x: event.clientX - box.left - box.width / 2, y: event.clientY - box.top - box.height / 2 };
}
</script>

<template>
  <div class="compass" @pointermove="track" :style="{ '--dx': delta.x + 'px', '--dy': delta.y + 'px' }">
    <div class="compass-arrow" />
  </div>
</template>

<style>
.compass-arrow {
  transform: rotate(atan2(var(--dy), var(--dx)));
}
</style>`,
      },
    ],
    edgeCases: [
      "Both arguments must have the same type and resolve when parsed, such as two px lengths or two numbers; `atan2(1em, 4px)` is invalid.",
      VAR_CONSTANT,
    ],
  },

  "function-var": {
    examples: [
      {
        title: "Theme a component with tokens",
        lang: "css",
        code: `:root {
  --accent: #7aa2ff;
}
.button {
  background-color: var(--accent);
}
.button.danger {
  --accent: #ff5c5c;
}`,
      },
      {
        title: "Fall back through a chain of tokens",
        lang: "css",
        code: `.toolbar {
  color: var(--toolbar-fg, var(--text, #e3e5e8));
  background-color: var(--toolbar-bg, var(--surface, #202225));
}`,
      },
      {
        title: "Drive a style from component state",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ percent: number }>();
</script>

<template>
  <div class="progress" :style="{ '--progress': percent + '%' }">
    <div class="progress-fill" />
  </div>
</template>

<style>
.progress-fill {
  width: var(--progress);
  height: 4px;
  background-color: #7aa2ff;
}
</style>`,
      },
    ],
    edgeCases: [
      "A `var()` with no fallback whose property is not set makes the declaration invalid without an advisory. `var(--x,)` is an empty fallback, which is valid.",
      "`transition` and its longhands written with `var()` are not applied, with an advisory: write the transition literally.",
      "Custom properties that refer to each other in a cycle are invalid on that element and its descendants, with an advisory.",
      "A substitution that grows past 64K characters stops with an advisory.",
      "`CSS.supports()` answers `true` for any declaration whose value contains `var()`, without checking what it will become.",
    ],
  },

  "custom-properties": {
    examples: [
      {
        title: "Override tokens for one region",
        lang: "css",
        code: `:root {
  --surface: #202225;
  --text: #e3e5e8;
}
.sidebar {
  --surface: #1b1d21;
}
.panel {
  background-color: var(--surface);
  color: var(--text);
}`,
        note: "A `.panel` inside `.sidebar` takes the darker surface; one elsewhere keeps the root's.",
      },
      {
        title: "Hand a measured size from script to CSS",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const header = ref<HTMLElement>();
const page = ref<HTMLElement>();

onMounted(() => {
  const height = header.value?.getBoundingClientRect().height ?? 48;
  page.value?.style.setProperty("--header-h", height + "px");
});
</script>

<template>
  <div ref="page" class="page">
    <header ref="header" class="page-header">Inbox</header>
    <main class="page-body" />
  </div>
</template>

<style>
.page-body {
  height: calc(100% - var(--header-h, 48px));
}
</style>`,
      },
    ],
    edgeCases: [
      "`--name: initial` leaves the property unset, so `var(--name, 1)` takes its fallback.",
      "`--name: inherit`, `unset` and `revert` all take the parent's value.",
      "`CSS.supports()` answers `true` for any custom property whose value has no `;`.",
    ],
  },

  "function-env": {
    examples: [
      {
        title: "Fit a custom titlebar to the window controls",
        lang: "css",
        code: `.titlebar {
  display: flex;
  align-items: center;
  height: env(titlebar-area-height, 36px);
  width: env(titlebar-area-width, 100%);
  padding-left: env(titlebar-area-x, 0px);
  app-region: drag;
}`,
        note: "The fallbacks apply when the window has no overlay, as in fullscreen.",
      },
    ],
    edgeCases: [
      'The `titlebar-area-*` variables exist only while the content owns the titlebar strip: not with `titleBarStyle: "default"`, in fullscreen or while minimized. Then the fallback applies, or the declaration is silently invalid without one.',
      "`titlebar-area-x` and `titlebar-area-y` are `0px`. In a popout without OS chrome, the area spans the popout's full width.",
      "`safe-area-inset-*` are always `0px`.",
      "Names are case-sensitive. An unknown name without a fallback makes the declaration invalid, with an advisory.",
    ],
  },

  "keyword-inherit": {
    examples: [
      {
        title: "Make links take the color of their text",
        lang: "css",
        code: `.notice {
  color: #ffcc00;
}
.notice a {
  color: inherit;
}`,
      },
      {
        title: "Round a header to its card's corners",
        lang: "css",
        code: `.card {
  border-radius: 12px;
  overflow: hidden;
}
.card-header {
  border-top-left-radius: inherit;
  border-top-right-radius: inherit;
}`,
      },
    ],
    edgeCases: [
      "`border-radius: inherit` copies the parent's corners, and an `::after` box takes its host's. A percentage stays a percentage of the child's own box.",
      "`border-radius: inherit` in a `:hover`, `:focus` or `:active` rule is not applied, with an advisory: inheritance reaches the base style only.",
      "A later rule's own value still beats an earlier `inherit`.",
    ],
  },

  "keyword-initial": {
    examples: [
      {
        title: "Re-dock a toast from the top to the bottom",
        lang: "css",
        code: `.toast {
  position: absolute;
  top: 16px;
  right: 16px;
}
.toast.bottom {
  top: initial;
  bottom: 16px;
}`,
      },
      {
        title: "Remove an inherited text shadow",
        lang: "css",
        code: `.hero-title {
  text-shadow: 0 1px 2px #00000066;
}
.hero-title .badge {
  text-shadow: initial;
}`,
      },
    ],
    edgeCases: [
      "`text-shadow: initial` removes every shadow, as `none` does.",
      "`--name: initial` leaves a custom property unset, so `var(--name, fallback)` takes the fallback.",
    ],
  },

  "keyword-unset": {
    examples: [
      {
        title: "Move a drawer from the right edge to the left",
        lang: "css",
        code: `.drawer {
  position: absolute;
  top: 0;
  bottom: 0;
  right: 0;
}
.drawer.left {
  right: unset;
  left: 0;
}`,
      },
    ],
    edgeCases: [
      "On a custom property, `unset` takes the parent's value; on one registered with `@property` and `inherits: false`, it takes the `initial-value`.",
    ],
  },

  "keyword-revert": {
    examples: [
      {
        title: "Drop a local token override",
        lang: "css",
        code: `:root {
  --accent: #7aa2ff;
}
.danger {
  --accent: #ff5c5c;
}
.dialog .danger {
  --accent: revert;
}`,
        note: "Inside `.dialog`, a `.danger` element takes `--accent` from its parent instead of its own red.",
      },
    ],
    edgeCases: [
      "On a custom property, `revert` takes the parent's value; on one registered with `@property` and `inherits: false`, it takes the `initial-value`.",
    ],
  },

  all: {
    edgeCases: ["`all: unset` and the other values are not applied, and `sk dev` prints `property not mapped`."],
  },
};
