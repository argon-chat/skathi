<script setup lang="ts">
import { defineKeymap, SkKbd, type SkKbdLabels } from "@skathi-native/runtime";

const confirm = defineKeymap({ keys: { alternatives: ["Mod+Enter", { gamepad: "south" }] } });
const labels: SkKbdLabels = {
  keys: {
    Enter: { text: "↵", label: "Enter" },
    "Gamepad:south": { text: "✕", label: "Cross button" },
  },
};
</script>

<template>
  <div class="command-hints">
    <SkKbd :map="confirm" :labels="labels" />
  </div>
</template>

<style scoped>
.command-hints :deep(.sk-kbd) {
  font-family: "IBM Plex Mono", monospace;
  font-size: 14px;
  line-height: 20px;
}
.command-hints :deep(.sk-kbd-key) {
  padding: 2px 7px;
  border: 1px solid #55647a;
  border-radius: 6px;
  background-color: #202938;
  color: #f3f6fa;
}
.command-hints :deep(.sk-kbd-key[data-device="gamepad"] .sk-kbd-glyph) {
  display: none;
}
</style>
