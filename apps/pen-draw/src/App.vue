<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";

const BARREL = 2;
const ERASER = 32;

type Tool = "ink" | "marker" | "eraser";
interface Dot {
  x: number;
  y: number;
  size: number;
}
interface Reading {
  x: number;
  y: number;
  pressure: number;
  tiltX: number;
  tiltY: number;
  t: number;
}
/** One stroke as `cargo run -p sk-tree --example predict-eval` reads it. */
interface TraceStroke {
  pointerType: string;
  down: Reading;
  moves: { t: number; readings: Reading[] }[];
  up: Reading | null;
}

const canvas = ref<HTMLCanvasElement | null>(null);
const overlay = ref<HTMLCanvasElement | null>(null);
const coalesced = ref(true);
const predicted = ref(true);
const rec = reactive({ on: false, strokes: 0, status: "" });
const recordLabel = computed(() =>
  rec.strokes > 0 ? `Save ${rec.strokes} stroke${rec.strokes === 1 ? "" : "s"}…` : rec.on ? "Stop" : "Record",
);
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
let tail: CanvasRenderingContext2D | null = null;
let last: Dot | null = null;
let tool: Tool = "ink";
let trace: TraceStroke[] = [];
let stroke: TraceStroke | null = null;

function fit() {
  const el = canvas.value;
  if (!el) return;
  const dpr = devicePixelRatio || 1;
  const w = Math.round(el.clientWidth * dpr);
  const h = Math.round(el.clientHeight * dpr);
  const top = overlay.value;
  if (top && (top.width !== w || top.height !== h || !tail)) {
    top.width = w;
    top.height = h;
    tail = top.getContext("2d");
    if (tail) {
      tail.setTransform(dpr, 0, 0, dpr, 0, 0);
      tail.lineCap = "round";
      tail.lineJoin = "round";
    }
  }
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
  if (!rec.on && rec.strokes > 0) {
    trace = [];
    Object.assign(rec, { strokes: 0, status: "" });
  }
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

function clearTail() {
  const top = overlay.value;
  if (!top || !tail) return;
  tail.save();
  tail.setTransform(1, 0, 0, 1, 0, 0);
  tail.clearRect(0, 0, top.width, top.height);
  tail.restore();
}

/** The predicted events, drawn on the overlay until the next move replaces them. */
function drawTail(e: PointerEvent) {
  clearTail();
  if (!tail || !last || !predicted.value || tool === "eraser") return;
  const ahead = e.getPredictedEvents();
  if (ahead.length === 0) return;
  tail.strokeStyle = "rgba(229, 72, 77, 0.8)";
  tail.lineWidth = last.size;
  tail.beginPath();
  tail.moveTo(last.x, last.y);
  for (const p of ahead) tail.lineTo(p.offsetX, p.offsetY);
  tail.stroke();
}

function reading(e: PointerEvent): Reading {
  return { x: e.clientX, y: e.clientY, pressure: e.pressure, tiltX: e.tiltX, tiltY: e.tiltY, t: e.timeStamp };
}

/** Starts recording; while recording, or with an unsaved recording, saves it. */
async function record() {
  if (!rec.on && rec.strokes === 0) {
    trace = [];
    Object.assign(rec, { on: true, status: "recording the next strokes" });
    return;
  }
  rec.on = false;
  stroke = null;
  if (rec.strokes === 0) {
    rec.status = "";
    return;
  }
  try {
    const file = await showSaveFilePicker({
      id: "pen-draw-trace",
      suggestedName: "pen-trace.json",
      types: [{ description: "Pointer trace", accept: { "application/json": [".json"] } }],
    });
    const out = await file.createWritable();
    await out.write(JSON.stringify({ format: "skathi-pointer-trace", version: 1, strokes: trace }));
    await out.close();
    rec.status = `saved ${trace.length} strokes to ${file.name}`;
    trace = [];
    rec.strokes = 0;
  } catch (err) {
    rec.status = `not saved (${err instanceof Error ? err.message : String(err)}); Clear drops it`;
  }
}

function key(e: KeyboardEvent) {
  if (e.key === "r" && !e.ctrlKey && !e.metaKey && !e.altKey) void record();
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
  if (rec.on) stroke = { pointerType: e.pointerType, down: reading(e), moves: [], up: null };
}

function move(e: PointerEvent) {
  const events = e.getCoalescedEvents();
  const samples = coalesced.value && events.length > 0 ? events : [e];
  show(e, samples.length);
  stroke?.moves.push({ t: e.timeStamp, readings: (events.length > 0 ? events : [e]).map(reading) });
  if (!last) return;
  for (const s of samples) {
    const next = dotFor(s);
    segment(last, next);
    last = next;
  }
  drawTail(e);
}

function up(e: PointerEvent) {
  show(e, 1);
  last = null;
  clearTail();
  if (stroke) {
    stroke.up = reading(e);
    trace.push(stroke);
    rec.strokes = trace.length;
    stroke = null;
  }
}

onMounted(() => {
  fit();
  addEventListener("resize", fit);
  addEventListener("keydown", key);
});
onBeforeUnmount(() => {
  removeEventListener("resize", fit);
  removeEventListener("keydown", key);
});
</script>

<template>
  <main class="page">
    <header class="bar">
      <strong>Pen draw</strong>
      <span class="hint">Tip draws, barrel button highlights, eraser erases.</span>
      <label class="toggle"><input v-model="coalesced" type="checkbox" /> coalesced events</label>
      <label class="toggle"><input v-model="predicted" type="checkbox" /> predicted tail</label>
      <button type="button" :class="{ recording: rec.on }" title="R" @click="record">
        {{ recordLabel }}
      </button>
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
      <canvas ref="overlay" class="tail" />
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
      <span v-if="rec.status" class="status">{{ rec.status }}</span>
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
.bar button.recording {
  border-color: #e5484d;
  color: #ffb3b5;
}
.sheet {
  position: relative;
  flex: 1;
  min-height: 0;
  margin: 0 16px;
}
.tail {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
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
.hud .status {
  color: #ffb3b5;
}
</style>
