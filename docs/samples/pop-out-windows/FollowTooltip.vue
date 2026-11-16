<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { appWindow, SkWindow, type SkWindowTarget } from "@skathi-native/runtime";

type Rect = { x: number; y: number; width: number; height: number };
type Overlay = { mapFromMain(target: SkWindowTarget | null): Rect | null };

const isWindows = navigator.platform === "Win32";
const tracks = Array.from({ length: 40 }, (_, i) => `Track ${i + 1}`);
const selected = ref<number | null>(null);
const list = ref<SkWindowTarget | null>(null);
const overlay = ref<Overlay | null>(null);
const tip = ref<Rect | null>(null);
const rows = new Map<number, SkWindowTarget>();
const label = computed(() => (selected.value === null ? "" : `${tracks[selected.value]}, 3:41`));

const rowRef = (i: number) => (el: unknown) => {
  if (el) rows.set(i, el as SkWindowTarget);
  else rows.delete(i);
};

function place() {
  const row = selected.value === null ? undefined : rows.get(selected.value);
  const r = row && overlay.value ? overlay.value.mapFromMain(row) : null;
  const box = overlay.value ? overlay.value.mapFromMain(list.value) : null;
  if (r && box && r.y + r.height > box.y && r.y < box.y + box.height) tip.value = r;
  else tip.value = null;
}

let frame = 0;
function schedule() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(place);
}

function select(i: number) {
  selected.value = selected.value === i ? null : i;
  schedule();
}

onMounted(() => {
  appWindow.addEventListener("move", schedule);
  window.addEventListener("resize", schedule);
});
onBeforeUnmount(() => {
  appWindow.removeEventListener("move", schedule);
  window.removeEventListener("resize", schedule);
  cancelAnimationFrame(frame);
});
</script>

<template>
  <div ref="list" class="list" @scroll="schedule">
    <div
      v-for="(t, i) in tracks"
      :key="t"
      :ref="rowRef(i)"
      class="row"
      :class="{ on: selected === i }"
      @click="select(i)"
    >
      {{ t }}
    </div>
  </div>
  <SkWindow
    v-if="isWindows"
    ref="overlay"
    class="layer"
    title="Track details"
    cover="monitor"
    chrome="hidden"
    backdrop="transparent"
    always-on-top
    ignore-mouse-events
    :focusable="false"
    :visible="tip !== null"
  >
    <div v-if="tip" class="tip" :style="{ left: `${tip.x + tip.width + 12}px`, top: `${tip.y}px` }">{{ label }}</div>
  </SkWindow>
</template>

<style>
.list {
  height: 100%;
  overflow-y: auto;
  padding: 8px;
}
.row {
  padding: 8px 12px;
  border-radius: 6px;
}
.row.on {
  background: #0090ff40;
}
.layer {
  position: relative;
}
.tip {
  position: absolute;
  padding: 8px 12px;
  border-radius: 8px;
  background: #1b1b1bf0;
  color: #ffffff;
  font-size: 12px;
}
</style>
