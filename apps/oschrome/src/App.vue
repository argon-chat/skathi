<script setup>
// docs/WINDOWING.md §4.2 mode 3 — OS-drawn chrome, `titleBarStyle: "default"`.
//
// The window above this content has a real `WS_CAPTION` (fork item F-03), so
// the titlebar, its three buttons, double-click-to-zoom, the system menu and
// the snap-layouts flyout are all the OS's. This app draws NO titlebar and
// declares no drag region — the point of the mode is that it does not have to.
//
// The `window-drag: move` rule below is deliberately still in the stylesheet:
// it is INERT here and says so once (A-25). That is the compat-first shape —
// the same component in `titleBarStyle: "hidden"` would drag the window, and
// silently doing nothing is exactly the failure the advisory exists to prevent.
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "vue-native";

const state = ref({});
const read = () => {
  const r = navigator.windowControlsOverlay.getTitlebarAreaRect();
  state.value = {
    titleBarStyle: appWindow.chrome.titleBarStyle,
    displayMode: appWindow.chrome.displayMode,
    wcoVisible: navigator.windowControlsOverlay.visible,
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
      The titlebar above is Windows', not this app's. `titleBarStyle: "default"`
      in vn.config.json, which is creation-time only — a window style cannot
      change mid-session without desyncing gpui's cached frame metrics (D-25).
    </p>

    <!-- Inert in this mode, and it says so once. See the comment above. -->
    <div class="fakebar">window-drag: move — inert here (advisory A-25)</div>

    <div class="grid">
      <template v-for="(v, k) in state" :key="k">
        <span class="k">{{ k }}</span>
        <span class="v">{{ String(v) }}</span>
      </template>
    </div>

    <div class="row">
      <button class="btn" @click="appWindow.backdrop = 'mica'; read()">mica</button>
      <button class="btn" @click="appWindow.backdrop = 'acrylic'; read()">acrylic</button>
      <button class="btn" @click="appWindow.backdrop = 'none'; read()">none</button>
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
  /* Would drag the window under `titleBarStyle: "hidden"`. Does nothing here. */
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
