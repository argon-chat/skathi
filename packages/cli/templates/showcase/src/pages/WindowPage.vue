<script setup lang="ts">
import { appWindow, SkWindow } from "@skathi-native/runtime";
import { computed, onMounted, onUnmounted, ref } from "vue";
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
const wideCounter = ref(true);
const opacity = ref(100);
const allWorkspaces = ref(false);
const insetLights = ref(false);
const mac = navigator.platform.startsWith("Mac");
const linux = navigator.platform.startsWith("Linux");
const counterOptions = computed(() => ({
  ...(wideCounter.value && { aspectRatio: 16 / 9 }),
  ...(mac && insetLights.value && { trafficLightPosition: { x: 20, y: 9 } }),
}));
const counterSize = ref("");
function measureCounter({ width, height }: { width: number; height: number }) {
  counterSize.value = `${Math.round(width)} × ${Math.round(height)}`;
}

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
  appWindow.addEventListener("fullscreenchange", read);
});
onUnmounted(() => {
  removeEventListener("resize", read);
  appWindow.removeEventListener("fullscreenchange", read);
});

const setSize = (width: number, height: number) => {
  const animate = !matchMedia("(prefers-reduced-motion: reduce)").matches;
  const x = Math.round(screen.availLeft + (screen.availWidth - width) / 2);
  const y = Math.round(screen.availTop + (screen.availHeight - height) / 2);
  appWindow.setBounds({ x, y, width, height }, { animate });
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
  <Page title="Window" lead="Resize the main window or open a counter with its own native size and appearance.">
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
        <button class="btn" @click="appWindow.center()">Center</button>
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
      <div class="row">
        <label class="option"><input v-model="wideCounter" type="checkbox" /> Lock to 16:9</label>
        <label class="option">
          Opacity
          <input v-model.number="opacity" type="range" min="50" max="100" step="5" aria-label="Pop-out opacity" />
          {{ opacity }}%
        </label>
        <label v-if="mac || linux" class="option">
          <input v-model="allWorkspaces" type="checkbox" /> Request all workspaces
        </label>
        <label v-if="mac" class="option"><input v-model="insetLights" type="checkbox" /> Inset traffic lights</label>
      </div>
      <p class="muted">
        Resize the counter between 256 × 144 and 640 × 420.
        {{ popped && counterSize ? "Current size " + counterSize + "." : "" }}
      </p>
      <p v-if="linux" class="faint">
        Opacity and workspace pinning depend on the X11 compositor and window manager. Wayland manages both itself.
      </p>
      <SkWindow
        v-if="popped"
        title="Counter"
        :bounds="{ width: 352, height: 198 }"
        :min-width="256"
        :min-height="144"
        :max-width="640"
        :max-height="420"
        v-bind="counterOptions"
        :opacity="opacity / 100"
        :visible-on-all-workspaces="allWorkspaces"
        always-on-top
        @resize="measureCounter"
        @show="measureCounter"
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
.option {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-muted);
}
.wide {
  width: 320px;
  max-width: 100%;
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
