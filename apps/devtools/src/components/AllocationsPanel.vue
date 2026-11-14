<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { allocationDiff, allocationKey, byteLabel, rowWindow } from "../inspector-model";
import type { AllocationRow, AllocationSnapshot, InspectorRequest } from "../inspector-model";
const props = defineProps<{
  height: number;
  request?: InspectorRequest | undefined;
  sessionKey?: string | number | undefined;
}>();
const emit = defineEmits<{ (event: "recording-change", active: boolean): void }>();
const snapshot = ref<AllocationSnapshot | null>(null),
  baseline = ref<AllocationRow[] | null>(null),
  busy = ref(false),
  error = ref("");
const category = ref(""),
  site = ref(""),
  minimum = ref("0"),
  sampleEvery = ref("1"),
  scroll = ref(0),
  selected = ref<AllocationRow | null>(null);
const baselineMinimum = ref(0),
  appliedMinimum = ref(0),
  notice = ref("");
let epoch = 0;
const categories = computed(() => [...new Set(snapshot.value?.rows.map((row) => row.category) || [])].sort());
const rows = computed(() => {
  const raw = baseline.value
    ? allocationDiff(baseline.value, snapshot.value?.rows || [])
    : [...(snapshot.value?.rows || [])].sort((left, right) => right.liveBytes - left.liveBytes);
  return raw.filter(
    (row) =>
      (!category.value || row.category === category.value) &&
      (!site.value || row.site.toLowerCase().includes(site.value.toLowerCase())),
  );
});
const viewportHeight = computed(() => Math.max(80, props.height));
const windowed = computed(() => rowWindow(rows.value.length, scroll.value, viewportHeight.value));
const visible = computed(() => rows.value.slice(windowed.value.start, windowed.value.end));
const coverage = computed(() =>
  Object.entries(snapshot.value?.coverage || {}).map(
    ([name, value]) =>
      `${name}: ${name === "bufferBytes" && typeof value === "number" ? byteLabel(value) : value === true ? "captured" : value === false ? "not captured" : value}`,
  ),
);
async function load(operation = "snapshot") {
  if (!props.request || busy.value) return;
  const token = epoch,
    minSize = Math.max(0, Number(minimum.value) || 0);
  busy.value = true;
  error.value = "";
  try {
    const value = await props.request<AllocationSnapshot>({
      cmd: "allocations",
      op: operation,
      sampleEvery: Number(sampleEvery.value),
      maxEvents: 65536,
      minSize,
      limit: 2000,
    });
    if (token !== epoch) return;
    if (snapshot.value?.session !== value.session || operation === "start") {
      baseline.value = null;
      notice.value = "";
    }
    if (baseline.value && (baselineMinimum.value !== minSize || !value.complete)) {
      baseline.value = null;
      notice.value =
        "Baseline cleared because the snapshot filter or capture completeness changed. Compare complete snapshots with the same minimum size.";
    }
    snapshot.value = value;
    sampleEvery.value = String(value.sampleEvery || 1);
    appliedMinimum.value = minSize;
    emit("recording-change", value.recording);
    selected.value = null;
    scroll.value = 0;
  } catch (reason) {
    if (token === epoch) error.value = String(reason);
  } finally {
    if (token === epoch) busy.value = false;
  }
}
function setBaseline() {
  if (snapshot.value?.complete) {
    baseline.value = snapshot.value.rows.map((row) => ({ ...row }));
    baselineMinimum.value = appliedMinimum.value;
    notice.value = "";
  }
}
watch(
  [() => props.request, () => props.sessionKey],
  () => {
    epoch++;
    snapshot.value = null;
    baseline.value = null;
    notice.value = "";
    busy.value = false;
    selected.value = null;
    error.value = "";
    if (props.request) void load("status");
  },
  { immediate: true },
);
onBeforeUnmount(() => {
  epoch++;
  if (snapshot.value?.recording && props.request)
    void props
      .request<AllocationSnapshot>({ cmd: "allocations", op: "stop" })
      .then((value) => emit("recording-change", value.recording))
      .catch(() => {});
});
</script>

<template>
  <section class="allocations" aria-label="Allocation recording">
    <div class="allocation-toolbar">
      <button
        class="btn"
        :class="{ on: snapshot?.recording }"
        :disabled="!request || busy || snapshot?.available === false"
        @click="load(snapshot?.recording ? 'stop' : 'start')"
      >
        {{ snapshot?.recording ? "Stop tracking" : "Start tracking" }}</button
      ><button class="btn" :disabled="!request || busy" @click="load()">{{ busy ? "Reading…" : "Snapshot" }}</button
      ><button
        class="btn"
        :disabled="busy || !snapshot?.complete || !snapshot.rows.length"
        title="Compare complete snapshots with the same capture filter"
        @click="setBaseline"
      >
        Set baseline</button
      ><button v-if="baseline" class="btn" @click="baseline = null">Clear baseline</button
      ><span class="muted">{{
        snapshot?.recording ? "Tracking allocations" : baseline ? "Change since baseline" : "Allocation totals"
      }}</span>
    </div>
    <div v-if="error" class="feedback failure" role="alert">
      {{ error }}<button class="btn" :disabled="busy" @click="load('status')">Retry</button>
    </div>
    <div v-if="!request" class="feedback">Attach to a target to inspect allocation activity.</div>
    <div v-else-if="snapshot?.available === false" class="feedback">
      Allocation event tracking is unavailable in this build. The coverage below states which totals are available.
    </div>
    <div v-if="notice" class="feedback" role="status">{{ notice }}</div>
    <template v-if="snapshot">
      <div class="coverage">
        <span v-for="item in coverage" :key="item">{{ item }}</span>
      </div>
      <div class="allocation-totals mono">
        <span v-if="snapshot.heap">Heap live {{ byteLabel(snapshot.heap.liveBytes) }}</span
        ><span>Captured survivors {{ byteLabel(snapshot.totals.liveBytes) }}</span
        ><span>Allocated {{ byteLabel(snapshot.totals.allocatedBytes) }}</span
        ><span>Freed {{ byteLabel(snapshot.totals.freedBytes) }}</span
        ><span>{{ snapshot.totals.liveBlocks }} live blocks</span>
      </div>
      <form class="allocation-filters" @submit.prevent="load()">
        <label
          >Category<select v-model="category" class="input" :disabled="busy">
            <option value="">All categories</option>
            <option v-for="name in categories" :key="name" :value="name">{{ name }}</option>
          </select></label
        ><label class="site-filter"
          >Site<input
            v-model="site"
            class="input"
            placeholder="Filter captured sites"
            aria-label="Filter allocation sites" /></label
        ><label
          >Minimum bytes<input
            v-model="minimum"
            class="input bytes-filter"
            inputmode="numeric"
            aria-label="Minimum allocation bytes" /></label
        ><label
          >Sample every<select v-model="sampleEvery" class="input" :disabled="busy || snapshot.recording">
            <option value="1">1 allocation</option>
            <option value="16">16 allocations</option>
            <option value="64">64 allocations</option>
          </select></label
        ><button class="btn" :disabled="busy">Apply</button>
      </form>
      <div class="allocation-head">
        <span class="category">Category</span><span class="site">Site / size class</span
        ><span class="bytes">{{ baseline ? "Live Δ" : "Captured live" }}</span
        ><span class="bytes allocated">Allocated</span><span class="blocks">Blocks</span>
      </div>
      <div class="allocation-rows" @scroll="scroll = $event.target.scrollTop" :aria-busy="busy">
        <div :style="{ height: windowed.padTop + 'px' }"></div>
        <button
          v-for="row in visible"
          :key="allocationKey(row)"
          class="allocation-row"
          :class="{ selected: selected === row }"
          @click="selected = row"
        >
          <span class="category">{{ row.category }}</span
          ><span class="site mono" :title="row.site"
            >{{ row.site || "Unscoped" }} <span class="muted">{{ row.sizeClass }}</span></span
          ><span
            class="bytes mono"
            :class="{ growth: baseline && row.liveBytes > 0, released: baseline && row.liveBytes < 0 }"
            >{{ byteLabel(row.liveBytes, !!baseline) }}</span
          ><span class="bytes mono allocated">{{ byteLabel(row.allocatedBytes, !!baseline) }}</span
          ><span class="blocks mono">{{ row.liveBlocks }}</span>
        </button>
        <div :style="{ height: windowed.padBottom + 'px' }"></div>
        <div v-if="!rows.length && !busy" class="feedback">
          {{
            snapshot.recording
              ? "No matching allocation events yet. Exercise the target and take another snapshot."
              : "Start tracking before the interaction you want to measure, then take a snapshot."
          }}
        </div>
      </div>
      <div class="allocation-footer">
        <span
          >{{ rows.length }} rows · {{ (snapshot.elapsedMs / 1000).toFixed(1) }} s · 1 /
          {{ snapshot.sampleEvery || 1 }} sampling</span
        ><span v-if="snapshot.capturedEvents !== undefined"
          >{{ snapshot.capturedEvents }} / {{ snapshot.maxEvents }} events</span
        ><span v-if="snapshot.droppedEvents" class="warning">{{ snapshot.droppedEvents }} events dropped</span
        ><span v-if="!snapshot.complete" class="warning">Bounded / partial snapshot</span
        ><span
          >Totals describe captured events; sampled counts are not extrapolated. Sites are explicit labels, not captured
          stacks.</span
        >
      </div>
      <div v-if="selected" class="allocation-detail">
        <strong class="mono">{{ selected.site || "Unscoped allocation" }}</strong
        ><span>{{ selected.category }} · size class {{ selected.sizeClass }}</span
        ><span class="mono"
          >{{ selected.allocations }} allocations · {{ selected.frees }} frees ·
          {{ byteLabel(selected.freedBytes, !!baseline) }} freed</span
        ><button class="btn" @click="selected = null">Close detail</button>
      </div>
    </template>
    <div v-else-if="busy" class="feedback" role="status">Reading allocation tracking capabilities…</div>
  </section>
</template>

<style scoped>
.allocations {
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  background: var(--surface-panel);
  color: var(--text);
  font-size: var(--fs-s);
}
button {
  font-family: inherit;
  cursor: pointer;
}
button:disabled,
select:disabled {
  opacity: 0.45;
  cursor: default;
}
button:focus-visible,
select:focus-visible {
  outline: 2px solid var(--line-focus);
  outline-offset: -2px;
}
.muted {
  color: var(--text-muted);
}
.warning {
  color: var(--warn);
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
.allocation-toolbar {
  padding: var(--sp-3) var(--sp-4);
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-3);
  border-bottom: 1px solid var(--line);
  min-height: 28px;
}
.coverage {
  display: flex;
  gap: var(--sp-5);
  flex-wrap: wrap;
  padding: var(--sp-3) var(--sp-4);
  font-size: var(--fs-xs);
  color: var(--text-muted);
  background: var(--surface-panel);
}
.allocation-totals {
  display: flex;
  gap: var(--sp-6);
  flex-wrap: wrap;
  padding: var(--sp-4);
  border-bottom: 1px solid var(--line);
  color: var(--text);
  font-size: var(--fs-s);
}
.allocation-filters {
  display: flex;
  align-items: flex-end;
  gap: var(--sp-4);
  padding: var(--sp-4);
  flex-wrap: wrap;
}
.allocation-filters label {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.site-filter {
  flex: 1;
  min-width: 130px;
}
.bytes-filter {
  width: 78px;
}
.allocation-filters select {
  max-width: 160px;
}
.allocation-head,
.allocation-row {
  display: flex;
  align-items: center;
  gap: var(--sp-4);
  width: 100%;
  padding: 0 var(--sp-4);
  box-sizing: border-box;
  min-width: 0;
}
.allocation-head {
  height: 24px;
  background: var(--surface-raised);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.allocations > * {
  flex-shrink: 0;
}
.allocations > .allocation-rows {
  flex: 1 1 0px;
  min-height: 80px;
  overflow-y: auto;
}
.allocation-row {
  height: 24px;
  padding-top: 0;
  padding-bottom: 0;
  border: 0;
  border-bottom: 1px solid var(--line-subtle);
  background: var(--surface-panel);
  color: var(--text);
  font-size: var(--fs-s);
  text-align: left;
}
.allocation-row:hover {
  background: var(--surface-hover);
}
.allocation-row.selected {
  background: var(--surface-active);
}
.category {
  width: 90px;
  flex-shrink: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.site {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--text-strong);
}
.bytes {
  width: 88px;
  text-align: right;
  flex-shrink: 0;
}
.blocks {
  width: 58px;
  text-align: right;
  flex-shrink: 0;
  color: var(--text);
}
.growth {
  color: var(--warn);
}
.released {
  color: var(--ok);
}
.allocation-footer {
  display: flex;
  gap: var(--sp-5);
  flex-wrap: wrap;
  padding: var(--sp-3) var(--sp-4);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  border-top: 1px solid var(--line);
  line-height: 1.5;
}
.allocation-detail {
  display: flex;
  gap: var(--sp-4) var(--sp-5);
  flex-wrap: wrap;
  padding: var(--sp-4);
  background: var(--surface-base);
  border-top: 1px solid var(--line);
  color: var(--text);
}
.allocation-detail strong {
  color: var(--text-strong);
  overflow-wrap: anywhere;
}
.allocation-detail .btn {
  margin-left: auto;
}
@media (max-width: 650px) {
  .allocated {
    display: none;
  }
  .category {
    width: 72px;
  }
  .bytes {
    width: 76px;
  }
  .blocks {
    width: 45px;
  }
}
</style>
