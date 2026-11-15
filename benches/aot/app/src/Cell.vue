<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

const props = defineProps<{ n: number }>();
const live = ref(false);
const text = computed(() => (live.value ? `#${props.n}` : ""));
onMounted(() => {
  live.value = true;
});
onBeforeUnmount(() => {
  live.value = false;
});
</script>

<template>
  <span class="cell" :class="{ odd: n % 2 === 1 }">{{ text }}</span>
</template>

<style scoped>
.cell {
  width: 24px;
}
.odd {
  color: #8ab4f8;
}
</style>
