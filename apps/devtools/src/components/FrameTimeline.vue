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
import { categoryColor, frameCeiling, traceBar, traceSummary, workClass } from "../perffmt";
const props = defineProps<{
  height: number;
  budgetMs: number;
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
const ceiling = computed(() =>
  frameCeiling(
    frames.value.map((frame) => frame.cpuMs),
    props.budgetMs,
  ),
);
const bars = computed(() =>
  buckets.value.map((bucket) => ({ ...bucket, bar: traceBar(bucket.frame, props.budgetMs, ceiling.value) })),
);
const budgetPct = computed(() => Math.min(100, (props.budgetMs / ceiling.value) * 100));
const summary = computed(() => traceSummary(frames.value, props.budgetMs));
const current = computed(() => (detail.value ? traceBar(detail.value.frame, props.budgetMs, 1) : null));
const lanes = computed(() => {
  const frame = detail.value?.frame;
  if (!frame) return [];
  const work = frame.categories.filter((lane) => lane.category !== "wait");
  const total = work.reduce((sum, lane) => sum + lane.selfMs, 0) || 1;
  return work
    .map((lane) => ({ ...lane, color: categoryColor(lane.category), pct: (lane.selfMs / total) * 100 }))
    .sort((a, b) => b.selfMs - a.selfMs);
});
function barTitle(id: number, work: number, wait: number, count: number) {
  return `Frame ${id} · ${milliseconds(work)} work · ${milliseconds(wait)} waiting${count > 1 ? ` · busiest of ${count} frames` : ""}`;
}
const rows = computed(() => spanRows(detail.value?.spans || [], collapsed.value));
const viewportHeight = computed(() => Math.max(72, props.height));
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
function spanStyle(span: TraceSpan) {
  const start = span.startMs - (detail.value?.frame.startMs || 0),
    width = range.value.end - range.value.start;
  const left = Math.max(range.value.start, start),
    end = Math.min(range.value.end, start + span.durationMs);
  return {
    left: `${((left - range.value.start) / width) * 100}%`,
    width: `${Math.max(0, ((end - left) / width) * 100)}%`,
    background: categoryColor(span.category),
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
  <section class="capture" aria-label="Frame recording" @keydown="keys">
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
      <div v-if="summary" class="facts">
        <div class="fact">
          <span class="v mono" :class="workClass(summary.p50, budgetMs)">{{ milliseconds(summary.p50) }}</span
          ><span class="k">median work</span>
        </div>
        <div class="fact">
          <span class="v mono" :class="workClass(summary.p95, budgetMs)">{{ milliseconds(summary.p95) }}</span
          ><span class="k">p95 work</span>
        </div>
        <div class="fact">
          <span class="v mono" :class="workClass(summary.worst, budgetMs)">{{ milliseconds(summary.worst) }}</span
          ><span class="k">worst work</span>
        </div>
        <div class="fact">
          <span class="v mono" :class="{ bad: summary.overBudget > 0 }">{{ summary.overBudget }}</span
          ><span class="k">over the {{ milliseconds(budgetMs) }} budget</span>
        </div>
        <div class="fact">
          <span class="v mono">{{ Math.round(summary.waitShare * 100) }} %</span
          ><span class="k">of the time waiting</span>
        </div>
      </div>
      <div
        class="overview"
        role="group"
        aria-label="Captured frames; the busiest frame of each group"
        tabindex="0"
        @keydown.left.prevent="!$event.shiftKey && moveFrame(-1)"
        @keydown.right.prevent="!$event.shiftKey && moveFrame(1)"
      >
        <div class="budget-line" :style="{ bottom: budgetPct + '%' }"></div>
        <button
          v-for="item in bars"
          :key="item.frame.id"
          class="frame-bar"
          :class="[item.bar.cls, { selected: selected === item.frame.id }]"
          :title="barTitle(item.frame.id, item.bar.work, item.bar.wait, item.count)"
          :aria-label="`Select frame ${item.frame.id}, ${milliseconds(item.bar.work)} of work`"
          :aria-pressed="selected === item.frame.id"
          @click="selectFrame(item.frame.id)"
        >
          <i class="wait" :class="{ clipped: item.bar.clipped }" :style="{ height: item.bar.waitPct + '%' }"></i
          ><i
            v-for="segment in item.bar.segments"
            :key="segment.category"
            class="seg"
            :style="{ height: segment.pct + '%', background: segment.color }"
          ></i>
        </button>
      </div>
      <div class="overview-caption">
        <span class="key"><i class="sw work"></i>work, by category</span
        ><span class="key"><i class="sw wait"></i>waiting for the next frame</span
        ><span class="key"><i class="sw budget"></i>budget</span><span class="grow"></span
        ><span>{{ frames.length }} / {{ totalFrames }} frames · top {{ ceiling }} ms</span
        ><span v-if="status && (status.droppedFrames || status.droppedSpans)" class="t-yellow"
          >dropped {{ status.droppedFrames }} frames / {{ status.droppedSpans }} spans</span
        ><button v-if="frameNext !== null" class="btn" :disabled="busy" @click="refresh(true)">Load more</button>
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
        <strong class="frame-name">Frame {{ detail.frame.id }}</strong>
        <span v-if="current" class="split mono"
          ><span class="w" :class="current.cls">{{ milliseconds(current.work) }} work</span><span class="sep">+</span
          ><span class="t-faint">{{ milliseconds(current.wait) }} waiting</span><span class="sep">=</span
          ><span class="t-dim">{{ milliseconds(detail.frame.durationMs) }}</span></span
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
        <div class="mix">
          <i v-for="lane in lanes" :key="lane.category" :style="{ width: lane.pct + '%', background: lane.color }"></i>
        </div>
        <div class="lane-keys">
          <span v-for="lane in lanes" :key="lane.category" class="lane"
            ><i :style="{ background: lane.color }"></i>{{ lane.category }}
            <span class="mono t-dim">{{ milliseconds(lane.selfMs) }}</span></span
          >
        </div>
      </div>
      <div class="call-columns">
        <span class="call-name">Ordered calls</span><span class="call-time">Self / total CPU</span>
        <div class="ruler mono">
          <span>{{ milliseconds(range.start) }}</span
          ><span>{{ milliseconds((range.start + range.end) / 2) }}</span
          ><span>{{ milliseconds(range.end) }}</span>
        </div>
      </div>
      <div class="call-scroll" @scroll="scroll = $event.target.scrollTop" role="tree" aria-label="Frame calls">
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
      <div v-for="(hint, i) in detail.frame.hints" :key="i" class="frame-hint">
        {{ hint.message }}<span v-if="hint.count > 1"> ({{ hint.count }})</span>
      </div>
    </template>
  </section>
</template>

<style scoped>
.capture {
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  color: var(--text);
  font-size: var(--fs-s);
  background: var(--surface-panel);
}
.capture-toolbar,
.detail-toolbar {
  min-height: 34px;
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-2) var(--sp-4);
  flex-wrap: wrap;
  border-bottom: 1px solid var(--line);
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
  outline: 2px solid var(--line-focus);
  outline-offset: -2px;
}
.record {
  color: var(--bad);
}
.record.recording {
  background: var(--bad);
  border-color: var(--bad);
  color: var(--surface-base);
}
.summary {
  color: var(--text);
}
.cpu-label {
  color: var(--text-muted);
  margin-left: auto;
}
.grow {
  flex: 1;
}
.feedback {
  padding: var(--sp-6) var(--sp-5);
  color: var(--text-muted);
  line-height: 1.5;
}
.failure {
  color: var(--bad);
  display: flex;
  align-items: center;
  gap: var(--sp-4);
}
.facts {
  display: flex;
  flex-direction: row;
  gap: var(--sp-6);
  padding: var(--sp-4) var(--sp-5);
  border-bottom: 1px solid var(--line-subtle);
}
.fact {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.fact .v {
  font-size: var(--fs-xl);
  color: var(--text-strong);
}
.fact .v.warn {
  color: var(--warn);
}
.fact .v.bad {
  color: var(--bad);
}
.fact .k {
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.overview {
  position: relative;
  height: 88px;
  margin: var(--sp-4) var(--sp-5) 0;
  display: flex;
  flex-direction: row;
  align-items: stretch;
  gap: var(--sp-1);
  background: var(--surface-sunken);
  border-radius: var(--r-m);
  overflow: hidden;
}
.budget-line {
  position: absolute;
  left: 0;
  right: 0;
  height: 1px;
  background: var(--chart-guide);
}
.frame-bar {
  flex: 1;
  min-width: 2px;
  max-width: 14px;
  border: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  background: transparent;
  opacity: 0.8;
}
.frame-bar:hover {
  opacity: 1;
  background: var(--surface-raised);
}
.frame-bar.selected {
  opacity: 1;
  background: var(--surface-active);
}
.frame-bar .wait {
  background: var(--chart-wait);
}
.frame-bar .wait.clipped {
  border-top: 2px solid var(--line-strong);
}
.frame-bar .seg {
  min-height: 0;
}
.frame-bar.bad .wait {
  border-bottom: 2px solid var(--bad);
}
.key {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.sw {
  width: 8px;
  height: 8px;
  border-radius: var(--r-s);
}
.sw.work {
  background: var(--cat-js);
}
.sw.wait {
  background: var(--chart-wait);
  border: 1px solid var(--line-strong);
}
.sw.budget {
  height: 1px;
  background: var(--text-muted);
}
.overview-caption,
.detail-footer {
  display: flex;
  align-items: center;
  gap: var(--sp-4);
  min-height: 25px;
  padding: var(--sp-2) var(--sp-4);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  flex-wrap: wrap;
}
.frame-name {
  font-weight: 600;
  color: var(--text-strong);
}
.split {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-xs);
}
.split .w {
  color: var(--text);
}
.split .w.warn {
  color: var(--warn);
}
.split .w.bad {
  color: var(--bad);
}
.sep {
  color: var(--text-ghost);
}
.lanes {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4) var(--sp-5);
  border-bottom: 1px solid var(--line-subtle);
  font-size: var(--fs-xs);
}
.mix {
  display: flex;
  flex-direction: row;
  height: 6px;
  border-radius: var(--r-s);
  overflow: hidden;
  background: var(--surface-raised);
}
.lane-keys {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-5);
  color: var(--text);
}
.lane {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.lane i {
  width: 8px;
  height: 8px;
  border-radius: var(--r-s);
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
  color: var(--text-muted);
  background: var(--surface-raised);
  border-bottom: 1px solid var(--line);
  font-size: var(--fs-xs);
}
.call-name {
  width: 34%;
  min-width: 100px;
  flex-shrink: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding-left: var(--sp-4);
  box-sizing: border-box;
  display: flex;
  align-items: center;
  gap: var(--sp-2);
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
  padding-right: var(--sp-4);
  box-sizing: border-box;
  color: var(--text);
  font-size: var(--fs-xs);
}
.ruler {
  flex: 1;
  min-width: 0;
  display: flex;
  justify-content: space-between;
  padding: 0 var(--sp-2);
}
.capture > * {
  flex-shrink: 0;
}
.capture > .call-scroll {
  flex: 1 1 0px;
  min-height: 72px;
  overflow-y: auto;
}
.call-row {
  height: 24px;
  border: 0;
  padding: 0;
  background: var(--surface-panel);
  color: var(--text);
  text-align: left;
  font-size: var(--fs-s);
}
.call-row:hover {
  background: var(--surface-hover);
}
.call-row.chosen {
  background: var(--surface-active);
}
.call-track {
  height: 24px;
  flex: 1;
  min-width: 30px;
  position: relative;
  overflow: hidden;
  border-left: 1px solid var(--line);
}
.call-track i {
  position: absolute;
  top: 6px;
  height: 12px;
  min-width: 1px;
  border-radius: var(--r-s);
}
.selection-detail {
  border-top: 1px solid var(--line);
  background: var(--surface-base);
  padding: var(--sp-4);
  min-height: 55px;
  color: var(--text);
  line-height: 1.5;
}
.selection-detail > div:first-child {
  display: flex;
  align-items: center;
  gap: var(--sp-4);
  flex-wrap: wrap;
}
.selection-detail strong {
  color: var(--text-strong);
}
.selection-detail p {
  margin: var(--sp-2) 0;
  color: var(--warn);
}
.callsite {
  overflow-wrap: anywhere;
  color: var(--text);
}
.counter {
  margin-right: var(--sp-5);
}
.muted {
  color: var(--text-muted);
}
.frame-hint {
  padding: var(--sp-2) var(--sp-4);
  color: var(--warn);
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
