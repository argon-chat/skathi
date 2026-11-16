<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "@skathi-native/runtime";

const STATE_EVENTS = ["maximize", "unmaximize", "minimize", "restore", "fullscreenchange", "move"];
const log = ref<string[]>([]);
const viewport = ref("");

const push = (e: Event) => (log.value = [e.type, ...log.value].slice(0, 8));
const onResize = (e: Event) => {
  viewport.value = `${Math.round(innerWidth)} x ${Math.round(innerHeight)} @ ${devicePixelRatio}x`;
  push(e);
};

onMounted(() => {
  STATE_EVENTS.forEach((type) => appWindow.addEventListener(type, push));
  addEventListener("resize", onResize);
  addEventListener("focus", push);
  addEventListener("blur", push);
});
onUnmounted(() => {
  STATE_EVENTS.forEach((type) => appWindow.removeEventListener(type, push));
  removeEventListener("resize", onResize);
  removeEventListener("focus", push);
  removeEventListener("blur", push);
});
</script>

<template>
  <div class="panel">
    <p>{{ appWindow.isFocused() ? "Active" : "In the background" }}, {{ viewport }}</p>
    <ol class="log">
      <li v-for="(type, i) in log" :key="i">{{ type }}</li>
    </ol>
  </div>
</template>

<style scoped>
.panel {
  padding: 12px;
  font-size: 13px;
}
.log {
  margin-top: 8px;
  color: #ffffffb0;
}
</style>
