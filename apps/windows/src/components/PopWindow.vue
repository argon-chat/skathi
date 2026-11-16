<script setup lang="ts">
// <SkWindow> plus the demo cursor's layer for that window. Props and listeners pass straight through.
import { computed, ref } from "vue";
import { SkWindow, type SkWindowTarget } from "@skathi-native/runtime";
import CursorLayer from "../demo/CursorLayer.vue";
import { demo, mainLayer, type Point } from "../demo/cursor";

defineOptions({ inheritAttrs: false });
defineProps<{ noCursor?: boolean }>();

interface Handle {
  el: SkWindowTarget | null;
  windowId: number;
  mapFromMain(target: SkWindowTarget | null | undefined): SkWindowRect | null;
}
const win = ref<Handle | null>(null);

// gpui opens a window created visible on the primary display whenever its `bounds` center lies
// on another monitor. A window created hidden is placed by Skathi at its first show, on any
// monitor, so the window opens hidden and shows once it exists.
const opened = ref(false);

const mapFromMain = (target: unknown): SkWindowRect | null =>
  win.value?.mapFromMain(target as SkWindowTarget | null) ?? null;

const offset = (): Point | null => {
  const r = mapFromMain(mainLayer.value);
  return r ? { x: r.x, y: r.y } : null;
};

defineExpose({ mapFromMain, el: computed(() => win.value?.el ?? null) });
</script>

<template>
  <SkWindow ref="win" v-bind="$attrs" :visible="opened && $attrs.visible !== false" @open="opened = true">
    <slot />
    <CursorLayer v-if="demo.enabled && !noCursor" :offset="offset" />
  </SkWindow>
</template>
