<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";

const count = ref(0);
// HMR observability: this line runs once per component *instance creation*.
// A template-only edit (rerender) must NOT print it again; a script edit
// (reload) must.
console.log("[app] Counter setup ran");

// `setInterval` answers a plain number here (engine.d.ts), not a Node Timeout.
let timer: number | undefined;
onMounted(() => {
  console.log("[app] Counter mounted");
  // Self-driving state, so component state can be observed surviving an edit
  // without anyone having to click.
  timer = setInterval(() => {
    count.value++;
    console.log("[app] tick count=" + count.value);
  }, 1000);
});
onUnmounted(() => {
  clearInterval(timer);
  console.log("[app] Counter unmounted");
});
</script>

<template>
  <div class="counter">
    <div class="btn" @click="count++">count = {{ count }} — click / hover me</div>
    <div v-if="count > 0 && count % 5 === 0" class="label">divisible by 5!</div>
    <div class="label">Counter-scoped .label — must be GREEN</div>
  </div>
</template>

<style scoped>
.counter {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.btn {
  display: flex;
  padding: 10px;
  width: 220px;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  border-radius: 8px;
  font-size: 13px;
  color: #ffffff;
  cursor: pointer;
}
.btn:hover {
  background: #4f8cffcc;
  border: 1px solid #9cc2ffcc;
}
.label {
  font-size: 12px;
  color: #a5f3a5;
}
</style>
