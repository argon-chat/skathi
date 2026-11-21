import { importLine, section } from "./shared";

const IMPORT = importLine("skathi:runtime", "Runtime", "runtime");

export const runtime = section("skathi:runtime", [
  {
    slug: "runtime",
    name: "runtime",
    kind: "object",
    summary: "The Skathi version of the host and the optional parts it was built with.",
    syntax: `${IMPORT}

readonly runtime.version: string
readonly runtime.features: ReadonlySet<string>

Runtime.isAot(): boolean
Runtime.isSingleFile(): boolean`,
    notes: [
      '`runtime.version` is the version of Skathi the host was built from, for example `"0.7.0"`. It is not the app\'s own version, which is [`app.getVersion()`](/std/app-get-version/).',
      "`runtime.features` names the optional parts this host has: `audio`, `spatial-audio`, `storages`, `clipboard`, `indexeddb`, `webgpu`, `lottie`, `video`, `hotkeys`, `webview`, `battery`, `devtools`, `native-aot`, `no-jit`, `no-wasm`. A name is in the set only when the host was built with that part.",
      "An embedder chooses the features ([Features](/embedding/features/)). The hosts `sk build` uses have the default set.",
      "Use `features` for diagnostics and bug reports. To decide whether to call an API, test the API itself (`typeof localStorage`, `appHotkeys.available`), as on the web.",
      "`Runtime` is the class of `runtime`. It has no constructor to call; it carries the static methods [`Runtime.isAot()`](/std/runtime-is-aot/) and [`Runtime.isSingleFile()`](/std/runtime-is-single-file/).",
    ],
    examples: [
      {
        title: "Collect a diagnostics report",
        lang: "ts",
        code: `import { app } from "skathi:app";
import { os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";

export const report = {
  app: app.getVersion(),
  skathi: runtime.version,
  features: [...runtime.features],
  aot: Runtime.isAot(),
  os: \`\${os.platform} \${os.release} \${os.arch}\`,
};`,
      },
      {
        title: "Name a missing part in an error message",
        lang: "ts",
        code: `import { runtime } from "skathi:runtime";

export function describeMissingVideo(): string {
  return runtime.features.has("video")
    ? "This file could not be played."
    : \`Skathi \${runtime.version} on this host was built without video.\`;
}`,
      },
    ],
    related: ["runtime-is-aot", "runtime-is-single-file", "app-get-version", "os"],
    aliases: [
      "skathi:runtime",
      "Runtime",
      "runtime.version",
      "runtime.features",
      "Skathi version",
      "host features",
      "feature detection",
    ],
  },
  {
    slug: "runtime-is-aot",
    name: "Runtime.isAot()",
    kind: "function",
    summary: "Tells whether the app's code is running from its native image.",
    syntax: "Runtime.isAot(): boolean",
    returns: "`true` while the app's code runs from its [native image](/guides/aot/), else `false`.",
    notes: [
      "It is `false` under `sk dev`, in a build without `--aot`, and with the environment variable `SK_APP_ENGINE=jit`.",
      "It is `false` after a reload, which runs the bundle. The native entry runs once per process.",
      "The `native-aot` name in [`runtime.features`](/std/runtime/) says the host can load a native image. `isAot()` says the app is running from one now.",
    ],
    examples: [
      {
        title: "Log how the app's code runs",
        lang: "ts",
        code: `import { Runtime } from "skathi:runtime";

console.info(Runtime.isAot() ? "Running from the native image" : "Running the bundle");`,
      },
    ],
    related: ["runtime", "runtime-is-single-file"],
    aliases: ["Runtime.isAot", "isAot", "AOT", "native image", "SK_APP_ENGINE"],
  },
  {
    slug: "runtime-is-single-file",
    name: "Runtime.isSingleFile()",
    kind: "function",
    summary: "Tells whether the app needs no file beside its executable.",
    syntax: "Runtime.isSingleFile(): boolean",
    returns:
      "`true` when the payload is inside the executable and there are no asset packs, no `icu.skpk` and no native image file to load.",
    notes: [
      "A macOS `.app` bundle counts as one file.",
      "It is `false` under `sk dev` and for a payload read from a file.",
    ],
    examples: [
      {
        title: "Choose how to update the app",
        lang: "ts",
        code: `import { Runtime } from "skathi:runtime";

export function updateStrategy(): "replace-executable" | "replace-folder" {
  return Runtime.isSingleFile() ? "replace-executable" : "replace-folder";
}`,
      },
    ],
    related: ["runtime", "runtime-is-aot", "app-restart"],
    aliases: ["Runtime.isSingleFile", "isSingleFile", "single file", "portable", "asset packs"],
  },
]);
