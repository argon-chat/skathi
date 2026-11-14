<script setup lang="ts">
import { computed, ref, onMounted, onBeforeUnmount } from "vue";
import { appWindow } from "@skathi-native/runtime";
import { state, currentWindow, setTab } from "../store";

const isMac = navigator!.platform.startsWith("Mac");
const isFullscreen = ref(appWindow.isFullscreen());
const onFullscreenChanged = () => {
  isFullscreen.value = appWindow.isFullscreen();
};
onMounted(() => appWindow.addEventListener("fullscreenchange", onFullscreenChanged));
onBeforeUnmount(() => appWindow.removeEventListener("fullscreenchange", onFullscreenChanged));

const attachedTo = computed(() => {
  if (state.status !== "attached" || !state.target) return null;
  return state.target;
});

const windowLabel = computed(() => {
  const w = currentWindow();
  if (!w) return "";
  return w.kind === "main" ? "main window" : `pop-out · node ${w.root}`;
});

const statusText = computed(() => {
  if (state.status === "attaching") return "attaching…";
  if (state.status === "lost") return "lost the target";
  if (state.status === "detached") return "not attached";
  return "attached";
});
</script>

<template>
  <header class="titlebar">
    <div v-if="isMac && !isFullscreen" class="native-controls" aria-hidden="true"></div>
    <span class="mark">◈</span>
    <span class="brand">devtools</span>

    <div class="what" @click="setTab('targets')">
      <span class="dot" :class="state.status"></span>
      <template v-if="attachedTo">
        <span class="slot mono">{{ attachedTo.slot }}</span>
        <span class="sep">·</span>
        <span class="pid mono">pid {{ attachedTo.pid }}</span>
        <span v-if="windowLabel" class="sep">·</span>
        <span v-if="windowLabel" class="where">{{ windowLabel }}</span>
      </template>
      <span v-else class="status" :class="state.status">{{ statusText }}</span>
    </div>

    <div class="spacer"></div>

    <div class="feed" :class="{ live: state.pushLive }" :title="state.pushNote">
      <span class="pulse"></span>
      <span>{{ state.pushLive ? "live" : "polling" }}</span>
    </div>

    <div v-if="!isMac" class="controls">
      <div class="ctl min" title="Minimize">–</div>
      <div class="ctl max" title="Maximize">□</div>
      <div class="ctl close" title="Close">✕</div>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  height: env(titlebar-area-height, var(--h-titlebar));
  width: env(titlebar-area-width, 100%);
  padding-left: env(titlebar-area-x, 0);
  window-drag: move;
  user-select: none;
  background: var(--surface-window);
  border-bottom-width: 1px;
  border-color: var(--line);
}
.native-controls {
  width: 70px;
  height: 100%;
  flex-shrink: 0;
  window-drag: none;
}
.mark {
  margin-left: var(--sp-5);
  font-size: var(--fs-m);
  color: var(--accent);
}
.brand {
  font-size: var(--fs-m);
  color: var(--text-muted);
}
.what {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
  height: var(--h-tab);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  border-radius: var(--r-pill);
  background: var(--surface-raised);
  border-width: 1px;
  border-color: var(--line);
  window-drag: none;
  cursor: pointer;
  transition:
    background-color 120ms ease,
    border-color 120ms ease;
}
.what:hover {
  background: var(--surface-hover);
  border-color: var(--line-strong);
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: var(--r-pill);
  background: var(--surface-handle);
}
.dot.attached {
  background: var(--ok);
}
.dot.attaching {
  background: var(--warn);
}
.dot.lost {
  background: var(--bad);
}
.slot {
  font-size: var(--fs-s);
  color: var(--text);
}
.pid,
.where {
  font-size: var(--fs-s);
  color: var(--text-muted);
}
.sep {
  font-size: var(--fs-s);
  color: var(--text-ghost);
}
.status {
  font-size: var(--fs-s);
  color: var(--text-muted);
}
.status.lost {
  color: var(--bad);
}
.spacer {
  flex: 1;
}
.feed {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-xs);
  color: var(--text-faint);
  padding-right: var(--sp-3);
  window-drag: none;
}
.feed.live {
  color: var(--ok);
}
.pulse {
  width: 5px;
  height: 5px;
  border-radius: var(--r-pill);
  background: var(--surface-handle);
}
.feed.live .pulse {
  background: var(--ok);
}
.controls {
  display: flex;
  flex-direction: row;
  height: 100%;
  window-drag: none;
}
.ctl {
  width: 44px;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-m);
  color: var(--text);
  transition: background-color 120ms ease;
}
.ctl:hover {
  background: var(--surface-active);
  color: var(--text-strong);
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
  background: var(--bad);
  color: var(--text-strong);
}
</style>
