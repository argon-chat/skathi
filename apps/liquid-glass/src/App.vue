<script setup lang="ts">
import { computed, reactive, ref } from "vue";

type Model = "Lens" | "Measured";
const models: Model[] = ["Lens", "Measured"];
const model = ref<Model>("Lens");
const dock = ["#ff5f57", "#febc2e", "#28c840", "#4aa8ff", "#c77dff"];

const dark = ref(false);
const lens = reactive({ blur: 0, refraction: 0.3, dispersion: 0.008, edge: 2, rim: 0.15, fresnel: 0.3 });
const measured = reactive({ blur: 8, detail: 0.275, lens: 1.3, dispersion: 0.14, frost: 0.7, rim: 1.4 });

const lensSliders = [
  { key: "blur", label: "Blur", min: 0, max: 24, step: 0.5 },
  { key: "refraction", label: "Refraction", min: 0.02, max: 1, step: 0.01 },
  { key: "dispersion", label: "Dispersion", min: 0, max: 0.2, step: 0.001 },
  { key: "edge", label: "Edge", min: 0, max: 8, step: 0.1 },
  { key: "rim", label: "Rim", min: 0, max: 2, step: 0.01 },
  { key: "fresnel", label: "Fresnel", min: 0, max: 2, step: 0.01 },
] as const;
const measuredSliders = [
  { key: "blur", label: "Blur", min: 0, max: 40, step: 0.5 },
  { key: "detail", label: "Detail", min: 0, max: 1, step: 0.005 },
  { key: "lens", label: "Lens", min: 0, max: 2, step: 0.05 },
  { key: "dispersion", label: "Dispersion", min: 0, max: 0.5, step: 0.01 },
  { key: "frost", label: "Frost", min: 0, max: 1, step: 0.01 },
  { key: "rim", label: "Rim light", min: 0, max: 3, step: 0.05 },
] as const;

const shading = computed(() =>
  model.value === "Lens"
    ? `liquid(${lens.blur}px, ${lens.refraction}, ${lens.dispersion}, ${lens.edge}, ${lens.rim}, ${lens.fresnel})`
    : `measured(${measured.blur}px, ${measured.detail}, ${measured.lens}, ${measured.dispersion}, ` +
      `${measured.frost}, ${dark.value ? 1 : 0}, ${measured.rim}, 18px)`,
);
const light = computed(() => model.value === "Measured" && !dark.value);

// The pill is dragged by its own pointer events, so it shows the lens over any part of the page.
const pill = reactive({ x: 380, y: 250 });
let grab: { dx: number; dy: number } | null = null;

function onDown(e: PointerEvent) {
  grab = { dx: e.clientX - pill.x, dy: e.clientY - pill.y };
  (e.currentTarget as Element).setPointerCapture(e.pointerId);
}
function onMove(e: PointerEvent) {
  if (!grab) return;
  pill.x = e.clientX - grab.dx;
  pill.y = e.clientY - grab.dy;
}
function onUp() {
  grab = null;
}
</script>

<template>
  <div class="desk" :class="{ light }">
    <img class="wallpaper" src="assets/wallpaper.jpg" alt="" />
    <div class="titlebar" />

    <div class="headline">
      <div class="eyebrow">Drag the glass over the words</div>
      <div class="h1">Liquid Glass</div>
      <div class="spectrum" />
    </div>

    <div class="toolbar" :style="{ '--shading': shading }">
      <button v-for="name in models" :key="name" class="tab" :class="{ on: model === name }" @click="model = name">
        {{ name }}
      </button>
    </div>

    <button v-if="model === 'Measured'" class="round" :style="{ '--shading': shading }" @click="dark = !dark">
      {{ dark ? "Dark" : "Light" }}
    </button>

    <div class="card" :style="{ '--shading': shading }">
      <template v-if="model === 'Lens'">
        <label v-for="s in lensSliders" :key="s.key" class="slider">
          <span class="name">{{ s.label }}</span>
          <input v-model.number="lens[s.key]" type="range" :min="s.min" :max="s.max" :step="s.step" />
          <span class="value">{{ lens[s.key] }}</span>
        </label>
      </template>
      <template v-else>
        <label v-for="s in measuredSliders" :key="s.key" class="slider">
          <span class="name">{{ s.label }}</span>
          <input v-model.number="measured[s.key]" type="range" :min="s.min" :max="s.max" :step="s.step" />
          <span class="value">{{ measured[s.key] }}</span>
        </label>
      </template>
    </div>

    <div
      class="pill"
      :style="{ left: pill.x + 'px', top: pill.y + 'px', '--shading': shading }"
      @pointerdown="onDown"
      @pointermove="onMove"
      @pointerup="onUp"
      @pointercancel="onUp"
    />

    <div class="dock" :style="{ '--shading': shading }">
      <div v-for="color in dock" :key="color" class="app"><div class="dot" :style="{ background: color }" /></div>
    </div>
  </div>
</template>

<style scoped>
.desk {
  --ink: #ffffff;
  --ink-soft: #ffffffc7;
  --fill: #ffffff24;
  --fill-on: #ffffff3d;
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #151a24;
  color: var(--ink);
  font-family: system-ui, sans-serif;
  user-select: none;
}
.desk.light {
  --ink: #111114;
  --ink-soft: #111114c7;
  --fill: #00000014;
  --fill-on: #00000026;
}
.wallpaper {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.titlebar {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 44px;
  app-region: drag;
}

.headline {
  position: absolute;
  top: 190px;
  left: 0;
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 18px;
  color: #ffffff;
  text-shadow: 0 2px 24px #00000059;
}
.eyebrow {
  font-size: 14px;
  font-weight: 600;
  letter-spacing: 0.32em;
  text-transform: uppercase;
  color: #ffffffcc;
}
.h1 {
  font-size: 120px;
  font-weight: 800;
  line-height: 120px;
}
.spectrum {
  width: 620px;
  height: 6px;
  border-radius: 3px;
  background: linear-gradient(90deg, #9dc627, #ffce00, #f60000, #fefdfe);
}

.toolbar {
  position: absolute;
  top: 56px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  gap: 4px;
  padding: 4px;
  border-radius: 26px;
}
.tab {
  height: 44px;
  padding: 0 22px;
  border: none;
  border-radius: 22px;
  background: transparent;
  color: var(--ink-soft);
  font-size: 14px;
  font-weight: 600;
}
.tab:hover {
  background: var(--fill);
}
.tab.on {
  background: var(--fill-on);
  color: var(--ink);
}

.round {
  position: absolute;
  top: 56px;
  right: 56px;
  width: 52px;
  height: 52px;
  border: none;
  border-radius: 26px;
  background: transparent;
  color: var(--ink);
  font-size: 12px;
  font-weight: 600;
}

.card {
  position: absolute;
  left: 40px;
  bottom: 40px;
  width: 340px;
  padding: 22px 24px;
  border-radius: 32px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  text-shadow: 0 1px 6px #00000066;
}
.desk.light .card {
  text-shadow: none;
}
.slider {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 12px;
  font-weight: 600;
}
.slider .name {
  width: 76px;
}
.slider input {
  flex: 1;
}
.slider .value {
  width: 44px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.pill {
  position: absolute;
  width: 360px;
  height: 144px;
  border-radius: 54px;
  cursor: grab;
}

.dock {
  position: absolute;
  bottom: 40px;
  right: 40px;
  display: flex;
  gap: 14px;
  padding: 12px 16px;
  border-radius: 38px;
}
.app {
  width: 52px;
  height: 52px;
  border-radius: 26px;
  background: var(--fill);
  display: flex;
  align-items: center;
  justify-content: center;
}
.dot {
  width: 22px;
  height: 22px;
  border-radius: 11px;
}
</style>
