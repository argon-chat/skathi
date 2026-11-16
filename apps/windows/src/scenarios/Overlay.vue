<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import Icon from "../components/Icon.vue";
import PopWindow from "../components/PopWindow.vue";
import Scenario from "../components/Scenario.vue";

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

const ring = computed(() => `conic-gradient(#d4a5e6 ${(left.value / total) * 360}deg, #ffffff26 0deg)`);

const tasks = ref([
  { text: "Record the pop-out clips", done: false },
  { text: "Check the posters in light theme", done: false },
  { text: "Link every clip from its guide", done: false },
  { text: "Send the guides for review", done: false },
]);
</script>

<template>
  <Scenario id="pop-out/overlay">
    <div class="card">
      <div class="timer-row">
        <button v-if="left === 0" class="btn accent" data-demo="break" @click="start">
          <Icon name="overlay" />Start a 10-second break
        </button>
        <span v-else class="running" data-demo="break-running">Break ends in {{ left }} s. Keep working.</span>
      </div>
      <p class="muted">The ring is drawn by a separate, transparent window over the whole monitor, in its corner.</p>
    </div>

    <div class="card">
      <div class="card-title">Today</div>
      <div
        v-for="(t, i) in tasks"
        :key="t.text"
        class="task"
        :class="{ done: t.done }"
        :data-demo="`task-${i}`"
        @click="t.done = !t.done"
      >
        <span class="box"><Icon v-if="t.done" name="check" /></span>
        <span>{{ t.text }}</span>
      </div>
    </div>

    <PopWindow
      v-if="isWindows"
      no-cursor
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
      <div class="ring-wrap">
        <div class="ring" :style="{ background: ring }">
          <div class="hole">
            <span class="num">{{ left }}</span>
            <span class="unit">break</span>
          </div>
        </div>
      </div>
    </PopWindow>
  </Scenario>
</template>

<style scoped>
.timer-row {
  height: 36px;
  display: flex;
  align-items: center;
}
.running {
  font-size: 14px;
  color: var(--accent-text);
  font-variant-numeric: tabular-nums;
}
.task {
  height: 40px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 8px;
  border-radius: 5px;
  font-size: 14px;
  color: var(--text);
  cursor: pointer;
  transition:
    background-color 100ms ease,
    color 160ms ease;
}
.task:hover,
.task.demo-hover {
  background: var(--subtle-hover);
}
.task.done {
  color: var(--text-3);
}
.box {
  width: 20px;
  height: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 4px;
  border: 1px solid var(--text-2);
  color: var(--on-accent);
  transition:
    background-color 140ms ease,
    border-color 140ms ease;
}
.done .box {
  background: var(--accent);
  border-color: var(--accent);
}
.box .icon {
  width: 14px;
  height: 14px;
}
.overlay {
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
}
.ring-wrap {
  padding: 0 64px 96px 0;
}
.ring {
  width: 148px;
  height: 148px;
  border-radius: 74px;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 12px 32px #00000066;
}
.hole {
  width: 120px;
  height: 120px;
  border-radius: 60px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  background: #1f1a22f2;
  color: #ffffff;
  font-family: var(--font);
}
.num {
  font-family: var(--font-display);
  font-size: 40px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.unit {
  font-size: 12px;
  color: #ffffffa0;
}
</style>
