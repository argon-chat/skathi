<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "skathi:app";
import Icon from "../components/Icon.vue";
import Scenario from "../components/Scenario.vue";

const state = computed(() => {
  if (appWindow.isFullscreen()) return "fullscreen";
  if (appWindow.isMinimized()) return "minimized";
  return appWindow.isMaximized() ? "maximized" : "normal";
});

const EVENTS = ["maximize", "unmaximize", "minimize", "restore"] as const;
const log = ref<{ n: number; name: string; at: string }[]>([]);
let seq = 0;
const listeners = EVENTS.map((name) => {
  const fn = () => {
    const at = new Date().toTimeString().slice(0, 8);
    log.value = [{ n: ++seq, name, at }, ...log.value].slice(0, 5);
  };
  return [name, fn] as const;
});
onMounted(() => listeners.forEach(([name, fn]) => appWindow.addEventListener(name, fn)));
onUnmounted(() => listeners.forEach(([name, fn]) => appWindow.removeEventListener(name, fn)));
</script>

<template>
  <Scenario id="windows/state-verbs">
    <div class="toolbar card">
      <div class="row">
        <button class="btn" data-demo="minimize" @click="appWindow.minimize()"><Icon name="minimize" />Minimize</button>
        <button class="btn" data-demo="maximize" @click="appWindow.maximize()"><Icon name="maximize" />Maximize</button>
        <button class="btn" data-demo="restore" @click="appWindow.unmaximize()"><Icon name="restore" />Restore</button>
        <button class="btn" data-demo="toggle" @click="appWindow.toggleMaximize()"><Icon name="toggle" />Toggle</button>
      </div>
    </div>

    <div class="grid">
      <div class="card state">
        <span class="label">State</span>
        <span class="value" data-demo="state">{{ state }}</span>
        <div class="reads">
          <span class="read" :class="{ on: appWindow.isMaximized() }">isMaximized() {{ appWindow.isMaximized() }}</span>
          <span class="read" :class="{ on: appWindow.isMinimized() }">isMinimized() {{ appWindow.isMinimized() }}</span>
          <span class="read" :class="{ on: appWindow.isFullscreen() }">{{
            `isFullscreen() ${appWindow.isFullscreen()}`
          }}</span>
        </div>
      </div>
      <div class="card events">
        <span class="label">Events on appWindow</span>
        <div v-if="!log.length" class="empty">Nothing yet. Use the toolbar, the caption buttons or Win+Up.</div>
        <div v-for="e in log" :key="e.n" class="event">
          <span class="name">{{ e.name }}</span>
          <span class="at">{{ e.at }}</span>
        </div>
      </div>
    </div>
  </Scenario>
</template>

<style scoped>
.toolbar {
  padding: 14px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 16px;
}
.label {
  font-size: 12px;
  color: var(--text-3);
}
.value {
  font-family: var(--font-display);
  font-size: 40px;
  font-weight: 600;
  color: var(--text);
}
.reads {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.read {
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text-3);
  transition: color 160ms ease;
}
.read.on {
  color: var(--accent-text);
}
.empty {
  font-size: 13px;
  line-height: 20px;
  color: var(--text-3);
}
.event {
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 12px;
  border-radius: 5px;
  background: var(--well);
}
.name {
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text);
}
.at {
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
</style>
