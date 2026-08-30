<script setup lang="ts">
// The inspector's own titlebar + the target picker.
//
// Same two CSS facts as apps/hello's titlebar (docs/WINDOWING.md §12):
// `window-drag: move` makes the strip an HTCAPTION region, `--window-control`
// makes the three buttons OS-behaved. The carve-out matters more here than
// there — the target dropdown lives INSIDE the drag strip, so it has to opt
// out with `window-drag: none` or clicking it would drag the window.
import { computed } from "vue";
import type { Target } from "../protocol";
import { state, attach, detach, refreshTargets, bestTarget } from "../store";

const emit = defineEmits(["reattach"]);

const label = (t: Target) => `${t.slot}${t.self ? " (this window)" : ""}${t.alive ? "" : " · dead"}`;

const statusText = computed(() => {
  if (state.status === "attached" && state.target) {
    return `attached · ${state.target.slot} · pid ${state.target.pid}`;
  }
  if (state.status === "attaching") return "attaching…";
  if (state.status === "lost") return "lost the target";
  return "not attached";
});

function rescan() {
  const found = refreshTargets();
  if (!state.target || state.status === "lost") {
    const next = bestTarget(found);
    if (next) attach(next);
  }
}
</script>

<template>
  <header class="titlebar">
    <span class="mark">◈</span>
    <span class="brand">vue-native devtools</span>

    <div class="picker">
      <div
        v-for="t in state.targets"
        :key="t.dir"
        class="target"
        :class="{ on: state.target && state.target.dir === t.dir, dead: !t.alive }"
        @click="attach(t)"
      >
        {{ label(t) }}
      </div>
      <div v-if="!state.targets.length" class="target empty">no dev host is running</div>
      <div class="btn" @click="rescan">rescan</div>
      <div v-if="state.target" class="btn" @click="detach">detach</div>
    </div>

    <span class="status" :class="state.status">{{ statusText }}</span>

    <div class="controls">
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
  gap: 10px;
  height: env(titlebar-area-height, 34px);
  width: env(titlebar-area-width, 100%);
  padding-left: env(titlebar-area-x, 0px);
  window-drag: move;
  user-select: none;
  background: #0b1220cc;
  border-bottom-width: 1px;
  border-color: #ffffff1a;
}
.mark {
  margin-left: 12px;
  font-size: 12px;
  color: #6cb6ff;
}
.brand {
  font-size: 12px;
  color: #ffffffb0;
}
.picker {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  /* Interactive furniture inside a drag region has to opt out, or a click on a
     target becomes a window drag. */
  window-drag: none;
}
.target {
  font-size: 11px;
  color: #cfe1ff;
  background: #ffffff14;
  padding-left: 8px;
  padding-right: 8px;
  padding-top: 3px;
  padding-bottom: 3px;
  border-radius: 5px;
  border-width: 1px;
  border-color: #ffffff00;
  cursor: pointer;
  transition: background-color 120ms ease, border-color 120ms ease;
}
.target:hover {
  background: #ffffff26;
}
.target.on {
  background: #1e4c8a;
  border-color: #6cb6ff;
  color: #ffffff;
}
.target.dead {
  color: #ffffff59;
}
.target.empty {
  background: #00000000;
  color: #ffffff70;
  cursor: default;
}
.btn {
  font-size: 11px;
  color: #ffffff99;
  padding-left: 7px;
  padding-right: 7px;
  padding-top: 3px;
  padding-bottom: 3px;
  border-radius: 5px;
  border-width: 1px;
  border-color: #ffffff26;
  cursor: pointer;
}
.btn:hover {
  background: #ffffff1a;
  color: #ffffff;
}
.status {
  flex: 1;
  text-align: right;
  padding-right: 10px;
  font-size: 11px;
  color: #ffffff70;
}
.status.attached {
  color: #7ee787;
}
.status.lost {
  color: #ff7b72;
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
  color: #ffffffcc;
  transition: background-color 120ms ease;
}
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
