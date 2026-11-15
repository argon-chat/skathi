<script setup lang="ts">
import { computed } from "vue";
import type { Row } from "./bench";

const props = defineProps<{ row: Row }>();
const tone = computed(() => (props.row.value > 64 ? "hot" : props.row.value > 32 ? "warm" : "cold"));
</script>

<template>
  <div class="row" :class="[tone, { done: row.done }]">
    <span class="id">{{ row.id }}</span>
    <span class="label">{{ row.label }}</span>
    <span class="value">{{ row.value }}</span>
  </div>
</template>

<style scoped>
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  height: 16px;
}
.hot {
  color: #ff8a65;
}
.done {
  opacity: 0.6;
}
</style>
