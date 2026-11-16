<script setup lang="ts">
import { computed, ref } from "vue";

const ROW = 28;
const HEIGHT = 420;
const OVERSCAN = 4;
const rows = Array.from({ length: 10_000 }, (_, i) => ({ id: i, text: `Row ${i + 1}` }));

// #region slice
const first = ref(0);
const visible = computed(() => rows.slice(first.value, first.value + Math.ceil(HEIGHT / ROW) + 2 * OVERSCAN));

function onScroll(e: Event) {
  const top = (e.currentTarget as HTMLElement).scrollTop;
  first.value = Math.max(0, Math.floor(top / ROW) - OVERSCAN);
}
// #endregion slice
</script>

<template>
  <!-- #region template -->
  <div class="viewport" @scroll="onScroll">
    <div class="spacer" :style="{ height: `${rows.length * ROW}px` }">
      <div v-for="row in visible" :key="row.id" class="row" :style="{ top: `${row.id * ROW}px` }">
        {{ row.text }}
      </div>
    </div>
  </div>
  <!-- #endregion template -->
</template>

<style scoped>
.viewport {
  height: 420px;
  overflow-y: auto;
}
.spacer {
  position: relative;
}
.row {
  position: absolute;
  left: 0;
  right: 0;
  height: 28px;
  padding: 0 12px;
  line-height: 28px;
}
</style>
