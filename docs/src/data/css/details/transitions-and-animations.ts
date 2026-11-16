import type { Details } from "../../details";

export const details: Details = {
  transition: {
    examples: [
      {
        title: "Slide a sidebar open from a toggle",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const expanded = ref(false);
</script>

<template>
  <nav class="sidebar" :class="{ expanded }">
    <button class="toggle" @click="expanded = !expanded">Menu</button>
  </nav>
</template>

<style scoped>
.sidebar {
  width: 56px;
  background-color: #202124;
  transition: width 200ms ease-in-out, background-color 200ms ease-in-out;
}
.sidebar.expanded {
  width: 240px;
  background-color: #303134;
}
</style>`,
      },
      {
        title: "Expand an accordion panel",
        lang: "css",
        code: `.panel-body {
  max-height: 0px;
  padding: 0px 16px;
  overflow: hidden;
  transition: max-height 250ms ease-out, padding 250ms ease-out;
}
.panel[data-open] .panel-body {
  max-height: 320px;
  padding: 12px 16px;
}`,
        note: "`height: auto` does not animate, so the panel animates `max-height` between two pixel values.",
      },
      {
        title: "Lift a card on hover",
        lang: "css",
        code: `.card {
  transform: translateY(0px);
  background-color: #ffffff;
  transition: transform 150ms ease-out, background-color 150ms ease-out;
}
.card:hover {
  transform: translateY(-2px);
  background-color: #f8f9fa;
}`,
        note: "The `transition` sits on the base rule; the `:hover` rule only changes the values.",
      },
    ],
    edgeCases: [
      "Declare `transition` on the base rule. A `transition` inside a `:hover`, `:focus` or other state rule is ignored, with an advisory in `sk dev`.",
      "An element's first style never transitions: a node mounted with a `transition` starts at its declared values. Change a class or a style after mount to animate.",
      "A change to or from `auto`, a percentage or a `calc()` value snaps to the end value, with an advisory. So does a change between `px` and `rem`, unless one side is `0`.",
      "A property outside the animated set, such as `filter` or `box-shadow`, snaps; naming it in `transition` gives an advisory.",
      "When the OS asks for reduced motion, every transition snaps to its end value and no `transitionend` fires.",
    ],
  },
  "transition-property": {
    examples: [
      {
        title: "Animate a chip's colours and let its size snap",
        lang: "css",
        code: `.chip {
  transition-property: background-color, color, border-color;
  transition-duration: 120ms;
  transition-timing-function: ease-out;
}
.chip.selected {
  background-color: #e8f0fe;
  color: #1967d2;
  border-color: #1967d2;
  padding: 4px 14px;
}`,
        note: "The padding change snaps because `padding` is not in the list.",
      },
    ],
    edgeCases: [
      "Shorthand names expand to their longhands: `padding` covers the four sides, `border-radius` the four corners, `gap` both gaps, `border` the colour and the four widths, and `margin-inline` or `padding-block` their two physical sides.",
      "`all` covers every interpolated property except `--shading`; name `--shading` in the list to animate it.",
      "When a property is listed twice, or after `all`, the last entry's timing wins.",
      "Narrowing the list while a transition runs stops the dropped properties at their end values, and removing `transition` stops all of them. No `transitionend` fires for a stopped property.",
      "Colours interpolate premultiplied in sRGB, so a fade in from `transparent` keeps the target hue instead of passing through grey.",
    ],
  },
  "transition-duration": {
    examples: [
      {
        title: "Show a toast faster than it hides",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const visible = ref(false);
</script>

<template>
  <button @click="visible = !visible">Notify</button>
  <div class="toast" :class="{ visible }">Draft saved</div>
</template>

<style scoped>
.toast {
  opacity: 0;
  transition: opacity 400ms ease-in;
}
.toast.visible {
  opacity: 1;
  transition-duration: 150ms;
}
</style>`,
        note: "The duration comes from the style being changed to, so the toast appears in 150 ms and fades out over 400 ms.",
      },
    ],
    edgeCases: [
      "Lists cycle against `transition-property`: with three properties and `100ms, 200ms`, the third property gets `100ms`.",
      "`transition-duration` with no `transition-property` transitions `all`.",
      "A duration and a delay that add up to zero or less change the value at once, without a `transitionend`.",
      "`transitionend` reports the duration in seconds as `elapsedTime`, without the delay. It fires once per longhand, so `transition: padding` gives four events, each with its own `propertyName`.",
      "`transitionend` bubbles, so a listener on a list hears its rows' transitions; check `event.target`. `transitionrun`, `transitionstart` and `transitioncancel` are not fired.",
    ],
  },
  "transition-timing-function": {
    examples: [
      {
        title: "Pop a badge in with an overshoot",
        lang: "css",
        code: `.badge {
  transform: scale(0);
  opacity: 0;
  transition: transform 300ms cubic-bezier(0.34, 1.56, 0.64, 1), opacity 150ms linear;
}
.badge.shown {
  transform: scale(1);
  opacity: 1;
}`,
        note: "The curve's second control point is above 1, so the badge grows past full size and settles back.",
      },
    ],
    edgeCases: [
      "An easing that overshoots carries `transform` and lengths past their end value and back, while `opacity` and colour channels are clamped to their valid range.",
      "`cubic-bezier()` x values outside 0 to 1 are clamped into that range, with an advisory.",
    ],
  },
  "transition-delay": {
    examples: [
      {
        title: "Show a tooltip only after the pointer rests",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const shown = ref(false);
</script>

<template>
  <span class="anchor" @mouseenter="shown = true" @mouseleave="shown = false">
    Sync status
    <span class="tooltip" :class="{ shown }">Last synced 2 minutes ago</span>
  </span>
</template>

<style scoped>
.tooltip {
  opacity: 0;
  transition: opacity 120ms ease-out;
}
.tooltip.shown {
  opacity: 1;
  transition-delay: 500ms;
}
</style>`,
        note: "The delay is on `.shown` only, so the tooltip waits before it appears and starts hiding at once.",
      },
    ],
    edgeCases: [
      "The old value stays on screen during the delay. With a `0s` duration, it holds for the delay and then jumps to the end value.",
      "If the value changes back before the delay ends, nothing moves: the pending transition is dropped.",
    ],
  },
  "transition-behavior": {
    edgeCases: [
      'A `transition-behavior` declaration is dropped with an advisory, and `CSS.supports("transition-behavior", "allow-discrete")` returns `false`.',
    ],
  },
  animation: {
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
  width: 20px;
  height: 20px;
  border-radius: 10px;
  border: 2px solid #dadce0;
  border-top-color: #1a73e8;
  animation: spin 800ms linear infinite;
}`,
      },
      {
        title: "Fade a panel in while it rises",
        lang: "css",
        code: `@keyframes fade-in {
  from {
    opacity: 0;
  }
}
@keyframes rise {
  from {
    transform: translateY(8px);
  }
}
.panel {
  animation: fade-in 200ms ease-out, rise 300ms ease-out;
}`,
        note: "Each animation in the list runs on its own clock, so the fade ends before the movement does.",
      },
    ],
    edgeCases: [
      "When the OS asks for reduced motion, CSS animations do not start and fire no events. Turning the setting on cancels running ones with `animationcancel`.",
      "`display: none` on the element or an ancestor cancels its animations with `animationcancel`; showing it again starts them from the beginning.",
      "An animation with a `0s` duration and no delay never runs: neither `animationstart` nor `animationend` fires.",
      "Skathi fires `animationstart`, `animationend` and `animationcancel`, and they bubble; `animationiteration` is not fired.",
      "Once `animationend` has fired, removing the animation fires no `animationcancel`.",
    ],
  },
  "animation-name": {
    examples: [
      {
        title: "Shake a field after a failed save",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { saveDraft } from "../drafts";

const failed = ref(false);

async function save() {
  failed.value = !(await saveDraft());
}
</script>

<template>
  <input class="title" :class="{ shake: failed }" @animationend="failed = false" />
  <button @click="save">Save</button>
</template>

<style scoped>
@keyframes shake {
  25% { transform: translateX(-6px); }
  75% { transform: translateX(6px); }
}
.title.shake {
  animation: shake 240ms ease-in-out;
}
</style>`,
        note: "Clearing the class on `animationend` removes the name, so the next failure adds it back and the shake starts again.",
      },
      {
        title: "Swap enter and exit animations by state",
        lang: "css",
        code: `@keyframes slide-in {
  from { transform: translateX(100%); }
}
@keyframes slide-out {
  to { transform: translateX(100%); }
}
.drawer[data-state="open"] {
  animation: slide-in 200ms ease-out;
}
.drawer[data-state="closed"] {
  animation: slide-out 160ms ease-in forwards;
}`,
        note: "A new name at the same position in the list starts a new animation, so each state change plays its own.",
      },
    ],
    edgeCases: [
      "Only a new name at a position in the list restarts an animation; changing its other longhands keeps it running.",
      "A name with no matching `@keyframes` runs nothing and gives an advisory in `sk dev`.",
      "`@keyframes` inside `@media` exist only in windows where the query matches; a popout is checked against its own viewport.",
      "`animation-name: none` cancels the running animations and fires `animationcancel` for each one that has not ended.",
    ],
  },
  "animation-duration": {
    examples: [
      {
        title: "Run one keyframes block at two speeds",
        lang: "css",
        code: `@keyframes bounce {
  to {
    transform: translateY(-4px);
  }
}
.typing-dot {
  animation: bounce 400ms ease-in-out infinite alternate;
}
.typing-dot.idle {
  animation-duration: 1200ms;
}`,
      },
    ],
    edgeCases: [
      "Changing the duration of a running animation keeps its start time, so it jumps to the point that the elapsed time reaches under the new duration.",
      "A lone `animation-duration` in another rule or an inline style changes the animation already declared instead of replacing the list.",
    ],
  },
  "animation-timing-function": {
    examples: [
      {
        title: "Ease each leg of a bounce separately",
        lang: "css",
        code: `@keyframes drop {
  0% {
    transform: translateY(-40px);
    animation-timing-function: ease-in;
  }
  60% {
    transform: translateY(0px);
    animation-timing-function: ease-out;
  }
  80% {
    transform: translateY(-8px);
    animation-timing-function: ease-in;
  }
  100% {
    transform: translateY(0px);
  }
}
.badge {
  animation: drop 600ms both;
}`,
        note: "Each keyframe's easing shapes the segment from it to the next keyframe that sets the same property.",
      },
    ],
    edgeCases: [
      "The easing shapes each segment between two keyframes, not the whole cycle: with `ease` and keyframes at `0%`, `50%` and `100%`, the value eases in and out twice per cycle.",
      "Inside `@keyframes`, `animation-timing-function` applies to the segment that starts at that keyframe; a keyframe without one uses the element's `animation-timing-function`.",
    ],
  },
  "animation-iteration-count": {
    examples: [
      {
        title: "Flash a row three times after it updates",
        lang: "css",
        code: `@keyframes flash {
  50% {
    background-color: #fef7e0;
  }
}
.row.updated {
  animation: flash 600ms ease-in-out 3;
}`,
      },
    ],
    edgeCases: [
      "A fractional count stops part-way through a cycle: `1.5` with `forwards` holds the point halfway through the keyframes.",
      "With `infinite`, `animationend` never fires.",
    ],
  },
  "animation-direction": {
    examples: [
      {
        title: "Breathe a recording indicator in and out",
        lang: "css",
        code: `@keyframes breathe {
  from {
    opacity: 0.4;
  }
  to {
    opacity: 1;
  }
}
.rec-dot {
  animation: breathe 1.2s ease-in-out infinite alternate;
}`,
        note: "`alternate` plays every second cycle backwards, so the dot fades out as smoothly as it fades in.",
      },
    ],
    edgeCases: [
      "With `alternate` and an even iteration count, the animation ends on its `from` keyframe, and `forwards` holds that value.",
      "With `reverse`, a `backwards` fill shows the `to` keyframe during the delay.",
    ],
  },
  "animation-fill-mode": {
    examples: [
      {
        title: "Keep a toast where its entrance ends",
        lang: "css",
        code: `@keyframes toast-in {
  from {
    transform: translateY(16px);
    opacity: 0;
  }
  to {
    transform: translateY(0px);
    opacity: 1;
  }
}
.toast {
  opacity: 0;
  animation: toast-in 250ms ease-out 100ms both;
}`,
        note: "`both` shows the `from` keyframe during the 100 ms delay and keeps the `to` keyframe after the end, over the toast's own `opacity: 0`.",
      },
    ],
    edgeCases: [
      "`forwards` holds, for each property, the last keyframe that sets it; a property with no `100%` keyframe ends on the element's own value.",
      "A lone `animation-fill-mode` in another rule or an inline style changes the animation already declared instead of replacing the list.",
    ],
  },
  "animation-play-state": {
    examples: [
      {
        title: "Pause a sync spinner from a button",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const syncing = ref(true);
</script>

<template>
  <button @click="syncing = !syncing">{{ syncing ? "Pause sync" : "Resume sync" }}</button>
  <div class="sync-icon" :style="{ animationPlayState: syncing ? 'running' : 'paused' }" />
</template>

<style scoped>
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.sync-icon {
  width: 16px;
  height: 16px;
  animation: spin 1s linear infinite;
}
</style>`,
      },
    ],
    edgeCases: [
      "Resuming continues from where the animation paused; the time spent paused does not count.",
      "A lone `animation-play-state` in an inline style or another rule pauses the animation the stylesheet declared instead of replacing it.",
    ],
  },
  "animation-delay": {
    examples: [
      {
        title: "Stagger list items as they enter",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ results: { id: string; title: string }[] }>();
</script>

<template>
  <ul class="results">
    <li v-for="(result, i) in results" :key="result.id" class="result" :style="{ animationDelay: i * 40 + 'ms' }">
      {{ result.title }}
    </li>
  </ul>
</template>

<style scoped>
@keyframes fade-up {
  from {
    opacity: 0;
    transform: translateY(6px);
  }
}
.result {
  animation: fade-up 200ms ease-out backwards;
}
</style>`,
        note: "`backwards` holds each row at the `from` keyframe while it waits, so later rows do not show before their turn.",
      },
    ],
    edgeCases: [
      "`animationstart` fires when the delay ends, not when the animation is applied.",
      "`elapsedTime` on `animationstart`, `animationend` and `animationcancel` excludes the delay.",
    ],
  },
  "animation-timeline": {
    edgeCases: [
      "An `animation-timeline` declaration is dropped with an advisory, and the animation runs on the document timeline.",
      "`element.animate()` with a `timeline` other than `document.timeline` runs on `document.timeline`, with an advisory.",
    ],
  },
  "animation-range": {
    edgeCases: [
      "An `animation-range` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
      "`element.animate()` with `rangeStart` or `rangeEnd` runs on `document.timeline`, with an advisory.",
    ],
  },
  "animation-composition": {
    edgeCases: [
      "An `animation-composition` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
      'In `element.animate()`, `composite: "add"` and `"accumulate"` give an advisory; the timing, promises and events still run.',
    ],
  },
  "easing-functions": {
    examples: [
      {
        title: "Blink a text caret",
        lang: "css",
        code: `@keyframes blink {
  50% {
    opacity: 0;
  }
}
.caret {
  width: 1px;
  height: 16px;
  background-color: #202124;
  animation: blink 1s step-end infinite;
}`,
        note: "`step-end` holds each keyframe until the next one, so the caret switches instead of fading.",
      },
      {
        title: "Open a bottom sheet on a CSS curve from script",
        lang: "ts",
        code: `const sheet = document.querySelector<HTMLElement>(".bottom-sheet")!;

sheet.animate([{ transform: "translateY(100%)" }, { transform: "translateY(0px)" }], {
  duration: 280,
  easing: "cubic-bezier(0.2, 0, 0, 1)",
});`,
      },
    ],
    edgeCases: [
      "`step-start` is `steps(1, jump-start)` and `step-end` is `steps(1, jump-end)`.",
      "`steps(1, jump-none)` is invalid CSS; Skathi runs it as `steps(2, jump-none)` and gives an advisory.",
      '`CSS.supports("animation-timing-function", "linear(0, 1)")` returns `false`, and `element.animate()` throws a `TypeError` for a `linear()` easing. Test for it before you use one.',
    ],
  },
  "feature-web-animations": {
    examples: [
      {
        title: "Shake a field when validation fails",
        lang: "ts",
        code: `function shake(field: HTMLElement) {
  field.animate(
    [
      { transform: "translateX(0px)" },
      { transform: "translateX(-6px)" },
      { transform: "translateX(6px)" },
      { transform: "translateX(0px)" },
    ],
    { duration: 240, easing: "ease-in-out" },
  );
}`,
      },
      {
        title: "Fade a toast out, then remove it",
        lang: "ts",
        code: `async function dismiss(toast: HTMLElement) {
  const fade = toast.animate({ opacity: [1, 0] }, { duration: 180, fill: "forwards" });
  try {
    await fade.finished;
  } catch {
    return;
  }
  toast.remove();
}`,
        note: "If another part of the app calls `fade.cancel()`, `finished` rejects and the toast stays.",
      },
      {
        title: "Keep a panel's end position after the animation",
        lang: "ts",
        code: `async function slideIn(panel: HTMLElement) {
  const slide = panel.animate(
    { transform: ["translateX(-240px)", "translateX(0px)"] },
    { duration: 200, easing: "ease-out", fill: "forwards" },
  );
  await slide.finished;
  slide.commitStyles();
  slide.cancel();
}`,
        note: "`commitStyles()` writes the end value into the inline style, so cancelling the filling animation changes nothing on screen.",
      },
    ],
    edgeCases: [
      "`cancel()` removes the effect at once and rejects `finished` with an `AbortError` `DOMException`; catch it wherever you await `finished`.",
      "`getAnimations()` lists only animations started from script; CSS animations and transitions have no `Animation` objects.",
      "`finished` settles, and `finish` fires, one frame after the frame that reaches the end; a filling animation paints its end value in between.",
      "Keyframes for a property Skathi does not interpolate, such as `filter`, give an advisory and change at the end; timing, promises and events still run.",
      'Reduced motion stops CSS transitions and animations but not `element.animate()`; check `matchMedia("(prefers-reduced-motion: reduce)")` before you start one.',
    ],
  },
  "feature-view-transitions": {
    examples: [
      {
        title: "Cross-fade a theme switch",
        lang: "vue",
        code: `<script setup lang="ts">
import { nextTick, ref } from "vue";

const dark = ref(false);

function toggleTheme() {
  document.startViewTransition(async () => {
    dark.value = !dark.value;
    await nextTick();
  });
}
</script>

<template>
  <main class="app" :class="{ dark }">
    <button @click="toggleTheme">Toggle theme</button>
  </main>
</template>`,
        note: "Awaiting `nextTick()` resolves the update callback after Vue has patched the DOM, so the new state is the one captured.",
      },
      {
        title: "Focus the search field once the transition ends",
        lang: "ts",
        code: `async function openSearch(showSearch: () => Promise<void>) {
  const transition = document.startViewTransition(showSearch);
  await transition.finished;
  document.querySelector<HTMLInputElement>("#search")?.focus();
}`,
      },
    ],
    edgeCases: [
      "While a transition runs, an overlay covers the window and takes every pointer event; clicks reach the page again when it ends.",
      "Starting a second transition skips the first: its `ready` rejects with `AbortError`, its update still runs and its `finished` resolves.",
      "An update callback that throws or rejects rejects `updateCallbackDone`, `ready` and `finished` with its reason. One that has not settled after 4 seconds aborts the transition with a `TimeoutError`.",
      "While `document.hidden` is true, as in a minimized window, a transition is skipped at once: the update runs and `ready` rejects with `InvalidStateError`. Minimizing the window mid-transition skips it the same way.",
      "With reduced motion on, the transition captures both states and finishes without animating.",
    ],
  },
  "view-transition-name": {
    examples: [
      {
        title: "Move task rows to their new order",
        lang: "vue",
        code: `<script setup lang="ts">
import { nextTick, ref } from "vue";

const tasks = ref([
  { id: "a1", title: "Review pull request", done: false },
  { id: "b2", title: "Update changelog", done: false },
]);

function toggleDone(id: string) {
  document.startViewTransition(async () => {
    const task = tasks.value.find((t) => t.id === id)!;
    task.done = !task.done;
    tasks.value.sort((a, b) => Number(a.done) - Number(b.done));
    await nextTick();
  });
}
</script>

<template>
  <ul class="tasks">
    <li v-for="task in tasks" :key="task.id" :style="{ viewTransitionName: 'task-' + task.id }" @click="toggleDone(task.id)">
      {{ task.title }}
    </li>
  </ul>
</template>`,
        note: "Each row needs a name of its own; the group then animates the row from its old position to its new one.",
      },
    ],
    edgeCases: [
      'Two rendered elements with the same name skip the transition: `ready` rejects with `InvalidStateError` ("Unexpected duplicate view-transition-name") and the update still runs.',
      "An element with `display: none`, or inside one, is not captured, so one name can move from a hidden element to a shown one between the two states.",
      "The root element is captured as `root` unless you give it `view-transition-name: none`.",
      "`auto` names an element by its `id` attribute, or by a generated name when it has none; `match-element` always generates one per element.",
    ],
  },
  "view-transition-class": {
    examples: [
      {
        title: "Time every card's move with one rule",
        lang: "css",
        code: `.card {
  view-transition-class: card;
}
::view-transition-group(*.card) {
  animation-duration: 350ms;
  animation-timing-function: ease-out;
}`,
        note: "Each card still needs its own `view-transition-name`; the class only groups them for styling.",
      },
    ],
  },
  "pseudo-active-view-transition": {
    examples: [
      {
        title: "Ignore clicks while a transition runs",
        lang: "css",
        code: `:root:active-view-transition .task-list {
  pointer-events: none;
}`,
      },
    ],
  },
  "pseudo-active-view-transition-type": {
    examples: [
      {
        title: "Slide in the direction of navigation",
        lang: "ts",
        code: `function go(page: string, direction: "forward" | "back") {
  document.startViewTransition({
    update: () => router.push(page),
    types: [direction],
  });
}`,
        note: "A rule such as `:root:active-view-transition-type(back)::view-transition-new(root)` then picks the slide for each direction.",
      },
    ],
  },
  "pseudo-view-transition": {
    examples: [
      {
        title: "Slow the default cross-fade",
        lang: "css",
        code: `::view-transition-group(*) {
  animation-duration: 400ms;
  animation-timing-function: ease-in-out;
}`,
        note: "Timing set on a group carries to its image pair and its old and new images unless they set their own.",
      },
      {
        title: "Slide the old page out and the new one in",
        lang: "css",
        code: `@keyframes slide-out {
  to {
    transform: translateX(-40px);
    opacity: 0;
  }
}
@keyframes slide-in {
  from {
    transform: translateX(40px);
    opacity: 0;
  }
}
::view-transition-old(root) {
  animation: slide-out 220ms ease-in both;
}
::view-transition-new(root) {
  animation: slide-in 220ms ease-out both;
}`,
      },
    ],
    edgeCases: [
      "By default each group animates its size and position over `0.25s` with `animation-fill-mode: both`, `-old()` fades out and `-new()` fades in.",
      "An element present in only one state has only that image: a new element gets a `::view-transition-new()` and no `-old()`.",
      "A view transition pseudo-element selector that Skathi cannot compile drops its rule, with an advisory.",
    ],
  },
  "at-view-transition": {
    edgeCases: ['`@view-transition` is ignored, with an "at-rule ignored" advisory in `sk dev`.'],
  },
};
