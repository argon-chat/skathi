import type { Details } from "../../details";

const DROPPED_PSEUDO_CLASS = [
  "A rule that uses it is dropped with an advisory; the other selectors of its list, as in `.a, .b:invalid`, still apply.",
  "In `querySelector()`, `querySelectorAll()` and `matches()` it matches nothing and logs an advisory; nothing is thrown.",
];

export const details: Details = {
  "selector-type": {
    examples: [
      {
        title: "Set base styles for headings and buttons",
        lang: "css",
        code: `body {
  margin: 0;
  color: #e6e8eb;
  background-color: #16181c;
}
h2 {
  font-size: 15px;
  font-weight: 600;
}
button {
  padding: 4px 10px;
  border-radius: 6px;
}`,
      },
    ],
    edgeCases: [
      "`html`, `body` and `:root` all name the app's one root element, so `html body { }` matches nothing. Write a compound such as `body.dark` instead of `html.dark body`.",
    ],
  },

  "selector-class": {
    examples: [
      {
        title: "Mark the selected folder from component state",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const folders = ["Inbox", "Drafts", "Archive"];
const selected = ref("Inbox");
</script>

<template>
  <nav class="folders">
    <button v-for="name in folders" :key="name" class="folder" :class="{ selected: selected === name }" @click="selected = name">
      {{ name }}
    </button>
  </nav>
</template>

<style scoped>
.folder {
  color: #9aa0a6;
}
.folder.selected {
  color: #ffffff;
  background-color: #2b2f36;
}
</style>`,
      },
    ],
    edgeCases: ['Class names compare case-sensitively: `.Selected` does not match `class="selected"`.'],
  },

  "selector-id": {
    examples: [
      {
        title: "Size the sidebar and the scrolling content region",
        lang: "css",
        code: `#sidebar {
  width: 240px;
  flex-shrink: 0;
}
#content {
  flex: 1;
  min-width: 0;
  overflow: auto;
}`,
      },
    ],
  },

  "selector-universal": {
    examples: [
      {
        title: "Use border-box sizing everywhere",
        lang: "css",
        code: `* {
  box-sizing: border-box;
}`,
      },
      {
        title: "Keep every toolbar item at its own size",
        lang: "css",
        code: `.toolbar {
  display: flex;
  gap: 4px;
}
.toolbar > * {
  flex-shrink: 0;
}`,
      },
    ],
    edgeCases: [
      "The `hidden` attribute maps to `display: none` below every author rule, so `* { display: flex }` shows hidden elements again. Add `[hidden] { display: none }` after it.",
    ],
  },

  "selector-attribute": {
    examples: [
      {
        title: "Style a panel by its open state",
        lang: "css",
        code: `.panel[data-state="open"] {
  border-color: #4c8dff;
}
.panel[data-state="closed"] .panel-body {
  display: none;
}`,
      },
      {
        title: "Underline the selected tab from an ARIA attribute",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const tabs = ["General", "Audio", "Video"];
const active = ref("General");
</script>

<template>
  <div class="tabs" role="tablist">
    <button v-for="tab in tabs" :key="tab" role="tab" :aria-selected="tab === active" @click="active = tab">
      {{ tab }}
    </button>
  </div>
</template>

<style scoped>
[role="tab"][aria-selected="true"] {
  border-bottom: 2px solid #4c8dff;
}
</style>`,
      },
      {
        title: "Tint PDF attachments whatever the case of the extension",
        lang: "css",
        code: `.attachment[data-name$=".pdf" i] .icon {
  background-color: #d93025;
}`,
      },
    ],
    edgeCases: [
      'Values compare case-sensitively unless the selector ends with `i`: `[data-state="OPEN"]` does not match `data-state="open"`.',
      'An empty `class` attribute counts as absent, so `[class]` does not match `class=""`.',
      "The `hidden` attribute maps to `display: none` below every author rule: any rule that sets `display` on the element shows it again, while your own `[hidden] { display: none }` is an ordinary rule that wins by specificity.",
    ],
  },

  "selector-descendant": {
    examples: [
      {
        title: "Size every icon inside the toolbar",
        lang: "css",
        code: `.toolbar .icon {
  width: 16px;
  height: 16px;
  color: #9aa0a6;
}`,
      },
    ],
    edgeCases: [
      "A popout's content stays in the same tree under its DOM ancestors, so a selector such as `.workspace .palette` still matches it in the popout window.",
    ],
  },

  "selector-child": {
    examples: [
      {
        title: "Indent nested menu levels only",
        lang: "css",
        code: `.menu > li {
  padding: 4px 8px;
}
.menu > li > ul {
  padding-left: 12px;
}`,
      },
    ],
  },

  "selector-next-sibling": {
    examples: [
      {
        title: "Tighten the hint under a field label",
        lang: "css",
        code: `.label + .hint {
  margin-top: 2px;
  color: #7d838c;
}`,
      },
      {
        title: "Collapse repeated dividers",
        lang: "css",
        code: `.divider + .divider {
  display: none;
}`,
      },
    ],
    edgeCases: [
      "Text and comment nodes are skipped, so `.label + .hint` still matches when an off `v-if` sits between the two elements.",
    ],
  },

  "selector-subsequent-sibling": {
    examples: [
      {
        title: "Dim the wizard steps after the current one",
        lang: "css",
        code: `.step.current ~ .step {
  opacity: 0.5;
}`,
      },
    ],
    edgeCases: [
      "A user-action state left of `~`, as in `.a:hover ~ .b`, drops the rule with an advisory; set a class from a pointer event instead.",
    ],
  },

  "pseudo-hover": {
    examples: [
      {
        title: "Fade in a row highlight under the pointer",
        lang: "css",
        code: `.row {
  background-color: transparent;
  transition: background-color 120ms ease-out;
}
.row:hover {
  background-color: #23262c;
}`,
      },
      {
        title: "Open a dropdown while the pointer is over its menu",
        lang: "css",
        note: "The dropdown is laid out below `.menu`, outside its box, but as a DOM child it keeps `.menu` hovered.",
        code: `.menu {
  position: relative;
}
.menu .dropdown {
  display: none;
  position: absolute;
  top: 100%;
  left: 0;
}
.menu:hover .dropdown {
  display: flex;
  flex-direction: column;
}`,
      },
    ],
    edgeCases: [
      "A `transition` declared inside the `:hover` rule is ignored with an advisory. Declare it on the base rule, as in the first example; the change then interpolates both ways.",
      "`:hover` is the topmost element under the pointer and its DOM ancestors. An overlay on top takes the hover even if it listens for nothing.",
      "`pointer-events: none` keeps an element from being hit, not from being hovered: it is still `:hover` while a descendant that takes pointer events is under the pointer.",
      "Several states in one compound, such as `.btn:hover:focus`, are not an AND: the rule applies on the strongest state it names (`:active`, then `:hover`, `:focus-visible`, `:focus`), with an advisory.",
      "When content moves under a still pointer, `:hover` catches up within about 100 ms.",
    ],
  },

  "pseudo-active": {
    examples: [
      {
        title: "Give a button press feedback",
        lang: "css",
        code: `.btn {
  background-color: #2b6ff3;
}
.btn:active {
  background-color: #1f5ad1;
  transform: scale(0.97);
}`,
      },
    ],
    edgeCases: [
      "It matches the pressed element and its ancestors until the button is released.",
      "A `transition` on the base rule does not animate the change into or out of `:active`; only `:hover` changes interpolate.",
      "An element with `pointer-events: none` never shows its `:active` style.",
    ],
  },

  "pseudo-focus": {
    examples: [
      {
        title: "Highlight the field that has focus",
        lang: "css",
        code: `.field {
  border: 1px solid #3a3f47;
  outline: none;
}
.field:focus {
  border-color: #4c8dff;
}`,
      },
    ],
    edgeCases: [
      "`focus()` on an element inside a `visibility: hidden` or `display: none` box leaves focus where it was, so `:focus` does not move to it.",
      "A `transition` on the base rule does not animate the change into `:focus`; the style applies at once.",
      "Each window keeps its own focus: in `matches()`, a field focused in a popout is `:focus` while the main window's focused element still is too.",
    ],
  },

  "pseudo-focus-visible": {
    examples: [
      {
        title: "Show a focus ring for keyboard users only",
        lang: "css",
        code: `.btn:focus-visible {
  outline: 2px solid #4c8dff;
  outline-offset: 2px;
}`,
      },
    ],
    edgeCases: [
      "A button focused by a click does not match; the same button reached with Tab does.",
      "A `<textarea>`, or an `<input>` of a text type such as `text`, `search`, `email` or `password`, matches whenever it has focus, a click included, as in Chromium.",
    ],
  },

  "pseudo-focus-within": {
    examples: [
      {
        title: "Highlight a search box while its input has focus",
        lang: "css",
        code: `.search {
  border: 1px solid #3a3f47;
  border-radius: 8px;
}
.search:focus-within {
  border-color: #4c8dff;
  box-shadow: 0 0 0 3px rgb(76 141 255 / 0.25);
}`,
      },
    ],
    edgeCases: [
      "It stops at the window's root: while focus is in a popout, the elements above the popout's root in the main window do not match.",
    ],
  },

  "selector-ancestor-state": {
    examples: [
      {
        title: "Reveal row actions when the pointer is over the row",
        lang: "css",
        code: `.row .row-actions {
  opacity: 0;
  transition: opacity 120ms;
}
.row:hover .row-actions {
  opacity: 1;
}`,
      },
      {
        title: "Light up the send button while the composer has focus",
        lang: "css",
        code: `.composer .send {
  background-color: #3a3f47;
}
.composer:focus-within .send {
  background-color: #4c8dff;
}`,
      },
    ],
    edgeCases: [
      "The nearest ancestor that matches the compound is the one whose state counts.",
      "A `transition` on the descendant's base rule animates changes driven by an ancestor's `:hover`, as in the first example; changes driven by an ancestor's focus apply at once.",
      "The state follows the hover path, so an overlay covering the ancestor takes the state away even where the ancestor's box is under the pointer.",
    ],
  },

  "pseudo-disabled": {
    examples: [
      {
        title: "Dim the save button while a save runs",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const props = defineProps<{ save: () => Promise<void> }>();
const saving = ref(false);
async function saveDraft() {
  saving.value = true;
  await props.save().finally(() => (saving.value = false));
}
</script>

<template>
  <button class="save" :disabled="saving" @click="saveDraft">Save</button>
</template>

<style scoped>
.save {
  transition: opacity 150ms;
}
.save:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>`,
      },
    ],
    edgeCases: [
      "It reads the `disabled` attribute of any element, not only form controls.",
      "It is part of the element's base style, not a pointer state: a `transition` on the base rule animates the change, and it works inside `:not()`, `:is()` and `:has()`.",
    ],
  },

  "pseudo-enabled": {
    examples: [
      {
        title: "Show hover feedback only on buttons that can be used",
        lang: "css",
        code: `button:enabled:hover {
  background-color: #2b2f36;
}`,
      },
    ],
    edgeCases: [
      "It matches every element without a `disabled` attribute, not only form controls, so qualify it with a type or class as in the example.",
    ],
  },

  "pseudo-checked": {
    examples: [
      {
        title: "Build a toggle switch on a button",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const notify = ref(true);
</script>

<template>
  <button class="switch" role="switch" :checked="notify" :aria-checked="notify" @click="notify = !notify">
    <span class="knob" />
  </button>
</template>

<style scoped>
.switch {
  display: flex;
  width: 36px;
  padding: 2px;
  border-radius: 10px;
  background-color: #3a3f47;
}
.switch:checked {
  justify-content: flex-end;
  background-color: #4c8dff;
}
.knob {
  width: 16px;
  height: 16px;
  border-radius: 8px;
  background-color: #ffffff;
}
</style>`,
      },
    ],
    edgeCases: [
      "On a checkbox or radio button it follows the control's state, as `input.checked` reads it, and on an `<option>` it matches the selected one. A click, Space, `v-model` and `bind:checked` update the match.",
      'On any other element it reads the `checked` attribute, and `checked="false"` does not match, so a boolean binding works on a `<button>` as in the example.',
    ],
  },

  "pseudo-fullscreen": {
    examples: [
      {
        title: "Restyle a video player in fullscreen",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const player = ref<HTMLElement | null>(null);
function toggleFullscreen() {
  if (document.fullscreenElement) document.exitFullscreen();
  else player.value?.requestFullscreen();
}
</script>

<template>
  <div ref="player" class="player">
    <video class="video" src="./intro.mp4" />
    <button class="controls" @click="toggleFullscreen">Fullscreen</button>
  </div>
</template>

<style scoped>
.player {
  border-radius: 8px;
}
.player:fullscreen {
  border-radius: 0;
  background-color: #000000;
}
</style>`,
      },
    ],
    edgeCases: [
      "The engine sizes the fullscreen element to fill the window after the cascade, so `width`, `height`, `margin`, `inset`, `flex` and `transform` from your rules do not apply to it while it is fullscreen.",
      "The element is not moved in the DOM, so selectors such as `.app .player` keep matching it.",
      "One element matches at a time: requesting fullscreen on another element moves the match to it.",
    ],
  },

  "pseudo-root": {
    examples: [
      {
        title: "Switch the theme from script",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watchEffect } from "vue";
const dark = ref(true);
watchEffect(() => document.documentElement.classList.toggle("dark", dark.value));
</script>

<template>
  <button class="theme" @click="dark = !dark">Toggle theme</button>
</template>

<style>
:root {
  --bg: #ffffff;
  --fg: #1d1f23;
}
:root.dark {
  --bg: #16181c;
  --fg: #e6e8eb;
}
body {
  background-color: var(--bg);
  color: var(--fg);
}
</style>`,
      },
    ],
    edgeCases: [
      "`:root`, `html` and `body` are the same element, and `document.documentElement` and `document.body` both return it.",
    ],
  },

  "pseudo-empty": {
    examples: [
      {
        title: "Show a message when a result list has no items",
        lang: "css",
        code: `.results:empty::before {
  content: "No matches";
  color: #7d838c;
}`,
      },
      {
        title: "Hide a badge that has no count",
        lang: "css",
        code: `.badge:empty {
  display: none;
}`,
      },
    ],
    edgeCases: [
      "Comments and empty text nodes, which Vue leaves for an off `v-if` or an empty `v-for`, do not count, so the parent stays `:empty`. Whitespace-only text does count.",
      "A text child gaining its first character, or losing its last, restyles the parent.",
    ],
  },

  "pseudo-first-child": {
    examples: [
      {
        title: "Drop the top border of the first list row",
        lang: "css",
        code: `.list-row {
  border-top: 1px solid #2b2f36;
}
.list-row:first-child {
  border-top: none;
}`,
      },
    ],
    edgeCases: [
      "Text and comment nodes are skipped, so the comment an off `v-if` leaves before the first row does not stop it from matching.",
    ],
  },

  "pseudo-last-child": {
    examples: [
      {
        title: "Separate breadcrumb items except after the last",
        lang: "css",
        code: `.crumb:not(:last-child)::after {
  content: "/";
  margin: 0 6px;
  color: #7d838c;
}`,
      },
    ],
  },

  "pseudo-only-child": {
    examples: [
      {
        title: "Hide the close button when only one tab is open",
        lang: "css",
        code: `.tab:only-child .close {
  display: none;
}`,
      },
    ],
  },

  "pseudo-nth-last-child": {
    examples: [
      {
        title: "Show only the three newest toasts",
        lang: "css",
        note: "New toasts are appended at the end, so everything before the last three is hidden.",
        code: `.toast:nth-last-child(n + 4) {
  display: none;
}`,
      },
    ],
  },

  "pseudo-nth-of-type": {
    examples: [
      {
        title: "Stripe alternate paragraphs of a transcript",
        lang: "css",
        code: `.transcript p:nth-of-type(odd) {
  background-color: #1d2025;
}`,
      },
    ],
  },

  "pseudo-nth-last-of-type": {
    examples: [
      {
        title: "Remove the rule under the last changelog section",
        lang: "css",
        code: `.changelog section {
  border-bottom: 1px solid #2b2f36;
}
.changelog section:nth-last-of-type(1) {
  border-bottom: none;
}`,
      },
    ],
  },

  "pseudo-first-of-type": {
    examples: [
      {
        title: "Enlarge the lead paragraph of an article",
        lang: "css",
        code: `.article p:first-of-type {
  font-size: 15px;
  color: #e6e8eb;
}`,
      },
    ],
  },

  "pseudo-last-of-type": {
    examples: [
      {
        title: "Remove the gap after the last list item",
        lang: "css",
        code: `.thread li {
  margin-bottom: 8px;
}
.thread li:last-of-type {
  margin-bottom: 0;
}`,
      },
    ],
  },

  "pseudo-only-of-type": {
    examples: [
      {
        title: "Stretch a card's image when it is the only one",
        lang: "css",
        code: `.card img:only-of-type {
  width: 100%;
  height: auto;
}`,
      },
    ],
  },

  "pseudo-nth-child": {
    examples: [
      {
        title: "Stripe the rows of a list",
        lang: "css",
        code: `.row:nth-child(even) {
  background-color: #1d2025;
}`,
      },
      {
        title: "Emphasize the first three recent files",
        lang: "css",
        code: `.recent-file:nth-child(-n + 3) {
  font-weight: 600;
}`,
      },
    ],
    edgeCases: [
      "In `querySelector()` and `matches()`, the `of S` form matches nothing and logs an advisory; nothing is thrown.",
    ],
  },

  "pseudo-not": {
    examples: [
      {
        title: "Draw separators between menu items",
        lang: "css",
        code: `.menu-item:not(:last-child) {
  border-bottom: 1px solid #2b2f36;
}`,
      },
      {
        title: "Give hover feedback only to buttons that are not disabled",
        lang: "css",
        code: `.btn:not(:disabled):hover {
  background-color: #2b2f36;
}`,
      },
    ],
    edgeCases: [
      "A user-action state inside, as in `:not(:hover)`, drops the rule with an advisory in a stylesheet, but works in `matches()` and `querySelector()`, which read the live state.",
    ],
  },

  "pseudo-is": {
    examples: [
      {
        title: "Space every heading level in the editor",
        lang: "css",
        code: `.editor :is(h1, h2, h3) {
  margin: 12px 0 4px;
  font-weight: 600;
}`,
      },
      {
        title: "Share a section title style between two panels",
        lang: "css",
        code: `:is(.sidebar, .inspector) .section-title {
  font-size: 11px;
  text-transform: uppercase;
  color: #7d838c;
}`,
      },
    ],
  },

  "pseudo-where": {
    examples: [
      {
        title: "Ship component defaults that any class overrides",
        lang: "css",
        note: "`.compact` wins wherever it appears in the sheet, because `:where(.btn)` adds no specificity.",
        code: `.compact {
  padding: 2px 8px;
}
:where(.btn) {
  padding: 6px 12px;
  border-radius: 6px;
}`,
      },
    ],
  },

  "pseudo-has": {
    examples: [
      {
        title: "Drop the padding of cards whose first level holds an image",
        lang: "css",
        code: `.card:has(> img) {
  padding: 0;
}`,
      },
      {
        title: "Grey out a field label when its input is disabled",
        lang: "css",
        code: `.field:has(input:disabled) .field-label {
  color: #7d838c;
}`,
      },
      {
        title: "Block the workspace while a modal is open",
        lang: "css",
        code: `body:has(.modal[data-open]) .workspace {
  pointer-events: none;
  opacity: 0.6;
}`,
      },
    ],
    edgeCases: [
      "`:has(> x)` looks at children only and `:has(+ x)` at the next sibling only; `:has(x)` searches every descendant.",
      "It looks through Vue fragments, so elements rendered under a `v-if` or `v-for` count.",
    ],
  },

  "pseudo-scope": {
    examples: [
      {
        title: "Query the direct children of a list",
        lang: "ts",
        code: `const playlist = document.querySelector(".playlist")!;
const tracks = playlist.querySelectorAll(":scope > .track");
const current = playlist.querySelector(":scope > .track.playing");
current?.scrollIntoView();`,
      },
    ],
    edgeCases: ["In a stylesheet, `:scope` matches the root element, as `:root` does."],
  },

  "pseudo-placeholder-shown": {
    examples: [
      {
        title: "Instead: hide the clear button while the search is empty",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const query = ref("");
</script>

<template>
  <div class="search" :class="{ empty: query === '' }">
    <input v-model="query" placeholder="Search" />
    <button class="clear" @click="query = ''">Clear</button>
  </div>
</template>

<style scoped>
.search.empty .clear {
  display: none;
}
</style>`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-read-only": {
    examples: [
      {
        title: "Instead: style read-only fields by their attribute",
        lang: "css",
        code: `.field[readonly] {
  border-color: transparent;
  background-color: transparent;
}`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-required": {
    examples: [
      {
        title: "Instead: mark required fields by their attribute",
        lang: "css",
        code: `.field[required] {
  border-left: 2px solid #ff5c5c;
}`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-indeterminate": {
    examples: [
      {
        title: "Dim a select-all checkbox while only some rows are picked",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
const rows = ref([true, false, true]);
const all = computed(() => rows.value.every(Boolean));
const some = computed(() => rows.value.some(Boolean) && !all.value);
</script>

<template>
  <input type="checkbox" class="all" :checked="all" :indeterminate="some" />
</template>

<style scoped>
.all:indeterminate {
  opacity: 0.6;
}
</style>`,
      },
    ],
    edgeCases: [
      "It matches a checkbox whose `indeterminate` property is true; a click clears it. Radio buttons and progress bars never match.",
    ],
  },

  "pseudo-default": {
    examples: [
      {
        title: "Instead: label the default choice from a data attribute",
        lang: "css",
        code: `.choice[data-default] .choice-label::after {
  content: " (default)";
  color: #7d838c;
}`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-autofill": {
    examples: [
      {
        title: "Instead: tint fields your app filled from a saved profile",
        lang: "css",
        code: `.field[data-prefilled] {
  background-color: #1f2a3a;
}`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-valid": {
    examples: [
      {
        title: "Instead: confirm a checked field from aria-invalid",
        lang: "css",
        code: `.field[aria-invalid="false"] {
  border-color: #3fb950;
}`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-invalid": {
    examples: [
      {
        title: "Instead: flag an email field from computed validity",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
const email = ref("");
const invalid = computed(() => email.value !== "" && !email.value.includes("@"));
</script>

<template>
  <input v-model="email" class="field" type="email" :aria-invalid="invalid" />
</template>

<style scoped>
.field[aria-invalid="true"] {
  border-color: #ff5c5c;
}
</style>`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-user-valid": {
    examples: [
      {
        title: "Instead: confirm a field once the user has left it",
        lang: "css",
        code: `.field.touched:not(.invalid) {
  border-color: #3fb950;
}`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-user-invalid": {
    examples: [
      {
        title: "Instead: flag an empty name only after the field loses focus",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
const name = ref("");
const touched = ref(false);
const invalid = computed(() => touched.value && name.value.trim() === "");
</script>

<template>
  <input v-model="name" class="field" :class="{ invalid }" @blur="touched = true" />
</template>

<style scoped>
.field.invalid {
  border-color: #ff5c5c;
}
</style>`,
      },
    ],
    edgeCases: DROPPED_PSEUDO_CLASS,
  },

  "pseudo-link": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-visited": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-any-link": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-target": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-lang": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-dir": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-modal": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-popover-open": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-open": { edgeCases: DROPPED_PSEUDO_CLASS },
  "pseudo-defined": { edgeCases: DROPPED_PSEUDO_CLASS },

  "pseudo-placeholder": {
    examples: [
      {
        title: "Soften the placeholder of a search field",
        lang: "css",
        code: `.search-input::placeholder {
  color: #7d838c;
}`,
      },
    ],
    edgeCases: [
      "Only `color` applies. Without a rule, the placeholder is the input's text color at 45% opacity.",
      "A state before it, as in `input:focus::placeholder`, drops the rule with an advisory.",
    ],
  },

  "pseudo-selection": {
    examples: [
      {
        title: "Color selected text in a transcript and in an input",
        lang: "css",
        code: `.transcript::selection {
  background-color: rgb(76 141 255 / 0.35);
  color: #ffffff;
}
.code-input::selection {
  background-color: rgb(255 196 0 / 0.4);
}`,
      },
    ],
    edgeCases: [
      "Selected text takes the `::selection` style of its nearest element that has one, so one rule on a container covers all the text inside it.",
      "In `<input>` and `<textarea>`, only `background-color` applies; `color` is ignored.",
    ],
  },

  "pseudo-file-selector-button": {
    examples: [
      {
        title: "Match the file picker button to the app's buttons",
        lang: "css",
        code: `.upload::file-selector-button {
  margin-right: 8px;
  padding: 4px 12px;
  border: none;
  border-radius: 6px;
  background-color: #2b2f36;
  color: #e6e8eb;
}`,
      },
    ],
    edgeCases: [
      "Your declarations refine the built-in button, which has a gray background, a 1px border and a 3px radius; reset `border` if you do not want it.",
      "A state before it, as in `input:hover::file-selector-button`, drops the rule with an advisory.",
    ],
  },

  "pseudo-webkit-slider-thumb": {
    examples: [
      {
        title: "Grow a zoom slider's thumb under the pointer",
        lang: "css",
        code: `.zoom::-webkit-slider-thumb {
  width: 14px;
  height: 14px;
  background-color: #e6e8eb;
}
.zoom:hover::-webkit-slider-thumb {
  width: 16px;
  height: 16px;
}`,
      },
    ],
    edgeCases: [
      "The default thumb is a 16px circle in the input's `accent-color`; `width` and `height` resize it.",
      "The engine positions the thumb on the track: `top`, `left`, `margin-top` and `margin-left` on it are overwritten, so the WebKit `margin-top: -6px` centering trick is not needed.",
      "`:hover`, `:active` and `:focus-visible` before it follow the whole input, not the thumb alone.",
    ],
  },

  "pseudo-webkit-slider-runnable-track": {
    examples: [
      {
        title: "Paint a volume slider's filled part from its value",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
const volume = ref(60);
function onInput(event: Event) {
  volume.value = Number((event.target as HTMLInputElement).value);
}
</script>

<template>
  <input class="volume" type="range" min="0" max="100" :value="volume" :style="{ '--fill': volume + '%' }" @input="onInput" />
</template>

<style scoped>
.volume::-webkit-slider-runnable-track {
  height: 4px;
  border-radius: 2px;
  background: linear-gradient(90deg, #4c8dff var(--fill), #3a3f47 var(--fill));
}
</style>`,
      },
    ],
    edgeCases: [
      "Once a rule styles the track, the built-in accent fill left of the thumb is no longer drawn; paint the progress yourself, as in the example.",
      "The default track is 4px tall and gray, and it stays centered vertically whatever its `height`.",
      "`appearance: none` on the input removes the default track color and fill; the thumb keeps its accent color.",
      "Custom properties set on the input, inline or by a rule, resolve inside the part's rules.",
    ],
  },

  "pseudo-moz-range-thumb": {
    examples: [
      {
        title: "Recolor the thumb of a seek bar",
        lang: "css",
        code: `.seek::-moz-range-thumb {
  width: 10px;
  height: 10px;
  background-color: #ff5c5c;
}`,
      },
    ],
    edgeCases: [
      "It styles the same part as `::-webkit-slider-thumb`; rules written with either spelling cascade together by specificity and order.",
    ],
  },

  "pseudo-moz-range-track": {
    examples: [
      {
        title: "Thicken the track of a seek bar",
        lang: "css",
        code: `.seek::-moz-range-track {
  height: 6px;
  border-radius: 3px;
  background-color: #3a3f47;
}`,
      },
    ],
    edgeCases: [
      "It styles the same part as `::-webkit-slider-runnable-track`; rules written with either spelling cascade together by specificity and order.",
    ],
  },

  "pseudo-moz-range-progress": {
    examples: [
      {
        title: "Instead: color the filled part of a seek bar",
        lang: "css",
        code: `.seek {
  accent-color: #e8710a;
}`,
        note: "A styled `::-moz-range-track` or `::-webkit-slider-runnable-track` drops the filled part, so leave the track unstyled.",
      },
    ],
  },

  "pseudo-webkit-color-swatch": {
    examples: [
      {
        title: "Round a color picker's swatch",
        lang: "css",
        code: `.swatch {
  width: 28px;
  height: 28px;
}
.swatch::-webkit-color-swatch-wrapper {
  padding: 0;
}
.swatch::-webkit-color-swatch {
  border: none;
  border-radius: 6px;
}`,
      },
    ],
    edgeCases: [
      "The swatch sits in a wrapper with 2px of padding, which `::-webkit-color-swatch-wrapper` styles.",
      "The swatch is filled with the input's `value`. A value that is not a 3, 4, 6 or 8 digit hex color paints black, and alpha digits are ignored.",
    ],
  },

  "pseudo-first-line": { edgeCases: ["A rule that uses it is dropped with an advisory."] },
  "pseudo-first-letter": { edgeCases: ["A rule that uses it is dropped with an advisory."] },
  "pseudo-backdrop": { edgeCases: ["A rule that uses it is kept but matches nothing, without an advisory."] },
  "pseudo-highlight": { edgeCases: ["A rule that uses it is dropped with an advisory."] },
  "pseudo-host": {
    edgeCases: ["No advisory is printed, and the other selectors of a list such as `html, :host` still apply."],
  },
  "pseudo-part": {
    edgeCases: ["No advisory is printed, and the other selectors of its list still apply."],
  },
  "pseudo-slotted": {
    edgeCases: ["No advisory is printed, and the other selectors of its list still apply."],
  },
  "selector-namespace": {
    edgeCases: [
      "A rule with a namespace prefix, or a namespaced attribute such as `[xlink|href]`, is dropped with an advisory.",
    ],
  },
};
