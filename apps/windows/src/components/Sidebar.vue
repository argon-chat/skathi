<script setup lang="ts">
import { GUIDES, scenarios } from "../scenarios";
import { go, nav } from "../store";
import Icon from "./Icon.vue";

const groups = GUIDES.map((guide) => ({ guide, items: scenarios.filter((s) => s.guide === guide) }));
</script>

<template>
  <nav class="sidebar">
    <div v-for="g in groups" :key="g.guide" class="group">
      <div class="group-title">{{ g.guide }}</div>
      <div
        v-for="s in g.items"
        :key="s.id"
        class="item"
        :class="{ on: nav.id === s.id }"
        :title="s.title"
        @click="go(s.id)"
      >
        <span class="indicator" />
        <Icon :name="s.icon" />
        <span class="label">{{ s.title }}</span>
      </div>
    </div>
  </nav>
</template>

<style scoped>
.sidebar {
  width: 272px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 6px 8px 16px;
  overflow-y: auto;
}
.group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.group-title {
  height: 28px;
  display: flex;
  align-items: center;
  padding: 0 14px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
}
.item {
  height: 36px;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 12px 0 0;
  border-radius: 5px;
  color: var(--text);
  font-size: 14px;
  cursor: pointer;
  user-select: none;
  transition: background-color 100ms ease;
}
.item:hover {
  background: var(--subtle-hover);
}
.item.on {
  background: var(--subtle-hover);
}
.indicator {
  width: 3px;
  height: 16px;
  border-radius: 2px;
  background: transparent;
  transition: background-color 120ms ease;
}
.item.on .indicator {
  background: var(--accent);
}
.label {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
@media (max-width: 900px) {
  .sidebar {
    width: 64px;
    padding: 6px 6px 16px;
    gap: 10px;
  }
  .group-title {
    height: 1px;
    padding: 0;
    margin: 0 10px;
    overflow: hidden;
    background: var(--layer-line);
    color: transparent;
  }
  .item {
    gap: 0;
    padding: 0;
  }
  .item .icon {
    margin-left: 14px;
  }
  .label {
    display: none;
  }
}
</style>
