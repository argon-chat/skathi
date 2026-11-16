import { OK, all, importLine, section } from "./shared";

const IMPORT = importLine("appStorage");
const PLATFORMS = all(OK);
const PROVIDERS =
  "Encryption at rest uses the platform's key store: DPAPI for the signed-in user on Windows, a key in the login Keychain on macOS, and the Secret Service on Linux, or a key file only the user can read where no Secret Service runs.";

export const storage = section("Storage", [
  {
    slug: "app-storage",
    name: "appStorage",
    kind: "property",
    summary:
      "Where `localStorage` and IndexedDB keep their data, whether it is encrypted at rest, and the user's standard folders.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

readonly appStorage.folder: string
readonly appStorage.encrypted: boolean
readonly appStorage.kinds: string[]
appStorage.getUserFolder(kind: string, subfolder?: string): string
appStorage.setStorageFolder(path: string, options?: { encrypted?: boolean }): string
appStorage.info(): StorageInfo
appStorage.flush(): boolean`,
    notes: [
      "`localStorage` persists to one file in the storage folder, `localStorage.json`, or `localStorage.sealed` when encrypted. IndexedDB keeps its databases in the `indexeddb` folder beside it.",
      "The default folder is `<app name>/storage` under the local app data folder: `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, `$XDG_DATA_HOME` or `~/.local/share` on Linux. The name is `SK_APP_NAME` when set, else the `name` in `sk.config.json`, else the app's folder or executable name, with characters other than letters, digits, `-`, `_` and `.` turned into `-`.",
      "Every call is synchronous and does no waiting on the network.",
      "Writes are coalesced to at most one per frame, and the last one is written when the app quits. A failed write is retried at most once a second.",
      "A program that embeds Skathi can choose the folder and the encryption before the app starts (`sk_app::embed::Builder::data_dir` and `storage_encrypted`).",
      "A Svelte app imports it from `@skathi-native/runtime/svelte`.",
    ],
    examples: [
      {
        title: "Encrypt the app's data from the first run",
        lang: "ts",
        code: `import { appStorage, createApp } from "@skathi-native/runtime";
import App from "./App.vue";

if (!appStorage.encrypted) appStorage.setStorageFolder(appStorage.folder, { encrypted: true });

createApp(App).mount();`,
        note: "In the entry module, before anything reads `localStorage` or opens IndexedDB. An existing plaintext area is migrated in place.",
      },
    ],
    edgeCases: [
      "Two processes on one folder are last-writer-wins. Give concurrent development copies different `SK_APP_NAME` values.",
    ],
    limitations: [PROVIDERS],
    related: ["storage-set-storage-folder", "storage-get-user-folder", "storage-info", "storage-flush"],
    aliases: ["app.getPath", "userData", "storage folder", "encryption at rest", "safeStorage"],
  },
  {
    slug: "storage-folder",
    name: "appStorage.folder",
    kind: "property",
    summary: "The absolute path of the folder `localStorage` writes to now.",
    platforms: PLATFORMS,
    syntax: "readonly appStorage.folder: string",
    notes: [
      "Before any `setStorageFolder()` call, it is the default folder, also when nothing has been written yet and the folder does not exist.",
      "After `setStorageFolder()`, it is the canonical path that call returned.",
    ],
    examples: [
      {
        title: "Show where the data lives",
        lang: "vue",
        code: `<script setup lang="ts">
import { appShell, appStorage } from "@skathi-native/runtime";
</script>

<template>
  <p>
    Data folder: <code>{{ appStorage.folder }}</code>
    <button @click="appShell.openPath(appStorage.folder)">Open</button>
  </p>
</template>`,
      },
    ],
    related: ["storage-encrypted", "storage-set-storage-folder", "storage-info"],
    aliases: ["userData"],
  },
  {
    slug: "storage-encrypted",
    name: "appStorage.encrypted",
    kind: "property",
    summary: "Whether `localStorage` is written encrypted at rest.",
    platforms: PLATFORMS,
    syntax: "readonly appStorage.encrypted: boolean",
    notes: [
      "Encryption is sticky: a folder that already holds an encrypted area stays encrypted until `setStorageFolder(path, { encrypted: false })` asks for plaintext. It reads `true` once such a folder has been loaded.",
      "The HTTP cache follows the same setting, and so does IndexedDB when it is set before IndexedDB's first use.",
    ],
    examples: [
      {
        title: "A privacy setting",
        lang: "ts",
        code: `import { appStorage } from "@skathi-native/runtime";

export function setEncrypted(on: boolean): boolean {
  if (appStorage.encrypted !== on) appStorage.setStorageFolder(appStorage.folder, { encrypted: on });
  return appStorage.encrypted;
}`,
        note: "Turning it off rewrites `localStorage` as plaintext. IndexedDB keeps the format it started with until the next launch.",
      },
    ],
    limitations: [PROVIDERS],
    related: ["storage-set-storage-folder", "storage-info"],
    aliases: ["encryption at rest", "DPAPI", "Keychain"],
  },
  {
    slug: "storage-get-user-folder",
    name: "appStorage.getUserFolder()",
    kind: "method",
    summary: "Returns the absolute path of one of the user's standard folders, optionally with a subfolder joined.",
    platforms: PLATFORMS,
    syntax: `appStorage.getUserFolder(kind: string, subfolder?: string): string

readonly appStorage.kinds: string[] // ["APPDATA", "LOCALAPPDATA", "TEMP", "DOCUMENTS", "HOME"]`,
    params: [
      {
        name: "kind",
        type: "string",
        description:
          "One of `appStorage.kinds`: `APPDATA`, `LOCALAPPDATA`, `TEMP`, `DOCUMENTS` or `HOME`. Case does not matter.",
      },
      {
        name: "subfolder",
        type: "string",
        description: "A relative path joined to it; `/` and `\\` both separate.",
        optional: true,
      },
    ],
    returns: "The path. It creates nothing and does not check that it exists.",
    values: [
      {
        value: "APPDATA",
        status: OK,
        note: "Windows: the roaming `AppData\\Roaming`. macOS: `~/Library/Application Support`. Linux: `$XDG_DATA_HOME` or `~/.local/share`.",
      },
      {
        value: "LOCALAPPDATA",
        status: OK,
        note: "Windows: `AppData\\Local`, never synced. macOS and Linux: the same folder as `APPDATA`.",
      },
      { value: "TEMP", status: OK, note: "The temporary folder." },
      {
        value: "DOCUMENTS",
        status: OK,
        note: "Windows: the Documents known folder, also when redirected. macOS and Linux: `~/Documents`.",
      },
      { value: "HOME", status: OK, note: "The user's profile or home folder." },
    ],
    notes: ["On Windows the paths come from the shell's known folders, not from environment variables."],
    examples: [
      {
        title: "Export into Documents",
        lang: "ts",
        code: `import { appShell, appStorage } from "@skathi-native/runtime";

export async function revealExport(fileName: string): Promise<void> {
  const folder = appStorage.getUserFolder("DOCUMENTS", "Notes exports");
  await appShell.showItemInFolder(\`\${folder}/\${fileName}\`);
}`,
      },
    ],
    edgeCases: [
      "An unknown kind (`ROAMING`, `APP_DATA`) throws a `TypeError` that lists the valid ones.",
      "A subfolder with `..`, a drive or stream name (`C:`, `a:b`), or nothing but separators throws a `TypeError`: it names a place under the known folder, never a path elsewhere.",
    ],
    related: ["storage-folder", "shell-open-path"],
    aliases: ["app.getPath", "known folder", "APPDATA", "Documents", "kinds"],
  },
  {
    slug: "storage-set-storage-folder",
    name: "appStorage.setStorageFolder()",
    kind: "method",
    summary:
      "Moves `localStorage` (and IndexedDB, before its first use) to another folder, and turns encryption at rest on or off.",
    platforms: PLATFORMS,
    syntax: "appStorage.setStorageFolder(path: string, options?: { encrypted?: boolean }): string",
    params: [
      {
        name: "path",
        type: "string",
        description: "The folder. It is created when missing; a relative path is taken from the working directory.",
      },
      {
        name: "options.encrypted",
        type: "boolean",
        description: "`true` encrypts, `false` writes plaintext. Left out, the current format is kept.",
        optional: true,
      },
    ],
    returns: "The canonical absolute path of the folder.",
    notes: [
      "If `localStorage` has been used, its current contents are written to the new folder at once, and the old folder's file is left where it was. If not, the new folder's own data loads on first use.",
      "Changing only the encryption in the same folder rewrites the area in the new format and deletes the old file.",
      "Without `encrypted`, a folder that already holds an encrypted area stays encrypted: a folder change never decrypts silently.",
      "IndexedDB takes its folder and encryption at its first use and keeps them for the run, so call this before the first `indexedDB` call.",
      "The HTTP cache follows `encrypted` but stays in its own folder.",
    ],
    examples: [
      {
        title: "Keep the data in the folder the app was started from",
        lang: "ts",
        code: `import { appLaunch, appStorage } from "@skathi-native/runtime";

if (appLaunch.argv.includes("--portable")) {
  appStorage.setStorageFolder(\`\${appLaunch.cwd}/data\`);
}`,
      },
    ],
    edgeCases: [
      "An empty path, a folder that cannot be created, or a write that fails throws a `TypeError` starting with `appStorage.setStorageFolder:`.",
      "`{ encrypted: true }` on a platform with no key store throws instead of writing plaintext. A key store that is locked or refuses access is an error for the rest of the run; Skathi never makes a new key that would orphan the data sealed with the old one.",
    ],
    limitations: [PROVIDERS],
    related: ["storage-folder", "storage-encrypted", "storage-info"],
    aliases: ["app.setPath", "move storage", "portable"],
  },
  {
    slug: "storage-info",
    name: "appStorage.info()",
    kind: "method",
    summary:
      "Describes the storage on disk: folder, file, size, encryption provider, IndexedDB and cookie jar state, and the last error.",
    platforms: PLATFORMS,
    syntax: `appStorage.info(): StorageInfo

interface StorageInfo {
  folder: string;
  file: string; // localStorage.json or localStorage.sealed
  encrypted: boolean;
  bytes: number; // size of the file on disk
  loaded: boolean; // localStorage has been read
  pending: boolean; // a write is waiting
  provider: string; // the key store's name, "" without one
  kinds: string[];
  error: string | null;
  indexedDB: IndexedDbInfo | null;
  cookies?: CookieJarInfo;
}`,
    returns: "A fresh object on each call.",
    notes: [
      "`error` is the last read or write failure of `localStorage`, cleared by the next successful write. An unreadable file starts an empty area and says why here.",
      "`indexedDB.mode` is `null` until the first `indexedDB` call, then one of `plaintext`, `encrypted`, `decrypting` or `refused`; `indexedDB` is `null` in a build without on-disk IndexedDB.",
      "`cookies` describes the cookie jar, which is always sealed and never stores session cookies.",
    ],
    examples: [
      {
        title: "A diagnostics panel",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { appStorage } from "@skathi-native/runtime";

const info = ref(appStorage.info());
</script>

<template>
  <dl>
    <dt>File</dt>
    <dd>{{ info.file }} ({{ info.bytes }} bytes{{ info.encrypted ? ", " + info.provider : "" }})</dd>
    <dt>IndexedDB</dt>
    <dd>{{ info.indexedDB?.mode ?? "not used yet" }}</dd>
    <dd v-if="info.error" class="error">{{ info.error }}</dd>
  </dl>
  <button @click="info = appStorage.info()">Refresh</button>
</template>`,
      },
    ],
    related: ["app-storage", "storage-flush"],
    aliases: ["StorageInfo", "IndexedDbInfo", "CookieJarInfo"],
  },
  {
    slug: "storage-flush",
    name: "appStorage.flush()",
    kind: "method",
    summary: "Writes a pending `localStorage` change to disk now instead of at the end of the frame.",
    platforms: PLATFORMS,
    syntax: "appStorage.flush(): boolean",
    returns: "`true` when it wrote, `false` when nothing was pending.",
    notes: [
      "Quitting writes the last change by itself; call it when another process is about to read the file, or before something that may end the process abruptly.",
    ],
    examples: [
      {
        title: "Save before handing off to an updater",
        lang: "ts",
        code: `import { appShell, appStorage } from "@skathi-native/runtime";

export async function runInstaller(path: string): Promise<void> {
  localStorage.setItem("lastVersionPrompt", new Date().toISOString());
  appStorage.flush();
  await appShell.openPath(path, { allowExecutable: true });
}`,
      },
    ],
    edgeCases: [
      "A write that fails throws a `TypeError` starting with `appStorage.flush:`, and the change stays pending.",
    ],
    related: ["storage-info", "app-storage"],
  },
]);
