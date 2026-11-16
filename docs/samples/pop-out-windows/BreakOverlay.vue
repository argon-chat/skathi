<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import { SkWindow } from "@skathi-native/runtime";

const isWindows = navigator.platform === "Win32";
const total = 10;
const left = ref(0);
let timer = 0;

function start() {
  left.value = total;
  clearInterval(timer);
  timer = setInterval(() => {
    left.value -= 1;
    if (left.value <= 0) clearInterval(timer);
  }, 1000);
}
onBeforeUnmount(() => clearInterval(timer));

const ring = computed(() => `conic-gradient(#0090ff ${(left.value / total) * 360}deg, #ffffff26 0deg)`);
</script>

<template>
  <main class="page">
    <button v-if="left === 0" @click="start">Start a 10-second break</button>
    <p v-else>Break ends in {{ left }} s</p>
    <!-- #region overlay -->
    <SkWindow
      v-if="isWindows"
      class="overlay"
      title="Break timer"
      cover="monitor"
      chrome="hidden"
      backdrop="transparent"
      always-on-top
      ignore-mouse-events
      :focusable="false"
      :visible="left > 0"
    >
      <!-- #endregion overlay -->
      <div class="ring" :style="{ background: ring }">
        <div class="hole">{{ left }}</div>
      </div>
    </SkWindow>
  </main>
</template>

<style>
.page {
  padding: 24px;
}
.overlay {
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
  padding: 48px;
}
.ring {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 120px;
  height: 120px;
  border-radius: 60px;
}
.hole {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 96px;
  height: 96px;
  border-radius: 48px;
  background: #1b1b1bf0;
  color: #ffffff;
  font-size: 32px;
}
</style>
