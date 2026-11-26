<script setup lang="ts">
import { ref, useTemplateRef } from "vue";
import {
  SkContextMenu,
  SkMenuItem,
  SkMenuSeparator,
  SkSubmenu,
  type SkContextMenuHandle,
} from "@skathi-native/runtime";

const menu = useTemplateRef<SkContextMenuHandle<string>>("menu");
const files = ["Notes.txt", "Draft.txt"];
const last = ref("");
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
  <SkContextMenu ref="menu" v-slot="{ context: file }" aria-label="File actions" backdrop="acrylic">
    <SkMenuItem @select="last = 'Opened ' + file">Open</SkMenuItem>
    <SkMenuItem disabled>Rename</SkMenuItem>
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
