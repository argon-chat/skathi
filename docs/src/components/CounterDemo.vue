<script setup lang="ts">
import { ref } from "vue";
const count = ref(0);
</script>

<template>
  <div class="vue-example">
    <div>
      <strong>A little reactivity.</strong>
      <p>A Vue island, running inside this documentation page.</p>
    </div>
    <button @click="count++" aria-live="polite">
      {{ count === 0 ? "Try it" : `Clicked ${count} ${count === 1 ? "time" : "times"}` }}
    </button>
  </div>
</template>
