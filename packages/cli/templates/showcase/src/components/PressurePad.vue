<script setup lang="ts">
import { appHaptics } from "@skathi-native/runtime";
import { onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import Card from "./Card.vue";

const width = 640;
const height = 200;
const ruling = 40;
const dpr = devicePixelRatio;
const canvas = shallowRef<HTMLCanvasElement | null>(null);
const brush = ref<"ink" | "pencil">("ink");
const weight = ref(4);
const snap = ref(true);
const pointerType = ref("");
const pressure = ref(0);
const feedback = ref<boolean | null>(null);
const pulseResult = ref("");
let context: CanvasRenderingContext2D | null = null;
let active: number | null = null;
let sliderPointer: number | null = null;
let guide: number | null = null;
let last: { x: number; y: number; size: number } | null = null;
let continuous = false;
let checkedPressure = false;
let hue = 220;

function clear() {
  const c = context;
  if (!c) return;
  const bg = c.createLinearGradient(0, 0, width, height);
  bg.addColorStop(0, "#14141c");
  bg.addColorStop(1, "#1c1630");
  c.fillStyle = bg;
  c.fillRect(0, 0, width, height);
  c.strokeStyle = "#8b9dff3d";
  c.lineWidth = 1;
  c.beginPath();
  for (let y = ruling; y < height; y += ruling) {
    c.moveTo(0, y + 0.5);
    c.lineTo(width, y + 0.5);
  }
  c.stroke();
}

function probe(event: PointerEvent) {
  pointerType.value = event.pointerType;
  feedback.value = appHaptics.supportsPointer(event.pointerId);
}

function testHaptics() {
  if (appHaptics.perform("generic")) {
    pulseResult.value = "The system accepted the pulse.";
  } else if (appHaptics.supported) {
    pulseResult.value = "The active device could not play this pulse.";
  } else {
    pulseResult.value = "The system reports no haptic feedback for the active device.";
  }
}

function paint(event: PointerEvent) {
  const c = context;
  const bounds = canvas.value?.getBoundingClientRect();
  if (!c || !bounds || !bounds.width || !bounds.height) return;
  const readings = event.getCoalescedEvents();
  let aligned = false;
  for (const p of readings.length ? readings : [event]) {
    const x = ((p.clientX - bounds.left) * width) / bounds.width;
    const rawY = ((p.clientY - bounds.top) * height) / bounds.height;
    const line = Math.round(rawY / ruling) * ruling;
    const nextGuide =
      snap.value && line > 0 && line < height && Math.abs(rawY - line) <= (guide === line ? 8 : 5) ? line : null;
    if (nextGuide !== null && nextGuide !== guide) aligned = true;
    guide = nextGuide;
    const y = guide ?? rawY;
    const size = 1 + weight.value * p.pressure * (brush.value === "pencil" ? 1.5 : 3);
    if (last) {
      c.lineWidth = (last.size + size) / 2;
      c.beginPath();
      c.moveTo(last.x, last.y);
      c.lineTo(x, y);
      c.stroke();
    }
    c.beginPath();
    c.arc(x, y, size / 2, 0, 2 * Math.PI);
    c.fill();
    last = { x, y, size };
  }
  pressure.value = event.pressure;
  if (event.pointerType === "mouse" && !checkedPressure && event.pressure !== 0.5) {
    checkedPressure = true;
    probe(event);
  }
  if (aligned) appHaptics.perform("alignment", { pointerId: event.pointerId, when: "next-frame" });
}

function down(event: PointerEvent) {
  if (!context || !canvas.value || active !== null || event.button !== 0) return;
  event.preventDefault();
  active = event.pointerId;
  canvas.value.setPointerCapture(active);
  checkedPressure = false;
  probe(event);
  hue = (hue + 23) % 360;
  context.fillStyle = context.strokeStyle = `hsl(${hue}, 84%, 76%)`;
  if (event.pointerType === "pen") {
    continuous = appHaptics.perform(brush.value === "ink" ? "ink-continuous" : "pencil-continuous", {
      pointerId: active,
    });
  }
  paint(event);
}

function finish() {
  if (active !== null && continuous) appHaptics.stop(active);
  active = null;
  last = null;
  guide = null;
  continuous = false;
  pressure.value = 0;
}

function setWeight(event: Event) {
  const next = Number((event.currentTarget as HTMLInputElement).value);
  if (next === weight.value) return;
  weight.value = next;
  if (sliderPointer !== null) {
    appHaptics.perform(next === 1 || next === 8 ? "generic" : "level-change", {
      pointerId: sliderPointer,
      when: "next-frame",
    });
  }
}

function beginSlider(e: PointerEvent) {
  sliderPointer = e.pointerId;
}
function endSlider() {
  sliderPointer = null;
}
function move(e: PointerEvent) {
  if (e.pointerId === active) paint(e);
}
function up(e: PointerEvent) {
  if (e.pointerId === active) finish();
}

onMounted(() => {
  context = canvas.value?.getContext("2d") ?? null;
  if (context) {
    context.scale(dpr, dpr);
    context.lineCap = "round";
    clear();
  }
});
onBeforeUnmount(finish);
</script>

<template>
  <Card title="Pressure sketchpad" api="PointerEvent.pressure · appHaptics">
    <p class="muted">Draw across the guides. Press harder with a pen or Force Touch trackpad to widen the stroke.</p>
    <canvas
      ref="canvas"
      class="pressure-pad"
      aria-label="Pressure sketchpad"
      :width="width * dpr"
      :height="height * dpr"
      @pointerenter="probe"
      @pointerdown="down"
      @pointermove="move"
      @pointerup="up"
      @pointercancel="up"
      @lostpointercapture="up"
    />
    <div class="row">
      <label class="tool">
        Tip
        <select v-model="brush" class="input" aria-label="Brush tip">
          <option value="ink">Ink</option>
          <option value="pencil">Pencil</option>
        </select>
      </label>
      <label class="tool">
        Width
        <input
          type="range"
          aria-label="Brush width"
          min="1"
          max="8"
          step="1"
          :value="weight"
          @input="setWeight"
          @pointerdown="beginSlider"
          @pointerup="endSlider"
          @pointercancel="endSlider"
          @lostpointercapture="endSlider"
        />
        {{ weight }}
      </label>
      <label class="tool"><input v-model="snap" type="checkbox" /> Snap to guides</label>
      <button class="btn" @click="clear">Clear</button>
      <button class="btn" aria-label="Test haptics on the current device" @click="testHaptics">Test haptics</button>
    </div>
    <p v-if="pulseResult" class="muted" role="status">{{ pulseResult }}</p>
    <p class="muted readout" v-if="pointerType">
      {{ pointerType }} · pressure {{ pressure.toFixed(2) }} ·
      {{ feedback ? "Device feedback available" : "Device feedback unavailable" }}
    </p>
  </Card>
</template>

<style scoped>
.pressure-pad {
  width: 100%;
  max-width: 640px;
  aspect-ratio: 16 / 5;
  border-radius: 10px;
  cursor: crosshair;
  touch-action: none;
}
.tool {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
  font-size: 13px;
}
.readout {
  font-variant-numeric: tabular-nums;
}
</style>
