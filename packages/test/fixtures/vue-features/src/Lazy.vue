<script setup lang="ts">
defineProps<{ label: string }>();
</script>

<template>
  <span id="lazy">{{ label }}</span>
</template>
