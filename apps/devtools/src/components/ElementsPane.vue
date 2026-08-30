<script setup>
// The element tree: what the TARGET is actually rendering, straight off the
// `tree` command — the retained tree the renderer lowers into gpui, not a
// re-parse of anything. Every row is a real node id, which is what the styles
// panel, the highlight overlay and `eval` all address nodes by.
import { computed } from "vue";
import { state, refreshTree, select, toggle, togglePick, highlight } from "../store.js";

const rowStyle = (row) => ({ paddingLeft: `${6 + row.depth * 13}px` });

const classesOf = (node) => (node.classes || []).join(" ");

const textPreview = (node) => {
  const t = (node.text || "").trim();
  return t.length > 60 ? `${t.slice(0, 60)}…` : t;
};

const attrs = (node) => {
  const props_ = node.props || {};
  const keep = ["id", "href", "type", "value", "placeholder", "data-state", "aria-current"];
  return keep
    .filter((k) => props_[k] !== undefined && props_[k] !== "")
    .map((k) => `${k}="${String(props_[k]).slice(0, 24)}"`)
    .join(" ");
};

const count = computed(() => Object.keys(state.byId).length);

// Hovering a row in the inspector outlines the node in the target — the
// half of a browser's element panel people use without noticing it.
function peek(row) {
  if (state.picking || state.selected === row.id) return;
  highlight(row.id, 1200);
}
</script>

<template>
  <section class="pane">
    <header class="head">
      <span class="title">Elements</span>
      <span class="count">{{ count }} nodes</span>
      <div class="spacer" />
      <div
        class="btn"
        :class="{ on: state.picking }"
        @click="togglePick"
        title="hover the target app, then come back here — the last node under the pointer is selected"
      >
        {{ state.picking ? "picking…" : "pick" }}
      </div>
      <div class="btn" :class="{ on: state.autoTree }" @click="state.autoTree = !state.autoTree" title="re-read the tree every 2.5s">
        auto
      </div>
      <div class="btn" @click="refreshTree">refresh</div>
    </header>

    <div v-if="state.picking" class="hint">
      hover the target app, then move the pointer back here — whatever you left it on is selected
    </div>
    <div v-if="state.pickInfo && state.pickInfo.node" class="hint live">
      over &lt;{{ state.pickInfo.node.tag }}&gt; #{{ state.pickInfo.node.id }} at
      {{ Math.round(state.pickInfo.x) }},{{ Math.round(state.pickInfo.y) }}
    </div>
    <div v-if="state.treeError" class="error">{{ state.treeError }}</div>

    <div class="rows">
      <div
        v-for="row in state.rows"
        :key="row.id"
        class="row"
        :class="{ sel: state.selected === row.id, text: row.node.kind === 'text' }"
        :style="rowStyle(row)"
        @click="select(row.id)"
        @mouseenter="peek(row)"
      >
        <span v-if="row.children" class="twisty" @click.stop="toggle(row.id)">{{ row.open ? "▾" : "▸" }}</span>
        <span v-else class="twisty spacer2">·</span>

        <template v-if="row.node.kind === 'text'">
          <span class="txt">"{{ textPreview(row.node) }}"</span>
        </template>
        <template v-else>
          <span class="tag">&lt;{{ row.node.tag }}</span>
          <span v-if="classesOf(row.node)" class="cls">class="{{ classesOf(row.node) }}"</span>
          <span v-if="attrs(row.node)" class="attr">{{ attrs(row.node) }}</span>
          <span class="tag">&gt;</span>
          <span v-if="!row.open && textPreview(row.node)" class="txt">{{ textPreview(row.node) }}</span>
          <span v-if="row.truncated" class="more">+{{ row.truncated }} deeper</span>
        </template>
      </div>
      <div v-if="!state.rows.length" class="empty">
        {{ state.target ? "nothing mounted yet" : "attach to a target above" }}
      </div>
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
  border-right-width: 1px;
  border-color: #ffffff14;
}
.head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding-left: 10px;
  padding-right: 8px;
  height: 30px;
  background: #ffffff08;
  border-bottom-width: 1px;
  border-color: #ffffff14;
}
.title {
  font-size: 11px;
  color: #cfe1ff;
}
.count {
  font-size: 10px;
  color: #ffffff59;
}
.spacer {
  flex: 1;
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
  transition: background-color 120ms ease, color 120ms ease;
}
.btn:hover {
  background: #ffffff1a;
  color: #ffffff;
}
.btn.on {
  background: #1e4c8a;
  border-color: #6cb6ff;
  color: #ffffff;
}
.hint {
  font-size: 10px;
  color: #ffd479;
  padding: 5px;
  background: #ffd4791a;
}
.hint.live {
  color: #6cb6ff;
  background: #6cb6ff14;
}
.error {
  font-size: 10px;
  color: #ff7b72;
  padding: 5px;
  background: #ff7b721a;
}
.rows {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-top: 4px;
  padding-bottom: 8px;
}
.row {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 5px;
  height: 18px;
  font-size: 11px;
  font-family: Consolas, monospace;
  color: #c8d6e5;
  cursor: pointer;
}
.row:hover {
  background: #ffffff0f;
}
.row.sel {
  background: #1e4c8a;
}
.row.text {
  color: #9aa7b8;
}
.twisty {
  width: 10px;
  color: #ffffff70;
}
.spacer2 {
  color: #ffffff33;
}
.tag {
  color: #7ee787;
}
.cls {
  color: #ffd479;
}
.attr {
  color: #d2a8ff;
}
.txt {
  color: #9aa7b8;
}
.more {
  color: #ffffff59;
}
.empty {
  padding: 12px;
  font-size: 11px;
  color: #ffffff70;
}
</style>
