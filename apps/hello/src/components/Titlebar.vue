<script setup lang="ts">
// `--window-control: <role>` on each button → the OS answers WM_NCHITTEST with
// HTMIN/HTMAX/HTCLOSE, which is what buys the Windows 11 SNAP LAYOUTS flyout on hover over the
// maximize button.
import { computed, onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "vue-native";

const hostControls = ref(Boolean(appWindow.chrome.hostControls));
const maximized = ref(appWindow.isMaximized());
// engine.d.ts types `document` as `VnDocument | undefined` because the bun test harness has none.
const doc = document!;
const wco = navigator!.windowControlsOverlay;
const title = ref(doc.title);

const MIN = "–";
const CLOSE = "✕";
const maxGlyph = computed(() => (maximized.value ? "❐" : "□"));

const readGeometry = () => {
  hostControls.value = Boolean(appWindow.chrome.hostControls);
};
const readState = () => {
  maximized.value = appWindow.isMaximized();
  title.value = doc.title;
};

onMounted(() => {
  readGeometry();
  wco.addEventListener("geometrychange", readGeometry);
  appWindow.addEventListener("maximize", readState);
  appWindow.addEventListener("unmaximize", readState);
  // `document.title` has no change event in any spec (it is a plain property), so the editor on
  // the Window page pokes this custom one.
  globalThis.addEventListener("vn:title", readState);
});
onUnmounted(() => {
  wco.removeEventListener("geometrychange", readGeometry);
  appWindow.removeEventListener("maximize", readState);
  appWindow.removeEventListener("unmaximize", readState);
  globalThis.removeEventListener("vn:title", readState);
});
</script>

<template>
  <header class="titlebar">
    <span class="mark">◆</span>
    <span class="title">{{ title }}</span>
    <div v-if="!hostControls" class="controls">
      <div class="ctl min" title="Minimize">{{ MIN }}</div>
      <div class="ctl max" :title="maximized ? 'Restore' : 'Maximize'">{{ maxGlyph }}</div>
      <div class="ctl close" title="Close">{{ CLOSE }}</div>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  /* Same host-side rect the JS getter reads; the fallbacks are what a browser without WCO would
     use. */
  height: env(titlebar-area-height, 36px);
  width: env(titlebar-area-width, 100%);
  /* Zero on Windows (buttons right); non-zero on macOS, where the traffic lights sit left. */
  padding-left: env(titlebar-area-x, 0px);
  /* Inherited, so every descendant is draggable until one says otherwise. */
  window-drag: move;
  user-select: none;
  background: #ffffff0a;
  border-bottom-width: 1px;
  border-color: #ffffff1f;
}
.mark {
  margin-left: 12px;
  font-size: 12px;
  color: #7aa2ff;
}
.title {
  flex: 1;
  font-size: 12px;
  color: #ffffffb0;
}
.controls {
  display: flex;
  flex-direction: row;
  height: 100%;
  padding-left: 10px;
  window-drag: none;
}
.ctl {
  width: 46px;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  color: #ffffffcc;
  transition: background-color 120ms ease, color 120ms ease;
}
/* Ours is painted by this stylesheet, off the NC mouse-move gpui forwards into the content tree. */
.ctl:hover {
  background: #ffffff1a;
}
.min {
  --window-control: minimize;
}
.max {
  --window-control: maximize;
}
.close {
  --window-control: close;
}
.close:hover {
  background: #c42b1c;
  color: #ffffff;
}
</style>
