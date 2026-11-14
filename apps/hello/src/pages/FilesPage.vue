<script setup lang="ts">
// Every picker needs a click (a user activation), as in Chromium; a handle never shows its path,
// only its `name`.
import { computed, onBeforeUnmount, ref, watch } from "vue";

defineOptions({ name: "FilesPage" });

const describe = (e: unknown) => (e instanceof Error ? `${e.name}: ${e.message}` : String(e));
const size = (n: number) =>
  n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KiB` : `${(n / 1048576).toFixed(1)} MiB`;

type Picked = { name: string; size: number; type: string; rel: string };
const picked = ref<Picked[]>([]);
const preview = ref("");
const inputLog = ref<string[]>([]);

function readText(file: SkFile): Promise<string> {
  return new Promise((done, fail) => {
    const r = new FileReader();
    r.onload = () => done(String(r.result));
    r.onerror = () => fail(r.error);
    r.readAsText(file.slice(0, 4096));
  });
}

async function onPick(e: SkEvent) {
  const input = e.target as SkFileInputElement;
  inputLog.value = [...inputLog.value, `change (${input.value || "—"})`].slice(-4);
  picked.value = [...input.files].map((f) => ({ name: f.name, size: f.size, type: f.type, rel: f.webkitRelativePath }));
  const first = input.files.item(0);
  preview.value = "";
  if (first && (first.type.startsWith("text/") || first.type === "application/json" || first.type === "")) {
    try {
      preview.value = await readText(first);
    } catch (err) {
      preview.value = describe(err);
    }
  }
}
const onCancel = () => (inputLog.value = [...inputLog.value, "cancel"].slice(-4));

const handle = ref<SkFileSystemFileHandle | null>(null);
const text = ref("");
const fsaStatus = ref("");
const TEXT_TYPES = [
  { description: "Text", accept: { "text/plain": [".txt", ".md", ".log"], "application/json": [".json"] } },
];

async function openFile() {
  try {
    const [h] = await showOpenFilePicker({ id: "hello-files", types: TEXT_TYPES });
    handle.value = h!;
    text.value = await (await h!.getFile()).text();
    fsaStatus.value = `${h!.name} · ${await h!.queryPermission({ mode: "readwrite" })}`;
  } catch (e) {
    fsaStatus.value = describe(e);
  }
}

async function writeTo(h: SkFileSystemFileHandle) {
  const w = await h.createWritable();
  await w.write(text.value);
  await w.close();
}

async function save() {
  if (!handle.value) return;
  try {
    await writeTo(handle.value);
    fsaStatus.value = `saved ${handle.value.name}`;
  } catch (e) {
    fsaStatus.value = describe(e);
  }
}

async function saveAs() {
  try {
    const h = await showSaveFilePicker({
      id: "hello-files",
      suggestedName: handle.value?.name ?? "notes.txt",
      types: TEXT_TYPES,
    });
    await writeTo(h);
    handle.value = h;
    fsaStatus.value = `saved as ${h.name}`;
  } catch (e) {
    fsaStatus.value = describe(e);
  }
}

const folder = ref("");
const entries = ref<string[]>([]);
async function openFolder() {
  try {
    const dir = await showDirectoryPicker({ id: "hello-folder" });
    folder.value = dir.name;
    const out: string[] = [];
    for await (const [name, h] of dir.entries()) {
      out.push(h.kind === "directory" ? `${name}/` : name);
      if (out.length >= 40) break;
    }
    entries.value = out.sort();
  } catch (e) {
    folder.value = describe(e);
    entries.value = [];
  }
}

const notes = computed(() => text.value || "Saved from the Skathi hello app.\n");
const blobUrl = ref("");
const refreshUrl = () => {
  if (blobUrl.value) URL.revokeObjectURL(blobUrl.value);
  blobUrl.value = URL.createObjectURL(new Blob([notes.value], { type: "text/plain" }));
};
refreshUrl();
watch(notes, refreshUrl);
onBeforeUnmount(() => URL.revokeObjectURL(blobUrl.value));
</script>

<template>
  <div class="page">
    <div class="hd">
      <span class="h1">Files</span>
      <span class="sub">pickers, File System Access, FileReader, downloads</span>
    </div>

    <section class="card">
      <div class="cap">
        <span>&lt;input type=file&gt;</span>
        <span class="tier t1">tier 1 · HTML</span>
      </div>
      <div class="row">
        <input id="fi-input" class="file-in" type="file" multiple @change="onPick" @cancel="onCancel" />
        <label class="btn" for="fi-input">or click this &lt;label&gt;</label>
      </div>
      <div class="grid" v-if="picked.length">
        <div class="prow" v-for="f in picked" :key="f.name + f.rel">
          <span class="k">{{ f.rel || f.name }}</span>
          <span class="v">{{ size(f.size) }}</span>
          <span class="v">{{ f.type || "(no type)" }}</span>
        </div>
      </div>
      <div class="pre" v-if="preview">{{ preview }}</div>
      <div class="note">events: {{ inputLog.join(" · ") || "none yet" }}</div>
    </section>

    <section class="card">
      <div class="cap">
        <span>a file, edited in place</span>
        <span class="tier t1">tier 1 · File System Access</span>
      </div>
      <div class="row">
        <span class="btn" id="fi-open" @click="openFile">Open…</span>
        <span class="btn" id="fi-save" :class="{ off: !handle }" @click="save">Save</span>
        <span class="btn" id="fi-save-as" @click="saveAs">Save as…</span>
        <span class="pill" id="fi-status">{{ fsaStatus || "nothing open" }}</span>
      </div>
      <textarea class="ta" v-model="text" placeholder="open a text file, or type here and Save as…"></textarea>
    </section>

    <section class="card">
      <div class="cap">
        <span>a folder</span>
        <span class="tier t1">tier 1 · showDirectoryPicker</span>
      </div>
      <div class="row">
        <span class="btn" id="fi-folder" @click="openFolder">Choose a folder…</span>
        <span class="pill">{{ folder || "none" }}</span>
      </div>
      <div class="note" v-if="entries.length">{{ entries.join("  ") }}</div>
    </section>

    <section class="card">
      <div class="cap">
        <span>a download</span>
        <span class="tier t1">tier 1 · &lt;a download&gt;</span>
      </div>
      <div class="row">
        <a class="btn" id="fi-download" :href="blobUrl" download="hello-notes.txt">
          Download the text as hello-notes.txt
        </a>
      </div>
      <div class="note">
        A browser saves to Downloads without asking (unless "Ask where to save each file" is on); here it is a Save As
        that starts in Downloads, and the host says so once (D-F6).
      </div>
    </section>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.hd {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
}
.h1 {
  font-size: 16px;
  color: #ffffff;
}
.sub {
  font-size: 11px;
  color: #ffffff70;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-radius: 10px;
  background: #ffffff0d;
}
.cap {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: #ffffff80;
}
.tier {
  padding: 2px;
  border-radius: 4px;
  font-size: 10px;
}
.t1 {
  background: #4f8cff40;
  color: #b9d0ff;
}
.row {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
}
.grid {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.prow {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding: 4px;
  border-radius: 6px;
  background: #ffffff08;
}
.k {
  width: 260px;
  font-size: 12px;
  color: #ffffffcc;
}
.v {
  font-size: 11px;
  color: #ffffff99;
}
.file-in {
  font-size: 12px;
  color: #ffffffcc;
}
.file-in::file-selector-button {
  padding: 6px 10px;
  margin-right: 8px;
  border-radius: 6px;
  border: 0;
  background: #4f8cff59;
  color: #ffffff;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  background: #ffffff14;
  font-size: 11px;
  color: #ffffffdd;
  cursor: pointer;
}
.btn:hover {
  background: #ffffff22;
}
.off {
  color: #ffffff55;
}
.pill {
  padding: 2px;
  border-radius: 6px;
  background: #ffffff14;
  font-size: 11px;
  color: #ffffff99;
}
.ta {
  height: 120px;
  padding: 6px;
  border-radius: 6px;
  background: #00000040;
  font-size: 12px;
  color: #ffffff;
}
.pre {
  padding: 6px;
  border-radius: 6px;
  background: #00000040;
  font-size: 11px;
  color: #ffffffbb;
  white-space: pre-wrap;
}
.note {
  font-size: 11px;
  color: #ffffff77;
}
</style>
