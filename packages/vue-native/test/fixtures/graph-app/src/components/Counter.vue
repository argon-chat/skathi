<script setup lang="ts">
import { ref } from "vue";
import { greet } from "../util";

const n = ref(0);
</script>

<template>
  <div class="counter" @click="n++">{{ greet(n) }}</div>
</template>
