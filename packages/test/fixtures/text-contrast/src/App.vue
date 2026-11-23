<script setup lang="ts">
import { ref } from "vue";

const smoothing = ref("antialiased");

(globalThis as { __SK_TEST_CONTRAST__?: unknown }).__SK_TEST_CONTRAST__ = {
  smooth(value: string) {
    smoothing.value = value;
  },
};
</script>

<template>
  <div :class="['root', smoothing]">
    <div v-for="tone in ['dark', 'light']" :key="tone" :class="['pane', tone]">
      <p v-for="law in ['auto', 'luminance']" :key="law" :class="['line', law]">
        Settings · Display · Notifications · 14 new
      </p>
    </div>
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  font-family: "Segoe UI", sans-serif;
  font-size: 14px;
}
.pane {
  padding: 12px 16px;
}
.dark {
  background: #1c1c1e;
  color: #ececec;
}
.light {
  background: #ffffff;
  color: #1c1c1e;
}
.line {
  margin: 0 0 8px;
}
.antialiased {
  font-smoothing: antialiased;
}
.subpixel {
  font-smoothing: subpixel-antialiased;
}
.luminance {
  text-contrast: luminance;
}
</style>
