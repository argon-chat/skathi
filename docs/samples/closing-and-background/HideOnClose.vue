<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { appLaunch, appTray, appWindow } from "skathi:app";

const seconds = ref(0);
const tick = setInterval(() => seconds.value++, 1000);

function onCloseRequested(e: Event) {
  e.preventDefault();
  appWindow.hide();
}
const bringBack = () => appWindow.show();

onMounted(() => {
  appWindow.addEventListener("close-requested", onCloseRequested);
  appLaunch.addEventListener("launch", bringBack);
  if (appTray.available) {
    appTray.set({
      icon: { width: 16, height: 16, data: new Uint8Array(16 * 16 * 4).fill(255), template: true },
      tooltip: "Focus timer",
      onClick: bringBack,
      menu: [
        { label: "Open", default: true, onSelect: bringBack },
        { type: "separator" },
        { label: "Quit", onSelect: () => appWindow.close() },
      ],
    });
  }
});
onUnmounted(() => {
  clearInterval(tick);
  appWindow.removeEventListener("close-requested", onCloseRequested);
  appLaunch.removeEventListener("launch", bringBack);
  if (appTray.available) appTray.set(null);
});
</script>

<template>
  <div class="timer">
    <p class="time">{{ seconds }} s</p>
    <p class="hint">Close the window: the timer keeps running. Start the app again to bring it back.</p>
    <button class="btn" @click="appWindow.close()">Quit</button>
  </div>
</template>

<style scoped>
.timer {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  height: 100%;
}
.time {
  font-size: 40px;
  font-variant-numeric: tabular-nums;
}
.hint {
  font-size: 12px;
  color: #ffffffa0;
}
.btn {
  padding: 6px 14px;
  border-radius: 6px;
  background: #ffffff14;
  color: #f3f3f3;
}
</style>
