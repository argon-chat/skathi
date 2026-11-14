<script setup lang="ts">
import { icons, type IconName } from "../icons";

defineProps<{ name: IconName }>();
</script>

<template>
  <svg
    class="icon"
    xmlns="http://www.w3.org/2000/svg"
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.8"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path :d="icons[name]" />
  </svg>
</template>

<style scoped>
.icon {
  width: 16px;
  height: 16px;
}
</style>
