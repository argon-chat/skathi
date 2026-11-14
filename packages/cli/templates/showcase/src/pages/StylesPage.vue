<script setup lang="ts">
import { ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const hot = ref(false);
</script>

<template>
  <Page
    title="Styles"
    lead="Ordinary CSS, mapped onto the GPU renderer. Add a tailwind.config.ts next to sk.config.json and Tailwind 3.4 classes work too."
  >
    <Card title="Gradients and shadows" api="background · box-shadow">
      <div class="row gap">
        <div class="swatch g1">linear</div>
        <div class="swatch g2">radial</div>
        <div class="swatch g3">conic</div>
        <div class="swatch lifted">layered</div>
        <div class="swatch ring">ring</div>
      </div>
    </Card>

    <Card title="Glass" api="backdrop-filter: blur()">
      <div class="scene">
        <div class="blob b1" />
        <div class="blob b2" />
        <div class="blob b3" />
        <div class="glass">Frosted panel over moving colour</div>
      </div>
    </Card>

    <Card title="Engine shading" api="--shading">
      <div class="row gap">
        <div class="swatch frost">frost</div>
        <div class="swatch noise">noise</div>
        <div class="swatch glow" :class="{ hot }" @click="hot = !hot">glow · click</div>
      </div>
      <p class="faint">Skathi's own effects, in a custom property that a browser simply ignores.</p>
    </Card>

    <Card title="Layout" api="grid · ::before · :hover">
      <div class="grid">
        <div class="cell wide">grid-column: span 2</div>
        <div class="cell">1fr</div>
        <div class="cell status" data-state="online">online</div>
        <div class="cell status" data-state="away">away</div>
        <div class="cell hover">hover me</div>
      </div>
    </Card>
  </Page>
</template>

<style scoped>
.gap {
  gap: 14px;
}
.swatch {
  width: 112px;
  height: 64px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  font-size: 12px;
  font-weight: 600;
  color: #ffffff;
  background: #1d1d2a;
}
.g1 {
  background: linear-gradient(135deg, #8b9dff, #b48cff);
}
.g2 {
  background: radial-gradient(circle at 30% 30%, #6ee7b7, #0f766e);
}
.g3 {
  background: conic-gradient(#f472b6, #8b9dff, #6ee7b7, #f472b6);
}
.lifted {
  box-shadow:
    0 12px 24px -8px #000000cc,
    0 2px 6px #00000080,
    inset 0 1px 0 #ffffff26;
}
.ring {
  box-shadow:
    0 0 0 2px #0b0b10,
    0 0 0 5px #8b9dff;
}
.scene {
  position: relative;
  z-index: 1;
  height: 150px;
  border-radius: 12px;
  background: #101018;
  overflow: hidden;
}
@keyframes drift {
  50% {
    transform: translateX(80px);
  }
}
.blob {
  position: absolute;
  width: 110px;
  height: 110px;
  border-radius: 55px;
  animation: drift 6s ease-in-out infinite;
}
.b1 {
  left: 40px;
  top: 10px;
  background: #8b9dff;
}
.b2 {
  left: 230px;
  top: 40px;
  background: #f472b6;
  animation-duration: 8s;
}
.b3 {
  left: 420px;
  top: 0px;
  background: #34d399;
  animation-duration: 7s;
}
.glass {
  position: absolute;
  left: 24px;
  right: 24px;
  top: 34px;
  bottom: 34px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  border: 1px solid #ffffff26;
  background: #ffffff14;
  backdrop-filter: blur(18px);
  font-size: 14px;
  font-weight: 600;
}
.frost {
  background: linear-gradient(135deg, #3b82f6, #a855f7);
  --shading: frost(0.85, 14px, #cfe9ff1f);
}
.noise {
  background: linear-gradient(135deg, #f59e0b, #ef4444);
  --shading: noise(0.4, 2px, 18);
}
.glow {
  cursor: pointer;
  --shading: glow(#8b9dff, 10px, 0.35);
  transition: --shading 420ms ease;
}
.glow.hot {
  --shading: glow(#ff2d95, 30px, 1);
}
.grid {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 8px;
}
.cell {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px;
  border-radius: 9px;
  background: var(--surface-hover);
  font-size: 12px;
  color: var(--text-muted);
}
.wide {
  grid-column: span 2;
}
.status::before {
  content: "";
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background: var(--text-faint);
}
.status[data-state="online"]::before {
  background: var(--ok);
}
.status[data-state="away"]::before {
  background: var(--warn);
}
.hover {
  transition:
    background-color 150ms ease,
    color 150ms ease;
}
.hover:hover {
  background: var(--accent-strong);
  color: #ffffff;
}
</style>
