<script setup>
import { ref } from "vue";
import { greet } from "../util.js";

const n = ref(0);
</script>

<template>
  <div class="counter" @click="n++">{{ greet(n) }}</div>
</template>
