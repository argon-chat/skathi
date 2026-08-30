<script setup lang="ts">
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

    <!-- ===================================================================
         m15: the CSS surface finished. Everything in this block used to be
         an advisory line and nothing on screen.
         =================================================================== -->
    <div class="sub">m15 — gradients · shadows · outline · grid · ::after</div>
    <div class="m15">
      <!-- linear-gradient(): gpui's two-stop Background. `to right` is 90deg
           in both CSS and gpui, so the ramp runs the same way. -->
      <div class="g-row">
        <div class="g g-a">to right</div>
        <div class="g g-b">135deg</div>
        <div class="g g-c">to bottom</div>
      </div>

      <!-- box-shadow: three shadows on one box, with spread and an inset —
           the same stack Tailwind's `--tw-ring-*` builds a focus ring from.
           And `outline` + `outline-offset`, a ring OUTSIDE the border box
           that takes no layout space at all. -->
      <div class="g-row">
        <div class="sh">3 shadows + inset</div>
        <div class="ring">ring (box-shadow)</div>
        <div class="ol">outline + offset</div>
      </div>

      <!-- grid-template-columns: an ARBITRARY track list. `1fr auto` is the
           shape `grid-cols-[1fr_auto]` compiles to; the second row places its
           items with grid-column line numbers and spans. -->
      <div class="gr">
        <div class="cell">1fr</div>
        <div class="cell">auto</div>
        <div class="cell">120px</div>
        <div class="cell span2">grid-column: span 2</div>
        <div class="cell">3</div>
      </div>

      <!-- ::before / ::after: real boxes, first and last child of the
           element, laid out by the same rules as a hand-written span.
           TEXT-TRANSFORM on the label; the ::after is driven by an
           ATTRIBUTE, which is the shape reka and shadcn actually use. -->
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

/* ===== m15: gradients / shadows / outline / grid / generated content ==== */
.m15 {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.g-row {
  display: flex;
  gap: 12px;
}

/* A gradient is a real `Background`, not a dropped declaration. */
.g {
  width: 118px;
  height: 52px;
  padding: 8px;
  border-radius: 10px;
  font-size: 11px;
  color: #0b0b12;
}
.g-a { background-image: linear-gradient(to right, #7ee8fa, #eec0c6); }
.g-b { background: linear-gradient(135deg, #fbc7a4, #ffe29f); }
.g-c { background-image: linear-gradient(#a1ffce, #faffd1); }

/* Three shadows on one box, spread and inset included. */
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
/* The `--tw-ring-*` shape: a zero-blur spread shadow IS a focus ring. */
.ring {
  width: 118px;
  height: 52px;
  padding: 8px;
  border-radius: 10px;
  font-size: 11px;
  color: #ffffff;
  background: #1d2333;
  box-shadow: 0 0 0 2px #0b0b12, 0 0 0 5px #38bdf8;
}
/* …and a real `outline`, which takes no layout space — the two boxes above
   and this one are the same size and sit on the same baseline. */
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

/* An arbitrary track list, plus line-number placement. */
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

/* ::before and ::after are children, first and last. `content` gates them:
   no content, no box — exactly as in CSS. */
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
/* Driven by an ATTRIBUTE on the originating element — the shape reka-ui and
   shadcn use for every open/closed indicator. */
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
