<script setup lang="ts">
import { computed, type Component } from "vue";
import { appWindow } from "skathi:app";
import CursorLayer from "./CursorLayer.vue";
import { nav } from "./demo";
import CanvasPage from "./pages/CanvasPage.vue";
import FormControlsPage from "./pages/FormControlsPage.vue";
import IframePage from "./pages/IframePage.vue";
import LottiePage from "./pages/LottiePage.vue";
import StickersPage from "./pages/StickersPage.vue";
import SurfacePage from "./pages/SurfacePage.vue";
import TextFieldsPage from "./pages/TextFieldsPage.vue";
import VideoPage from "./pages/VideoPage.vue";

const pages: Record<string, Component> = {
  lottie: LottiePage,
  "sk-sticker": StickersPage,
  video: VideoPage,
  canvas: CanvasPage,
  "sk-surface": SurfacePage,
  iframe: IframePage,
  "text-fields": TextFieldsPage,
  "form-controls": FormControlsPage,
};
const page = computed(() => pages[nav.id]);
const solid = computed(() => appWindow.effectiveBackdrop === "none");
</script>

<template>
  <div class="app" :class="{ solid }">
    <header class="titlebar">
      <span class="mark">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <rect x="1.5" y="1.5" width="4.5" height="4.5" rx="1.2" fill="currentColor" />
          <rect x="8" y="1.5" width="4.5" height="4.5" rx="2.25" fill="currentColor" opacity=".75" />
          <rect x="1.5" y="8" width="4.5" height="4.5" rx="2.25" fill="currentColor" opacity=".75" />
          <rect x="8" y="8" width="4.5" height="4.5" rx="1.2" fill="currentColor" />
        </svg>
      </span>
      <span class="name">Elements</span>
      <span class="sub">Skathi samples</span>
      <div class="spacer" />
      <div class="controls">
        <div class="control min">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
        </div>
        <div class="control max">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <path d="M.5 .5h9v9h-9z" stroke="currentColor" />
          </svg>
        </div>
        <div class="control close">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" stroke="currentColor" /></svg>
        </div>
      </div>
    </header>
    <component :is="page" :key="nav.key" />
    <CursorLayer />
  </div>
</template>

<style>
.sk-window-root {
  position: relative;
}
.app {
  position: absolute;
  inset: 0;
  overflow: hidden;
  background: var(--tint);
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
}
.app.solid {
  background: var(--bg);
}
.titlebar {
  position: absolute;
  left: 0;
  top: 0;
  right: 0;
  height: 40px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding-left: 14px;
  app-region: drag;
  user-select: none;
  border-bottom: 1px solid #ffffff0a;
  z-index: 10;
}
.mark {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #f7a8d0, #c9a7f5);
  color: #2a1730;
}
.name {
  font-size: 12px;
  color: var(--text);
}
.sub {
  font-size: 12px;
  color: #ffffff70;
}
.spacer {
  flex: 1;
  height: 100%;
}
.controls {
  height: 100%;
  display: flex;
}
.control {
  width: 46px;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text);
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
</style>
