<script setup>
// The multi-page shell: RouterLink tabs + a KeepAlive'd RouterView.
//
// RouterLink renders an <a> (a generic host element here) and stamps
// `router-link-active` on the active one — styled below through the ordinary
// class matcher.
//
// <KeepAlive> is the unmodified browser pattern: RouterView's scoped slot
// hands us the resolved component, KeepAlive caches its instance and MOVES the
// rendered subtree into a detached container instead of unmounting it. Coming
// back moves it home again, so page-local state (typed text, timers, scroll
// offsets) survives a tab switch.
//
// `:exclude` keeps ONE page out of the cache on purpose: the Lab tab remounts
// every visit, which is what proves pinia state is independent of KeepAlive.
import Titlebar from "./components/Titlebar.vue";
import { useSession } from "./stores/session.js";

const session = useSession();
// Matched against the component NAME (`defineOptions({ name })`, or the
// filename the SFC compiler infers into `__name`) — same rule as the browser.
const excluded = ["LabPage"];
</script>

<template>
  <div class="frame">
    <!-- The app's own titlebar. Declaring a `window-drag: move` region is what
         retires the host's built-in fallback strip (docs/WINDOWING.md §4.2
         mode 2) and hands the app the whole window. -->
    <Titlebar />
    <div class="shell">
    <div class="bar">
      <div class="nav">
        <RouterLink class="tab" to="/">Forms</RouterLink>
        <RouterLink class="tab" to="/list">List</RouterLink>
        <RouterLink class="tab" to="/motion">Motion</RouterLink>
        <RouterLink class="tab" to="/styles">Styles</RouterLink>
        <RouterLink class="tab" to="/window">Window</RouterLink>
        <RouterLink class="tab" to="/lab">Lab</RouterLink>
      </div>
      <!-- Live store read, visible on EVERY tab. -->
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
  /* The app owns the whole window now, so it also owns the scrollport the
     host's 24px-padded content box used to imply. */
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
  transition: background-color 150ms ease, color 150ms ease;
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
