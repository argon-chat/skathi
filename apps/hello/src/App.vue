<script setup>
import { ref } from "vue";
import Counter from "./components/Counter.vue";

const title = ref("Hello from a real .vue SFC");
</script>

<template>
  <div class="stack">
    <div class="title">
      {{ title }}
      <span class="label">·scoped·</span>
    </div>
    <Counter />
    <div class="label">App-scoped .label — must be BLUE</div>
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
.label {
  font-size: 12px;
  color: #7fd4ff;
}
</style>
