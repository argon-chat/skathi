<script setup lang="ts">
import { onUnmounted, ref } from "vue";
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

const status = ref("Off");
let orbit: (() => void) | null = null;
const orbiting = ref(false);

function describeOutput(ctx: AudioContext) {
  const s = ctx.spatialAudio;
  const out = `${ctx.destination.maxChannelCount} output channels`;
  if (!s) return `HRTF · ${out}`;
  if (s.active) return `${s.format} · ${s.objectsInUse}/${s.maxObjects} objects · ${out}`;
  return `HRTF · ${s.reason ?? "opening the output"}`;
}

// With a spatial sound format on (Windows Sonic, Dolby Atmos, DTS) the system renders the pulse as an
// audio object; elsewhere the same panner renders with HRTF.
function toggleOrbit() {
  if (orbit) {
    orbit();
    orbit = null;
    orbiting.value = false;
    status.value = "Off";
    return;
  }
  const ctx = new AudioContext();
  const osc = ctx.createOscillator();
  const level = ctx.createGain();
  const panner = ctx.createPanner();
  panner.panningModel = "spatial";
  osc.type = "triangle";
  osc.frequency.value = 330;
  level.gain.value = 0;
  osc.connect(level).connect(panner).connect(ctx.destination);
  osc.start();
  const stop = everyFrame((time) => {
    const t = time / 1000;
    panner.positionX.value = Math.sin(t) * 2;
    panner.positionZ.value = -Math.cos(t) * 2;
    level.gain.value = 0.08 * Math.max(0, Math.sin(t * 6));
    status.value = describeOutput(ctx);
  });
  orbit = () => {
    stop();
    void ctx.close();
  };
  orbiting.value = true;
}

onUnmounted(() => {
  engine?.stop();
  engine?.ctx.suspend();
  orbit?.();
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
      <div class="row">
        <button class="btn" :class="{ on: orbiting }" @click="toggleOrbit">{{ orbiting ? "Stop" : "Orbit" }}</button>
        <span class="muted">{{ status }}</span>
      </div>
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
</style>
