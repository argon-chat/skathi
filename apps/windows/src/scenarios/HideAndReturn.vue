<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import Scenario from "../components/Scenario.vue";
import { background } from "../store";

const seconds = ref(0);
const tick = setInterval(() => seconds.value++, 1000);
onUnmounted(() => clearInterval(tick));

const clock = computed(() => {
  const m = Math.floor(seconds.value / 60);
  return `${m}:${String(seconds.value % 60).padStart(2, "0")}`;
});
</script>

<template>
  <Scenario id="closing/hide-and-return">
    <div class="card">
      <div
        class="switch"
        :class="{ on: background.keepRunning }"
        data-demo="keep-running"
        @click="background.keepRunning = !background.keepRunning"
      >
        <span class="track"><span class="thumb" /></span>
        <span>Keep running when the window is closed</span>
      </div>
      <p class="muted">
        On: close-requested is cancelled and the window hides, taskbar button included. A tray icon appears while the
        switch is on. Off: the close button quits as usual.
      </p>
    </div>

    <div class="grid">
      <div class="card stat">
        <span class="label">Running for</span>
        <span class="value">{{ clock }}</span>
        <span class="muted">The timer keeps counting while the window is hidden.</span>
      </div>
      <div class="card stat">
        <span class="label">Brought back</span>
        <span class="value">{{ background.returns }}</span>
        <span class="muted">{{ background.lastReturn ? `Last time by ${background.lastReturn}.` : "Not yet." }}</span>
      </div>
    </div>
  </Scenario>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: 16px;
}
.stat {
  gap: 6px;
}
.label {
  font-size: 12px;
  color: var(--text-3);
}
.value {
  font-family: var(--font-display);
  font-size: 40px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--text);
}
</style>
