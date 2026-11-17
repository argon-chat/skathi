<script setup lang="ts">
import { release } from "./log";

await release("suspense");
const message = "resolved";
</script>

<template>
  <p id="sus-done">{{ message }}</p>
</template>
