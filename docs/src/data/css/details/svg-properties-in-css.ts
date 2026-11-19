import type { Details } from "../../details";

const UNSUPPORTED = (name: string, value: string) =>
  `In a CSS rule, \`${name}\` is not applied and \`sk dev\` prints an advisory; \`CSS.supports("${name}", "${value}")\` is \`false\`.`;

export const details: Details = {
  fill: {
    examples: [
      {
        title: "Tint an icon with the text color",
        lang: "vue",
        code: `<template>
  <button class="icon-button">
    <svg viewBox="0 0 24 24" width="16" height="16">
      <path d="M12 2l3 7h7l-5.5 4.5 2 7.5-6.5-4-6.5 4 2-7.5L2 9h7z" />
    </svg>
    Favorite
  </button>
</template>

<style>
.icon-button {
  color: #9aa0a6;
}
.icon-button svg {
  fill: currentColor;
}
.icon-button:hover {
  color: #ffd166;
}
</style>`,
        note: "The hover changes `color`, which a `currentColor` fill follows; a `fill` in the `:hover` rule would not apply.",
      },
      {
        title: "Fill a star when an item is a favorite",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const favorite = ref(false);
</script>

<template>
  <button class="favorite" @click="favorite = !favorite">
    <svg class="star" :class="{ on: favorite }" viewBox="0 0 24 24" width="18" height="18">
      <path d="M12 2l3 7h7l-5.5 4.5 2 7.5-6.5-4-6.5 4 2-7.5L2 9h7z" />
    </svg>
  </button>
</template>

<style>
.star {
  fill: none;
  stroke: #9aa0a6;
  stroke-width: 2;
}
.star.on {
  fill: #ffd166;
  stroke: #ffd166;
}
</style>`,
      },
    ],
    edgeCases: [
      "An icon whose painted parts all use `currentColor` follows `color` without being rasterized again, so a `color` change on hover is cheap.",
      "A `fill` set on the `<svg>` inherits into its shapes, but a shape's own `fill` attribute beats the inherited value.",
      "A CSS rule beats a `fill` attribute on the same element, and an inline `style` beats the rule.",
    ],
  },

  stroke: {
    examples: [
      {
        title: "Draw outline icons in the text color",
        lang: "css",
        code: `.outline-icon {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-linejoin: round;
}`,
      },
      {
        title: "Highlight the selected series in a chart",
        lang: "css",
        code: `.series {
  fill: none;
  stroke: #5f6b7a;
}
.series.selected {
  stroke: #7aa2ff;
  stroke-width: 3;
}`,
        note: "Toggle `selected` from your component; a `stroke` in a `:hover` rule is not applied.",
      },
    ],
    edgeCases: [
      "A CSS rule beats a `stroke` attribute on the same element, and a shape's own attribute beats a value it would inherit from a rule on the `<svg>`.",
    ],
  },

  "stroke-width": {
    examples: [
      {
        title: "Thicken the active line of a chart",
        lang: "css",
        code: `.chart path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
}
.chart path.active {
  stroke-width: 3;
}`,
      },
    ],
    edgeCases: ["`stroke-width` does not animate: a transition on it jumps to the end value."],
  },

  "stroke-linecap": {
    examples: [
      {
        title: "Round the ends of a sparkline",
        lang: "css",
        code: `.sparkline {
  fill: none;
  stroke: #34c759;
  stroke-width: 2;
  stroke-linecap: round;
}`,
      },
    ],
  },

  "stroke-linejoin": {
    examples: [
      {
        title: "Soften the corners of a line chart",
        lang: "css",
        code: `.line-chart polyline {
  fill: none;
  stroke: #7aa2ff;
  stroke-width: 2;
  stroke-linejoin: round;
}`,
      },
    ],
  },

  "stroke-miterlimit": {
    examples: [
      {
        title: "Keep sharp chevrons pointed",
        lang: "css",
        code: `.chevron {
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linejoin: miter;
  stroke-miterlimit: 10;
}`,
        note: "With the default limit of 4, a narrow chevron's tip is beveled.",
      },
    ],
  },

  "stroke-dasharray": {
    examples: [
      {
        title: "Draw a dashed selection rectangle",
        lang: "css",
        code: `.selection-rect {
  fill: #7aa2ff14;
  stroke: #7aa2ff;
  stroke-width: 1;
  stroke-dasharray: 4 3;
}`,
      },
    ],
    edgeCases: ["`stroke-dasharray: none` removes the dashes."],
  },

  "stroke-dashoffset": {
    examples: [
      {
        title: "Fill a circular progress ring",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ progress: number }>();
const offset = computed(() => String(113 * (1 - props.progress)));
</script>

<template>
  <svg class="ring" viewBox="0 0 40 40" width="40" height="40">
    <circle cx="20" cy="20" r="18" stroke="#ffffff1f" />
    <circle class="value" cx="20" cy="20" r="18" :style="{ strokeDashoffset: offset }" />
  </svg>
</template>

<style>
.ring circle {
  fill: none;
  stroke-width: 4;
}
.value {
  stroke: #7aa2ff;
  stroke-dasharray: 113;
}
</style>`,
        note: "113 is the circle's circumference; the inline style beats the stylesheet rule.",
      },
    ],
    edgeCases: ["`stroke-dashoffset` does not animate: a transition on it jumps, so drive a moving ring from state."],
  },

  "fill-opacity": {
    examples: [
      {
        title: "Shade the area under a chart line",
        lang: "css",
        code: `.chart .area {
  fill: #7aa2ff;
  fill-opacity: 0.2;
}
.chart .line {
  fill: none;
  stroke: #7aa2ff;
}`,
      },
    ],
  },

  "stroke-opacity": {
    examples: [
      {
        title: "Draw faint grid lines",
        lang: "css",
        code: `.chart-grid line {
  stroke: currentColor;
  stroke-opacity: 0.12;
  stroke-width: 1;
}`,
      },
    ],
  },

  "fill-rule": {
    examples: [
      {
        title: "Cut the hole out of a ring icon",
        lang: "vue",
        code: `<template>
  <svg class="status-ring" viewBox="0 0 24 24" width="16" height="16">
    <path d="M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20zM12 7a5 5 0 1 0 0 10a5 5 0 1 0 0-10z" />
  </svg>
</template>

<style>
.status-ring {
  fill: #34c759;
  fill-rule: evenodd;
}
</style>`,
        note: "Both circles run the same direction, so under the default `nonzero` the inner one would be filled too.",
      },
    ],
  },

  "shape-rendering": {
    examples: [
      {
        title: "Keep 1px grid lines crisp",
        lang: "css",
        code: `.chart-grid {
  shape-rendering: crispEdges;
}
.chart-grid line {
  stroke: #ffffff1f;
  stroke-width: 1;
}`,
      },
    ],
    edgeCases: ["The keyword is case-insensitive in CSS: `crispedges` reaches the renderer as `crispEdges`."],
  },

  "stop-color": {
    examples: [
      {
        title: "Color a gradient stop from a theme",
        lang: "css",
        code: ".icon stop { stop-color: var(--accent, #34c759); }",
      },
    ],
  },

  "stop-opacity": {
    examples: [{ title: "Fade a gradient stop", lang: "css", code: ".icon stop { stop-opacity: 50%; }" }],
  },

  "clip-rule": {
    examples: [
      {
        title: "Instead: clip a badge to a ring with an attribute",
        lang: "vue",
        code: `<template>
  <svg viewBox="0 0 48 48" width="48" height="48">
    <defs>
      <clipPath id="ring">
        <path clip-rule="evenodd" d="M24 2a22 22 0 1 0 0 44a22 22 0 1 0 0-44zM24 12a12 12 0 1 0 0 24a12 12 0 1 0 0-24z" />
      </clipPath>
    </defs>
    <rect width="48" height="48" fill="#7aa2ff" clip-path="url(#ring)" />
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("clip-rule", "nonzero")],
  },

  "flood-color": {
    examples: [
      {
        title: "Instead: tint an SVG drop shadow with an attribute",
        lang: "vue",
        code: `<template>
  <svg viewBox="0 0 48 48" width="48" height="48">
    <defs>
      <filter id="glow">
        <feDropShadow dx="0" dy="0" stdDeviation="3" flood-color="#7aa2ff" />
      </filter>
    </defs>
    <circle cx="24" cy="24" r="12" fill="#7aa2ff" filter="url(#glow)" />
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("flood-color", "red")],
  },

  "lighting-color": {
    examples: [
      {
        title: "Instead: color a lighting filter with an attribute",
        lang: "vue",
        code: `<template>
  <svg viewBox="0 0 64 64" width="64" height="64">
    <defs>
      <filter id="emboss">
        <feDiffuseLighting in="SourceAlpha" lighting-color="#fff4d6" surfaceScale="3">
          <feDistantLight azimuth="225" elevation="45" />
        </feDiffuseLighting>
        <feComposite in2="SourceAlpha" operator="in" />
      </filter>
    </defs>
    <rect x="8" y="8" width="48" height="48" rx="10" filter="url(#emboss)" />
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("lighting-color", "red")],
  },

  x: {
    examples: [{ title: "Inset a rectangle horizontally", lang: "css", code: ".icon rect { x: 0.5px; }" }],
  },

  y: {
    examples: [{ title: "Inset a rectangle vertically", lang: "css", code: ".icon rect { y: 0.5px; }" }],
  },

  cx: {
    examples: [{ title: "Center a circle horizontally", lang: "css", code: ".icon circle { cx: 50%; }" }],
  },

  cy: {
    examples: [{ title: "Center a circle vertically", lang: "css", code: ".icon circle { cy: 50%; }" }],
  },

  r: {
    examples: [{ title: "Scale a circle with its viewport", lang: "css", code: ".icon circle { r: 25%; }" }],
  },

  rx: {
    examples: [{ title: "Round the horizontal corners", lang: "css", code: ".icon rect { rx: 7.5px; }" }],
  },

  ry: {
    examples: [{ title: "Round the vertical corners", lang: "css", code: ".icon rect { ry: 4px; }" }],
  },

  d: {
    examples: [
      {
        title: "Instead: redraw a sparkline from data",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ values: number[] }>();
const path = computed(() =>
  props.values.map((v, i) => (i === 0 ? "M" : "L") + i * 10 + " " + (30 - v)).join(" "),
);
</script>

<template>
  <svg viewBox="0 0 200 30" width="200" height="30">
    <path :d="path" fill="none" stroke="#34c759" stroke-width="2" />
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("d", 'path("M0 0L10 10")')],
  },

  "vector-effect": {
    edgeCases: [UNSUPPORTED("vector-effect", "non-scaling-stroke")],
  },

  "paint-order": {
    examples: [
      {
        title: "Instead: outline SVG text behind its fill",
        lang: "vue",
        code: `<template>
  <svg viewBox="0 0 160 40" width="160" height="40">
    <text x="8" y="28" font-size="22" fill="#ffffff" stroke="#000000" stroke-width="4" paint-order="stroke">
      Live
    </text>
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("paint-order", "stroke")],
  },

  marker: {
    examples: [
      {
        title: "Instead: put an arrowhead on a connector",
        lang: "vue",
        code: `<template>
  <svg viewBox="0 0 120 40" width="120" height="40">
    <defs>
      <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">
        <path d="M0 0L10 5L0 10z" fill="#9aa0a6" />
      </marker>
    </defs>
    <path d="M8 20H108" stroke="#9aa0a6" stroke-width="2" marker-end="url(#arrow)" />
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("marker", "none")],
  },

  "text-anchor": {
    examples: [
      {
        title: "Instead: center a gauge's value label",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ percent: number }>();
</script>

<template>
  <svg viewBox="0 0 80 80" width="80" height="80">
    <text x="40" y="46" text-anchor="middle" font-size="18" fill="currentColor">{{ percent }}%</text>
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("text-anchor", "middle")],
  },

  "dominant-baseline": {
    examples: [
      {
        title: "Instead: center a label vertically in a chip",
        lang: "vue",
        code: `<template>
  <svg viewBox="0 0 64 24" width="64" height="24">
    <rect width="64" height="24" rx="12" fill="#ffffff14" />
    <text x="32" y="12" text-anchor="middle" dominant-baseline="middle" font-size="11" fill="currentColor">
      Beta
    </text>
  </svg>
</template>`,
      },
    ],
    edgeCases: [UNSUPPORTED("dominant-baseline", "middle")],
  },
};
