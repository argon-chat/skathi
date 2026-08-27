<script setup>
import { onActivated, onDeactivated, ref } from "vue";
import Counter from "../components/Counter.vue";

const wide = ref(false);

// --- CSS animations + the events they fire ----------------------------------
// `transitionend` and `animationstart`/`end`/`cancel` come from vn-css's own
// animator (m12 items 3 and 6): the host drains its event queue once a frame
// and dispatches through the ordinary capture/bubble walk, so these are real
// DOM events with `propertyName` / `animationName` / `elapsedTime`.
const ends = ref(0);
const lastEnd = ref("—");
const onTransitionEnd = (e) => {
  ends.value++;
  lastEnd.value = `${e.propertyName} after ${e.elapsedTime}s`;
};
const spinning = ref(true);
const animStarts = ref(0);
const animEnds = ref(0);
const animCancels = ref(0);

// --- <lottie> ---------------------------------------------------------------
// A REAL Telegram sticker (.tgs = gzipped Bodymovin JSON), resolved through the
// same asset base as <img src>: "assets/…" means apps/hello/assets/… .
//
// `autoplay` and `loop` default to FALSE here, mirroring the <lottie-player>
// web component rather than lottie-web's loadAnimation() — see
// crates/vn-lottie/CONTRACT.md. So `loop` is written out, and `:autoplay` is
// bound to a ref, which is what makes it a play/pause switch.
const playing = ref(true);
const speed = ref(1);
const loops = ref(0);
const lastLoopFrame = ref(0);

// @loopComplete is lottie-web's event name, and it carries lottie-web's
// payload: currentLoop (1-based), currentTime (the frame at the boundary) and
// totalTime (frames in the animation).
const onLoop = (e) => {
  loops.value = e.currentLoop;
  lastLoopFrame.value = Math.round(e.totalTime);
};

// Counter's setInterval is started in onMounted and cleared in onUnmounted —
// and <KeepAlive> runs NEITHER on a tab switch. So the counter keeps ticking
// while this page sits in the detached cache, exactly like in a browser. If
// you wanted it frozen you would clear the timer in onDeactivated yourself:
// that is app policy, not renderer policy.
//
// The <lottie> above behaves the same way and for the same reason: a detached
// <lottie-player> keeps its rAF loop running in a browser, so ours keeps its
// clock advancing while the page is cached — come back and it is where it
// would have been, and the loop counter has moved on.
onActivated(() => console.log("[app] MotionPage ACTIVATED"));
onDeactivated(() => console.log("[app] MotionPage DEACTIVATED (Counter keeps ticking)"));
</script>

<template>
  <div class="card">
    <div class="title">Motion — CSS transitions + &lt;lottie&gt; playback</div>
    <div
      class="anim"
      :class="{ wide }"
      @click="wide = !wide"
      @transitionend="onTransitionEnd"
    >
      transition: {{ wide ? "wide" : "narrow" }} — click me
    </div>
    <div class="hint">
      transitionend fired {{ ends }}× — last: {{ lastEnd }} (a real event, not a timer)
    </div>

    <!-- ===== @keyframes ================================================== -->
    <!-- Three CSS animations on one row: an opacity pulse, a colour cycle with
         a mid keyframe, and `spin` on an inline <svg> — the only element a real
         rotation matrix can reach here (m12 items 1 / 3 / 4). -->
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
      animation events: start {{ animStarts }} · end {{ animEnds }} · cancel {{ animCancels }} —
      click the spinner to toggle animation-play-state ({{ spinning ? "running" : "paused" }})
    </div>

    <!-- ===== <lottie> ==================================================== -->
    <div class="lottie-row">
      <lottie
        class="sticker"
        src="assets/sticker.tgs"
        loop
        :autoplay="playing"
        :speed="speed"
        @loopComplete="onLoop"
      />
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
          @loopComplete fired {{ loops }}× · {{ lastLoopFrame }} frames per pass · speed
          {{ speed }}× · {{ playing ? "playing" : "paused" }}
        </div>
      </div>
    </div>

    <Counter />
    <!-- Scoped-CSS isolation: this `.label` and Counter's `.label` are two
         different rules under two different data-v hashes. -->
    <div class="label">MotionPage-scoped .label — must be BLUE</div>
    <div class="hint keep">
      &lt;KeepAlive&gt;: the counter's setInterval is never cleared on a tab switch — and neither is
      the sticker's clock. Leave, wait, come back: both have advanced.
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
/* ===== @keyframes ======================================================= */
/* Three shapes of animation, all through the same engine:
   - `pulse` interpolates one property back and forth, forever;
   - `cycle` has a MID keyframe with its own timing function, which is what
     makes the per-segment easing observable;
   - `spin` rotates, and rotation only reaches an inline <svg> — gpui carries a
     transformation matrix on a sprite and none at all on a laid out box. */
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
.anim {
  transition: width 350ms ease, background-color 350ms ease, border-radius 350ms ease;
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

/* ===== <lottie> block ==================================================== */
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
/* CSS width/height win over the animation's intrinsic 512×512 canvas — the
   <img> rule, and the reason a sticker does not blow the layout apart. */
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
