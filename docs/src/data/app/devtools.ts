import { OK, all, importLine, section } from "./shared";

const IMPORT = importLine("appDevtools");
const PLATFORMS = all(OK);
const NO_DEVTOOLS =
  'In a build without devtools (`sk publish` without `"publishDevtools": true`), `available` is `false`, `isEnabled()` and `open()` return `false`, and `enable()` returns `false` with an advisory. Code that calls them needs no check.';

export const devtools = section("Devtools", [
  {
    slug: "app-devtools",
    name: "appDevtools",
    kind: "property",
    summary: "Turns the devtools off or back on at run time, opens the inspector from code, and reports their state.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

readonly appDevtools.available: boolean
appDevtools.isEnabled(): boolean
appDevtools.enable(): boolean
appDevtools.disable(): boolean
appDevtools.open(): boolean
appDevtools.info(): DevtoolsInfo
appDevtools.onChange(listener: (enabled: boolean) => void): () => void`,
    notes: [
      "With devtools on, the app listens on a local port for the inspector and F12 opens it. See [Devtools](/devtools/overview/).",
      '`"devtools": false` in `sk.config.json` starts the app with them off; the environment variable `SK_DEVTOOLS=1` turns them on and makes `disable()` refuse.',
    ],
    examples: [
      {
        title: "A Developer menu",
        lang: "ts",
        code: `import { appDevtools } from "skathi:app";

export const developerMenu = () =>
  appDevtools.available
    ? [
        { label: "Open inspector", onSelect: () => appDevtools.open() },
        { label: appDevtools.isEnabled() ? "Turn devtools off" : "Turn devtools on", onSelect: () => (appDevtools.isEnabled() ? appDevtools.disable() : appDevtools.enable()) },
      ]
    : [];`,
      },
    ],
    limitations: [NO_DEVTOOLS],
    related: ["devtools-enable", "devtools-open", "devtools-info", "devtools-on-change"],
    aliases: ["webContents.openDevTools", "inspector", "F12"],
  },
  {
    slug: "devtools-enable",
    name: "appDevtools.enable() / disable()",
    kind: "method",
    summary: "Starts or stops the devtools listener at run time; `isEnabled()` reads the current state.",
    platforms: PLATFORMS,
    syntax: "appDevtools.enable(): boolean\nappDevtools.disable(): boolean\nappDevtools.isEnabled(): boolean",
    returns: "The state afterwards: `true` when the devtools are on.",
    notes: [
      "`enable()` binds a fresh port and token and publishes them for the inspector. Calling it while on changes nothing, so an attached inspector keeps its connection.",
      "`disable()` closes the listener and the inspector connections and deletes the handshake file.",
      "A refused change prints an advisory and returns the unchanged state: `disable()` while `SK_DEVTOOLS=1` holds them on.",
    ],
    examples: [
      {
        title: "Turn the devtools off in production settings",
        lang: "ts",
        code: `import { appDevtools } from "skathi:app";

if (localStorage.getItem("developerMode") !== "1") appDevtools.disable();`,
        note: "In the entry module, before the user can press F12.",
      },
    ],
    limitations: [NO_DEVTOOLS],
    related: ["devtools-on-change", "devtools-info", "app-devtools"],
    aliases: ["isEnabled"],
  },
  {
    slug: "devtools-open",
    name: "appDevtools.open()",
    kind: "method",
    summary: "Opens the inspector on the main window, as F12 does.",
    platforms: PLATFORMS,
    syntax: "appDevtools.open(): boolean",
    returns: "`true` when an inspector was opened or brought to the front.",
    notes: [
      "It goes through the same path as F12: a second call brings the open inspector to the front instead of opening another.",
      "It works when the F12 key is turned off with `SK_DEVTOOLS_KEY=off`.",
    ],
    examples: [
      {
        title: "A Help menu item",
        lang: "vue",
        code: `<script setup lang="ts">
import { appDevtools } from "skathi:app";
</script>

<template>
  <button v-if="appDevtools.available" @click="appDevtools.open()">Open inspector</button>
</template>`,
      },
    ],
    edgeCases: [
      "With the devtools off, it prints an advisory that says to call `enable()` first, and returns `false`.",
    ],
    limitations: [NO_DEVTOOLS],
    related: ["devtools-enable", "app-devtools"],
    aliases: ["openDevTools"],
  },
  {
    slug: "devtools-info",
    name: "appDevtools.info()",
    kind: "method",
    summary:
      "Describes the devtools of this process: on or off, held on, the port, the F12 gate and the attached inspectors.",
    platforms: PLATFORMS,
    syntax: `appDevtools.info(): DevtoolsInfo

interface DevtoolsInfo {
  enabled: boolean;
  locked: boolean; // SK_DEVTOOLS=1 holds them on
  port: number | null;
  pid: number;
  key: boolean; // F12 opens the inspector
  inspectors: number;
  slot: string; // the build folder whose devtools.json this process publishes
}`,
    returns: "A fresh object on each call.",
    notes: ["`port` is `null` while the devtools are off."],
    examples: [
      {
        title: "Print how to attach",
        lang: "ts",
        code: `import { appDevtools } from "skathi:app";

const { enabled, port, pid } = appDevtools.info();
if (enabled) console.log(\`devtools for process \${pid} on 127.0.0.1:\${port}\`);`,
      },
    ],
    limitations: [NO_DEVTOOLS],
    related: ["devtools-enable", "app-devtools"],
    aliases: ["DevtoolsInfo"],
  },
  {
    slug: "devtools-on-change",
    name: "appDevtools.onChange()",
    kind: "method",
    summary: "Calls a listener with the new state when `enable()` or `disable()` turns the devtools on or off.",
    platforms: PLATFORMS,
    syntax: "appDevtools.onChange(listener: (enabled: boolean) => void): () => void",
    returns: "A function that removes the listener.",
    notes: [
      "It runs only on a real change, inside the `enable()` or `disable()` call that made it.",
      "The same function added twice runs once. A listener that throws prints an advisory; the others still run.",
    ],
    examples: [
      {
        title: "Keep a toggle in sync",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { appDevtools } from "skathi:app";

const on = ref(appDevtools.isEnabled());
onUnmounted(appDevtools.onChange((enabled) => (on.value = enabled)));
</script>

<template>
  <label>
    <input type="checkbox" :checked="on" @change="on ? appDevtools.disable() : appDevtools.enable()" />
    Devtools
  </label>
</template>`,
      },
    ],
    edgeCases: ["Not a function throws a `TypeError`."],
    related: ["devtools-enable", "app-devtools"],
  },
]);
