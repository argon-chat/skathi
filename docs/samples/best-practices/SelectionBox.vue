<script setup lang="ts">
const props = defineProps<{ sel: { row: number; col: number } }>();
</script>

<template>
  <div class="selection" :style="{ transform: `translate(${props.sel.col * 80}px, ${props.sel.row * 24}px)` }" />
</template>

<style scoped>
.selection {
  position: absolute;
  top: 0;
  left: 0;
  width: 80px;
  height: 24px;
  border: 2px solid #4c8dff;
  pointer-events: none;
}
</style>
