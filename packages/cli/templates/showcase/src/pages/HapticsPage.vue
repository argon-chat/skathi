<script setup lang="ts">
import { appHaptics, type HapticOptions, type HapticPattern } from "@skathi-native/runtime";
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import Card from "../components/Card.vue";
import HapticCss from "../components/HapticCss.vue";
import Page from "../components/Page.vue";

const effects = [
  "generic",
  "hover",
  "click",
  "press",
  "release",
  "alignment",
  "level-change",
  "collide",
  "grow",
  "success",
  "error",
] as const;
type Pulse = (typeof effects)[number];
const gestures = { button: "Button activation", hover: "Pointer entry", step: "Slider step" } as const;
type Gesture = keyof typeof gestures;
const binding = reactive<Record<Gesture, Pulse | "off">>({ button: "success", hover: "hover", step: "level-change" });
const fallback = ref<Pulse | "off">("generic");
const when = ref<"now" | "next-frame">("now");
const level = ref(3);
const available = ref<HapticPattern[]>([]);
const device = ref("Current device");
const result = ref("Try a gesture to see its result.");
const count = ref(0);
const pulses = computed(() => available.value.filter(p => !p.endsWith("-continuous")));
const textures = computed(() => available.value.filter(p => p.endsWith("-continuous")));
let sliderPointer: number | undefined;
let pen: number | undefined;
let lastPointer: number | undefined;

function inspect(id?: number) {
  available.value = appHaptics.getSupportedPatterns(id);
}
function point(event: PointerEvent) {
  lastPointer = event.pointerId;
  device.value =
    event.pointerType === "pen" ? "Pen" : event.pointerType === "mouse" ? "Mouse or trackpad" : "Current device";
  inspect(event.pointerId);
}
function trackDevice(event: PointerEvent) {
  if (event.pointerId !== lastPointer) point(event);
}
function send(label: string, pattern: HapticPattern, options: HapticOptions) {
  const supported = appHaptics.getSupportedPatterns(options.pointerId);
  const accepted = appHaptics.perform(pattern, options);
  const selected = [pattern, options.fallback].find(p => typeof p === "string" && supported.includes(p));
  if (accepted) {
    const action = options.when === "next-frame" ? "Queued for next frame" : "Accepted";
    const effect = selected && selected !== pattern ? `${selected} (fallback for ${pattern})` : selected;
    result.value = effect ? `${label} · ${action} · ${effect}` : `${label} · ${action}`;
  } else {
    result.value = selected
      ? `${label} · Device declined ${selected}`
      : `${label} · ${pattern} unavailable for this device`;
  }
  count.value++;
  return accepted;
}
function trigger(key: Gesture, id?: number) {
  const pattern = binding[key];
  const label = gestures[key];
  if (pattern === "off") {
    result.value = `${label} · Feedback off`;
    return;
  }
  send(label, pattern, {
    ...(id !== undefined ? { pointerId: id } : {}),
    when: when.value,
    fallback: fallback.value === "off" ? false : fallback.value,
  });
}
function playDeviceEffect(pattern: HapticPattern) {
  send("Device effect", pattern, {
    ...(lastPointer !== undefined ? { pointerId: lastPointer } : {}),
    fallback: false,
  });
}
function step(event: Event) {
  const next = Number((event.currentTarget as HTMLInputElement).value);
  if (next === level.value) return;
  level.value = next;
  trigger("step", sliderPointer);
}
function startTexture(event: PointerEvent, pattern: HapticPattern) {
  if (pen !== undefined || event.pointerType !== "pen" || event.button !== 0) return;
  if (send("Pen contact", pattern, { pointerId: event.pointerId, fallback: false })) {
    pen = event.pointerId;
    (event.currentTarget as HTMLElement).setPointerCapture(pen);
  }
}
function stopTexture(event?: PointerEvent) {
  if (pen === undefined || (event && event.pointerId !== pen)) return;
  appHaptics.stop(pen);
  pen = undefined;
  result.value = "Pen lifted · Texture stopped";
}
onMounted(() => inspect());
onBeforeUnmount(() => stopTexture());
</script>

<template>
  <Page title="Haptics" lead="Choose how each gesture feels, then try it with your mouse, trackpad or pen.">
    <div class="stack" @pointerenter="point" @pointermove="trackDevice" @pointerdown.capture="point">
      <Card title="Gesture bindings" api="appHaptics.perform">
        <div v-for="(label, key) in gestures" :key="key" class="binding">
          <div class="sample">
            <span class="sample-label">{{ label }}</span>
            <button
              v-if="key === 'button'"
              class="btn primary"
              aria-label="Try the button gesture"
              @click="trigger('button')"
            >
              Confirm
            </button>
            <button
              v-else-if="key === 'hover'"
              class="btn hover-pad"
              aria-label="Try the hover gesture"
              @pointerenter="e => trigger('hover', e.pointerId)"
              @click="
                e => {
                  if (e.detail === 0) trigger('hover');
                }
              "
            >
              Hover here
            </button>
            <label v-else class="slider">
              <input
                type="range"
                min="1"
                max="5"
                step="1"
                aria-label="Haptic sample level"
                :value="level"
                @input="step"
                @pointerdown="sliderPointer = $event.pointerId"
                @keydown="sliderPointer = undefined"
                @pointerup="sliderPointer = undefined"
                @pointercancel="sliderPointer = undefined"
                @lostpointercapture="sliderPointer = undefined"
              />
              <span>{{ level }} / 5</span>
            </label>
          </div>
          <select v-model="binding[key]" class="input pattern" :aria-label="label + ' effect'">
            <option value="off">Off</option>
            <option v-for="p in effects" :key="p" :value="p">{{ p }}</option>
          </select>
        </div>
        <div class="routing">
          <label
            >Fallback
            <select v-model="fallback" class="input" aria-label="Fallback effect">
              <option value="off">None</option>
              <option v-for="p in effects" :key="p" :value="p">{{ p }}</option>
            </select>
          </label>
          <label
            >Timing
            <select v-model="when" class="input" aria-label="Feedback timing">
              <option value="now">Immediate</option>
              <option value="next-frame">Next frame</option>
            </select>
          </label>
        </div>
        <p class="receipt" role="status" tabindex="0">
          <span>{{ result }}</span>
          <span v-if="count" class="mono">{{ count }} {{ count === 1 ? "request" : "requests" }}</span>
        </p>
      </Card>

      <Card title="Device effects" api="getSupportedPatterns()">
        <div class="device">
          <span class="muted">{{ device }} · {{ available.length }} available</span>
          <button class="btn" aria-label="Refresh device effects" @click="inspect(lastPointer)">Refresh</button>
        </div>
        <p v-if="!available.length" class="muted">
          Use a haptic device in this window, then press Refresh. On a Force Touch trackpad, press firmly once.
        </p>
        <div v-if="pulses.length" class="row">
          <button
            v-for="p in pulses"
            :key="p"
            class="btn pulse"
            :aria-label="'Play ' + p"
            @click="playDeviceEffect(p)"
          >
            {{ p }}
          </button>
        </div>
        <template v-if="textures.length">
          <p class="muted">Hold a texture with the pen tip. Lift to stop.</p>
          <div class="row">
            <button
              v-for="p in textures"
              :key="p"
              class="btn"
              :aria-label="'Hold ' + p"
              @pointerdown="e => startTexture(e, p)"
              @pointerup="stopTexture"
              @pointercancel="stopTexture"
              @lostpointercapture="stopTexture"
            >
              {{ p }}
            </button>
          </div>
        </template>
      </Card>
      <HapticCss />
    </div>
  </Page>
</template>

<style scoped>
.binding,
.routing,
.device,
.receipt {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px 18px;
}
.binding {
  justify-content: space-between;
  padding: 4px 0 12px;
  border-bottom: 1px solid var(--line);
}
.sample,
.slider,
.routing label {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px 12px;
}
.sample-label {
  width: 112px;
  font-size: 13px;
}
.pattern {
  width: 134px;
}
.hover-pad {
  border-style: dashed;
}
.hover-pad:hover {
  color: var(--accent);
  border-color: var(--accent);
  background: var(--accent-soft);
}
.slider input {
  width: 104px;
}
.slider span {
  color: var(--text-muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.routing {
  padding-top: 2px;
}
.routing label {
  color: var(--text-muted);
  font-size: 12px;
}
.device,
.receipt {
  justify-content: space-between;
}
.receipt {
  margin: 0;
  padding: 9px 12px;
  border-radius: 8px;
  background: var(--accent-soft);
  font-size: 13px;
}
.pulse {
  font-family: var(--mono);
  font-size: 12px;
}
@media (max-width: 680px) {
  .sample {
    flex-direction: column;
    align-items: flex-start;
  }
  .sample-label {
    width: auto;
  }
}
</style>
