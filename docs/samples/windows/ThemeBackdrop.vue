<script setup lang="ts">
import { appWindow } from "@skathi-native/runtime";

const THEMES = ["system", "light", "dark"];
const BACKDROPS = ["mica", "acrylic", "none"];

// #region persist
appWindow.theme = localStorage.getItem("theme") ?? "system";

function setTheme(theme: string) {
  appWindow.theme = theme;
  localStorage.setItem("theme", theme);
}
// #endregion persist
</script>

<template>
  <div class="panel">
    <div class="row">
      <button v-for="t in THEMES" :key="t" class="chip" :class="{ on: appWindow.theme === t }" @click="setTheme(t)">
        {{ t }}
      </button>
    </div>
    <div class="row">
      <button
        v-for="b in BACKDROPS"
        :key="b"
        class="chip"
        :class="{ on: appWindow.backdrop === b }"
        @click="appWindow.backdrop = b"
      >
        {{ b }}
      </button>
    </div>
    <p>Drawn: {{ appWindow.effectiveBackdrop }}</p>
  </div>
</template>

<style scoped>
.panel {
  --fg: #1b1b1b;
  --chip: #0000000f;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  color: var(--fg);
}
@media (prefers-color-scheme: dark) {
  .panel {
    --fg: #f3f3f3;
    --chip: #ffffff14;
  }
}
.row {
  display: flex;
  gap: 6px;
}
.chip {
  padding: 6px 12px;
  border-radius: 6px;
  background: var(--chip);
  color: var(--fg);
}
.chip.on {
  background: #0067c0;
  color: #ffffff;
}
</style>
