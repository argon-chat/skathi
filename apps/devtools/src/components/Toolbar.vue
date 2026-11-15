<script setup lang="ts">
import { computed } from "vue";
import type { Tab } from "../protocol";
import { state, selectWindow, setTab, togglePick, toggleConsole } from "../store";

const TABS: { id: Tab; label: string; key: string }[] = [
  { id: "elements", label: "Elements", key: "1" },
  { id: "network", label: "Network", key: "2" },
  { id: "performance", label: "Performance", key: "3" },
  { id: "storage", label: "Storage", key: "4" },
  { id: "targets", label: "Targets", key: "5" },
  { id: "effects", label: "Effects", key: "6" },
];

const showWindows = computed(() => state.windows.length > 1);

const activeWindow = computed(() =>
  state.window === null ? (state.windows.find((w) => w.kind === "main")?.id ?? null) : state.window,
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
button {
  font-family: inherit;
  background: transparent;
  border-width: 0;
  cursor: pointer;
}
button:focus-visible {
  outline: 1px solid var(--line-focus);
  outline-offset: 1px;
}
button:disabled {
  opacity: 0.45;
  cursor: default;
}
.toolbar {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  height: var(--h-toolbar);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  background: var(--surface-panel);
  border-bottom-width: 1px;
  border-color: var(--line);
}
.pick {
  gap: var(--sp-2);
}
.glyph {
  font-size: var(--fs-m);
}
.tabs {
  display: flex;
  flex-direction: row;
  gap: var(--sp-1);
  margin-left: var(--sp-2);
}
.tab {
  height: var(--h-tab);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  border-radius: var(--r-m);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-s);
  color: var(--text-muted);
  transition:
    background-color 110ms ease,
    color 110ms ease;
}
.tab:hover {
  background: var(--surface-raised);
  color: var(--text);
}
.tab.on {
  background: var(--surface-hover);
  color: var(--accent);
  border-bottom-width: 2px;
  border-color: var(--accent);
}
.count {
  font-size: var(--fs-2xs);
  color: var(--text-faint);
  background: var(--surface-hover);
  border-radius: var(--r-l);
  padding-left: var(--sp-2);
  padding-right: var(--sp-2);
}
.count.bad {
  color: var(--bad);
  background: var(--bad-soft);
}
.spacer {
  flex: 1;
}
.windows {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
}
.win {
  height: var(--h-chip);
  padding-left: var(--sp-3);
  padding-right: var(--sp-3);
  border-radius: var(--r-pill);
  font-size: var(--fs-xs);
  color: var(--text-muted);
  background: var(--surface-raised);
  border-width: 1px;
  border-color: var(--line);
  display: flex;
  align-items: center;
}
.win:hover {
  color: var(--text);
  border-color: var(--line-strong);
}
.win.on {
  color: var(--accent);
  background: var(--accent-soft);
  border-color: var(--accent-line);
}
.win.focused {
  border-left-width: 3px;
  border-color: var(--ok);
}
</style>
