<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { appHost } from "@skathi-native/runtime";

const PERIOD = 8;
const size = ref("");
const ready = ref(false);
const presented = ref(0);

// The palette as linear-ish RGB stops; the producer gets one frame per animation frame.
const STOPS: [number, number, number][] = [
  [247, 168, 208],
  [201, 167, 245],
  [159, 205, 245],
  [236, 127, 182],
];
function colorAt(t: number): [number, number, number] {
  const f = ((t / PERIOD) % 1) * STOPS.length;
  const i = Math.floor(f);
  const k = f - i;
  const e = k * k * (3 - 2 * k);
  const a = STOPS[i]!;
  const b = STOPS[(i + 1) % STOPS.length]!;
  return [0, 1, 2].map((c) => Math.round(a[c]! + (b[c]! - a[c]!) * e)) as [number, number, number];
}

let raf = 0;
let busy = false;
function frame(now: number) {
  raf = requestAnimationFrame(frame);
  if (busy) return;
  const [r, g, b] = colorAt(now / 1000);
  busy = true;
  appHost
    .call("present", "viewport", "bgra", [b, g, r, 255], 64, 36)
    .then(() => presented.value++)
    .catch(() => {})
    .finally(() => (busy = false));
}
onMounted(() => (raf = requestAnimationFrame(frame)));
onBeforeUnmount(() => cancelAnimationFrame(raf));

function resized(e: SkSurfaceEvent) {
  size.value = `${e.width} × ${e.height}`;
}
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;sk-surface&gt;</span>
      <p>Frames presented from Rust, clipped by <code>border-radius</code>, with the app's UI on top.</p>
    </div>
    <div class="stage">
      <div class="viewport">
        <sk-surface class="surface" name="viewport" @surfaceresize="resized" @surfaceready="ready = true" />
        <div class="scrim" />
        <div class="top">
          <span class="chip"><span class="live" />SurfaceHandle::present_bgra</span>
          <span class="chip mono">{{ size }} px</span>
        </div>
        <div class="crosshair">
          <svg width="64" height="64" viewBox="0 0 64 64" fill="none">
            <circle cx="32" cy="32" r="22" stroke="#ffffffb0" stroke-width="2" />
            <path d="M32 2v14M32 48v14M2 32h14M48 32h14" stroke="#ffffffb0" stroke-width="2" stroke-linecap="round" />
          </svg>
        </div>
        <div class="bottom">
          <div class="title">
            <b>Native viewport</b>
            <span>Every <code>&lt;sk-surface name="viewport"&gt;</code> shows the newest frame.</span>
          </div>
          <div class="thumb">
            <sk-surface class="mini" name="viewport" />
            <span>same surface</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.head code,
.title code {
  font-family: var(--mono);
  font-size: 13px;
  color: var(--lilac);
}
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.viewport {
  position: relative;
  width: 1008px;
  height: 567px;
  border-radius: 34px;
  overflow: hidden;
  background: #241f2e;
  box-shadow: 0 24px 60px #0007;
}
.surface {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.scrim {
  position: absolute;
  inset: 0;
  background:
    radial-gradient(ellipse at 50% 45%, #ffffff30 0%, #ffffff00 55%),
    linear-gradient(180deg, #17161e33 0%, #17161e00 30%, #17161e00 55%, #17161ecc 100%);
}
.top {
  position: absolute;
  left: 24px;
  right: 24px;
  top: 22px;
  display: flex;
  justify-content: space-between;
}
.chip {
  height: 32px;
  padding: 0 14px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 16px;
  background: #17161ecc;
  color: var(--text);
  font-size: 13px;
  font-family: var(--mono);
}
.live {
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background: #7fd6a4;
}
.crosshair {
  position: absolute;
  left: 50%;
  top: 46%;
  width: 64px;
  height: 64px;
  margin: -32px 0 0 -32px;
}
.bottom {
  position: absolute;
  left: 30px;
  right: 30px;
  bottom: 26px;
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
}
.title {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.title b {
  font-family: var(--font-display);
  font-size: 34px;
  font-weight: 600;
  color: #ffffff;
}
.title span {
  font-size: 14px;
  color: #ffffffcc;
}
.thumb {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: #ffffffcc;
}
.mini {
  width: 92px;
  height: 92px;
  border-radius: 50%;
  border: 3px solid #ffffffd0;
  object-fit: cover;
}
</style>
