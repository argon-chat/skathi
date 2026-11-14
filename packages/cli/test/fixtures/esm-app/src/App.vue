<script lang="ts">
// A glob inside an SFC goes through the same expansion as a plain module.
const parts = import.meta.glob("./kinds/*.ts", { eager: true });
export default { name: "EsmApp", parts: Object.keys(parts) };
</script>

<template>
  <div class="app">esm</div>
</template>
