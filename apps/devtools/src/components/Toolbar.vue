<script setup lang="ts">
// The tab strip, the picker toggle, and the window switcher.
//
// Chrome puts three things in this row and so do we: the inspect toggle on the
// left (where the shortcut lives in muscle memory), the panels in the middle,
// and the state that changes what those panels are ABOUT on the right. Here the
// third one is not a device emulator, it is a WINDOW: one vue-native process
// can paint several OS windows off one retained tree (docs/MULTIWINDOW.md), so
// "which window" is a question every panel's answer depends on.
import { computed } from "vue";
import type { Tab } from "../protocol";
import { state, selectWindow, setTab, togglePick, toggleConsole } from "../store";

const TABS: { id: Tab; label: string; key: string }[] = [
  { id: "elements", label: "Elements", key: "1" },
  { id: "network", label: "Network", key: "2" },
  { id: "performance", label: "Performance", key: "3" },
  { id: "storage", label: "Storage", key: "4" },
  { id: "targets", label: "Targets", key: "5" },
];

/** Only worth showing when there is more than one — most apps have one window. */
const showWindows = computed(() => state.windows.length > 1);

const activeWindow = computed(() =>
  state.window === null ? state.windows.find((w) => w.kind === "main")?.id ?? null : state.window,
);

function windowLabel(id: number): string {
  const w = state.windows.find((x) => x.id === id);
  if (!w) return String(id);
  if (w.kind === "main") return "main";
  return w.title || `node ${w.root}`;
}

function pickWindow(id: number): void {
  const w = state.windows.find((x) => x.id === id);
  void selectWindow(w && w.kind === "main" ? null : id);
}

// Exchanges only: a reload marker is a divider in the list, not a request.
const netCount = computed(() => state.net.reduce((n, e) => (e.kind === "reload" ? n : n + 1), 0));
const errorCount = computed(() => state.logs.filter((l) => l.kind === "error").length);
</script>

<template>
  <div class="toolbar">
    <!-- Native picking consumes the click in the target before app dispatch. -->
    <button
      type="button"
      class="btn pick"
      :disabled="state.status !== 'attached'"
      :aria-pressed="state.picking"
      :class="{ on: state.picking }"
      title="Inspect an element  ·  Ctrl+Shift+C"
      @click="togglePick()"
    >
      <span class="glyph">⌖</span>
      <span>{{ state.picking ? "picking…" : "inspect" }}</span>
    </button>

    <div class="tabs" role="tablist" aria-label="Inspector panels">
      <button
        type="button"
        role="tab"
        :aria-selected="state.tab === tab.id"
        v-for="tab in TABS"
        :key="tab.id"
        class="tab"
        :class="{ on: state.tab === tab.id }"
        :title="`Ctrl+${tab.key}`"
        @click="setTab(tab.id)"
      >
        {{ tab.label }}
        <span v-if="tab.id === 'network' && netCount" class="count">{{ netCount }}</span>
        <span v-if="tab.id === 'elements' && errorCount" class="count bad">{{ errorCount }}</span>
      </button>
    </div>

    <div class="spacer"></div>

    <div v-if="showWindows" class="windows">
      <span class="label">window</span>
      <button
        type="button"
        v-for="w in state.windows"
        :key="w.id"
        class="win"
        :class="{ on: activeWindow === w.id, focused: w.focused }"
        :title="`${w.kind} · ${w.viewport.w}×${w.viewport.h} · id ${w.id}`"
        @click="pickWindow(w.id)"
      >
        {{ windowLabel(w.id) }}
      </button>
    </div>

    <button
      type="button"
      class="btn"
      :aria-pressed="state.consoleOpen"
      :class="{ on: state.consoleOpen }"
      title="Console drawer  ·  Esc"
      @click="toggleConsole()"
    >
      console
    </button>
  </div>
</template>

<style scoped>
button { font-family: inherit; background: transparent; border-width: 0; cursor: pointer; }
button:focus-visible { outline: 1px solid #6cb6ff; outline-offset: 1px; }
button:disabled { opacity: 0.45; cursor: default; }
.toolbar {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  height: 30px;
  padding-left: 8px;
  padding-right: 8px;
  background: #0d1219cc;
  border-bottom-width: 1px;
  border-color: #1e2733;
}
.pick {
  gap: 5px;
}
.glyph {
  font-size: 12px;
}
.tabs {
  display: flex;
  flex-direction: row;
  gap: 2px;
  margin-left: 4px;
}
.tab {
  height: 22px;
  padding-left: 9px;
  padding-right: 9px;
  border-radius: 4px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: #8494a8;
  transition: background-color 110ms ease, color 110ms ease;
}
.tab:hover {
  background: #171e29;
  color: #d7e0ea;
}
.tab.on {
  background: #16202e;
  color: #a8d4ff;
  border-bottom-width: 2px;
  border-color: #6cb6ff;
}
.count {
  font-size: 9px;
  color: #6d7c90;
  background: #1c2532;
  border-radius: 7px;
  padding-left: 4px;
  padding-right: 4px;
}
.count.bad {
  color: #ff9d96;
  background: #2a1512;
}
.spacer {
  flex: 1;
}
.windows {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
}
.win {
  height: 18px;
  padding-left: 7px;
  padding-right: 7px;
  border-radius: 9px;
  font-size: 10px;
  color: #8494a8;
  background: #141b26;
  border-width: 1px;
  border-color: #232c3a;
  display: flex;
  align-items: center;
}
.win:hover {
  color: #d7e0ea;
  border-color: #33415a;
}
.win.on {
  color: #a8d4ff;
  background: #14344f;
  border-color: #2f6ea8;
}
.win.focused {
  border-left-width: 3px;
  border-color: #56d364;
}
</style>
