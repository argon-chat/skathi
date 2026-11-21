import type { Details } from "../../details";

const IGNORED = "The whole block, rules inside included, is ignored with an advisory.";

export const details: Details = {
  "at-media": {
    examples: [
      {
        title: "Collapse the sidebar in a narrow window",
        lang: "css",
        code: `.sidebar {
  width: 240px;
}
@media (max-width: 720px) {
  .sidebar {
    width: 56px;
  }
  .sidebar .label {
    display: none;
  }
}`,
      },
      {
        title: "Follow the system's dark theme",
        lang: "css",
        code: `:root {
  --bg: #ffffff;
  --fg: #1d1f23;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #16181c;
    --fg: #e6e8eb;
  }
}`,
      },
      {
        title: "Change the grid's column count with range syntax",
        lang: "css",
        code: `.gallery {
  display: grid;
  grid-template-columns: 1fr;
}
@media (600px <= width < 1000px) {
  .gallery {
    grid-template-columns: repeat(2, 1fr);
  }
}
@media (width >= 1000px) {
  .gallery {
    grid-template-columns: repeat(3, 1fr);
  }
}`,
      },
    ],
    edgeCases: [
      "Each window answers for itself: at the same moment, one rule can take the narrow branch in a 380px popout and the wide branch in the main window.",
      "A matching `@media` block adds no specificity: an unconditional rule later in the sheet with the same specificity still wins.",
      "Nested `@media` blocks apply only when every level matches.",
      "`@media print` blocks parse and never apply, without an advisory; an unknown media type drops its block with one.",
    ],
  },

  "at-supports": {
    examples: [
      {
        title: "Fall back to an opaque sheet without background blur",
        lang: "css",
        code: `.sheet {
  background-color: rgb(28 30 34 / 0.96);
}
@supports (backdrop-filter: blur(12px)) {
  .sheet {
    background-color: rgb(28 30 34 / 0.7);
    backdrop-filter: blur(12px);
  }
}`,
      },
      {
        title: "Choose a layout path from script",
        lang: "ts",
        code: `const fitsContent = CSS.supports("width", "fit-content");
const canBlur = CSS.supports("(backdrop-filter: blur(8px)) and (display: grid)");
document.documentElement.classList.toggle("no-fit-content", !fitsContent);
document.documentElement.classList.toggle("no-blur", !canBlur);`,
      },
    ],
    edgeCases: [
      "`(width: fit-content)` is false. `(resize: none)` and `(tab-size: 4)` are true, even though boxes have no resize handle and tab sizing is fixed.",
      "`(-webkit-backdrop-filter: blur(2px)) or (backdrop-filter: blur(2px))` is true when either spelling applies.",
      "`selector()` tests what the cascade can run: `selector(:has(a))` is true, while `selector(::first-line)` and `selector(:not(:hover))` are false.",
      "`CSS.supports()` gives the same answers, in its one-argument and two-argument forms.",
    ],
  },

  "at-container": {
    examples: [
      {
        title: "Arrange the caption controls in a row on a wide preview",
        lang: "css",
        code: `.preview {
  container-type: size;
  width: 300px;
  height: 200px;
}
.caption { display: flex; flex-direction: column; }
@container (aspect-ratio >= 3/2) {
  .caption { flex-direction: row; }
}`,
        note: "The query measures the preview's content box. It also works in a portrait window.",
      },
      {
        title: "Nested named containers",
        lang: "css",
        code: `.page {
  container: page / inline-size;
}
.panel {
  container: panel / inline-size;
}
@container page (width >= 60rem) {
  .toolbar {
    gap: 12px;
  }
}
@container panel (width < 320px) {
  .toolbar .label {
    display: none;
  }
}`,
        note: "Each rule asks its own container: the toolbar's spacing follows the page, its labels the panel it sits in.",
      },
      {
        title: "Switch on a theme the container sets",
        lang: "css",
        code: `.sidebar {
  --tone: muted;
}
@container style(--tone: muted) {
  .item {
    opacity: 0.7;
  }
}`,
      },
    ],
    edgeCases: [
      "With no container that can answer the query (no ancestor of that name, or only `inline-size` ones for a `height` query), the rules do not apply, and neither does their `not`.",
      "Before the container's first layout its size is unknown; the frame lays it out, restyles and lays out again before painting, so the first paint already answers the query.",
      "An element mounted in that frame starts no transition from the unknown-size style.",
      "A container whose size depends on its own query results (a cycle) stops after 8 layouts in one frame, with an advisory.",
    ],
  },

  "at-layer": {
    edgeCases: [
      '`@import "base.css" layer(base)` is inlined by `sk dev` and `sk build` without the layer, so those rules do apply, in source order.',
    ],
  },

  "at-keyframes": {
    examples: [
      {
        title: "Spin a loading indicator",
        lang: "css",
        code: `@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.spinner {
  width: 16px;
  height: 16px;
  animation: spin 800ms linear infinite;
}`,
      },
      {
        title: "Slide a toast in from below",
        lang: "css",
        note: "The keyframes give only `from`, so the toast ends on its own `opacity` and `transform`.",
        code: `@keyframes toast-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
}
.toast {
  animation: toast-in 200ms ease-out;
}`,
      },
      {
        title: "Grow each meter bar to its own level",
        lang: "vue",
        note: "One `@keyframes` serves every bar: `var(--level)` resolves against each animating element.",
        code: `<script setup lang="ts">
defineProps<{ levels: number[] }>();
</script>

<template>
  <div class="meters">
    <div v-for="(level, i) in levels" :key="i" class="bar" :style="{ '--level': level + '%' }" />
  </div>
</template>

<style scoped>
@keyframes grow {
  from {
    width: 0;
  }
  to {
    width: var(--level);
  }
}
.bar {
  height: 6px;
  background-color: #4c8dff;
  animation: grow 600ms ease-out forwards;
}
</style>`,
      },
    ],
    edgeCases: [
      "A missing `from` or `to` keyframe takes the element's own value, as in the toast example.",
      "A later `@keyframes` with the same name replaces an earlier one. One inside `@media` exists only in windows where the query holds, so a popout can run a different version than the main window.",
      "An unresolvable `var()` in a keyframe drops only that declaration, silently; the keyframe's other declarations still run.",
      "While the OS asks for reduced motion, CSS animations do not run: the element shows its own style and `animationend` does not fire.",
      "A declaration marked `!important` inside a keyframe is applied like the others; browsers ignore it.",
    ],
  },

  "at-font-face": {
    examples: [
      {
        title: "Bundle a variable font for the whole app",
        lang: "css",
        code: `@font-face {
  font-family: "Inter";
  src: url("./fonts/InterVariable.woff2") format("woff2-variations");
  font-weight: 100 900;
}
body {
  font-family: "Inter", sans-serif;
}`,
      },
      {
        title: "Lead the stack with an icon font limited to its code points",
        lang: "css",
        note: "Latin text skips `App Icons` and is drawn with `Inter`; icon code points in the private use area are drawn with the icon font.",
        code: `@font-face {
  font-family: "App Icons";
  src: url("./fonts/app-icons.woff2") format("woff2");
  unicode-range: U+E000-E0FF;
}
.button-label {
  font-family: "App Icons", "Inter", sans-serif;
}`,
      },
    ],
    edgeCases: [
      "A face loads the first time text drawn in its family contains a character in its `unicode-range`; until then the text is drawn with the next font and then swaps. A family used only by empty elements never loads.",
      "`src: local()` is not consulted, with an advisory, and the next `url()` is loaded. A source whose `format()` is not `woff`, `woff2`, `truetype`, `opentype` or `collection` (with or without `-variations`) is skipped.",
      "A web family listed after an installed font in `font-family` is never reached: the installed font draws the text, with an advisory. List the web family first.",
      "An `@font-face` inside `@media` or `@supports` is declared whatever the condition.",
    ],
  },

  "at-import": {
    examples: [
      {
        title: "Split the app's stylesheet into files",
        lang: "css",
        code: `@import "./tokens.css";
@import "./layout.css";
@import "./compact.css" (max-width: 700px);`,
      },
      {
        title: "Share form styles between components",
        lang: "vue",
        code: `<template>
  <form class="settings">
    <input class="field" placeholder="Display name" />
  </form>
</template>

<style scoped>
@import "../styles/forms.css";
.settings {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
</style>`,
      },
    ],
    edgeCases: [
      "A media query after the URL wraps the imported rules in `@media`, as `compact.css` is in the first example. A `layer()` or `supports()` condition is dropped and the rules are inlined unconditionally.",
      "A path that does not resolve to a file is dropped with a warning. Paths starting with `/` resolve from the app root, `./` and `../` from the importing file, and bare names from packages.",
      "A file imported twice, or in a cycle, is inlined once.",
      "Relative `url()`s inside an imported file resolve against that file, not the importing one.",
    ],
  },

  "at-property": {
    examples: [
      {
        title: "Sweep a shimmer across a loading skeleton",
        lang: "css",
        code: `@property --shine {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 0%;
}
@keyframes shimmer {
  to {
    --shine: 100%;
  }
}
.skeleton {
  background-image: linear-gradient(90deg, #2a2d33, #3a3e46, #2a2d33);
  background-size: 200% 100%;
  background-position: var(--shine) 0;
  animation: shimmer 1.2s linear infinite;
}`,
      },
    ],
    edgeCases: [
      "With `inherits: false`, an element that does not set the property itself gets the `initial-value`, whatever its parent sets.",
      "A `<percentage>` registration whose `initial-value` is missing or not a percentage is not registered, and no advisory is printed.",
      "A declaration of the property marked `!important` on the element wins over the animation, as in browsers.",
      "`CSS.registerProperty()` does nothing and prints an advisory; register the property with `@property` in a stylesheet.",
    ],
  },

  "at-scope": {
    examples: [
      {
        title: "Instead: keep a card's title style inside the card component",
        lang: "vue",
        note: "Only elements this component's template renders match `.title`; content passed in through the slot keeps its own styles.",
        code: `<script setup lang="ts">
defineProps<{ title: string }>();
</script>

<template>
  <article class="card">
    <h3 class="title">{{ title }}</h3>
    <slot />
  </article>
</template>

<style scoped>
.title {
  font-size: 14px;
  font-weight: 600;
}
</style>`,
      },
    ],
    edgeCases: [IGNORED],
  },

  "at-starting-style": {
    examples: [
      {
        title: "Fade a panel in when it is shown",
        lang: "css",
        code: `.panel {
  display: block;
  opacity: 1;
  translate: 0;
  transition: opacity 220ms ease, translate 220ms ease, display 220ms allow-discrete;

  @starting-style {
    opacity: 0;
    translate: 0 8px;
  }
}
.panel[hidden] {
  display: none;
  opacity: 0;
  translate: 0 8px;
}`,
      },
    ],
  },

  "at-page": { edgeCases: [IGNORED] },

  "at-namespace": {
    edgeCases: ["It is ignored with an advisory, and rules whose selectors use a namespace prefix are dropped."],
  },

  "at-counter-style": {
    examples: [
      {
        title: "Instead: draw custom list markers",
        lang: "css",
        code: `.checklist li {
  list-style: none;
}
.checklist li::before {
  content: "✓ ";
  color: #3fb950;
}`,
      },
    ],
    edgeCases: [IGNORED],
  },

  "at-font-feature-values": { edgeCases: [IGNORED] },
};
