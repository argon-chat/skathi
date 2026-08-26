<script setup>
// Everything visual that is *not* layout/state lives here now: real Tailwind
// utilities, GPU backdrop blur, and the `--shading` effect system. Moved off
// the Lab page in milestone 10 so no single tab outgrows the window again.
import { ref } from "vue";

// `--shading` params are floats, so `transition: --shading 420ms` interpolates
// them like any other animatable value. State-rule transitions (:hover) are
// rejected by design, so this rides a click-toggled class.
const hot = ref(false);
</script>

<template>
  <div class="card">
    <div class="title">Styles — Tailwind · backdrop-filter · --shading</div>

    <!--
      Pure Tailwind: not one of these classes exists in this file's <style>.
      They come from the real Tailwind JIT (v3.4) through the synthetic
      ".vn/tailwind.css" module. `hover:` rides vn-css's state ladder;
      `transition-colors` + `duration-300` animate the background swap.
    -->
    <div
      class="flex flex-col gap-2 p-4 rounded-xl bg-white/10 hover:bg-white/25 transition-colors duration-300 border-2 border-emerald-400/70"
    >
      <div class="text-sm font-semibold text-white">Tailwind card</div>
      <div class="text-xs text-white/70">flex flex-col gap-2 p-4 rounded-xl bg-white/10 — hover me</div>
      <div class="text-xs font-mono text-sky-300">hover:bg-white/25 · transition-colors duration-300</div>
    </div>

    <!-- ===================================================================
         backdrop-filter: blur() — GPU backdrop blur (gpui BackdropBlurRect)
         Two identical translucent panels sit over the same colour stripes.
         The LEFT one declares `backdrop-filter: blur(16px)`, the RIGHT one
         does not: the stripe edges must be smeared under the left panel and
         razor-sharp under the right one.
         =================================================================== -->
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

    <!-- ===================================================================
         `--shading`, the custom effect system. Sources:
         crates/vn-effects/shaders/*.slang, compiled by `bun shaders.ts` to
         checked-in HLSL and run at ps_5_0 on a gpui EffectQuad primitive.

           frost  reads the render-target snapshot (same machinery as
                  backdrop-filter) and adds grain + a rim highlight.
           noise  animated film grain OVER the element.
           glow   a halo centred on the rounded-rect edge, reaching OUTSIDE
                  the border box — the primitive's quad is dilated for it.

         A browser parses `--shading`, stores it and renders nothing, so this
         whole block degrades to plain cards there rather than erroring.
         =================================================================== -->
    <div class="fx">
      <div class="fx-title">--shading — frost · noise · glow</div>

      <!-- Same colour stripes as above: frost has to smear the edges under the
           left panel and leave them razor-sharp under the right one, PLUS
           carry its own grain and rim. -->
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

        <!-- `transition: --shading` over the param vector: radius, strength
             and every colour channel sweep together. -->
        <div class="fx-card fx-glow" :class="{ 'fx-hot': hot }" @click="hot = !hot">
          <div class="fx-card-name">glow — click me</div>
          <div class="fx-card-note">{{ hot ? "#ff2d95 · 34px · 1" : "#7fd4ff · 10px · 0.35" }}</div>
        </div>
      </div>
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

/* ===== backdrop-filter: blur() demo ==================================== */
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
.bf-s1 { background: #ff3b30; }
.bf-s2 { background: #ffcc00; }
.bf-s3 { background: #34c759; }
.bf-s4 { background: #00c7ff; }
.bf-s5 { background: #5e5ce6; }
.bf-s6 { background: #ff2d95; }

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
/* The one under test. */
.bf-blur {
  left: 18px;
  backdrop-filter: blur(16px);
}
/* Identical panel, no filter — the control. */
.bf-plain {
  left: 202px;
}
</style>

<!-- ===================================================================
     `--shading` styles in their own <style scoped> block; the SFC pipeline
     concatenates every block under the same `data-v-` hash.
     =================================================================== -->
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

/* --- frost over the colour stripes ------------------------------------- */
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
.fx-s1 { background: #ff3b30; }
.fx-s2 { background: #ffcc00; }
.fx-s3 { background: #34c759; }
.fx-s4 { background: #00c7ff; }
.fx-s5 { background: #5e5ce6; }
.fx-s6 { background: #ff2d95; }

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
/* The one under test: `strength` fades the whole treatment, `radius` is the
   backdrop blur the renderer builds the Dual-Kawase pyramid for, and the
   third argument tints it. */
.fx-frost {
  left: 18px;
  --shading: frost(0.85, 14px, #cfe9ff1f);
}
/* Identical panel, no effect — the control. */
.fx-plain {
  left: 202px;
}

/* --- noise + glow cards ------------------------------------------------ */
.fx-row {
  display: flex;
  flex-direction: row;
  gap: 44px;
  /* The glow's halo reaches `radius` OUTSIDE the card, so leave it room. */
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

/* Film grain painted over the card's own background. `speed` > 0 makes the
   effect animated, which is what keeps the window repainting. */
.fx-noise {
  background: #2b3350;
  border: 1px solid #ffffff2e;
  --shading: noise(0.4, 2px, 18);
}

/* `transition: --shading` interpolates the param vector — radius, strength
   and all four colour channels sweep together. Driven by a click-toggled
   class, because state-rule transitions are rejected by design. */
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
