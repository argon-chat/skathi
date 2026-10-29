<script setup lang="ts">
// A body, readable: text as lines, JSON pretty-printed, binary as a hex dump.
//
// Over a virtual list (`../virtual.ts`), because the cap is 256 KB and a
// 256 KB binary body is sixteen thousand rows of hex — the engine pays for
// every node it is asked to lay out, so only the rows in view exist. Long text
// lines are HARD-wrapped at a column count derived from the pane's width
// (`textLines`): nothing here can measure text, and a fixed row height is what
// lets the list skip to any row without measuring the ones before it.
import { computed, ref, watch } from "vue";
import { BODY_BAR_H, BODY_LINE_H, MONO_W } from "../layout";
import { copyText } from "../store";
import {
  bodyAsText,
  bodyBytes,
  bodySummary,
  formatBytes,
  hexLine,
  hexRows,
  isJsonMime,
  prettyJson,
  textLines,
} from "../netfmt";
import { useVirtual } from "../virtual";
import type { NetBody } from "../protocol";

const props = defineProps<{
  body: NetBody | null | undefined;
  width: number;
  height: number;
  /** `limits.budget`, for the "evicted" line. */
  budget: number;
  /** How the status bar names a copy: "the response body". */
  what: string;
}>();

const view = ref<"pretty" | "raw">("pretty");
watch(
  () => props.body,
  () => {
    view.value = "pretty";
    list.scroll = 0;
  },
);

const bytes = computed(() => bodyBytes(props.body));
const text = computed(() =>
  props.body && props.body.encoding === "utf8" && props.body.data !== undefined ? props.body.data : null,
);
const pretty = computed(() => {
  const t = text.value;
  if (t === null) return null;
  if (!isJsonMime(props.body?.mime) && !/^\s*[[{]/.test(t)) return null;
  return prettyJson(t);
});
const cols = computed(() => Math.max(24, Math.floor((props.width - 30) / MONO_W)));

const mode = computed<"hex" | "pretty" | "raw" | "none">(() => {
  if (bytes.value) return "hex";
  if (text.value === null) return "none";
  return view.value === "pretty" && pretty.value !== null ? "pretty" : "raw";
});

const lines = computed(() => {
  if (mode.value === "pretty") return textLines(pretty.value || "", cols.value);
  if (mode.value === "raw") return textLines(text.value || "", cols.value);
  return [];
});
const count = computed(() => (mode.value === "hex" ? hexRows(bytes.value!.length) : lines.value.length));

const listHeight = computed(() => Math.max(BODY_LINE_H, props.height - BODY_BAR_H));
const list = useVirtual(
  () => new Array<number>(count.value).fill(BODY_LINE_H),
  () => listHeight.value,
);
const rows = computed(() => {
  const out: { i: number; text: string }[] = [];
  for (let i = list.start; i < list.end; i++) {
    out.push({ i, text: mode.value === "hex" ? hexLine(bytes.value!, i) : lines.value[i] });
  }
  return out;
});

const summary = computed(() => bodySummary(props.body, props.budget));
const hasData = computed(() => !!props.body && props.body.data !== undefined && !props.body.evicted);

const empty = computed(() => {
  const b = props.body;
  if (b === undefined) return "Not read yet — the body is read off a clone after the row is recorded.";
  if (b === null) return "No body.";
  if (b.evicted) return "";
  if (b.data === undefined) return b.note ? "" : "Nothing was captured.";
  if (!count.value || (b.data === "" && mode.value !== "hex")) return `Empty body (${formatBytes(b.stored)}).`;
  return "";
});

function copy(): void {
  const b = props.body;
  if (!b) return;
  // The text as it came (not the pretty-printed one) — a body pasted into a
  // bug report should be the bytes the server sent. Binary goes as the dump.
  void copyText(bodyAsText(b), b.truncated ? `${props.what} (truncated)` : props.what);
}
</script>

<template>
  <div class="bodyview">
    <div class="bar">
      <span class="sum mono" :class="{ warn: body && (body.truncated || body.evicted) }">{{ summary }}</span>
      <div class="spacer"></div>
      <template v-if="pretty !== null">
        <div class="seg" :class="{ on: view === 'pretty' }" title="Pretty-printed JSON" @click="view = 'pretty'">
          pretty
        </div>
        <div class="seg" :class="{ on: view === 'raw' }" title="The text as it came" @click="view = 'raw'">raw</div>
      </template>
      <span v-if="mode === 'hex'" class="seg on">hex</span>
      <div v-if="hasData" class="btn copy" :title="'Copy ' + what" @click="copy()">⧉ copy</div>
    </div>
    <div v-if="body && body.note" class="note">{{ body.note }}</div>
    <div
      class="list mono"
      :style="{ height: listHeight - (body && body.note ? 22 : 0) + 'px' }"
      @wheel="list.onWheel($event)"
    >
      <div class="viewport" :style="{ marginTop: list.padTop + 'px' }">
        <div v-for="row in rows" :key="row.i" class="ln" :class="{ hex: mode === 'hex' }">{{ row.text }}</div>
      </div>
      <div
        v-if="list.thumb"
        class="vscroll"
        :style="{ top: list.thumb.top + 'px', height: list.thumb.height + 'px' }"
      ></div>
      <div v-if="empty" class="empty">{{ empty }}</div>
    </div>
  </div>
</template>

<style scoped>
.bodyview {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.bar {
  height: 24px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
  padding-left: 10px;
  padding-right: 8px;
  background: #0f141d;
  border-bottom-width: 1px;
  border-color: #1a2230;
}
.sum {
  font-size: 10px;
  color: #8494a8;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.sum.warn {
  color: #e3b341;
}
.spacer {
  flex: 1;
}
.seg {
  height: 17px;
  padding-left: 7px;
  padding-right: 7px;
  border-radius: 3px;
  font-size: 10px;
  color: #8494a8;
  display: flex;
  align-items: center;
}
.seg:hover {
  color: #d7e0ea;
  background: #171e29;
}
.seg.on {
  background: #14344f;
  color: #a8d4ff;
}
.copy {
  height: 18px;
  font-size: 10px;
}
.note {
  height: 22px;
  display: flex;
  align-items: center;
  padding-left: 10px;
  padding-right: 10px;
  font-size: 10px;
  color: #c6a663;
  background: #2a1f0726;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
.ln {
  height: 16px;
  padding-left: 10px;
  padding-right: 14px;
  font-size: 11px;
  line-height: 16px;
  color: #c9d4e2;
  white-space: pre;
  overflow: hidden;
}
.ln.hex {
  color: #a5d6ff;
}
</style>
