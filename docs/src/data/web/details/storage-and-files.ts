import type { Details } from "../../details";

export const details: Details = {
  "local-storage": {
    examples: [
      {
        title: "Remember the sidebar width between launches",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watch } from "vue";

const KEY = "layout.sidebarWidth";
const width = ref(Number(localStorage.getItem(KEY)) || 240);

watch(width, (px) => localStorage.setItem(KEY, String(px)));
</script>

<template>
  <aside class="sidebar" :style="{ width: width + 'px' }">
    <input v-model.number="width" type="range" min="160" max="480" />
  </aside>
</template>`,
      },
      {
        title: "Save settings as JSON and handle a full store",
        lang: "ts",
        code: `export function saveSettings(settings: Record<string, unknown>): boolean {
  try {
    localStorage.setItem("settings", JSON.stringify(settings));
    return true;
  } catch (e) {
    if (e instanceof DOMException && e.name === "QuotaExceededError") return false;
    throw e;
  }
}

export const loadSettings = (): Record<string, unknown> => JSON.parse(localStorage.getItem("settings") ?? "{}");`,
        note: "A refused write leaves the previous value in place.",
      },
    ],
    edgeCases: [
      'Keys and values are converted to strings: an object is stored as `"[object Object]"`, and `setItem("key")` with no value stores `"undefined"`.',
      "The quota is 5 MiB, counting every key and value at two bytes per UTF-16 code unit. A write past it throws a `QuotaExceededError` (`code` `22`) and stores nothing.",
      "The `storage` event never fires: there is no second tab to hear a change. Adding a listener for it logs an advisory once.",
      "Changes reach the disk at most once per frame and once more when the app quits; `appStorage.flush()` writes the pending change at once.",
      "Two running instances that share one storage folder do not see each other's writes, and the last one to write wins. Give a second instance its own folder with `SK_APP_NAME`.",
    ],
  },
  "session-storage": {
    examples: [
      {
        title: "Keep a wizard's progress while the app runs",
        lang: "ts",
        code: `const KEY = "onboarding.step";

export function currentStep(): number {
  return Number(sessionStorage.getItem(KEY) ?? 0);
}

export function advance() {
  sessionStorage.setItem(KEY, String(currentStep() + 1));
}

export const finish = () => sessionStorage.removeItem(KEY);`,
        note: "The step survives route changes and unmounted components, and the next launch starts over.",
      },
    ],
    edgeCases: [
      "It starts empty on every launch and is never written to disk.",
      "It has the same 5 MiB quota and string conversion as `localStorage`.",
      "Components in an `<SkWindow>` run in the same runtime as the main window and see the same `sessionStorage`.",
    ],
  },
  "indexed-db": {
    examples: [
      {
        title: "Store and list notes",
        lang: "ts",
        code: `type Note = { id?: number; title: string; updated: Date };

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

const open = indexedDB.open("notes", 1);
open.onupgradeneeded = () => open.result.createObjectStore("notes", { keyPath: "id", autoIncrement: true });
const db = req(open);

export async function addNote(title: string) {
  const store = (await db).transaction("notes", "readwrite").objectStore("notes");
  return req(store.add({ title, updated: new Date() }));
}

export async function allNotes(): Promise<Note[]> {
  return req((await db).transaction("notes").objectStore("notes").getAll());
}`,
        note: "The `Date` comes back as a `Date`: values go through the structured clone.",
      },
      {
        title: "Add an index in a later version",
        lang: "ts",
        code: `const open = indexedDB.open("notes", 2);

open.onupgradeneeded = (e) => {
  const db = open.result;
  if (e.oldVersion < 1) db.createObjectStore("notes", { keyPath: "id", autoIncrement: true });
  const notes = open.transaction!.objectStore("notes");
  if (!notes.indexNames.contains("byTag")) notes.createIndex("byTag", "tags", { multiEntry: true });
};`,
        note: "A connection still open on version 1 gets `versionchange` and is closed, so the upgrade is not blocked.",
      },
    ],
    edgeCases: [
      "Databases live in the `indexeddb` folder inside the `localStorage` folder and follow its encryption. Both are fixed at the first `indexedDB` call, so call `appStorage.setStorageFolder` before it.",
      "A transaction created in a timer or `requestAnimationFrame` callback stays active until the end of that frame's pump, not the end of the callback. Code that expects a `TransactionInactiveError` from a later callback in the same frame does not get one.",
      "One `readwrite` transaction runs per database at a time; others wait for it.",
      "A second instance of the app on the same storage folder fails `open()` and `databases()` with an `UnknownError` and logs an advisory.",
      "`getAllRecords()` and the options-dictionary form of `getAll()` are not available.",
    ],
  },
  "navigator-storage": {
    examples: [
      {
        title: "Show how much space the app's data uses",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const used = ref("");

onMounted(async () => {
  const { usage = 0, quota = 0 } = await navigator.storage.estimate();
  used.value = (usage / 2 ** 20).toFixed(1) + " MB of " + (quota / 2 ** 30).toFixed(0) + " GB";
});
</script>

<template>
  <p class="storage-usage">Local data: {{ used }}</p>
</template>`,
      },
    ],
    edgeCases: [
      "`usage` counts the IndexedDB databases, not `localStorage`.",
      "`quota` is the smaller of 60% of the disk that holds the storage folder and the space used plus the space free; 1 GiB if the disk cannot be asked.",
      "`persist()` and `persisted()` both resolve `true`: the runtime never evicts the app's data.",
      "There is no `getDirectory()`: the origin private file system is not available.",
    ],
  },
  "file-system-access": {
    examples: [
      {
        title: "Open a text document and save it back",
        lang: "ts",
        code: `let current: FileSystemFileHandle | null = null;
const types = [{ description: "Markdown", accept: { "text/markdown": [".md", ".markdown"] } }];

export async function openDocument(): Promise<string> {
  [current] = await showOpenFilePicker({ types, id: "documents" });
  return (await current.getFile()).text();
}

export async function saveDocument(text: string) {
  current ??= await showSaveFilePicker({ types, suggestedName: "Untitled.md" });
  const writable = await current.createWritable();
  await writable.write(text);
  await writable.close();
}`,
        note: "Call both from click or key handlers: a picker needs a user gesture.",
      },
      {
        title: "List the images in a folder",
        lang: "ts",
        code: `export async function pickPhotoFolder(): Promise<File[]> {
  const folder = await showDirectoryPicker({ startIn: "pictures" });
  const photos: File[] = [];
  for await (const [name, handle] of folder.entries()) {
    if (handle.kind === "file" && /\\.(jpe?g|png|webp)$/i.test(name)) photos.push(await handle.getFile());
  }
  return photos;
}`,
      },
      {
        title: "Reopen the last project folder on the next launch",
        lang: "ts",
        code: `const db = new Promise<IDBDatabase>((resolve) => {
  const open = indexedDB.open("project", 1);
  open.onupgradeneeded = () => open.result.createObjectStore("handles");
  open.onsuccess = () => resolve(open.result);
});

export async function rememberFolder(folder: FileSystemDirectoryHandle) {
  (await db).transaction("handles", "readwrite").objectStore("handles").put(folder, "last");
}

export async function reopenFolder(): Promise<FileSystemDirectoryHandle | null> {
  const get = (await db).transaction("handles").objectStore("handles").get("last");
  const folder = await new Promise<FileSystemDirectoryHandle | undefined>((r) => (get.onsuccess = () => r(get.result)));
  if (!folder || (await folder.requestPermission({ mode: "readwrite" })) !== "granted") return null;
  return folder;
}`,
        note: "A handle survives the structured clone into IndexedDB. Call `reopenFolder` from a click: `requestPermission` on a restored handle rejects with a `SecurityError` without a user gesture.",
      },
    ],
    edgeCases: [
      "Without a user gesture every picker rejects with a `SecurityError` and no dialog opens. Options are checked first, so a bad option rejects with a `TypeError` even then.",
      "Cancelling a picker rejects with an `AbortError`; asking for a second picker while one is open rejects with a `NotAllowedError`.",
      "An extension in `accept` must start with a dot, an `id` may hold at most 32 letters, digits, `_` or `-`, `startIn` must be a handle or one of `desktop`, `documents`, `downloads`, `music`, `pictures` and `videos`, and `excludeAcceptAllOption: true` needs `types`. Otherwise the call rejects with a `TypeError`.",
      "A writable stream writes to a `.vnswap` file beside the target, which stays untouched until `close()` replaces it in one step. `abort()`, a crash or a stream collected without `close()` leaves the file as it was.",
      "A user gesture is a mouse press or a key press, other than Escape or a lone modifier, in the last 5 seconds; `el.click()` and `dispatchEvent` do not count. Opening a picker uses the gesture up, so a second picker from the same click rejects with a `SecurityError`.",
    ],
  },
  "input-type-file": {
    examples: [
      {
        title: "Import contacts from a CSV file",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const rows = ref<string[][]>([]);

async function onPick(e: Event) {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (!file) return;
  rows.value = (await file.text()).trim().split("\\n").map((line) => line.split(","));
}
</script>

<template>
  <input type="file" accept=".csv,text/csv" @change="onPick" />
  <p>{{ rows.length }} contacts</p>
</template>`,
      },
      {
        title: "Open the picker from your own button",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const emit = defineEmits<{ attach: [files: File[]] }>();
const input = ref<HTMLInputElement | null>(null);
</script>

<template>
  <button class="attach" @click="input!.showPicker()">Attach files</button>
  <input ref="input" class="hidden" type="file" multiple @change="emit('attach', Array.from(input!.files!))" />
</template>

<style scoped>
.hidden { display: none; }
</style>`,
        note: "`showPicker()` runs inside the button's click, which supplies the user gesture.",
      },
    ],
    edgeCases: [
      '`value` reads `C:\\fakepath\\` followed by the first file\'s name. Setting it to `""` clears the selection; any other value throws an `InvalidStateError`.',
      "A `click()` without a user gesture opens nothing and logs `File chooser dialog can only be shown with a user activation.`; `showPicker()` without one throws a `NotAllowedError`.",
      "A choice sets `files`, then fires `input` and `change`. Dismissing the picker, or choosing the same files again, fires `cancel` instead.",
      "A `<label for>` pointing at the input opens the same picker.",
      "The button and its status text copy Chromium's English strings (`Choose File`, `No file chosen`); restyle the button with `::file-selector-button`.",
    ],
  },
  "a-download": {
    examples: [
      {
        title: "Export a table as a CSV file",
        lang: "ts",
        code: `export function downloadCsv(rows: string[][], name = "report.csv") {
  const blob = new Blob([rows.map((r) => r.join(",")).join("\\n")], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}`,
        note: "The link never joins the tree, and revoking the URL right after `click()` is safe.",
      },
      {
        title: "Offer a generated file as a link",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watchEffect } from "vue";

const props = defineProps<{ notes: string }>();
const url = ref("");

watchEffect((onCleanup) => {
  const current = URL.createObjectURL(new Blob([props.notes], { type: "text/plain" }));
  url.value = current;
  onCleanup(() => URL.revokeObjectURL(current));
});
</script>

<template>
  <a :href="url" download="meeting-notes.txt">Save notes</a>
</template>`,
      },
    ],
    edgeCases: [
      "The file is saved through a Save As dialog that starts in the Downloads folder with the `download` name filled in. Chromium saves to Downloads without asking.",
      "Only `blob:` and `data:` URLs are saved. An `https:` link with `download` opens in the system browser instead.",
      "The URL is read at the click, so `URL.revokeObjectURL` right after `click()` still saves the file.",
    ],
  },
};
