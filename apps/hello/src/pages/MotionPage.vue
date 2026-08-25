<script setup>
import { ref } from "vue";
import Counter from "../components/Counter.vue";

const wide = ref(false);
</script>

<template>
  <div class="card">
    <div class="title">Motion — CSS transitions on class-driven changes</div>
    <div class="anim" :class="{ wide }" @click="wide = !wide">
      transition: {{ wide ? "wide" : "narrow" }} — click me
    </div>
    <Counter />
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
</style>
