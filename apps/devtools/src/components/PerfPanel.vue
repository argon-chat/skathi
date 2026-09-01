<script setup lang="ts">
// Performance: the target's frame clock, live.
//
// ## Where the numbers come from
//
// `crates/vn-app/src/perf.rs` has had a frame profiler since milestone 11, but
// it is a REPORT: `VN_PERF=1` prints a summary to stderr every second, and you
// have to have decided to turn it on before the run you now wish you had
// measured. v2 adds a second, unconditional, fixed-size ring beside it — one
// `Instant` and one array store per frame — so the question "why is this
// janky" can be asked of a host that is already running and already janky.
//
// The push channel sends a sample four times a second while this panel is
// open, and stops when it is not (`store.streams()`): a sparkline nobody is
// looking at is not worth 4 messages a second.
//
// ## What it deliberately does not show
//
// The per-phase breakdown — restyle / build / layout / paint — belongs to
// `VN_PERF`, which OWNS those counters and clears them when it reports
// (`vn_tree::perf::take`). Reading them here would silently break the stderr
// report of a host somebody is profiling. So the panel says whether the probe
// is on and points at the environment variable instead of showing zeros that
// read as "nothing costs anything".
import { computed } from "vue";
import { refreshPerf, state } from "../store";

const props = defineProps<{ height: number }>();

const perf = computed(() => state.perf);
const samples = computed(() => state.perfHistory);

/** The chart's ceiling: a frame budget, or the worst sample if it is worse. */
const ceiling = computed(() => {
  const worst = samples.value.length ? Math.max(...samples.value) : 0;
  return Math.max(20, Math.ceil(worst / 10) * 10);
});

function barHeight(ms: number): number {
  return Math.max(1, Math.min(100, (ms / ceiling.value) * 100));
}

/**
 * A bar's colour is about the DISPLAY, not a fixed 16.7 ms budget: a 165 Hz
 * monitor has a 6 ms frame and a 60 Hz one has 16.7, and painting the first
 * red for missing the second would be a lie. So the reference is the window's
 * own median — a bar is "slow" when it is well past what this machine has been
 * managing, which is what a hitch actually is.
 */
const median = computed(() => perf.value?.p50Ms || 0);

function barClass(ms: number): string {
  const p50 = median.value;
  if (!p50) return "";
  if (ms > p50 * 3) return "bad";
  if (ms > p50 * 1.6) return "warn";
  return "";
}

function seconds(ms: number | null | undefined): string {
  if (!ms && ms !== 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.floor(ms / 60_000)} m ${Math.round((ms % 60_000) / 1000)} s`;
}
</script>

<template>
  <section class="pane perf" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Performance</span>
      <span class="chip" title="The rolling ring is always on — no environment variable needed"
        >live frame clock</span
      >
      <div class="spacer"></div>
      <span v-if="perf && !perf.phases" class="chip" title="VN_PERF=1 in the target's environment"
        >phase breakdown off · VN_PERF</span
      >
      <span v-else-if="perf" class="chip ok">VN_PERF on · see the target's stderr</span>
      <div class="btn" @click="refreshPerf()">↻</div>
    </header>

    <div v-if="!perf" class="empty">
      Waiting for the first frame sample. If this stays empty the push channel is not connected —
      the titlebar says <b>polling</b> when that happens.
    </div>

    <div v-else class="body">
      <div class="stats">
        <div class="stat big">
          <span class="v">{{ perf.fps.toFixed(1) }}</span><span class="u">fps</span>
          <span class="k">average over the ring</span>
        </div>
        <div class="stat">
          <span class="v">{{ perf.p50Ms.toFixed(2) }}</span><span class="u">ms</span>
          <span class="k">p50 frame</span>
        </div>
        <div class="stat">
          <span class="v" :class="{ warn: perf.p95Ms > perf.p50Ms * 1.6 }">{{
            perf.p95Ms.toFixed(2)
          }}</span
          ><span class="u">ms</span>
          <span class="k">p95 frame</span>
        </div>
        <div class="stat">
          <span class="v" :class="{ bad: perf.maxMs > perf.p50Ms * 3 }">{{
            perf.maxMs.toFixed(2)
          }}</span
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
        one bar per push · four a second · a bar is amber past 1.6× the median frame and red past
        3×, because a hitch is relative to what this machine has been managing
      </div>
    </div>
  </section>
</template>

<style scoped>
.perf {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.body {
  flex: 1;
  min-height: 0;
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
  flex: 1;
  min-height: 80px;
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
  width: 44px;
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
</style>
