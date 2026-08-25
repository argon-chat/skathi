<script setup>
import { ref } from "vue";

const label = ref("style fixture");
</script>

<template>
  <div class="box">{{ label }}</div>
</template>

<style scoped>
.box {
  color: red;
}
</style>
