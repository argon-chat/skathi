<script setup lang="ts">
import { ref } from "vue";

const tick = ref(0);
const shown = ref(false);
const spinning = ref(false);
let ticking = false;

function step() {
  if (!ticking) return;
  tick.value++;
  requestAnimationFrame(step);
}

(globalThis as { __SK_TEST_SETTLE__?: unknown }).__SK_TEST_SETTLE__ = {
  show(on: boolean) {
    shown.value = on;
  },
  spin(on: boolean) {
    spinning.value = on;
  },
  start() {
    ticking = true;
    requestAnimationFrame(step);
  },
  stop() {
    ticking = false;
  },
};
</script>

<template>
  <div class="root">
    <div class="ticker">{{ tick }}</div>
    <div v-if="shown" class="stage">
      <div class="block">
        <div class="box scaled">
          <span class="label">Settled text</span>
          <span class="label">is rasterized</span>
          <span class="label">at its size</span>
        </div>
      </div>
      <div class="block">
        <div class="box big">
          <span class="label">Settled text</span>
          <span class="label">is rasterized</span>
          <span class="label">at its size</span>
        </div>
      </div>
      <div class="block">
        <div :class="['box', 'scaled', { spin: spinning }]">
          <span class="label">Settled text</span>
          <span class="label">is rasterized</span>
          <span class="label">at its size</span>
        </div>
      </div>
    </div>
    <div v-if="shown" id="list" class="list">
      <div v-for="i in 24" :key="i" class="row">
        <div class="box scaled list-scaled">
          <span class="label">Row {{ i }} settles</span>
          <span class="label">while it scrolls</span>
        </div>
        <div class="box big list-big">
          <span class="label">Row {{ i }} settles</span>
          <span class="label">while it scrolls</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  background: #fff;
  color: #111;
  font-family: "Segoe UI", sans-serif;
}
.ticker {
  height: 20px;
  font-size: 12px;
}
.stage {
  display: flex;
  gap: 24px;
  padding: 16px;
}
.block {
  width: 180px;
  height: 120px;
}
.list {
  height: 150px;
  margin: 0 16px;
  overflow-y: auto;
}
.row {
  display: flex;
  gap: 48px;
  height: 64px;
}
.row .box {
  width: 180px;
}
.box {
  display: flex;
  flex-direction: column;
  font-size: 16px;
  line-height: 22px;
}
.big {
  font-size: 18.24px;
  line-height: 25.08px;
}
.scaled {
  transform: scale(1.14);
  transform-origin: 0 0;
}
.spin {
  animation: pulse 1s linear infinite alternate;
}
@keyframes pulse {
  from {
    transform: scale(1.1);
  }
  to {
    transform: scale(1.18);
  }
}
</style>
