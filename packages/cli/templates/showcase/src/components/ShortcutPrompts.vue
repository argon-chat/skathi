<script setup lang="ts">
import { defineKeymap, SkKbd, type SkKbdGamepadLayout, type SkKbdKeys, type SkKbdPlatform } from "@skathi-native/runtime";
import { onScopeDispose, ref, useTemplateRef } from "vue";
import Card from "./Card.vue";
import { appCommand, useDesktop } from "../stores/desktop";

const devices = ["Keyboard", "Mouse", "Gamepad"] as const;
type Device = (typeof devices)[number];
const device = ref<Device>("Keyboard");
const platform = ref<SkKbdPlatform>("auto");
const gamepad = ref<SkKbdGamepadLayout>("generic");
const primary = ref<"left" | "right">("left");
const lastMouse = ref<SkKbdKeys | null>(null);
const area = useTemplateRef<HTMLElement>("gesture-area");
const listening = ref(true);
const gesture = ref("Ready");
const input = defineKeymap({
  keys: { alternatives: [{ mouse: "primary" }, "Space"] },
  when: (event) => event?.target instanceof Node && !!area.value?.contains(event.target),
});
onScopeDispose(input.on("down", () => { gesture.value = "Down"; }));
onScopeDispose(input.on("up", (event) => { if (event.cancelled) gesture.value = "Ready"; }));
onScopeDispose(input.on("hold", () => { gesture.value = "Hold"; }));
onScopeDispose(input.on("click", () => { gesture.value = "Single click"; }));
onScopeDispose(input.on("doubleClick", () => { gesture.value = "Double click"; }));

function toggleGestures() {
  if (input.enabled) input.disable();
  else input.enable();
  listening.value = input.enabled;
  gesture.value = input.enabled ? "Ready" : "Paused";
}
const desktop = useDesktop();
const examples = {
  Keyboard: [
    ["Command palette", "Mod+Shift+P"],
    ["Comment selection", { sequence: ["Mod+K", "Mod+C"] }],
    ["Zoom in", { alternatives: [["Mod", "+"], "Mod+NumpadAdd"] }],
    ["Push to talk", { buttons: [{ device: 0, code: 0xa3 }, { device: 1, code: 4 }] }],
  ],
  Mouse: [
    ["Select", { mouse: "primary" }],
    ["Context menu", { mouse: "secondary" }],
    ["Pan view", { mouse: "middle" }],
    ["Go back", { mouse: "back" }],
    ["Go forward", { mouse: "forward" }],
    ["Zoom in", ["Mod", { mouse: "wheel-up" }]],
  ],
  Gamepad: [
    ["Confirm", { alternatives: ["Enter", { gamepad: 0, mapping: "standard" }] }],
    ["Cancel", { gamepad: "east" }],
    ["Reload", { gamepad: "west" }],
    ["Switch tool", { gamepad: "north" }],
    ["Aim and fire", [{ gamepad: "left-trigger" }, { gamepad: "right-trigger" }]],
    ["Sprint", { gamepad: "left-stick" }],
    ["Next item", { gamepad: "dpad-right" }],
  ],
} as const satisfies Record<Device, readonly (readonly [string, SkKbdKeys])[]>;

function showWheel(event: WheelEvent) {
  const { deltaX: x, deltaY: y } = event;
  if (!x && !y) return;
  if (Math.abs(x) > Math.abs(y)) lastMouse.value = { mouse: x < 0 ? "wheel-left" : "wheel-right" };
  else lastMouse.value = { mouse: y < 0 ? "wheel-up" : "wheel-down" };
}
</script>

<template>
  <Card title="Input shortcuts" api="<SkKbd>">
    <div class="row controls">
      <div class="row" role="group" aria-label="Shortcut device">
        <button
          v-for="kind in devices"
          :key="kind"
          class="btn"
          :class="{ on: device === kind }"
          :aria-pressed="device === kind"
          @click="device = kind"
        >
          {{ kind }}
        </button>
      </div>
      <label v-if="device === 'Keyboard'" class="row muted">
        Platform
        <select v-model="platform" class="input" aria-label="Shortcut platform">
          <option value="auto">This device</option>
          <option value="windows">Windows</option>
          <option value="macos">macOS</option>
          <option value="linux">Linux</option>
        </select>
      </label>
      <label v-else-if="device === 'Mouse'" class="row muted">
        Primary button
        <select v-model="primary" class="input" aria-label="Primary mouse button">
          <option value="left">Left</option>
          <option value="right">Right</option>
        </select>
      </label>
      <label v-else class="row muted">
        Controller
        <select v-model="gamepad" class="input" aria-label="Controller layout">
          <option value="generic">Generic</option>
          <option value="xbox">Xbox</option>
          <option value="playstation">PlayStation · DualSense</option>
          <option value="nintendo">Nintendo · Switch Pro</option>
        </select>
      </label>
    </div>
    <div v-if="device === 'Keyboard'" class="row map-example">
      <div class="map-description">
        <strong>In this app</strong>
        <p class="muted">Switch keyboard layouts or press the shortcut in any Showcase window.</p>
      </div>
      <div class="row">
        <output aria-label="App shortcut activations">{{ desktop.appPresses }} {{ desktop.appPresses === 1 ? "press" : "presses" }}</output>
        <SkKbd :map="appCommand" />
      </div>
    </div>
    <div v-if="device === 'Mouse'" class="row controls">
      <span id="gesture-instructions" class="muted">Click, double-click, or hold in the box. You can also use Space.</span>
      <button class="btn" aria-label="Recognize gestures" :aria-pressed="listening" @click="toggleGestures">
        {{ listening ? "Pause" : "Resume" }}
      </button>
    </div>
    <div v-if="device === 'Mouse'"
      id="gesture-area"
      ref="gesture-area"
      class="row mouse-test"
      role="group"
      tabindex="0"
      aria-describedby="gesture-instructions"
      aria-label="Mouse input area"
      @mousedown="lastMouse = { mouse: $event.button }"
      @contextmenu.prevent
      @wheel.prevent="showWheel"
    >
      <output aria-label="Recognized gesture" aria-live="polite">{{ gesture }}</output>
      <output aria-live="polite"><SkKbd :keys="lastMouse" :mouse-primary="primary" tabindex="0" /></output>
    </div>
    <div v-for="[name, keys] in examples[device]" :key="name" class="row shortcut">
      <span>{{ name }}</span>
      <SkKbd :keys="keys" :platform="platform" :gamepad-layout="gamepad" :mouse-primary="primary" />
    </div>
  </Card>
</template>

<style scoped>
.controls,
.map-example,
.shortcut,
.mouse-test {
  justify-content: space-between;
}
.map-example {
  padding: 10px 12px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  font-size: 13px;
}
.map-description {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
}
.map-description p {
  margin: 0;
}
.mouse-test output {
  display: flex;
}
.shortcut {
  min-height: 32px;
  font-size: 13px;
}
.mouse-test {
  min-height: 48px;
  padding: 8px 12px;
  border: 1px dashed var(--line-strong);
  border-radius: 8px;
  color: var(--text-muted);
  font-size: 13px;
}
</style>
