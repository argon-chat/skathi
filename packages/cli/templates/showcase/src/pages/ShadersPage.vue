<script setup lang="ts">
import { computed, ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { fullWindow, shatter } from "../fullwindow";

const LENS_WIDTH = 220;
const scene = shallowRef<HTMLElement | null>(null);
const lensX = ref(48);
let grab: number | null = null;
const lensDown = (e: MouseEvent) => (grab = e.clientX - lensX.value);
const lensUp = () => (grab = null);
function lensMove(e: MouseEvent) {
  if (grab === null || !scene.value) return;
  const room = scene.value.getBoundingClientRect().width - LENS_WIDTH;
  lensX.value = Math.min(Math.max(e.clientX - grab, 0), room);
}

const warm = ref(false);

const burnt = ref(false);

const cells = [6, 9, 14];
const cell = ref(9);
const halftone = computed(() => `halftone(${cell.value}px, 0.125, 1, #0b0b12)`);
</script>

<template>
  <Page
    title="Shaders"
    lead="Each effect is one Slang file in shaders/, compiled into the app for DirectX, Vulkan and Metal, and applied with a CSS custom property. Each of these uses a different part of the effect API."
  >
    <Card title="Over the whole window" api="shatter · ripples · snow · fireflies">
      <div class="row">
        <button class="btn" :disabled="fullWindow.shatter !== null" @click="shatter($event.clientX, $event.clientY)">
          Shatter
        </button>
        <button class="btn" :class="{ on: fullWindow.ripples }" @click="fullWindow.ripples = !fullWindow.ripples">
          Ripples
        </button>
        <button class="btn" :class="{ on: fullWindow.snow }" @click="fullWindow.snow = !fullWindow.snow">Snow</button>
        <button class="btn" :class="{ on: fullWindow.fireflies }" @click="fullWindow.fireflies = !fullWindow.fireflies">
          Fireflies
        </button>
      </div>
      <p class="faint">
        Layers over the window that take no clicks. Shatter breaks a snapshot of the window into falling shards and puts
        it back, the cracks and gaps letting through what is behind the window. With ripples on, a click anywhere sends
        out a ring of water that bends the window, and rings that meet interfere. Snow parts around the pointer;
        fireflies drift towards it.
      </p>
    </Card>

    <Card title="Liquid glass" api="lens · [SkBackdrop]">
      <div ref="scene" class="scene" @mousemove="lensMove" @mouseup="lensUp" @mouseleave="lensUp">
        <div class="blob b1" />
        <div class="blob b2" />
        <div class="blob b3" />
        <div class="word">refraction</div>
        <div class="lens" :style="{ left: `${lensX}px` }" @mousedown="lensDown">drag me</div>
      </div>
      <p class="faint">
        Reads the backdrop at displaced coordinates: the rim bends what is behind it, red and blue by different amounts,
        and catches a specular highlight.
      </p>
    </Card>

    <Card title="Nebula" api="nebula · [SkAnimated] · transition">
      <div class="nebula" :class="{ warm }" @click="warm = !warm">
        <span class="hint">click to shift the palette</span>
      </div>
      <p class="faint">
        Noise sampled at coordinates warped by more noise, redrawn every frame. A CSS transition on --shading blends the
        palette while it moves.
      </p>
    </Card>

    <Card title="Orbit" api="orbit · [SkBleed]">
      <div class="row orbits">
        <div class="orbit-card">card</div>
        <button class="orbit-pill">Get started</button>
        <div class="orbit-ring">ring</div>
      </div>
      <p class="faint">
        Light running along the rounded border. The halo parameter grows the drawn area, so the glow spills past the
        element's own box.
      </p>
    </Card>

    <Card title="Halftone" api="halftone · backdrop per cell">
      <div class="print">
        <img class="photo" src="assets/media/aurora.png" alt="Aurora" />
        <div class="screen" :style="{ '--shading': halftone }" />
      </div>
      <div class="row">
        <button v-for="c in cells" :key="c" class="btn" :class="{ on: cell === c }" @click="cell = c">{{ c }}px</button>
        <span class="faint">Re-screens the image under the right half as print dots.</span>
      </div>
    </Card>

    <Card title="Burn away" api="dissolve · [SkContent] · transition">
      <div class="row">
        <div class="ticket" :class="{ burnt }">
          <img class="ticket-photo" src="assets/media/aurora.png" alt="Aurora" />
          <div class="ticket-text">
            <strong>Northern lights</strong>
            <span>Admit one · row 12</span>
          </div>
        </div>
        <button class="btn" @click="burnt = !burnt">{{ burnt ? "Restore" : "Burn" }}</button>
      </div>
      <p class="faint">
        Shades the element's own painting, image and text included. A transition on --shading moves the burning edge
        across it.
      </p>
    </Card>

    <Card title="Pond" api="pond · [SkContent] · [SkHistory] · [SkPointer]">
      <div class="pond">
        <img class="pond-photo" src="assets/media/aurora.png" alt="Aurora" />
        <span class="pond-word">still water</span>
      </div>
      <p class="faint">
        Drag the pointer across. The water is a wave equation the effect steps in its own state from frame to frame, and
        it bends and lights the element's painting under it.
      </p>
    </Card>

    <Card title="Spotlight" api="spotlight · [SkPointer]">
      <div class="pad">
        <span class="hint">move the pointer here</span>
      </div>
      <p class="faint">
        The host hands the effect the pointer and repaints it as the pointer moves, so no script runs. The light dims as
        the pointer leaves the panel.
      </p>
    </Card>
  </Page>
</template>

<style scoped>
.scene {
  position: relative;
  z-index: 1;
  height: 190px;
  border-radius: 12px;
  background: #0e0e16;
  overflow: hidden;
}
@keyframes drift {
  50% {
    transform: translateX(120px);
  }
}
.blob {
  position: absolute;
  width: 120px;
  height: 120px;
  border-radius: 60px;
  animation: drift 7s ease-in-out infinite;
}
.b1 {
  left: 30px;
  top: 20px;
  background: #8b9dff;
}
.b2 {
  left: 260px;
  top: 60px;
  background: #f472b6;
  animation-duration: 9s;
}
.b3 {
  left: 480px;
  top: 10px;
  background: #34d399;
  animation-duration: 8s;
}
.word {
  position: absolute;
  left: 24px;
  top: 58px;
  font-size: 64px;
  font-weight: 800;
  color: #ffffffd9;
}
.lens {
  position: absolute;
  top: 30px;
  width: 220px;
  height: 130px;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-bottom: 12px;
  border-radius: 36px;
  font-size: 12px;
  color: #ffffffb0;
  cursor: grab;
  --shading: lens(22px, 32px, 0.2, #ffffff0d);
}

.nebula {
  height: 190px;
  display: flex;
  align-items: flex-end;
  padding: 14px;
  border-radius: 12px;
  cursor: pointer;
  --shading: nebula(0.1, 1.3, 0, #ffe6c7);
  transition: --shading 1600ms ease;
}
.nebula.warm {
  --shading: nebula(0.1, 1.3, 0.45, #fff1d6);
}

.orbits {
  gap: 40px;
  padding: 24px 28px;
}
.orbit-card {
  width: 220px;
  height: 110px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 16px;
  background: #ffffff05;
  font-size: 13px;
  color: var(--text-muted);
  --shading: orbit(0.3, 2px, 24px, 0.4, #7fd4ff);
}
.orbit-pill {
  height: 44px;
  padding: 0 26px;
  border: none;
  border-radius: 22px;
  background: #ffffff08;
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  --shading: orbit(0.55, 1.5px, 16px, 0.3, #ff8ad4);
}
.orbit-ring {
  width: 96px;
  height: 96px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 48px;
  font-size: 12px;
  color: var(--text-faint);
  --shading: orbit(0.9, 3px, 28px, 0.25, #fbbf24);
}

.print {
  position: relative;
  z-index: 1;
  height: 220px;
  border-radius: 12px;
  overflow: hidden;
}
.photo {
  width: 100%;
  height: 220px;
  object-fit: cover;
}
.screen {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 50%;
}

.ticket {
  display: flex;
  width: 360px;
  height: 120px;
  border-radius: 14px;
  background: #1b1b28;
  overflow: hidden;
  --shading: dissolve(0, 46px);
  transition: --shading 1800ms ease-in;
}
.ticket.burnt {
  --shading: dissolve(1, 46px);
  transition-timing-function: ease-out;
}
.ticket-photo {
  width: 120px;
  height: 120px;
  object-fit: cover;
}
.ticket-text {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 6px;
  padding: 0 20px;
  font-size: 13px;
  color: var(--text-muted);
}
.ticket-text strong {
  font-size: 18px;
  color: var(--text);
}

.pond {
  position: relative;
  height: 220px;
  border-radius: 12px;
  overflow: hidden;
  --shading: pond(0.985, 40px);
}
.pond-photo {
  width: 100%;
  height: 220px;
  object-fit: cover;
}
.pond-word {
  position: absolute;
  left: 24px;
  bottom: 18px;
  font-size: 44px;
  font-weight: 800;
  color: #ffffffe0;
}

.pad {
  height: 200px;
  display: flex;
  align-items: flex-end;
  padding: 14px;
  border-radius: 12px;
  background: #0e0e16;
  --shading: spotlight(150px, 1, #8b9dff);
}
.hint {
  font-size: 12px;
  color: #ffffff8c;
}
</style>
