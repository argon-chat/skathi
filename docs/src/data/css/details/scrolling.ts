import type { Details } from "../../details";

export const details: Details = {
  "scrollbar-width": {
    examples: [
      {
        title: "Use thin scrollbars in a sidebar list",
        lang: "css",
        code: `.sidebar-list {
  height: 100%;
  overflow-y: auto;
  scrollbar-width: thin;
}`,
      },
      {
        title: "Hide the scrollbar of a tab strip that still scrolls",
        lang: "css",
        code: `.tab-strip {
  display: flex;
  overflow-x: auto;
  scrollbar-width: none;
}`,
      },
    ],
    edgeCases: [
      "`auto` draws a 14px bar and `thin` an 8px one. Both are overlay bars drawn over the content, so neither takes layout space.",
      "`none` hides the bar only: the box still scrolls with the wheel and from script, and its `overscroll-behavior` still applies.",
      'Whether an idle bar fades away follows the platform\'s setting; on Windows that is "Automatically hide scroll bars".',
    ],
  },
  "scrollbar-color": {
    examples: [
      {
        title: "Match scrollbars to a dark log panel",
        lang: "css",
        code: `.log-panel {
  overflow: auto;
  background-color: #1f1f1f;
  scrollbar-color: #5f6368 transparent;
}`,
      },
      {
        title: "Tint scrollbars from a theme token",
        lang: "css",
        code: `:root {
  --muted-foreground: 240 5% 64%;
}
.sheet-body {
  overflow-y: auto;
  scrollbar-color: hsl(var(--muted-foreground) / 0.35) transparent;
}`,
      },
    ],
    edgeCases: [
      "Without `scrollbar-color`, the thumb is the element's `color` at 32% opacity, and 62% while hovered or dragged. With it, the thumb and track keep the declared colours in every state.",
      "It takes exactly two colours, thumb then track, or `auto`. A single colour is dropped with an advisory.",
      "`currentColor`, including inside `color-mix()`, resolves against the element's computed `color`.",
    ],
  },
  "scrollbar-gutter": {
    edgeCases: [
      "A `scrollbar-gutter` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it. Scrollbars here are overlays and take no layout space, so there is no shift to reserve against.",
    ],
  },
  "pseudo-webkit-scrollbar": {
    examples: [
      {
        title: "Instead: style a chat's scrollbar",
        lang: "css",
        code: `.chat-scroll {
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: #9aa0a6 transparent;
}`,
      },
    ],
    edgeCases: [
      "A rule with a `::-webkit-scrollbar` pseudo-element is dropped, with an advisory that points to `scrollbar-width` and `scrollbar-color`.",
    ],
  },
  "overscroll-behavior": {
    examples: [
      {
        title: "Keep a dropdown's scrolling from moving the page",
        lang: "css",
        code: `.menu-list {
  max-height: 240px;
  overflow-y: auto;
  overscroll-behavior: contain;
}`,
        note: "When the list reaches its top or bottom, further wheel movement stops there instead of scrolling the page behind it.",
      },
      {
        title: "Stop a horizontal carousel from chaining sideways",
        lang: "css",
        code: `.carousel {
  display: flex;
  overflow-x: auto;
  overscroll-behavior-x: contain;
}`,
      },
    ],
    edgeCases: [
      "`contain` and `none` behave the same: there is no bounce to suppress, and both keep the wheel from reaching an ancestor scroll box once this one is at its edge.",
      "A wheel gesture locks to the axis it started on.",
      "The two-value form, such as `overscroll-behavior: auto contain`, is dropped with an advisory; set `overscroll-behavior-x` and `overscroll-behavior-y` instead.",
      "`overscroll-behavior-inline` sets the x axis and `overscroll-behavior-block` the y axis.",
    ],
  },
  "scroll-behavior": {
    examples: [
      {
        title: "Glide to the newest message",
        lang: "vue",
        code: `<script setup lang="ts">
import { nextTick, ref } from "vue";

const messages = ref<string[]>([]);
const list = ref<HTMLElement | null>(null);

async function receive(text: string) {
  messages.value.push(text);
  await nextTick();
  list.value!.scrollTop = list.value!.scrollHeight;
}
</script>

<template>
  <div ref="list" class="messages">
    <p v-for="(text, i) in messages" :key="i">{{ text }}</p>
  </div>
</template>

<style scoped>
.messages {
  height: 360px;
  overflow-y: auto;
  scroll-behavior: smooth;
}
</style>`,
      },
      {
        title: "Jump to the top without the glide",
        lang: "ts",
        code: `const messages = document.querySelector<HTMLElement>(".messages")!;

messages.scrollTo({ top: 0, behavior: "instant" });`,
        note: "A `behavior` passed to the call overrides the box's `scroll-behavior`.",
      },
    ],
    edgeCases: [
      'It applies to the box being scrolled: a call with no `behavior`, or `behavior: "auto"`, glides when that box has `scroll-behavior: smooth`.',
      "A glide takes 150 to 500 ms, longer for a longer distance. Reading `scrollTop` during it returns the position so far, not the destination.",
      "A smooth `scrollBy()` during a glide adds to where the glide is going.",
      "An instant scroll of the same box, or the user scrolling it, stops the glide where it is.",
    ],
  },
  "scroll-snap-type": {
    edgeCases: [
      "A `scroll-snap-type` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "scroll-snap-align": {
    edgeCases: [
      "A `scroll-snap-align` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "scroll-snap-stop": {
    edgeCases: [
      "A `scroll-snap-stop` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "scroll-padding": {
    edgeCases: ['`CSS.supports("scroll-padding", "10px")` returns `true`, and so do the physical longhands.'],
  },
  "scroll-margin": {
    edgeCases: [
      "`scroll-margin` and all its longhands, the logical ones included, are accepted without an advisory, and `CSS.supports()` returns `true`.",
    ],
  },
};
