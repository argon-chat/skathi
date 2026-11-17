import type { Details } from "../../details";

const advisory =
  "A declaration prints an advisory in `sk dev` and changes nothing, and `CSS.supports()` answers `false` for it.";

const anchorWorkaround = {
  title: "Instead: place a tooltip under its button from script",
  lang: "ts" as const,
  code: `function placeTooltip(button: HTMLElement, tooltip: HTMLElement) {
  const box = button.getBoundingClientRect();
  tooltip.style.position = "fixed";
  tooltip.style.left = box.left + box.width / 2 + "px";
  tooltip.style.top = box.bottom + 8 + "px";
  tooltip.style.transform = "translateX(-50%)";
}`,
  note: "Call it again from a `ResizeObserver` or a `scroll` listener when the button can move.",
};

export const details: Details = {
  display: {
    examples: [
      {
        title: "Switch a file browser between list and grid views",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const view = ref<"list" | "grid">("list");
</script>

<template>
  <button @click="view = view === 'list' ? 'grid' : 'list'">Toggle view</button>
  <div class="files" :class="view"><slot /></div>
</template>

<style scoped>
.files.list {
  display: flex;
  flex-direction: column;
}
.files.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 8px;
}
</style>`,
      },
      {
        title: "Hide a drawer while it is closed",
        lang: "css",
        code: `.drawer[data-open="false"] {
  display: none;
}`,
      },
    ],
    edgeCases: [
      "The `hidden` attribute hides an element only until a rule sets its `display`: `hidden` on a `.toolbar { display: flex }` element shows it.",
      "An element inside a `display: none` box refuses `focus()`.",
    ],
  },
  flex: {
    examples: [
      {
        title: "Split a window into a fixed sidebar and a flexible pane",
        lang: "css",
        code: `.workspace {
  display: flex;
  height: 100%;
}
.sidebar {
  flex: none;
  width: 240px;
}
.editor {
  flex: 1;
  min-width: 0;
}`,
      },
      {
        title: "Share a toolbar's space two to one",
        lang: "css",
        code: `.find-bar {
  display: flex;
  gap: 8px;
}
.find-input {
  flex: 2;
}
.replace-input {
  flex: 1;
}`,
      },
    ],
  },
  "flex-grow": {
    examples: [
      {
        title: "Push the last status bar items to the right",
        lang: "vue",
        code: `<template>
  <footer class="status-bar">
    <span>Ln 12, Col 4</span>
    <span class="spacer" />
    <span>UTF-8</span>
    <span>TypeScript</span>
  </footer>
</template>

<style scoped>
.status-bar {
  display: flex;
  gap: 12px;
}
.spacer {
  flex-grow: 1;
}
</style>`,
      },
    ],
  },
  "flex-shrink": {
    examples: [
      {
        title: "Keep an avatar round in a squeezed row",
        lang: "css",
        code: `.member {
  display: flex;
  align-items: center;
  gap: 8px;
}
.member-avatar {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  flex-shrink: 0;
}`,
      },
    ],
    edgeCases: [
      "An item does not shrink below its automatic minimum width, its min-content width, whatever its `flex-shrink`; set `min-width: 0` to let it go further.",
    ],
  },
  "flex-basis": {
    examples: [
      {
        title: "Give tabs equal widths regardless of their labels",
        lang: "css",
        code: `.tabs {
  display: flex;
}
.tab {
  flex-grow: 1;
  flex-basis: 0;
}`,
      },
      {
        title: "Wrap cards at a preferred width",
        lang: "css",
        code: `.cards {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
.card {
  flex-grow: 1;
  flex-basis: 220px;
}`,
      },
    ],
    edgeCases: [
      "A percentage basis in a flex column whose height is `auto` sizes the item to its content, even over a declared `height`.",
    ],
  },
  "flex-direction": {
    examples: [
      {
        title: "Stack a settings form vertically",
        lang: "css",
        code: `.settings-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}`,
      },
      {
        title: "Right-align dialog buttons with the first one rightmost",
        lang: "css",
        code: `.dialog-actions {
  display: flex;
  flex-direction: row-reverse;
  gap: 8px;
}`,
      },
    ],
  },
  "flex-wrap": {
    examples: [
      {
        title: "Wrap tag chips onto new lines",
        lang: "css",
        code: `.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.tag {
  padding: 2px 8px;
  border-radius: 9999px;
  background: #ffffff14;
}`,
      },
    ],
  },
  "flex-flow": {
    examples: [
      {
        title: "A wrapping row of tags",
        lang: "css",
        code: `.tags {
  display: flex;
  flex-flow: row wrap;
  gap: 6px;
}`,
      },
    ],
    edgeCases: ["The two values can come in either order: `wrap column` is `column wrap`."],
  },
  order: {
    examples: [
      {
        title: "Move the primary action first in narrow windows",
        lang: "css",
        code: `.actions {
  display: flex;
  gap: 8px;
}
@media (max-width: 600px) {
  .actions .primary {
    order: -1;
  }
}`,
      },
    ],
    edgeCases: [
      "Items with equal `order` keep their document order, and a negative value places an item before the items left at `0`.",
      "Changing an item's `order` from script, for example by swapping its class, reflows the container.",
    ],
  },
  "align-items": {
    examples: [
      {
        title: "Center icons and labels in a toolbar",
        lang: "css",
        code: `.toolbar {
  display: flex;
  align-items: center;
  height: 40px;
  gap: 6px;
}`,
      },
      {
        title: "Top-align an avatar with a multi-line message",
        lang: "css",
        code: `.message-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}
.message-avatar {
  width: 36px;
  height: 36px;
  flex-shrink: 0;
}`,
      },
    ],
  },
  "align-self": {
    examples: [
      {
        title: "Align your own chat messages to the right",
        lang: "css",
        code: `.thread {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
}
.bubble.mine {
  align-self: flex-end;
}`,
      },
    ],
  },
  "align-content": {
    examples: [
      {
        title: "Keep wrapped chip lines at the top of a tall box",
        lang: "css",
        code: `.recipients {
  display: flex;
  flex-wrap: wrap;
  align-content: flex-start;
  height: 96px;
  gap: 4px;
}`,
        note: "Without it the lines stretch to share the box's height.",
      },
    ],
  },
  "justify-content": {
    examples: [
      {
        title: "Spread a title bar's title and window buttons apart",
        lang: "css",
        code: `.titlebar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 32px;
}`,
      },
      {
        title: "Center an empty state",
        lang: "css",
        code: `.empty {
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  height: 100%;
}`,
      },
    ],
  },
  "justify-items": {
    examples: [
      {
        title: "Center icons in their grid cells",
        lang: "css",
        code: `.emoji-grid {
  display: grid;
  grid-template-columns: repeat(8, 36px);
  justify-items: center;
}`,
      },
    ],
    edgeCases: [
      "On a `display: block` container, a value other than `normal` or `stretch` shrinks each block child to its content width and places it, as `justify-self` does for one child.",
      "A child's `justify-self: auto` does not defer to it: it stretches the child.",
    ],
  },
  "justify-self": {
    examples: [
      {
        title: "Pin one grid cell's content to the end",
        lang: "css",
        code: `.form-grid {
  display: grid;
  grid-template-columns: 120px 1fr;
  gap: 8px;
}
.form-grid .submit {
  grid-column: 2;
  justify-self: end;
}`,
      },
    ],
    edgeCases: [
      "On a child of a `display: block` container, `center`, `start` or `end` shrinks the child to its content width and places it in the line.",
    ],
  },
  "place-items": {
    examples: [
      {
        title: "Center a loading spinner in its pane",
        lang: "css",
        code: `.loading-pane {
  display: grid;
  place-items: center;
  height: 100%;
}`,
      },
    ],
    edgeCases: ["One value sets both axes; `normal` sets both to `stretch`."],
  },
  "place-content": {
    examples: [
      {
        title: "Instead: center a grid's tracks with the longhands",
        lang: "css",
        code: `.color-swatches {
  display: grid;
  grid-template-columns: repeat(6, 24px);
  align-content: center;
  justify-content: center;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "place-self": {
    examples: [
      {
        title: "Instead: center one grid item with the longhands",
        lang: "css",
        code: `.dashboard .clock {
  align-self: center;
  justify-self: center;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  gap: {
    examples: [
      {
        title: "Space toolbar buttons without margins",
        lang: "css",
        code: `.toolbar {
  display: flex;
  gap: 4px;
}`,
      },
      {
        title: "Space a card grid's rows and columns differently",
        lang: "css",
        code: `.cards {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px 12px;
}`,
      },
    ],
    edgeCases: ["`em` resolves against the container's own `font-size`."],
  },
  "row-gap": {
    examples: [
      {
        title: "Separate wrapped lines more than items",
        lang: "css",
        code: `.tags {
  display: flex;
  flex-wrap: wrap;
  column-gap: 4px;
  row-gap: 8px;
}`,
      },
    ],
  },
  "column-gap": {
    examples: [
      {
        title: "Separate a form's label column from its fields",
        lang: "css",
        code: `.form-grid {
  display: grid;
  grid-template-columns: 120px 1fr;
  column-gap: 16px;
  row-gap: 8px;
}`,
      },
    ],
  },
  "grid-template-columns": {
    examples: [
      {
        title: "Lay out a sidebar, content and inspector",
        lang: "css",
        code: `.ide {
  display: grid;
  grid-template-columns: 240px 1fr 300px;
  height: 100%;
}`,
      },
      {
        title: "Fit as many photo cards as the window allows",
        lang: "css",
        code: `.gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(100px, 1fr));
  gap: 8px;
}`,
        note: "In a 450px-wide gallery this makes four 112.5px columns and wraps the fifth card.",
      },
    ],
    edgeCases: [
      "`auto-fit` collapses the tracks no item uses, so two cards share the whole row; `auto-fill` keeps the empty tracks.",
      "A `fit-content()` track, or a `repeat(auto-fill, …)` next to a track that is not a fixed size, drops the whole list with an advisory.",
    ],
  },
  "grid-template-rows": {
    examples: [
      {
        title: "Give an app shell a header, a scrolling body and a footer",
        lang: "css",
        code: `.shell {
  display: grid;
  grid-template-rows: auto 1fr auto;
  height: 100vh;
}
.shell-body {
  min-height: 0;
  overflow-y: auto;
}`,
      },
    ],
  },
  "grid-column": {
    examples: [
      {
        title: "Stretch a section header across every column",
        lang: "css",
        code: `.settings-grid {
  display: grid;
  grid-template-columns: 160px 1fr 1fr;
}
.settings-grid .section-title {
  grid-column: 1 / -1;
}`,
      },
      {
        title: "Make a featured card two columns wide",
        lang: "css",
        code: `.card.featured {
  grid-column: span 2;
}`,
      },
    ],
    edgeCases: ["A negative line number counts from the end of the explicit grid, so `1 / -1` spans every column."],
  },
  "grid-row": {
    examples: [
      {
        title: "Make a portrait photo two rows tall",
        lang: "css",
        code: `.photo.portrait {
  grid-row: span 2;
}`,
      },
      {
        title: "Run a sidebar down the full height of a shell grid",
        lang: "css",
        code: `.shell {
  display: grid;
  grid-template-columns: 220px 1fr;
  grid-template-rows: 40px 1fr;
}
.shell .sidebar {
  grid-column: 1;
  grid-row: 1 / 3;
}`,
      },
    ],
  },
  "grid-area": {
    examples: [
      {
        title: "Place a preview pane by its four lines",
        lang: "css",
        code: `.mail {
  display: grid;
  grid-template-columns: 200px 320px 1fr;
  grid-template-rows: 40px 1fr;
}
.mail .preview {
  grid-area: 1 / 3 / 3 / 4;
}`,
        note: "The order is row-start / column-start / row-end / column-end.",
      },
    ],
  },
  "grid-template-areas": {
    examples: [
      {
        title: "Instead: place a header, sidebar and main area by line number",
        lang: "css",
        code: `.layout {
  display: grid;
  grid-template-columns: 220px 1fr;
  grid-template-rows: 48px 1fr;
}
.layout .header {
  grid-column: 1 / -1;
}
.layout .sidebar {
  grid-row: 2;
}
.layout .main {
  grid-row: 2;
  grid-column: 2;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  grid: {
    examples: [
      {
        title: "Instead: declare a grid with the longhands",
        lang: "css",
        code: `.board {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  grid-auto-rows: 120px;
  grid-auto-flow: row;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "grid-template": {
    examples: [
      {
        title: "Instead: set rows and columns separately",
        lang: "css",
        code: `.calendar {
  display: grid;
  grid-template-rows: 32px repeat(6, 1fr);
  grid-template-columns: repeat(7, 1fr);
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "grid-auto-flow": {
    examples: [
      {
        title: "Fill the holes a wide tile leaves in a gallery",
        lang: "css",
        code: `.gallery {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  grid-auto-rows: 120px;
  grid-auto-flow: row dense;
}
.gallery .wide {
  grid-column: span 2;
}`,
        note: "With `dense`, a later small tile moves up into a gap that a wide tile could not fill.",
      },
      {
        title: "Lay a timeline out left to right",
        lang: "css",
        code: `.timeline {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: 160px;
  overflow-x: auto;
}`,
      },
    ],
  },
  "grid-auto-rows": {
    examples: [
      {
        title: "Give rows a minimum height that grows with content",
        lang: "css",
        code: `.issue-board {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  grid-auto-rows: minmax(80px, auto);
  gap: 8px;
}`,
      },
    ],
    edgeCases: ["A `fit-content()` track drops the whole declaration with an advisory."],
  },
  "grid-auto-columns": {
    examples: [
      {
        title: "Size the columns a horizontal list creates",
        lang: "css",
        code: `.recent-files {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: 140px;
  gap: 8px;
}`,
      },
    ],
    edgeCases: ["A `fit-content()` track drops the whole declaration with an advisory."],
  },
  position: {
    examples: [
      {
        title: "Pin an unread badge to an avatar's corner",
        lang: "css",
        code: `.avatar {
  position: relative;
  width: 40px;
  height: 40px;
}
.avatar .badge {
  position: absolute;
  top: -4px;
  right: -4px;
}`,
      },
      {
        title: "Show a toast over the whole window",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ message: string }>();
</script>

<template>
  <Teleport to="body">
    <div class="toast">{{ message }}</div>
  </Teleport>
</template>

<style scoped>
.toast {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 100;
}
</style>`,
        note: "Teleported to the app root, the `fixed` box is placed against the window's edges.",
      },
      {
        title: "Keep each section's header on top while its section scrolls",
        lang: "vue",
        code: `<template>
  <div class="list">
    <section v-for="group in groups" :key="group.name">
      <h3 class="header">{{ group.name }}</h3>
      <div v-for="item in group.items" :key="item.id" class="row">{{ item.label }}</div>
    </section>
  </div>
</template>

<style scoped>
.list {
  height: 400px;
  overflow: auto;
}
.header {
  position: sticky;
  top: 0;
  background: white;
}
</style>`,
        note: "The header stays inside its `<section>`, so the next section's header pushes it out.",
      },
      {
        title: "Freeze the header row and the first column of a grid",
        lang: "css",
        code: `.sheet {
  overflow: auto;
}
.sheet .row {
  display: flex;
}
.sheet .head {
  position: sticky;
  top: 0;
  z-index: 2;
}
.sheet .row > :first-child {
  position: sticky;
  left: 0;
  z-index: 1;
}
.sheet .head > :first-child {
  z-index: 3;
}`,
        note: "The corner cell is sticky on both axes. The `z-index` values keep the header over the column and the corner over both.",
      },
    ],
    edgeCases: [
      "A `fixed` box whose parent is not the size of the window prints an advisory in `sk dev`.",
      "In a pop-out window, a `fixed` box is placed against the pop-out's edges, not the main window's, with a one-time advisory.",
      "`fixed` and `sticky` always open a stacking context, even with `z-index: auto`.",
      "A `sticky` box with `z-index: auto` paints over the in-flow content after it, as in a browser.",
      "The nearest ancestor with `overflow` `hidden`, `scroll` or `auto` is the scroll container a `sticky` box holds to. Inside an `overflow: hidden` card it holds to the card, and so does not follow the page's scroll.",
      "A percentage or `calc()` in `top` or `bottom` resolves against the scroll container's height, and in `left` or `right` against its width.",
      "With no inset set, a `sticky` box stays where the flow put it.",
      "When `top` and `bottom` both apply and conflict, `top` wins; likewise `left` over `right`.",
    ],
  },
  top: {
    examples: [
      {
        title: "Drop a menu just under its trigger",
        lang: "css",
        code: `.menu-anchor {
  position: relative;
}
.menu {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
}`,
      },
    ],
    edgeCases: ["`initial` and `unset` reset the offset to `auto`."],
  },
  right: {
    examples: [
      {
        title: "Place a close button in a dialog's corner",
        lang: "css",
        code: `.dialog {
  position: relative;
}
.dialog .close {
  position: absolute;
  top: 12px;
  right: 12px;
}`,
      },
    ],
    edgeCases: ["`initial` and `unset` reset the offset to `auto`."],
  },
  bottom: {
    examples: [
      {
        title: "Keep a jump-to-latest button above the composer",
        lang: "css",
        code: `.thread {
  position: relative;
}
.jump-to-latest {
  position: absolute;
  bottom: 72px;
  right: 16px;
}`,
      },
    ],
    edgeCases: ["`initial` and `unset` reset the offset to `auto`."],
  },
  left: {
    examples: [
      {
        title: "Center a tooltip over its trigger",
        lang: "css",
        code: `.tooltip {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 50%;
  transform: translateX(-50%);
}`,
      },
    ],
    edgeCases: ["`initial` and `unset` reset the offset to `auto`."],
  },
  inset: {
    examples: [
      {
        title: "Cover a window with a modal scrim",
        lang: "css",
        code: `.scrim {
  position: fixed;
  inset: 0;
  background: #00000080;
  z-index: 50;
}`,
      },
      {
        title: "Stretch a drawer along one edge",
        lang: "css",
        code: `.drawer {
  position: absolute;
  inset: 0 0 0 auto;
  width: 320px;
}`,
      },
    ],
    edgeCases: [
      "`initial` and `unset` reset all four offsets to `auto`; a later longhand in the same block still applies on top.",
    ],
  },
  "z-index": {
    examples: [
      {
        title: "Raise an open dropdown over the content below it",
        lang: "css",
        code: `.select {
  position: relative;
}
.select-options {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  z-index: 10;
}`,
      },
      {
        title: "Raise the selected card in an overlapping row",
        lang: "css",
        code: `.cards {
  display: flex;
}
.card {
  margin-left: -12px;
}
.card.selected {
  z-index: 1;
}`,
        note: "A flex or grid item takes `z-index` without a `position`.",
      },
    ],
    edgeCases: [
      "On a non-positioned box in a block container, `z-index` is ignored.",
      "`z-index: 0` opens a stacking context; `auto` does not, so a child's own `z-index` then stacks against the rest of the page.",
      "A negative `z-index` paints over its stacking context's background and under its in-flow content.",
      "`opacity` below `1` opens a stacking context, so a `z-index: 100` child stays under a later sibling with `z-index: 10`.",
    ],
  },
  float: {
    examples: [
      {
        title: "Instead: put an image beside a message with flex",
        lang: "css",
        code: `.message {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}
.message img {
  width: 48px;
  flex-shrink: 0;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  clear: {
    examples: [
      {
        title: "Instead: start a footer on its own row with flex",
        lang: "css",
        code: `.article {
  display: flex;
  flex-direction: column;
  gap: 16px;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  columns: {
    examples: [
      {
        title: "Instead: lay a link list out in three columns with grid",
        lang: "css",
        code: `.shortcuts {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  column-gap: 24px;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "column-count": {
    examples: [
      {
        title: "Instead: split a list into two grid columns",
        lang: "css",
        code: `.contacts {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "column-width": {
    examples: [
      {
        title: "Instead: make as many 200px columns as fit",
        lang: "css",
        code: `.contacts {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "column-rule": {
    examples: [
      {
        title: "Instead: draw a divider between grid columns",
        lang: "css",
        code: `.split {
  display: grid;
  grid-template-columns: 1fr 1fr;
}
.split > :nth-child(2) {
  border-left: 1px solid #ffffff26;
  padding-left: 16px;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "border-collapse": { edgeCases: [advisory] },
  "border-spacing": { edgeCases: [advisory] },
  "table-layout": { edgeCases: [advisory] },
  "anchor-name": { examples: [anchorWorkaround], edgeCases: [advisory] },
  "position-anchor": { examples: [anchorWorkaround], edgeCases: [advisory] },
  "function-anchor": { examples: [anchorWorkaround] },
};
