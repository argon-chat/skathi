<script setup lang="ts">
import { ref } from "vue";

const tint = ref("rgb(0, 128, 0)");
</script>

<template>
  <div id="styled" class="scoped">
    <button id="vbind" class="vbind" @click="tint = 'rgb(0, 0, 255)'">v-bind</button>
    <slot />
    <div class="inner"><span id="deep" class="deep">deep</span></div>
  </div>
</template>

<style scoped>
.scoped {
  color: rgb(10, 20, 30);
}
.vbind {
  color: v-bind(tint);
}
:deep(.deep) {
  color: rgb(255, 0, 255);
}
:slotted(.slotted) {
  color: rgb(0, 200, 200);
}
:global(.global-mark) {
  color: rgb(200, 100, 0);
}
</style>
