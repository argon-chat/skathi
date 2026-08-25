<script setup>
import { ref } from "vue";
import Counter from "./components/Counter.vue";
import { greet } from "./util.js";

const who = ref("fixture");
</script>

<template>
  <div class="app">
    <div class="title">{{ greet(who) }}</div>
    <Counter />
  </div>
</template>

<style scoped>
.app {
  padding: 8px;
}
</style>
