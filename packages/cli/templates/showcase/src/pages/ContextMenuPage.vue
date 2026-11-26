<script setup lang="ts">
import {
  SkContextMenu,
  SkMenuItem,
  SkMenuSeparator,
  SkSubmenu,
  type SkContextMenuHandle,
  type SkMenuBackdrop,
} from "@skathi-native/runtime";
import { computed, ref, useTemplateRef } from "vue";
import Card from "../components/Card.vue";
import Icon from "../components/Icon.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";
import type { IconName } from "../icons";

const folders = ["Drafts", "Review", "Archive"] as const;
type Folder = (typeof folders)[number];
interface SampleFile {
  name: string;
  icon: IconName;
  folder: Folder;
  pinned: boolean;
}

const files = ref<SampleFile[]>([
  { name: "Release notes.md", icon: "files", folder: "Drafts", pinned: false },
  { name: "Cover artwork.png", icon: "media", folder: "Review", pinned: false },
  { name: "App icon.svg", icon: "graphics", folder: "Drafts", pinned: false },
]);
const ordered = computed(() => [...files.value].sort((a, b) => Number(b.pinned) - Number(a.pinned)));
const menu = useTemplateRef<SkContextMenuHandle<SampleFile>>("menu");
const backdrop = ref<SkMenuBackdrop>("acrylic");
const status = ref("");

function togglePin(file: SampleFile) {
  file.pinned = !file.pinned;
  status.value = (file.pinned ? "Pinned " : "Unpinned ") + file.name;
}
function moveTo(file: SampleFile, folder: Folder) {
  file.folder = folder;
  status.value = `Moved ${file.name} to ${folder}`;
}
async function copyName(file: SampleFile) {
  try {
    await navigator.clipboard.writeText(file.name);
    status.value = "Copied " + file.name;
  } catch (error) {
    status.value = describe(error);
  }
}
</script>

<template>
  <Page
    title="Context menu"
    lead="Right-click a file, or open Actions to anchor its menu to the button. With Actions focused, try Shift+F10 and the arrow keys."
  >
    <Card title="Sample files" api="<SkContextMenu>">
      <ul class="files">
        <li v-for="file in ordered" :key="file.name" class="file" @contextmenu.prevent="menu?.open($event, file)">
          <span class="file-icon"><Icon :name="file.icon" /></span>
          <div class="file-info">
            <span class="file-name">{{ file.name }}</span>
            <span class="muted">{{ file.folder }}{{ file.pinned ? " · Pinned" : "" }}</span>
          </div>
          <button
            class="btn"
            aria-haspopup="menu"
            :aria-label="'Actions for ' + file.name"
            @click="menu?.open($event.currentTarget as Element, file)"
          >
            Actions
          </button>
        </li>
      </ul>
      <div class="row controls">
        <label class="material">
          Menu backdrop
          <select v-model="backdrop" class="input" aria-label="Menu backdrop">
            <option value="acrylic">Acrylic</option>
            <option value="none">Opaque</option>
          </select>
        </label>
        <output class="muted" role="status">{{ status }}</output>
      </div>
    </Card>
    <SkContextMenu ref="menu" v-slot="{ context: file }" :backdrop="backdrop" aria-label="File actions">
      <SkMenuItem @select="togglePin(file)">{{ file.pinned ? "Unpin" : "Pin to top" }}</SkMenuItem>
      <SkMenuItem @select="copyName(file)">Copy file name</SkMenuItem>
      <SkMenuSeparator />
      <SkSubmenu label="Move to">
        <SkMenuItem
          v-for="folder in folders"
          :key="folder"
          :disabled="folder === file.folder"
          @select="moveTo(file, folder)"
        >
          {{ folder }}
        </SkMenuItem>
      </SkSubmenu>
    </SkContextMenu>
  </Page>
</template>

<style scoped>
.files {
  display: flex;
  flex-direction: column;
  margin: 0;
  padding: 0;
  list-style: none;
}
.file {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 0;
  border-bottom: 1px solid var(--line);
}
.file-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: 8px;
  background: var(--accent-soft);
  color: var(--accent);
}
.file-info {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.file-name {
  color: var(--text);
  font-size: 14px;
  font-weight: 600;
}
.controls {
  justify-content: space-between;
}
.material {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--text-muted);
}
</style>
