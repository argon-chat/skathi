<script setup lang="ts">
import { ref } from "vue";

const formats = ref<string[]>(["flac"]);
const quality = ref("high");
</script>

<template>
  <fieldset>
    <legend>Export as</legend>
    <!-- #region checks -->
    <label><input type="checkbox" value="flac" v-model="formats" /> FLAC</label>
    <label><input type="checkbox" value="opus" v-model="formats" /> Opus</label>
    <!-- #endregion checks -->
  </fieldset>
  <fieldset>
    <legend>Quality</legend>
    <!-- #region radios -->
    <label><input type="radio" value="high" v-model="quality" /> High</label>
    <label><input type="radio" value="small" v-model="quality" /> Small</label>
    <!-- #endregion radios -->
  </fieldset>
  <p>{{ formats.join(", ") || "Nothing" }}, {{ quality }}</p>
</template>

<style scoped>
input {
  accent-color: #c4569a;
}
</style>
