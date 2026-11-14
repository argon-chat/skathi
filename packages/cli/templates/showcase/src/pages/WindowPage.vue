<script setup lang="ts">
import { appWindow, SkWindow } from "@skathi-native/runtime";
import { onMounted, onUnmounted, ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { useCounter } from "../stores/counter";

const counter = useCounter();

const title = ref(appWindow.title);
const size = ref("");
const onTop = ref(appWindow.alwaysOnTop);
const fullscreen = ref(appWindow.isFullscreen());
const backdrop = ref(appWindow.backdrop);
const popped = ref(false);

const BACKDROPS = ["mica", "mica-alt", "acrylic", "none"];
const SIZES = [
  [900, 620],
  [1100, 720],
  [1280, 800],
] as const;

// `resize` is the standard event on `window`; appWindow carries the state the web has no event for.
const read = () => {
  size.value = `${Math.round(innerWidth)} × ${Math.round(innerHeight)}`;
  fullscreen.value = appWindow.isFullscreen();
};
onMounted(() => {
  read();
  addEventListener("resize", read);
});
onUnmounted(() => removeEventListener("resize", read));

const setSize = (width: number, height: number) => {
  appWindow.setBounds({ width, height });
  appWindow.center();
};
const toggleOnTop = () => {
  appWindow.alwaysOnTop = !appWindow.alwaysOnTop;
  onTop.value = appWindow.alwaysOnTop;
};
const setBackdrop = (material: string) => {
  appWindow.backdrop = material;
  backdrop.value = appWindow.backdrop;
};
const toggleFullscreen = () => {
  appWindow.setFullscreen(!appWindow.isFullscreen());
  read();
};
</script>

<template>
  <Page title="Window" lead="appWindow drives the OS window directly; every call is synchronous.">
    <Card title="Title" api="appWindow.title">
      <div class="row">
        <input v-model="title" class="input wide" @keyup.enter="appWindow.title = title" />
        <button class="btn" @click="appWindow.title = title">Set</button>
      </div>
      <p class="faint">Shown in the taskbar and Alt+Tab; this window paints its own caption.</p>
    </Card>

    <Card title="Size and state" :api="size">
      <div class="row">
        <button v-for="[w, h] in SIZES" :key="w" class="btn" @click="setSize(w, h)">{{ w }} × {{ h }}</button>
      </div>
      <div class="row">
        <button class="btn" @click="appWindow.minimize()">Minimize</button>
        <button class="btn" @click="appWindow.toggleMaximize()">Maximize</button>
        <button class="btn" :class="{ on: fullscreen }" @click="toggleFullscreen">Fullscreen</button>
        <button class="btn" :class="{ on: onTop }" @click="toggleOnTop">Always on top</button>
      </div>
    </Card>

    <Card title="Backdrop" api="appWindow.backdrop">
      <div class="row">
        <button v-for="b in BACKDROPS" :key="b" class="btn" :class="{ on: backdrop === b }" @click="setBackdrop(b)">
          {{ b }}
        </button>
      </div>
      <p class="faint">The page background is translucent, so the material behind the window shows through.</p>
    </Card>

    <Card title="Pop-out window" api="<SkWindow>">
      <div class="row">
        <button class="btn primary" @click="popped = !popped">{{ popped ? "Close it" : "Pop out a counter" }}</button>
        <span class="muted">count = {{ counter.count }}</span>
      </div>
      <p class="faint">
        A second OS window painting part of this component: same tree, same store, nothing serialized.
      </p>
      <SkWindow
        v-if="popped"
        title="Counter"
        :bounds="{ width: 280, height: 200 }"
        always-on-top
        :resizable="false"
        @close="popped = false"
      >
        <div class="mini">
          <div class="mini-count">{{ counter.count }}</div>
          <button class="btn primary" @click="counter.increment()">Count up</button>
        </div>
      </SkWindow>
    </Card>
  </Page>
</template>

<style scoped>
.wide {
  width: 320px;
}
.mini {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  background: #0b0b10;
  color: var(--text);
  font-family: var(--font);
}
.mini-count {
  font-size: 44px;
  font-weight: 700;
}
</style>
