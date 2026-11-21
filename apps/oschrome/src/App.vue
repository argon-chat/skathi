<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "skathi:app";

// engine.d.ts types `navigator` as optional; under sk-app it is installed before this module
// evaluates.
const wco = navigator.windowControlsOverlay;

const state = ref<Record<string, unknown>>({});
const read = () => {
  const r = wco.getTitlebarAreaRect();
  state.value = {
    titleBarStyle: appWindow.chrome.titleBarStyle,
    displayMode: appWindow.chrome.displayMode,
    wcoVisible: wco.visible,
    wcoRect: `${r.width} x ${r.height}`,
    standalone: matchMedia("(display-mode: standalone)").matches,
    backdrop: appWindow.backdrop,
    inner: `${innerWidth} x ${innerHeight}`,
    outer: `${outerWidth} x ${outerHeight}`,
  };
};

onMounted(() => {
  read();
  addEventListener("resize", read);
});
onUnmounted(() => removeEventListener("resize", read));
</script>

<template>
  <main class="page">
    <h1 class="h">OS chrome</h1>
    <p class="lede">
      The titlebar above is Windows', not this app's. `titleBarStyle: "default"` in sk.config.json, which is
      creation-time only — a window style cannot change mid-session without desyncing gpui's cached frame metrics
      (D-25).
    </p>

    <div class="fakebar">window-drag: move — inert here (advisory A-25)</div>

    <div class="grid">
      <template v-for="(v, k) in state" :key="k">
        <span class="k">{{ k }}</span>
        <span class="v">{{ String(v) }}</span>
      </template>
    </div>

    <div class="row">
      <button
        class="btn"
        @click="
          appWindow.backdrop = 'mica';
          read();
        "
      >
        mica
      </button>
      <button
        class="btn"
        @click="
          appWindow.backdrop = 'acrylic';
          read();
        "
      >
        acrylic
      </button>
      <button
        class="btn"
        @click="
          appWindow.backdrop = 'none';
          read();
        "
      >
        none
      </button>
      <button class="btn" @click="appWindow.maximize()">maximize</button>
      <button class="btn" @click="appWindow.unmaximize()">restore</button>
    </div>
  </main>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 22px;
  color: #ffffffe6;
  font-size: 13px;
}
.h {
  font-size: 20px;
  color: #ffffff;
}
.lede {
  color: #ffffffa0;
  line-height: 20px;
}
.fakebar {
  /* Would drag the window under `titleBarStyle: "hidden"`. */
  window-drag: move;
  padding-top: 8px;
  padding-bottom: 8px;
  padding-left: 12px;
  padding-right: 12px;
  border-radius: 8px;
  background: #ffffff0d;
  color: #ffffff80;
  font-size: 12px;
}
.grid {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.k {
  color: #7aa2ff;
  font-size: 12px;
}
.v {
  color: #ffffffcc;
  font-size: 12px;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.btn {
  padding-top: 6px;
  padding-bottom: 6px;
  padding-left: 12px;
  padding-right: 12px;
  border-radius: 8px;
  background: #ffffff14;
  color: #ffffffe6;
  font-size: 12px;
}
.btn:hover {
  background: #ffffff26;
}
</style>
