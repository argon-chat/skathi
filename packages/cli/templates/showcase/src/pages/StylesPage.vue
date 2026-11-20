<script setup lang="ts">
import { ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const hot = ref(false);
const bandWidth = ref(180);
const warm = ref(false);

const shapes = ["squircle", "bevel", "scoop", "notch", "superellipse"];

const hues = Array.from({ length: 12 }, (_, i) => i * 30);
const mix = ref(50);

const schemes = ["light", "dark", "light dark"];
const scheme = ref("light");
const notify = ref(true);
const level = ref(60);

const hue = ref(265);

const cursors = [
  "pointer",
  "text",
  "move",
  "help",
  "wait",
  "progress",
  "crosshair",
  "not-allowed",
  "ew-resize",
  "ns-resize",
  "nwse-resize",
  "col-resize",
  "all-scroll",
  "none",
];
</script>

<template>
  <Page
    title="Styles"
    lead="Ordinary CSS, mapped onto the GPU renderer. Install tailwindcss@3, add a tailwind.config.ts next to sk.config.json, and Tailwind classes work too."
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

    <Card title="Inline SVG" api="var() · stop-color · rect">
      <svg
        class="band-preview"
        width="240"
        height="30"
        viewBox="0 0 240 30"
        role="img"
        aria-label="Gradient width and palette preview"
        :style="{ '--band-width': bandWidth + 'px', '--end-color': warm ? '#f0b45a' : '#5ad1a8' }"
      >
        <defs>
          <linearGradient id="showcase-band">
            <stop class="band-start" offset="0" />
            <stop class="band-end" offset="1" />
          </linearGradient>
        </defs>
        <rect class="band" fill="url(#showcase-band)" />
      </svg>
      <div class="row">
        <label class="row muted">
          Width <input v-model.number="bandWidth" type="range" min="24" max="240" step="12" aria-label="SVG width" />
          {{ bandWidth }} px
        </label>
        <label class="row muted"><input v-model="warm" type="checkbox" aria-label="Warm end color" /> Warm end color</label>
      </div>
    </Card>

    <Card title="Corner shapes" api="corner-shape">
      <div class="row gap">
        <div v-for="shape in shapes" :key="shape" class="swatch shaped" :class="shape">
          {{ shape === "superellipse" ? "superellipse(3)" : shape }}
        </div>
      </div>
      <p class="faint">Squircles, bevels and notches from one border-radius. Hover one: shapes interpolate.</p>
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
        <div class="swatch sheen">sheen</div>
      </div>
      <p class="faint">
        Skathi's own effects, in a custom property that a browser simply ignores. sheen is this app's, from
        shaders/sheen.slang.
      </p>
    </Card>

    <Card title="Text shadow" api="text-shadow">
      <div class="row type-row">
        <span class="type neon">Neon</span>
        <span class="type offset">Offset</span>
        <span class="type soft">Soft</span>
        <span class="type outline">Outline</span>
      </div>
      <p class="faint">Several shadows with blur, and they animate: hover the neon sign.</p>
    </Card>

    <Card title="Modern colour" api="oklch() · color-mix()">
      <div class="ramp-row">
        <span class="ramp-label mono">hsl()</span>
        <div class="ramp">
          <span v-for="h in hues" :key="h" class="step" :style="{ background: `hsl(${h} 85% 60%)` }" />
        </div>
      </div>
      <div class="ramp-row">
        <span class="ramp-label mono">oklch()</span>
        <div class="ramp">
          <span v-for="h in hues" :key="h" class="step" :style="{ background: `oklch(0.72 0.14 ${h})` }" />
        </div>
      </div>
      <p class="faint">The same hues at one lightness: in oklch() they really are one lightness.</p>
      <label class="field">
        <span class="label">Blue {{ mix }}% · yellow {{ 100 - mix }}%</span>
        <input v-model.number="mix" type="range" min="0" max="100" />
      </label>
      <div class="ramp-row">
        <span class="ramp-label mono">in srgb</span>
        <div class="mixed" :style="{ background: `color-mix(in srgb, #3b82f6 ${mix}%, #facc15)` }" />
      </div>
      <div class="ramp-row">
        <span class="ramp-label mono">in oklch</span>
        <div class="mixed" :style="{ background: `color-mix(in oklch, #3b82f6 ${mix}%, #facc15)` }" />
      </div>
      <div class="row">
        <span class="chip" style="background: lab(62% 52 -38)">lab()</span>
        <span class="chip" style="background: lch(70% 60 150)">lch()</span>
        <span class="chip" style="background: hwb(20 10% 5%)">hwb()</span>
        <span class="chip" style="background: color(display-p3 0.2 0.6 1)">color(display-p3)</span>
        <span class="chip" style="background: rgb(from #8b9dff r g b / 45%)">rgb(from …)</span>
      </div>
    </Card>

    <Card title="Light and dark" api="light-dark() · color-scheme">
      <div class="row">
        <button v-for="s in schemes" :key="s" class="btn mono" :class="{ on: scheme === s }" @click="scheme = s">
          {{ s }}
        </button>
      </div>
      <div class="scheme" :style="{ colorScheme: scheme }">
        <div class="scheme-title">Notifications</div>
        <p class="scheme-text">Colours from light-dark(), resolved per element from color-scheme.</p>
        <label class="scheme-line"><input v-model="notify" type="checkbox" /> Show previews</label>
        <label class="scheme-line">
          Volume <input v-model.number="level" type="range" min="0" max="100" />
        </label>
      </div>
      <p class="faint">
        The checkbox and slider follow the scheme too. "light dark" follows the window's theme, live.
      </p>
    </Card>

    <Card title="Custom functions" api="@function">
      <div class="palette" :style="{ '--hue': hue }">
        <span class="tone t1" />
        <span class="tone t2" />
        <span class="tone t3" />
        <span class="tone base">base</span>
        <span class="tone t4" />
        <span class="tone t5" />
        <span class="tone t6" />
      </div>
      <label class="field">
        <span class="label">Base hue · {{ hue }}</span>
        <input v-model.number="hue" type="range" min="0" max="359" />
      </label>
      <p class="faint mono">--tint(var(--base), 25%), --shade(var(--base)): one colour in, a palette out.</p>
    </Card>

    <Card title="Cursors" api="cursor">
      <div class="cursors">
        <span v-for="c in cursors" :key="c" class="cursor mono" :style="{ cursor: c }">{{ c }}</span>
      </div>
      <p class="faint">The system's own cursors. Hover each one.</p>
    </Card>

    <Card title="Grid and selectors" api="grid · ::before · :hover">
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
.band-preview {
  background: var(--surface-strong);
  border-radius: 6px;
}
.band {
  width: var(--band-width);
  height: 30px;
  rx: 6px;
}
.band-start {
  stop-color: var(--accent);
}
.band-end {
  stop-color: var(--end-color);
}
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
.shaped {
  border-radius: 24px;
  background: linear-gradient(135deg, #6d7cff, #b48cff);
  font-family: var(--mono);
  font-weight: 400;
  transition: corner-shape 400ms ease;
}
.squircle {
  corner-shape: squircle;
}
.bevel {
  corner-shape: bevel;
}
.scoop {
  corner-shape: scoop;
}
.notch {
  corner-shape: notch;
}
.superellipse {
  corner-shape: superellipse(3);
}
.shaped:hover {
  corner-shape: round;
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
.sheen {
  background: linear-gradient(135deg, #3b3f9e, #8b5cf6);
  --shading: sheen(0.5, #ffffffcc);
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
.type-row {
  gap: 28px;
  padding: 8px 4px;
}
.type {
  font-size: 30px;
  font-weight: 800;
}
.neon {
  color: #fff0fa;
  text-shadow:
    0 0 4px #ffffff,
    0 0 12px #ff2d95,
    0 0 24px #ff2d95;
  cursor: pointer;
  transition: text-shadow 300ms ease;
}
.neon:hover {
  text-shadow:
    0 0 6px #ffffff,
    0 0 18px #ff2d95,
    0 0 32px #ff2d95;
}
.offset {
  color: #ffffff;
  text-shadow: 3px 3px 0 var(--accent-strong);
}
.soft {
  color: #e0e7ff;
  text-shadow: 0 6px 16px #000000;
}
.outline {
  color: #0b0b10;
  text-shadow:
    1px 1px 0 var(--accent),
    -1px 1px 0 var(--accent),
    1px -1px 0 var(--accent),
    -1px -1px 0 var(--accent);
}
.ramp-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.ramp-label {
  width: 64px;
  color: var(--text-faint);
}
.ramp {
  flex: 1;
  display: flex;
  gap: 4px;
}
.step {
  flex: 1;
  height: 26px;
  border-radius: 6px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-width: 420px;
}
.label {
  font-size: 12px;
  color: var(--text-faint);
}
.mixed {
  flex: 1;
  height: 26px;
  border-radius: 6px;
}
.chip {
  padding: 6px 10px;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 600;
  color: #ffffff;
}
.scheme {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-width: 420px;
  padding: 16px;
  border-radius: 12px;
  border: 1px solid light-dark(#d4d4dc, #ffffff1f);
  background: light-dark(#f6f6f9, #17171f);
  color: light-dark(#18181b, #ececf1);
  accent-color: light-dark(#4f46e5, #8b9dff);
  transition:
    background-color 200ms ease,
    color 200ms ease,
    border-color 200ms ease;
}
.scheme-title {
  font-size: 15px;
  font-weight: 700;
}
.scheme-text {
  margin: 0;
  font-size: 13px;
  color: light-dark(#52525b, #a1a1aa);
}
.scheme-line {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
}
@function --tint(--c <color>, --amount <percentage>: 50%) returns <color> {
  result: color-mix(in oklch, var(--c), white var(--amount));
}
@function --shade(--c <color>, --amount <percentage>: 50%) returns <color> {
  result: color-mix(in oklch, var(--c), black var(--amount));
}
.palette {
  --base: oklch(0.62 0.17 var(--hue));
  display: flex;
  gap: 6px;
}
.tone {
  flex: 1;
  height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 600;
  color: #ffffff;
}
.t1 {
  background: --tint(var(--base), 75%);
}
.t2 {
  background: --tint(var(--base), 50%);
}
.t3 {
  background: --tint(var(--base), 25%);
}
.base {
  background: var(--base);
}
.t4 {
  background: --shade(var(--base), 25%);
}
.t5 {
  background: --shade(var(--base));
}
.t6 {
  background: --shade(var(--base), 70%);
}
.cursors {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 6px;
}
.cursor {
  padding: 10px 6px;
  border-radius: 8px;
  background: var(--surface-hover);
  color: var(--text-muted);
  text-align: center;
}
.cursor:hover {
  background: var(--surface-strong);
  color: var(--text);
}
</style>
