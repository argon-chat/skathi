<script setup lang="ts">
import { ref } from "vue";
import type { SkWindowBounds } from "@skathi-native/runtime";
import { appWindow } from "skathi:app";
import { compact, leaveCompact } from "../store";
import Icon from "./Icon.vue";
import PopWindow from "./PopWindow.vue";

const referenceBounds = ref<SkWindowBounds>({ width: 520, height: 340 });

function toggleReference() {
  if (!compact.reference) {
    const note = appWindow.getContentBounds();
    referenceBounds.value = { x: note.x - 420, y: note.y + 70, width: 540, height: 340 };
  }
  compact.reference = !compact.reference;
}
const togglePin = () => (appWindow.alwaysOnTop = !appWindow.alwaysOnTop);
</script>

<template>
  <div class="notes">
    <header class="bar">
      <span class="title">Notes</span>
      <button class="pin" :class="{ on: appWindow.alwaysOnTop }" data-demo="pin" @click="togglePin">
        <Icon name="pin" />
        {{ appWindow.alwaysOnTop ? "Pinned" : "Pin" }}
      </button>
      <button class="icon-btn" title="Back to the gallery" data-demo="expand" @click="leaveCompact">
        <Icon name="fullscreen" />
      </button>
    </header>
    <textarea v-model="compact.note" class="text" data-demo="note-text" />
    <footer class="foot">
      <span class="hint">{{ appWindow.alwaysOnTop ? "Stays above other windows" : "Not pinned" }}</span>
      <button class="btn" data-demo="reference" @click="toggleReference">
        {{ compact.reference ? "Hide reference" : "Open reference" }}
      </button>
    </footer>

    <PopWindow
      v-if="compact.reference"
      title="Reference: window guides"
      backdrop="acrylic"
      chrome="hidden"
      :bounds="referenceBounds"
      :title-bar-height="36"
      @close="compact.reference = false"
    >
      <article class="ref">
        <header class="ref-bar">
          <Icon name="doc" />
          <span>window-guides.md</span>
        </header>
        <div class="ref-body">
          <h2 class="ref-title">Always on top</h2>
          <p>
            appWindow.alwaysOnTop keeps the window above other apps' windows without taking focus. Set "alwaysOnTop":
            true in the window block of sk.config.json to start pinned.
          </p>
          <p>
            On macOS and Linux the setter has no effect on the window, yet the getter returns what you set. Hide the
            control on those platforms.
          </p>
        </div>
      </article>
    </PopWindow>
  </div>
</template>

<style scoped>
.notes {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--layer);
}
.bar {
  height: env(titlebar-area-height, 40px);
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 6px 0 16px;
  app-region: drag;
  user-select: none;
  flex-shrink: 0;
}
.title {
  flex: 1;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.pin {
  height: 28px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  gap: 6px;
  border-radius: 5px;
  background: var(--control);
  border: 1px solid var(--control-line);
  color: var(--text);
  font-family: var(--font);
  font-size: 12px;
  app-region: no-drag;
  transition: background-color 120ms ease;
}
.pin:hover,
.pin.demo-hover {
  background: var(--control-hover);
}
.pin.on {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--on-accent);
}
.icon-btn {
  width: 32px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  color: var(--text-2);
  app-region: no-drag;
}
.icon-btn:hover {
  background: var(--subtle-hover);
}
.text {
  flex: 1;
  margin: 4px 12px 0;
  padding: 12px 14px;
  border-radius: 6px;
  border: 1px solid var(--control-line);
  background: var(--card);
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
  line-height: 22px;
}
.foot {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px;
}
.hint {
  flex: 1;
  font-size: 12px;
  color: var(--text-3);
}
.ref {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--popout);
  color: var(--text);
  font-family: var(--font);
}
.ref-bar {
  height: 36px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 14px;
  font-size: 12px;
  color: var(--text-2);
  border-bottom: 1px solid var(--card-line);
  app-region: drag;
  flex-shrink: 0;
}
.ref-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px 24px;
  font-size: 14px;
  line-height: 22px;
  color: var(--text-2);
}
.ref-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: 20px;
  font-weight: 600;
  color: var(--text);
}
.ref-body p {
  margin: 0;
}
</style>
