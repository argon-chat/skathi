<script setup>
import { ref } from "vue";

const count = ref(0);
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
