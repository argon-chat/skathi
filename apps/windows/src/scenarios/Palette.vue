<script setup lang="ts">
import { computed, ref } from "vue";
import { appWindow, type SkWindowBounds } from "@skathi-native/runtime";
import Icon from "../components/Icon.vue";
import PopWindow from "../components/PopWindow.vue";
import Scenario from "../components/Scenario.vue";

const COLORS = [
  { name: "Plum", hex: "#8a44a3" },
  { name: "Terracotta", hex: "#c8553d" },
  { name: "Amber", hex: "#e0a43a" },
  { name: "Sage", hex: "#5f8f6a" },
  { name: "Lagoon", hex: "#2a8fbd" },
  { name: "Ink", hex: "#2f3a56" },
];
const ink = ref(COLORS[0]!);
const open = ref(false);
const bounds = ref<SkWindowBounds>({ width: 232, height: 300 });

function togglePalette() {
  if (!open.value && navigator.platform === "Win32") {
    const main = appWindow.getContentBounds();
    bounds.value = { x: main.x + main.width - 300, y: main.y + 210, width: 232, height: 300 };
  }
  open.value = !open.value;
}

const tint = computed(() => `${ink.value.hex}2e`);
</script>

<template>
  <Scenario id="pop-out/palette">
    <div class="card canvas-card">
      <svg class="canvas" viewBox="0 0 640 300" preserveAspectRatio="xMidYMid slice">
        <rect x="0" y="0" width="640" height="300" class="paint" :style="{ fill: tint }" />
        <circle cx="190" cy="104" r="58" class="paint" :style="{ fill: ink.hex }" />
        <path
          d="M0 230 C120 150 220 170 330 215 C430 255 520 200 640 175 L640 300 L0 300 Z"
          class="paint soft"
          :style="{ fill: ink.hex }"
        />
        <path
          d="M0 262 C140 215 260 240 380 262 C480 280 560 252 640 236 L640 300 L0 300 Z"
          class="paint"
          :style="{ fill: ink.hex }"
        />
      </svg>
      <div class="caption">
        <span class="swatch-dot" :style="{ background: ink.hex }" />
        <span>Ink: {{ ink.name }}</span>
        <span class="hex code">{{ ink.hex }}</span>
        <button class="btn" data-demo="palette-toggle" @click="togglePalette">
          <Icon name="palette" />{{ open ? "Hide palette" : "Show palette" }}
        </button>
      </div>
    </div>

    <PopWindow
      v-if="open"
      title="Palette"
      backdrop="acrylic"
      chrome="hidden"
      :bounds="bounds"
      :taskbar="false"
      :min-width="160"
      :min-height="200"
      :title-bar-height="36"
      @close="open = false"
    >
      <div class="palette">
        <header class="bar">
          <span>Palette</span>
          <span class="close" title="Close"><Icon name="close" /></span>
        </header>
        <div class="swatches">
          <button
            v-for="(c, i) in COLORS"
            :key="c.hex"
            class="swatch"
            :class="{ on: c.hex === ink.hex }"
            :style="{ background: c.hex }"
            :title="c.name"
            :data-demo="`swatch-${i}`"
            @click="ink = c"
          />
        </div>
        <div class="current">{{ ink.name }}</div>
      </div>
    </PopWindow>
  </Scenario>
</template>

<style scoped>
.canvas-card {
  padding: 0;
  gap: 0;
  overflow: hidden;
}
.canvas {
  width: 100%;
  height: 300px;
}
.paint {
  transition: fill 380ms ease;
}
.soft {
  fill-opacity: 0.55;
}
.caption {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 16px;
  font-size: 14px;
  color: var(--text);
}
.swatch-dot {
  width: 14px;
  height: 14px;
  border-radius: 7px;
  transition: background-color 380ms ease;
}
.hex {
  flex: 1;
  color: var(--text-3);
}
.palette {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--popout);
  color: var(--text);
  font-family: var(--font);
}
.bar {
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-left: 14px;
  font-size: 12px;
  color: var(--text-2);
  app-region: drag;
  flex-shrink: 0;
}
.close {
  width: 40px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  --window-control: close;
}
.close:hover {
  background: #c42b1c;
  color: #ffffff;
}
.swatches {
  display: grid;
  grid-template-columns: repeat(3, 48px);
  gap: 14px;
  padding: 18px 26px 10px;
}
.swatch {
  width: 48px;
  height: 48px;
  border-radius: 24px;
  border: 3px solid transparent;
  transition: border-color 140ms ease;
}
.swatch:hover,
.swatch.demo-hover {
  border-color: var(--control-line);
}
.swatch.on {
  border-color: var(--text);
}
.current {
  padding: 8px 26px;
  font-size: 13px;
  color: var(--text-2);
}
</style>
