<script setup lang="ts">
import { defineAsyncComponent, ref, useId, useTemplateRef } from "vue";

const Lazy = defineAsyncComponent(() => import("./App.vue"));
const text = ref("");
const picked = ref("a");
const show = ref(true);
const id = useId();
const field = useTemplateRef<HTMLInputElement>("field");
</script>

<template>
  <Suspense>
    <Lazy />
    <template #fallback>loading</template>
  </Suspense>
  <Teleport to="body" defer><p :id="id">{{ field?.value }}</p></Teleport>
  <KeepAlive><component :is="Lazy" /></KeepAlive>
  <Transition name="fade"><p v-if="show">fade</p></Transition>
  <input ref="field" v-model.trim="text" type="text" />
  <select v-model="picked"><option value="a">a</option></select>
  <p class="checkbox" title="v-html">innerHTML</p>
</template>

<style scoped>
.fade-enter-active {
  transition: opacity 0.2s;
}
</style>
