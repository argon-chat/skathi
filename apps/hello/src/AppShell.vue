<script setup lang="ts">
// `:exclude` keeps ONE page out of the cache on purpose: the Lab tab remounts every visit, which
// is what proves pinia state is independent of KeepAlive.
import Titlebar from "./components/Titlebar.vue";
import { useSession } from "./stores/session";

const session = useSession();
const excluded = ["LabPage"];
</script>

<template>
  <div class="frame">
    <Titlebar />
    <div class="shell">
      <div class="bar">
        <div class="nav">
          <RouterLink class="tab" to="/">Forms</RouterLink>
          <RouterLink class="tab" to="/list">List</RouterLink>
          <RouterLink class="tab" to="/motion">Motion</RouterLink>
          <RouterLink class="tab" to="/media">Media</RouterLink>
          <RouterLink class="tab" to="/audio">Audio</RouterLink>
          <RouterLink class="tab" to="/styles">Styles</RouterLink>
          <RouterLink class="tab" to="/window">Window</RouterLink>
          <RouterLink class="tab" to="/permissions">Perms</RouterLink>
          <RouterLink class="tab" to="/storage">Storage</RouterLink>
          <RouterLink class="tab" to="/lab">Lab</RouterLink>
          <!-- After Lab, so the specs' tab indices (hello, hover, engine) still hold. -->
          <RouterLink class="tab" to="/desktop">Files</RouterLink>
        </div>
        <div class="store">pinia · session · {{ session.badge }}</div>
      </div>

      <RouterView v-slot="{ Component }">
        <KeepAlive :exclude="excluded">
          <component :is="Component" />
        </KeepAlive>
      </RouterView>
    </div>
  </div>
</template>

<style scoped>
.frame {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.shell {
  display: flex;
  flex-direction: column;
  flex: 1;
  gap: 10px;
  padding: 14px;
  overflow-y: scroll;
}
.bar {
  display: flex;
  flex-direction: row;
  gap: 10px;
  align-items: center;
}
.nav {
  display: flex;
  flex-direction: row;
  gap: 6px;
}
.tab {
  padding: 6px;
  border-radius: 6px;
  font-size: 12px;
  color: #ffffffaa;
  background: #ffffff0d;
  cursor: pointer;
  transition:
    background-color 150ms ease,
    color 150ms ease;
}
.tab:hover {
  background: #ffffff22;
  color: #ffffff;
}
.tab.router-link-active {
  background: #4f8cff59;
  color: #ffffff;
}
.store {
  padding: 6px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffd479;
  background: #ffd4791a;
  border: 1px solid #ffd47940;
}
</style>
