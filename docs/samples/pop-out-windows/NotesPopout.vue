<script setup lang="ts">
import { ref } from "vue";
import { SkWindow, type SkWindowMetrics } from "@skathi-native/runtime";

const notesOpen = ref(false);
const note = ref("");
const size = ref("");

function onResize(m: SkWindowMetrics) {
  size.value = `${Math.round(m.width)} × ${Math.round(m.height)}`;
}
</script>

<template>
  <main class="page">
    <button @click="notesOpen = !notesOpen">{{ notesOpen ? "Close notes" : "Open notes" }}</button>
    <p>{{ note.length }} characters</p>
    <!-- #region window -->
    <SkWindow
      v-if="notesOpen"
      title="Notes"
      :bounds="{ width: 360, height: 280 }"
      activate
      @close="notesOpen = false"
      @resize="onResize"
    >
      <textarea v-model="note" class="notes" />
      <footer class="size">{{ size }}</footer>
    </SkWindow>
    <!-- #endregion window -->
  </main>
</template>

<style>
.page {
  padding: 24px;
}
.notes {
  width: 100%;
  height: 200px;
  padding: 12px;
}
@media (max-width: 300px) {
  .size {
    display: none;
  }
}
</style>
