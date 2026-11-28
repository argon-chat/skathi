import { bindings } from "../../../scripts/bindings-reference";
import { appReference } from "../../lib/app";
import { entryUrl } from "../../lib/reference";
import { windowReference } from "../../lib/window";
import { section, type Draft } from "./shared";

/** The public desktop objects each link to their App API or Window API page. */
const exported = [...bindings().std.get("skathi:app")!.keys()].filter((name) => /^app[A-Z]\w*$/.test(name));

function exportRow(name: string): Draft[] {
  for (const reference of [appReference, windowReference]) {
    const entry = reference.entries.find((e) => e.name === name);
    if (!entry) continue;
    return [
      {
        slug: entry.slug,
        name,
        kind: "object",
        summary: entry.summary,
        platforms: entry.platforms,
        ...(entry.since ? { since: entry.since } : {}),
        ...(entry.platformSince ? { platformSince: entry.platformSince } : {}),
        link: { url: entryUrl(reference, entry.slug), title: reference.title },
      },
    ];
  }
  return [];
}
const EMBEDDED_RESTART =
  "In an app that runs inside another program, `restart()` throws an `Error` named `NotSupportedError`: the process is the embedder's. Ask the embedder to restart through a [native module](/embedding/native-modules/).";

export const app = section("skathi:app", [
  {
    slug: "app",
    name: "app",
    kind: "object",
    summary: "Ends the app, starts it again, reads its version, and sets how far the wheel scrolls.",
    declaration: true,
    notes: [
      "The module also exports the `app*` objects listed with it here; each has its page in the [App API](/app-api/) or [Window API](/window-api/) reference. Importing them from `@skathi-native/runtime` still works but is deprecated.",
      "`quit()` and `restart()` end the app the way [`appWindow.close()`](/window-api/) does. See [Closing, hiding and quitting](/guides/closing-and-background/).",
      "`getVersion()` is the app's own version. The version of Skathi is [`runtime.version`](/std/runtime/).",
      "The three methods work on Windows, macOS and Linux. In an embedded app `quit()` tells the embedder the app closed, `restart()` throws, and `getVersion()` reads the embedder's payload.",
    ],
    examples: [
      {
        title: "Show the version and offer a restart",
        lang: "vue",
        code: `<script setup lang="ts">
import { app } from "skathi:app";

const version = app.getVersion();
</script>

<template>
  <footer>
    <span>Version {{ version }}</span>
    <button @click="app.restart()">Restart</button>
    <button @click="app.quit()">Quit</button>
  </footer>
</template>`,
      },
    ],
    related: ["app-quit", "app-restart", "app-get-version", "app-wheel-scroll-scale", "runtime"],
    aliases: ["skathi:app", "App", "exit", "relaunch", "app version"],
  },
  {
    slug: "app-quit",
    name: "app.quit()",
    kind: "function",
    summary: "Ends the app with an exit code, the way `appWindow.close()` does.",
    declaration: true,
    params: [
      {
        name: "options.exitCode",
        type: "number",
        description: "The process's exit code, an integer from 0 to 255.",
        optional: true,
        default: "0",
      },
    ],
    returns: "Nothing. The call returns and the app closes behind it.",
    notes: [
      "`pagehide` fires once. `close-requested` and `beforeunload` are not asked.",
      "Every window hides at once and the teardown runs behind them, under the same deadline as [`appWindow.close()`](/window-api/). See [Closing, hiding and quitting](/guides/closing-and-background/).",
      "In an embedded app, `quit()` tells the embedder the app closed. `Exit::exit_code` carries the code.",
    ],
    examples: [
      {
        title: "Exit with a code when startup fails",
        lang: "ts",
        code: `import { app } from "skathi:app";

declare function migrate(): Promise<boolean>;

if (!(await migrate())) app.quit({ exitCode: 3 });`,
      },
      {
        title: "Save before the app ends",
        lang: "ts",
        code: `import { app } from "skathi:app";

declare function saveDraft(): void;

addEventListener("pagehide", () => saveDraft());

export function quitFromMenu(): void {
  app.quit();
}`,
      },
    ],
    edgeCases: [
      "An `exitCode` that is not an integer from 0 to 255 (`1.5`, `-1`, `256`, a string) throws a `TypeError` and the app keeps running.",
      "Calling `quit()` again while the app is closing does nothing.",
    ],
    related: ["app", "app-restart"],
    aliases: ["app.quit", "exit", "exit code", "process.exit", "close app"],
  },
  {
    slug: "app-restart",
    name: "app.restart()",
    kind: "function",
    summary: "Ends the app as `quit()` does and starts it again, with the same arguments or with new ones.",
    declaration: true,
    params: [
      {
        name: "options.args",
        type: "string[]",
        description: "The command-line arguments of the new process.",
        optional: true,
        default: "the arguments this process was started with",
      },
    ],
    returns: "Nothing. The call returns and the app closes behind it.",
    notes: [
      "The new process is started before the teardown begins.",
      "A single-instance app restarts correctly: the new process waits for this one to end.",
      "On Windows the same executable is started. On macOS it is the executable in the bundle. On Linux it is the AppImage (`$APPIMAGE`), else the executable.",
      "If the new process cannot be started, the app still closes. The host log gets a `[shutdown] app.restart` line with the reason.",
    ],
    limitations: [EMBEDDED_RESTART],
    examples: [
      {
        title: "Restart to apply a setting",
        lang: "ts",
        code: `import { app } from "skathi:app";

export function applyGpuSetting(enabled: boolean): void {
  localStorage.setItem("gpu", String(enabled));
  app.restart();
}`,
      },
      {
        title: "Restart with other arguments",
        lang: "ts",
        code: `import { app } from "skathi:app";

export function restartInSafeMode(): void {
  app.restart({ args: ["--safe-mode"] });
}`,
      },
    ],
    edgeCases: [
      "`args` that is not an array of strings throws a `TypeError` and the app keeps running.",
      EMBEDDED_RESTART,
      "Calling `restart()` while the app is closing does nothing: no second process is started.",
    ],
    related: ["app", "app-quit", "runtime-is-single-file"],
    aliases: ["app.restart", "relaunch", "restart app", "NotSupportedError"],
  },
  {
    slug: "app-get-version",
    name: "app.getVersion()",
    kind: "function",
    summary: "Returns the app's own version string.",
    declaration: true,
    returns: '`"version"` from `sk.config.json`, else the one in `package.json`, else `"0.0.0"`.',
    notes: [
      "It is the string `sk build` stamps into the executable's version resource and the macOS bundle.",
      "In an embedded app it is the version in the embedder's payload.",
      "The version of Skathi itself is [`runtime.version`](/std/runtime/).",
    ],
    examples: [
      {
        title: "Send the version with a bug report",
        lang: "ts",
        code: `import { app } from "skathi:app";

export async function sendReport(text: string): Promise<void> {
  await fetch("https://example.com/reports", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text, version: app.getVersion() }),
  });
}`,
      },
    ],
    related: ["app", "runtime"],
    aliases: ["app.getVersion", "app version", "version"],
  },
  {
    slug: "app-wheel-scroll-scale",
    name: "app.wheelScrollScale",
    kind: "object",
    summary: "How far a mouse-wheel notch scrolls the app, as a multiple of a browser's distance.",
    declaration: true,
    returns:
      'The scale now in use: `"input": { "wheelScrollScale" }` from `sk.config.json` at start, else `1`, until the app sets another.',
    notes: [
      "At `1` a notch scrolls what it does in a browser: 100 px at Windows' default of 3 lines to scroll, 40 px per line on macOS, 120 px on Linux.",
      "It scales scroll containers, `<textarea>` and `<sk-editor>`, and takes effect at the next notch, in every window.",
      "`WheelEvent.deltaX` and `deltaY` are not scaled: a `wheel` listener sees a browser's deltas whatever the scale. Touchpad scrolling, already in pixels, is not scaled either.",
      "A value is kept between `0.1` and `10`.",
    ],
    examples: [
      {
        title: "A scroll speed slider",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { app } from "skathi:app";

const speed = ref(app.wheelScrollScale);

function onInput() {
  app.wheelScrollScale = speed.value;
}
</script>

<template>
  <label>
    Scroll speed
    <input type="range" min="0.5" max="3" step="0.25" v-model.number="speed" @input="onInput" />
  </label>
</template>`,
      },
    ],
    edgeCases: [
      "A value that is not a finite number above 0 (`0`, `-1`, `NaN`, `Infinity`, a string) throws a `TypeError` and the scale stays as it was.",
      "A value above `10` is stored as `10`, one below `0.1` as `0.1`; reading it back gives the stored value.",
    ],
    related: ["app"],
    aliases: ["wheelScrollScale", "scroll speed", "wheel speed", "mouse wheel"],
  },
  ...exported.flatMap(exportRow),
]);
