import type { Details } from "../../details";

export const details: Details = {
  transform: {
    examples: [
      {
        title: "Center a dialog in the window",
        lang: "css",
        code: `.dialog {
  position: fixed;
  left: 50%;
  top: 50%;
  width: 360px;
  transform: translate(-50%, -50%);
}`,
        note: "Percentages in `translate()` resolve against the element's own size.",
      },
      {
        title: "Rotate a disclosure chevron when a section opens",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const open = ref(false);
</script>

<template>
  <button class="section-toggle" @click="open = !open">
    <span class="chevron" :class="{ open }">›</span>
    Advanced settings
  </button>
</template>

<style scoped>
.chevron {
  display: inline-block;
  transition: transform 150ms ease;
}
.chevron.open {
  transform: rotate(90deg);
}
</style>`,
      },
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
  border: 2px solid #3f3f46;
  border-top-color: #3b82f6;
  border-radius: 50%;
  animation: spin 800ms linear infinite;
}`,
      },
    ],
    edgeCases: [
      "`getBoundingClientRect()`, `getClientRects()` and `IntersectionObserver` report the transformed box. `offsetWidth`, `clientWidth` and `ResizeObserver` keep the layout box, so a `scale()` animation fires no resize.",
      "Clicks hit the element where it is drawn, not where layout put it.",
      "Translate, rotate and scale interpolate separately, even between different function lists, so `none` to `rotate(360deg)` makes one full turn.",
      "`rotateZ()` rotates in the plane. `translate3d()` and `scale3d()` keep their X and Y components. `rotateX(0)` and `rotateY(0)` leave the box unchanged. A 3D rotation through a nonzero X or Y angle is dropped, and the surrounding 2D functions still apply.",
      "A non-identity `transform` makes a stacking context.",
    ],
  },
  "transform-origin": {
    examples: [
      {
        title: "Grow a dropdown out of its corner",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const open = ref(false);
</script>

<template>
  <button @click="open = !open">Options</button>
  <div class="menu" :class="{ open }">Rename · Duplicate · Delete</div>
</template>

<style scoped>
.menu {
  position: absolute;
  right: 0;
  transform-origin: top right;
  transform: scale(0.95);
  opacity: 0;
  transition: transform 120ms ease-out, opacity 120ms ease-out;
}
.menu.open {
  transform: scale(1);
  opacity: 1;
}
</style>`,
      },
      {
        title: "Zoom a preview from its top-left corner",
        lang: "css",
        code: `.page-preview {
  transform-origin: 0 0;
  transform: scale(2);
}`,
      },
    ],
    edgeCases: [
      "Declaring `transform-origin` alone leaves the element untransformed.",
      "Percentages resolve against the element's own box; the initial value is `50% 50%`.",
    ],
  },
  perspective: {
    edgeCases: [
      'A declaration prints an advisory and changes nothing; `CSS.supports("perspective", "100px")` returns `false`.',
      "The `perspective()` function inside `transform` is dropped with an advisory, and the rest of the list still applies.",
    ],
  },
  "transform-style": {
    edgeCases: ["A declaration prints an advisory and changes nothing; children are drawn flat."],
  },
  "backface-visibility": {
    edgeCases: [
      'A declaration prints an advisory and changes nothing; `CSS.supports("backface-visibility", "hidden")` returns `false`.',
    ],
  },
  "perspective-origin": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
  translate: {
    examples: [
      {
        title: "Instead: Slide a drawer in with transform",
        lang: "css",
        code: `.drawer {
  transform: translateX(-100%);
  transition: transform 200ms ease;
}
.drawer.open {
  transform: translateX(0);
}`,
      },
    ],
    edgeCases: [
      "A declaration prints an advisory and changes nothing, and `getComputedStyle(el).translate` reads `none`.",
    ],
  },
  rotate: {
    examples: [
      {
        title: "Instead: Turn an icon with transform",
        lang: "css",
        code: `.refresh-icon.busy {
  transform: rotate(180deg);
  transition: transform 300ms ease;
}`,
      },
    ],
    edgeCases: [
      "A declaration prints an advisory and changes nothing, and `getComputedStyle(el).rotate` reads `none`.",
    ],
  },
  scale: {
    examples: [
      {
        title: "Instead: Shrink a button while pressed",
        lang: "css",
        code: `.primary-button {
  transition: transform 80ms ease;
}
.primary-button:active {
  transform: scale(0.97);
}`,
        note: "The `transition` stays on the base rule; one declared on a state rule is ignored.",
      },
    ],
    edgeCases: ["A declaration prints an advisory and changes nothing, and `getComputedStyle(el).scale` reads `none`."],
  },
  "transform-box": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
  "offset-path": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
};
