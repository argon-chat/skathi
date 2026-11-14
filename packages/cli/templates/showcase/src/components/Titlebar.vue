<script setup lang="ts">
// sk.config.json sets "titleBarStyle": "hidden", so the app paints its own caption: `window-drag: move` makes the
// strip drag the window, and `--window-control` on each button gives it the OS behaviour (snap layouts on maximize).
import { appWindow } from "@skathi-native/runtime";
import { onMounted, onUnmounted, ref } from "vue";
import { productName } from "../product";

defineProps<{ page: string }>();

const hostControls = ref(appWindow.chrome.hostControls);
const maximized = ref(appWindow.isMaximized());
const read = () => {
  hostControls.value = appWindow.chrome.hostControls;
  maximized.value = appWindow.isMaximized();
};

onMounted(() => {
  navigator.windowControlsOverlay.addEventListener("geometrychange", read);
  appWindow.addEventListener("maximize", read);
  appWindow.addEventListener("unmaximize", read);
});
onUnmounted(() => {
  navigator.windowControlsOverlay.removeEventListener("geometrychange", read);
  appWindow.removeEventListener("maximize", read);
  appWindow.removeEventListener("unmaximize", read);
});
</script>

<template>
  <header class="titlebar">
    <div class="brand">
      <span class="mark">{{ productName.charAt(0) }}</span>
      <span class="name">{{ productName }}</span>
      <span class="sep">/</span>
      <span class="page">{{ page }}</span>
    </div>
    <div v-if="!hostControls" class="controls">
      <div class="control minimize" title="Minimize">
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
      </div>
      <div class="control maximize" :title="maximized ? 'Restore' : 'Maximize'">
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
          <path v-if="maximized" d="M2.5 2.5V.5h7v7h-2 M.5 2.5h7v7h-7z" stroke="currentColor" />
          <path v-else d="M.5 .5h9v9h-9z" stroke="currentColor" />
        </svg>
      </div>
      <div class="control close" title="Close">
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" stroke="currentColor" /></svg>
      </div>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  height: env(titlebar-area-height, 40px);
  padding-left: env(titlebar-area-x, 0px);
  display: flex;
  align-items: center;
  justify-content: space-between;
  window-drag: move;
  user-select: none;
  border-bottom: 1px solid var(--line);
}
@media (display-mode: fullscreen) {
  .titlebar {
    display: none;
  }
}
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-left: 14px;
  font-size: 12px;
}
.mark {
  width: 20px;
  height: 20px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, var(--accent), #b48cff);
  color: #ffffff;
  font-size: 11px;
  font-weight: 700;
}
.name {
  font-weight: 600;
  color: var(--text);
}
.sep {
  color: var(--text-faint);
}
.page {
  color: var(--text-muted);
}
.controls {
  height: 100%;
  display: flex;
  window-drag: none;
}
.control {
  width: 46px;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  transition:
    background-color 120ms ease,
    color 120ms ease;
}
.control:hover {
  background: var(--surface-strong);
  color: var(--text);
}
.minimize {
  --window-control: minimize;
}
.maximize {
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
