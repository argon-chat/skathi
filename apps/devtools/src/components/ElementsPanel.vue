<script setup lang="ts">
// The element tree: a windowed list, a search, breadcrumbs, and the keyboard.
//
// Three things v1 did not have, in the order they were missed:
//
//   1. KEYBOARD. Up/Down walk the visible rows, Left/Right collapse and expand
//      (Left on a closed row goes to the parent, exactly as every tree in every
//      IDE does), Enter re-arms the outline in the target. It is the difference
//      between reading a tree and using one.
//   2. SEARCH. Ctrl+F, then Enter / Shift+Enter to step. It searches the whole
//      tree — not the visible rows — and opens whatever is collapsed over a
//      hit, because a search that only finds what you have already expanded is
//      a filter, not a search.
//   3. SCROLLING THAT FOLLOWS. Selecting a node off-screen (a pick, a search
//      hit, an arrow key) scrolls to it. That needs a list that owns its own
//      offset — see `../virtual.ts` for why the native scroll container could
//      not have done it.
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { Row } from "../protocol";
import { onKeydown } from "../keys";
import { CRUMBS_H, PANE_HEAD_H, ROW_H } from "../layout";
import { nodeLabel, rowKind } from "../dump";
import {
  ancestors,
  highlight,
  refreshTree,
  searchElements,
  select,
  setExpanded,
  state,
  stepMatch,
  toggle,
  toggleAnchors,
  ui,
} from "../store";
import { useVirtual } from "../virtual";

const props = defineProps<{ height: number }>();

const listHeight = computed(() => Math.max(40, props.height - PANE_HEAD_H - CRUMBS_H));
const rows = computed(() => state.rows);
const list = useVirtual(
  () => rows.value.map(() => ROW_H),
  () => listHeight.value,
);

const searchOpen = ref(false);
const searchText = ref("");
const searchBox = ref<VnElement | null>(null);

const selectedIndex = computed(() => rows.value.findIndex((r) => r.id === state.selected));
const crumbs = computed(() => ancestors(state.selected));

/** The rendered slice. */
const slice = computed(() => rows.value.slice(list.start, list.end));

// Selection moves for reasons this component did not cause — a pick in the
// target, a search step, a click in the breadcrumbs — and every one of them
// should bring the row into view.
watch(
  () => state.selected,
  () => {
    const i = selectedIndex.value;
    if (i >= 0) list.ensureVisible(i);
  },
);

function rowClick(row: Row): void {
  void select(row.id);
}

function twistyClick(row: Row, event: VnMouseEvent): void {
  event.stopPropagation();
  toggle(row.id);
  const i = selectedIndex.value;
  if (i >= 0) list.ensureVisible(i);
}

function move(delta: number): void {
  const rowsNow = rows.value;
  if (!rowsNow.length) return;
  const at = selectedIndex.value;
  const next = at < 0 ? 0 : Math.max(0, Math.min(rowsNow.length - 1, at + delta));
  void select(rowsNow[next].id);
  list.ensureVisible(next);
}

function collapseOrParent(): void {
  const at = selectedIndex.value;
  if (at < 0) return;
  const row = rows.value[at];
  if (row.open && row.children) {
    setExpanded(row.id, false);
    return;
  }
  // Already closed (or a leaf): step out to the parent, which is the nearest
  // row above with a smaller depth.
  for (let i = at - 1; i >= 0; i--) {
    if (rows.value[i].depth < row.depth) {
      void select(rows.value[i].id);
      list.ensureVisible(i);
      return;
    }
  }
}

function expandOrChild(): void {
  const at = selectedIndex.value;
  if (at < 0) return;
  const row = rows.value[at];
  if (row.children && !row.open) {
    setExpanded(row.id, true);
    return;
  }
  if (row.children) move(1);
}

function onKey(event: VnKeyboardEvent): void {
  if (state.tab !== "elements") return;
  // A search box has the keyboard while it is focused; arrow keys there move a
  // caret, not a selection. Escape is deliberately NOT handled here at all:
  // two listeners on one target cannot stop each other's propagation, and the
  // child's runs FIRST (Vue mounts children before parents), so a panel that
  // closed its own search would have the shell toggle the drawer on the same
  // keystroke. The shell owns Escape and calls `ui.closeElementSearch`.
  if (ui.typing && event.key !== "Enter") return;
  switch (event.key) {
    case "ArrowDown":
      move(1);
      break;
    case "ArrowUp":
      move(-1);
      break;
    case "ArrowLeft":
      collapseOrParent();
      break;
    case "ArrowRight":
      expandOrChild();
      break;
    case "PageDown":
      move(Math.floor(listHeight.value / ROW_H));
      break;
    case "PageUp":
      move(-Math.floor(listHeight.value / ROW_H));
      break;
    case "Home":
      if (rows.value.length) {
        void select(rows.value[0].id);
        list.scroll = 0;
      }
      break;
    case "End":
      if (rows.value.length) {
        void select(rows.value[rows.value.length - 1].id);
        list.toBottom();
      }
      break;
    case "Enter":
      if (ui.typing && searchOpen.value) {
        stepMatch(event.shiftKey ? -1 : 1);
        break;
      }
      // Re-arm the outline in the target and hold it there a little longer —
      // "show me where this is" without moving the mouse.
      if (state.selected !== null) void highlight(state.selected, 4000);
      break;
    default:
      return;
  }
}

function openSearch(): void {
  searchOpen.value = true;
  ui.elementSearchOpen = true;
  searchBox.value?.focus();
}

function closeSearch(): void {
  searchOpen.value = false;
  ui.elementSearchOpen = false;
  searchText.value = "";
  searchElements("");
  ui.typing = false;
}

watch(searchText, (value) => searchElements(value));

let offKey: (() => void) | null = null;

onMounted(() => {
  offKey = onKeydown(onKey);
  ui.openElementSearch = openSearch;
  ui.closeElementSearch = closeSearch;
});
onUnmounted(() => {
  offKey?.();
  offKey = null;
  ui.openElementSearch = null;
  ui.closeElementSearch = null;
  ui.elementSearchOpen = false;
});

/** `<div class="a b" id="x">` — one row, coloured by part. */
function classesOf(row: Row): string[] {
  return row.node.classes || [];
}
function idOf(row: Row): string {
  return row.node.props?.id || "";
}
function textOf(row: Row): string {
  const t = (row.node.text || "").replace(/\s+/g, " ").trim();
  return t.length > 90 ? `${t.slice(0, 90)}…` : t;
}
function scopeOf(row: Row): string {
  const s = row.node.scopes || [];
  return s.length ? s[0] : "";
}
</script>

<template>
  <section class="pane elements">
    <header class="pane-head">
      <span class="label">Elements</span>
      <span class="chip">{{ Object.keys(state.byId).length }} nodes</span>
      <span v-if="state.treeRev !== state.seenRev" class="chip warn">stale</span>
      <div class="spacer"></div>
      <div v-if="searchOpen" class="search">
        <input
          ref="searchBox"
          v-model="searchText"
          class="input"
          placeholder="tag, .class, #id or text"
          @focus="ui.typing = true"
          @blur="ui.typing = false"
        />
        <span class="hits">{{ state.matches.length }}</span>
        <div class="btn" title="Previous  ·  Shift+Enter" @click="stepMatch(-1)">↑</div>
        <div class="btn" title="Next  ·  Enter" @click="stepMatch(1)">↓</div>
        <div class="btn" title="Close  ·  Esc" @click="closeSearch()">✕</div>
      </div>
      <div v-else class="btn" title="Find  ·  Ctrl+F" @click="openSearch()">find</div>
      <div
        class="btn"
        :class="{ on: state.showAnchors }"
        title="Show Vue's fragment anchors (one per v-if / v-for; they paint nothing)"
        @click="toggleAnchors()"
      >
        &lt;!----&gt;
      </div>
      <div class="btn" title="Re-read the tree" @click="refreshTree()">↻</div>
    </header>

    <div v-if="state.treeError" class="empty t-red">{{ state.treeError }}</div>

    <div v-else class="list" :style="{ height: listHeight + 'px' }" @wheel="list.onWheel($event)">
      <div class="viewport" :style="{ marginTop: list.padTop + 'px' }">
        <div
          v-for="row in slice"
          :key="row.id"
          class="row mono"
          :class="{ sel: row.id === state.selected, hit: row.hit }"
          :style="{ paddingLeft: 6 + row.depth * 11 + 'px' }"
          @click="rowClick(row)"
        >
          <span
            class="twisty"
            :class="{ open: row.open, leaf: !row.children }"
            @click="twistyClick(row, $event)"
            >{{ row.children ? (row.open ? "▾" : "▸") : "" }}</span
          >
          <!-- Rows are drawn by KIND, never by tag: a text node and a fragment
               anchor both carry `tag: ""`, and guessing from the tag is what
               used to print a run of `<root>` rows. See `rowKind` in dump.ts. -->
          <template v-if="rowKind(row.node) === 'text'">
            <span class="t-faint">"</span><span class="text">{{ textOf(row) }}</span
            ><span class="t-faint">"</span>
          </template>
          <template v-else-if="rowKind(row.node) === 'anchor'">
            <span class="anchor">&lt;!----&gt;</span>
            <span class="scope">fragment anchor</span>
          </template>
          <template v-else>
            <span class="t-faint">&lt;</span>
            <span class="tag">{{ row.node.tag }}</span>
            <span v-if="idOf(row)" class="id">#{{ idOf(row) }}</span>
            <span v-for="c in classesOf(row)" :key="c" class="cls">.{{ c }}</span>
            <span class="t-faint">&gt;</span>
            <span v-if="textOf(row)" class="text">{{ textOf(row) }}</span>
            <span v-if="row.truncated" class="more">+{{ row.truncated }}</span>
            <span v-if="scopeOf(row)" class="scope">{{ scopeOf(row) }}</span>
          </template>
        </div>
      </div>
      <div
        v-if="list.thumb"
        class="vscroll"
        :style="{ top: list.thumb.top + 'px', height: list.thumb.height + 'px' }"
      ></div>
    </div>

    <!-- Breadcrumbs. Chrome puts them under the tree and so do we: they are the
         answer to "where am I", and the fastest way back up a deep subtree. -->
    <footer class="crumbs mono">
      <template v-if="crumbs.length">
        <template v-for="(node, i) in crumbs" :key="node.id">
          <span v-if="i" class="sep">›</span>
          <span
            class="crumb"
            :class="{ on: node.id === state.selected }"
            @click="select(node.id)"
            >{{ nodeLabel(node) }}</span
          >
        </template>
      </template>
      <span v-else class="t-faint">no selection — click a row, or press ⌖ inspect</span>
    </footer>
  </section>
</template>

<style scoped>
.elements {
  min-width: 0;
}
.spacer {
  flex: 1;
}
.search {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
}
.search .input {
  width: 170px;
}
.hits {
  font-size: 10px;
  color: #6d7c90;
  min-width: 18px;
  text-align: right;
}
.list {
  position: relative;
  overflow: hidden;
  min-height: 0;
}
.viewport {
  display: flex;
  flex-direction: column;
}
.row {
  height: 18px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 2px;
  font-size: 11px;
  color: #b9c6d6;
  white-space: nowrap;
  padding-right: 10px;
}
.row:hover {
  background: #ffffff0a;
}
.row.sel {
  background: #14344f;
}
.row.hit {
  background: #3a2f0f;
}
.row.sel.hit {
  background: #245078;
}
.twisty {
  width: 11px;
  color: #6d7c90;
  font-size: 9px;
  flex-shrink: 0;
}
.twisty:hover {
  color: #d7e0ea;
}
.tag {
  color: #79c0ff;
}
.id {
  color: #ffa657;
}
.cls {
  color: #d2a8ff;
}
.text {
  color: #9db3c9;
  margin-left: 3px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.anchor {
  color: #6d7c90;
}
.more {
  color: #6d7c90;
  margin-left: 4px;
}
.scope {
  color: #46556a;
  margin-left: 6px;
  font-size: 10px;
}
.crumbs {
  height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
  padding-left: 8px;
  padding-right: 8px;
  border-top-width: 1px;
  border-color: #1e2733;
  background: #0d1219cc;
  font-size: 10px;
  overflow: hidden;
  white-space: nowrap;
}
.crumb {
  color: #8494a8;
}
.crumb:hover {
  color: #d7e0ea;
}
.crumb.on {
  color: #a8d4ff;
}
.sep {
  color: #3c4859;
}
</style>
