<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import {
  callsiteLabel,
  frameBuckets,
  milliseconds,
  rowWindow,
  spanRows,
  timeRange,
  zoomRange,
} from "../inspector-model";
import type {
  InspectorRequest,
  TraceDetail,
  TraceFrame,
  TraceFrames,
  TraceSpan,
  TraceStatus,
} from "../inspector-model";
const props = defineProps<{
  height: number;
  request?: InspectorRequest | undefined;
  sessionKey?: string | number | undefined;
}>();
const emit = defineEmits<{ (event: "recording-change", active: boolean): void }>();
const status = ref<TraceStatus | null>(null),
  frames = ref<TraceFrame[]>([]),
  detail = ref<TraceDetail | null>(null);
const selected = ref<number | null>(null),
  selectedSpan = ref<number | null>(null),
  busy = ref(false),
  detailBusy = ref(false),
  error = ref("");
const frameNext = ref<number | null>(null),
  totalFrames = ref(0),
  scroll = ref(0),
  collapsed = ref(new Set<number>());
const range = ref({ start: 0, end: 1 });
let epoch = 0,
  selectionEpoch = 0;
const buckets = computed(() => frameBuckets(frames.value));
const largestFrame = computed(() => Math.max(1, ...frames.value.map((frame) => frame.durationMs)));
const rows = computed(() => spanRows(detail.value?.spans || [], collapsed.value));
const viewportHeight = computed(() => Math.max(72, props.height - 293));
const windowed = computed(() => rowWindow(rows.value.length, scroll.value, viewportHeight.value));
const visible = computed(() => rows.value.slice(windowed.value.start, windowed.value.end));
const inspected = computed(() => detail.value?.spans.find((span) => span.id === selectedSpan.value));
const inspectedHasChildren = computed(
  () => rows.value.find((row) => row.span.id === selectedSpan.value)?.hasChildren || false,
);
function setStatus(value: TraceStatus) {
  status.value = value;
  emit("recording-change", value.recording);
}
async function refresh(append = false) {
  if (!props.request || busy.value) return;
  const token = epoch;
  busy.value = true;
  error.value = "";
  try {
    const value = await props.request<TraceStatus>({ cmd: "trace", op: "recording-status" });
    if (token !== epoch) return;
    setStatus(value);
    if (!value.sessionId) {
      frames.value = [];
      totalFrames.value = 0;
      frameNext.value = null;
      detail.value = null;
      selected.value = null;
      return;
    }
    const page = await props.request<TraceFrames>({
      cmd: "trace",
      op: "frames",
      offset: append ? frameNext.value : 0,
      limit: 240,
    });
    if (token !== epoch) return;
    if (page.sessionId !== value.sessionId)
      throw new Error("Capture changed while loading frames. Refresh to read the current capture.");
    frames.value = append ? [...frames.value, ...page.frames] : page.frames;
    frameNext.value = page.nextOffset;
    totalFrames.value = page.total;
    if (!append && !frames.value.some((frame) => frame.id === selected.value)) {
      detail.value = null;
      selected.value = null;
    }
    if (selected.value === null && frames.value.length)
      void selectFrame(frames.value.reduce((best, frame) => (frame.cpuMs > best.cpuMs ? frame : best)).id);
  } catch (reason) {
    if (token === epoch) error.value = String(reason);
  } finally {
    if (token === epoch) busy.value = false;
  }
}
async function record() {
  if (!props.request || busy.value) return;
  const token = epoch;
  busy.value = true;
  error.value = "";
  try {
    const value = await props.request<TraceStatus>({
      cmd: "trace",
      op: status.value?.recording ? "recording-stop" : "recording-start",
      cap: 1200,
    });
    if (token !== epoch) return;
    setStatus(value);
    if (value.recording) {
      selectionEpoch++;
      frames.value = [];
      detail.value = null;
      selected.value = null;
    }
  } catch (reason) {
    if (token === epoch) error.value = String(reason);
  } finally {
    if (token === epoch) {
      busy.value = false;
      if (!status.value?.recording) void refresh();
    }
  }
}
async function selectFrame(id: number, append = false) {
  if (!props.request) return;
  const token = epoch,
    selection = ++selectionEpoch;
  selected.value = id;
  detailBusy.value = true;
  error.value = "";
  if (!append) {
    detail.value = null;
    selectedSpan.value = null;
    scroll.value = 0;
    collapsed.value = new Set();
  }
  try {
    const page = await props.request<TraceDetail>({
      cmd: "trace",
      op: "frame",
      frameId: id,
      offset: append ? detail.value?.nextOffset : 0,
      limit: 400,
    });
    if (token !== epoch || selection !== selectionEpoch) return;
    if (status.value && page.sessionId !== status.value.sessionId)
      throw new Error("This frame belongs to another capture. Refresh the frame list.");
    detail.value = append && detail.value ? { ...page, spans: [...detail.value.spans, ...page.spans] } : page;
    if (!append) range.value = timeRange(0, page.frame.durationMs, page.frame.durationMs);
  } catch (reason) {
    if (token === epoch && selection === selectionEpoch) error.value = String(reason);
  } finally {
    if (token === epoch && selection === selectionEpoch) detailBusy.value = false;
  }
}
function moveFrame(direction: number) {
  const index = frames.value.findIndex((frame) => frame.id === selected.value);
  const frame = frames.value[Math.max(0, Math.min(frames.value.length - 1, index + direction))];
  if (frame) void selectFrame(frame.id);
}
function zoom(factor: number) {
  if (detail.value) range.value = zoomRange(range.value.start, range.value.end, detail.value.frame.durationMs, factor);
}
function pan(direction: number) {
  if (!detail.value) return;
  const shift = (range.value.end - range.value.start) * 0.4 * direction;
  range.value = timeRange(range.value.start + shift, range.value.end + shift, detail.value.frame.durationMs);
}
function toggleSpan(id: number) {
  const next = new Set(collapsed.value);
  next.has(id) ? next.delete(id) : next.add(id);
  collapsed.value = next;
}
function selectRow(span: TraceSpan) {
  selectedSpan.value = span.id;
}
function keys(event: { key: string; shiftKey?: boolean; preventDefault(): void }) {
  if (event.key === "+" || event.key === "=") {
    event.preventDefault();
    zoom(0.5);
  } else if (event.key === "-") {
    event.preventDefault();
    zoom(2);
  } else if (event.key === "ArrowLeft" && event.shiftKey) {
    event.preventDefault();
    pan(-1);
  } else if (event.key === "ArrowRight" && event.shiftKey) {
    event.preventDefault();
    pan(1);
  }
}
function color(category: string) {
  return (
    (
      {
        js: "#c49af4",
        dom: "#80baf2",
        style: "#e3b341",
        layout: "#76d4dc",
        paint: "#69c484",
        render: "#b69ceb",
        wait: "#8494a8",
        native: "#8cbee6",
      } as Record<string, string>
    )[category.toLowerCase()] || "#91a6bd"
  );
}
function spanStyle(span: TraceSpan) {
  const start = span.startMs - (detail.value?.frame.startMs || 0),
    width = range.value.end - range.value.start;
  const left = Math.max(range.value.start, start),
    end = Math.min(range.value.end, start + span.durationMs);
  return {
    left: `${((left - range.value.start) / width) * 100}%`,
    width: `${Math.max(0, ((end - left) / width) * 100)}%`,
    background: color(span.category),
    display: end < left ? "none" : "block",
  };
}
watch(
  [() => props.request, () => props.sessionKey],
  () => {
    epoch++;
    selectionEpoch++;
    status.value = null;
    frames.value = [];
    detail.value = null;
    selected.value = null;
    busy.value = false;
    detailBusy.value = false;
    error.value = "";
    if (props.request) void refresh();
  },
  { immediate: true },
);
onBeforeUnmount(() => {
  epoch++;
  selectionEpoch++;
  if (status.value?.recording && props.request)
    void props
      .request<TraceStatus>({ cmd: "trace", op: "recording-stop" })
      .then((value) => emit("recording-change", value.recording))
      .catch(() => {});
});
</script>

<template>
  <section class="capture" :style="{ height: height + 'px' }" aria-label="Frame recording" @keydown="keys">
    <div class="capture-toolbar">
      <button class="btn record" :class="{ recording: status?.recording }" :disabled="!request || busy" @click="record">
        {{ status?.recording ? "Stop recording" : "Record interaction" }}
      </button>
      <button class="btn" :disabled="!request || busy" @click="refresh()">{{ busy ? "Loading…" : "Refresh" }}</button>
      <span class="summary" role="status">{{
        status?.recording ? "Recording target CPU work" : `${totalFrames} captured frames`
      }}</span>
      <span class="cpu-label">CPU timings</span>
    </div>
    <div v-if="error" class="feedback failure" role="alert">
      {{ error }} <button class="btn" :disabled="busy" @click="refresh()">Retry</button>
    </div>
    <div v-if="!request" class="feedback">Attach to a target to record an interaction.</div>
    <div v-else-if="!frames.length" class="feedback">
      {{
        status?.recording
          ? "Interact with the target, then stop recording to inspect its frames."
          : busy
            ? "Reading capture status…"
            : "Record a short interaction, stop, then select a frame to inspect its ordered calls."
      }}
    </div>
    <template v-if="frames.length">
      <div
        class="frame-overview"
        role="group"
        aria-label="Captured frames; tallest bar in each group"
        tabindex="0"
        @keydown.left.prevent="!$event.shiftKey && moveFrame(-1)"
        @keydown.right.prevent="!$event.shiftKey && moveFrame(1)"
      >
        <button
          v-for="bucket in buckets"
          :key="bucket.frame.id"
          class="frame-bar"
          :class="{ selected: selected === bucket.frame.id }"
          :style="{ height: Math.max(4, (bucket.frame.durationMs / largestFrame) * 66) + 'px' }"
          :title="`Frame ${bucket.frame.id} · ${milliseconds(bucket.frame.durationMs)}${bucket.count > 1 ? ` · slowest of ${bucket.count} frames` : ''}`"
          :aria-label="`Select frame ${bucket.frame.id}, ${milliseconds(bucket.frame.durationMs)}`"
          :aria-pressed="selected === bucket.frame.id"
          @click="selectFrame(bucket.frame.id)"
        ></button>
      </div>
      <div class="overview-caption">
        <span>{{ frames.length }} / {{ totalFrames }} frames · bars show CPU frame duration</span
        ><button v-if="frameNext !== null" class="btn" :disabled="busy" @click="refresh(true)">Load next frames</button
        ><span v-if="status && (status.droppedFrames || status.droppedSpans)"
          >Dropped {{ status.droppedFrames }} frames / {{ status.droppedSpans }} spans</span
        >
      </div>
    </template>
    <div v-if="detailBusy && !detail" class="feedback">Loading ordered calls for frame {{ selected }}…</div>
    <template v-if="detail">
      <div class="detail-toolbar">
        <button
          class="btn"
          :disabled="detailBusy || frames[0]?.id === selected"
          @click="moveFrame(-1)"
          aria-label="Previous frame"
        >
          Previous</button
        ><button
          class="btn"
          :disabled="detailBusy || frames[frames.length - 1]?.id === selected"
          @click="moveFrame(1)"
          aria-label="Next frame"
        >
          Next
        </button>
        <strong class="frame-name">Frame {{ detail.frame.id }}</strong
        ><span class="mono"
          >{{ milliseconds(detail.frame.durationMs) }} total · {{ milliseconds(detail.frame.cpuMs) }} CPU</span
        >
        <span class="grow"></span
        ><button class="btn" :disabled="range.start <= 0" @click="pan(-1)" aria-label="Pan earlier">Earlier</button
        ><button class="btn" @click="zoom(0.5)" title="Zoom in (+)">Zoom in</button
        ><button class="btn" @click="zoom(2)" title="Zoom out (-)">Zoom out</button
        ><button class="btn" @click="range = timeRange(0, detail.frame.durationMs, detail.frame.durationMs)">Fit</button
        ><button class="btn" :disabled="range.end >= detail.frame.durationMs" @click="pan(1)" aria-label="Pan later">
          Later
        </button>
      </div>
      <div class="lanes">
        <span v-for="lane in detail.frame.categories" :key="lane.category" class="lane"
          ><i :style="{ background: color(lane.category) }"></i>{{ lane.category }}
          <span class="mono">{{ milliseconds(lane.selfMs) }}</span></span
        >
      </div>
      <div class="call-columns">
        <span class="call-name">Ordered calls</span><span class="call-time">Self / total CPU</span>
        <div class="ruler mono">
          <span>{{ milliseconds(range.start) }}</span
          ><span>{{ milliseconds((range.start + range.end) / 2) }}</span
          ><span>{{ milliseconds(range.end) }}</span>
        </div>
      </div>
      <div
        class="call-scroll"
        :style="{ height: viewportHeight + 'px' }"
        @scroll="scroll = $event.target.scrollTop"
        role="tree"
        aria-label="Frame calls"
      >
        <div :style="{ height: windowed.padTop + 'px' }"></div>
        <button
          v-for="row in visible"
          :key="row.span.id"
          class="call-row"
          :class="{ chosen: selectedSpan === row.span.id }"
          :aria-level="row.depth + 1"
          :aria-expanded="row.hasChildren ? !collapsed.has(row.span.id) : undefined"
          role="treeitem"
          :title="callsiteLabel(row.span)"
          @click="selectRow(row.span)"
          @dblclick="row.hasChildren && toggleSpan(row.span.id)"
          @keydown.left.prevent="
            !$event.shiftKey && row.hasChildren && !collapsed.has(row.span.id) && toggleSpan(row.span.id)
          "
          @keydown.right.prevent="
            !$event.shiftKey && row.hasChildren && collapsed.has(row.span.id) && toggleSpan(row.span.id)
          "
        >
          <span class="call-name" :style="{ paddingLeft: 8 + Math.min(18, row.depth) * 12 + 'px' }"
            ><svg
              v-if="row.hasChildren"
              width="10"
              height="10"
              viewBox="0 0 10 10"
              :style="{ transform: collapsed.has(row.span.id) ? 'rotate(-90deg)' : undefined }"
            >
              <path d="M2 3.5 5 6.5 8 3.5" fill="none" stroke="currentColor" stroke-width="1.5" /></svg
            ><span>{{ row.span.name }}</span></span
          >
          <span class="call-time mono">{{ row.span.selfMs.toFixed(2) }} / {{ row.span.durationMs.toFixed(2) }}</span
          ><span class="call-track"><i :style="spanStyle(row.span)"></i></span>
        </button>
        <div :style="{ height: windowed.padBottom + 'px' }"></div>
      </div>
      <div class="detail-footer">
        <span
          >{{ detail.spans.length }} / {{ detail.total }} spans{{ detail.truncated ? " · bounded capture" : "" }}</span
        ><button
          v-if="detail.nextOffset !== null"
          class="btn"
          :disabled="detailBusy"
          @click="selectFrame(detail.frame.id, true)"
        >
          {{ detailBusy ? "Loading…" : "Load more calls" }}</button
        ><span class="grow"></span><span>+ / − zoom · Shift + arrows pan</span>
      </div>
      <div v-if="inspected" class="selection-detail">
        <div>
          <strong>{{ inspected.name }}</strong
          ><span
            >{{ inspected.category }} · {{ milliseconds(inspected.selfMs) }} self ·
            {{ milliseconds(inspected.durationMs) }} total</span
          ><button v-if="inspectedHasChildren" class="btn" @click="toggleSpan(inspected.id)">
            {{ collapsed.has(inspected.id) ? "Expand calls" : "Collapse calls" }}
          </button>
        </div>
        <div class="callsite mono">{{ callsiteLabel(inspected) }}</div>
        <p v-for="hint in inspected.hints || []" :key="hint">{{ hint }}</p>
        <span v-for="(value, key) in inspected.counters || {}" :key="key" class="counter mono"
          >{{ key }}: {{ value }}</span
        >
      </div>
      <div v-else class="selection-detail muted">
        Select a call to inspect its callsite and captured counters. Render/submit spans measure host CPU work; GPU
        execution timestamps are not recorded.
      </div>
      <div v-for="hint in detail.frame.hints" :key="hint.code" class="frame-hint">
        {{ hint.message }}<span v-if="hint.count > 1"> ({{ hint.count }})</span>
      </div>
    </template>
  </section>
</template>

<style scoped>
.capture {
  min-width: 0;
  overflow-y: auto;
  color: #d7e0ea;
  font-size: 11px;
  background: #10151d;
}
.capture-toolbar,
.detail-toolbar {
  min-height: 34px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  flex-wrap: wrap;
  border-bottom: 1px solid #232c3a;
}
button {
  font-family: inherit;
  cursor: pointer;
}
button:disabled {
  opacity: 0.45;
  cursor: default;
}
button:focus-visible {
  outline: 2px solid #6cb6ff;
  outline-offset: -2px;
}
.record {
  color: #f1b7b2;
}
.record.recording {
  background: #462521;
  border-color: #b95750;
  color: #fff0ed;
}
.summary {
  color: #b9c6d6;
}
.cpu-label {
  color: #8494a8;
  margin-left: auto;
}
.grow {
  flex: 1;
}
.feedback {
  padding: 18px 12px;
  color: #a9b6c7;
  line-height: 1.5;
}
.failure {
  color: #ff9d96;
  display: flex;
  align-items: center;
  gap: 8px;
}
.frame-overview {
  height: 76px;
  padding: 8px 8px 0;
  display: flex;
  align-items: flex-end;
  gap: 2px;
  background: #0b1018;
  overflow: hidden;
}
.frame-bar {
  flex: 1;
  min-width: 2px;
  max-width: 24px;
  border: 0;
  border-radius: 2px 2px 0 0;
  background: #537eac;
  padding: 0;
}
.frame-bar:hover,
.frame-bar.selected {
  background: #9acbff;
}
.overview-caption,
.detail-footer {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 25px;
  padding: 4px 8px;
  color: #a9b6c7;
  font-size: 10px;
  flex-wrap: wrap;
}
.detail-toolbar .mono {
  color: #a9b6c7;
  font-size: 10px;
}
.frame-name {
  font-weight: 600;
}
.lanes {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  padding: 6px 8px;
  background: #121b26;
  color: #bcc9d8;
  font-size: 10px;
}
.lane {
  display: flex;
  align-items: center;
  gap: 5px;
}
.lane i {
  width: 7px;
  height: 7px;
  border-radius: 2px;
}
.call-columns,
.call-row {
  display: flex;
  width: 100%;
  align-items: center;
  min-width: 0;
  box-sizing: border-box;
}
.call-columns {
  height: 24px;
  color: #a9b6c7;
  background: #171e29;
  border-bottom: 1px solid #232c3a;
  font-size: 10px;
}
.call-name {
  width: 34%;
  min-width: 100px;
  flex-shrink: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding-left: 8px;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  gap: 4px;
}
.call-name span {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.call-name svg {
  flex-shrink: 0;
}
.call-time {
  width: 105px;
  flex-shrink: 0;
  text-align: right;
  padding-right: 8px;
  box-sizing: border-box;
  color: #b9c6d6;
  font-size: 10px;
}
.ruler {
  flex: 1;
  min-width: 0;
  display: flex;
  justify-content: space-between;
  padding: 0 5px;
}
.call-scroll {
  overflow-y: auto;
  min-height: 72px;
}
.call-row {
  height: 24px;
  border: 0;
  padding: 0;
  background: #10151d;
  color: #d7e0ea;
  text-align: left;
  font-size: 11px;
}
.call-row:hover {
  background: #1b2735;
}
.call-row.chosen {
  background: #163854;
}
.call-track {
  height: 24px;
  flex: 1;
  min-width: 30px;
  position: relative;
  overflow: hidden;
  border-left: 1px solid #232c3a;
}
.call-track i {
  position: absolute;
  top: 6px;
  height: 12px;
  min-width: 1px;
  border-radius: 2px;
}
.selection-detail {
  border-top: 1px solid #232c3a;
  background: #0c121b;
  padding: 8px;
  min-height: 55px;
  color: #b9c6d6;
  line-height: 1.5;
}
.selection-detail > div:first-child {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.selection-detail strong {
  color: #e6edf3;
}
.selection-detail p {
  margin: 4px 0;
  color: #e3b341;
}
.callsite {
  overflow-wrap: anywhere;
  color: #a5c7e8;
}
.counter {
  margin-right: 12px;
}
.muted {
  color: #a9b6c7;
}
.frame-hint {
  padding: 5px 8px;
  color: #e3b341;
}
@media (max-width: 650px) {
  .call-name {
    width: 38%;
    min-width: 80px;
  }
  .call-time {
    width: 82px;
  }
  .ruler span:nth-child(2) {
    display: none;
  }
  .cpu-label {
    display: none;
  }
}
</style>
