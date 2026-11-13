<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import Titlebar from "./components/Titlebar.vue";
import Toolbar from "./components/Toolbar.vue";
import ElementsPanel from "./components/ElementsPanel.vue";
import StylesPanel from "./components/StylesPanel.vue";
import ConsolePanel from "./components/ConsolePanel.vue";
import NetworkPanel from "./components/NetworkPanel.vue";
import PerfPanel from "./components/PerfPanel.vue";
import StoragePanel from "./components/StoragePanel.vue";
import TargetsPanel from "./components/TargetsPanel.vue";
import ContextMenu from "./components/ContextMenu.vue";
import { onKeydown } from "./keys";
import { mainHeight, trackViewport, viewport } from "./layout";
import {
  attach,
  bestTarget,
  closeMenu,
  closeNetDetail,
  highlight,
  cancelPick,
  panelRequest,
  setCaptureActive,
  stopCapture,
  captureHeartbeat,
  resubscribe,
  pickStep,
  pollFallback,
  refreshMem,
  refreshTargets,
  refreshWindows,
  setTab,
  state,
  togglePick,
  ui,
} from "./store";

const SELECT_REFRESH_MS = 4000;
const request = computed(() => { void state.session; return panelRequest(); });

const mainH = computed(() =>
  mainHeight(state.consoleOpen, state.consoleHeight, !!state.error),
);

const dragFrom = ref(0);
const dragBase = ref(0);

function startDrag(which: "tree" | "console" | "net", event: VnMouseEvent): void {
  if (event.button !== 0) return;
  state.dragging = which;
  dragFrom.value = which === "console" ? event.clientY : event.clientX;
  dragBase.value =
    which === "tree" ? state.treeWidth : which === "net" ? state.netListWidth : state.consoleHeight;
}

function onDrag(event: VnMouseEvent): void {
  if (state.dragging === "tree") {
    const next = dragBase.value + (event.clientX - dragFrom.value);
    state.treeWidth = Math.max(220, Math.min(viewport.w - 280, next));
  } else if (state.dragging === "net") {
    const next = dragBase.value + (event.clientX - dragFrom.value);
    state.netListWidth = Math.max(220, Math.min(viewport.w - 320, next));
  } else if (state.dragging === "console") {
    const next = dragBase.value - (event.clientY - dragFrom.value);
    state.consoleHeight = Math.max(80, Math.min(viewport.h - 220, next));
  }
}

function endDrag(): void {
  state.dragging = null;
}

function onKey(event: VnKeyboardEvent): void {
  const ctrl = event.ctrlKey || event.metaKey;
  if (ctrl && event.shiftKey && event.key.toLowerCase() === "c") {
    event.preventDefault();
    togglePick();
    return;
  }
  if (ctrl && event.key.toLowerCase() === "f") {
    event.preventDefault();
    if (state.tab === "elements") ui.openElementSearch?.();
    return;
  }
  if (ctrl && event.key === "`") {
    event.preventDefault();
    state.consoleOpen = true;
    resubscribe();
    ui.focusPrompt?.();
    return;
  }
  if (ctrl && "12345".includes(event.key)) {
    event.preventDefault();
    setTab((["elements", "network", "performance", "storage", "targets"] as const)[
      Number(event.key) - 1
    ]);
    return;
  }
  if (event.key === "Escape") {
    if (state.picking) { event.preventDefault(); void cancelPick(); return; }
    if (ui.menu) {
      closeMenu();
      return;
    }
    if (ui.elementSearchOpen) {
      ui.closeElementSearch?.();
      return;
    }
    if (state.tab === "network" && state.netSelected !== null && !ui.typing) {
      closeNetDetail();
      return;
    }
    state.consoleOpen = !state.consoleOpen;
    resubscribe();
  }
}

let timers: ReturnType<typeof setInterval>[] = [];
const every = (ms: number, fn: () => void) => timers.push(setInterval(fn, ms));
let stopViewport: (() => void) | null = null;
let offKey: (() => void) | null = null;

onMounted(async () => {
  stopViewport = trackViewport();
  offKey = onKeydown(onKey);

  const found = await refreshTargets();
  const target = bestTarget(found);
  if (target) await attach(target);

  every(150, () => pickStep());
  every(3000, captureHeartbeat);
  every(3000, () => { void refreshTargets(); });
  every(SELECT_REFRESH_MS, () => {
    if (state.selected !== null && state.status === "attached" && !state.picking) {
      highlight(state.selected, SELECT_REFRESH_MS + 2000);
    }
  });
  // The window list only changes when an app opens or closes a pop-out, which is rare — so it is
  // refreshed while the panel that shows it is open, and once per attach otherwise.
  every(4000, () => {
    if (state.status === "attached" && (state.tab === "targets" || state.windows.length > 1)) {
      void refreshWindows();
    }
  });
  every(1000, () => pollFallback());
  // Heap/address-space walks are explicit snapshots, never a periodic UI task.

});

onUnmounted(() => {
  stopCapture();
  if (state.picking) void cancelPick();
  for (const t of timers) clearInterval(t);
  timers = [];
  offKey?.();
  offKey = null;
  stopViewport?.();
});
</script>

<template>
  <div class="app">
    <Titlebar />
    <Toolbar />

    <div v-if="state.error" class="banner">{{ state.error }}</div>

    <div class="main" :style="{ height: mainH + 'px' }">
      <template v-if="state.tab === 'elements'">
        <ElementsPanel :height="mainH" :style="{ width: state.treeWidth + 'px' }" />
        <div
          class="split-v"
          :class="{ dragging: state.dragging === 'tree' }"
          @mousedown="startDrag('tree', $event)"
        ></div>
        <StylesPanel :height="mainH" />
      </template>
      <NetworkPanel v-else-if="state.tab === 'network'" :height="mainH" @drag="startDrag('net', $event)" />
      <PerfPanel v-else-if="state.tab === 'performance'" :height="mainH" :request="request" :session-key="state.session" @recording-change="setCaptureActive" />
      <StoragePanel v-else-if="state.tab === 'storage'" :height="mainH" :request="request" :session-key="state.session" />
      <TargetsPanel v-else :height="mainH" />
    </div>

    <div
      v-if="state.consoleOpen"
      class="split-h"
      :class="{ dragging: state.dragging === 'console' }"
      @mousedown="startDrag('console', $event)"
    ></div>
    <ConsolePanel v-if="state.consoleOpen" :height="state.consoleHeight" />

    <footer class="statusbar mono">
      <span v-if="state.picking" class="mode">
        ⌖ click an element in the app · Esc to cancel
      </span>
      <span v-else-if="ui.flash" class="flash" :class="{ bad: ui.flashBad }">{{ ui.flash }}</span>
      <span v-else class="hint">
        Ctrl+Shift+C inspect · Ctrl+F find · Esc console · Ctrl+1…5 panels · Ctrl+` prompt · Ctrl+C copy row
      </span>
      <span class="spacer"></span>
      <span v-if="state.tab === 'elements' && state.selected !== null" class="fact"
        >node {{ state.selected }}</span
      >
      <span class="fact">{{ state.rows.length }} rows</span>
      <span class="fact" :class="{ live: state.pushLive }">{{
        state.pushLive ? "push" : "poll"
      }}</span>
    </footer>

    <div v-if="state.dragging" class="shield" @mousemove="onDrag" @mouseup="endDrag"></div>

    <ContextMenu />
  </div>
</template>

<style scoped>
/* The chrome (tabs, toolbars, buttons, labels) is not text to select, as in a browser's devtools:
   a drag across it must not paint a highlight. */
.app {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  background: #0a0e16d9;
  color: #d7e0ea;
  font-family: "Segoe UI", sans-serif;
  user-select: none;
}
.banner {
  height: 22px;
  padding-left: 12px;
  display: flex;
  align-items: center;
  font-size: 11px;
  color: #ff7b72;
  background: #ff7b7214;
  border-bottom-width: 1px;
  border-color: #ff7b7233;
}
.main {
  display: flex;
  flex-direction: row;
  min-height: 0;
}
.split-v {
  width: 5px;
  background: #0a0e16;
  border-left-width: 1px;
  border-right-width: 1px;
  border-color: #1e2733;
  cursor: col-resize;
}
.split-v:hover,
.split-v.dragging {
  background: #2f6ea8;
}
.split-h {
  height: 5px;
  background: #0a0e16;
  border-top-width: 1px;
  border-bottom-width: 1px;
  border-color: #1e2733;
  cursor: row-resize;
}
.split-h:hover,
.split-h.dragging {
  background: #2f6ea8;
}
.statusbar {
  height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  padding-left: 10px;
  padding-right: 10px;
  font-size: 10px;
  color: #5c6b80;
  background: #0a0e16;
  border-top-width: 1px;
  border-color: #1e2733;
}
.mode {
  color: #6cb6ff;
}
.flash {
  color: #7ee08a;
}
.flash.bad {
  color: #ff9d96;
}
.spacer {
  flex: 1;
}
.fact {
  color: #4e5b6d;
}
.fact.live {
  color: #56d364;
}
.shield {
  position: absolute;
  left: 0px;
  top: 0px;
  width: 100%;
  height: 100%;
  background: #00000001;
}
</style>
