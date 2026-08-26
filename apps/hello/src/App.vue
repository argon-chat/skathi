<script setup>
import { onActivated, onMounted, onUnmounted, ref } from "vue";
import { useSession } from "./stores/session.js";

// `name` is what <KeepAlive>'s include/exclude matches on — the shell excludes
// exactly this page, so it is the one tab that really unmounts and remounts on
// every visit. Without it the compiler would infer "App" from the filename.
defineOptions({ name: "LabPage" });

// --- KeepAlive is OFF here --------------------------------------------------
// Local state therefore resets on every visit…
const localVisits = ref(0);
const scratch = ref("");
console.log("[app] LabPage setup ran (NOT kept alive — this prints every visit)");
onMounted(() => {
  localVisits.value++;
  console.log("[app] LabPage mounted (local state is brand new)");
});
onUnmounted(() => console.log("[app] LabPage unmounted (left the tab)"));
// …and onActivated never fires for a page outside the cache.
onActivated(() => console.log("[app] LabPage ACTIVATED — should NEVER appear"));

// …while the pinia store is untouched by any of that: it lives in the app's
// effect scope, not in this component's.
const session = useSession();
const mountId = Math.random().toString(36).slice(2, 7);

// --- round 5 ----------------------------------------------------------------
// These drive ELEMENT PROPERTIES, not classes: `:disabled`, `:checked` and
// `[data-state="…"]` are answered by the cascade straight off the property
// map, so flipping a ref restyles without a single state class.
const locked = ref(false);
const checked = ref(false);
const open = ref(false);
</script>

<template>
  <div class="stack">
    <div class="title">
      Lab — the one tab OUTSIDE &lt;KeepAlive&gt;
      <span class="label">·scoped·</span>
    </div>

    <!-- ===================================================================
         KeepAlive vs pinia: the shell passes :exclude="['LabPage']", so this
         component is torn down and rebuilt on every visit. Local refs reset;
         the store does not.
         =================================================================== -->
    <div class="ka">
      <div class="ka-title">local component state vs pinia store state</div>
      <div class="hint">
        local: mount id <span class="bad">{{ mountId }}</span> — a fresh random per instance, so it
        changes on every visit ({{ localVisits }} mount on this one)
      </div>
      <input class="field" v-model="scratch" tabindex="6" placeholder="type — this is NOT kept" />
      <div class="hint">local scratch: "{{ scratch }}" (empty again on every visit)</div>
      <div class="hint good">
        pinia session.badge: {{ session.badge }} · notes {{ session.noteCount }} — survives anyway
      </div>
      <div class="ka-row">
        <div class="btn" @click="session.bump()">visits++ from Lab</div>
      </div>
    </div>

    <!-- ===================================================================
         ROUND 5 — everything below is answered by the CSS cascade alone.
         No JS reads a focus state, no handler adds a state class:
         `:focus` / `:focus-visible` ride gpui's focus rungs, `:disabled` /
         `:checked` / `[data-state]` read the element's own properties, and the
         `@media` blocks are evaluated against the live window (size, OS theme,
         reduced-motion).
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
.label {
  font-size: 12px;
  color: #7fd4ff;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
.good {
  color: #ffd479;
}
.bad {
  color: #ff9aa2;
}

/* ===== KeepAlive-vs-pinia block ======================================== */
.ka {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: #ffffff0f;
  border: 1px solid #ffffff1f;
}
.ka-title {
  font-size: 12px;
  color: #ffffffcc;
}
.ka-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.btn:hover {
  background: #4f8cffcc;
}
</style>

<!-- ===================================================================
     ROUND 5 — a second <style scoped> block; the SFC pipeline concatenates
     every style block under the same `data-v-` hash.
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

/* The other fields get a ring from the same one-line rule. */
.field:focus {
  border-color: #7fd4ff;
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
