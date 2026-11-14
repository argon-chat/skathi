<script setup lang="ts">
import { ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Missing from "../components/Missing.vue";
import Page from "../components/Page.vue";
import { clipboard, describe } from "../engine";

// The File System Access API: a picker needs a click, and a handle names its file but never shows its path.
const TYPES = [{ description: "Text", accept: { "text/plain": [".txt", ".md", ".log"] } }];

const text = ref("Write something, then save it.\n");
const file = shallowRef<SkFileSystemFileHandle | null>(null);
const status = ref("");

async function attempt(action: () => Promise<string>) {
  try {
    status.value = await action();
  } catch (e) {
    status.value = describe(e);
  }
}

async function write(handle: SkFileSystemFileHandle) {
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

type Clipboard = SkNavigator["clipboard"];

const copy = (board: Clipboard) =>
  attempt(async () => {
    await board.writeText(text.value);
    return "Copied to the clipboard";
  });

const paste = (board: Clipboard) =>
  attempt(async () => {
    text.value = await board.readText();
    return "Pasted from the clipboard";
  });
</script>

<template>
  <Page title="Files" lead="Native open and save dialogs through the web's own APIs, and the system clipboard.">
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
      <div v-if="clipboard" class="row">
        <button class="btn" @click="copy(clipboard)">Copy the text</button>
        <button class="btn" @click="paste(clipboard)">Paste over it</button>
      </div>
      <Missing v-else part="clipboard" />
      <p class="faint">Ctrl+C and Ctrl+V in text fields work either way: they belong to the field.</p>
    </Card>
  </Page>
</template>

<style scoped>
.editor {
  height: 220px;
  padding: 12px;
  font-family: var(--mono);
  line-height: 19px;
}
</style>
