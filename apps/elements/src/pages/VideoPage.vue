<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";

const video = ref<HTMLVideoElement | null>(null);
const playing = ref(false);
const time = ref(0);
const duration = ref(9);
const hovering = ref(false);
const scrubHover = ref(false);

function sync(e: Event) {
  const v = (e as SkMediaEvent).currentTime;
  time.value = v;
  const d = (e as SkMediaEvent).duration;
  if (d && Number.isFinite(d)) duration.value = d;
}

let raf = 0;
function tick() {
  const v = video.value;
  if (v && !v.paused) time.value = v.currentTime;
  raf = requestAnimationFrame(tick);
}
raf = requestAnimationFrame(tick);
onBeforeUnmount(() => cancelAnimationFrame(raf));

function toggle() {
  const v = video.value!;
  if (v.paused) void v.play();
  else v.pause();
}

const clock = (s: number) =>
  `${Math.floor(s / 60)}:${Math.floor(s % 60)
    .toString()
    .padStart(2, "0")}`;
const progress = computed(() => Math.min(1, time.value / (duration.value || 1)));
const shown = computed(() => hovering.value || !playing.value);
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;video&gt;</span>
      <p>Played by the OS, clipped by <code>border-radius</code>, with controls drawn by the app.</p>
    </div>
    <div class="stage">
      <div class="frame" data-demo="frame" @pointerenter="hovering = true" @pointerleave="hovering = false">
        <video
          ref="video"
          class="clip"
          src="assets/daylight.mp4"
          poster="assets/daylight-poster.jpg"
          autoplay
          muted
          loop
          @loadedmetadata="sync"
          @timeupdate="sync"
          @playing="playing = true"
          @pause="playing = false"
        />
        <div class="chip">
          <span class="dot" />
          Valley time-lapse
        </div>
        <div class="controls" :class="{ shown }">
          <button class="round" data-demo="play" @click="toggle">
            <svg v-if="playing" width="18" height="18" viewBox="0 0 18 18">
              <rect x="4" y="3" width="3.6" height="12" rx="1.2" fill="currentColor" />
              <rect x="10.4" y="3" width="3.6" height="12" rx="1.2" fill="currentColor" />
            </svg>
            <svg v-else width="18" height="18" viewBox="0 0 18 18">
              <path
                d="M5 3.2v11.6c0 .8.9 1.3 1.6.8l8.6-5.8c.6-.4.6-1.2 0-1.6L6.6 2.4C5.9 1.9 5 2.4 5 3.2z"
                fill="currentColor"
              />
            </svg>
          </button>
          <span class="time">{{ clock(time) }} / {{ clock(duration) }}</span>
          <div
            class="scrub"
            data-demo="scrub"
            :class="{ hot: scrubHover }"
            @pointerenter="scrubHover = true"
            @pointerleave="scrubHover = false"
          >
            <div class="rail">
              <div class="fill" :style="{ width: `${progress * 100}%` }" />
            </div>
            <div class="thumb" :style="{ left: `${progress * 100}%` }" />
          </div>
          <svg class="icon" width="20" height="20" viewBox="0 0 20 20" fill="none">
            <path d="M3 8v4h3l4 3.5v-11L6 8H3z" fill="currentColor" />
            <path
              d="M13 7.5c1 .8 1.5 1.6 1.5 2.5s-.5 1.7-1.5 2.5"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
            />
          </svg>
          <span class="hd">HD</span>
          <svg class="icon" width="20" height="20" viewBox="0 0 20 20" fill="none">
            <path
              d="M3.5 8V3.5H8M12 3.5h4.5V8M16.5 12v4.5H12M8 16.5H3.5V12"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.head code {
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
.frame {
  position: relative;
  width: 1008px;
  height: 567px;
  border-radius: 26px;
  overflow: hidden;
  background: #000;
  box-shadow: 0 24px 60px #0007;
}
.clip {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.chip {
  position: absolute;
  left: 22px;
  top: 20px;
  height: 32px;
  padding: 0 14px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 16px;
  background: #17161ecc;
  color: var(--text);
  font-size: 13px;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background: var(--pink);
}
.controls {
  position: absolute;
  left: 18px;
  right: 18px;
  bottom: 18px;
  height: 60px;
  padding: 0 18px 0 8px;
  display: flex;
  align-items: center;
  gap: 16px;
  border-radius: 20px;
  background: #17161ed9;
  border: 1px solid #ffffff14;
  color: var(--text);
  opacity: 0;
  transform: translateY(10px);
  transition:
    opacity 260ms ease,
    transform 260ms cubic-bezier(0.16, 1, 0.3, 1);
}
.controls.shown {
  opacity: 1;
  transform: translateY(0);
}
.round {
  width: 44px;
  height: 44px;
  border-radius: 22px;
  border: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--pink);
  color: var(--ink);
  transition:
    background-color 120ms ease,
    transform 120ms ease;
}
.round:hover {
  background: #f9bddc;
}
.round:active {
  transform: scale(0.94);
}
.time {
  width: 86px;
  font-family: var(--mono);
  font-size: 13px;
  color: var(--body);
}
.scrub {
  position: relative;
  flex: 1;
  height: 28px;
  display: flex;
  align-items: center;
}
.rail {
  width: 100%;
  height: 5px;
  border-radius: 3px;
  background: #ffffff26;
  overflow: hidden;
  transition: height 150ms ease;
}
.scrub.hot .rail {
  height: 8px;
}
.fill {
  height: 100%;
  background: linear-gradient(90deg, var(--lilac), var(--pink));
}
.thumb {
  position: absolute;
  top: 50%;
  width: 14px;
  height: 14px;
  margin: -7px 0 0 -7px;
  border-radius: 7px;
  background: #ffffff;
  box-shadow: 0 0 0 4px #f7a8d055;
  transform: scale(0);
  transition: transform 150ms ease;
}
.scrub.hot .thumb {
  transform: scale(1);
}
.icon {
  color: var(--body);
}
.hd {
  padding: 1px 6px;
  border-radius: 5px;
  border: 1px solid #ffffff38;
  font-size: 11px;
  font-weight: 700;
  color: var(--body);
}
</style>
