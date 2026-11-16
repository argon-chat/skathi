<script setup lang="ts">
import { ref } from "vue";
import { SkWindow } from "@skathi-native/runtime";

const open = ref(false);
const revealed = ref(false);
</script>

<template>
  <main class="page">
    <button @click="open = true">Show recovery codes</button>
    <SkWindow
      v-if="open"
      title="Recovery codes"
      :bounds="{ width: 320, height: 220 }"
      :content-protection="revealed"
      @close="open = false"
    >
      <div class="codes">
        <p v-if="revealed">4F7K-29QX · 8LMD-0RTE · ZC3V-71HN</p>
        <button @click="revealed = !revealed">{{ revealed ? "Hide" : "Reveal" }}</button>
      </div>
    </SkWindow>
  </main>
</template>

<style>
.page,
.codes {
  padding: 24px;
}
</style>
