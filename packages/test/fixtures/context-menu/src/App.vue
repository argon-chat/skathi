<script setup lang="ts">
import { ref, useTemplateRef } from "vue";
import {
  SkWindow,
  SkContextMenu,
  SkMenuItem,
  SkMenuSeparator,
  SkSubmenu,
  type SkContextMenuHandle,
} from "@skathi-native/runtime";

const menu = useTemplateRef<SkContextMenuHandle<string>>("menu");
const files = ["Notes.txt", "Draft.txt"];
const last = ref("");
const extra = ref(false);
const detached = ref(false);
Object.assign(globalThis, {
  resizeMenu: () => (extra.value = !extra.value),
  showDetached: (value: boolean) => (detached.value = value),
  closeMenu: () => menu.value?.close(),
});
</script>

<template>
  <main class="files">
    <button
      v-for="file in files"
      :key="file"
      @contextmenu.prevent="menu?.open($event, file)"
      @click="menu?.open($event.currentTarget as Element, file)"
    >
      {{ file }}
    </button>
    <output>{{ last }}</output>
  </main>
  <SkWindow v-if="detached" title="Detached files" :bounds="{ width: 250, height: 140 }">
    <button id="detached-trigger" @click="menu?.open($event.currentTarget as Element, 'Detached.txt')">
      Detached.txt
    </button>
  </SkWindow>
  <SkContextMenu ref="menu" v-slot="{ context: file }" aria-label="File actions" backdrop="acrylic">
    <SkMenuItem @select="last = 'Opened ' + file">Open</SkMenuItem>
    <SkMenuItem disabled>Rename</SkMenuItem>
    <SkMenuItem v-if="extra">Open in a separate application window</SkMenuItem>
    <SkMenuSeparator />
    <SkSubmenu label="Copy">
      <SkMenuItem @select="last = 'Copied ' + file">File name</SkMenuItem>
    </SkSubmenu>
  </SkContextMenu>
</template>

<style>
.files {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 8px;
  padding: 16px;
}
</style>
