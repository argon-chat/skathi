<script setup lang="ts">
import { onActivated, onDeactivated, ref } from "vue";
import Counter from "../components/Counter.vue";

const wide = ref(false);

const ends = ref(0);
const lastEnd = ref("—");
const onTransitionEnd = (e: SkTransitionEvent) => {
  ends.value++;
  lastEnd.value = `${e.propertyName} after ${e.elapsedTime}s`;
};
const spinning = ref(true);
const tilted = ref(false);
const animStarts = ref(0);
const animEnds = ref(0);
const animCancels = ref(0);

const playing = ref(true);
const speed = ref(1);
const loops = ref(0);
const lastLoopFrame = ref(0);

const onLoop = (e: SkLottieLoopEvent) => {
  loops.value = e.currentLoop;
  lastLoopFrame.value = Math.round(e.totalTime);
};

// So the counter keeps ticking while this page sits in the detached cache, exactly like in a
// browser.
onActivated(() => console.log("[app] MotionPage ACTIVATED"));
onDeactivated(() => console.log("[app] MotionPage DEACTIVATED (Counter keeps ticking)"));
</script>

<template>
  <div class="card">
    <div class="title">Motion — CSS transitions + &lt;lottie&gt; playback</div>
    <div class="anim" :class="{ wide }" @click="wide = !wide" @transitionend="onTransitionEnd">
      transition: {{ wide ? "wide" : "narrow" }} — click me
    </div>
    <div class="hint">transitionend fired {{ ends }}× — last: {{ lastEnd }} (a real event, not a timer)</div>

    <div class="kf-row">
      <div class="pulse">pulse</div>
      <div class="cycle">cycle</div>
      <div
        class="spinner"
        @click="spinning = !spinning"
        @animationstart="animStarts++"
        @animationend="animEnds++"
        @animationcancel="animCancels++"
      >
        <svg
          class="spin"
          :style="{ animationPlayState: spinning ? 'running' : 'paused' }"
          xmlns="http://www.w3.org/2000/svg"
          width="24"
          height="24"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
        >
          <path d="M21 12a9 9 0 1 1-6.219-8.56" />
        </svg>
      </div>
    </div>
    <div class="hint">
      animation events: start {{ animStarts }} · end {{ animEnds }} · cancel {{ animCancels }} — click the spinner to
      toggle animation-play-state ({{ spinning ? "running" : "paused" }})
    </div>

    <div class="tx-row">
      <div class="tx-card">
        rotate(15deg)
        <div class="tx-sub">text · border · shadow</div>
      </div>
      <div class="tx-spin-box">spin</div>
      <div class="tx-scale">scale on :hover</div>
      <div class="tx-hover" @click="tilted = !tilted" :class="{ tilted }">click to tilt<br />(transition)</div>
    </div>
    <div class="hint">
      1: transform: rotate(15deg) on a &lt;div&gt; — text, border and shadow turn with it. 2: @keyframes spin on a
      &lt;div&gt; rather than an &lt;svg&gt;. 3: scale(1.25) on :hover, with transition: transform. 4: click to
      transition rotate+scale+translate. Layout does not move — the row keeps its four upright slots, which is the
      browser's rule.
    </div>

    <!-- That is CSS 2.1 Appendix E's rule, and it costs no layout because an absolutely positioned
         box does not care where it sits among its siblings. -->
    <div class="z-card">
      <div class="z-under">z-index: -1 — behind the text, in front of the card's background</div>
      <div class="z-text">in-flow content</div>
      <div class="z-badge">sibling</div>
    </div>

    <div class="lottie-row">
      <lottie class="sticker" src="assets/sticker.tgs" loop :autoplay="playing" :speed="speed" @loopComplete="onLoop" />
      <div class="lottie-side">
        <div class="lottie-title">.tgs Telegram sticker · 512×512 · 60 fps</div>
        <div class="row">
          <div class="btn toggle" @click="playing = !playing">
            {{ playing ? "pause" : "play" }}
          </div>
          <div class="btn ghost" :class="{ on: speed === 0.5 }" @click="speed = 0.5">0.5×</div>
          <div class="btn ghost" :class="{ on: speed === 1 }" @click="speed = 1">1×</div>
          <div class="btn ghost" :class="{ on: speed === 2 }" @click="speed = 2">2×</div>
        </div>
        <div class="hint loops">
          @loopComplete fired {{ loops }}× · {{ lastLoopFrame }} frames per pass · speed {{ speed }}× ·
          {{ playing ? "playing" : "paused" }}
        </div>
      </div>
    </div>

    <Counter />
    <div class="label">MotionPage-scoped .label — must be BLUE</div>
    <div class="hint keep">
      &lt;KeepAlive&gt;: the counter's setInterval is never cleared on a tab switch — and neither is the sticker's
      clock. Leave, wait, come back: both have advanced.
    </div>
  </div>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 14px;
  color: #ffffff;
}
@keyframes pulse {
  50% {
    opacity: 0.25;
  }
}
@keyframes cycle {
  0% {
    background-color: #4f8cff59;
  }
  50% {
    background-color: #ff7ad959;
    animation-timing-function: steps(3, jump-end);
  }
  100% {
    background-color: #4f8cff59;
  }
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.kf-row {
  display: flex;
  flex-direction: row;
  gap: 10px;
  align-items: center;
}
.pulse,
.cycle,
.spinner {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 90px;
  height: 34px;
  border-radius: 8px;
  font-size: 11px;
  color: #ffffff;
  background: #ffffff1a;
  border: 1px solid #ffffff2e;
}
.pulse {
  animation: pulse 1.6s ease-in-out infinite;
}
.cycle {
  animation: cycle 2.4s linear infinite;
}
.spinner {
  width: 44px;
  cursor: pointer;
  color: #7fd4ff;
}
.spin {
  width: 20px;
  height: 20px;
  animation: spin 1s linear infinite;
}
.tx-row {
  display: flex;
  flex-direction: row;
  gap: 22px;
  align-items: center;
  padding: 14px 4px;
}
.tx-card,
.tx-spin-box,
.tx-scale,
.tx-hover {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 120px;
  height: 56px;
  border-radius: 10px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border: 1px solid #9dc0ff;
  box-shadow: 0 6px 14px #0009;
}
.tx-card {
  transform: rotate(15deg);
}
.tx-sub {
  font-size: 9px;
  color: #cfe0ff;
}
.tx-spin-box {
  width: 56px;
  animation: spin 2s linear infinite;
}
.tx-scale {
  transition: transform 250ms ease;
}
.tx-scale:hover {
  transform: scale(1.25);
}
.tx-hover {
  cursor: pointer;
  transition: transform 600ms ease;
}
.tx-hover.tilted {
  transform: rotate(-12deg) scale(1.1) translate(10px, 0);
}
.z-card {
  position: relative;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  height: 54px;
  padding: 0 12px;
  border-radius: 10px;
  background: #1d4ed8;
  border: 1px solid #93c5fd;
}
.z-under {
  position: absolute;
  inset: 0;
  z-index: -1;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  background: #16a34a;
  font-size: 10px;
  color: #dcfce7;
}
.z-text {
  font-size: 12px;
  color: #ffffff;
}
.z-badge {
  padding: 3px 8px;
  border-radius: 6px;
  background: #f59e0b;
  font-size: 10px;
  color: #1c1917;
}
.anim {
  transition:
    width 350ms ease,
    background-color 350ms ease,
    border-radius 350ms ease;
  width: 170px;
  padding: 8px;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  border-radius: 6px;
  font-size: 12px;
  color: #ffffff;
  cursor: pointer;
}
.anim.wide {
  width: 340px;
  background: #d0637c88;
  border-radius: 16px;
}

.lottie-row {
  display: flex;
  flex-direction: row;
  gap: 12px;
  align-items: center;
  padding: 10px;
  border-radius: 10px;
  background: #7fd4ff0f;
  border: 1px solid #7fd4ff33;
}
/* CSS width/height win over the animation's intrinsic 512×512 canvas — the <img> rule, and the
   reason a sticker does not blow the layout apart. */
.sticker {
  width: 88px;
  height: 88px;
}
.lottie-side {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.lottie-title {
  font-size: 12px;
  color: #7fd4ff;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 6px;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.btn:hover {
  background: #4f8cffcc;
}
.btn.ghost {
  background: #ffffff0d;
  border: 1px solid #ffffff2e;
  color: #ffffffcc;
}
.btn.ghost:hover {
  background: #ffffff22;
}
.btn.ghost.on {
  background: #7fd4ff40;
  border: 1px solid #7fd4ff88;
  color: #ffffff;
}
.label {
  font-size: 12px;
  color: #7fd4ff;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
.loops {
  color: #a5f3a5;
}
.keep {
  color: #a5f3a5;
}
</style>
