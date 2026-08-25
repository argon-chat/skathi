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
