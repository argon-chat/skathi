<script setup lang="ts">
import { appWindow } from "@skathi-native/runtime";

function toggleFullscreen() {
  if (document.fullscreenElement) void document.exitFullscreen();
  else void document.documentElement.requestFullscreen();
}
</script>

<template>
  <div class="player">
    <button class="btn" @click="toggleFullscreen">
      {{ appWindow.isFullscreen() ? "Leave fullscreen" : "Fullscreen" }}
    </button>
    <p class="hint">Press Escape to leave.</p>
  </div>
</template>

<style scoped>
.player {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  height: 100%;
}
.btn {
  padding: 8px 16px;
  border-radius: 6px;
  background: #ffffff14;
  color: #f3f3f3;
}
.hint {
  font-size: 12px;
  color: #ffffff90;
}
</style>
