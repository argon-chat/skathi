<script setup lang="ts">
import { computed, ref } from "vue";

const SIZES = [11, 12, 13, 14, 16, 20, 24];
const FONTS = [
  { key: "seg", family: '"Segoe UI", sans-serif' },
  { key: "man", family: "Manrope, sans-serif" },
];
const WEIGHTS = [400, 600];
const WORDS = ["Download", "Hamburgefonstiv", "minimum"];
const SCHEMES: Record<string, { fg: string; bg: string }> = {
  light: { fg: "#111111", bg: "#ffffff" },
  dark: { fg: "#f0f0f0", bg: "#1c1c1c" },
  grey: { fg: "#808080", bg: "#ffffff" },
  color: { fg: "#1a5fd6", bg: "#ffffff" },
  mica: { fg: "#f0f0f0", bg: "transparent" },
};
const PARAGRAPH =
  "Static text keeps its pixels while another subtree changes. Few phases, a whole-pixel baseline and the right weight decide how it reads.";

const page = ref("light");
const motion = ref("none");
const pinned = ref("none");
const smoothing = ref("");
const extra = ref("");
const scheme = computed(() => SCHEMES[page.value] ?? SCHEMES.light!);

(globalThis as { __SK_TEXT_EXP__?: unknown }).__SK_TEXT_EXP__ = {
  show(name: string) {
    page.value = name;
  },
  motion(kind: string) {
    motion.value = kind;
  },
  pin(transform: string) {
    pinned.value = transform;
  },
  with(css: string) {
    extra.value = css;
  },
  smooth(value: string) {
    smoothing.value = value;
  },
};

const offsets = Array.from({ length: 10 }, (_, k) => k);
const pinnedStyle = computed(() => (pinned.value === "none" ? {} : { transform: pinned.value }));
</script>

<template>
  <div
    :class="['root', extra, smoothing && `smooth-${smoothing}`]"
    :style="{ color: scheme.fg, background: scheme.bg }"
  >
    <div v-if="page === 'offsets'" class="offsets">
      <div v-for="size in [13, 16]" :key="size" class="col">
        <div
          v-for="k in offsets"
          :key="k"
          :class="['line', 'w', `k-seg-400-${size}-${k}`]"
          :style="{ fontFamily: FONTS[0]!.family, fontSize: `${size}px`, transform: `translateX(${k / 10}px)` }"
        >
          Illuminating minimum
        </div>
      </div>
    </div>
    <div v-else-if="page === 'motion'" class="motion">
      <div :class="['card', 'w', 'k-card', `m-${motion}`]" :style="pinnedStyle">
        <div class="rule" />
        <div class="hs">H H H H H H H H H H</div>
        <div class="rule" />
      </div>
    </div>
    <div v-else-if="page === 'mica'" class="mica">
      <div v-for="where in ['clear', 'card']" :key="where" :class="['plane', where]">
        <div v-for="tone in ['light', 'dark']" :key="tone" :class="['tone', tone]">
          <div
            v-for="size in [12, 14, 16]"
            :key="size"
            :class="['line', 'w', `k-mica-${where}-${tone}-${size}`]"
            :style="{ fontSize: `${size}px` }"
          >
            Hamburgefonstiv minimum
          </div>
        </div>
      </div>
    </div>
    <div v-else class="matrix">
      <div v-for="font in FONTS" :key="font.key" class="col" :style="{ fontFamily: font.family }">
        <template v-for="weight in WEIGHTS" :key="weight">
          <div v-for="size in SIZES" :key="size" class="row" :style="{ fontSize: `${size}px`, fontWeight: weight }">
            <span v-for="(word, i) in WORDS" :key="i" :class="['w', `k-${font.key}-${weight}-${size}-${i}`]">
              {{ word }}
            </span>
          </div>
        </template>
        <p
          v-for="size in [13, 16]"
          :key="size"
          :class="['para', 'w', `k-${font.key}-400-${size}-p`]"
          :style="{ fontSize: `${size}px` }"
        >
          {{ PARAGRAPH }}
        </p>
      </div>
    </div>
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  padding: 12px;
}
.matrix,
.offsets {
  display: flex;
  gap: 24px;
}
.col {
  display: flex;
  flex-direction: column;
  width: 420px;
}
.row {
  display: flex;
  gap: 10px;
  line-height: 1.3;
}
.para {
  margin: 8px 0 0;
  line-height: 1.4;
}
.line {
  line-height: 22px;
}
.contrast {
  text-contrast: luminance;
}
.layer .card {
  will-change: transform;
}
.smooth-subpixel {
  font-smoothing: subpixel-antialiased;
}
.smooth-gray {
  font-smoothing: antialiased;
}
.mica {
  display: flex;
  gap: 24px;
  font-family: "Segoe UI", sans-serif;
}
.plane {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 300px;
}
.tone {
  padding: 8px 12px;
}
.tone.light {
  color: #f0f0f0;
}
.tone.dark {
  color: #111111;
}
.card .tone.light {
  background: #1c1c1c;
}
.card .tone.dark {
  background: #ffffff;
}
.motion {
  display: flex;
  align-items: flex-start;
  padding: 40px;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  transform-origin: left center;
}
.rule {
  height: 1px;
  background: currentColor;
}
.hs {
  font-family: "Segoe UI", sans-serif;
  font-size: 24px;
  white-space: pre;
}
.m-scale {
  animation: grow 2s linear infinite alternate;
}
.m-translate {
  animation: slide 2s linear infinite alternate;
}
@keyframes grow {
  from {
    transform: scale(1);
  }
  to {
    transform: scale(1.14);
  }
}
@keyframes slide {
  from {
    transform: translate(0, 0);
  }
  to {
    transform: translate(8.3px, 3.3px);
  }
}
</style>
