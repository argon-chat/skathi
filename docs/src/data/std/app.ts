import { importLine, section } from "./shared";

const IMPORT = importLine("skathi:app", "app");
const EMBEDDED_RESTART =
  "In an app that runs inside another program, `restart()` throws an `Error` named `NotSupportedError`: the process is the embedder's. Ask the embedder to restart through a [native module](/embedding/native-modules/).";

export const app = section("skathi:app", [
  {
    slug: "app",
    name: "app",
    kind: "object",
    summary: "Ends the app, starts it again, and reads its version.",
    syntax: `${IMPORT}

app.quit(options?: { exitCode?: number }): void
app.restart(options?: { args?: string[] }): void
app.getVersion(): string`,
    notes: [
      "The module also exports every [App API](/app-api/) object (`appTray`, `appShell`, `appWindow` and the rest). Import them from `skathi:app`. Importing them from `@skathi-native/runtime` still works but is deprecated.",
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
    related: ["app-quit", "app-restart", "app-get-version", "runtime"],
    aliases: ["skathi:app", "App", "exit", "relaunch", "app version"],
  },
  {
    slug: "app-quit",
    name: "app.quit()",
    kind: "function",
    summary: "Ends the app with an exit code, the way `appWindow.close()` does.",
    syntax: "app.quit(options?: { exitCode?: number }): void",
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
    syntax: "app.restart(options?: { args?: string[] }): void",
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
    syntax: "app.getVersion(): string",
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
]);
