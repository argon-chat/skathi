<script setup lang="ts">
import { ref } from "vue";
import { SkWindow, type SkWindowBounds, type SkWindowTarget } from "@skathi-native/runtime";

type Rect = { x: number; y: number; width: number; height: number };
type Menu = { mapFromMain(target: SkWindowTarget | null): Rect | null };

const actions = ["Cut", "Copy", "Paste"];
const page = ref<SkWindowTarget | null>(null);
const menu = ref<Menu | null>(null);
const open = ref(false);
const last = ref("nothing yet");
const bounds = ref<SkWindowBounds>({ x: 100, y: 100, width: 160, height: 112 });

function showAt(e: MouseEvent) {
  const r = menu.value?.mapFromMain(page.value) ?? null;
  if (!r) return;
  const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const { x = 0, y = 0 } = bounds.value;
  bounds.value = { ...bounds.value, x: x + r.x + e.clientX - box.left, y: y + r.y + e.clientY - box.top };
  open.value = true;
}

function pick(action: string) {
  last.value = action;
  open.value = false;
}
</script>

<template>
  <main ref="page" class="page" @contextmenu.prevent="showAt" @click="open = false">
    <p>Right-click anywhere. Last action: {{ last }}.</p>
  </main>
  <SkWindow
    ref="menu"
    title="Menu"
    chrome="hidden"
    backdrop="transparent"
    :resizable="false"
    :focusable="false"
    :taskbar="false"
    :visible="open"
    :bounds="bounds"
  >
    <div class="menu">
      <div v-for="a in actions" :key="a" class="item" @click="pick(a)">{{ a }}</div>
    </div>
  </SkWindow>
</template>

<style>
.page {
  height: 100%;
  padding: 24px;
}
.menu {
  display: flex;
  flex-direction: column;
  padding: 4px;
  border-radius: 8px;
  background: #1b1b1bf0;
  color: #ffffff;
}
.item {
  padding: 6px 12px;
  border-radius: 4px;
}
.item:hover {
  background: #0090ff;
}
</style>
