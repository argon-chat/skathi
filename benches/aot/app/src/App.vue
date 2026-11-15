<script setup lang="ts">
import { onMounted } from "vue";
import RowView from "./RowView.vue";
import Cell from "./Cell.vue";
import { bench, churn, rows, run } from "./bench";

const cells = Array.from({ length: 500 }, (_, i) => i);

onMounted(() => {
  bench.mountedAt = performance.now();
  void run();
});
</script>

<template>
  <div class="page">
    <div class="head ready">aotbench {{ rows.length }}</div>
    <div class="list">
      <RowView v-for="row in rows" :key="row.id" :row="row" />
    </div>
    <div v-if="churn" class="cells">
      <Cell v-for="i in cells" :key="i" :n="i" />
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #12141a;
  color: #d8dbe2;
  font-size: 12px;
}
.list {
  flex: 1;
  overflow: hidden;
}
.cells {
  display: flex;
  flex-wrap: wrap;
  height: 40px;
  overflow: hidden;
}
</style>
