<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { allocationDiff, allocationKey, byteLabel, rowWindow } from "../inspector-model";
import type { AllocationRow, AllocationSnapshot, InspectorRequest } from "../inspector-model";
const props = defineProps<{ height: number; request?: InspectorRequest; sessionKey?: string | number }>();
const emit = defineEmits<{ (event: "recording-change", active: boolean): void }>();
const snapshot = ref<AllocationSnapshot | null>(null), baseline = ref<AllocationRow[] | null>(null), busy = ref(false), error = ref("");
const category = ref(""), site = ref(""), minimum = ref("0"), sampleEvery = ref("1"), scroll = ref(0), selected = ref<AllocationRow | null>(null);
const baselineMinimum = ref(0), appliedMinimum = ref(0), notice = ref("");
let epoch = 0;
const categories = computed(() => [...new Set(snapshot.value?.rows.map(row => row.category) || [])].sort());
const rows = computed(() => {
  const raw = baseline.value ? allocationDiff(baseline.value, snapshot.value?.rows || []) : [...(snapshot.value?.rows || [])].sort((left, right) => right.liveBytes - left.liveBytes);
  return raw.filter(row => (!category.value || row.category === category.value) && (!site.value || row.site.toLowerCase().includes(site.value.toLowerCase())));
});
const viewportHeight = computed(() => Math.max(80, props.height - (selected.value ? 257 : 212)));
const windowed = computed(() => rowWindow(rows.value.length, scroll.value, viewportHeight.value));
const visible = computed(() => rows.value.slice(windowed.value.start, windowed.value.end));
const coverage = computed(() => Object.entries(snapshot.value?.coverage || {}).map(([name, value]) => `${name}: ${name === "bufferBytes" && typeof value === "number" ? byteLabel(value) : value === true ? "captured" : value === false ? "not captured" : value}`));
async function load(operation = "snapshot") {
  if (!props.request || busy.value) return;
  const token = epoch, minSize = Math.max(0, Number(minimum.value) || 0); busy.value = true; error.value = "";
  try {
    const value = await props.request<AllocationSnapshot>({ cmd: "allocations", op: operation, sampleEvery: Number(sampleEvery.value), maxEvents: 65536, minSize, limit: 2000 });
    if (token !== epoch) return;
    if (snapshot.value?.session !== value.session || operation === "start") { baseline.value = null; notice.value = ""; }
    if (baseline.value && (baselineMinimum.value !== minSize || !value.complete)) {
      baseline.value = null;
      notice.value = "Baseline cleared because the snapshot filter or capture completeness changed. Compare complete snapshots with the same minimum size.";
    }
    snapshot.value = value; sampleEvery.value = String(value.sampleEvery || 1); appliedMinimum.value = minSize; emit("recording-change", value.recording); selected.value = null; scroll.value = 0;
  } catch (reason) { if (token === epoch) error.value = String(reason); }
  finally { if (token === epoch) busy.value = false; }
}
function setBaseline() {
  if (snapshot.value?.complete) {
    baseline.value = snapshot.value.rows.map(row => ({ ...row }));
    baselineMinimum.value = appliedMinimum.value; notice.value = "";
  }
}
watch([() => props.request, () => props.sessionKey], () => { epoch++; snapshot.value = null; baseline.value = null; notice.value = ""; busy.value = false; selected.value = null; error.value = ""; if (props.request) void load("status"); }, { immediate: true });
onBeforeUnmount(() => { epoch++; if (snapshot.value?.recording && props.request) void props.request<AllocationSnapshot>({ cmd: "allocations", op: "stop" }).then(value => emit("recording-change", value.recording)).catch(() => {}); });
</script>

<template>
  <section class="allocations" :style="{ height: height + 'px' }" aria-label="Allocation recording">
    <div class="allocation-toolbar"><button class="btn" :class="{ on: snapshot?.recording }" :disabled="!request || busy || snapshot?.available === false" @click="load(snapshot?.recording ? 'stop' : 'start')">{{ snapshot?.recording ? "Stop tracking" : "Start tracking" }}</button><button class="btn" :disabled="!request || busy" @click="load()">{{ busy ? "Reading…" : "Snapshot" }}</button><button class="btn" :disabled="busy || !snapshot?.complete || !snapshot.rows.length" title="Compare complete snapshots with the same capture filter" @click="setBaseline">Set baseline</button><button v-if="baseline" class="btn" @click="baseline = null">Clear baseline</button><span class="muted">{{ snapshot?.recording ? "Tracking allocations" : baseline ? "Change since baseline" : "Allocation totals" }}</span></div>
    <div v-if="error" class="feedback failure" role="alert">{{ error }}<button class="btn" :disabled="busy" @click="load('status')">Retry</button></div>
    <div v-if="!request" class="feedback">Attach to a target to inspect allocation activity.</div>
    <div v-else-if="snapshot?.available === false" class="feedback">Allocation event tracking is unavailable in this build. The coverage below states which totals are available.</div>
    <div v-if="notice" class="feedback" role="status">{{ notice }}</div>
    <template v-if="snapshot">
      <div class="coverage"><span v-for="item in coverage" :key="item">{{ item }}</span></div>
      <div class="allocation-totals mono"><span v-if="snapshot.heap">Heap live {{ byteLabel(snapshot.heap.liveBytes) }}</span><span>Captured survivors {{ byteLabel(snapshot.totals.liveBytes) }}</span><span>Allocated {{ byteLabel(snapshot.totals.allocatedBytes) }}</span><span>Freed {{ byteLabel(snapshot.totals.freedBytes) }}</span><span>{{ snapshot.totals.liveBlocks }} live blocks</span></div>
      <form class="allocation-filters" @submit.prevent="load()"><label>Category<select v-model="category" class="input" :disabled="busy"><option value="">All categories</option><option v-for="name in categories" :key="name" :value="name">{{ name }}</option></select></label><label class="site-filter">Site<input v-model="site" class="input" placeholder="Filter captured sites" aria-label="Filter allocation sites"/></label><label>Minimum bytes<input v-model="minimum" class="input bytes-filter" inputmode="numeric" aria-label="Minimum allocation bytes"/></label><label>Sample every<select v-model="sampleEvery" class="input" :disabled="busy || snapshot.recording"><option value="1">1 allocation</option><option value="16">16 allocations</option><option value="64">64 allocations</option></select></label><button class="btn" :disabled="busy">Apply</button></form>
      <div class="allocation-head"><span class="category">Category</span><span class="site">Site / size class</span><span class="bytes">{{ baseline ? "Live Δ" : "Captured live" }}</span><span class="bytes allocated">Allocated</span><span class="blocks">Blocks</span></div>
      <div class="allocation-rows" :style="{ height: viewportHeight + 'px' }" @scroll="scroll = $event.target.scrollTop" :aria-busy="busy"><div :style="{ height: windowed.padTop + 'px' }"></div><button v-for="row in visible" :key="allocationKey(row)" class="allocation-row" :class="{ selected: selected === row }" @click="selected = row"><span class="category">{{ row.category }}</span><span class="site mono" :title="row.site">{{ row.site || 'Unscoped' }} <span class="muted">{{ row.sizeClass }}</span></span><span class="bytes mono" :class="{ growth: baseline && row.liveBytes > 0, released: baseline && row.liveBytes < 0 }">{{ byteLabel(row.liveBytes, !!baseline) }}</span><span class="bytes mono allocated">{{ byteLabel(row.allocatedBytes, !!baseline) }}</span><span class="blocks mono">{{ row.liveBlocks }}</span></button><div :style="{ height: windowed.padBottom + 'px' }"></div><div v-if="!rows.length && !busy" class="feedback">{{ snapshot.recording ? "No matching allocation events yet. Exercise the target and take another snapshot." : "Start tracking before the interaction you want to measure, then take a snapshot." }}</div></div>
      <div class="allocation-footer"><span>{{ rows.length }} rows · {{ (snapshot.elapsedMs / 1000).toFixed(1) }} s · 1 / {{ snapshot.sampleEvery || 1 }} sampling</span><span v-if="snapshot.capturedEvents !== undefined">{{ snapshot.capturedEvents }} / {{ snapshot.maxEvents }} events</span><span v-if="snapshot.droppedEvents" class="warning">{{ snapshot.droppedEvents }} events dropped</span><span v-if="!snapshot.complete" class="warning">Bounded / partial snapshot</span><span>Totals describe captured events; sampled counts are not extrapolated. Sites are explicit labels, not captured stacks.</span></div>
      <div v-if="selected" class="allocation-detail"><strong class="mono">{{ selected.site || "Unscoped allocation" }}</strong><span>{{ selected.category }} · size class {{ selected.sizeClass }}</span><span class="mono">{{ selected.allocations }} allocations · {{ selected.frees }} frees · {{ byteLabel(selected.freedBytes, !!baseline) }} freed</span><button class="btn" @click="selected = null">Close detail</button></div>
    </template>
    <div v-else-if="busy" class="feedback" role="status">Reading allocation tracking capabilities…</div>
  </section>
</template>

<style scoped>
.allocations { min-width: 0; overflow-y: auto; background: #10151d; color: #d7e0ea; font-size: 11px; }button { font-family: inherit; cursor: pointer; }button:disabled, select:disabled { opacity: 0.45; cursor: default; }button:focus-visible, select:focus-visible { outline: 2px solid #6cb6ff; outline-offset: -2px; }.muted { color: #a9b6c7; }.warning { color: #e3b341; }.feedback { padding: 16px 12px; color: #a9b6c7; line-height: 1.5; }.failure { color: #ff9d96; display: flex; align-items: center; gap: 8px; }
.allocation-toolbar { padding: 6px 8px; display: flex; align-items: center; flex-wrap: wrap; gap: 6px; border-bottom: 1px solid #232c3a; min-height: 28px; }.coverage { display: flex; gap: 12px; flex-wrap: wrap; padding: 7px 8px; font-size: 10px; color: #a9b6c7; background: #121b26; }.allocation-totals { display: flex; gap: 16px; flex-wrap: wrap; padding: 8px; border-bottom: 1px solid #232c3a; color: #c9d8e8; font-size: 11px; }.allocation-filters { display: flex; align-items: flex-end; gap: 8px; padding: 8px; flex-wrap: wrap; }.allocation-filters label { display: flex; flex-direction: column; gap: 4px; color: #a9b6c7; font-size: 10px; }.site-filter { flex: 1; min-width: 130px; }.bytes-filter { width: 78px; }.allocation-filters select { max-width: 160px; }
.allocation-head, .allocation-row { display: flex; align-items: center; gap: 10px; width: 100%; padding: 0 10px; box-sizing: border-box; min-width: 0; }.allocation-head { height: 24px; background: #171e29; color: #a9b6c7; font-size: 10px; }.allocation-rows { overflow-y: auto; }.allocation-row { height: 24px; padding-top: 0; padding-bottom: 0; border: 0; border-bottom: 1px solid #1b2430; background: #10151d; color: #d7e0ea; font-size: 11px; text-align: left; }.allocation-row:hover { background: #1b2735; }.allocation-row.selected { background: #163854; }.category { width: 90px; flex-shrink: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }.site { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; color: #bdd8f2; }.bytes { width: 88px; text-align: right; flex-shrink: 0; }.blocks { width: 58px; text-align: right; flex-shrink: 0; color: #b9c6d6; }.growth { color: #f1b083; }.released { color: #7bd28b; }.allocation-footer { display: flex; gap: 12px; flex-wrap: wrap; padding: 6px 8px; color: #a9b6c7; font-size: 10px; border-top: 1px solid #232c3a; line-height: 1.5; }.allocation-detail { display: flex; gap: 8px 14px; flex-wrap: wrap; padding: 10px; background: #0c121b; border-top: 1px solid #232c3a; color: #b9c6d6; }.allocation-detail strong { color: #e6edf3; overflow-wrap: anywhere; }.allocation-detail .btn { margin-left: auto; }
@media (max-width: 650px) { .allocated { display: none; }.category { width: 72px; }.bytes { width: 76px; }.blocks { width: 45px; } }
</style>
