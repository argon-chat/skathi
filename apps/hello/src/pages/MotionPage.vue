<script setup>
import { onActivated, onDeactivated, ref } from "vue";
import Counter from "../components/Counter.vue";

const wide = ref(false);

// Counter's setInterval is started in onMounted and cleared in onUnmounted —
// and <KeepAlive> runs NEITHER on a tab switch. So the counter keeps ticking
// while this page sits in the detached cache, exactly like in a browser. If
// you wanted it frozen you would clear the timer in onDeactivated yourself:
// that is app policy, not renderer policy.
onActivated(() => console.log("[app] MotionPage ACTIVATED"));
onDeactivated(() => console.log("[app] MotionPage DEACTIVATED (Counter keeps ticking)"));
</script>

<template>
  <div class="card">
    <div class="title">Motion — CSS transitions on class-driven changes</div>
    <div class="anim" :class="{ wide }" @click="wide = !wide">
      transition: {{ wide ? "wide" : "narrow" }} — click me
    </div>
    <Counter />
    <!-- Scoped-CSS isolation: this `.label` and Counter's `.label` are two
         different rules under two different data-v hashes. -->
    <div class="label">MotionPage-scoped .label — must be BLUE</div>
    <div class="hint keep">
      &lt;KeepAlive&gt;: the counter's setInterval is never cleared on a tab switch — leave, wait,
      come back and the number has advanced
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
.keep {
  color: #a5f3a5;
}
</style>
