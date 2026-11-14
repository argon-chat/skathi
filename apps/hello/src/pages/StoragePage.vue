<script setup lang="ts">
// Both are incremented once per mount; `localStorage` remembers across a restart of the process
// and `sessionStorage` does not.
import { onMounted, ref } from "vue";
import { appStorage } from "@skathi-native/runtime";

defineOptions({ name: "StoragePage" });

const BOOTS = "sk.demo.boots";
const NOTE = "sk.demo.note";

type StorageInfo = ReturnType<typeof appStorage.info>;

// Starts empty on purpose — `refresh()` fills it in `onMounted`, and the template reads through
// `info.folder` etc.
const info = ref<Partial<StorageInfo>>({});
const localBoots = ref(0);
const sessionBoots = ref(0);
const note = ref("");
const status = ref("");
const clipText = ref("copied from Skathi");
const clipRead = ref("");

function refresh() {
  info.value = appStorage.info();
}

onMounted(() => {
  localBoots.value = Number(localStorage.getItem(BOOTS) || 0) + 1;
  localStorage.setItem(BOOTS, String(localBoots.value));
  sessionBoots.value = Number(sessionStorage.getItem(BOOTS) || 0) + 1;
  sessionStorage.setItem(BOOTS, String(sessionBoots.value));
  note.value = localStorage.getItem(NOTE) || "";
  refresh();
});

function saveNote() {
  localStorage.setItem(NOTE, note.value);
  const wrote = appStorage.flush();
  status.value = wrote ? "written to disk" : "no change to write";
  refresh();
}

function clearLocal() {
  localStorage.clear();
  appStorage.flush();
  localBoots.value = 0;
  note.value = "";
  status.value = "localStorage cleared — restart and the boot count starts at 1";
  refresh();
}

function moveTo(kind: string, sub: string, encrypted: boolean) {
  try {
    const folder = appStorage.getUserFolder(kind, sub);
    const at = appStorage.setStorageFolder(folder, { encrypted });
    status.value = `storage is now ${at}${encrypted ? " (encrypted)" : ""}`;
  } catch (e) {
    status.value = e instanceof Error ? e.message : String(e);
  }
  refresh();
}

// A `storage` listener is the one thing on this page that does NOT work like a browser, so the
// page says so rather than leaving a dead handler behind.
function listenForStorage() {
  addEventListener("storage", () => {});
  status.value =
    "listener added — and it will never fire: `storage` tells one browsing " +
    "context that ANOTHER wrote a key, and there is only one here.";
}

const describe = (e: unknown) => (e instanceof Error ? `${e.name}: ${e.message}` : String(e));

const nav = navigator;

async function clipboardRoundTrip() {
  try {
    await nav.clipboard.writeText(clipText.value);
    clipRead.value = await nav.clipboard.readText();
  } catch (e) {
    clipRead.value = describe(e);
  }
}

async function clipboardImage() {
  try {
    await nav.clipboard.read();
    clipRead.value = "read() resolved?!";
  } catch (e) {
    clipRead.value = describe(e);
  }
}

const execCommandResult = ref("");
function tryExecCommand() {
  execCommandResult.value = String(document.execCommand("copy"));
}
</script>

<template>
  <div class="page">
    <div class="hd">
      <span class="h1">Storage</span>
      <span class="sub">Web Storage on a disk, and the clipboard</span>
      <span class="pill live" id="st-encrypted">{{ info.encrypted ? "encrypted" : "plaintext" }}</span>
    </div>

    <section class="card">
      <div class="cap">
        <span>persistence</span>
        <span class="tier t1">tier 1 · localStorage / sessionStorage</span>
      </div>
      <div class="grid">
        <div class="prow">
          <span class="k">localStorage boots</span>
          <span class="s ok" id="st-local-boots">{{ localBoots }}</span>
          <span class="note">survives a restart of the whole process</span>
        </div>
        <div class="prow">
          <span class="k">sessionStorage boots</span>
          <span class="s unk" id="st-session-boots">{{ sessionBoots }}</span>
          <span class="note">always 1 — per run, never written anywhere</span>
        </div>
      </div>
      <div class="row">
        <input class="in" id="st-note" v-model="note" placeholder="a note to remember" />
        <span class="btn" id="st-save" @click="saveNote">Save + flush</span>
        <span class="btn" id="st-clear" @click="clearLocal">Clear localStorage</span>
      </div>
      <div class="note" id="st-status">{{ status || "nothing done yet" }}</div>
    </section>

    <section class="card">
      <div class="cap">
        <span>where the bytes are</span>
        <span class="tier t3">tier 2 · appStorage</span>
      </div>
      <div class="grid" id="st-info">
        <div class="prow">
          <span class="k">folder</span><span class="v">{{ info.folder }}</span>
        </div>
        <div class="prow">
          <span class="k">file</span><span class="v">{{ info.file }}</span>
        </div>
        <div class="prow">
          <span class="k">bytes</span><span class="v" id="st-bytes">{{ info.bytes }}</span>
        </div>
        <div class="prow">
          <span class="k">provider</span><span class="v">{{ info.provider || "none" }}</span>
        </div>
        <div class="prow" v-if="info.error">
          <span class="k">error</span><span class="v no">{{ info.error }}</span>
        </div>
      </div>
      <div class="row">
        <span class="btn" id="st-move-local" @click="moveTo('LOCALAPPDATA', 'hello/storage', false)">
          LOCALAPPDATA (plain)
        </span>
        <span class="btn" id="st-move-sealed" @click="moveTo('LOCALAPPDATA', 'hello/storage', true)">
          LOCALAPPDATA (encrypted)
        </span>
        <span class="btn" id="st-move-roaming" @click="moveTo('APPDATA', 'hello/storage', false)">
          APPDATA (roaming)
        </span>
      </div>
      <div class="note">
        Encryption is DPAPI, keyed by the Windows user — the key never exists in this process. Turning it on migrates
        the plaintext area and deletes the plaintext copy; turning it off does the reverse. It protects the file from
        other users and from being copied off this machine, and not from anything running as you.
      </div>
    </section>

    <section class="card">
      <div class="cap">
        <span>clipboard</span>
        <span class="tier t1">tier 1 · navigator.clipboard</span>
      </div>
      <div class="row">
        <input class="in" id="st-clip-text" v-model="clipText" />
        <span class="btn" id="st-clip-go" @click="clipboardRoundTrip">writeText → readText</span>
        <span class="btn" id="st-clip-img" @click="clipboardImage">read() (images)</span>
      </div>
      <div class="note" id="st-clip-out">{{ clipRead || "nothing read yet" }}</div>
      <div class="row">
        <span class="btn" id="st-exec" @click="tryExecCommand">document.execCommand("copy")</span>
        <span class="pill" id="st-exec-out">{{ execCommandResult || "—" }}</span>
      </div>
      <div class="note">
        The same OS clipboard the native &lt;input&gt; uses for Ctrl+C — one owner, not two. `read()`/`write()` with
        images are not implemented in v1, so they reject and `ClipboardItem` is deliberately undefined; `execCommand` is
        inert and returns false, as a browser does when a command cannot be executed.
      </div>
    </section>

    <section class="card">
      <div class="cap">the one divergence worth clicking</div>
      <div class="row">
        <span class="btn" id="st-listen" @click="listenForStorage">addEventListener("storage")</span>
      </div>
      <div class="note">
        In a browser `storage` fires on OTHER tabs of the same origin. There is one context here, so nothing fires it —
        and the host says so on stderr the first time an app adds the listener, rather than leaving a handler that
        quietly never runs. `new StorageEvent("storage", {...})` plus `dispatchEvent` still works if you want to notify
        your own components.
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
.t3 {
  background: #ffb04f40;
  color: #ffd9a8;
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
.row {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
}
.k {
  width: 160px;
  font-size: 12px;
  color: #ffffffcc;
}
.v {
  font-size: 11px;
  color: #ffffff99;
}
.s {
  width: 72px;
  padding: 2px;
  border-radius: 4px;
  font-size: 12px;
  text-align: center;
}
.ok {
  background: #35d07f30;
  color: #9ff0c4;
}
.no {
  background: #ff5c5c30;
  color: #ffb3b3;
}
.unk {
  background: #ffffff14;
  color: #ffffff88;
}
.pill {
  padding: 2px;
  border-radius: 6px;
  background: #ffffff14;
  font-size: 11px;
  color: #ffffff99;
}
.live {
  background: #4f8cff30;
  color: #b9d0ff;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  background: #ffffff14;
  font-size: 11px;
  color: #ffffffdd;
}
.btn:hover {
  background: #ffffff22;
}
.in {
  width: 260px;
  padding: 6px;
  border-radius: 6px;
  background: #00000040;
  font-size: 12px;
  color: #ffffff;
}
.note {
  font-size: 11px;
  color: #ffffff77;
}
</style>
