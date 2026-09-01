<script setup lang="ts">
// Storage: `localStorage` and `sessionStorage`, read AND written.
//
// The values come from the app's own JS rather than from vn-storage's disk
// (see `cmd_storage` in `crates/vn-app/src/devtools/mod.rs` for why): the area
// an app sees is the spec'd object, and a panel that read the sealed file on
// disk would show something subtly different — a pending write, or a session
// area that has no file at all.
//
// Editing is real and it is the first WRITE surface this inspector has. That is
// a deliberate line: the Styles panel still reads only (editing the cascade
// needs an authoring channel back into it, docs/DEVTOOLS-UI.md §7), but a
// storage area is a key/value store with a specified setter, and "change this
// flag and reload" is most of what a storage panel is for.
import { ref } from "vue";
import { PANE_HEAD_H } from "../layout";
import { clearStorage, refreshStorage, removeStorage, setStorage, state, ui } from "../store";

const props = defineProps<{ height: number }>();

const editing = ref<string | null>(null);
const draft = ref("");
const newKey = ref("");
const newValue = ref("");

function startEdit(key: string, value: string): void {
  editing.value = key;
  draft.value = value;
}

async function commit(key: string): Promise<void> {
  editing.value = null;
  await setStorage(key, draft.value);
}

async function add(): Promise<void> {
  if (!newKey.value.trim()) return;
  await setStorage(newKey.value, newValue.value);
  newKey.value = "";
  newValue.value = "";
}

function switchArea(area: string): void {
  state.storageArea = area;
  void refreshStorage();
}

/** How much a value takes, for the size column. */
function size(value: string): string {
  const n = value.length;
  return n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`;
}
</script>

<template>
  <section class="pane storage" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Storage</span>
      <div class="areas">
        <div
          class="area"
          :class="{ on: state.storageArea === 'local' }"
          @click="switchArea('local')"
        >
          localStorage
        </div>
        <div
          class="area"
          :class="{ on: state.storageArea === 'session' }"
          @click="switchArea('session')"
        >
          sessionStorage
        </div>
      </div>
      <span class="chip">{{ state.storage.length }} keys</span>
      <div class="spacer"></div>
      <div class="btn" @click="refreshStorage()">↻</div>
      <div class="btn danger" title="Remove every key in this area" @click="clearStorage()">
        clear
      </div>
    </header>

    <div v-if="state.storageError" class="empty t-red">{{ state.storageError }}</div>

    <div v-else class="body">
      <div class="cols mono">
        <span class="c-key">key</span>
        <span class="c-val">value</span>
        <span class="c-size">size</span>
        <span class="c-act"></span>
      </div>
      <div class="rows">
        <div v-for="entry in state.storage" :key="entry.key" class="row mono">
          <span class="c-key" :title="entry.key">{{ entry.key }}</span>
          <template v-if="editing === entry.key">
            <input
              v-model="draft"
              class="input c-val"
              @keydown.enter="commit(entry.key)"
              @keydown.escape="editing = null"
              @focus="ui.typing = true"
              @blur="ui.typing = false"
            />
            <span class="c-size"></span>
            <span class="c-act">
              <span class="btn" @click="commit(entry.key)">save</span>
            </span>
          </template>
          <template v-else>
            <span class="c-val" :title="entry.value" @click="startEdit(entry.key, entry.value)">{{
              entry.value
            }}</span>
            <span class="c-size">{{ size(entry.value) }}</span>
            <span class="c-act">
              <span class="btn" @click="startEdit(entry.key, entry.value)">edit</span>
              <span class="btn danger" @click="removeStorage(entry.key)">✕</span>
            </span>
          </template>
        </div>
        <div v-if="!state.storage.length" class="empty">
          This area is empty. Add a key below, or run
          <b class="mono">localStorage.setItem("k","v")</b> in the console — both write to the same
          place, because this panel goes through the app's own JS.
        </div>
      </div>

      <footer class="add">
        <input
          v-model="newKey"
          class="input k"
          placeholder="new key"
          @focus="ui.typing = true"
          @blur="ui.typing = false"
        />
        <input
          v-model="newValue"
          class="input v"
          placeholder="value"
          @keydown.enter="add()"
          @focus="ui.typing = true"
          @blur="ui.typing = false"
        />
        <div class="btn" @click="add()">add</div>
      </footer>
    </div>
  </section>
</template>

<style scoped>
.storage {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.areas {
  display: flex;
  flex-direction: row;
  gap: 2px;
}
.area {
  height: 18px;
  padding-left: 8px;
  padding-right: 8px;
  border-radius: 4px;
  font-size: 10px;
  color: #8494a8;
  display: flex;
  align-items: center;
  font-family: "Cascadia Mono", "Consolas", monospace;
}
.area:hover {
  color: #d7e0ea;
  background: #171e29;
}
.area.on {
  background: #14344f;
  color: #a8d4ff;
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.cols {
  height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  padding-left: 10px;
  padding-right: 12px;
  font-size: 10px;
  color: #5c6b80;
  background: #0f141d;
  border-bottom-width: 1px;
  border-color: #1a2230;
}
.rows {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
.row {
  min-height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  padding-left: 10px;
  padding-right: 12px;
  font-size: 11px;
  border-bottom-width: 1px;
  border-color: #ffffff08;
}
.row:hover {
  background: #ffffff0a;
}
.c-key {
  width: 190px;
  flex-shrink: 0;
  color: #d2a8ff;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.c-val {
  flex: 1;
  min-width: 0;
  color: #a5d6ff;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.c-size {
  width: 60px;
  flex-shrink: 0;
  text-align: right;
  color: #5c6b80;
}
.c-act {
  width: 84px;
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  gap: 4px;
  justify-content: flex-end;
}
.add {
  height: 32px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  padding-left: 10px;
  padding-right: 12px;
  border-top-width: 1px;
  border-color: #1e2733;
  background: #0d1219;
}
.add .k {
  width: 190px;
}
.add .v {
  flex: 1;
  min-width: 0;
}
</style>
