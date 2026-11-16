<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import Icon from "../components/Icon.vue";
import Scenario from "../components/Scenario.vue";
import type { IconName } from "../icons";

const width = ref(0);
const read = () => (width.value = Math.round(innerWidth));
onMounted(() => {
  read();
  addEventListener("resize", read);
});
onUnmounted(() => removeEventListener("resize", read));

const FILES: { name: string; icon: IconName; meta: string }[] = [
  { name: "Window guides.md", icon: "doc", meta: "Edited 10 min ago" },
  { name: "Poster draft.svg", icon: "image", meta: "Edited yesterday" },
  { name: "Clips.zip", icon: "archive", meta: "48.2 MB" },
  { name: "Budget 2027.xlsx", icon: "sheet", meta: "Shared by Mira" },
  { name: "Launch notes.md", icon: "doc", meta: "Edited Monday" },
  { name: "Theme study.png", icon: "image", meta: "2560 × 1440" },
];
</script>

<template>
  <Scenario id="title-bar/edge-resize">
    <div class="card reads">
      <div class="read">
        <span class="k">innerWidth</span><span class="v">{{ width }} px</span>
      </div>
      <div class="read">
        <span class="k">Sidebar</span>
        <span class="v">
          <span class="wide-only">full, wider than 900 px</span>
          <span class="narrow-only">collapsed to a rail, 900 px and narrower</span>
        </span>
      </div>
    </div>
    <div class="files">
      <div v-for="f in FILES" :key="f.name" class="file card">
        <span class="thumb"><Icon :name="f.icon" /></span>
        <span class="name">{{ f.name }}</span>
        <span class="meta">{{ f.meta }}</span>
      </div>
    </div>
  </Scenario>
</template>

<style scoped>
.reads {
  gap: 10px;
}
.read {
  display: flex;
  align-items: center;
  gap: 16px;
}
.k {
  width: 140px;
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text-3);
}
.v {
  font-size: 14px;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
.narrow-only {
  display: none;
}
@media (max-width: 900px) {
  .wide-only {
    display: none;
  }
  .narrow-only {
    display: inline;
  }
}
.files {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 12px;
}
.file {
  gap: 8px;
  padding: 16px;
}
.thumb {
  width: 40px;
  height: 40px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-soft);
  color: var(--accent-text);
}
.thumb .icon {
  width: 20px;
  height: 20px;
}
.name {
  font-size: 14px;
  color: var(--text);
}
.meta {
  font-size: 12px;
  color: var(--text-3);
}
</style>
