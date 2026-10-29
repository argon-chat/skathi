<script setup lang="ts">
import { ref } from "vue";

const tint = ref("red");
const parts = import.meta.glob("./kinds/*.ts", { eager: true });
</script>

<template>
  <div class="p-4 app-root">{{ Object.keys(parts).length }}</div>
</template>

<style scoped>
@import "./styles/scoped-import.css";

.app-root {
  @apply mt-2;
  color: v-bind(tint);
}
</style>
