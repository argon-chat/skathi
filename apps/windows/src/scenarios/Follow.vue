<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { appWindow } from "@skathi-native/runtime";
import Icon from "../components/Icon.vue";
import PopWindow from "../components/PopWindow.vue";
import Scenario from "../components/Scenario.vue";
import type { IconName } from "../icons";

type Rect = { x: number; y: number; width: number; height: number };
type Overlay = { mapFromMain(target: unknown): Rect | null };

const isWindows = navigator.platform === "Win32";

const KINDS: { ext: string; kind: string; icon: IconName }[] = [
  { ext: "md", kind: "Markdown document", icon: "doc" },
  { ext: "png", kind: "PNG image", icon: "image" },
  { ext: "zip", kind: "Compressed folder", icon: "archive" },
  { ext: "xlsx", kind: "Spreadsheet", icon: "sheet" },
  { ext: "flac", kind: "Audio", icon: "music" },
];
const NAMES: [string, number][] = [
  ["Window guides", 0],
  ["Poster draft", 1],
  ["Quarterly report", 3],
  ["Site photos", 2],
  ["Budget 2027", 3],
  ["Interview, part 2", 4],
  ["Launch notes", 0],
  ["Theme study", 1],
  ["Clips for the docs", 2],
  ["Release checklist", 0],
  ["Team offsite", 1],
  ["Brand palette", 1],
  ["Field recording", 4],
  ["Roadmap", 0],
  ["Pop-out sketches", 1],
  ["Contract scan", 1],
  ["Font specimens", 2],
  ["Invoice 0412", 3],
  ["Demo script", 0],
  ["Backup, November", 2],
];
const files = NAMES.map(([name, kind], i) => {
  const k = KINDS[kind]!;
  return {
    name: `${name}.${k.ext}`,
    kind: k.kind,
    icon: k.icon,
    size: `${(((i * 7919) % 900) / 10 + 0.4).toFixed(1)} MB`,
    modified: `${(i % 9) + 2} Nov 2026, ${9 + (i % 8)}:${String((i * 13) % 60).padStart(2, "0")}`,
  };
});

const selected = ref<number | null>(null);
const list = ref<unknown>(null);
const overlay = ref<Overlay | null>(null);
const tip = ref<Rect | null>(null);
const rows = new Map<number, unknown>();
const file = computed(() => (selected.value === null ? null : files[selected.value]!));

const rowRef = (i: number) => (el: unknown) => {
  if (el) rows.set(i, el);
  else rows.delete(i);
};

function place() {
  const row = selected.value === null ? undefined : rows.get(selected.value);
  const r = row && overlay.value ? overlay.value.mapFromMain(row) : null;
  const box = overlay.value ? overlay.value.mapFromMain(list.value) : null;
  if (r && box && r.y + r.height > box.y && r.y < box.y + box.height) tip.value = r;
  else tip.value = null;
}

let frame = 0;
function schedule() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(place);
}

function select(i: number) {
  selected.value = selected.value === i ? null : i;
  schedule();
}

onMounted(() => {
  appWindow.addEventListener("move", schedule);
  addEventListener("resize", schedule);
});
onBeforeUnmount(() => {
  appWindow.removeEventListener("move", schedule);
  removeEventListener("resize", schedule);
  cancelAnimationFrame(frame);
});
</script>

<template>
  <Scenario id="pop-out/follow">
    <div class="card browser">
      <div class="head">
        <span>Name</span>
        <span class="col">Kind</span>
      </div>
      <div ref="list" class="list" data-demo="file-list" @scroll="schedule">
        <div
          v-for="(f, i) in files"
          :key="f.name"
          :ref="rowRef(i)"
          class="row"
          :class="{ on: selected === i }"
          :data-demo="`row-${i}`"
          @click="select(i)"
        >
          <Icon :name="f.icon" />
          <span class="name">{{ f.name }}</span>
          <span class="col">{{ f.kind }}</span>
        </div>
      </div>
    </div>

    <PopWindow
      v-if="isWindows"
      ref="overlay"
      no-cursor
      title="File details"
      cover="monitor"
      chrome="hidden"
      backdrop="transparent"
      always-on-top
      ignore-mouse-events
      :focusable="false"
      :visible="tip !== null"
    >
      <div
        v-if="tip && file"
        class="tip"
        :style="{ left: `${tip.x + tip.width + 14}px`, top: `${tip.y + tip.height / 2 - 52}px` }"
      >
        <span class="tip-name">{{ file.name }}</span>
        <span class="tip-row">{{ file.kind }} · {{ file.size }}</span>
        <span class="tip-row">Modified {{ file.modified }}</span>
        <span class="tip-map code">mapFromMain: {{ Math.round(tip.x) }}, {{ Math.round(tip.y) }}</span>
      </div>
    </PopWindow>
  </Scenario>
</template>

<style scoped>
.browser {
  padding: 0;
  gap: 0;
  width: 420px;
  overflow: hidden;
}
.head {
  height: 36px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 16px 0 46px;
  font-size: 12px;
  color: var(--text-3);
  border-bottom: 1px solid var(--card-line);
}
.head span:first-child {
  flex: 1;
}
.list {
  height: 340px;
  overflow-y: auto;
  padding: 4px;
}
.row {
  height: 36px;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 12px 0 16px;
  border-radius: 5px;
  color: var(--text-2);
  font-size: 13px;
  cursor: pointer;
  transition: background-color 100ms ease;
}
.row:hover,
.row.demo-hover {
  background: var(--subtle-hover);
}
.row.on {
  background: var(--accent-soft);
  color: var(--text);
}
.name {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.col {
  width: 130px;
  font-size: 12px;
  color: var(--text-3);
}
.tip {
  position: absolute;
  width: 240px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px 14px;
  border-radius: 8px;
  border: 1px solid #ffffff1a;
  background: #2b2b2bf2;
  box-shadow: 0 8px 24px #00000059;
  color: #ffffff;
  font-family: var(--font);
}
.tip-name {
  font-size: 14px;
  font-weight: 600;
}
.tip-row {
  font-size: 12px;
  color: #ffffffb0;
}
.tip-map {
  margin-top: 4px;
  color: #d4a5e6;
}
</style>
