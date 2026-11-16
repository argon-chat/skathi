import type { Details } from "../../details";

export const details: Details = {
  "elements-and-attributes": {
    examples: [
      {
        title: "Handle clicks for a whole list with one listener",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ files: { id: string; name: string }[] }>();
const emit = defineEmits<{ open: [id: string] }>();

function onClick(e: MouseEvent) {
  const row = (e.target as HTMLElement).closest<HTMLElement>("[data-file-id]");
  if (row) emit("open", row.dataset.fileId!);
}
</script>

<template>
  <ul class="file-list" @click="onClick">
    <li v-for="file in files" :key="file.id" :data-file-id="file.id">{{ file.name }}</li>
  </ul>
</template>`,
      },
      {
        title: "Toggle an expanded state on a disclosure button",
        lang: "ts",
        code: `export function toggleSection(button: HTMLButtonElement) {
  const open = button.getAttribute("aria-expanded") !== "true";
  button.setAttribute("aria-expanded", String(open));
  button.classList.toggle("is-open", open);
  const panel = document.getElementById(button.getAttribute("aria-controls")!);
  panel?.toggleAttribute("hidden", !open);
}`,
      },
    ],
    edgeCases: [
      "`querySelector`, `closest` and `matches` with a selector the engine cannot run, such as `:has()` or a pseudo-element, match nothing and log an advisory instead of throwing.",
      "`getBoundingClientRect()` is where the box was drawn, including `transform`s on it and its ancestors; `offsetWidth` and `clientWidth` are the layout box before transforms.",
      "For an element that was already painted, these reads answer from the last drawn frame and do not force a layout: right after a change in the same handler they still give the old size. Read in `requestAnimationFrame` or a `ResizeObserver` callback to see the new one.",
      "An element mounted during a mouse or keyboard event has no size yet: its rect reads `0` and `sk dev` logs an advisory. Measure it in `requestAnimationFrame` or a `ResizeObserver` callback.",
    ],
  },
  "get-computed-style": {
    examples: [
      {
        title: "Draw a canvas chart in the theme's colors",
        lang: "ts",
        code: `export function themeColors(chart: HTMLCanvasElement) {
  const style = getComputedStyle(chart);
  return {
    line: style.getPropertyValue("--chart-line").trim(),
    grid: style.getPropertyValue("--chart-grid").trim(),
    text: style.color,
  };
}`,
        note: "Custom properties resolve through the cascade, so a theme class on an ancestor changes the result.",
      },
      {
        title: "Wait for a closing transition before unmounting",
        lang: "ts",
        code: `export function closeDuration(drawer: HTMLElement): number {
  const value = getComputedStyle(drawer).transitionDuration.split(",")[0]!.trim();
  return value.endsWith("ms") ? parseFloat(value) : parseFloat(value) * 1000;
}`,
      },
    ],
    edgeCases: [
      "`transform` reads the function list from the cascade, such as `rotate(45deg)`, not the `matrix()` a browser returns.",
      "`width` and `height` are the laid-out size in `px`, also when the CSS value is `auto`.",
      "The object is live: each read returns the current value. Writing to it throws a `NoModificationAllowedError`.",
      "An argument that is not an element reads the root element's style instead of throwing.",
    ],
  },
  "element-from-point": {
    examples: [
      {
        title: "Find the folder under a dragged file",
        lang: "ts",
        code: `export function folderAt(e: PointerEvent): string | null {
  const hit = document.elementFromPoint(e.clientX, e.clientY);
  const folder = hit?.closest<HTMLElement>(".folder");
  return folder?.dataset.path ?? null;
}`,
        note: "Give the dragged ghost `pointer-events: none` so the lookup sees what is under it.",
      },
      {
        title: "List every layer under a click for a debug overlay",
        lang: "ts",
        code: `addEventListener("click", (e: MouseEvent) => {
  if (!e.altKey) return;
  const stack = document.elementsFromPoint(e.clientX, e.clientY);
  console.table(stack.map((el) => ({ tag: el.tagName, id: el.id, class: el.className })));
});`,
      },
    ],
    edgeCases: [
      "Elements with `pointer-events: none` are skipped, by `elementFromPoint` and `elementsFromPoint` alike.",
      "A point outside the window returns `null`; `elementsFromPoint` returns an empty array.",
      "`elementsFromPoint` lists every box at the point, topmost first, down to the root.",
    ],
  },
  "scroll-into-view": {
    examples: [
      {
        title: "Keep the keyboard-selected row visible",
        lang: "vue",
        code: `<script setup lang="ts">
import { nextTick, ref } from "vue";

const props = defineProps<{ items: string[] }>();
const selected = ref(0);

async function move(step: number) {
  selected.value = Math.max(0, Math.min(props.items.length - 1, selected.value + step));
  await nextTick();
  document.querySelector(".row.selected")?.scrollIntoView({ block: "nearest" });
}
</script>

<template>
  <div class="list" tabindex="0" @keydown.down.prevent="move(1)" @keydown.up.prevent="move(-1)">
    <div v-for="(item, i) in items" :key="item" class="row" :class="{ selected: i === selected }">{{ item }}</div>
  </div>
</template>`,
        note: '`block: "nearest"` leaves a row that is already visible where it is.',
      },
      {
        title: "Jump to a search match and center it",
        lang: "ts",
        code: `export function revealMatch(match: HTMLElement) {
  match.classList.add("highlight");
  match.scrollIntoView({ behavior: "smooth", block: "center" });
}`,
      },
    ],
    edgeCases: [
      "A row far below the fold that has never been painted still scrolls into place, because the whole document is laid out.",
      "`focus()` never scrolls, so call `scrollIntoView` after focusing an off-screen element.",
    ],
  },
  event: {
    examples: [
      {
        title: "Let a listener veto closing an editor",
        lang: "ts",
        code: `export function requestClose(editor: HTMLElement): boolean {
  const allowed = editor.dispatchEvent(new Event("before-close", { cancelable: true }));
  if (allowed) editor.remove();
  return allowed;
}

export function guardUnsaved(editor: HTMLElement, isDirty: () => boolean) {
  editor.addEventListener("before-close", (e) => {
    if (isDirty()) e.preventDefault();
  });
}`,
        note: "`dispatchEvent` returns `false` when a listener called `preventDefault()` on a cancelable event.",
      },
    ],
    edgeCases: [
      'On an element, `addEventListener` with an all-lowercase name the host never delivers, such as `"beforeclose"`, registers nothing and logs an advisory. A name with a dash, a dot or a capital, such as `"before-close"`, is registered.',
      "A listener that throws does not stop the dispatch; the remaining listeners run and the error is rethrown from a microtask afterwards.",
      "Each input event from the host gets its own microtask checkpoint, so Vue has re-rendered before the next event's listeners run.",
      "`wheel`, `mousewheel`, `touchstart` and `touchmove` listeners on `window`, `document` or `<body>` are passive unless `passive: false` is given, so their `preventDefault()` does nothing.",
    ],
  },
  "custom-event": {
    examples: [
      {
        title: "Announce a saved document to the rest of the window",
        lang: "ts",
        code: `type SavedDoc = { id: string; title: string };

export const appEvents = new EventTarget();

export function announceSaved(doc: SavedDoc) {
  appEvents.dispatchEvent(new CustomEvent<SavedDoc>("doc-saved", { detail: doc }));
}

appEvents.addEventListener("doc-saved", (e) => {
  console.log("saved", (e as CustomEvent<SavedDoc>).detail.title);
});`,
      },
      {
        title: "Bubble a custom event out of a native element",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const emit = defineEmits<{ pick: [color: string] }>();
const palette = ref<HTMLElement | null>(null);

onMounted(() => palette.value!.addEventListener("swatch-pick", (e) => emit("pick", (e as CustomEvent<string>).detail)));
const pick = (color: string, e: MouseEvent) =>
  e.currentTarget!.dispatchEvent(new CustomEvent("swatch-pick", { detail: color, bubbles: true }));
</script>

<template>
  <div ref="palette" class="palette">
    <button v-for="c in ['#e11d48', '#2563eb']" :key="c" :style="{ background: c }" @click="pick(c, $event)" />
  </div>
</template>`,
        note: "Without `bubbles: true` only listeners on the button itself would hear it.",
      },
    ],
    edgeCases: [
      "A template listener such as `@swatch-pick` on a native element is not registered for an event the host does not deliver; `sk dev` says the handler will never fire. Add it with `addEventListener`, or use a component `emit`.",
    ],
  },
  "event-target": {
    examples: [
      {
        title: "Give a download queue its own events",
        lang: "ts",
        code: `export class DownloadQueue extends EventTarget {
  private active = 0;

  async add(url: string) {
    this.active++;
    this.dispatchEvent(new CustomEvent("change", { detail: this.active }));
    try {
      await fetch(url);
    } finally {
      this.active--;
      this.dispatchEvent(new CustomEvent("change", { detail: this.active }));
    }
  }
}`,
      },
      {
        title: "Remove a page's document listeners at once with an AbortSignal",
        lang: "ts",
        code: `export function holdSpaceToPan(setPanning: (on: boolean) => void) {
  const controller = new AbortController();
  const { signal } = controller;
  document.addEventListener("keydown", (e) => e.key === " " && setPanning(true), { signal });
  document.addEventListener("keyup", (e) => e.key === " " && setPanning(false), { signal });
  return () => controller.abort();
}`,
      },
    ],
    edgeCases: [
      "A listener added to `window` or `document` with an already aborted `signal` is never added.",
      "A listener that throws is reported to the host's error log and the other listeners still run.",
    ],
  },
  "ui-event": {
    examples: [
      {
        title: "Select a word on double click and a line on triple click",
        lang: "vue",
        code: `<script setup lang="ts">
function onClick(e: MouseEvent) {
  if (e.detail === 2) selectWordAt(e.clientX, e.clientY);
  else if (e.detail === 3) selectLineAt(e.clientY);
}
declare function selectWordAt(x: number, y: number): void;
declare function selectLineAt(y: number): void;
</script>

<template>
  <canvas class="code-view" @click="onClick" />
</template>`,
        note: "`detail` on `click`, `mousedown` and `mouseup` is the click count.",
      },
    ],
    edgeCases: [
      "`detail` is the click count on `click`, `auxclick`, `mousedown` and `mouseup`, `2` on `dblclick`, and `0` on every other event from the host.",
    ],
  },
  "mouse-event": {
    examples: [
      {
        title: "Open a context menu at the pointer",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const menu = ref<{ x: number; y: number } | null>(null);
const open = (e: MouseEvent) => (menu.value = { x: e.clientX, y: e.clientY });
</script>

<template>
  <div class="canvas-area" @contextmenu.prevent="open" @click="menu = null">
    <ul v-if="menu" class="menu" :style="{ left: menu.x + 'px', top: menu.y + 'px' }">
      <li>Paste</li>
      <li>Select all</li>
    </ul>
  </div>
</template>

<style scoped>
.menu { position: fixed; }
</style>`,
      },
      {
        title: "Close a tab with a middle click",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ title: string }>();
const emit = defineEmits<{ close: [] }>();
const onAuxClick = (e: MouseEvent) => e.button === 1 && emit("close");
</script>

<template>
  <div class="tab" @auxclick="onAuxClick">{{ title }}</div>
</template>`,
      },
    ],
    edgeCases: [
      "On events from the host, `offsetX` and `offsetY` are `0`, and `pageX` and `pageY` equal `clientX` and `clientY`.",
      "`buttons` is already `0` on `mouseup`, `click`, `dblclick`, `auxclick` and `contextmenu`.",
      "At most one `mousemove` is delivered per frame; that frame's `mouseleave` and `mouseenter` come before it.",
      "When the pointer moves from one element to another, `mouseleave` and `mouseenter` can come in either order.",
    ],
  },
  "pointer-event": {
    examples: [
      {
        title: "Drag a splitter even when the pointer leaves it",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const sidebarWidth = ref(240);
const dragging = ref(false);

function start(e: PointerEvent) {
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  dragging.value = true;
}
const move = (e: PointerEvent) => dragging.value && (sidebarWidth.value = Math.max(160, e.clientX));
</script>

<template>
  <aside :style="{ width: sidebarWidth + 'px' }" />
  <div class="splitter" @pointerdown="start" @pointermove="move" @pointerup="dragging = false" />
</template>`,
        note: "With capture, moves outside the handle and the release still reach it; the release ends the capture.",
      },
    ],
    edgeCases: [
      "`setPointerCapture` with an id other than `1` throws a `NotFoundError`; called while no button is down, it does nothing.",
      "Removing the element that holds the capture releases it and fires `lostpointercapture`.",
    ],
  },
  "keyboard-event": {
    examples: [
      {
        title: "Save with Ctrl+S",
        lang: "ts",
        code: `export function bindSaveShortcut(save: () => void) {
  const onKey = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      save();
    }
  };
  document.addEventListener("keydown", onKey);
  return () => document.removeEventListener("keydown", onKey);
}`,
      },
      {
        title: "Close a dialog with Escape",
        lang: "vue",
        code: `<script setup lang="ts">
const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <div class="dialog" tabindex="-1" @keydown.esc.stop="emit('close')">
    <slot />
  </div>
</template>`,
      },
    ],
    edgeCases: [
      "Enter on a `<button>` clicks it on the press, before `keyup`; Space clicks it on the release, after the `keyup` listeners. Cancelling that `keydown` or `keyup` clicks nothing.",
      'The Menu key arrives as `key: "ContextMenu"`, and its `contextmenu` event follows the `keyup`.',
      "A `keydown` handler cannot force layout: after a DOM change in the same handler, sizes and `scrollHeight` are a frame old. Read them in `requestAnimationFrame`.",
    ],
  },
  "wheel-event": {
    examples: [
      {
        title: "Zoom an image with Ctrl+wheel",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const zoom = ref(1);

function onWheel(e: WheelEvent) {
  if (!e.ctrlKey) return;
  e.preventDefault();
  zoom.value = Math.min(8, Math.max(0.25, zoom.value * (e.deltaY < 0 ? 1.1 : 1 / 1.1)));
}
</script>

<template>
  <div class="viewer" @wheel="onWheel">
    <img src="/photo.jpg" :style="{ transform: 'scale(' + zoom + ')' }" />
  </div>
</template>`,
        note: "A listener on an element is not passive, so `preventDefault()` stops the scroll.",
      },
    ],
    edgeCases: [
      "`deltaMode` is always `0`: the deltas are in pixels.",
      "A cancelled `wheel` scrolls nothing, neither the element it targets nor a scroll container around it.",
      "The `wheel` event fires before the `scroll` event it causes.",
    ],
  },
  "focus-event": {
    examples: [
      {
        title: "Close a popover when focus leaves it",
        lang: "vue",
        code: `<script setup lang="ts">
const emit = defineEmits<{ close: [] }>();

function onFocusOut(e: FocusEvent) {
  const popover = e.currentTarget as HTMLElement;
  if (!popover.contains(e.relatedTarget as Node | null)) emit("close");
}
</script>

<template>
  <div class="popover" @focusout="onFocusOut">
    <slot />
  </div>
</template>`,
        note: "`focusout` bubbles, so one listener covers every field inside the popover.",
      },
    ],
    edgeCases: [
      "One focus move fires `blur` and `focusout` on the old element, then `focus` and `focusin` on the new one.",
      "During `blur` and `focusout`, `document.activeElement` is `<body>`.",
      "A `focus()` call made while another move's events run abandons that move: its target never gets `focus`.",
    ],
  },
  "input-event": {
    examples: [
      {
        title: "Accept only digits in a code field",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const code = ref("");

function onBeforeInput(e: InputEvent) {
  if (e.data && /\\D/.test(e.data)) e.preventDefault();
}
</script>

<template>
  <input v-model="code" maxlength="6" @beforeinput="onBeforeInput" />
</template>`,
        note: "Cancelling `beforeinput` stops the edit, so no `input` event follows.",
      },
      {
        title: "Skip autosave for undo and redo",
        lang: "ts",
        code: `export function onEditorInput(e: InputEvent, autosave: () => void) {
  if (e.inputType === "historyUndo" || e.inputType === "historyRedo") return;
  autosave();
}`,
      },
    ],
    edgeCases: [
      'Typing fires `keydown`, `beforeinput`, the edit, then `input`, each with `inputType: "insertText"` and the character in `data`.',
      "Enter in a `<textarea>` is `insertLineBreak` with `data` `null`; Backspace, Ctrl+Backspace and Delete are deletion types with no `data`, and with nothing to delete no event fires.",
      "Paste and cut fire after their clipboard event, as `insertFromPaste` with the text and `deleteByCut`.",
      "During an IME composition, `beforeinput` cannot be cancelled, and `v-model` leaves the model alone until the composition ends.",
    ],
  },
  "drag-event": {
    examples: [
      {
        title: "Reorder a list by dragging",
        lang: "vue",
        code: `<script setup lang="ts">
const tasks = defineModel<string[]>({ required: true });

const onDragStart = (e: DragEvent, from: number) => e.dataTransfer!.setData("text/plain", String(from));

function onDrop(e: DragEvent, to: number) {
  const from = Number(e.dataTransfer!.getData("text/plain"));
  const next = [...tasks.value];
  next.splice(to, 0, next.splice(from, 1)[0]!);
  tasks.value = next;
}
</script>

<template>
  <ul>
    <li v-for="(task, i) in tasks" :key="task" draggable="true"
      @dragstart="onDragStart($event, i)" @dragover.prevent @drop="onDrop($event, i)">{{ task }}</li>
  </ul>
</template>`,
        note: "Cancelling `dragover` is what makes an element a drop target.",
      },
    ],
    edgeCases: [
      'A drag starts after a left press on a `draggable="true"` element moves more than 4 px; the release fires `drop`, then `dragend`, and no `click`.',
      "Escape cancels the drag, and a drag ends at the window's edge.",
      "`setDragImage` does nothing: no drag image is drawn.",
      "Files dragged in from another app fire no drag events.",
    ],
  },
  "touch-event": {
    examples: [
      {
        title: "Test a swipe handler with synthesized touches",
        lang: "ts",
        code: `export function simulateSwipe(card: HTMLElement, fromX: number, toX: number) {
  const touch = (x: number) => new Touch({ identifier: 1, target: card, clientX: x, clientY: 100 });
  card.dispatchEvent(new TouchEvent("touchstart", { touches: [touch(fromX)], bubbles: true }));
  card.dispatchEvent(new TouchEvent("touchmove", { touches: [touch(toX)], bubbles: true }));
  card.dispatchEvent(new TouchEvent("touchend", { changedTouches: [touch(toX)], bubbles: true }));
}`,
        note: "Handle real input with pointer events, which the host does deliver.",
      },
    ],
    edgeCases: [
      "`new Touch()` without a `target` throws a `TypeError`.",
      "A `touches` entry that is not a `Touch` throws a `TypeError` from the `TouchEvent` constructor.",
    ],
  },
  "resize-observer": {
    examples: [
      {
        title: "Keep a canvas the size of its panel",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

const panel = ref<HTMLElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const observer = new ResizeObserver(([entry]) => {
  const { inlineSize, blockSize } = entry!.borderBoxSize[0]!;
  canvas.value!.width = Math.round(inlineSize * devicePixelRatio);
  canvas.value!.height = Math.round(blockSize * devicePixelRatio);
});

onMounted(() => observer.observe(panel.value!));
onBeforeUnmount(() => observer.disconnect());
</script>

<template>
  <div ref="panel" class="chart-panel"><canvas ref="canvas" /></div>
</template>`,
        note: "The callback sees the size this frame lays out, and the new canvas size paints in the same frame.",
      },
    ],
    edgeCases: [
      "When a callback resizes an observed element at or above the tree depth of the elements just reported, that change is delivered next frame and the loop error `ResizeObserver loop completed with undelivered notifications.` is dispatched at the window.",
    ],
  },
  "intersection-observer": {
    examples: [
      {
        title: "Load the next page when the list reaches its end",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

const emit = defineEmits<{ more: [] }>();
const scroller = ref<HTMLElement | null>(null);
const sentinel = ref<HTMLElement | null>(null);
let observer: IntersectionObserver | null = null;

onMounted(() => {
  observer = new IntersectionObserver(([e]) => e!.isIntersecting && emit("more"), {
    root: scroller.value,
    rootMargin: "200px",
  });
  observer.observe(sentinel.value!);
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div ref="scroller" class="feed"><slot /><div ref="sentinel" /></div>
</template>`,
        note: "The scroll container is the `root`, so the sentinel counts as visible only inside it.",
      },
    ],
    edgeCases: [
      "The first update after `observe()` always reports, intersecting or not.",
      "A `threshold` outside `0` to `1` throws a `RangeError`; a `root` that is not an element or a document throws a `TypeError`.",
      "Rectangles are where boxes were drawn, transforms included, as `getBoundingClientRect()` reports.",
    ],
  },
  "mutation-observer": {
    examples: [
      {
        title: "Follow a log view as lines are added",
        lang: "ts",
        code: `export function followLog(log: HTMLElement) {
  const observer = new MutationObserver(() => {
    requestAnimationFrame(() => (log.scrollTop = log.scrollHeight));
  });
  observer.observe(log, { childList: true });
  return () => observer.disconnect();
}`,
      },
      {
        title: "React when a theme class changes on the root",
        lang: "ts",
        code: `export function onThemeChange(apply: (dark: boolean) => void) {
  const root = document.documentElement;
  new MutationObserver(() => apply(root.classList.contains("dark"))).observe(root, {
    attributes: true,
    attributeFilter: ["class"],
  });
}`,
      },
    ],
    edgeCases: [
      "`observe(document, ...)` observes the root element.",
      "Observing something that is not a rendered element observes nothing and logs an advisory.",
      "`attributeFilter` alone turns on `attributes`.",
      "`previousSibling` and `nextSibling` on records are always `null`.",
    ],
  },
  selection: {
    examples: [
      {
        title: "Quote the selected text into a reply",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

const selected = ref("");
const reply = ref("");
const update = () => (selected.value = getSelection()?.toString().trim() ?? "");

onMounted(() => document.addEventListener("selectionchange", update));
onBeforeUnmount(() => document.removeEventListener("selectionchange", update));
const quote = () => (reply.value = "> " + selected.value + "\\n\\n" + reply.value);
</script>

<template>
  <button :disabled="!selected" @click="quote">Quote</button>
  <textarea v-model="reply" />
</template>`,
      },
    ],
    edgeCases: [
      "Ctrl+C copies the selection; the `copy` event sees it.",
      "`user-select: none` starts no selection, and a press there keeps the current one.",
      "`modify()` does nothing and logs an advisory; move the boundaries with `collapse()` and `extend()`.",
      "`addRange()` is ignored while the selection already has a range.",
    ],
  },
  range: {
    examples: [
      {
        title: "Select a whole message from script",
        lang: "ts",
        code: `export function selectMessage(message: HTMLElement) {
  const range = document.createRange();
  range.selectNodeContents(message);
  const selection = getSelection()!;
  selection.removeAllRanges();
  selection.addRange(range);
}`,
        note: "A selection set from script is painted and reads back like one made with the mouse.",
      },
    ],
  },
  "match-media": {
    examples: [
      {
        title: "Follow the system's dark mode in a canvas",
        lang: "ts",
        code: `const dark = matchMedia("(prefers-color-scheme: dark)");

export function watchScheme(redraw: (dark: boolean) => void) {
  redraw(dark.matches);
  dark.addEventListener("change", (e) => redraw((e as MediaQueryListEvent).matches));
}`,
      },
      {
        title: "Skip a decorative animation for reduced motion",
        lang: "ts",
        code: `export function celebrate(badge: HTMLElement) {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  badge.animate([{ transform: "scale(1)" }, { transform: "scale(1.2)" }, { transform: "scale(1)" }], 300);
}`,
      },
    ],
    edgeCases: [
      "A query that does not parse matches `false` and logs an advisory once per query.",
      "`onchange` runs before the listeners added with `addEventListener`.",
    ],
  },
  "fullscreen-api": {
    examples: [
      {
        title: "Toggle a viewer between windowed and full screen",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

const viewer = ref<HTMLElement | null>(null);
const isFull = ref(false);
const sync = () => (isFull.value = document.fullscreenElement === viewer.value);

const toggle = () => (isFull.value ? document.exitFullscreen() : viewer.value!.requestFullscreen());
onMounted(() => document.addEventListener("fullscreenchange", sync));
onBeforeUnmount(() => document.removeEventListener("fullscreenchange", sync));
</script>

<template>
  <div ref="viewer" class="viewer">
    <button class="corner" @click="toggle">{{ isFull ? "Exit full screen" : "Full screen" }}</button>
  </div>
</template>`,
      },
    ],
    edgeCases: [
      "The promise resolves once the window has resized and laid out, a frame or two after `fullscreenchange`.",
      "Removing the full-screen element from the tree exits full screen.",
      "`requestFullscreen()` on a detached element rejects with a `TypeError`. No user gesture is needed.",
      "If the window was already full screen before the request, exiting keeps it full screen.",
      "`document.fullscreenEnabled` is always `true`.",
    ],
  },
  "web-animations": {
    examples: [
      {
        title: "Shake a field that failed validation",
        lang: "ts",
        code: `export function shake(field: HTMLElement) {
  field.animate(
    [{ transform: "translateX(0)" }, { transform: "translateX(-6px)" }, { transform: "translateX(6px)" }, { transform: "translateX(0)" }],
    { duration: 240, iterations: 2, easing: "ease-in-out" },
  );
}`,
      },
      {
        title: "Fade out a toast, then remove it",
        lang: "ts",
        code: `export async function dismissToast(toast: HTMLElement) {
  const fade = toast.animate({ opacity: [1, 0] }, { duration: 200, fill: "forwards" });
  try {
    await fade.finished;
    toast.remove();
  } catch (e) {
    if ((e as DOMException).name !== "AbortError") throw e;
  }
}`,
        note: "If something cancels the fade first, `finished` rejects with an `AbortError` and the toast stays.",
      },
      {
        title: "Keep a panel where its slide-in ended",
        lang: "ts",
        code: `export async function slideOpen(panel: HTMLElement) {
  const slide = panel.animate(
    { transform: ["translateX(-100%)", "translateX(0)"] },
    { duration: 250, easing: "ease-out", fill: "forwards" },
  );
  await slide.finished;
  slide.commitStyles();
  slide.cancel();
}`,
        note: "`commitStyles()` writes the end value into the inline style, so cancelling the filling animation changes nothing on screen.",
      },
    ],
    edgeCases: [
      "`cancel()` removes the effect at once and rejects `finished` with an `AbortError`.",
      '`easing: "linear(...)"` throws a `TypeError`, and `CSS.supports("animation-timing-function", "linear(0, 1)")` is `false`.',
      "`animate` and `getAnimations` are on `HTMLElement.prototype` and `SVGElement.prototype`, not `Element.prototype`. `getAnimations()` lists script animations only, not CSS animations and transitions.",
      "`finish` and `finished` settle one frame after the frame that reached the end; a filling animation paints its end value meanwhile.",
      "Composite `add` and `accumulate`, pseudo-element targets and properties the animator cannot interpolate log an advisory; timing, promises and events still run.",
    ],
  },
  "view-transitions": {
    examples: [
      {
        title: "Morph a card between grid and list layouts",
        lang: "vue",
        code: `<script setup lang="ts">
import { nextTick, ref } from "vue";

const layout = ref<"grid" | "list">("grid");

function switchLayout() {
  document.startViewTransition(async () => {
    layout.value = layout.value === "grid" ? "list" : "grid";
    await nextTick();
  });
}
</script>

<template>
  <button @click="switchLayout">Switch layout</button>
  <div :class="layout"><div class="hero-card">Featured</div></div>
</template>

<style>
.hero-card { view-transition-name: hero; }
::view-transition-group(*) { animation-duration: 0.3s; }
</style>`,
        note: "Awaiting `nextTick()` makes the transition capture the state after Vue has patched the DOM.",
      },
      {
        title: "Use a slower transition for one kind of change",
        lang: "ts",
        code: `export async function openGallery(showGallery: () => Promise<void>) {
  const transition = document.startViewTransition({ update: showGallery, types: ["gallery"] });
  await transition.finished;
}`,
        note: "Style it with `:active-view-transition-type(gallery)::view-transition-group(*)`.",
      },
    ],
    edgeCases: [
      "The old state stays on screen until the update callback's promise settles.",
      "A second `startViewTransition` skips the first: the first's `ready` rejects with an `AbortError`, and both update callbacks still run.",
      "`skipTransition()` ends the animation at once and resolves `finished`.",
      "`document.activeViewTransition` is the running transition and `null` once it finishes.",
    ],
  },
  "smooth-scrolling": {
    examples: [
      {
        title: "Scroll a chat to its newest message",
        lang: "ts",
        code: `export function scrollToNewest(chat: HTMLElement) {
  requestAnimationFrame(() => {
    chat.scrollTo({ top: chat.scrollHeight, behavior: "smooth" });
  });
}`,
        note: "Called right after a message is added from a key handler, `scrollHeight` is a frame old; the frame callback reads the new one.",
      },
      {
        title: "Animate every scroll of a container from CSS",
        lang: "css",
        code: `.settings-page {
  overflow-y: auto;
  scroll-behavior: smooth;
}`,
      },
    ],
    edgeCases: [
      "`scrollTop` read right after a smooth `scrollTo` still returns the starting position; it changes as the scroll runs.",
      "A smooth `scrollBy` during a smooth scroll adds to where the running scroll is going, not to where it is now.",
      "A target past either end is clamped, the same as a wheel scroll.",
      "The `scroll` event fires once, on the next frame.",
    ],
  },
  "pointer-lock": {
    examples: [
      {
        title: "Turn a knob by dragging without the cursor hitting the screen edge",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const value = ref(50);
const knob = ref<HTMLElement | null>(null);

const lock = () => knob.value!.requestPointerLock().catch(() => {});
const release = () => document.exitPointerLock();
function turn(e: MouseEvent) {
  if (document.pointerLockElement !== knob.value) return;
  value.value = Math.min(100, Math.max(0, value.value - e.movementY * 0.25));
}
</script>

<template>
  <div ref="knob" class="knob" @mousedown="lock" @mouseup="release" @mousemove="turn">{{ Math.round(value) }}</div>
</template>`,
      },
    ],
    edgeCases: [
      "A request without a user gesture fires `pointerlockerror` and rejects with a `NotAllowedError`.",
      "While locked, every move reaches the locked element with its `movementX` and `movementY`, and `clientX` and `clientY` stay where the lock started.",
      "Escape releases the lock before your listeners hear the key, and only a new gesture can lock again.",
      "Removing the locked element releases the lock; a second `exitPointerLock()` does nothing.",
    ],
  },
  "font-face": {
    examples: [
      {
        title: "Preview a font file the user picked",
        lang: "ts",
        code: `export async function previewFont(file: File, sample: HTMLElement) {
  const face = new FontFace("Preview Font", await file.arrayBuffer());
  await face.load();
  document.fonts.add(face);
  sample.style.fontFamily = "'Preview Font', sans-serif";
}`,
      },
      {
        title: "Load a display face from the app bundle on demand",
        lang: "ts",
        code: `const headline = new FontFace("Headline", "url(/fonts/headline.woff2)", { weight: "700" });

export async function useHeadlineFont() {
  if (headline.status === "unloaded") document.fonts.add(await headline.load());
}`,
      },
    ],
    edgeCases: [
      'A face made from an `ArrayBuffer` starts in `"loading"` and loads without a network round trip.',
      'A URL that does not exist rejects `load()` with a `NetworkError`, and `status` becomes `"error"`.',
      "`local()` sources are not consulted; `sk dev` logs an advisory.",
    ],
  },
  "document-fonts": {
    examples: [
      {
        title: "Measure text only after the fonts are in",
        lang: "ts",
        code: `export async function labelWidth(text: string, font = "14px Inter"): Promise<number> {
  await document.fonts.load(font, text);
  const ctx = document.createElement("canvas").getContext("2d")!;
  ctx.font = font;
  return ctx.measureText(text).width;
}`,
      },
      {
        title: "Hold the first paint of a chart until fonts are ready",
        lang: "ts",
        code: `export async function drawWhenReady(draw: () => void) {
  await document.fonts.ready;
  draw();
}`,
      },
    ],
    edgeCases: [
      'Faces from `@font-face` rules are in the set from startup with status `"unloaded"`, and each loads when text first asks for it.',
      '`check()` and `load()` need a size in the font string: `check("Inter")` throws a `SyntaxError`, `check("16px Inter")` does not.',
      "A face whose `unicode-range` covers none of the text is not loaded for it.",
    ],
  },
  "xml-serializer": {
    examples: [
      {
        title: "Export an inline SVG chart as a file",
        lang: "ts",
        code: `export async function exportChart(svg: SVGSVGElement) {
  const markup = new XMLSerializer().serializeToString(svg);
  const handle = await showSaveFilePicker({
    suggestedName: "chart.svg",
    types: [{ description: "SVG image", accept: { "image/svg+xml": [".svg"] } }],
  });
  const writable = await handle.createWritable();
  await writable.write(markup);
  await writable.close();
}`,
        note: "Call it from a click handler: the save picker needs a user gesture.",
      },
    ],
    edgeCases: [
      "An element in a namespace declares it once on the outermost element that needs it; children reuse the declaration.",
      "Text escapes `&`, `<` and `>`; attribute values also escape quotes and newlines.",
      "An argument that is not a node throws a `TypeError`.",
    ],
  },
  "custom-elements": {
    examples: [
      {
        title: "Instead: register a reusable widget as a Vue component",
        lang: "ts",
        code: `import { createApp } from "vue";
import App from "./App.vue";
import RatingStars from "./components/RatingStars.vue";

createApp(App).component("RatingStars", RatingStars).mount();`,
        note: '`<RatingStars :value="4" />` then works in every template, with no element registry.',
      },
    ],
    edgeCases: [
      "Vue's `defineCustomElement()` logs an advisory, and constructing the class it returns throws a `NotSupportedError`.",
    ],
  },
  "shadow-dom": {
    examples: [
      {
        title: "Instead: encapsulate a widget's styles with a scoped component",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ label: string }>();
</script>

<template>
  <span class="badge"><slot name="icon" />{{ label }}</span>
</template>

<style scoped>
.badge { display: inline-flex; gap: 4px; padding: 2px 8px; border-radius: 999px; }
</style>`,
        note: "Scoped styles stay inside the component, and slots take the place of shadow DOM slots.",
      },
    ],
  },
  "window-open": {
    examples: [
      {
        title: "Open a help page in the system browser",
        lang: "vue",
        code: `<script setup lang="ts">
const openHelp = () => window.open("https://docs.example.com/shortcuts", "_blank");
</script>

<template>
  <button class="help" @click="openHelp">Keyboard shortcuts</button>
</template>`,
      },
      {
        title: "Instead: open a second window of your own",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { SkWindow } from "@skathi-native/runtime";
import Palette from "./Palette.vue";

const showPalette = ref(false);
</script>

<template>
  <button @click="showPalette = true">Palette</button>
  <SkWindow v-if="showPalette" title="Palette" :bounds="{ width: 320 }" @close="showPalette = false">
    <Palette />
  </SkWindow>
</template>`,
      },
    ],
    edgeCases: [
      "For an `http:`, `https:` or `mailto:` URL it returns `null`, as with `noopener`, so there is no window object to script.",
      "Other schemes, such as `file:` or a custom protocol, are refused with an advisory; open a URL your app trusts with `appShell.openExternal(url)`.",
    ],
  },
  alert: {
    examples: [
      {
        title: "Instead: show the message in a banner",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const message = ref<string | null>(null);
const show = (text: string) => (message.value = text);
</script>

<template>
  <button @click="show('Export finished.')">Export</button>
  <div v-if="message" role="alert" class="banner">
    {{ message }}
    <button @click="message = null">Dismiss</button>
  </div>
</template>`,
      },
    ],
    edgeCases: [
      "Code after `alert()` runs at once instead of waiting for the user.",
      "The advisory is logged on the first call only; later calls are silent.",
    ],
  },
  confirm: {
    examples: [
      {
        title: "Instead: ask in a dialog component and await the answer",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const emit = defineEmits<{ delete: [] }>();
const pending = ref<((ok: boolean) => void) | null>(null);

const ask = () => new Promise<boolean>((resolve) => (pending.value = resolve));
function answer(ok: boolean) {
  pending.value?.(ok);
  pending.value = null;
}

async function deleteDraft() {
  if (await ask()) emit("delete");
}
</script>

<template>
  <button @click="deleteDraft">Delete draft</button>
  <div v-if="pending" class="overlay">
    <div role="dialog" aria-labelledby="confirm-title" class="dialog">
      <h2 id="confirm-title">Delete this draft?</h2>
      <button @click="answer(false)">Cancel</button>
      <button @click="answer(true)">Delete</button>
    </div>
  </div>
</template>

<style scoped>
.overlay { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; }
</style>`,
      },
    ],
    edgeCases: [
      "Code guarded by `if (confirm(...))` never runs, so a destructive action behind it is skipped rather than done without asking.",
    ],
  },
  prompt: {
    examples: [
      {
        title: "Instead: ask for a name in an inline field",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const emit = defineEmits<{ rename: [name: string] }>();
const editing = ref(false);
const name = ref("");

function submit() {
  if (name.value.trim()) emit("rename", name.value.trim());
  editing.value = false;
}
</script>

<template>
  <button v-if="!editing" @click="editing = true">Rename</button>
  <form v-else role="dialog" aria-label="Rename" @submit.prevent="submit">
    <input v-model="name" placeholder="New name" />
    <button type="submit">Save</button>
    <button type="button" @click="editing = false">Cancel</button>
  </form>
</template>`,
      },
    ],
    edgeCases: ["The default value passed as the second argument is ignored: the result is always `null`."],
  },
};
