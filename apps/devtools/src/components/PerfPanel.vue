<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import FrameTimeline from "./FrameTimeline.vue";
import AllocationsPanel from "./AllocationsPanel.vue";
import type { InspectorRequest } from "../inspector-model";
import { forceGc, measureResident, refreshMem, refreshPerf, state } from "../store";
import { GPU_COLOR, formatMem, historyColumns, itemsOf, memColor, percent, segments, totalLabel } from "../memfmt";
import { FALLBACK_BUDGET_MS, frameBars, frameCeiling, hasFrameWork, share, workClass } from "../perffmt";

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

// A frame split into its work and the wait after it; an older target only reports intervals.
const split = computed(() => !!perf.value && hasFrameWork(perf.value));
const budget = computed(() => perf.value?.budgetMs || FALLBACK_BUDGET_MS);
const frameTop = computed(() => (perf.value && split.value ? frameCeiling(perf.value.work ?? [], budget.value) : 0));
const frames = computed(() => (perf.value && split.value ? frameBars(perf.value, budget.value, frameTop.value) : []));
const budgetPct = computed(() => (frameTop.value ? Math.min(100, (budget.value / frameTop.value) * 100) : 0));

function fixed(ms: number | undefined): string {
  return ms === undefined ? "—" : ms.toFixed(2);
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
        title="SK_PERF=1 in the target's environment"
        >phase breakdown off · SK_PERF</span
      >
      <span v-else-if="mode === 'overview' && perf" class="chip ok">SK_PERF on · see the target's stderr</span>
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
      :budget-ms="budget"
      @recording-change="frameRecordingChanged"
    />
    <AllocationsPanel
      v-else-if="mode === 'allocations'"
      :height="Math.max(0, props.height - 57)"
      :request="request"
      :session-key="sessionKey"
      @recording-change="allocationRecordingChanged"
    />
    <div v-else class="body scroll">
      <div v-if="!perf" class="empty">
        Waiting for the first frame sample. If this stays empty the push channel is not connected — the titlebar says
        <b>polling</b> when that happens.
      </div>

      <template v-else-if="split">
        <div class="stats">
          <div class="stat big" title="Main-thread time of a frame: callbacks, the JS pump, layout, draw and present">
            <span class="v">{{ fixed(perf.workP50Ms) }}</span
            ><span class="u">ms</span>
            <span class="k">frame work · p50</span>
          </div>
          <div class="stat">
            <span class="v" :class="workClass(perf.workP95Ms ?? 0, budget)">{{ fixed(perf.workP95Ms) }}</span
            ><span class="u">ms</span>
            <span class="k">frame work · p95</span>
          </div>
          <div class="stat">
            <span class="v" :class="workClass(perf.workMaxMs ?? 0, budget)">{{ fixed(perf.workMaxMs) }}</span
            ><span class="u">ms</span>
            <span class="k">frame work · worst</span>
          </div>
          <div class="stat" title="Frames after which the clock slowed down because nothing was happening">
            <span class="v">{{ share(perf.pacedShare) }}</span>
            <span class="k">idle-paced frames</span>
          </div>
        </div>
        <div class="meta">
          <span>budget {{ budget.toFixed(2) }} ms{{ perf.budgetMs ? "" : " (assumed 60 Hz)" }}</span>
          <span data-metric="fps" title="Frames per second over the ring, waiting included: low while idle on purpose"
            >{{ perf.fps.toFixed(1) }} fps with waiting</span
          >
          <span title="Frames that drew; the rest ran the pump with nothing to repaint"
            >{{ share(perf.drewShare) }} drew</span
          >
          <span>{{ perf.nodes }} nodes</span>
          <span>boot {{ seconds(perf.bootMs) }}</span>
          <span>up {{ seconds(perf.uptimeMs) }}</span>
        </div>

        <div class="chart">
          <div class="axis">
            <span class="tick">{{ frameTop }} ms</span>
            <span class="tick mid">{{ frameTop / 2 }}</span>
            <span class="tick">0</span>
          </div>
          <div class="bars frames">
            <div
              class="budget-line"
              :style="{ bottom: budgetPct + '%' }"
              :title="`frame budget ${budget.toFixed(2)} ms`"
            ></div>
            <div v-for="(f, i) in frames" :key="i" class="fbar" :title="f.title">
              <div class="part work" :class="[f.cls, { quiet: !f.drew }]" :style="{ height: f.workPct + '%' }"></div>
              <div class="part wait" :class="{ paced: f.paced }" :style="{ height: f.waitPct + '%' }"></div>
            </div>
            <div v-if="!frames.length" class="empty">collecting…</div>
          </div>
        </div>
        <div class="intervals">
          frame to frame, waiting included: p50 {{ perf.p50Ms.toFixed(2) }} · p95 {{ perf.p95Ms.toFixed(2) }} · worst
          {{ perf.maxMs.toFixed(2) }} ms<template v-if="perf.throttledShare">
            · {{ share(perf.throttledShare) }} of frames held a request back at the frame-rate limit</template
          >
        </div>
        <div class="legend keyed">
          <span class="key"><i class="sw work"></i>frame work</span>
          <span class="key"><i class="sw work quiet"></i>work, nothing to redraw</span>
          <span class="key"><i class="sw work warn"></i>past 60 % of the budget</span>
          <span class="key"><i class="sw work bad"></i>past the budget</span>
          <span class="key"><i class="sw wait paced"></i>waiting: idle pacing</span>
          <span class="key"><i class="sw wait"></i>waiting for the next frame</span>
          <span class="key"><i class="sw line"></i>display frame budget</span>
        </div>
        <div class="legend">
          one bar per frame, the last {{ frames.length }} · only the work counts against the budget: a long bar of
          waiting is the clock resting, not a slow frame
        </div>
      </template>

      <template v-else>
        <div class="stats">
          <div class="stat big">
            <span class="v" data-metric="fps">{{ perf.fps.toFixed(1) }}</span
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
            title="The Rust heap is counted by default in a dev build; set SK_MEMSTATS=1 in the target's environment to count it here"
            >Rust heap: off · SK_MEMSTATS=1</span
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
  gap: var(--sp-2);
  padding: var(--sp-2) var(--sp-4);
  box-sizing: border-box;
  border-bottom: 1px solid var(--line);
  background: var(--surface-panel);
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
  outline: 2px solid var(--line-focus);
  outline-offset: -2px;
}
.recording-note {
  color: var(--warn);
  font-size: var(--fs-xs);
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
  flex: 1;
  padding: var(--sp-5);
  gap: var(--sp-5);
}
.stats {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: var(--sp-4);
}
.stat {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  align-items: baseline;
  min-width: 112px;
  padding: var(--sp-4) var(--sp-5);
  border-radius: var(--r-l);
  background: var(--surface-raised);
}
.stat .k {
  width: 100%;
}
.v {
  font-size: var(--fs-2xl);
  color: var(--text);
  font-family: var(--font-mono);
}
.stat.big .v {
  color: var(--text-strong);
}
.v.warn {
  color: var(--warn);
}
.v.bad {
  color: var(--bad);
}
.u {
  font-size: var(--fs-xs);
  color: var(--text-faint);
  margin-left: var(--sp-1);
}
.k {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-top: var(--sp-1);
}
.chart {
  height: 120px;
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  gap: var(--sp-3);
  padding: var(--sp-4);
  border-radius: var(--r-l);
  background: var(--surface-base);
  border-width: 1px;
  border-color: var(--line);
}
.axis {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  width: 52px;
  flex-shrink: 0;
}
.tick {
  font-size: var(--fs-2xs);
  color: var(--text-faint);
  text-align: right;
}
.bars {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: row;
  align-items: flex-end;
  gap: 0;
}
.bar {
  flex: 1;
  min-width: 1px;
  background: var(--chart-work-quiet);
  border-radius: var(--r-s);
}
.bar.warn {
  background: var(--warn);
}
.bar.bad {
  background: var(--bad);
}
.bars.frames {
  position: relative;
  gap: 0;
}
.fbar {
  flex: 1;
  min-width: 1px;
  height: 100%;
  display: flex;
  flex-direction: column-reverse;
  padding-right: 0;
}
.part {
  width: 100%;
  flex-shrink: 0;
}
.part.work {
  min-height: 1px;
  background: var(--chart-work);
  border-radius: var(--r-s);
}
.part.work.quiet {
  background: var(--chart-work-quiet);
}
.part.work.warn {
  background: var(--warn);
}
.part.work.bad {
  background: var(--bad);
}
.part.wait {
  background: var(--chart-wait);
}
.part.wait.paced {
  background: var(--chart-wait-paced);
}
.budget-line {
  position: absolute;
  left: 0;
  right: 0;
  height: 1px;
  background: var(--chart-guide);
}
.meta {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: var(--sp-5);
  margin-top: var(--sp-3);
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.intervals {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  line-height: 15px;
}
.legend.keyed {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: var(--sp-5);
}
.key {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
}
.sw.work {
  background: var(--chart-work);
}
.sw.work.quiet {
  background: var(--chart-work-quiet);
}
.sw.work.warn {
  background: var(--warn);
}
.sw.work.bad {
  background: var(--bad);
}
.sw.wait {
  background: var(--chart-wait);
  border-width: 1px;
  border-color: var(--line-strong);
}
.sw.wait.paced {
  background: var(--chart-wait-paced);
  border-color: var(--ok-line);
}
.sw.line {
  height: 2px;
  background: var(--surface-handle);
}
.legend {
  font-size: var(--fs-xs);
  color: var(--text-faint);
  line-height: 15px;
}

.mem-head {
  display: flex;
  flex-direction: row;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-3);
  padding-top: var(--sp-4);
  border-top-width: 1px;
  border-color: var(--line);
}
.stack {
  height: 16px;
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  border-radius: var(--r-m);
  overflow: hidden;
  background: var(--surface-base);
  border-width: 1px;
  border-color: var(--line);
}
.seg {
  height: 100%;
}
.mem-grid {
  display: flex;
  flex-direction: row;
  gap: var(--sp-5);
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
  height: var(--h-row);
  gap: var(--sp-3);
  font-size: var(--fs-s);
  color: var(--text);
}
.row.cat {
  border-radius: var(--r-s);
}
.row.cat:hover {
  background: var(--surface-raised);
}
.row.sub {
  color: var(--text-muted);
  padding-left: var(--sp-6);
}
.row.sub.info {
  color: var(--text-faint);
  font-style: italic;
}
.row.note {
  color: var(--text-faint);
  font-size: var(--fs-xs);
  padding-left: var(--sp-6);
}
.tw {
  width: 10px;
  flex-shrink: 0;
  font-size: var(--fs-2xs);
  color: var(--text-faint);
}
.sw {
  width: 9px;
  height: 9px;
  border-radius: var(--r-s);
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
  font-family: var(--font-mono);
}
.num.res {
  color: var(--cyan);
}
.row.head {
  height: 16px;
  font-size: var(--fs-2xs);
  color: var(--text-faint);
  letter-spacing: 0.4px;
  text-transform: uppercase;
}
.row.head .num,
.row.head .pct {
  font-family: inherit;
  color: var(--text-faint);
}
.pct {
  width: 54px;
  flex-shrink: 0;
  text-align: right;
  color: var(--text-faint);
  font-family: var(--font-mono);
}
.history {
  width: 40%;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.hchart {
  height: 140px;
  display: flex;
  flex-direction: row;
  gap: var(--sp-3);
  padding: var(--sp-4);
  border-radius: var(--r-l);
  background: var(--surface-base);
  border-width: 1px;
  border-color: var(--line);
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
