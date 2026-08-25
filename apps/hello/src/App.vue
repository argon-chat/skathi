<script setup>
import { ref, computed } from "vue";
import Counter from "./components/Counter.vue";

const title = ref("Hello from a real .vue SFC");
const name = ref("");
const wheelY = ref(0);

// --- scroll container ------------------------------------------------------
const rows = Array.from({ length: 15 }, (_, i) => `row ${i + 1}`);
const scrollTop = ref(0);
// Straight off the event target, exactly like the browser.
const onScroll = (e) => (scrollTop.value = e.target.scrollTop);

// --- keydown forwarding ----------------------------------------------------
// The native <input> hands every keystroke to JS *before* it edits, so
// @keydown.enter fires (and could preventDefault) like in a browser.
const submitted = ref([]);
const submit = () => {
  const v = name.value.trim();
  if (!v) return;
  submitted.value = [...submitted.value.slice(-2), v];
  name.value = "";
};
const ellipsis = (s) => (s.length > 14 ? s.slice(0, 14) + "…" : s);
const submittedText = computed(() => submitted.value.map(ellipsis).join(" · ") || "—");

// preventDefault() on keydown must cancel the widget's own editing, exactly as
// it cancels the browser's — the classic "this field takes no digits" guard.
const blocked = ref(0);
const lastKey = ref("—");
const guardDigits = (e) => {
  lastKey.value = JSON.stringify(e.key);
  if (e.key.length === 1 && e.key >= "0" && e.key <= "9") {
    e.preventDefault();
    blocked.value++;
  }
};

// --- focus traversal -------------------------------------------------------
// focus/blur fire for mouse, Tab and programmatic focus alike.
const focused = ref("nothing");

// --- CSS transitions -------------------------------------------------------
// Plain browser CSS: `transition:` on the base rule animates class-driven
// changes (width + background here). Toggled by click.
const wide = ref(false);

// --- round 5 (appended block) ----------------------------------------------
// These drive ELEMENT PROPERTIES, not classes: `:disabled`, `:checked` and
// `[data-state="…"]` are answered by the cascade straight off the property
// map, so flipping a ref restyles without a single state class.
const locked = ref(false);
const checked = ref(false);
const open = ref(false);

// --- shaders (appended block) ----------------------------------------------
// `--shading` params are floats, so `transition: --shading 420ms` interpolates
// them like any other animatable value. State-rule transitions (:hover) are
// rejected by design, so this rides a click-toggled class exactly like `.anim`.
const hot = ref(false);
</script>

<template>
  <div class="stack" @wheel="wheelY = $event.deltaY">
    <div class="title">
      {{ title }}
      <span class="label">·scoped·</span>
    </div>

    <!-- 100% browser-standard markup: vModelText from the stock compiler -->
    <input
      class="field"
      v-model="name"
      tabindex="1"
      placeholder="type here, press Enter"
      @keydown="guardDigits"
      @keydown.enter="submit"
      @focus="focused = 'input'"
      @blur="focused = 'nothing'"
    />
    <div class="hint">v-model echo: "{{ name }}" ({{ name.length }} chars)</div>
    <div class="hint">Enter-submitted: {{ submittedText }}</div>
    <div class="hint">last e.key: {{ lastKey }} — digits blocked by preventDefault: {{ blocked }}</div>

    <!-- overflow: scroll + a @scroll handler reading e.target.scrollTop -->
    <div
      class="scrollbox"
      tabindex="2"
      @scroll="onScroll"
      @focus="focused = 'scroll box'"
      @blur="focused = 'nothing'"
    >
      <div v-for="row in rows" :key="row" class="row">{{ row }}</div>
    </div>
    <div class="hint">scrollTop: {{ scrollTop.toFixed(0) }}px (wheel over the box)</div>
    <div class="hint">Tab / Shift-Tab focus → {{ focused }}</div>

    <div class="anim" :class="{ wide }" @click="wide = !wide">
      transition: {{ wide ? "wide" : "narrow" }} — click me
    </div>

    <Counter />

    <!--
      Pure Tailwind: not one of these classes exists in this file's <style>.
      They come from the real Tailwind JIT (v3.4) through the synthetic
      ".vn/tailwind.css" module. `hover:` rides vn-css's state ladder;
      `transition-colors` + `duration-300` animate the background swap.
    -->
    <div class="flex flex-col gap-2 p-4 rounded-xl bg-white/10 hover:bg-white/25 transition-colors duration-300 border-2 border-emerald-400/70">
      <div class="text-sm font-semibold text-white">Tailwind card</div>
      <div class="text-xs text-white/70">
        flex flex-col gap-2 p-4 rounded-xl bg-white/10 — hover me
      </div>
      <div class="text-xs font-mono text-sky-300">hover:bg-white/25 · transition-colors duration-300</div>
    </div>

    <div class="label">App-scoped .label — must be BLUE</div>
    <div class="hint">last wheel ΔY over the card: {{ wheelY.toFixed(1) }}</div>

    <!-- ===================================================================
         backdrop-filter: blur() — GPU backdrop blur (gpui BackdropBlurRect)
         Two identical translucent panels sit over the same colour stripes.
         The LEFT one declares `backdrop-filter: blur(16px)`, the RIGHT one
         does not: the stripe edges must be smeared under the left panel and
         razor-sharp under the right one.
         =================================================================== -->
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
    <!-- =============== end backdrop-filter demo ========================= -->

    <!-- ===================================================================
         ROUND 5 (appended block) — everything below is answered by the CSS
         cascade alone. No JS reads a focus state, no handler adds a state
         class: `:focus` / `:focus-visible` ride gpui's focus rungs,
         `:disabled` / `:checked` / `[data-state]` read the element's own
         properties, and the `@media` blocks are evaluated against the live
         window (size, OS theme, reduced-motion).
         =================================================================== -->
    <div class="r5">
      <div class="r5-title">round 5 — :focus · props · [attr] · @media</div>

      <!-- :focus on the native <input>. -->
      <input class="field r5-focus" tabindex="3" placeholder="Tab or click me — :focus ring" />

      <!-- :focus on a plain tabbable div rides gpui's .focus() rung;
           :focus-visible only lights up when focus arrived from the KEYBOARD,
           exactly as in a browser. -->
      <div class="r5-box" tabindex="4">
        plain div, tabindex=4 — blue on :focus, amber on :focus-visible (Tab)
      </div>

      <!-- :disabled reads the property, so v-bind alone restyles. -->
      <div class="r5-row">
        <input class="field r5-input" :disabled="locked" :value="locked ? 'locked' : 'editable'" />
        <div class="r5-toggle" @click="locked = !locked">{{ locked ? "unlock" : "lock" }} — :disabled</div>
      </div>

      <div class="r5-check" :checked="checked" @click="checked = !checked">:checked = {{ checked }} — click me</div>

      <!-- [attr="…"] — the exact shape Tailwind's `data-[state=open]:` emits. -->
      <div class="r5-state" :data-state="open ? 'open' : 'closed'" @click="open = !open">
        [data-state="{{ open ? "open" : "closed" }}"] — click me
      </div>

      <!-- @media, hand written: resize the window across 640px and 900px. -->
      <div class="r5-media">@media width: GREY &lt; 640px · BLUE ≥ 640px · GREEN ≥ 900px</div>
      <div class="r5-dark">@media (prefers-color-scheme: dark) — AMBER while the OS theme is dark</div>
      <div class="r5-motion">@media (prefers-reduced-motion: reduce) — RED when the OS asks for less motion</div>

      <!-- …and the same four capabilities through Tailwind's own variants. -->
      <div class="flex flex-col gap-2 p-3 rounded-lg bg-white/5 border border-white/20 text-xs text-white/60 sm:text-sky-300 lg:text-emerald-300 dark:bg-amber-400/10">
        <div class="font-mono">tailwind: sm:text-sky-300 · lg:text-emerald-300 · dark:bg-amber-400/10</div>
        <input class="field focus:border-sky-400 disabled:opacity-40" tabindex="5" :disabled="locked" placeholder="tailwind focus: + disabled:" />
      </div>
    </div>
    <!-- =============== end round 5 demo ================================== -->

    <!-- ===================================================================
         SHADERS (appended block) — `--shading`, the custom effect system.
         Sources: crates/vn-effects/shaders/*.slang, compiled by
         `bun shaders.ts` to checked-in HLSL and run at ps_5_0 on a
         gpui EffectQuad primitive.

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

      <!-- Same colour stripes as the backdrop-filter demo: frost has to smear
           the edges under the left panel and leave them razor-sharp under the
           right one, PLUS carry its own grain and rim. -->
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
    <!-- =============== end shaders demo ================================== -->
  </div>
</template>

<style scoped>
.stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 15px;
  color: #ffffff;
}
.field {
  height: 32px;
  width: 260px;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  font-size: 13px;
  color: #ffffff;
}
/* vn-css maps the `overflow` shorthand and the overflow-x / overflow-y
   longhands (Tailwind's `overflow-y-auto` and friends). */
.scrollbox {
  overflow: scroll;
  height: 90px;
  width: 260px;
  display: flex;
  flex-direction: column;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
}
.row {
  padding: 4px;
  font-size: 12px;
  color: #ffffffcc;
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
.label {
  font-size: 12px;
  color: #7fd4ff;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}

/* ===== backdrop-filter: blur() demo ================================== */
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
     ROUND 5 (appended block) — a second <style scoped> block; the SFC
     pipeline concatenates every style block under the same `data-v-` hash.
     =================================================================== -->
<style scoped>
.r5 {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: #ffffff0f;
  border: 1px solid #ffffff1f;
}
.r5-title {
  font-size: 12px;
  color: #ffffffcc;
}

/* :focus — on an <input> the rung is folded from the editor's own focus
   handle (tracking it on the wrapper too would double the tab stop). */
.r5-focus {
  width: 300px;
}
.r5-focus:focus {
  border-color: #7fd4ff;
  background: #7fd4ff26;
}

/* The round-1/2 controls get a ring from the same one-line rule — the
   `<input>` through the folded path, the scroll box through gpui's rung. */
.field:focus {
  border-color: #7fd4ff;
}
.scrollbox:focus {
  border-color: #7fd4ff;
  background: #7fd4ff1f;
}
.scrollbox:focus-visible {
  border-color: #ffcc00;
}

/* :focus / :focus-visible on a plain tabbable div — gpui's .focus() and
   .focus_visible() rungs. The latter needs the focus to have come from the
   keyboard, which is exactly what the browser means by :focus-visible. */
.r5-box {
  width: 300px;
  padding: 6px;
  font-size: 11px;
  color: #ffffffaa;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
}
.r5-box:focus {
  border-color: #7fd4ff;
  color: #ffffff;
}
.r5-box:focus-visible {
  border-color: #ffcc00;
  color: #ffcc00;
}

/* :disabled / :checked are ELEMENT PROPERTIES, so they fold into the base
   bucket — there is no gpui rung for them and none is needed. */
.r5-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.r5-input {
  width: 150px;
}
.r5-input:disabled {
  opacity: 0.35;
  border-color: #ff3b3080;
}
.r5-toggle {
  padding: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border-radius: 6px;
  cursor: pointer;
}
.r5-check {
  padding: 6px;
  font-size: 11px;
  color: #ffffff88;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  cursor: pointer;
}
.r5-check:checked {
  color: #34c759;
  border-color: #34c759;
  background: #34c75926;
}

/* [attr="value"] straight off the property map. */
.r5-state {
  padding: 6px;
  font-size: 11px;
  color: #ffffff88;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  cursor: pointer;
}
.r5-state[data-state="open"] {
  color: #ff2d95;
  border-color: #ff2d95;
}

/* @media against the live window environment. */
.r5-media,
.r5-dark,
.r5-motion {
  font-size: 11px;
  color: #ffffff66;
}
@media (min-width: 640px) {
  .r5-media {
    color: #7fd4ff;
  }
}
@media (min-width: 900px) {
  .r5-media {
    color: #34c759;
  }
}
@media (prefers-color-scheme: dark) {
  .r5-dark {
    color: #ffcc00;
  }
}
@media (prefers-reduced-motion: reduce) {
  .r5-motion {
    color: #ff3b30;
  }
}
</style>

<!-- ===================================================================
     SHADERS (appended block) — a third <style scoped> block. The SFC
     pipeline concatenates them, so appending never disturbs what is
     above.
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
