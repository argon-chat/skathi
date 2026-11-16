<script setup lang="ts">
import { computed, type Component } from "vue";
import { appWindow } from "@skathi-native/runtime";
import CloseSheet from "./components/CloseSheet.vue";
import CompactNotes from "./components/CompactNotes.vue";
import Sidebar from "./components/Sidebar.vue";
import TitleBar from "./components/TitleBar.vue";
import CursorLayer from "./demo/CursorLayer.vue";
import { demo } from "./demo/cursor";
import AlwaysOnTop from "./scenarios/AlwaysOnTop.vue";
import DragRegion from "./scenarios/DragRegion.vue";
import EdgeResize from "./scenarios/EdgeResize.vue";
import Follow from "./scenarios/Follow.vue";
import Fullscreen from "./scenarios/Fullscreen.vue";
import HideAndReturn from "./scenarios/HideAndReturn.vue";
import Overlay from "./scenarios/Overlay.vue";
import Palette from "./scenarios/Palette.vue";
import Reattach from "./scenarios/Reattach.vue";
import StateVerbs from "./scenarios/StateVerbs.vue";
import ThemeBackdrop from "./scenarios/ThemeBackdrop.vue";
import TitleBarOverlay from "./scenarios/TitleBarOverlay.vue";
import UnsavedChanges from "./scenarios/UnsavedChanges.vue";
import { compact, draft, nav } from "./store";

const pages: Record<string, Component> = {
  "windows/state-verbs": StateVerbs,
  "windows/always-on-top": AlwaysOnTop,
  "windows/theme-backdrop": ThemeBackdrop,
  "windows/fullscreen": Fullscreen,
  "title-bar/drag-region": DragRegion,
  "title-bar/overlay": TitleBarOverlay,
  "title-bar/edge-resize": EdgeResize,
  "pop-out/palette": Palette,
  "pop-out/follow": Follow,
  "pop-out/overlay": Overlay,
  "pop-out/reattach": Reattach,
  "closing/unsaved-changes": UnsavedChanges,
  "closing/hide-and-return": HideAndReturn,
};

const page = computed(() => pages[nav.id]);
const solid = computed(() => appWindow.effectiveBackdrop === "none");

try {
  const theme = localStorage.getItem("windows.theme");
  if (theme === "light" || theme === "dark") appWindow.theme = theme;
} catch {}
</script>

<template>
  <div class="app" :class="{ solid }">
    <CompactNotes v-if="compact.on" />
    <template v-else>
      <TitleBar />
      <div class="body">
        <Sidebar />
        <main class="layer">
          <component :is="page" :key="nav.key" />
        </main>
      </div>
    </template>
    <CloseSheet v-if="draft.asking" />
    <CursorLayer v-if="demo.enabled" />
  </div>
</template>

<style>
.sk-window-root {
  position: relative;
}
.app {
  height: 100%;
  display: flex;
  flex-direction: column;
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
  transition: background-color 200ms ease;
}
.app.solid {
  background: var(--solid);
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.layer {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  border-top-left-radius: 8px;
  border-top: 1px solid var(--layer-line);
  border-left: 1px solid var(--layer-line);
  background: var(--layer);
}
@media (display-mode: fullscreen) {
  .layer {
    border-top-left-radius: 0;
  }
}
</style>
