<script setup lang="ts">
import { computed } from "vue";
import { ROW_HEIGHT, ROWS, VISIBLE, offset } from "./list";

const first = computed(() => Math.floor(offset.value / ROW_HEIGHT));
const visible = computed(() => {
  const out = [];
  for (let id = first.value; id < Math.min(first.value + VISIBLE, ROWS); id++) {
    out.push({
      id,
      label: `item ${id + 1}`,
      hash: ((id * 2654435761) >>> 0).toString(16).padStart(8, "0"),
      value: (id * 1.37).toFixed(2),
      tag: id % 3 === 0 ? "alpha" : id % 3 === 1 ? "beta" : "gamma",
    });
  }
  return out;
});
const shift = computed(() => `translateY(${-(offset.value % ROW_HEIGHT)}px)`);
</script>

<template>
  <div class="page">
    <div class="head">nojitbench · {{ ROWS }} rows · first {{ first }}</div>
    <div class="viewport">
      <div class="rows" :style="{ transform: shift }">
        <div v-for="row in visible" :key="row.id" class="row" :class="{ odd: row.id % 2 === 1 }">
          <div class="idx">{{ row.id }}</div>
          <div class="label">{{ row.label }}</div>
          <div class="hash">{{ row.hash }}</div>
          <div class="value">{{ row.value }}</div>
          <div class="tag" :class="row.tag">{{ row.tag }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 8px;
  gap: 6px;
  background: #12141a;
  color: #ffffffdd;
  font-size: 12px;
}
.viewport {
  flex-grow: 1;
  overflow: hidden;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  height: 24px;
  align-items: center;
  padding: 0 6px;
}
.row.odd {
  background: #ffffff0a;
}
.idx {
  width: 56px;
  color: #ffffff66;
}
.label {
  width: 100px;
}
.hash {
  width: 80px;
  color: #ffffff88;
}
.value {
  width: 70px;
  text-align: right;
}
.tag {
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
