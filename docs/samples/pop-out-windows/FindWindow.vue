<script setup lang="ts">
import { ref } from "vue";
import { SkWindow } from "@skathi-native/runtime";
import { appWindow } from "skathi:app";

const findOpen = ref(false);
const findFocused = ref(false);
const query = ref("");
</script>

<template>
  <main class="page">
    <button @click="findOpen = true">Find</button>
    <p>Main window active: {{ appWindow.isFocused() }}. Find window active: {{ findFocused }}.</p>
    <SkWindow
      v-if="findOpen"
      title="Find"
      :bounds="{ width: 320, height: 120 }"
      :taskbar="false"
      activate
      @focus="findFocused = true"
      @blur="findFocused = false"
      @close="findOpen = false"
    >
      <input v-model="query" class="query" placeholder="Search the document" />
    </SkWindow>
  </main>
</template>

<style>
.page {
  padding: 24px;
}
.query {
  width: 100%;
  padding: 8px;
}
</style>
