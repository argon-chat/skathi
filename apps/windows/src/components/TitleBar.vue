<script setup lang="ts">
import { appWindow } from "@skathi-native/runtime";
import { titleBar } from "../store";
import Icon from "./Icon.vue";

const TABS = ["Notes", "Canvas", "Export"];
</script>

<template>
  <header class="titlebar">
    <div class="brand">
      <span class="mark"><Icon name="window" /></span>
      <span class="name">Windows</span>
      <span v-if="!titleBar.tabs" class="sub">Skathi samples</span>
    </div>
    <div v-if="titleBar.tabs" class="tabs">
      <button
        v-for="tab in TABS"
        :key="tab"
        class="tab"
        :class="{ active: titleBar.tab === tab }"
        :data-demo="`tab-${tab}`"
        @click="titleBar.tab = tab"
      >
        {{ tab }}
      </button>
    </div>
    <div class="spacer" data-demo="titlebar-empty" />
    <div v-if="!appWindow.chrome.hostControls" class="controls">
      <div class="control min" data-demo="cap-min" title="Minimize">
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
      </div>
      <div class="control max" data-demo="cap-max" :title="appWindow.isMaximized() ? 'Restore' : 'Maximize'">
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
          <path v-if="appWindow.isMaximized()" d="M2.5 2.5V.5h7v7h-2 M.5 2.5h7v7h-7z" stroke="currentColor" />
          <path v-else d="M.5 .5h9v9h-9z" stroke="currentColor" />
        </svg>
      </div>
      <div class="control close" data-demo="cap-close" title="Close">
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" stroke="currentColor" /></svg>
      </div>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  height: env(titlebar-area-height, 40px);
  width: env(titlebar-area-width, 100%);
  display: flex;
  align-items: center;
  app-region: drag;
  user-select: none;
  flex-shrink: 0;
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
  padding-right: 16px;
}
.mark {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #b46fcc, #8a44a3);
  color: #ffffff;
}
.mark .icon {
  width: 14px;
  height: 14px;
}
.name {
  font-size: 12px;
  color: var(--text);
}
.sub {
  font-size: 12px;
  color: var(--text-3);
}
.tabs {
  height: 100%;
  display: flex;
  align-items: flex-end;
  gap: 2px;
}
.tab {
  height: 32px;
  min-width: 112px;
  padding: 0 14px;
  display: flex;
  align-items: center;
  border-radius: 8px 8px 0 0;
  color: var(--text-2);
  font-family: var(--font);
  font-size: 12px;
  app-region: no-drag;
  transition:
    background-color 120ms ease,
    color 120ms ease;
}
.tab:hover {
  background: var(--subtle-hover);
  color: var(--text);
}
.tab.active {
  background: var(--layer);
  color: var(--text);
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
  transition:
    background-color 100ms ease,
    color 100ms ease;
}
.control:hover {
  background: var(--subtle-hover);
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
