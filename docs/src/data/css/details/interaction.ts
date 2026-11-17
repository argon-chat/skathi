import type { Details } from "../../details";

export const details: Details = {
  cursor: {
    examples: [
      {
        title: "Show grab and grabbing on a draggable card",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const dragging = ref(false);
</script>

<template>
  <div class="board-card" :class="{ dragging }" @mousedown="dragging = true" @mouseup="dragging = false">
    Design review
  </div>
</template>

<style scoped>
.board-card {
  cursor: grab;
}
.board-card.dragging {
  cursor: grabbing;
}
</style>`,
      },
      {
        title: "Mark resize handles and disabled actions",
        lang: "css",
        code: `.column-resizer {
  width: 6px;
  cursor: col-resize;
}
.row-resizer {
  height: 6px;
  cursor: row-resize;
}
.toolbar button:disabled {
  cursor: not-allowed;
}`,
      },
    ],
    edgeCases: [
      "`move` and `all-scroll` share the four-way arrow on Windows, and `no-drop` shows `not-allowed` there.",
      "The diagonal resize cursors come in two shapes: `nwse-resize`, `nw-resize` and `se-resize` show one, and `nesw-resize`, `ne-resize` and `sw-resize` the other.",
      "`getComputedStyle(el).cursor` reads the keyword, inherited from the nearest ancestor that sets one.",
    ],
  },
  "pointer-events": {
    examples: [
      {
        title: "Let clicks pass through a drop hint",
        lang: "css",
        code: `.drop-zone {
  position: relative;
}
.drop-zone .drop-hint {
  position: absolute;
  inset: 0;
  background-color: rgb(26 115 232 / 0.08);
  pointer-events: none;
}`,
        note: "The hint covers the zone but every press lands on the content under it.",
      },
      {
        title: "Make a layer click-through except for its card",
        lang: "css",
        code: `.notification-layer {
  position: absolute;
  inset: 0;
  pointer-events: none;
}
.notification-layer .card {
  pointer-events: auto;
}`,
      },
    ],
    edgeCases: [
      "`pointer-events` inherits: `auto` on a descendant of a `none` element makes that descendant a target again.",
      "A `none` element still matches `:hover` and gets `mouseenter` while one of its `pointer-events: auto` descendants is under the pointer.",
      "`document.elementFromPoint()` and `document.elementsFromPoint()` skip `pointer-events: none` elements.",
      "An element with `pointer-events: none` never matches `:active`, and a text field with it takes no press at all.",
      "Values other than `auto` and `none`, such as SVG's `visiblePainted`, are dropped with an advisory.",
    ],
  },
  "user-select": {
    examples: [
      {
        title: "Keep toolbar labels out of text selection",
        lang: "css",
        code: `.toolbar {
  user-select: none;
}
.toolbar .path-label {
  user-select: text;
}`,
        note: "Everything in the toolbar ignores drag-selection except the path label, which opts back in.",
      },
      {
        title: "Select an API key as one unit",
        lang: "css",
        code: `.api-key {
  font-family: monospace;
  user-select: all;
}`,
      },
    ],
    edgeCases: [
      "`auto` takes `none` or `all` from the parent and is `text` otherwise, so `text` on a descendant makes it selectable inside a `none` parent.",
      "A selection that touches a `user-select: all` element extends to cover all of it.",
      "Copying a selection that spans a `user-select: none` element leaves that element's text out.",
      "A press on `user-select: none` text starts no selection and keeps the current one.",
      "`-webkit-user-select` is read as `user-select`; `-moz-user-select` is not.",
    ],
  },
  "touch-action": {
    examples: [
      {
        title: "Reserve touch gestures for a drawing canvas",
        lang: "css",
        code: `.sketch-area {
  width: 100%;
  height: 480px;
  touch-action: none;
}`,
      },
    ],
    edgeCases: ['It is accepted without an advisory, and `CSS.supports("touch-action", "none")` returns `true`.'],
  },
  appearance: {
    examples: [
      {
        title: "Draw a filter select without the native chevron",
        lang: "css",
        code: `select.filter {
  appearance: none;
  padding: 4px 8px;
  border: 1px solid #dadce0;
  border-radius: 6px;
  background-color: #ffffff;
}`,
      },
      {
        title: "Restyle a volume slider",
        lang: "css",
        code: `input[type="range"].volume {
  appearance: none;
  accent-color: #e8710a;
}
input[type="range"].volume::-webkit-slider-runnable-track {
  height: 6px;
  border-radius: 3px;
  background-color: #3c4043;
}
input[type="range"].volume::-webkit-slider-thumb {
  width: 14px;
  height: 14px;
}`,
      },
    ],
    edgeCases: [
      "On `<select>`, `none` removes the chevron and the internal padding drawn around the selected label.",
      'On `<input type="range">`, `none` removes the grey track and its filled part; the thumb stays, in `accent-color`.',
      "On a checkbox or radio button, `none` removes the box, the check mark and the dot: the input draws only its own background, border and `::before`.",
      "Any value other than `none`, such as `auto` or `menulist-button`, keeps the native look.",
      "`-webkit-appearance` is read as `appearance`; `-moz-appearance` is not.",
    ],
  },
  "accent-color": {
    examples: [
      {
        title: "Brand a volume slider",
        lang: "css",
        code: `input[type="range"].volume {
  width: 160px;
  accent-color: #e8710a;
}`,
      },
      {
        title: "Take the accent from a theme token",
        lang: "css",
        code: `.settings {
  --brand: #7c4dff;
}
.settings input[type="range"] {
  accent-color: var(--brand);
}`,
      },
    ],
    edgeCases: [
      'On `<input type="range">` it colours the thumb and the filled part of the track. `auto` uses `#0075ff`.',
      "On a checked checkbox or radio button it colours the box and the dot. They take it from the nearest ancestor that sets it, so one rule on a form colours all of them.",
      "A `background` in a `::-webkit-slider-thumb` rule overrides the accent colour on the thumb, and a styled `::-webkit-slider-runnable-track` drops the filled part.",
    ],
  },
  "caret-color": {
    edgeCases: [
      "A `caret-color` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "caret-shape": {
    edgeCases: [
      "A `caret-shape` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  resize: {
    examples: [
      {
        title: "Instead: resize a sidebar with a drag handle",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const width = ref(260);
let startX = 0;
let startWidth = 0;

function onDown(e: PointerEvent) {
  startX = e.clientX;
  startWidth = width.value;
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
}

function onMove(e: PointerEvent) {
  if (!(e.currentTarget as HTMLElement).hasPointerCapture(e.pointerId)) return;
  width.value = Math.max(180, startWidth + e.clientX - startX);
}
</script>

<template>
  <aside class="sidebar" :style="{ width: width + 'px' }">
    <div class="resize-handle" @pointerdown="onDown" @pointermove="onMove" />
  </aside>
</template>`,
        note: "Pointer capture keeps the moves coming to the handle while the pointer is outside it.",
      },
    ],
    edgeCases: [
      "`resize: none` is accepted; for any other value, such as `both`, `CSS.supports()` returns `false` and no resize corner is drawn.",
    ],
  },
  "field-sizing": {
    edgeCases: [
      "A `field-sizing` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  interactivity: {
    edgeCases: [
      "An `interactivity` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "app-region": {
    examples: [
      {
        title: "Drag the window by a custom titlebar",
        lang: "css",
        code: `.titlebar {
  display: flex;
  align-items: center;
  height: 36px;
  app-region: drag;
  user-select: none;
}
.titlebar .search,
.titlebar .account-menu {
  app-region: no-drag;
}`,
        note: "The search field and the menu are carved out of the drag region, so they take clicks and focus.",
      },
    ],
    edgeCases: [
      "It inherits: every descendant drags the window until one sets `app-region: no-drag` (or `none`).",
      "Unlike a browser or Electron, Skathi still delivers clicks inside a drag region. `sk dev` advises `no-drag` on any element there with a click or mouse listener, so the app behaves the same in a browser.",
      "An element with `--window-control` is never part of the drag region.",
      'With `titleBarStyle: "default"` the OS draws the caption, and `app-region` does nothing, with an advisory.',
    ],
  },
  "window-control": {
    examples: [
      {
        title: "Wire custom titlebar buttons to the window",
        lang: "css",
        code: `.titlebar .ctl {
  width: 46px;
  height: 36px;
}
.titlebar .min {
  --window-control: minimize;
}
.titlebar .max {
  --window-control: maximize;
}
.titlebar .close {
  --window-control: close;
}
.titlebar .close:hover {
  background-color: #c42b1c;
}`,
        note: "The OS or the host performs the action, so the buttons need no click handlers; on Windows 11, hovering the `maximize` button opens the snap layouts flyout.",
      },
    ],
    edgeCases: [
      "Do not also attach a click handler that does the same: the window would act twice. `sk dev` warns about it.",
      "`el.click()` on the element or on one of its descendants performs the action, as a real press does.",
      "`--window-control: none` removes an earlier role but leaves the element in an ancestor's drag region; add `app-region: no-drag` to take it out.",
      "Declared in a state rule such as `:hover`, it applies whatever the state, with an advisory.",
      'With `titleBarStyle: "default"` the OS owns the caption buttons, and the property does nothing, with an advisory.',
    ],
  },
};
