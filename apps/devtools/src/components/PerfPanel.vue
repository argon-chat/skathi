<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import FrameTimeline from "./FrameTimeline.vue";
import AllocationsPanel from "./AllocationsPanel.vue";
import type { InspectorRequest } from "../inspector-model";
import { forceGc, measureResident, refreshMem, refreshPerf, state } from "../store";
import { GPU_COLOR, formatMem, historyColumns, itemsOf, memColor, percent, segments, totalLabel } from "../memfmt";

const props = defineProps<{
  height: number;
  request?: InspectorRequest | undefined;
  sessionKey?: string | number | undefined;
}>();
const emit = defineEmits<{ (event: "recording-change", active: boolean): void }>();
const mode = ref<"overview" | "frames" | "allocations">("overview");
const frameRecording = ref(false),
  allocationRecording = ref(false);
const recording = computed(() => frameRecording.value || allocationRecording.value);
function frameRecordingChanged(active: boolean) {
  frameRecording.value = active;
  emit("recording-change", recording.value);
}
function allocationRecordingChanged(active: boolean) {
  allocationRecording.value = active;
  emit("recording-change", recording.value);
}

const perf = computed(() => state.perf);
const samples = computed(() => state.perfHistory);

const ceiling = computed(() => {
  const worst = samples.value.length ? Math.max(...samples.value) : 0;
  return Math.max(20, Math.ceil(worst / 10) * 10);
});

function barHeight(ms: number): number {
  return Math.max(1, Math.min(100, (ms / ceiling.value) * 100));
}

/**
 * A bar's colour is about the DISPLAY, not a fixed 16.7 ms budget: a 165 Hz monitor has a 6 ms
 * frame and a 60 Hz one has 16.7, and painting the first red for missing the second would be a
 * lie.
 */
const reference = computed(() => perf.value?.budgetMs || perf.value?.p50Ms || 0);

function barClass(ms: number): string {
  const ref = reference.value;
  if (!ref) return "";
  if (ms > ref * 3) return "bad";
  if (ms > ref * 1.6) return "warn";
  return "";
}

function seconds(ms: number | null | undefined): string {
  if (!ms && ms !== 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.floor(ms / 60_000)} m ${Math.round((ms % 60_000) / 1000)} s`;
}

const mem = computed(() => state.mem);
const memTooOld = computed(() => /unknown command/i.test(state.memError));
const bar = computed(() => (mem.value ? segments(mem.value) : []));
const keys = computed(() => (mem.value ? mem.value.categories.map((c) => c.key) : []));
const history = computed(() => historyColumns(state.memHistory, keys.value));
const gpuTotal = computed(() => (mem.value ? mem.value.gpu.dedicatedBytes + mem.value.gpu.sharedBytes : 0));
const open = reactive<Record<string, boolean>>({});

function toggle(key: string): void {
  open[key] = !open[key];
}

function residentCat(key: string) {
  return state.memResident?.breakdown.categories.find((c) => c.key === key);
}
function residentOf(key: string): string {
  const c = residentCat(key);
  return c?.residentBytes === undefined ? "" : formatMem(c.residentBytes);
}
function residentTitle(key: string): string {
  const c = residentCat(key);
  if (c?.residentBytes === undefined) return "";
  return `in RAM: ${formatMem(c.residentBytes)}, of which private ${formatMem(c.residentPrivateBytes)}`;
}
</script>

<template>
  <section class="pane perf" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Performance</span>
      <span
        class="chip"
        :title="
          mode === 'overview' ? 'The rolling frame clock is always on' : 'Record CPU work without restarting the target'
        "
        >{{ mode === "overview" ? "live frame clock" : "CPU capture" }}</span
      >
      <div class="spacer"></div>
      <span
        v-if="mode === 'overview' && perf && !perf.phases"
        class="chip"
        title="VN_PERF=1 in the target's environment"
        >phase breakdown off · VN_PERF</span
      >
      <span v-else-if="mode === 'overview' && perf" class="chip ok">VN_PERF on · see the target's stderr</span>
      <button
        v-if="mode === 'overview'"
        class="btn"
        :disabled="recording || state.captureActive"
        @click="refreshPerf()"
        aria-label="Refresh frame clock"
      >
        Refresh
      </button>
    </header>

    <nav class="perf-tabs" aria-label="Performance views">
      <button
        class="btn"
        :class="{ on: mode === 'overview' }"
        :disabled="recording && mode !== 'overview'"
        :aria-pressed="mode === 'overview'"
        @click="mode = 'overview'"
      >
        Overview
      </button>
      <button
        class="btn"
        :class="{ on: mode === 'frames' }"
        :disabled="recording && mode !== 'frames'"
        :aria-pressed="mode === 'frames'"
        @click="mode = 'frames'"
      >
        Frames
      </button>
      <button
        class="btn"
        :class="{ on: mode === 'allocations' }"
        :disabled="recording && mode !== 'allocations'"
        :aria-pressed="mode === 'allocations'"
        @click="mode = 'allocations'"
      >
        Allocations
      </button>
      <span v-if="recording" class="recording-note">Stop the recording before switching views</span>
    </nav>
    <FrameTimeline
      v-if="mode === 'frames'"
      :height="Math.max(0, props.height - 57)"
      :request="request"
      :session-key="sessionKey"
      @recording-change="frameRecordingChanged"
    />
    <AllocationsPanel
      v-else-if="mode === 'allocations'"
      :height="Math.max(0, props.height - 57)"
      :request="request"
      :session-key="sessionKey"
      @recording-change="allocationRecordingChanged"
    />
    <div v-else class="body scroll" :style="{ height: Math.max(0, props.height - 57) + 'px' }">
      <div v-if="!perf" class="empty">
        Waiting for the first frame sample. If this stays empty the push channel is not connected — the titlebar says
        <b>polling</b> when that happens.
      </div>

      <template v-else>
        <div class="stats">
          <div class="stat big">
            <span class="v">{{ perf.fps.toFixed(1) }}</span
            ><span class="u">fps</span>
            <span class="k">average over the ring</span>
          </div>
          <div class="stat">
            <span class="v">{{ perf.p50Ms.toFixed(2) }}</span
            ><span class="u">ms</span>
            <span class="k">p50 frame</span>
          </div>
          <div v-if="perf.budgetMs" class="stat">
            <span class="v">{{ perf.budgetMs.toFixed(2) }}</span
            ><span class="u">ms</span>
            <span class="k">frame budget · display</span>
          </div>
          <div class="stat">
            <span class="v" :class="{ warn: perf.p95Ms > reference * 1.6 }">{{ perf.p95Ms.toFixed(2) }}</span
            ><span class="u">ms</span>
            <span class="k">p95 frame</span>
          </div>
          <div class="stat">
            <span class="v" :class="{ bad: perf.maxMs > reference * 3 }">{{ perf.maxMs.toFixed(2) }}</span
            ><span class="u">ms</span>
            <span class="k">worst frame</span>
          </div>
          <div class="stat">
            <span class="v">{{ perf.nodes }}</span>
            <span class="k">live tree nodes</span>
          </div>
          <div class="stat">
            <span class="v">{{ perf.windows }}</span>
            <span class="k">OS windows</span>
          </div>
          <div class="stat">
            <span class="v">{{ seconds(perf.bootMs) }}</span>
            <span class="k">boot → first frame</span>
          </div>
          <div class="stat">
            <span class="v">{{ seconds(perf.uptimeMs) }}</span>
            <span class="k">uptime · {{ perf.frames }} frames</span>
          </div>
        </div>

        <div class="chart">
          <div class="axis">
            <span class="tick">{{ ceiling }} ms</span>
            <span class="tick mid">{{ Math.round(ceiling / 2) }}</span>
            <span class="tick">0</span>
          </div>
          <div class="bars">
            <div
              v-for="(ms, i) in samples"
              :key="i"
              class="bar"
              :class="barClass(ms)"
              :style="{ height: barHeight(ms) + '%' }"
              :title="ms.toFixed(2) + ' ms'"
            ></div>
            <div v-if="!samples.length" class="empty">collecting…</div>
          </div>
        </div>
        <div class="legend">
          one bar per push · four a second · a bar is amber past 1.6× the median frame and red past 3×, because a hitch
          is relative to what this machine has been managing
        </div>
      </template>

      <div class="mem-head">
        <span class="label">Memory</span>
        <template v-if="mem">
          <span class="chip" :title="mem.basis">{{ totalLabel(mem) }} {{ formatMem(mem.totalBytes) }}</span>
          <span
            class="chip"
            title="Windows: the commit charge (Task Manager: Commit size). macOS: the kernel's physical footprint (Activity Monitor: Memory)"
            >private {{ formatMem(mem.privateBytes) }}</span
          >
          <span class="chip" title="Windows: Task Manager's Memory column. macOS: anonymous memory in RAM (internal)"
            >private working set {{ formatMem(mem.privateWorkingSetBytes) }}</span
          >
          <span class="chip">working set {{ formatMem(mem.workingSetBytes) }}</span>
          <span
            v-if="!mem.rustCounting"
            class="chip warn"
            title="The Rust heap is counted by default in a dev build; set VN_MEMSTATS=1 in the target's environment to count it here"
            >Rust heap: off · VN_MEMSTATS=1</span
          >
        </template>
        <div class="spacer"></div>
        <button
          class="btn"
          :disabled="recording || state.captureActive || !state.target"
          @click="refreshMem()"
          title="Request a fresh memory breakdown"
        >
          Refresh memory
        </button>
        <span v-if="mem" class="chip" title="How long the target took to compute the breakdown"
          >{{ state.memCollectMs.toFixed(1) }} ms · on request</span
        >
        <div
          class="btn"
          :class="{ on: state.memResidentBusy }"
          title="Put every working-set page (RAM) in its category: mem --regions. Walks the target's heap, so it is taken on request, not every 2 s."
          @click="!recording && !state.captureActive && measureResident()"
        >
          {{ state.memResidentBusy ? "measuring…" : "Measure RAM" }}
        </div>
        <div
          class="btn"
          :class="{ on: state.memGcBusy }"
          title="Two full synchronous JSC collections, then free pages back to the OS (mem --gc). Changes the target's state."
          @click="!recording && !state.captureActive && forceGc()"
        >
          {{ state.memGcBusy ? "collecting…" : "Force GC" }}
        </div>
      </div>

      <div v-if="state.memUnsupported && !mem" class="empty">
        This target has no memory breakdown: {{ state.memUnsupported }}.
      </div>
      <div v-else-if="state.memError && !mem" class="empty">
        The target did not answer <b>mem</b>: {{ state.memError }}.<template v-if="memTooOld">
          An older host has no memory breakdown.</template
        >
      </div>
      <div v-else-if="!mem" class="empty">Asking the target where its memory is…</div>

      <template v-else>
        <div class="stack">
          <div
            v-for="s in bar"
            :key="s.key"
            class="seg"
            :style="{ width: s.pct + '%', background: s.color }"
            :title="`${s.label} · ${formatMem(s.bytes)} · ${s.pct.toFixed(1)} %`"
          ></div>
        </div>

        <div class="mem-grid">
          <div class="table">
            <div class="row head">
              <span class="tw"></span><span class="name"></span>
              <span class="num" :title="mem.basis">{{ totalLabel(mem) }}</span>
              <span class="pct">share</span>
              <span
                class="num res"
                :title="
                  state.memResident
                    ? 'In RAM (working set), measured at ' +
                      new Date(state.memResident.at).toLocaleTimeString() +
                      ' — the private part is what Task Manager\'s Memory column counts'
                    : 'Press Measure RAM'
                "
                >{{ state.memResident ? "in RAM" : "" }}</span
              >
            </div>
            <template v-for="c in mem.categories" :key="c.key">
              <div class="row cat" @click="toggle(c.key)">
                <span class="tw">{{ c.items.length ? (open[c.key] ? "▾" : "▸") : "" }}</span>
                <span class="sw" :style="{ background: memColor(c.key) }"></span>
                <span class="name">{{ c.label }}</span>
                <span class="num">{{ formatMem(c.bytes) }}</span>
                <span class="pct">{{ percent(c.bytes, mem.totalBytes) }}</span>
                <span class="num res" :title="residentTitle(c.key)">{{ residentOf(c.key) }}</span>
              </div>
              <template v-if="open[c.key]">
                <div class="row note">
                  <span class="tw"></span><span class="name">{{ c.note }}</span>
                </div>
                <div
                  v-for="(it, j) in itemsOf(c)"
                  :key="j"
                  class="row sub"
                  :class="{ info: it.info }"
                  :title="it.info ? 'Not a slice: another accountant\'s figure, inside the slices above' : ''"
                >
                  <span class="tw"></span>
                  <span class="name">{{ it.label }}</span>
                  <span class="num">{{ formatMem(it.bytes) }}</span>
                  <span class="pct">{{ it.info ? "" : percent(it.bytes, c.bytes) }}</span>
                  <span class="num res"></span>
                </div>
              </template>
              <div v-else-if="c.key === 'other'" class="row note">
                <span class="tw"></span><span class="name">{{ c.note }}</span>
              </div>
            </template>

            <div class="row cat gpu" @click="toggle('gpu')">
              <span class="tw">{{ open.gpu ? "▾" : "▸" }}</span>
              <span class="sw" :style="{ background: GPU_COLOR }"></span>
              <span class="name">{{
                mem.gpu.inBar
                  ? "GPU (unified memory: already in the bar, as graphics)"
                  : "GPU (not in the bar: outside the address space)"
              }}</span>
              <span class="num">{{ formatMem(gpuTotal) }}</span>
              <span class="pct"></span><span class="num res"></span>
            </div>
            <template v-if="open.gpu">
              <div class="row sub">
                <span class="tw"></span><span class="name">dedicated (VRAM)</span>
                <span class="num">{{ formatMem(mem.gpu.dedicatedBytes) }}</span
                ><span class="pct"></span><span class="num res"></span>
              </div>
              <div class="row sub">
                <span class="tw"></span><span class="name">shared (system memory)</span>
                <span class="num">{{ formatMem(mem.gpu.sharedBytes) }}</span
                ><span class="pct"></span><span class="num res"></span>
              </div>
              <div v-for="(it, j) in mem.gpu.items" :key="'g' + j" class="row sub info">
                <span class="tw"></span><span class="name">{{ it.label }}</span>
                <span class="num">{{ formatMem(it.bytes) }}</span
                ><span class="pct"></span><span class="num res"></span>
              </div>
            </template>
          </div>

          <div class="history">
            <div class="hchart">
              <div class="axis">
                <span class="tick">{{ formatMem(history.ceiling) }}</span>
                <span class="tick">0</span>
              </div>
              <div class="cols">
                <div v-for="(col, i) in history.columns" :key="i" class="hcol" :title="col.title">
                  <div
                    v-for="p in [...col.parts].reverse()"
                    :key="p.key"
                    class="hpart"
                    :style="{ height: p.pct + '%', background: p.color }"
                  ></div>
                </div>
                <div v-if="history.columns.length < 2" class="empty">collecting…</div>
              </div>
            </div>
            <div class="legend">
              {{ totalLabel(mem) }} memory by category · one column per requested sample · bounded history
            </div>
          </div>
        </div>
      </template>
    </div>
  </section>
</template>

<style scoped>
.perf-tabs {
  min-height: 30px;
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 8px;
  box-sizing: border-box;
  border-bottom: 1px solid #232c3a;
  background: #0f1620;
}
.perf-tabs button {
  font-family: inherit;
  cursor: pointer;
}
.perf-tabs button:disabled {
  opacity: 0.45;
  cursor: default;
}
.perf-tabs button:focus-visible {
  outline: 2px solid #6cb6ff;
  outline-offset: -2px;
}
.recording-note {
  color: #e3b341;
  font-size: 10px;
  margin-left: auto;
}

.perf {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.scroll {
  overflow-y: auto;
  overflow-x: hidden;
  min-height: 0;
}
.body {
  display: flex;
  flex-direction: column;
  padding: 12px;
  gap: 12px;
}
.stats {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 8px;
}
.stat {
  display: flex;
  flex-direction: column;
  min-width: 96px;
  padding: 8px;
  border-radius: 6px;
  background: #131a24;
  border-width: 1px;
  border-color: #1e2733;
}
.stat.big {
  min-width: 130px;
  background: #10202e;
  border-color: #244566;
}
.v {
  font-size: 19px;
  color: #d7e0ea;
  font-family: "Cascadia Mono", "Consolas", monospace;
}
.stat.big .v {
  font-size: 26px;
  color: #6cb6ff;
}
.v.warn {
  color: #e3b341;
}
.v.bad {
  color: #ff7b72;
}
.u {
  font-size: 10px;
  color: #5c6b80;
  margin-left: 3px;
}
.k {
  font-size: 10px;
  color: #7f8ea3;
  margin-top: 3px;
}
.chart {
  height: 120px;
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  gap: 6px;
  padding: 8px;
  border-radius: 6px;
  background: #0c1119;
  border-width: 1px;
  border-color: #1e2733;
}
.axis {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  width: 52px;
  flex-shrink: 0;
}
.tick {
  font-size: 9px;
  color: #5c6b80;
  text-align: right;
}
.bars {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: row;
  align-items: flex-end;
  gap: 1px;
}
.bar {
  flex: 1;
  min-width: 1px;
  background: #2f6ea8;
  border-radius: 1px;
}
.bar.warn {
  background: #b8891f;
}
.bar.bad {
  background: #cf4b43;
}
.legend {
  font-size: 10px;
  color: #5c6b80;
  line-height: 15px;
}

.mem-head {
  display: flex;
  flex-direction: row;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding-top: 8px;
  border-top-width: 1px;
  border-color: #1e2733;
}
.stack {
  height: 16px;
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  border-radius: 4px;
  overflow: hidden;
  background: #0c1119;
  border-width: 1px;
  border-color: #1e2733;
}
.seg {
  height: 100%;
}
.mem-grid {
  display: flex;
  flex-direction: row;
  gap: 12px;
  align-items: flex-start;
}
.table {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.row {
  display: flex;
  flex-direction: row;
  align-items: center;
  height: 20px;
  gap: 6px;
  font-size: 11px;
  color: #b9c6d6;
}
.row.cat {
  border-radius: 3px;
}
.row.cat:hover {
  background: #171e29;
}
.row.sub {
  color: #8494a8;
  padding-left: 18px;
}
.row.sub.info {
  color: #5c6b80;
  font-style: italic;
}
.row.note {
  color: #5c6b80;
  font-size: 10px;
  padding-left: 18px;
}
.tw {
  width: 10px;
  flex-shrink: 0;
  font-size: 9px;
  color: #5c6b80;
}
.sw {
  width: 9px;
  height: 9px;
  border-radius: 2px;
  flex-shrink: 0;
}
.name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.num {
  width: 72px;
  flex-shrink: 0;
  text-align: right;
  font-family: "Cascadia Mono", "Consolas", monospace;
}
.num.res {
  color: #76e3ea;
}
.row.head {
  height: 16px;
  font-size: 9px;
  color: #5c6b80;
  letter-spacing: 0.4px;
  text-transform: uppercase;
}
.row.head .num,
.row.head .pct {
  font-family: inherit;
  color: #5c6b80;
}
.pct {
  width: 54px;
  flex-shrink: 0;
  text-align: right;
  color: #5c6b80;
  font-family: "Cascadia Mono", "Consolas", monospace;
}
.history {
  width: 40%;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.hchart {
  height: 140px;
  display: flex;
  flex-direction: row;
  gap: 6px;
  padding: 8px;
  border-radius: 6px;
  background: #0c1119;
  border-width: 1px;
  border-color: #1e2733;
}
.cols {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: row;
  align-items: flex-end;
  gap: 0;
}
.hcol {
  flex: 1;
  min-width: 1px;
  height: 100%;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
}
.hpart {
  width: 100%;
}
</style>
