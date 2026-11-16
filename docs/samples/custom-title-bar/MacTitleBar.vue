<script setup lang="ts">
import { appWindow } from "@skathi-native/runtime";
</script>

<template>
  <header class="titlebar">{{ appWindow.title }}</header>
</template>

<style>
.titlebar {
  display: flex;
  align-items: center;
  height: env(titlebar-area-height, 36px);
  /* #region clear */
  padding-left: calc(env(titlebar-area-x, 0px) + 12px);
  /* #endregion clear */
  app-region: drag;
}
</style>
