<script setup lang="ts">
import { computed, ref, onMounted, onBeforeUnmount } from "vue";
import { appWindow } from "vue-native";
import { state, currentWindow, setTab } from "../store";

const isMac = navigator.platform.startsWith("Mac");
const isFullscreen = ref(appWindow.isFullscreen());
const onFullscreenChanged = () => { isFullscreen.value = appWindow.isFullscreen(); };
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
  gap: 8px;
  height: env(titlebar-area-height, 34px);
  width: env(titlebar-area-width, 100%);
  padding-left: env(titlebar-area-x, 0px);
  window-drag: move;
  user-select: none;
  background: #0a0e16cc;
  border-bottom-width: 1px;
  border-color: #1e2733;
}
.native-controls {
  width: 70px;
  height: 100%;
  flex-shrink: 0;
  window-drag: none;
}
.mark {
  margin-left: 11px;
  font-size: 12px;
  color: #6cb6ff;
}
.brand {
  font-size: 12px;
  color: #9fb0c4;
}
.what {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding-left: 8px;
  padding-right: 9px;
  border-radius: 11px;
  background: #141b26;
  border-width: 1px;
  border-color: #232c3a;
  window-drag: none;
  cursor: pointer;
  transition: background-color 120ms ease, border-color 120ms ease;
}
.what:hover {
  background: #1a2330;
  border-color: #33415a;
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: 3px;
  background: #5c6b80;
}
.dot.attached {
  background: #56d364;
}
.dot.attaching {
  background: #e3b341;
}
.dot.lost {
  background: #ff7b72;
}
.slot {
  font-size: 11px;
  color: #d7e0ea;
}
.pid,
.where {
  font-size: 11px;
  color: #8494a8;
}
.sep {
  font-size: 11px;
  color: #3c4859;
}
.status {
  font-size: 11px;
  color: #8494a8;
}
.status.lost {
  color: #ff7b72;
}
.spacer {
  flex: 1;
}
.feed {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 5px;
  font-size: 10px;
  color: #6d7c90;
  padding-right: 6px;
  window-drag: none;
}
.feed.live {
  color: #7ee08a;
}
.pulse {
  width: 5px;
  height: 5px;
  border-radius: 3px;
  background: #46556a;
}
.feed.live .pulse {
  background: #56d364;
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
  font-size: 12px;
  color: #b9c6d6;
  transition: background-color 120ms ease;
}
.ctl:hover {
  background: #ffffff14;
  color: #ffffff;
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
