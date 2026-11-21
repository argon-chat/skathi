<script setup lang="ts">
import { appDevtools } from "skathi:app";
import { computed } from "vue";
import { useRoute } from "vue-router";
import FullWindow from "./components/FullWindow.vue";
import Icon from "./components/Icon.vue";
import Titlebar from "./components/Titlebar.vue";
import { pages } from "./router";

const devtools = appDevtools.available && appDevtools.isEnabled();
const route = useRoute();
const current = computed(() => pages.find((p) => p.path === route.path)?.title ?? "");

const sections = computed(() => {
  const out = new Map<string, typeof pages>();
  for (const page of pages) out.set(page.section, [...(out.get(page.section) ?? []), page]);
  return [...out];
});
</script>

<template>
  <div class="app">
    <Titlebar :page="current" />
    <div class="body">
      <nav class="sidebar">
        <div v-for="[section, items] in sections" :key="section" class="section">
          <div v-if="section" class="section-title">{{ section }}</div>
          <RouterLink v-for="page in items" :key="page.path" :to="page.path" class="link">
            <Icon :name="page.icon" />
            <span>{{ page.title }}</span>
          </RouterLink>
        </div>
        <div v-if="devtools" class="foot">F12 opens the devtools</div>
      </nav>
      <main class="content">
        <RouterView />
      </main>
    </div>
    <FullWindow />
  </div>
</template>

<style>
.app {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #0b0b10a6;
  color: var(--text);
  font-family: var(--font);
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.sidebar {
  width: 210px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px 10px;
  border-right: 1px solid var(--line);
  background: #00000026;
  overflow-y: auto;
}
.section {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.section-title {
  padding: 0 10px 4px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: var(--text-faint);
}
.link {
  height: 32px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  border-radius: 8px;
  font-size: 13px;
  color: var(--text-muted);
  cursor: pointer;
  transition:
    background-color 120ms ease,
    color 120ms ease;
}
.link:hover {
  background: var(--surface-hover);
  color: var(--text);
}
.link.router-link-exact-active {
  background: var(--accent-soft);
  color: #ffffff;
}
.foot {
  margin-top: auto;
  padding: 8px 10px 0;
  font-size: 11px;
  color: var(--text-faint);
}
.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
}
/* Page navigations (router.ts): the page cross-fades while its title moves into place. */
::view-transition-group(*) {
  animation-duration: 220ms;
  animation-timing-function: cubic-bezier(0.2, 0, 0, 1);
}
::view-transition-old(page-title),
::view-transition-new(page-title) {
  width: auto;
  height: 100%;
}
</style>
