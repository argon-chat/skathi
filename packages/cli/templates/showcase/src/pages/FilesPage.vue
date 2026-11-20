<script setup lang="ts">
import { ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";

// The File System Access API: a picker needs a click, and a handle names its file but never shows its path.
const TYPES = [{ description: "Text", accept: { "text/plain": [".txt", ".md", ".log"] } }];

const text = ref("Write something, then save it.\n");
const file = shallowRef<FileSystemFileHandle | null>(null);
const status = ref("");
const dropped = shallowRef<File[]>([]);
const over = ref(false);
const transferStatus = ref("");

function acceptFiles(event: DragEvent) {
  const transfer = event.dataTransfer;
  if (!transfer?.types.includes("Files")) return;
  if (["none", "move", "link", "linkMove"].includes(transfer.effectAllowed)) return;
  event.preventDefault();
  transfer.dropEffect = "copy";
  over.value = true;
}

function receive(event: DragEvent) {
  over.value = false;
  const files = event.dataTransfer?.files;
  if (!files?.length) return;
  event.preventDefault();
  dropped.value = Array.from(files);
  transferStatus.value = "";
}

function send(event: DragEvent, files: File[]) {
  const transfer = event.dataTransfer;
  if (!transfer) return;
  transfer.effectAllowed = "copy";
  for (const item of files) transfer.items.add(item);
  transferStatus.value = "";
}

function sendNote(event: DragEvent) {
  send(event, [new File([text.value], "notes.txt", { type: "text/plain" })]);
}

function finished(event: DragEvent) {
  transferStatus.value = event.dataTransfer?.dropEffect === "copy" ? "Copy completed" : "Drag cancelled";
}

async function attempt(action: () => Promise<string>) {
  try {
    status.value = await action();
  } catch (e) {
    status.value = describe(e);
  }
}

async function write(handle: FileSystemFileHandle) {
  const stream = await handle.createWritable();
  await stream.write(text.value);
  await stream.close();
}

const open = () =>
  attempt(async () => {
    const [handle] = await showOpenFilePicker({ id: "notes", types: TYPES });
    if (!handle) return "";
    text.value = await (await handle.getFile()).text();
    file.value = handle;
    return `Opened ${handle.name}`;
  });

const save = () =>
  attempt(async () => {
    const handle = file.value ?? (await showSaveFilePicker({ id: "notes", suggestedName: "notes.txt", types: TYPES }));
    await write(handle);
    file.value = handle;
    return `Saved ${handle.name}`;
  });

const copy = () =>
  attempt(async () => {
    await navigator.clipboard.writeText(text.value);
    return "Copied to the clipboard";
  });

const paste = () =>
  attempt(async () => {
    text.value = await navigator.clipboard.readText();
    return "Pasted from the clipboard";
  });
</script>

<template>
  <Page title="Files" lead="Drag files between apps, open and save documents, or use the system clipboard.">
    <Card title="Drag and drop" api="DataTransfer">
      <div class="drop-zone" :class="{ over }" @dragover="acceptFiles" @dragleave="over = false" @drop="receive">
        {{ over ? "Release to add files" : "Drop files here" }}
      </div>
      <div class="row">
        <span class="btn drag-file note-export" draggable="true" @dragstart="sendNote" @dragend="finished">
          Drag notes.txt out
        </span>
        <span
          v-if="dropped.length"
          class="btn drag-file dropped-export"
          draggable="true"
          @dragstart="send($event, dropped)"
          @dragend="finished"
        >
          Drag {{ dropped.length === 1 ? dropped[0]?.name : dropped.length + " files" }} out
        </span>
        <span v-if="transferStatus" class="faint" role="status">{{ transferStatus }}</span>
      </div>
      <p class="faint">Drag notes.txt to a folder to export the current editor text.</p>
    </Card>
    <Card title="Editor" :api="file ? file.name : 'untitled'">
      <textarea v-model="text" class="input editor" spellcheck="false" />
      <div class="row">
        <button class="btn" @click="open">Open…</button>
        <button class="btn primary" @click="save">{{ file ? "Save" : "Save as…" }}</button>
        <button v-if="file" class="btn" @click="file = null">Forget the file</button>
        <span class="faint">{{ status }}</span>
      </div>
    </Card>

    <Card title="Clipboard" api="navigator.clipboard">
      <div class="row">
        <button class="btn" @click="copy">Copy the text</button>
        <button class="btn" @click="paste">Paste over it</button>
      </div>
      <p class="faint">Ctrl+C and Ctrl+V in text fields need none of this: they belong to the field.</p>
    </Card>
  </Page>
</template>

<style scoped>
.drop-zone {
  padding: 22px 12px;
  border: 1px dashed var(--line-strong);
  border-radius: 9px;
  text-align: center;
  color: var(--text-muted);
}
.drop-zone.over {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--text);
}
.drag-file {
  cursor: grab;
}
.editor {
  height: 220px;
  padding: 12px;
  font-family: var(--mono);
  line-height: 19px;
}
</style>
