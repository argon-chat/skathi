<script setup lang="ts">
import { ref } from "vue";

// Class-driven on purpose: state-rule transitions are rejected by vn-css by design, so a real
// class flip is the supported way to animate an effect's parameters.
const hot = ref(false);
</script>

<template>
  <div class="lab">
    <div class="head">
      <div class="h1">shaderlab — user-authored `--shading` effects</div>
      <div class="h2 app">
        app effects — ripple · aurora: compiled from apps/shaderlab/shaders/*.slang by `bun shaders.ts --app … --out …`,
        loaded at startup from $VN_EFFECTS_DIR.
      </div>
      <div class="h2 core">
        built-ins — frost · glow: the effects that ship with the engine, on the same registry and unchanged.
      </div>
    </div>

    <!-- Animated: `speed` carries [VnAnimated], so the host keeps repainting while it is above zero. -->
    <div class="row">
      <div class="panel ripple">
        <div class="name">ripple(1.6, 26px, 0.5, 0.42, #7fd4ff)</div>
        <div class="note">animated · float2 centre · colour</div>
      </div>
      <div class="panel ripple ripple-slow">
        <div class="name">ripple(0.5, 54px, 0.15, 0.8, #ff8ad4)</div>
        <div class="note">same shader, different arguments</div>
      </div>
    </div>

    <!-- Every parameter is a float, colour channels included, so the whole look interpolates
         component-wise on the class flip. -->
    <div class="row">
      <div class="panel aurora" :class="{ hot }" @click="hot = !hot">
        <div class="name">aurora — click to transition</div>
        <div class="note">
          {{ hot ? "aurora(1, 0.62, #ff9d4d)" : "aurora(0.55, 0.12, #7f5cff)" }}
        </div>
      </div>
      <div class="panel aurora-back">
        <div class="name">aurora over a light card</div>
        <div class="note">the sheen composes with any background</div>
      </div>
    </div>

    <!-- The BUILT-INS, in the same app, from the same registry — frost reads the render-target
         snapshot (so it needs the colour stripes behind it) and glow dilates its own quad. -->
    <div class="builtins">
      <div class="stripes">
        <div class="s s1"></div>
        <div class="s s2"></div>
        <div class="s s3"></div>
        <div class="s s4"></div>
        <div class="s s5"></div>
        <div class="s s6"></div>
      </div>
      <div class="over frost">frost(0.85, 14px)</div>
      <div class="over plain">no --shading</div>
    </div>

    <div class="row">
      <div class="panel glow">
        <div class="name">glow(#7fd4ff, 22px, 0.85)</div>
        <div class="note">built-in · halo outside the border box</div>
      </div>
      <div class="panel none">
        <div class="name">no --shading</div>
        <div class="note">the control</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.lab {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 4px;
}
.head {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.h1 {
  font-size: 15px;
  color: #ffffff;
}
.h2 {
  font-size: 11px;
  color: #ffffff99;
}
.h2.app {
  color: #7fd4ffcc;
}
.h2.core {
  color: #ffcc66aa;
}

.row {
  display: flex;
  flex-direction: row;
  gap: 36px;
  padding: 14px 20px;
}
.panel {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 210px;
  height: 96px;
  padding: 12px;
  border-radius: 14px;
  font-size: 12px;
  color: #ffffff;
}
.name {
  font-weight: 600;
}
.note {
  font-size: 11px;
  color: #ffffff8c;
}

.ripple {
  background: #10182b;
  border: 1px solid #ffffff26;
  --shading: ripple(1.6, 26px, 0.5, 0.42, #7fd4ff);
}
.ripple-slow {
  background: #1b1020;
  --shading: ripple(0.5, 54px, 0.15, 0.8, #ff8ad4);
}

.aurora {
  background: #0d1020;
  border: 1px solid #ffffff26;
  --shading: aurora(0.55, 0.12, #7f5cff);
  transition: --shading 1400ms ease;
}
.aurora.hot {
  --shading: aurora(1, 0.62, #ff9d4d);
}
.aurora-back {
  background: #e8ecf7;
  color: #101426;
  border: 1px solid #00000026;
  --shading: aurora(0.7, -0.3, #1f8fff);
}
.aurora-back .note {
  color: #10142699;
}

.builtins {
  position: relative;
  height: 132px;
  margin: 0 20px;
}
.stripes {
  display: flex;
  flex-direction: row;
  height: 124px;
  border-radius: 14px;
  overflow: hidden;
}
.s {
  width: 78px;
  height: 124px;
}
.s1 {
  background: #ff3b30;
}
.s2 {
  background: #ffcc00;
}
.s3 {
  background: #34c759;
}
.s4 {
  background: #00c7ff;
}
.s5 {
  background: #5e5ce6;
}
.s6 {
  background: #ff2d95;
}

.over {
  position: absolute;
  top: 26px;
  width: 190px;
  height: 72px;
  padding: 10px;
  background: #ffffff1a;
  border: 1px solid #ffffff55;
  border-radius: 14px;
  font-size: 12px;
  color: #ffffff;
}
.frost {
  left: 24px;
  --shading: frost(0.85, 14px, #cfe9ff1f);
}
.plain {
  left: 244px;
}

.glow {
  background: #141a2c;
  border: 1px solid #ffffff26;
  --shading: glow(#7fd4ff, 22px, 0.85);
}
.none {
  background: #141a2c;
  border: 1px solid #ffffff26;
}
</style>
