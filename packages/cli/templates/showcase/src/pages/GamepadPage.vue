<script setup lang="ts">
import { defineKeymap, SkKbd, type SkKbdGamepadLayout } from "@skathi-native/runtime";
import { gamepads, type Controller, type ControllerEvent } from "skathi:gamepad";
import { computed, onMounted, onScopeDispose, ref, shallowRef, watch } from "vue";
import Card from "../components/Card.vue";
import GamepadDeadZone from "../components/GamepadDeadZone.vue";
import GamepadReadout from "../components/GamepadReadout.vue";
import Icon from "../components/Icon.vue";
import Page from "../components/Page.vue";
import { gamepadLayouts } from "../gamepadLayouts";
import { gamepadReadoutInterval, useShowcaseGamepads } from "../useShowcaseGamepads";

const {
  available: webAvailable,
  slots,
  events,
  hapticStatus,
  playRumble,
  resetRumble,
  clearHapticStatus,
} = useShowcaseGamepads();
const connected = shallowRef<readonly Controller[]>([]);
const nativeId = ref<number>();
const webIndex = ref<number>();
const nativePad = computed(() => connected.value.find((p) => p.id === nativeId.value) ?? connected.value[0]);
// SkKbd only reads the controller's identity. Keep its props stable while the sticks and buttons move.
const nativeLegend = computed<Controller | undefined>((previous) => {
  const pad = nativePad.value;
  return previous?.id === pad?.id && previous?.mapping === pad?.mapping ? previous : pad;
});
const visiblePads = computed(() => slots.value.filter((p): p is Gamepad => p !== null));
const webPad = computed(() => visiblePads.value.find((p) => p.index === webIndex.value) ?? visiblePads.value[0]);
const dualRumble = computed(() => webPad.value?.vibrationActuator?.effects.includes("dual-rumble") ?? false);
const webButtons = computed(() => webPad.value?.buttons.map((b) => b.value) ?? []);
const nativeReceipt = ref("");
const motor = ref<"both" | "strong" | "weak">("both");
const recent = shallowRef<readonly { key: number; message: string }[]>([]);
let sequence = 0;
const deviceStates = new Map<number, Controller>();
let updateTimer: ReturnType<typeof setTimeout> | undefined;
let resync = false;
let unsubscribe: (() => void) | undefined;
const confirms = ref(0);
const legend = ref<SkKbdGamepadLayout | "auto">("auto");
const confirm = defineKeymap({ keys: { alternatives: ["Enter", { gamepad: "south" }] } });
onScopeDispose(confirm.bind(() => confirms.value++));

function refresh() {
  updateTimer = undefined;
  if (resync) {
    deviceStates.clear();
    for (const pad of gamepads.list()) deviceStates.set(pad.id, pad);
    resync = false;
  }
  connected.value = Array.from(deviceStates.values()).sort((a, b) => a.index - b.index);
}
function scheduleRefresh() {
  if (updateTimer === undefined) updateTimer = setTimeout(refresh, gamepadReadoutInterval);
}
function resyncDevices() {
  clearTimeout(updateTimer);
  resync = true;
  refresh();
}
function onFocus() {
  resync = true;
  scheduleRefresh();
}
function record(event: ControllerEvent) {
  if (event.type === "disconnected") deviceStates.delete(event.gamepad.id);
  else deviceStates.set(event.gamepad.id, event.gamepad);
  scheduleRefresh();
  if (event.type === "axis" || (event.type === "button" && !event.edge)) return;
  const change = event.type === "button" ? (event.button ?? "Button " + event.index) + " " + event.edge : event.type;
  recent.value = [
    { key: ++sequence, message: "Slot " + event.gamepad.index + " · " + change },
    ...recent.value.slice(0, 5),
  ];
}
onMounted(() => {
  resyncDevices();
  unsubscribe = gamepads.subscribe(record);
  window.addEventListener("focus", onFocus);
});
onScopeDispose(() => {
  unsubscribe?.();
  window.removeEventListener("focus", onFocus);
  clearTimeout(updateTimer);
});
watch(
  () => nativePad.value?.id,
  () => {
    nativeReceipt.value = "";
  },
);
watch(() => webPad.value?.index, clearHapticStatus);
function rumble(stop: boolean) {
  const pad = nativePad.value;
  if (!pad?.canRumble) return;
  const sent = gamepads.rumble(pad.id, {
    strong: motor.value === "weak" ? 0 : 0.65,
    weak: motor.value === "strong" ? 0 : 0.65,
    durationMs: stop ? 0 : 800,
  });
  nativeReceipt.value = sent ? (stop ? "Stopped" : "Sent an 800 ms pulse") : "Controller declined the request";
}
const hex = (n: number) => n.toString(16).padStart(4, "0").toUpperCase();
</script>

<template>
  <Page
    id="gamepad-page"
    title="Gamepad"
    lead="Connect a controller to inspect its inputs, compare the two APIs and try the motors."
  >
    <Card id="gamepad-native" title="Native input" api="skathi:gamepad">
      <template v-if="nativePad">
        <label class="row choice"
          >Controller
          <select
            :value="nativePad.id"
            class="input"
            aria-label="Native controller"
            @change="nativeId = Number(($event.target as HTMLSelectElement).value)"
          >
            <option v-for="p in connected" :key="p.id" :value="p.id">{{ p.index }} · {{ p.name }}</option>
          </select>
        </label>
        <div class="row facts">
          <span>{{ nativePad.family }}</span
          ><span>{{ nativePad.mapping || "Unmapped" }}</span> <span>Connection {{ nativePad.id }}</span
          ><span class="mono">{{ hex(nativePad.vendorId) }}:{{ hex(nativePad.productId) }}</span>
        </div>
        <GamepadReadout :controller="nativeLegend" :axes="nativePad.axisValues" :buttons="nativePad.buttonValues" />
        <GamepadDeadZone
          :controller="nativeLegend ?? nativePad"
          :effective="nativePad.deadZone"
          @change="resyncDevices"
        />
      </template>
      <div v-else id="gamepad-native-empty" class="empty">
        <Icon name="gamepad" />
        <div>
          <strong>{{ gamepads.available ? "No controller connected" : "Gamepad support is unavailable" }}</strong>
          <p class="muted">
            {{
              gamepads.available
                ? "Connect over USB or Bluetooth. For Steam Input, select a layout that outputs a gamepad."
                : "Open Showcase in a host built with the gamepad feature."
            }}
          </p>
        </div>
      </div>
      <div class="row outputs">
        <select v-model="motor" class="input" aria-label="Rumble motors" :disabled="!nativePad?.canRumble">
          <option value="both">Both motors</option>
          <option value="strong">Low frequency</option>
          <option value="weak">High frequency</option>
        </select>
        <button id="gamepad-native-rumble" class="btn" :disabled="!nativePad?.canRumble" @click="rumble(false)">
          Test rumble
        </button>
        <button class="btn" :disabled="!nativePad?.canRumble" @click="rumble(true)">Stop</button>
        <output class="muted" role="status">{{
          nativePad && !nativePad.canRumble ? "Rumble unavailable on this device" : nativeReceipt
        }}</output>
      </div>
      <div v-if="recent.length" class="history">
        <strong>Recent input</strong>
        <div v-for="entry in recent" :key="entry.key" class="mono">{{ entry.message }}</div>
      </div>
    </Card>
    <Card id="gamepad-web" title="Web Gamepad API" api="navigator.getGamepads()">
      <template v-if="webPad">
        <label class="row choice"
          >Web slot
          <select
            :value="webPad.index"
            class="input"
            aria-label="Web gamepad slot"
            @change="webIndex = Number(($event.target as HTMLSelectElement).value)"
          >
            <option v-for="p in visiblePads" :key="p.index" :value="p.index">{{ p.index }} · {{ p.id }}</option>
          </select>
        </label>
        <div class="row facts">
          <span>{{ webPad.mapping || "Unmapped" }}</span
          ><span>Timestamp {{ webPad.timestamp.toFixed(1) }} ms</span>
        </div>
        <GamepadReadout :axes="webPad.axes" :buttons="webButtons" />
      </template>
      <p v-else id="gamepad-web-empty" class="muted">
        {{
          webAvailable
            ? "Focus this window and press a controller button to expose its web slot."
            : "This host does not expose navigator.getGamepads()."
        }}
      </p>
      <div class="row outputs">
        <button id="gamepad-web-rumble" class="btn" :disabled="!dualRumble" @click="webPad && playRumble(webPad.index)">
          Play dual-rumble
        </button>
        <button class="btn" :disabled="!dualRumble" @click="webPad && resetRumble(webPad.index)">Reset effect</button>
        <output class="muted" role="status">{{
          webPad && !dualRumble ? "Dual-rumble unavailable on this device" : hapticStatus
        }}</output>
      </div>
      <div v-if="events.length" class="history">
        <strong>Connection events</strong>
        <div v-for="(event, i) in events" :key="i" class="mono">
          {{ event.type }} · {{ event.index }} · {{ event.id }}
        </div>
      </div>
    </Card>
    <Card title="Try a binding" api="defineKeymap · SkKbd">
      <div class="binding">
        <div>
          <strong>Confirm selection</strong>
          <p class="muted">Press either input to count a confirmation.</p>
        </div>
        <select v-model="legend" class="input" aria-label="Preview controller legends">
          <option v-for="[value, label] in gamepadLayouts" :key="value" :value="value">{{ label }}</option>
        </select>
        <SkKbd :map="confirm" :controller="nativeLegend" :gamepad-layout="legend === 'auto' ? undefined : legend" />
        <output class="count" tabindex="0"
          ><span id="gamepad-confirm-count">{{ confirms }}</span> {{ confirms === 1 ? "press" : "presses" }}</output
        >
      </div>
    </Card>
  </Page>
</template>

<style scoped>
.empty,
.binding {
  display: flex;
  align-items: center;
  gap: 14px;
}
.empty {
  padding: 10px 0;
}
.empty > svg {
  color: var(--accent);
  width: 30px;
  height: 30px;
  flex-shrink: 0;
}
strong {
  font-size: 13px;
}
p {
  margin: 0;
}
.empty p,
.binding p {
  margin-top: 4px;
}
.choice,
.facts,
.count {
  font-size: 12px;
}
.choice .input {
  max-width: 100%;
}
.facts {
  color: var(--text-muted);
  gap: 12px;
}
.outputs {
  border-top: 1px solid var(--line);
  padding-top: 12px;
}
.history {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.history .mono {
  color: var(--text-muted);
}
.binding {
  flex-wrap: wrap;
}
.binding > div {
  margin-right: auto;
}
.count {
  color: var(--text-muted);
}
</style>
