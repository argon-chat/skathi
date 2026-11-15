<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, shallowRef, watch } from "vue";
import { fullWindow } from "../fullwindow";

// Layers over the whole window that take no pointer events, so the page under them works as usual. The
// pointer is read from the document instead, and eased here every frame into the effects' arguments.
const root = shallowRef<HTMLElement | null>(null);
const pointer = { x: 0.5, y: 0.5, at: 0, speed: 0 };
const eased = reactive({ x: 0.5, y: 0.5, near: 0, wind: 0, drift: 0 });

function move(e: MouseEvent) {
  const rect = root.value?.getBoundingClientRect();
  if (!rect || !rect.width || !rect.height) return;
  const now = performance.now();
  const x = (e.clientX - rect.left) / rect.width;
  if (pointer.at) pointer.speed = ((x - pointer.x) * rect.width) / Math.max(now - pointer.at, 8);
  pointer.x = x;
  pointer.y = (e.clientY - rect.top) / rect.height;
  pointer.at = now;
}

let frame = 0;
let last = 0;
function tick(now: number) {
  const dt = last ? Math.min(now - last, 50) : 16;
  last = now;
  const present = now - pointer.at < 2500 ? 1 : 0;
  eased.x += (pointer.x - eased.x) * 0.15;
  eased.y += (pointer.y - eased.y) * 0.15;
  eased.near += (present - eased.near) * 0.05;
  eased.wind += (pointer.speed - eased.wind) * 0.08;
  pointer.speed *= 0.9;
  eased.drift += eased.wind * dt * 0.6;
  frame = requestAnimationFrame(tick);
}

const running = computed(() => fullWindow.snow || fullWindow.fireflies);
watch(
  running,
  (on) => {
    if (on && !frame) {
      last = 0;
      frame = requestAnimationFrame(tick);
    } else if (!on && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
    }
  },
  { immediate: true },
);
onMounted(() => document.addEventListener("mousemove", move));
onUnmounted(() => {
  document.removeEventListener("mousemove", move);
  cancelAnimationFrame(frame);
});

const f = (n: number) => n.toFixed(4);
const snow = computed(
  () => `snow(1, ${f(eased.x)}, ${f(eased.y)}, ${eased.drift.toFixed(1)}px, ${(130 * eased.near).toFixed(1)}px, 0.6)`,
);
const fireflies = computed(() => `fireflies(1, 16px, ${f(eased.x)}, ${f(eased.y)}, ${f(eased.near)})`);
const shatter = computed(() => {
  const rect = root.value?.getBoundingClientRect();
  const x = rect?.width ? (fullWindow.impact.x - rect.left) / rect.width : 0.5;
  const y = rect?.height ? (fullWindow.impact.y - rect.top) / rect.height : 0.5;
  return `shatter(${f(fullWindow.shatter ?? 0)}, ${f(x)}, ${f(y)}, 110px, #050508)`;
});
</script>

<template>
  <div ref="root" class="full">
    <div v-if="fullWindow.fireflies" class="layer" :style="{ '--shading': fireflies }" />
    <div v-if="fullWindow.snow" class="layer" :style="{ '--shading': snow }" />
    <div v-if="fullWindow.shatter !== null" class="layer" :style="{ '--shading': shatter }" />
  </div>
</template>

<style scoped>
.full,
.layer {
  position: absolute;
  left: 0;
  top: 0;
  right: 0;
  bottom: 0;
  pointer-events: none;
}
.full {
  z-index: 100;
}
</style>
