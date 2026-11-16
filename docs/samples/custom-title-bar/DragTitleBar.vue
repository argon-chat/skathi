<script setup lang="ts">
import { ref } from "vue";
import { appWindow } from "@skathi-native/runtime";

const query = ref("");

function toggleTheme() {
  appWindow.theme = appWindow.theme === "dark" ? "light" : "dark";
}
</script>

<template>
  <div class="shell">
    <header class="titlebar">
      <span class="title">{{ appWindow.title }}</span>
      <input v-model="query" class="search" placeholder="Search notes" />
      <button class="theme" @click="toggleTheme">Theme</button>
    </header>
    <main class="content">
      <p>Drag the bar above to move the window. Double-click it to maximize.</p>
    </main>
  </div>
</template>

<style>
.shell {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.titlebar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: env(titlebar-area-height, 36px);
  width: env(titlebar-area-width, 100%);
  padding: 0 12px;
  /* #region drag */
  app-region: drag;
  /* #endregion drag */
  user-select: none;
}
.title {
  flex: 1;
  font-size: 12px;
}
/* #region no-drag */
.search,
.theme {
  app-region: no-drag;
}
/* #endregion no-drag */
.search {
  width: 220px;
}
.content {
  flex: 1;
  padding: 24px;
}
</style>
