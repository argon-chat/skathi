<script setup lang="ts">
import { onBeforeUnmount, onMounted, reactive, ref } from "vue";

const BARREL = 2;
const ERASER = 32;

type Tool = "ink" | "marker" | "eraser";
interface Dot {
  x: number;
  y: number;
  size: number;
}

const canvas = ref<HTMLCanvasElement | null>(null);
const coalesced = ref(true);
const hud = reactive({
  type: "-",
  id: 0,
  buttons: 0,
  pressure: 0,
  tiltX: 0,
  tiltY: 0,
  altitude: 0,
  azimuth: 0,
  twist: 0,
  samples: 0,
  tool: "ink" as Tool,
});

let ctx: CanvasRenderingContext2D | null = null;
let last: Dot | null = null;
let tool: Tool = "ink";

function fit() {
  const el = canvas.value;
  if (!el) return;
  const dpr = devicePixelRatio || 1;
  const w = Math.round(el.clientWidth * dpr);
  const h = Math.round(el.clientHeight * dpr);
  if (el.width === w && el.height === h && ctx) return;
  const keep = ctx && el.width > 0 && el.height > 0 ? ctx.getImageData(0, 0, el.width, el.height) : null;
  el.width = w;
  el.height = h;
  ctx = el.getContext("2d");
  if (!ctx) return;
  ctx.fillStyle = "#fbfaf7";
  ctx.fillRect(0, 0, w, h);
  if (keep) ctx.putImageData(keep, 0, 0);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
}

function clear() {
  const el = canvas.value;
  if (!el || !ctx) return;
  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.globalCompositeOperation = "source-over";
  ctx.fillStyle = "#fbfaf7";
  ctx.fillRect(0, 0, el.width, el.height);
  ctx.restore();
}

function toolFor(e: PointerEvent): Tool {
  if (e.buttons & ERASER) return "eraser";
  if (e.buttons & BARREL) return "marker";
  return "ink";
}

function dotFor(e: PointerEvent): Dot {
  // A mouse reports 0.5 while pressed, so it draws a medium line.
  const p = e.pressure;
  const size = tool === "eraser" ? 8 + 28 * p : tool === "marker" ? 14 : 0.6 + 7 * p * p;
  return { x: e.offsetX, y: e.offsetY, size };
}

function segment(a: Dot, b: Dot) {
  if (!ctx) return;
  ctx.globalCompositeOperation = tool === "eraser" ? "destination-out" : "source-over";
  ctx.strokeStyle = tool === "marker" ? "rgba(255, 196, 0, 0.35)" : "#1d1b26";
  ctx.lineWidth = (a.size + b.size) / 2;
  ctx.beginPath();
  ctx.moveTo(a.x, a.y);
  ctx.lineTo(b.x, b.y);
  ctx.stroke();
}

function show(e: PointerEvent, samples: number) {
  hud.type = e.pointerType;
  hud.id = e.pointerId;
  hud.buttons = e.buttons;
  hud.pressure = e.pressure;
  hud.tiltX = e.tiltX;
  hud.tiltY = e.tiltY;
  hud.altitude = (e.altitudeAngle * 180) / Math.PI;
  hud.azimuth = (e.azimuthAngle * 180) / Math.PI;
  hud.twist = e.twist;
  hud.samples = samples;
}

function down(e: PointerEvent) {
  if (e.button !== 0 && e.button !== 2 && e.button !== 5) return;
  // No compatibility mouse events, and no text selection, while drawing.
  e.preventDefault();
  canvas.value?.setPointerCapture(e.pointerId);
  tool = hud.tool = toolFor(e);
  last = dotFor(e);
  segment(last, last);
  show(e, 1);
}

function move(e: PointerEvent) {
  const events = coalesced.value ? e.getCoalescedEvents() : [];
  const samples = events.length > 0 ? events : [e];
  show(e, samples.length);
  if (!last) return;
  for (const s of samples) {
    const next = dotFor(s);
    segment(last, next);
    last = next;
  }
}

function up(e: PointerEvent) {
  show(e, 1);
  last = null;
}

onMounted(() => {
  fit();
  addEventListener("resize", fit);
});
onBeforeUnmount(() => removeEventListener("resize", fit));
</script>

<template>
  <main class="page">
    <header class="bar">
      <strong>Pen draw</strong>
      <span class="hint">Tip draws, barrel button highlights, eraser erases. A mouse draws too.</span>
      <label class="toggle"><input v-model="coalesced" type="checkbox" /> coalesced events</label>
      <button type="button" @click="clear">Clear</button>
    </header>
    <div class="sheet">
      <canvas
        ref="canvas"
        class="paper"
        @pointerdown="down"
        @pointermove="move"
        @pointerup="up"
        @pointercancel="up"
        @contextmenu.prevent
      />
    </div>
    <footer class="hud">
      <span>{{ hud.type }} #{{ hud.id }}</span>
      <span>{{ hud.tool }}</span>
      <span>buttons {{ hud.buttons }}</span>
      <span>pressure {{ hud.pressure.toFixed(2) }}</span>
      <span>tilt {{ hud.tiltX }}°, {{ hud.tiltY }}°</span>
      <span>altitude {{ hud.altitude.toFixed(0) }}° azimuth {{ hud.azimuth.toFixed(0) }}°</span>
      <span>twist {{ hud.twist }}°</span>
      <span>{{ hud.samples }} sample{{ hud.samples === 1 ? "" : "s" }}/event</span>
    </footer>
  </main>
</template>

<style>
html,
body,
#app {
  height: 100%;
  margin: 0;
}
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #2a2733;
  color: #ece9f3;
  font-family: "Segoe UI", system-ui, sans-serif;
  font-size: 13px;
  user-select: none;
}
.bar {
  display: flex;
  align-items: center;
  gap: 16px;
  box-sizing: border-box;
  width: env(titlebar-area-width, 100%);
  height: env(titlebar-area-height, 44px);
  padding: 0 16px;
  margin-left: env(titlebar-area-x, 0px);
  app-region: drag;
}
.bar label,
.bar button {
  app-region: no-drag;
}
.hint {
  flex: 1;
  color: #a9a3b8;
}
.toggle {
  display: flex;
  align-items: center;
  gap: 6px;
}
.bar button {
  padding: 4px 12px;
  border: 1px solid #514b5e;
  border-radius: 6px;
  background: #3a3546;
  color: inherit;
}
.sheet {
  flex: 1;
  min-height: 0;
  margin: 0 16px;
}
.paper {
  display: block;
  width: 100%;
  height: 100%;
  border-radius: 8px;
  background: #fbfaf7;
  touch-action: none;
  cursor: crosshair;
}
.hud {
  display: flex;
  flex-wrap: wrap;
  gap: 18px;
  padding: 10px 16px;
  color: #c9c3d6;
  font-variant-numeric: tabular-nums;
}
</style>
