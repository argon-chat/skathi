<script setup lang="ts">
import { onMounted, ref } from "vue";

const canvas = ref<HTMLCanvasElement>();
let ctx: CanvasRenderingContext2D | null = null;
let last: PointerEvent | null = null;

onMounted(() => {
  const el = canvas.value!;
  el.width = el.clientWidth * devicePixelRatio;
  el.height = el.clientHeight * devicePixelRatio;
  ctx = el.getContext("2d");
  ctx?.scale(devicePixelRatio, devicePixelRatio);
});

// #region stroke
function down(e: PointerEvent) {
  e.preventDefault();
  canvas.value!.setPointerCapture(e.pointerId);
  last = e;
}

function move(e: PointerEvent) {
  if (!last || !ctx) return;
  const erasing = (e.buttons & 32) !== 0;
  ctx.globalCompositeOperation = erasing ? "destination-out" : "source-over";
  ctx.lineCap = "round";
  for (const r of e.getCoalescedEvents()) {
    ctx.lineWidth = erasing ? 24 : 1 + 8 * r.pressure;
    ctx.beginPath();
    ctx.moveTo(last.offsetX, last.offsetY);
    ctx.lineTo(r.offsetX, r.offsetY);
    ctx.stroke();
    last = r;
  }
}
// #endregion stroke
</script>

<template>
  <canvas
    ref="canvas"
    class="sketch"
    @pointerdown="down"
    @pointermove="move"
    @pointerup="last = null"
    @pointercancel="last = null"
    @contextmenu.prevent
  />
</template>

<style scoped>
.sketch {
  width: 100%;
  height: 400px;
  touch-action: none;
}
</style>
