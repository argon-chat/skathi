<script setup lang="ts">
const model = defineModel<string>({ required: true });
const [title, modifiers] = defineModel<string, "upper">("title", {
  set: (value) => value,
});
</script>

<template>
  <input id="dm-input" :value="model" @input="model = ($event.target as HTMLInputElement).value" />
  <button id="dm-title" @click="title = modifiers.upper ? 'NEW' : 'new'">title</button>
</template>
