<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, shallowRef, watch } from "vue";
import { fullWindow } from "../fullwindow";

// Layers over the whole window that take no pointer events, so the page under them works as usual. The
// pointer is read from the document instead, and eased here every frame into the effects' arguments.
const root = shallowRef<HTMLElement | null>(null);
const pointer = { x: 0.5, y: 0.5, at: 0 };
const eased = reactive({ x: 0.5, y: 0.5, near: 0 });

function move(e: MouseEvent) {
  const rect = root.value?.getBoundingClientRect();
  if (!rect || !rect.width || !rect.height) return;
  pointer.x = (e.clientX - rect.left) / rect.width;
  pointer.y = (e.clientY - rect.top) / rect.height;
  pointer.at = performance.now();
}

// The last four clicks while ripples are on. The effect takes each as its position packed into one
// number (see shaders/ripples.slang) and its age.
const RING_MS = 3000;
const drops: { packed: number; at: number }[] = [];
const ripples = shallowRef<string | null>(null);

function drop(e: MouseEvent) {
  const rect = root.value?.getBoundingClientRect();
  if (!fullWindow.ripples || !rect || !rect.width || !rect.height) return;
  const unit = (v: number) => Math.round(Math.min(Math.max(v, 0), 1) * 2047);
  const packed = unit((e.clientX - rect.left) / rect.width) * 2048 + unit((e.clientY - rect.top) / rect.height);
  drops.push({ packed, at: performance.now() });
  if (drops.length > 4) drops.shift();
  wake();
}

let frame = 0;
function tick() {
  const now = performance.now();
  const present = now - pointer.at < 2500 ? 1 : 0;
  eased.x += (pointer.x - eased.x) * 0.08;
  eased.y += (pointer.y - eased.y) * 0.08;
  eased.near += (present - eased.near) * 0.04;

  while (drops.length && now - drops[0]!.at > RING_MS) drops.shift();
  const rings = [0, 1, 2, 3].map((i) => {
    const d = drops[i];
    return d ? `${d.packed}, ${((now - d.at) / 1000).toFixed(3)}` : "0, -1";
  });
  ripples.value = drops.length ? `ripples(${rings.join(", ")})` : null;

  frame = fullWindow.snow || fullWindow.fireflies || drops.length ? requestAnimationFrame(tick) : 0;
}
const wake = () => {
  if (!frame) frame = requestAnimationFrame(tick);
};

watch(
  () => fullWindow.snow || fullWindow.fireflies,
  (on) => on && wake(),
  { immediate: true },
);
onMounted(() => {
  document.addEventListener("mousemove", move);
  document.addEventListener("mousedown", drop);
});
onUnmounted(() => {
  document.removeEventListener("mousemove", move);
  document.removeEventListener("mousedown", drop);
  cancelAnimationFrame(frame);
});

const f = (n: number) => n.toFixed(4);
const snow = computed(() => `snow(1, ${f(eased.x)}, ${f(eased.y)}, ${(90 * eased.near).toFixed(1)}px, 0.6)`);
const fireflies = computed(() => `fireflies(1, 16px, ${f(eased.x)}, ${f(eased.y)}, ${f(0.5 * eased.near)})`);
const shatter = computed(() => {
  const rect = root.value?.getBoundingClientRect();
  const x = rect?.width ? (fullWindow.impact.x - rect.left) / rect.width : 0.5;
  const y = rect?.height ? (fullWindow.impact.y - rect.top) / rect.height : 0.5;
  return `shatter(${f(fullWindow.shatter ?? 0)}, ${f(x)}, ${f(y)}, 110px)`;
});
</script>

<template>
  <div ref="root" class="full">
    <div v-if="ripples" class="layer" :style="{ '--shading': ripples }" />
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
