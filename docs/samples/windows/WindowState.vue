<script setup lang="ts">
import { computed } from "vue";
import { appWindow } from "skathi:app";

// #region state
const state = computed(() => {
  if (appWindow.isFullscreen()) return "fullscreen";
  if (appWindow.isMinimized()) return "minimized";
  return appWindow.isMaximized() ? "maximized" : "normal";
});
// #endregion state
</script>

<template>
  <div class="toolbar">
    <!-- #region buttons -->
    <button class="btn" @click="appWindow.minimize()">Minimize</button>
    <button class="btn" @click="appWindow.maximize()">Maximize</button>
    <button class="btn" @click="appWindow.unmaximize()">Restore</button>
    <button class="btn" @click="appWindow.toggleMaximize()">Toggle</button>
    <!-- #endregion buttons -->
    <span class="state">State: {{ state }}</span>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px;
}
.btn {
  padding: 6px 12px;
  border-radius: 6px;
  background: #ffffff14;
  color: #f3f3f3;
}
.state {
  margin-left: auto;
  font-size: 12px;
  color: #ffffffb0;
}
</style>
