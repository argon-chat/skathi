<script setup lang="ts">
import { computed, ref } from "vue";
import { appWindow } from "@skathi-native/runtime";

// #region state
const count = ref(0);
const label = computed(() => (appWindow.isMaximized() ? "Restore" : "Maximize"));
// #endregion state
</script>

<template>
  <main class="page">
    <h1>{{ appWindow.title }}</h1>
    <button @click="count++">Clicked {{ count }} times</button>
    <button @click="appWindow.toggleMaximize()">{{ label }}</button>
  </main>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 32px;
  color: #f4f4f5;
}
</style>
