<script setup lang="ts">
import { ref } from "vue";
import { SkWindow } from "@skathi-native/runtime";

const count = ref(0);
</script>

<template>
  <main class="page">
    <p>The counter opens in its own window. Close that window and the counter returns here.</p>
    <SkWindow title="Counter" :bounds="{ width: 240, height: 160 }">
      <div class="panel">
        <button @click="count++">Clicked {{ count }} times</button>
      </div>
    </SkWindow>
  </main>
</template>

<style>
.page {
  padding: 24px;
}
.panel {
  padding: 16px;
}
</style>
