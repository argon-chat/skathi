<script setup lang="ts">
import { ref } from "vue";

const folders = ["Inbox", "Drafts", "Sent", "Archive"];
const current = ref("Inbox");
</script>

<template>
  <div class="shell">
    <header class="titlebar">Mail</header>
    <div class="body">
      <nav class="sidebar">
        <div v-for="f in folders" :key="f" class="folder" :class="{ on: f === current }" @click="current = f">
          {{ f }}
        </div>
      </nav>
      <main class="list">{{ current }}</main>
    </div>
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
  height: env(titlebar-area-height, 36px);
  padding-left: 12px;
  font-size: 12px;
  app-region: drag;
}
.body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.sidebar {
  width: 200px;
  padding: 8px;
  border-right: 1px solid #80808040;
}
.folder {
  padding: 6px 10px;
  border-radius: 6px;
}
.folder.on {
  background: #80808033;
}
.list {
  flex: 1;
  padding: 16px;
}
/* #region reflow */
@media (max-width: 640px) {
  .sidebar {
    display: none;
  }
}
/* #endregion reflow */
</style>
