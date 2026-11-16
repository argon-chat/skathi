import type { Details } from "../../details";

export const details: Details = {
  "media-types": {
    examples: [
      {
        title: "Share a stylesheet with a web build",
        lang: "css",
        code: `@media screen and (min-width: 900px) {
  .layout {
    grid-template-columns: 240px 1fr;
  }
}

@media print {
  .toolbar {
    display: none;
  }
}`,
        note: "The `print` block parses and never applies, so a stylesheet written for a browser keeps working unchanged.",
      },
    ],
    edgeCases: [
      "`not print` always matches, because `print` never does.",
      "`only screen` is the same as `screen`.",
      "An unknown media type such as `tv` or `speech` drops the whole `@media` block with an advisory instead of never matching.",
    ],
  },

  "media-width": {
    examples: [
      {
        title: "Collapse the sidebar in a narrow window",
        lang: "css",
        code: `.layout {
  display: grid;
  grid-template-columns: 240px 1fr;
}

@media (max-width: 720px) {
  .layout {
    grid-template-columns: 1fr;
  }
  .sidebar {
    display: none;
  }
}`,
      },
      {
        title: "Follow the same breakpoint from script",
        lang: "ts",
        code: `const narrow = matchMedia("(max-width: 720px)");

function applyLayout(isNarrow: boolean) {
  sidebar.collapsed = isNarrow;
}

applyLayout(narrow.matches);
narrow.addEventListener("change", (event) => applyLayout(event.matches));`,
      },
    ],
    edgeCases: [
      "`width` is the window's content width, the same number as `innerWidth`.",
      "`max-width` is inclusive: `(max-width: 720px)` and `(min-width: 720px)` both match a 720px window. Write `(width < 720px)` to split cleanly.",
      "`(width: 800px)` tests for exactly 800px. The bare `(width)` always matches.",
      "In a popout, `@media` rules answer for the popout's own size, but `matchMedia()` always answers for the main window, even from code that renders into the popout.",
    ],
  },

  "media-height": {
    examples: [
      {
        title: "Drop a decorative header in a short window",
        lang: "css",
        code: `.hero {
  height: 160px;
}

@media (max-height: 480px) {
  .hero {
    display: none;
  }
}`,
      },
    ],
    edgeCases: [
      "`height` is the window's content height, the same number as `innerHeight`.",
      "`max-height` is inclusive: at exactly 480px both `(max-height: 480px)` and `(min-height: 480px)` match.",
      "In a popout, `@media` rules answer for the popout's own height; `matchMedia()` answers for the main window.",
    ],
  },

  "media-range-syntax": {
    examples: [
      {
        title: "Give a medium window a two-column layout",
        lang: "css",
        code: `.cards {
  display: grid;
  grid-template-columns: 1fr;
}

@media (600px <= width < 1000px) {
  .cards {
    grid-template-columns: 1fr 1fr;
  }
}

@media (width >= 1000px) {
  .cards {
    grid-template-columns: repeat(3, 1fr);
  }
}`,
        note: "Each boundary belongs to exactly one block, which `min-width` and `max-width` cannot express.",
      },
    ],
    edgeCases: [
      "`<` and `>` exclude the boundary: `(400px < width < 800px)` matches neither 400px nor 800px.",
      "The range syntax also works on `resolution`, for example `(1dppx < resolution < 3dppx)`.",
    ],
  },

  "media-prefers-color-scheme": {
    examples: [
      {
        title: "Switch theme tokens with the system",
        lang: "css",
        code: `:root {
  --bg: #ffffff;
  --fg: #1d1f23;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #16181c;
    --fg: #e3e5e8;
  }
}

.app {
  background-color: var(--bg);
  color: var(--fg);
}`,
      },
      {
        title: "Let the user pick light, dark or system",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watch } from "vue";
import { appWindow } from "@skathi-native/runtime";

const theme = ref(appWindow.theme);
watch(theme, (value) => (appWindow.theme = value));
</script>

<template>
  <select v-model="theme" class="theme-picker">
    <option value="system">Match system</option>
    <option value="light">Light</option>
    <option value="dark">Dark</option>
  </select>
</template>`,
        note: "Setting `appWindow.theme` moves `prefers-color-scheme`, so the stylesheet above follows the choice.",
      },
    ],
    edgeCases: [
      'Changing `appWindow.theme` fires `change` on a `matchMedia("(prefers-color-scheme: dark)")` list.',
      "The bare `(prefers-color-scheme)` always matches.",
      "An unknown value, such as `no-preference`, drops the whole `@media` block with an advisory.",
    ],
  },

  "media-prefers-reduced-motion": {
    examples: [
      {
        title: "Stop auto-advancing a carousel",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const slide = ref(0);
const reduce = matchMedia("(prefers-reduced-motion: reduce)");
let timer: ReturnType<typeof setInterval> | undefined;

function schedule() {
  clearInterval(timer);
  if (!reduce.matches) timer = setInterval(() => (slide.value = (slide.value + 1) % 4), 5000);
}

onMounted(() => {
  schedule();
  reduce.addEventListener("change", schedule);
});
onUnmounted(() => {
  clearInterval(timer);
  reduce.removeEventListener("change", schedule);
});
</script>

<template>
  <div class="carousel">Slide {{ slide + 1 }} of 4</div>
</template>`,
        note: "Motion driven from script is yours to stop; CSS motion is handled for you, as the edge cases below describe.",
      },
      {
        title: "Jump instead of scrolling smoothly",
        lang: "css",
        code: `.message-list {
  scroll-behavior: smooth;
}

@media (prefers-reduced-motion: reduce) {
  .message-list {
    scroll-behavior: auto;
  }
}`,
      },
    ],
    edgeCases: [
      "While the system asks for less motion, Skathi jumps every CSS transition to its end value and does not run `@keyframes` animations, whether or not your stylesheet has this query.",
      "Turning the setting on while an animation runs cancels it and fires `animationcancel`.",
      "The bare `(prefers-reduced-motion)` matches only when the setting is `reduce`.",
    ],
  },

  "media-resolution": {
    examples: [
      {
        title: "Load a sharper image on a high-density display",
        lang: "css",
        code: `.brand-logo {
  width: 120px;
  height: 32px;
  background-image: url(logo.png);
  background-size: contain;
}

@media (min-resolution: 2dppx) {
  .brand-logo {
    background-image: url(logo@2x.png);
  }
}`,
      },
      {
        title: "Re-render a canvas when the window moves to another display",
        lang: "ts",
        code: `let density = matchMedia("(resolution: " + devicePixelRatio + "dppx)");

function onDensityChange() {
  density.removeEventListener("change", onDensityChange);
  chart.redraw(devicePixelRatio);
  density = matchMedia("(resolution: " + devicePixelRatio + "dppx)");
  density.addEventListener("change", onDensityChange);
}

density.addEventListener("change", onDensityChange);`,
        note: "`devicePixelRatio` has no change event; a query for the current density does, and each change needs a query for the new one.",
      },
    ],
    edgeCases: [
      "The value is the window's scale factor, `devicePixelRatio`: `2dppx` on a display scaled to 200%.",
      "`dpi` divides by 96 and `dpcm` converts through inches: `(min-resolution: 192dpi)` is `(min-resolution: 2dppx)`.",
      "`(resolution: 1.5dppx)` tests for exactly 1.5. The bare `(resolution)` always matches.",
      "A popout answers with its own window's scale factor.",
    ],
  },

  "media-display-mode": {
    examples: [
      {
        title: "Hide a custom titlebar in fullscreen",
        lang: "css",
        code: `.titlebar {
  height: 36px;
  app-region: drag;
}

@media (display-mode: fullscreen) {
  .titlebar {
    display: none;
  }
}`,
      },
      {
        title: "Draw your own titlebar only when the OS does not",
        lang: "css",
        code: `.titlebar {
  display: none;
}

@media (display-mode: window-controls-overlay) {
  .titlebar {
    display: flex;
  }
}`,
        note: 'With `titleBarStyle: "default"` in the `window` section of `sk.config.json`, the OS draws the title bar and the window reports `standalone` instead.',
      },
    ],
    edgeCases: [
      'A window with the default `titleBarStyle: "hidden"` matches `window-controls-overlay`; one with `titleBarStyle: "default"` matches `standalone`.',
      "`fullscreen` replaces either mode while the window is fullscreen.",
      "A popout with OS chrome matches `standalone`; one without matches `window-controls-overlay`.",
      "`minimal-ui` and `picture-in-picture` never match, like `browser`. An unknown value drops the whole block with an advisory.",
    ],
  },

  "media-pointer": {
    examples: [
      {
        title: "Show row actions on hover for a mouse",
        lang: "css",
        code: `@media (pointer: fine) {
  .row .row-actions {
    opacity: 0;
  }
  .row:hover .row-actions {
    opacity: 1;
  }
}`,
      },
    ],
    edgeCases: [
      "The primary pointer of a desktop window is the mouse, so `(pointer: fine)` always matches, even on a touch laptop.",
      "`(pointer: coarse)` and `(pointer: none)` never match. Test `any-pointer` to detect a touch screen.",
      "An unknown value drops the whole `@media` block with an advisory.",
    ],
  },

  "media-hover": {
    examples: [
      {
        title: "Show a tooltip on hover",
        lang: "css",
        code: `.icon-button .tooltip {
  opacity: 0;
  transition: opacity 120ms ease;
}

@media (hover: hover) {
  .icon-button:hover .tooltip {
    opacity: 1;
  }
}`,
      },
    ],
    edgeCases: [
      "`(hover: hover)` always matches and `(hover: none)` never does: the primary pointer is the mouse.",
      "An unknown value such as `(hover: sometimes)` drops the whole `@media` block with an advisory.",
    ],
  },

  "media-any-pointer": {
    examples: [
      {
        title: "Enlarge hit targets when a touch screen is attached",
        lang: "css",
        code: `.toolbar button {
  min-height: 28px;
}

@media (any-pointer: coarse) {
  .toolbar button {
    min-height: 40px;
    padding: 0 14px;
  }
}`,
      },
    ],
    edgeCases: [
      "The digitizer check runs once, so a pen tablet plugged in later does not change the answer until the app restarts.",
    ],
  },

  "media-any-hover": {
    examples: [
      {
        title: "Reveal a drag handle on hover",
        lang: "css",
        code: `.list-item .drag-handle {
  opacity: 0;
}

@media (any-hover: hover) {
  .list-item:hover .drag-handle {
    opacity: 1;
  }
}`,
      },
    ],
    edgeCases: [
      "`(any-hover: hover)` always matches and `(any-hover: none)` never does, since the mouse can always hover.",
    ],
  },

  "media-logical-operators": {
    examples: [
      {
        title: "Use a compact layout in a narrow or short window",
        lang: "css",
        code: `@media (max-width: 600px), (max-height: 420px) {
  .composer {
    padding: 6px;
  }
}

@media (prefers-color-scheme: dark) and (min-resolution: 2dppx) {
  .divider {
    background-color: #ffffff14;
  }
}

@media not all and (min-width: 900px) {
  .inspector {
    display: none;
  }
}`,
      },
    ],
    edgeCases: [
      "An unsupported feature in any query of a comma-separated list drops the whole block, not only that query as a browser does.",
      "`or` works inside a query: `(width > 300px) or (prefers-reduced-motion: reduce)`.",
    ],
  },

  "media-orientation": {
    examples: [
      {
        title: "Instead: stack panels in a tall, narrow window",
        lang: "css",
        code: `@media (max-width: 700px) and (min-height: 700px) {
  .split {
    flex-direction: column;
  }
}`,
      },
    ],
    edgeCases: ['`matchMedia("(orientation: landscape)").matches` is `false`, with an advisory.'],
  },

  "media-aspect-ratio": {
    examples: [
      {
        title: "Instead: hide a side panel in a wide but short window",
        lang: "css",
        code: `@media (min-width: 1200px) and (max-height: 600px) {
  .side-panel {
    display: none;
  }
}`,
      },
    ],
    edgeCases: ['`matchMedia("(aspect-ratio: 16/9)").matches` is `false`, with an advisory.'],
  },

  "media-webkit-device-pixel-ratio": {
    examples: [
      {
        title: "Instead: load a sharper image on a high-density display",
        lang: "css",
        code: `@media (min-resolution: 2dppx) {
  .brand-logo {
    background-image: url(logo@2x.png);
  }
}`,
      },
    ],
  },

  "media-device-width": {
    examples: [
      {
        title: "Instead: widen the layout when the window is large",
        lang: "css",
        code: `@media (min-width: 1440px) {
  .content {
    max-width: 1200px;
  }
}`,
        note: "`screen.width` gives the display's width in script.",
      },
    ],
  },

  "media-device-height": {
    examples: [
      {
        title: "Instead: show more rows when the window is tall",
        lang: "css",
        code: `@media (min-height: 1000px) {
  .preview {
    height: 480px;
  }
}`,
      },
    ],
  },

  "media-custom-media": {
    edgeCases: [
      "A query that uses a custom media name, such as `@media (--narrow)`, drops its whole block with an advisory.",
    ],
  },
};
