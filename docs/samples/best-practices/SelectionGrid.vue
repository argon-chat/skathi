<script setup lang="ts">
import { reactive } from "vue";
import SelectionBox from "./SelectionBox.vue";

const ROWS = 40;
const COLS = 12;
const cells = Array.from({ length: ROWS }, (_, r) => Array.from({ length: COLS }, (_, c) => `${r + 1}:${c + 1}`));

// #region state
const sel = reactive({ row: 0, col: 0 });

function onKey(e: KeyboardEvent) {
  if (e.key === "ArrowDown") sel.row = Math.min(ROWS - 1, sel.row + 1);
  else if (e.key === "ArrowUp") sel.row = Math.max(0, sel.row - 1);
  else if (e.key === "ArrowRight") sel.col = Math.min(COLS - 1, sel.col + 1);
  else if (e.key === "ArrowLeft") sel.col = Math.max(0, sel.col - 1);
}
// #endregion state
</script>

<template>
  <div class="grid" role="grid" aria-label="Cells" tabindex="0" @keydown="onKey">
    <div v-for="(row, r) in cells" :key="r" class="row" role="row">
      <div v-for="(text, c) in row" :key="c" class="cell" role="gridcell">{{ text }}</div>
    </div>
    <!-- #region overlay -->
    <SelectionBox :sel="sel" />
    <!-- #endregion overlay -->
  </div>
</template>

<style scoped>
.grid {
  position: relative;
}
.row {
  display: flex;
}
.cell {
  width: 80px;
  height: 24px;
  padding: 0 6px;
  line-height: 24px;
}
</style>
