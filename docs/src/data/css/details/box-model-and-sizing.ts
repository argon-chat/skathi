import type { Details } from "../../details";

const logicalSizeEdge =
  "A declaration prints an advisory in `sk dev` and changes nothing, and `CSS.supports()` answers `false` for it.";

export const details: Details = {
  width: {
    examples: [
      {
        title: "Collapse a sidebar to an icon rail",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const collapsed = ref(false);
</script>

<template>
  <aside class="sidebar" :class="{ collapsed }">
    <button @click="collapsed = !collapsed">Toggle</button>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 240px;
  flex-shrink: 0;
  transition: width 180ms ease;
}
.sidebar.collapsed {
  width: 56px;
}
</style>`,
      },
      {
        title: "Fill a row minus a fixed gutter",
        lang: "css",
        code: `.composer-input {
  width: calc(100% - 40px);
}`,
      },
    ],
    edgeCases: [
      "A percentage, alone or inside `calc()`, resolves against the containing block's width.",
      "`min()`, `max()` and `clamp()` over a percentage and a length drop the declaration with an advisory; write `width: 100%; max-width: 640px` instead of `width: min(100%, 640px)`.",
      '`CSS.supports("width", "min-content")` answers `false`, as it does for the other ignored sizing keywords.',
    ],
  },
  height: {
    examples: [
      {
        title: "Give a list row a fixed height",
        lang: "css",
        code: `.message-row {
  display: flex;
  align-items: center;
  height: 40px;
}`,
      },
      {
        title: "Make the app shell fill the window",
        lang: "css",
        code: `.app-shell {
  height: 100vh;
  display: flex;
  flex-direction: column;
}
.content {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}`,
      },
    ],
    edgeCases: [
      "With `aspect-ratio`, a `height` set together with `width` wins: the ratio only fills in the dimension that is `auto`.",
    ],
  },
  "min-width": {
    examples: [
      {
        title: "Let a long title truncate inside a flex row",
        lang: "css",
        code: `.chat-header {
  display: flex;
  align-items: center;
  gap: 8px;
}
.chat-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}`,
        note: "Without `min-width: 0` the title never shrinks below its one line of text, and pushes the row wider.",
      },
      {
        title: "Keep a button wide enough for its label",
        lang: "css",
        code: `.dialog-button {
  min-width: 88px;
  padding: 6px 12px;
}`,
      },
    ],
    edgeCases: [
      "A flex item's `min-width: auto` is its min-content width: wrapping text can shrink to its longest word, and `white-space: nowrap` text keeps its whole line.",
      "An item with `overflow: hidden`, `auto` or `scroll` has an automatic minimum of `0`, so it shrinks without `min-width: 0`.",
    ],
  },
  "min-height": {
    examples: [
      {
        title: "Let a list scroll inside a flex column",
        lang: "css",
        code: `.panel {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.panel-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}`,
        note: "A flex item does not shrink below its content height on its own, so the list would grow past the panel instead of scrolling.",
      },
      {
        title: "Reserve space for an empty state",
        lang: "css",
        code: `.drop-zone {
  min-height: 160px;
  display: flex;
  align-items: center;
  justify-content: center;
}`,
      },
    ],
  },
  "max-width": {
    examples: [
      {
        title: "Cap a chat bubble's width",
        lang: "css",
        code: `.bubble {
  max-width: 520px;
  padding: 8px 12px;
  border-radius: 12px;
  overflow-wrap: break-word;
}`,
      },
      {
        title: "Center a readable column in a wide window",
        lang: "css",
        code: `.article {
  width: 100%;
  max-width: 680px;
  margin-inline: auto;
}`,
      },
    ],
    edgeCases: [
      "`max-width: none` removes a cap set by an earlier rule.",
      "When `max-width` is smaller than `min-width`, `min-width` wins.",
    ],
  },
  "max-height": {
    examples: [
      {
        title: "Let a dropdown scroll past a height",
        lang: "css",
        code: `.dropdown-menu {
  max-height: 320px;
  overflow-y: auto;
}`,
      },
      {
        title: "Expand a collapsible section with a transition",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const open = ref(false);
</script>

<template>
  <button @click="open = !open">Details</button>
  <div class="details" :class="{ open }"><slot /></div>
</template>

<style scoped>
.details {
  max-height: 0;
  overflow: hidden;
  transition: max-height 200ms ease;
}
.details.open {
  max-height: 400px;
}
</style>`,
      },
    ],
    edgeCases: ["`max-height: none` removes a cap set by an earlier rule."],
  },
  "box-sizing": {
    examples: [
      {
        title: "Size a padded input to its declared width",
        lang: "css",
        code: `.search-input {
  box-sizing: border-box;
  width: 240px;
  padding: 6px 10px;
  border: 1px solid #444;
}`,
        note: "The input is 240px wide including its padding and border, which is what every box does here.",
      },
    ],
    edgeCases: ["`content-box` prints an advisory in `sk dev`."],
  },
  "aspect-ratio": {
    examples: [
      {
        title: "Keep video thumbnails at 16:9 in a grid",
        lang: "css",
        code: `.thumbnail {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  border-radius: 8px;
}`,
      },
      {
        title: "Draw a square avatar from its width alone",
        lang: "css",
        code: `.avatar {
  width: 36px;
  aspect-ratio: 1;
  border-radius: 50%;
  flex-shrink: 0;
}`,
      },
    ],
    edgeCases: [
      "The ratio fills in only the dimension that is `auto`: with both `width` and `height` set, it is ignored.",
      "A single number is a ratio over `1`, so `aspect-ratio: 1.5` is `3 / 2`.",
    ],
  },
  "inline-size": {
    examples: [
      {
        title: "Instead: size a search field's width",
        lang: "css",
        code: `.search-field {
  width: 280px;
}`,
      },
    ],
    edgeCases: [logicalSizeEdge],
  },
  "block-size": {
      "With `max-width: 200px` and a ratio of `2`, a block that would stretch to 400px is 200 × 100: the limit carries through the ratio.",
      "A transition between two ratios interpolates them on a log scale, as browsers do; to or from `auto` it snaps.",
    examples: [
      {
        title: "Instead: give a toolbar a fixed height",
        lang: "css",
        code: `.toolbar {
  height: 40px;
}`,
      },
    ],
    edgeCases: [logicalSizeEdge],
  },
  "min-inline-size": {
    examples: [
      {
        title: "Instead: keep a button from getting too narrow",
        lang: "css",
        code: `.dialog-button {
  min-width: 88px;
}`,
      },
    ],
    edgeCases: [logicalSizeEdge],
  },
  "max-inline-size": {
    examples: [
      {
        title: "Instead: cap a paragraph's line length",
        lang: "css",
        code: `.release-notes p {
  max-width: 640px;
}`,
      },
    ],
    edgeCases: [logicalSizeEdge],
  },
  "min-block-size": {
    examples: [
      {
        title: "Instead: reserve height for an empty list",
        lang: "css",
        code: `.empty-list {
  min-height: 120px;
}`,
      },
    ],
    edgeCases: [logicalSizeEdge],
  },
  "max-block-size": {
    examples: [
      {
        title: "Instead: cap a popover's height",
        lang: "css",
        code: `.popover {
  max-height: 360px;
  overflow-y: auto;
}`,
      },
    ],
    edgeCases: [logicalSizeEdge],
  },
  overflow: {
    examples: [
      {
        title: "Clip a list to a rounded card",
        lang: "css",
        code: `.card {
  border-radius: 12px;
  overflow: hidden;
}
.card-row {
  height: 40px;
}`,
        note: "The rows are clipped at the card's curve, not at its square corners.",
      },
      {
        title: "Scroll a log panel",
        lang: "css",
        code: `.log-panel {
  height: 200px;
  overflow: auto;
  font-family: monospace;
}`,
      },
    ],
    edgeCases: [
      "With a `border` and `border-radius`, the content is clipped to the inner rounded edge, inside the border.",
      "`contain: paint` makes a `visible` axis compute to `clip`.",
      "An element that does not scroll reports `scrollTop` as `0` and ignores writes to it.",
    ],
  },
  "overflow-x": {
    examples: [
      {
        title: "Scroll a wide table sideways",
        lang: "css",
        code: `.table-wrap {
  overflow-x: auto;
  overflow-y: hidden;
}
.table {
  width: 960px;
}`,
      },
    ],
    edgeCases: [
      "When only one axis is `visible` or `clip`, it computes to `auto` or `hidden` against the other: `overflow-x: visible; overflow-y: hidden` lets the x axis scroll.",
    ],
  },
  "overflow-y": {
    examples: [
      {
        title: "Scroll a sidebar's channel list",
        lang: "css",
        code: `.channel-list {
  height: 100%;
  overflow-y: auto;
}`,
      },
    ],
    edgeCases: [
      "`overflow-y: auto` alone makes `overflow-x` compute to `auto` too, so a child wider than the box scrolls sideways, by script and by wheel; set `overflow-x: hidden` to stop it.",
    ],
  },
  "overflow-clip-margin": {
    edgeCases: ["A declaration prints an advisory in `sk dev` and changes nothing."],
  },
  visibility: {
    examples: [
      {
        title: "Show a row's timestamp on hover without shifting the layout",
        lang: "css",
        code: `.message .timestamp {
  visibility: hidden;
}
.message:hover .timestamp {
  visibility: visible;
}`,
      },
      {
        title: "Hide a popover until it is measured",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const ready = ref(false);
const top = ref(0);
const popover = ref<HTMLElement | null>(null);

onMounted(() => {
  top.value = -popover.value!.getBoundingClientRect().height;
  ready.value = true;
});
</script>

<template>
  <div
    ref="popover"
    class="popover"
    :style="{ top: top + 'px', visibility: ready ? 'visible' : 'hidden' }"
  >
    <slot />
  </div>
</template>`,
        note: "The hidden popover keeps its box, so it can be measured before it is shown.",
      },
    ],
    edgeCases: [
      "An element inside a `visibility: hidden` box refuses `focus()`; a `visibility` change written on the line before `focus()` already counts.",
      "A descendant that does not declare `visibility` reads `hidden` from `getComputedStyle()`.",
    ],
  },
};
