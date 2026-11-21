<script setup lang="ts">
import { ref } from "vue";
import { appWindow } from "skathi:app";

const note = ref(localStorage.getItem("note") ?? "");
const save = () => localStorage.setItem("note", note.value);
const togglePin = () => (appWindow.alwaysOnTop = !appWindow.alwaysOnTop);
</script>

<template>
  <div class="notes">
    <div class="bar">
      <span class="title">Notes</span>
      <button class="pin" :class="{ on: appWindow.alwaysOnTop }" @click="togglePin">
        {{ appWindow.alwaysOnTop ? "Pinned" : "Pin" }}
      </button>
    </div>
    <textarea v-model="note" class="text" @input="save" />
  </div>
</template>

<style scoped>
.notes {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.bar {
  display: flex;
  align-items: center;
  padding: 8px 12px;
}
.title {
  flex: 1;
  font-size: 13px;
}
.pin {
  padding: 4px 10px;
  border-radius: 6px;
  background: #ffffff14;
}
.pin.on {
  background: #4cc2ff;
  color: #000000;
}
.text {
  flex: 1;
  margin: 0 12px 12px;
}
</style>
