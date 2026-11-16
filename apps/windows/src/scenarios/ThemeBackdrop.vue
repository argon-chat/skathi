<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "@skathi-native/runtime";
import Scenario from "../components/Scenario.vue";

const THEMES = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];
const BACKDROPS = [
  { value: "mica", label: "Mica" },
  { value: "mica-alt", label: "Mica Alt" },
  { value: "acrylic", label: "Acrylic" },
  { value: "none", label: "None" },
];

function setTheme(theme: string) {
  appWindow.theme = theme;
  try {
    localStorage.setItem("windows.theme", theme);
  } catch {}
}

const dark = matchMedia("(prefers-color-scheme: dark)");
const scheme = ref(dark.matches ? "dark" : "light");
const onScheme = () => (scheme.value = dark.matches ? "dark" : "light");
onMounted(() => dark.addEventListener("change", onScheme));
onUnmounted(() => dark.removeEventListener("change", onScheme));
</script>

<template>
  <Scenario id="windows/theme-backdrop">
    <div class="card">
      <div class="card-title">Theme</div>
      <div class="segmented">
        <div
          v-for="t in THEMES"
          :key="t.value"
          class="seg"
          :class="{ on: appWindow.theme === t.value }"
          :data-demo="`theme-${t.value}`"
          @click="setTheme(t.value)"
        >
          {{ t.label }}
        </div>
      </div>
      <p class="muted">
        Drives prefers-color-scheme in every window of the app, and on Windows the caption and Mica tint.
      </p>
    </div>

    <div class="card">
      <div class="card-title">Backdrop</div>
      <div class="segmented">
        <div
          v-for="b in BACKDROPS"
          :key="b.value"
          class="seg"
          :class="{ on: appWindow.backdrop === b.value }"
          :data-demo="`backdrop-${b.value}`"
          @click="appWindow.backdrop = b.value"
        >
          {{ b.label }}
        </div>
      </div>
      <p class="muted">
        Mica samples the desktop wallpaper once and tints it; Acrylic blurs whatever is behind the window, live. With
        None the app paints its own solid background.
      </p>
    </div>

    <div class="card reads">
      <div class="read">
        <span class="k">appWindow.theme</span><span class="v">{{ appWindow.theme }}</span>
      </div>
      <div class="read">
        <span class="k">prefers-color-scheme</span><span class="v">{{ scheme }}</span>
      </div>
      <div class="read">
        <span class="k">appWindow.backdrop</span><span class="v">{{ appWindow.backdrop }}</span>
      </div>
      <div class="read">
        <span class="k">effectiveBackdrop</span><span class="v">{{ appWindow.effectiveBackdrop }}</span>
      </div>
    </div>
  </Scenario>
</template>

<style scoped>
.reads {
  gap: 10px;
}
.read {
  display: flex;
  align-items: center;
  gap: 16px;
}
.k {
  width: 200px;
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text-3);
}
.v {
  font-size: 14px;
  color: var(--text);
}
</style>
