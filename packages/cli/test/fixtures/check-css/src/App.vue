<script setup lang="ts">
const count = 1;
</script>

<template>
  <div class="card" style="color: red; float: left">{{ count }}</div>
  <p style="color: blue">fine</p>
</template>

<style scoped>
.card {
  color: red;
  filter: grayscale(1);
}
a:visited {
  color: purple;
}
</style>
