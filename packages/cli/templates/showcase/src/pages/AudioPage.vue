<script setup lang="ts">
import { computed, onUnmounted, reactive, ref, shallowRef, watch } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { everyFrame } from "../engine";

const NOTES = [
  ["C", 261.63],
  ["D", 293.66],
  ["E", 329.63],
  ["G", 392.0],
  ["A", 440.0],
  ["C", 523.25],
  ["D", 587.33],
  ["E", 659.25],
] as const;
const WAVES = ["sine", "triangle", "square", "sawtooth"];
const BARS = 32;

const wave = ref("triangle");
const levels = ref<number[]>(Array.from({ length: BARS }, () => 0));

let engine: { ctx: AudioContext; tap: AnalyserNode; stop: () => void } | null = null;

// Created on the first note, so the audio device is not opened until something plays.
function start() {
  const ctx = new AudioContext();
  const tap = ctx.createAnalyser();
  tap.fftSize = 128;
  tap.connect(ctx.destination);
  const bins = new Uint8Array(tap.frequencyBinCount);
  const stop = everyFrame(() => {
    tap.getByteFrequencyData(bins);
    levels.value = levels.value.map((_, i) => (bins[i] ?? 0) / 255);
  });
  return { ctx, tap, stop };
}

function play(frequency: number) {
  engine ??= start();
  const { ctx, tap } = engine;
  const now = ctx.currentTime;
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = wave.value;
  osc.frequency.value = frequency;
  gain.gain.setValueAtTime(0.0001, now);
  gain.gain.exponentialRampToValueAtTime(0.3, now + 0.02);
  gain.gain.exponentialRampToValueAtTime(0.0001, now + 1.2);
  osc.connect(gain);
  gain.connect(tap);
  osc.start(now);
  osc.stop(now + 1.25);
}

// Spatial: one source on a PannerNode, placed in a 7.1.4 room around the listener. Web Audio's axes:
// +x right, +y up, -z ahead, in metres.

type Model = "spatial" | "HRTF" | "equalpower";
type Sound = "pink" | "clicks" | "tone";
type Motion = "orbit" | "fly";

const MODELS: [Model, string][] = [
  ["spatial", "Spatial"],
  ["HRTF", "HRTF"],
  ["equalpower", "Equal power"],
];
const SOUNDS: [Sound, string][] = [
  ["pink", "Pink noise"],
  ["clicks", "Clicks"],
  ["tone", "Tone"],
];
const RADIUS = 2;
const ROOM = 3;
const PX = 40;
const Y_MIN = -1;
const Y_MAX = 2.5;
const FLY_SECONDS = 6;
// The bed's channels in WAVE order, as the destination numbers them.
const BED = ["FL", "FR", "C", "LFE", "BL", "BR", "SL", "SR", "TFL", "TFR", "TBL", "TBR"];

const rad = (deg: number) => (deg * Math.PI) / 180;
const SPEAKERS = (
  [
    ["FL", -30, 0],
    ["C", 0, 0],
    ["FR", 30, 0],
    ["SL", -90, 0],
    ["SR", 90, 0],
    ["BL", -135, 0],
    ["BR", 135, 0],
    ["TFL", -45, 45],
    ["TFR", 45, 45],
    ["TBL", -135, 45],
    ["TBR", 135, 45],
  ] as const
).map(([id, az, el]) => ({
  id,
  height: el > 0,
  x: Math.sin(rad(az)) * Math.cos(rad(el)) * RADIUS,
  y: Math.sin(rad(el)) * RADIUS,
  z: -Math.cos(rad(az)) * Math.cos(rad(el)) * RADIUS,
  // Labels sit outside the ring for the ear-level speakers and inside it for the heights.
  tag: {
    left: `${7 + Math.sin(rad(az)) * (el > 0 ? -16 : 18)}px`,
    top: `${7 - Math.cos(rad(az)) * (el > 0 ? -16 : 18)}px`,
  },
}));
const EAR_SPEAKERS = SPEAKERS.filter((s) => !s.height);
const HEIGHT_SPEAKERS = SPEAKERS.filter((s) => s.height);

// Seen from the left side, one speaker of each row stands for its pair.
const SIDE_SPEAKERS = SPEAKERS.filter((s) => ["C", "BL", "TFL", "TBL"].includes(s.id));

const pos = reactive({ x: 0, y: 0, z: -RADIUS });
const model = ref<Model>("spatial");
const sound = ref<Sound>("pink");
const volume = ref(0.1);
const speed = ref(0.1);
const motion = ref<Motion | null>(null);
const playing = ref(false);
const testing = ref(false);
const lit = ref<string | null>(null);
const status = ref("Off");

const motionQuery = matchMedia("(prefers-reduced-motion: reduce)");
const reduced = ref(motionQuery.matches);
const onMotionPreference = () => (reduced.value = motionQuery.matches);
motionQuery.addEventListener("change", onMotionPreference);
// With reduced motion the dot stays where the motion began; the sound still moves.
const still = reactive({ x: 0, y: 0, z: 0 });
const shown = computed(() => (reduced.value && motion.value ? still : pos));

const clampRoom = (v: number) => Math.min(ROOM, Math.max(-ROOM, v));
const roomPoint = (p: { x: number; z: number }) => ({
  left: `${(p.x + ROOM) * PX}px`,
  top: `${(p.z + ROOM) * PX}px`,
});
const sidePoint = (p: { y: number; z: number }) => ({
  left: `${(p.z + ROOM) * PX}px`,
  top: `${(Y_MAX - p.y) * PX}px`,
});
const dotScale = computed(() => 0.75 + ((shown.value.y - Y_MIN) / (Y_MAX - Y_MIN)) * 0.7);

interface Rig {
  ctx: AudioContext;
  level: GainNode;
  panner: PannerNode;
  buffers: Map<Sound, AudioBuffer>;
  source: AudioBufferSourceNode | null;
  stopStatus: number;
}
let rig: Rig | null = null;

function makeBuffer(ctx: AudioContext, seconds: number, fill: (data: Float32Array, rate: number) => void) {
  const rate = ctx.sampleRate;
  const data = new Float32Array(Math.round(seconds * rate));
  fill(data, rate);
  const buffer = ctx.createBuffer(1, data.length, rate);
  buffer.copyToChannel(data, 0);
  return buffer;
}

// Paul Kellet's pink noise filter.
function pinkNoise(data: Float32Array) {
  let [b0, b1, b2, b3, b4, b5, b6] = [0, 0, 0, 0, 0, 0, 0];
  for (let i = 0; i < data.length; i++) {
    const white = Math.random() * 2 - 1;
    b0 = 0.99886 * b0 + white * 0.0555179;
    b1 = 0.99332 * b1 + white * 0.0750759;
    b2 = 0.969 * b2 + white * 0.153852;
    b3 = 0.8665 * b3 + white * 0.3104856;
    b4 = 0.55 * b4 + white * 0.5329522;
    b5 = -0.7616 * b5 - white * 0.016898;
    data[i] = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362) * 0.11;
    b6 = white * 0.115926;
  }
}

function buffersFor(ctx: AudioContext) {
  return new Map<Sound, AudioBuffer>([
    ["pink", makeBuffer(ctx, 2, pinkNoise)],
    [
      "clicks",
      makeBuffer(ctx, 1, (data, rate) => {
        const length = Math.round(rate * 0.004);
        for (let start = 0; start < data.length; start += Math.round(rate / 4)) {
          for (let i = 0; i < length && start + i < data.length; i++) {
            data[start + i] = (Math.random() * 2 - 1) * (1 - i / length);
          }
        }
      }),
    ],
    [
      "tone",
      makeBuffer(ctx, 1, (data, rate) => {
        for (let i = 0; i < data.length; i++) data[i] = 0.5 * Math.sin((2 * Math.PI * 440 * i) / rate);
      }),
    ],
  ]);
}

function describeOutput(ctx: AudioContext, panner: PannerNode) {
  const dest = ctx.destination;
  const out = `${dest.channelCount} of ${dest.maxChannelCount} output channels`;
  const s = ctx.spatialAudio;
  if (panner.panningModel !== "spatial") return `${panner.panningModel} · ${out}`;
  if (!s) return `HRTF (no spatial output in this build) · ${out}`;
  if (s.active) return `${s.format} · ${s.objectsInUse}/${s.maxObjects} objects · ${out}`;
  return `HRTF · ${s.reason ?? "opening the spatial output"} · ${out}`;
}

function ensureRig(): Rig {
  if (rig) return rig;
  const ctx = new AudioContext();
  const panner = ctx.createPanner();
  panner.panningModel = model.value;
  panner.refDistance = RADIUS;
  const gain = ctx.createGain();
  gain.gain.value = 0;
  gain.connect(panner).connect(ctx.destination);
  const update = () => {
    status.value = describeOutput(ctx, panner);
  };
  rig = { ctx, level: gain, panner, buffers: buffersFor(ctx), source: null, stopStatus: setInterval(update, 400) };
  place();
  update();
  return rig;
}

function place() {
  if (!rig) return;
  rig.panner.positionX.value = pos.x;
  rig.panner.positionY.value = pos.y;
  rig.panner.positionZ.value = pos.z;
}
watch(pos, place);
watch(model, (m) => {
  if (rig) rig.panner.panningModel = m;
});

function fadeTo(value: number) {
  if (rig) rig.level.gain.setTargetAtTime(value, rig.ctx.currentTime, 0.02);
}
watch(volume, (v) => {
  if (playing.value && !testing.value) fadeTo(v);
});

function startSource() {
  const r = ensureRig();
  r.source?.stop();
  const source = r.ctx.createBufferSource();
  source.buffer = r.buffers.get(sound.value) ?? null;
  source.loop = true;
  source.connect(r.level);
  source.start();
  r.source = source;
}
watch(sound, () => {
  if (playing.value) startSource();
});

function togglePlay() {
  if (playing.value) {
    shutdown();
    return;
  }
  startSource();
  playing.value = true;
  fadeTo(volume.value);
}

function shutdown() {
  testing.value = false;
  playing.value = false;
  stopMotion();
  lit.value = null;
  if (rig) {
    clearInterval(rig.stopStatus);
    void rig.ctx.close();
    rig = null;
  }
  status.value = "Off";
}

const room = shallowRef<HTMLElement | null>(null);
let dragging = false;
function dragTo(e: PointerEvent) {
  if (!room.value) return;
  const r = room.value.getBoundingClientRect();
  pos.x = clampRoom((e.clientX - r.left) / PX - ROOM);
  pos.z = clampRoom((e.clientY - r.top) / PX - ROOM);
}
function grab(e: PointerEvent) {
  e.preventDefault();
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  stopMotion();
  dragging = true;
  dragTo(e);
}
const drag = (e: PointerEvent) => dragging && dragTo(e);
const drop = () => (dragging = false);

function snap(s: { x: number; y: number; z: number }) {
  stopMotion();
  Object.assign(pos, { x: s.x, y: s.y, z: s.z });
}

let stopFrames: (() => void) | null = null;
function stopMotion() {
  stopFrames?.();
  stopFrames = null;
  motion.value = null;
}

function run(kind: Motion) {
  if (motion.value === kind) {
    stopMotion();
    return;
  }
  stopMotion();
  Object.assign(still, pos);
  motion.value = kind;
  let last: number | null = null;
  let angle = Math.atan2(pos.x, -pos.z);
  const distance = Math.hypot(pos.x, pos.z) > 0.3 ? Math.hypot(pos.x, pos.z) : RADIUS;
  let flown = 0;
  stopFrames = everyFrame((time) => {
    const dt = last === null ? 0 : Math.min(0.1, (time - last) / 1000);
    last = time;
    if (kind === "orbit") {
      angle += dt * speed.value * 2 * Math.PI;
      pos.x = Math.sin(angle) * distance;
      pos.z = -Math.cos(angle) * distance;
      return;
    }
    flown = Math.min(1, flown + dt / FLY_SECONDS);
    const arc = flown * Math.PI;
    Object.assign(pos, { x: 0, y: Math.sin(arc) * RADIUS, z: -Math.cos(arc) * RADIUS });
    if (flown >= 1) stopMotion();
  });
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

// Windows' speaker test: a burst on each bed channel in turn, through a discrete merger, so the
// multichannel bed is heard apart from the panner.
async function speakerTest() {
  if (testing.value) {
    testing.value = false;
    return;
  }
  const r = ensureRig();
  const { ctx } = r;
  const dest = ctx.destination;
  const channels = Math.min(dest.maxChannelCount, BED.length);
  const before = { count: dest.channelCount, interpretation: dest.channelInterpretation };
  dest.channelCount = channels;
  dest.channelInterpretation = "discrete";
  const merger = ctx.createChannelMerger(channels);
  merger.channelInterpretation = "discrete";
  merger.connect(dest);
  const burst = makeBuffer(ctx, 0.45, (data) => {
    pinkNoise(data);
    const fade = Math.round(data.length / 10);
    for (let i = 0; i < fade; i++) {
      data[i] = (data[i] ?? 0) * (i / fade);
      data[data.length - 1 - i] = (data[data.length - 1 - i] ?? 0) * (i / fade);
    }
  });
  testing.value = true;
  fadeTo(0);
  for (let k = 0; k < channels && testing.value && rig === r; k++) {
    const id = BED[k];
    if (!id || id === "LFE") continue;
    lit.value = id;
    const gain = ctx.createGain();
    gain.gain.value = volume.value;
    const source = ctx.createBufferSource();
    source.buffer = burst;
    source.connect(gain);
    gain.connect(merger, 0, k);
    source.start();
    await sleep(750);
  }
  lit.value = null;
  if (rig !== r) return;
  merger.disconnect();
  dest.channelCount = before.count;
  dest.channelInterpretation = before.interpretation;
  testing.value = false;
  if (playing.value) fadeTo(volume.value);
  else shutdown();
}

onUnmounted(() => {
  engine?.stop();
  engine?.ctx.suspend();
  motionQuery.removeEventListener("change", onMotionPreference);
  shutdown();
});
</script>

<template>
  <Page title="Audio" lead="The Web Audio API on a native audio engine: oscillators, gain envelopes, an analyser and spatial audio.">
    <Card title="Synth" api="AudioContext">
      <div class="keys">
        <button v-for="([name, hz], i) in NOTES" :key="i" class="key" @mousedown="play(hz)">{{ name }}</button>
      </div>
      <div class="row">
        <button v-for="w in WAVES" :key="w" class="btn" :class="{ on: wave === w }" @click="wave = w">{{ w }}</button>
      </div>
      <div class="spectrum">
        <div v-for="(level, i) in levels" :key="i" class="bar" :style="{ height: `${4 + level * 96}px` }" />
      </div>
    </Card>
    <Card title="Spatial" api='panningModel = "spatial"'>
      <div class="spatial">
        <div class="views">
          <div ref="room" class="room" @pointerdown="grab" @pointermove="drag" @pointerup="drop" @lostpointercapture="drop">
            <div class="ring" />
            <div class="axis across" />
            <div class="axis along" />
            <div
              v-for="s in SPEAKERS"
              :key="s.id"
              class="speaker"
              :class="{ height: s.height, lit: lit === s.id }"
              :style="roomPoint(s)"
            >
              <span class="tag" :style="s.tag">{{ s.id }}</span>
            </div>
            <div class="listener" :style="roomPoint({ x: 0, z: 0 })"><div class="nose" /></div>
            <div
              class="source"
              :class="{ above: shown.y > 0.05, below: shown.y < -0.05 }"
              :style="{ ...roomPoint(shown), transform: `translate(-50%, -50%) scale(${dotScale})` }"
            />
          </div>
          <div class="side">
            <div class="floor" :style="{ top: `${Y_MAX * PX}px` }" />
            <div
              v-for="s in SIDE_SPEAKERS"
              :key="s.id"
              class="speaker small"
              :class="{ height: s.height }"
              :style="sidePoint(s)"
            />
            <div class="listener small" :style="sidePoint({ y: 0, z: 0 })" />
            <div class="source small" :style="sidePoint(shown)" />
            <span class="side-label name">side view</span>
            <span class="side-label front">front</span>
            <span class="side-label back">back</span>
          </div>
        </div>
        <div class="controls">
          <div class="row">
            <button class="btn primary" @click="togglePlay">{{ playing ? "Stop" : "Play" }}</button>
            <button class="btn" :class="{ on: testing }" @click="speakerTest">{{ testing ? "Stop test" : "Speaker test" }}</button>
          </div>
          <div class="group">
            <span class="label">Model</span>
            <div class="row">
              <button v-for="[m, name] in MODELS" :key="m" class="btn" :class="{ on: model === m }" @click="model = m">{{ name }}</button>
            </div>
          </div>
          <div class="group">
            <span class="label">Sound</span>
            <div class="row">
              <button v-for="[s, name] in SOUNDS" :key="s" class="btn" :class="{ on: sound === s }" @click="sound = s">{{ name }}</button>
            </div>
          </div>
          <div class="group">
            <span class="label">Move to a speaker</span>
            <div class="snaps">
              <button v-for="s in EAR_SPEAKERS" :key="s.id" class="btn snap" @click="snap(s)">{{ s.id }}</button>
            </div>
            <div class="snaps">
              <button v-for="s in HEIGHT_SPEAKERS" :key="s.id" class="btn snap height" @click="snap(s)">{{ s.id }}</button>
            </div>
          </div>
          <div class="group">
            <span class="label">Motion</span>
            <div class="row">
              <button class="btn" :class="{ on: motion === 'orbit' }" @click="run('orbit')">Orbit</button>
              <button class="btn" :class="{ on: motion === 'fly' }" @click="run('fly')">Fly over</button>
              <input v-model.number="speed" class="speed" type="range" min="0.02" max="0.5" step="0.01" aria-label="Orbit speed" />
              <span class="faint mono">{{ (1 / speed).toFixed(0) }} s a lap</span>
            </div>
            <span v-if="reduced && motion" class="faint">Reduced motion: the dot stays put while the sound moves.</span>
          </div>
          <label class="slider">
            <span class="label">Height</span>
            <input v-model.number="pos.y" type="range" :min="Y_MIN" :max="Y_MAX" step="0.1" />
            <span class="faint mono">{{ pos.y.toFixed(1) }} m</span>
          </label>
          <label class="slider">
            <span class="label">Level</span>
            <input v-model.number="volume" type="range" min="0" max="0.5" step="0.01" />
            <span class="faint mono">{{ volume.toFixed(2) }}</span>
          </label>
        </div>
      </div>
      <span class="muted mono">{{ status }}</span>
    </Card>
  </Page>
</template>

<style scoped>
.keys {
  display: flex;
  gap: 6px;
}
.key {
  flex: 1;
  height: 120px;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-bottom: 12px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: linear-gradient(180deg, #ffffff14, #ffffff06);
  color: var(--text-muted);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition:
    background-color 80ms ease,
    transform 80ms ease;
}
.key:hover {
  border-color: var(--line-strong);
  color: var(--text);
}
.key:active {
  transform: translateY(2px);
  background: var(--accent-soft);
}
.spectrum {
  height: 100px;
  display: flex;
  align-items: flex-end;
  gap: 3px;
}
.bar {
  flex: 1;
  border-radius: 3px;
  background: linear-gradient(0deg, var(--accent-strong), #b48cff);
  transition: height 60ms linear;
}

.spatial {
  display: flex;
  flex-wrap: wrap;
  gap: 20px;
}
.views {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.room,
.side {
  position: relative;
  width: 240px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: #00000040;
  overflow: hidden;
}
.room {
  height: 240px;
  cursor: crosshair;
}
.side {
  height: 140px;
}
.ring {
  position: absolute;
  left: 40px;
  top: 40px;
  width: 160px;
  height: 160px;
  border-radius: 80px;
  border: 1px dashed var(--line-strong);
}
.axis {
  position: absolute;
  background: var(--line);
}
.axis.across {
  left: 0;
  right: 0;
  top: 120px;
  height: 1px;
}
.axis.along {
  top: 0;
  bottom: 0;
  left: 120px;
  width: 1px;
}
.floor {
  position: absolute;
  left: 0;
  right: 0;
  height: 1px;
  background: var(--line-strong);
}
.speaker {
  position: absolute;
  width: 14px;
  height: 14px;
  margin: -7px 0 0 -7px;
  border-radius: 3px;
  border: 1px solid var(--line-strong);
  background: var(--surface-strong);
  transition:
    background-color 120ms ease,
    box-shadow 120ms ease;
}
.speaker.height {
  border-radius: 7px;
  border-color: #b48cff99;
  background: #b48cff22;
}
.speaker.lit {
  background: var(--warn);
  border-color: var(--warn);
  box-shadow: 0 0 14px #fbbf2499;
}
.speaker.small {
  width: 10px;
  height: 10px;
  margin: -5px 0 0 -5px;
}
.tag {
  position: absolute;
  transform: translate(-50%, -50%);
  font-family: var(--mono);
  font-size: 9px;
  color: var(--text-faint);
}
.speaker.height .tag {
  color: #b48cffcc;
}
.listener {
  position: absolute;
  width: 18px;
  height: 18px;
  margin: -9px 0 0 -9px;
  border-radius: 9px;
  border: 2px solid var(--text-muted);
  background: var(--surface-strong);
}
.listener.small {
  width: 12px;
  height: 12px;
  margin: -6px 0 0 -6px;
  border-width: 1px;
}
.nose {
  position: absolute;
  left: 5px;
  top: -8px;
  width: 4px;
  height: 7px;
  border-radius: 2px;
  background: var(--text-muted);
}
.source {
  position: absolute;
  width: 18px;
  height: 18px;
  border-radius: 9px;
  background: var(--accent);
  box-shadow: 0 0 0 4px var(--accent-soft);
  pointer-events: none;
}
.source.above {
  background: #b48cff;
  box-shadow: 0 0 0 4px #b48cff33;
}
.source.below {
  opacity: 0.6;
}
.source.small {
  width: 12px;
  height: 12px;
  margin: -6px 0 0 -6px;
  box-shadow: none;
}
.side-label {
  position: absolute;
  bottom: 6px;
  font-size: 10px;
  color: var(--text-faint);
}
.side-label.name {
  top: 6px;
  bottom: auto;
  left: 8px;
}
.side-label.front {
  left: 8px;
}
.side-label.back {
  right: 8px;
}
.controls {
  flex: 1;
  min-width: 300px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.label {
  font-size: 12px;
  color: var(--text-faint);
}
.snaps {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.btn.snap {
  height: 28px;
  min-width: 44px;
  padding: 0 8px;
  font-family: var(--mono);
  font-size: 11px;
}
.btn.snap.height {
  border-color: #b48cff55;
}
.speed {
  width: 110px;
  accent-color: var(--accent-strong);
}
.slider {
  display: flex;
  align-items: center;
  gap: 10px;
  accent-color: var(--accent-strong);
}
.slider .label {
  width: 50px;
}
.slider input {
  flex: 1;
}
</style>
