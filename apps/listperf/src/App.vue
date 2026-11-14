<script setup lang="ts">
import { computed, ref } from "vue";

// `SK_SCENARIO=<n>` picks the row count, so one process measures one size and the number in the
// report is never "whatever the app happened to default to".
const scenario = globalThis.__SK_SCENARIO__ ?? "";
const parsed = Number.parseInt(scenario, 10);
const COUNT = Number.isFinite(parsed) && parsed > 0 ? parsed : 1000;

type Row = { id: number; label: string; meta: string; tag: string };

const rows = ref<Row[]>(
  Array.from({ length: COUNT }, (_, i) => ({
    id: i,
    label: `row ${i + 1}`,
    meta: `id-${(i * 7919) % 100000}`,
    tag: i % 3 === 0 ? "alpha" : i % 3 === 1 ? "beta" : "gamma",
  })),
);

const scrollTop = ref(0);
const onScroll = (e: SkEvent) => (scrollTop.value = (e.target as SkElement).scrollTop);
const title = computed(() => `${rows.value.length} rows`);

const clicked = ref(-1);
</script>

<template>
  <div class="page">
    <div class="head">
      <div class="h1">listperf — {{ title }}</div>
      <div class="hint" id="pos">scrollTop {{ scrollTop.toFixed(0) }}</div>
      <div class="hint" id="clicked">clicked {{ clicked }}</div>
    </div>
    <div id="list" class="list" tabindex="0" @scroll="onScroll">
      <div v-for="row in rows" :key="row.id" class="row" :data-row="row.id" @click="clicked = row.id">
        <div class="idx">{{ row.id }}</div>
        <div class="label">{{ row.label }}</div>
        <div class="meta">{{ row.meta }}</div>
        <div class="tag" :class="row.tag">{{ row.tag }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  background: #12141a;
  height: 100%;
}
.head {
  display: flex;
  flex-direction: row;
  gap: 12px;
}
.h1 {
  font-size: 15px;
  color: #ffffff;
}
.hint {
  font-size: 12px;
  color: #ffffff88;
}
.list {
  overflow: scroll;
  flex-grow: 1;
  display: flex;
  flex-direction: column;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  padding: 4px;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  padding: 3px 6px;
  border-radius: 4px;
}
.row:hover {
  background: #ffffff14;
}
.idx {
  width: 48px;
  font-size: 12px;
  color: #ffffff66;
}
.label {
  width: 90px;
  font-size: 12px;
  color: #ffffffdd;
}
.meta {
  width: 110px;
  font-size: 12px;
  color: #ffffff88;
}
.tag {
  font-size: 11px;
  padding: 0 6px;
  border-radius: 3px;
  color: #10131a;
}
.tag.alpha {
  background: #7fd4ff;
}
.tag.beta {
  background: #a5f3a5;
}
.tag.gamma {
  background: #ffcc00;
}
</style>
