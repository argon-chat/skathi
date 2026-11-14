<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import Card from "../components/Card.vue";
import Missing from "../components/Missing.vue";
import Page from "../components/Page.vue";
import { audio, everyFrame } from "../engine";

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

let engine: { ctx: SkAudioContext; tap: SkAnalyserNode; stop: () => void } | null = null;

// Created on the first note, so the audio device is not opened until something plays.
function start(Context: NonNullable<typeof audio>) {
  const ctx = new Context();
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
  if (!audio) return;
  engine ??= start(audio);
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

onUnmounted(() => {
  engine?.stop();
  engine?.ctx.suspend();
});
</script>

<template>
  <Page title="Audio" lead="The Web Audio API on a native audio engine: oscillators, gain envelopes and an analyser.">
    <Card v-if="audio" title="Synth" api="AudioContext">
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
    <Missing v-else part="audio" />
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
