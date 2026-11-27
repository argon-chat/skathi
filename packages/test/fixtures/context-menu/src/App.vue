<script setup lang="ts">
import { onScopeDispose, ref, useTemplateRef } from "vue";
import {
  SkWindow,
  defineKeymap,
  SkContextMenu,
  SkKbd,
  SkMenuItem,
  SkMenuSeparator,
  SkSubmenu,
  type SkContextMenuHandle,
} from "@skathi-native/runtime";

const menu = useTemplateRef<SkContextMenuHandle<string>>("menu");
const files = ["Notes.txt", "Draft.txt"];
const last = ref("");
const shortcut = defineKeymap({ keys: "Control+KeyQ" });
onScopeDispose(
  shortcut.bind(() => {
    last.value = "Shortcut fired";
  }),
);
onScopeDispose(
  shortcut.on("up", (event) => {
    if (event.cancelled) last.value = "Shortcut cancelled";
  }),
);
const extra = ref(false);
const debugAim = ref(false);
const detached = ref(false);
Object.assign(globalThis, {
  showMenuAim: (value: boolean) => (debugAim.value = value),
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
    <SkKbd id="layout-shortcut" :map="shortcut" />
  </main>
  <SkWindow v-if="detached" title="Detached files" :bounds="{ width: 250, height: 140 }">
    <button id="detached-trigger" @click="menu?.open($event.currentTarget as Element, 'Detached.txt')">
      Detached.txt
    </button>
  </SkWindow>
  <SkContextMenu
    ref="menu"
    v-slot="{ context: file }"
    aria-label="File actions"
    backdrop="acrylic"
    :debug-aim="debugAim"
  >
    <SkMenuItem @select="last = 'Opened ' + file">Open</SkMenuItem>
    <SkMenuItem disabled>Rename <SkKbd keys="F2" variant="plain" /></SkMenuItem>
    <SkMenuItem v-if="extra">
      Open in a separate application window
      <SkKbd :keys="{ alternatives: [{ mouse: 'middle' }, { gamepad: 'south' }] }" gamepad-layout="playstation" />
    </SkMenuItem>
    <SkMenuSeparator />
    <SkSubmenu label="Copy">
      <SkMenuItem @select="last = 'Copied ' + file">File name</SkMenuItem>
      <SkMenuItem>File path</SkMenuItem>
      <SkMenuItem>Folder name</SkMenuItem>
    </SkSubmenu>
    <SkSubmenu label="Move">
      <SkMenuItem @select="last = 'Archived ' + file">Archive</SkMenuItem>
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
