<script setup lang="ts">
// The console: the target's log ring, tailed, plus an `eval` prompt that runs
// in the target's JSC in global scope.
//
// NEWEST FIRST, deliberately. A browser console scrolls itself to the bottom;
// nothing here can set `scrollTop` on a host element yet, so the newest line
// would sink out of view the moment the ring moved. Reversing the list is the
// honest fix rather than a scroll hack that would silently stop working.
//
// The ring carries more than `console.*`: `[js uncaught]`, CSS diagnostics and
// compat advisories are in it too (docs/DEVTOOLS.md), which is why the filter
// row has a kind next to it.
import { computed, ref } from "vue";
import { state, runEval, clearConsole, capture } from "../store";

const source = ref("");
const history = ref<string[]>([]);
const historyAt = ref(-1);

// Assembled rather than written out, because the builder scans source text for
// `require("…")` to build the module graph — a literal one in a placeholder
// string is a module it then cannot resolve, and the build says so.
const PROMPT_HINT = `evaluate in the target — document.title, or __VN__.${"require"}("<module id>")`;

/** The one-character gutter mark per log kind. A kind the host adds later
    that is not in here falls through to the empty string, which is the
    honest rendering of "no mark for this one" — hence the index signature
    rather than a closed union the host does not promise. */
const KINDS: Record<string, string> = {
  input: "›",
  result: "‹",
  error: "!",
  warn: "!",
  advisory: "?",
  css: "#",
  dev: "·",
};

const shown = computed(() => {
  const needle = state.logFilter.trim().toLowerCase();
  const rows = needle
    ? state.logs.filter((e) => `${e.kind} ${e.text}`.toLowerCase().includes(needle))
    : state.logs;
  return rows.slice(-300).reverse();
});

function submit() {
  const src = source.value;
  if (!src.trim()) return;
  history.value.push(src);
  historyAt.value = history.value.length;
  runEval(src);
  source.value = "";
}

// Up/Down through what has been evaluated, like every console ever.
function recall(step: number) {
  if (!history.value.length) return;
  const next = Math.min(history.value.length, Math.max(0, historyAt.value + step));
  historyAt.value = next;
  source.value = next === history.value.length ? "" : history.value[next];
}
</script>

<template>
  <section class="pane">
    <header class="head">
      <span class="title">Console</span>
      <span v-if="state.logStatus" class="status">{{ state.logStatus }}</span>
      <span v-if="state.dropped" class="dropped">{{ state.dropped }} dropped</span>
      <div class="spacer" />
      <input v-model="state.logFilter" class="filter" placeholder="filter" />
      <div class="btn" @click="capture" :title="'capture the target window'">
        {{ state.shotBusy ? "…" : "shot" }}
      </div>
      <div class="btn" @click="clearConsole">clear</div>
    </header>

    <div class="rows">
      <div v-for="(entry, i) in shown" :key="entry.seq + '-' + i" class="row" :class="entry.kind">
        <span class="glyph">{{ KINDS[entry.kind] || "·" }}</span>
        <span class="kind">{{ entry.kind }}</span>
        <span class="text">{{ entry.text }}</span>
      </div>
      <div v-if="!shown.length" class="empty">
        {{ state.target ? "nothing logged yet" : "attach to a target to tail its log ring" }}
      </div>
    </div>

    <div class="prompt">
      <span class="caret">›</span>
      <input
        v-model="source"
        class="input"
        :placeholder="PROMPT_HINT"
        @keydown.enter="submit"
        @keydown.up="recall(-1)"
        @keydown.down="recall(1)"
      />
    </div>
  </section>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  /* NO `height: 100%` here: the SHELL sizes the panes (App.vue), and a
     `height` on a component root would collide with the parent's rule at the
     same specificity — whichever module the builder emitted last would win,
     which is not a layout, it is a coin flip. */
  min-height: 0;
  border-top-width: 1px;
  border-color: #ffffff14;
}
.head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding-left: 10px;
  padding-right: 8px;
  height: 28px;
  background: #ffffff08;
}
.title {
  font-size: 11px;
  color: #cfe1ff;
}
.status {
  font-size: 10px;
  color: #ffffff59;
}
.dropped {
  font-size: 10px;
  color: #ffd479;
}
.spacer {
  flex: 1;
}
.filter,
.input {
  font-size: 11px;
  font-family: Consolas, monospace;
  color: #e6edf3;
  background: #ffffff0f;
  border-width: 1px;
  border-color: #ffffff26;
  border-radius: 4px;
  padding-left: 6px;
  padding-right: 6px;
  padding-top: 2px;
  padding-bottom: 2px;
}
.filter {
  width: 120px;
}
.filter:focus,
.input:focus {
  border-color: #6cb6ff;
}
.btn {
  font-size: 10px;
  color: #ffffff99;
  padding-left: 7px;
  padding-right: 7px;
  padding-top: 2px;
  padding-bottom: 2px;
  border-radius: 4px;
  border-width: 1px;
  border-color: #ffffff26;
  cursor: pointer;
}
.btn:hover {
  background: #ffffff1a;
  color: #ffffff;
}
.rows {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 6px;
  padding-left: 8px;
  padding-right: 8px;
  padding-top: 1px;
  padding-bottom: 1px;
  font-size: 11px;
  font-family: Consolas, monospace;
  color: #c8d6e5;
}
.row:hover {
  background: #ffffff08;
}
.glyph {
  width: 8px;
  color: #ffffff59;
}
.kind {
  width: 62px;
  color: #ffffff45;
}
.text {
  flex: 1;
}
.row.error .text {
  color: #ff7b72;
}
.row.warn .text {
  color: #ffd479;
}
.row.advisory .text {
  color: #ffd479;
}
.row.css .text {
  color: #d2a8ff;
}
.row.input .text {
  color: #6cb6ff;
}
.row.result .text {
  color: #7ee787;
}
.row.dev .text {
  color: #ffffff70;
}
.empty {
  padding: 10px;
  font-size: 11px;
  color: #ffffff70;
}
.prompt {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  padding-left: 8px;
  padding-right: 8px;
  padding-top: 4px;
  padding-bottom: 4px;
  border-top-width: 1px;
  border-color: #ffffff14;
}
.caret {
  font-size: 12px;
  color: #6cb6ff;
}
.input {
  flex: 1;
}
</style>
