<script setup lang="ts">
import { ref } from "vue";

const hot = ref(false);
</script>

<template>
  <div class="card">
    <div class="title">Styles — Tailwind · backdrop-filter · --shading</div>

    <div
      class="flex flex-col gap-2 p-4 rounded-xl bg-white/10 hover:bg-white/25 transition-colors duration-300 border-2 border-emerald-400/70"
    >
      <div class="text-sm font-semibold text-white">Tailwind card</div>
      <div class="text-xs text-white/70">flex flex-col gap-2 p-4 rounded-xl bg-white/10 — hover me</div>
      <div class="text-xs font-mono text-sky-300">hover:bg-white/25 · transition-colors duration-300</div>
    </div>

    <!-- backdrop-filter: blur() — GPU backdrop blur (gpui BackdropBlurRect) Two identical
         translucent panels sit over the same colour stripes. -->
    <div class="sub">backdrop-filter: blur(16px) — left panel only</div>
    <div class="bf-demo">
      <div class="bf-stripes">
        <div class="bf-s bf-s1"></div>
        <div class="bf-s bf-s2"></div>
        <div class="bf-s bf-s3"></div>
        <div class="bf-s bf-s4"></div>
        <div class="bf-s bf-s5"></div>
        <div class="bf-s bf-s6"></div>
      </div>
      <div class="bf-panel bf-blur">blur(16px)</div>
      <div class="bf-panel bf-plain">no filter</div>
    </div>

    <div class="fx">
      <div class="fx-title">--shading — frost · noise · glow</div>

      <div class="fx-demo">
        <div class="fx-stripes">
          <div class="fx-s fx-s1"></div>
          <div class="fx-s fx-s2"></div>
          <div class="fx-s fx-s3"></div>
          <div class="fx-s fx-s4"></div>
          <div class="fx-s fx-s5"></div>
          <div class="fx-s fx-s6"></div>
        </div>
        <div class="fx-panel fx-frost">frost(0.85, 14px)</div>
        <div class="fx-panel fx-plain">no --shading</div>
      </div>

      <div class="fx-row">
        <div class="fx-card fx-noise">
          <div class="fx-card-name">noise(0.4, 2px, 18)</div>
          <div class="fx-card-note">animated film grain</div>
        </div>

        <div class="fx-card fx-glow" :class="{ 'fx-hot': hot }" @click="hot = !hot">
          <div class="fx-card-name">glow — click me</div>
          <div class="fx-card-note">{{ hot ? "#ff2d95 · 34px · 1" : "#7fd4ff · 10px · 0.35" }}</div>
        </div>
      </div>
    </div>

    <div class="sub">gradients · shadows · outline · grid · ::after</div>
    <div class="m15">
      <!-- linear-gradient(): gpui's two-stop Background. `to right` is 90deg in both CSS and gpui,
           so the ramp runs the same way. -->
      <div class="g-row">
        <div class="g g-a">to right</div>
        <div class="g g-b">135deg</div>
        <div class="g g-c">to bottom</div>
      </div>

      <div class="g-row">
        <div class="sh">3 shadows + inset</div>
        <div class="ring">ring (box-shadow)</div>
        <div class="ol">outline + offset</div>
      </div>

      <div class="gr">
        <div class="cell">1fr</div>
        <div class="cell">auto</div>
        <div class="cell">120px</div>
        <div class="cell span2">grid-column: span 2</div>
        <div class="cell">3</div>
      </div>

      <!-- ::before / ::after: real boxes, first and last child of the element, laid out by the same
           rules as a hand-written span. -->
      <div class="pseudo" data-state="open">badge</div>
      <div class="pseudo" data-state="closed">badge</div>
    </div>
  </div>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 14px;
  color: #ffffff;
}
.sub {
  font-size: 12px;
  color: #ffffffcc;
}

.m15 {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.g-row {
  display: flex;
  gap: 12px;
}

.g {
  width: 118px;
  height: 52px;
  padding: 8px;
  border-radius: 10px;
  font-size: 11px;
  color: #0b0b12;
}
.g-a {
  background-image: linear-gradient(to right, #7ee8fa, #eec0c6);
}
.g-b {
  background: linear-gradient(135deg, #fbc7a4, #ffe29f);
}
.g-c {
  background-image: linear-gradient(#a1ffce, #faffd1);
}

.sh {
  width: 118px;
  height: 52px;
  padding: 8px;
  border-radius: 10px;
  font-size: 11px;
  color: #ffffff;
  background: #1d2333;
  box-shadow:
    0 10px 20px -6px #000000aa,
    0 2px 6px #00000066,
    inset 0 1px 0 #ffffff30;
}
.ring {
  width: 118px;
  height: 52px;
  padding: 8px;
  border-radius: 10px;
  font-size: 11px;
  color: #ffffff;
  background: #1d2333;
  box-shadow:
    0 0 0 2px #0b0b12,
    0 0 0 5px #38bdf8;
}
.ol {
  width: 118px;
  height: 52px;
  padding: 8px;
  border-radius: 10px;
  font-size: 11px;
  color: #ffffff;
  background: #1d2333;
  outline: 2px solid #f472b6;
  outline-offset: 3px;
}

.gr {
  display: grid;
  grid-template-columns: 1fr auto 120px;
  gap: 8px;
  width: 380px;
}
.cell {
  padding: 8px;
  border-radius: 8px;
  background: #ffffff1a;
  font-size: 11px;
  color: #ffffffdd;
}
.span2 {
  grid-column: span 2;
  background: #ffffff2e;
}

/* ::before and ::after are children, first and last. */
.pseudo {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 380px;
  padding: 8px 10px;
  border-radius: 8px;
  background: #ffffff14;
  font-size: 11px;
  color: #ffffffdd;
  text-transform: uppercase;
}
.pseudo::before {
  content: "";
  width: 8px;
  height: 8px;
  border-radius: 8px;
  background: #34d399;
}
.pseudo[data-state="open"]::after {
  content: "open";
  padding: 2px 8px;
  border-radius: 6px;
  background: #34d39933;
  color: #34d399;
}
.pseudo[data-state="closed"]::after {
  content: "closed";
  padding: 2px 8px;
  border-radius: 6px;
  background: #ffffff1a;
  color: #ffffff88;
}

.bf-demo {
  position: relative;
  width: 380px;
  height: 120px;
}
.bf-stripes {
  display: flex;
  flex-direction: row;
  width: 380px;
  height: 120px;
  border-radius: 10px;
  overflow: hidden;
}
.bf-s {
  width: 64px;
  height: 120px;
}
.bf-s1 {
  background: #ff3b30;
}
.bf-s2 {
  background: #ffcc00;
}
.bf-s3 {
  background: #34c759;
}
.bf-s4 {
  background: #00c7ff;
}
.bf-s5 {
  background: #5e5ce6;
}
.bf-s6 {
  background: #ff2d95;
}

.bf-panel {
  position: absolute;
  top: 22px;
  width: 160px;
  height: 76px;
  padding: 8px;
  background: #ffffff2e;
  border: 1px solid #ffffff55;
  border-radius: 14px;
  font-size: 12px;
  color: #ffffff;
}
.bf-blur {
  left: 18px;
  backdrop-filter: blur(16px);
}
.bf-plain {
  left: 202px;
}
</style>

<style scoped>
.fx {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
  border-radius: 12px;
  background: #ffffff0a;
  border: 1px solid #ffffff1f;
}
.fx-title {
  font-size: 12px;
  font-weight: 600;
  color: #ffffffcc;
}

.fx-demo {
  position: relative;
  width: 380px;
  height: 120px;
}
.fx-stripes {
  display: flex;
  flex-direction: row;
  width: 380px;
  height: 120px;
  border-radius: 10px;
  overflow: hidden;
}
.fx-s {
  width: 64px;
  height: 120px;
}
.fx-s1 {
  background: #ff3b30;
}
.fx-s2 {
  background: #ffcc00;
}
.fx-s3 {
  background: #34c759;
}
.fx-s4 {
  background: #00c7ff;
}
.fx-s5 {
  background: #5e5ce6;
}
.fx-s6 {
  background: #ff2d95;
}

.fx-panel {
  position: absolute;
  top: 22px;
  width: 160px;
  height: 76px;
  padding: 8px;
  background: #ffffff1a;
  border: 1px solid #ffffff55;
  border-radius: 14px;
  font-size: 12px;
  color: #ffffff;
}
.fx-frost {
  left: 18px;
  --shading: frost(0.85, 14px, #cfe9ff1f);
}
.fx-plain {
  left: 202px;
}

.fx-row {
  display: flex;
  flex-direction: row;
  gap: 44px;
  padding: 18px 22px;
}
.fx-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 150px;
  padding: 12px;
  border-radius: 12px;
  font-size: 12px;
  color: #ffffff;
}
.fx-card-name {
  font-weight: 600;
}
.fx-card-note {
  font-size: 11px;
  color: #ffffff8c;
}

.fx-noise {
  background: #2b3350;
  border: 1px solid #ffffff2e;
  --shading: noise(0.4, 2px, 18);
}

/* Driven by a click-toggled class, because state-rule transitions are rejected by design. */
.fx-glow {
  background: #1d2233;
  border: 1px solid #ffffff2e;
  --shading: glow(#7fd4ff, 10px, 0.35);
  transition: --shading 420ms ease;
}
.fx-glow.fx-hot {
  --shading: glow(#ff2d95, 34px, 1);
}
</style>
