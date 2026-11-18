<script setup lang="ts">
import { ref } from "vue";

const playing = ref(true);
const frame = ref(0);
const total = ref(180);

function onFrame(e: SkLottieFrameEvent) {
  frame.value = e.currentTime;
  total.value = e.totalTime;
}
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;lottie&gt;</span>
      <p>A Lottie animation, played and paused by binding <code>autoplay</code>.</p>
    </div>
    <div class="stage">
      <div class="card hero">
        <lottie class="anim" src="assets/orbit.json" loop :autoplay="playing" @enterFrame="onFrame" />
        <div class="bar">
          <button class="play" data-demo="toggle" @click="playing = !playing">
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
          <div class="track">
            <div class="fill" :style="{ width: `${(frame / total) * 100}%` }" />
          </div>
          <span class="count">{{ Math.floor(frame).toString().padStart(3, "0") }} / {{ total }}</span>
          <span class="state" :class="{ paused: !playing }">{{ playing ? "playing" : "paused" }}</span>
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
.hero {
  width: 620px;
  padding: 26px 30px 24px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 18px;
  background: radial-gradient(circle at 50% 38%, #3a2f4a99 0%, var(--surface) 62%);
}
.anim {
  width: 420px;
  height: 420px;
}
.bar {
  align-self: stretch;
  display: flex;
  align-items: center;
  gap: 16px;
}
.play {
  width: 48px;
  height: 48px;
  border-radius: 24px;
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
.play:hover {
  background: #f9bddc;
}
.play:active {
  transform: scale(0.94);
}
.track {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: var(--border);
  overflow: hidden;
}
.fill {
  height: 100%;
  border-radius: 3px;
  background: linear-gradient(90deg, var(--lilac), var(--pink));
}
.count {
  width: 78px;
  font-family: var(--mono);
  font-size: 13px;
  color: var(--body);
}
.state {
  width: 74px;
  padding: 3px 0;
  text-align: center;
  border-radius: 999px;
  font-size: 12px;
  background: #2b3a33;
  color: #9fe0bd;
}
.state.paused {
  background: var(--accent-bg);
  color: var(--pink);
}
</style>
