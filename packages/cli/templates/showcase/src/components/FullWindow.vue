<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef } from "vue";
import { fullWindow } from "../fullwindow";

// Layers over the whole window that take no pointer events, so the page under them works as usual.
// Snow and fireflies get the pointer from the host; ripples are clicks read from the document.
const root = shallowRef<HTMLElement | null>(null);

// The last eight clicks while ripples are on, each where it landed (0..1 across the window) and when,
// on the clock the effect reads (`performance.now()` in seconds). The effect ages them itself.
const RINGS = 8;
const RING_MS = 3000;
type Drop = { x: number; y: number; born: number };
const drops = shallowRef<Drop[]>([]);
let prune: ReturnType<typeof setTimeout> | undefined;

function drop(e: MouseEvent) {
  const rect = root.value?.getBoundingClientRect();
  if (!fullWindow.ripples || !rect || !rect.width || !rect.height) return;
  const at = { x: (e.clientX - rect.left) / rect.width, y: (e.clientY - rect.top) / rect.height };
  drops.value = [...drops.value, { ...at, born: performance.now() / 1000 }].slice(-RINGS);
  clearTimeout(prune);
  prune = setTimeout(() => (drops.value = []), RING_MS);
}

const f = (n: number) => n.toFixed(4);
const ripples = computed(() => {
  if (!drops.value.length) return null;
  const ring = (i: number) => drops.value[i];
  const at = Array.from({ length: RINGS }, (_, i) => `${f(ring(i)?.x ?? 0)}, ${f(ring(i)?.y ?? 0)}`);
  const born = Array.from({ length: RINGS }, (_, i) => (ring(i)?.born ?? -1000).toFixed(3));
  return `ripples(${[...at, ...born].join(", ")})`;
});

onMounted(() => document.addEventListener("mousedown", drop));
onUnmounted(() => {
  document.removeEventListener("mousedown", drop);
  clearTimeout(prune);
});

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
    <div v-if="fullWindow.fireflies" class="layer fireflies" />
    <div v-if="fullWindow.snow" class="layer snow" />
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
.snow {
  --shading: snow(1, 90px, 0.6);
}
.fireflies {
  --shading: fireflies(1, 16px, 0.5);
}
</style>
