<template>
  <div>{{ value }}</div>
</template>

<script setup lang="ts">
const value = eval("1") as number;
function handler(event) {
  return event;
}
handler(value);
</script>
