import type { Details } from "../../details";

export const details: Details = {
  "feature-style-blocks": {
    examples: [
      {
        title: "Style a settings panel from its own component",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const dirty = ref(false);
</script>

<template>
  <section class="settings" :class="{ dirty }">
    <h2 class="settings-title">Notifications</h2>
    <label><input type="checkbox" @change="dirty = true" /> Desktop alerts</label>
  </section>
</template>

<style>
.settings {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
}
.settings.dirty .settings-title {
  color: #f5a524;
}
</style>`,
      },
      {
        title: "Load a global theme sheet from the entry module",
        lang: "ts",
        code: `import { createApp } from "vue";
import App from "./App.vue";
import "./styles/theme.css";

createApp(App).mount();`,
        note: "A `.css` file imported from script applies to every window of the app, like a `<style>` block without `scoped`.",
      },
      {
        title: "Split a theme into partial sheets",
        lang: "css",
        code: `@import "./tokens.css";
@import "./compact.css" (max-width: 720px);

body {
  background: var(--surface);
  color: var(--text);
}`,
      },
    ],
    edgeCases: [
      "A local `@import` is inlined at build time. A path starting with `/` resolves from the app's root (the folder that holds `sk.config.json`); one that does not resolve is dropped with an advisory, and a file imported twice is included once.",
      "`@import` with a media query wraps the imported rules in that `@media` block.",
      "A relative `url()` in a `<style>` block resolves against the file that declares it.",
      "In `sk dev`, an edit to a `<style>` block's text swaps the CSS without reloading, so component state survives. Adding or removing `scoped`, `module` or `lang` reloads the app.",
      "`<style>` and local `<link rel=\"stylesheet\">` tags in `index.html`'s `<head>` load before the app's own CSS, so a component rule of equal specificity wins over them; those in `<body>` load after it.",
    ],
  },
  "feature-scoped-styles": {
    examples: [
      {
        title: "Keep a card's class names from leaking into the rest of the app",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ title: string; unread: number }>();
</script>

<template>
  <article class="card">
    <h3 class="title">{{ title }}</h3>
    <span v-if="unread" class="badge">{{ unread }}</span>
  </article>
</template>

<style scoped>
.card {
  display: flex;
  align-items: center;
  gap: 8px;
}
.badge {
  padding: 0 6px;
  border-radius: 9999px;
  background: #e5484d;
}
</style>`,
      },
      {
        title: "Restyle a child component's inner parts",
        lang: "vue",
        code: `<script setup lang="ts">
import SearchField from "./SearchField.vue";
</script>

<template>
  <header class="titlebar">
    <SearchField class="titlebar-search" />
  </header>
</template>

<style scoped>
.titlebar-search {
  width: 280px;
}
.titlebar-search :deep(input) {
  height: 28px;
  border-radius: 6px;
}
</style>`,
        note: "The first rule reaches `SearchField`'s root element as it is; the `<input>` inside it needs `:deep()`.",
      },
    ],
    edgeCases: [
      "A child component's root element carries the parent's scope id as well as its own, so a parent's scoped rule styles the child's root without `:deep()`, but not the child's inner elements.",
      "Elements created with `document.createElement()` carry no scope id, so scoped rules do not match them; a `:deep()` rule on a scoped ancestor does.",
    ],
  },
  "feature-v-bind-in-css": {
    examples: [
      {
        title: "Resize a sidebar from a slider",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";

const width = ref(240);
const sidebarWidth = computed(() => width.value + "px");
</script>

<template>
  <aside class="sidebar">
    <input v-model.number="width" type="range" min="180" max="400" />
    <slot />
  </aside>
</template>

<style scoped>
.sidebar {
  width: v-bind(sidebarWidth);
  flex-shrink: 0;
}
</style>`,
        note: "The bound value is a string with its unit; a bare number would be written without one.",
      },
      {
        title: "Theme a component from a store field",
        lang: "vue",
        code: `<script setup lang="ts">
import { useTheme } from "../stores/theme";

const theme = useTheme();
</script>

<template>
  <button class="primary">Send</button>
</template>

<style scoped>
.primary {
  background: v-bind("theme.accent");
  color: v-bind("theme.onAccent");
}
</style>`,
      },
    ],
    edgeCases: [
      "Each bound value becomes a custom property on the component's root element (every root, for a fragment) and on its teleported children, so descendants that read it through `var()` update too.",
      "`null` or `undefined` is written as `initial`, and a number as its digits with no unit.",
      "A string `:style` on the root element keeps the bound custom properties; it does not wipe them.",
    ],
  },
  "feature-inline-style": {
    examples: [
      {
        title: "Position a context menu where the user right-clicked",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const menu = ref<{ x: number; y: number } | null>(null);

function open(event: MouseEvent) {
  menu.value = { x: event.clientX, y: event.clientY };
}
</script>

<template>
  <div class="canvas" @contextmenu.prevent="open">
    <ul v-if="menu" class="menu" :style="{ left: menu.x + 'px', top: menu.y + 'px' }">
      <li>Copy</li>
      <li>Paste</li>
    </ul>
  </div>
</template>`,
      },
      {
        title: "Override a rule from script with important priority",
        lang: "ts",
        code: `const panel = document.querySelector<HTMLElement>(".inspector")!;

panel.style.setProperty("width", "320px", "important");
panel.style.setProperty("--panel-accent", "#4f8cff");

panel.style.getPropertyPriority("width");
panel.style.removeProperty("width");`,
        note: '`getPropertyPriority` answers `"important"` here, and `removeProperty` returns the value it removed.',
      },
    ],
    edgeCases: [
      'Assigning a value that ends in `!important` through a property (`el.style.width = "10px !important"`) is ignored; use `setProperty(name, value, "important")`.',
      'Assigning `null` or `""` removes the declaration; assigning `undefined` changes nothing.',
      '`el.style["--accent"] = value` sets the custom property, with an advisory, because a browser ignores it; `setProperty("--accent", value)` works in both.',
      '`cssText` and the `style` attribute read back values as written: `#f00` is not re-spelled, a shorthand is not expanded (after `margin: 0`, `el.style.marginTop` reads `""`), and a value the engine rejected is still listed.',
      'An array value in `:style` (`{ display: ["-webkit-box", "flex"] }`) keeps the last entry the engine accepts.',
    ],
  },
  "feature-cascade": {
    examples: [
      {
        title: "Let a selected row's colors win over hover",
        lang: "css",
        code: `.row {
  background: transparent;
}
.row:hover {
  background: #ffffff10;
}
.row.selected {
  background: #4f8cff33;
}`,
        note: "`.row.selected` and `.row:hover` have equal specificity, so source order decides: the later rule wins while the selected row is hovered.",
      },
      {
        title: "Force a disabled look over component styles",
        lang: "css",
        code: `.toolbar-button {
  opacity: 1;
  pointer-events: auto;
}
[aria-disabled="true"] {
  opacity: 0.4 !important;
  pointer-events: none !important;
}`,
      },
    ],
    edgeCases: [
      "Inline declarations beat every rule's normal declarations, `:hover` rules included; a rule's `!important` beats inline normal declarations; inline `!important` beats both.",
      "A base rule of higher specificity still beats a `:hover`, `:focus` or `:active` rule while that state is live, as in a browser.",
      "The `hidden` attribute is styled by `:where([hidden]) { display: none }`, which has zero specificity, so any author `display` on the element shows it again.",
    ],
  },
  "feature-css-nesting": {
    examples: [
      {
        title: "Instead: write a toolbar's nested rules flat",
        lang: "css",
        code: `.toolbar {
  display: flex;
  gap: 4px;
}
.toolbar > .button {
  padding: 4px 8px;
}
.toolbar > .button:hover {
  background: #ffffff14;
}`,
      },
    ],
  },
  "feature-css-modules": {
    edgeCases: [
      "`sk dev` and `sk build` print an advisory for each `<style module>` block. Its rules are still compiled, with the class names as written, unhashed.",
      "`useCssModule()` prints an advisory and returns `{}`, so every class lookup is `undefined`.",
    ],
  },
  "feature-preprocessors": {
    examples: [
      {
        title: "Instead: import the CSS your Sass build produced",
        lang: "ts",
        code: `import { createApp } from "vue";
import App from "./App.vue";
import "./styles/theme.compiled.css";

createApp(App).mount();`,
        note: "Run `sass` (or `lessc`) on the source file before `sk dev` or `sk build`, and import its output.",
      },
    ],
    edgeCases: [
      "`sk dev` and `sk build` print an advisory naming the component whose `<style lang>` was not preprocessed.",
    ],
  },
  "feature-tailwind-3": {
    examples: [
      {
        title: "Build a hoverable list row from utilities",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ name: string; status: string }>();
</script>

<template>
  <div class="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-white/10 transition-colors">
    <span class="w-8 h-8 rounded-full bg-emerald-500 shrink-0" />
    <span class="flex-1 min-w-0 truncate text-sm text-white">{{ name }}</span>
    <span class="text-xs text-white/60">{{ status }}</span>
  </div>
</template>`,
      },
      {
        title: "Name a repeated utility set with @apply",
        lang: "vue",
        code: `<template>
  <button class="pill">Archive</button>
</template>

<style scoped>
.pill {
  @apply px-3 py-1 rounded-full text-xs font-medium bg-white/10;
}
.pill:hover {
  @apply bg-white/20;
}
</style>`,
        note: "A `<style>` block that uses `@apply`, `@tailwind`, `@layer` or `theme()` runs through Tailwind before it is compiled.",
      },
      {
        title: "Point Tailwind at the app's components",
        lang: "ts",
        code: `export default {
  content: ["./index.html", "./src/**/*.{vue,ts}"],
  theme: {
    extend: {
      colors: { brand: "#4f8cff" },
    },
  },
};`,
        note: "Saved as `tailwind.config.js` next to `sk.config.json`.",
      },
    ],
    edgeCases: [
      "New in 0.4.0: the app installs `tailwindcss` 3 itself (`bun add -d tailwindcss@3`). With a `tailwind.config.*` and no `tailwindcss`, or a Tailwind 4, `sk dev`, `sk build` and `sk check` stop and name that command.",
      "The config is the nearest `tailwind.config.*` at or above the app's source folder, and its relative `content` globs resolve against the config's own folder.",
      "The utilities sheet loads before component `<style>` blocks, so a component rule of equal specificity wins over a utility class.",
    ],
  },
  "feature-tailwind-4": {
    examples: [
      {
        title: "Instead: configure Tailwind 3.4 from a config file",
        lang: "ts",
        code: `export default {
  content: ["./src/**/*.{vue,ts}"],
  theme: {
    extend: {
      colors: { surface: "#1e1e22" },
    },
  },
};`,
        note: 'Saved as `tailwind.config.js`, in place of a v4 `@import "tailwindcss"` and `@theme` sheet; install it with `bun add -d tailwindcss@3`.',
      },
    ],
  },
};
