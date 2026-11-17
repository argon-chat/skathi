import { OK, PLAN, all, importLine, os, section } from "./shared";

const IMPORT = importLine("appLaunch");
const UNTRUSTED = "Arguments and URLs come from whoever started the app: treat them as untrusted input.";
const PROTOCOL_PLATFORMS = os(OK, PLAN, PLAN);
const PROTOCOL_ELSEWHERE =
  "On macOS and Linux, `registerProtocol()` throws an `Error` named `NotSupportedError`, and the other two return `false`: not implemented yet.";

export const launch = section("Launch", [
  {
    slug: "app-launch",
    name: "appLaunch",
    kind: "property",
    summary:
      "How this process was started (command line, working folder, deep link, file arguments), the single-instance lock, and the `launch` event for later launches.",
    platforms: all(OK),
    syntax: `${IMPORT}

readonly appLaunch.argv: readonly string[]
readonly appLaunch.cwd: string
readonly appLaunch.url: string | null
readonly appLaunch.paths: readonly string[]
readonly appLaunch.singleInstance: boolean
appLaunch.registerProtocol(scheme: string): void
appLaunch.unregisterProtocol(scheme: string): boolean
appLaunch.isProtocolRegistered(scheme: string): boolean
appLaunch.addEventListener("launch", listener)`,
    notes: [
      "The web's `launchQueue.setConsumer()` receives the same launches as `LaunchParams` (`targetURL`, and `files` as file handles). `appLaunch` adds what a browser cannot say: the raw command line, the working folder and the lock.",
      'The launch identity, `"singleInstance"`, `"raiseOnSecondInstance"` and `"protocols"` are set in `sk.config.json`. See [singleInstance](/window-api/config-single-instance/).',
      UNTRUSTED,
      "A reload is not a launch: the values stay those of the process, and no event fires.",
    ],
    examples: [
      {
        title: "Open the files the app was started with, and later ones",
        lang: "ts",
        code: `import { appLaunch, appWindow } from "@skathi-native/runtime";
import type { SkLaunchEvent } from "@skathi-native/runtime";

export function handleLaunches(open: (path: string) => void): void {
  appLaunch.paths.forEach(open);
  appLaunch.addEventListener("launch", (event) => {
    (event as SkLaunchEvent).paths.forEach(open);
    appWindow.show();
  });
}`,
        note: 'Needs `"singleInstance": true` for the later launches to reach this process.',
      },
    ],
    related: ["launch-argv", "launch-url", "launch-single-instance", "launch-event", "launch-register-protocol"],
    aliases: ["process.argv", "requestSingleInstanceLock", "second-instance", "deep link", "launchQueue"],
  },
  {
    slug: "launch-argv",
    name: "appLaunch.argv / cwd / paths",
    kind: "property",
    summary:
      "The raw command line of this process, the folder it was started in, and the arguments that name existing files or folders.",
    platforms: all(OK),
    syntax:
      "readonly appLaunch.argv: readonly string[]\nreadonly appLaunch.cwd: string\nreadonly appLaunch.paths: readonly string[]",
    notes: [
      "`argv` is the whole command line, the executable first, host flags included; the arrays are frozen.",
      "`paths` holds the app's own arguments (host flags left out) that exist on disk, made absolute against `cwd`. Arguments starting with `-` and URLs are skipped.",
      "`cwd` is the working directory at startup, absolute.",
      UNTRUSTED,
    ],
    examples: [
      {
        title: "Read a --profile option",
        lang: "ts",
        code: `import { appLaunch } from "@skathi-native/runtime";

const at = appLaunch.argv.indexOf("--profile");
export const profile = (at >= 0 && appLaunch.argv[at + 1]) || "default";`,
      },
    ],
    edgeCases: ["A path is checked once at startup: it may be gone by the time the app opens it."],
    related: ["app-launch", "launch-event"],
    aliases: ["process.argv", "process.cwd", "command line"],
  },
  {
    slug: "launch-url",
    name: "appLaunch.url",
    kind: "property",
    summary: "The deep link (`myapp://…`) this process was started for, or `null`.",
    platforms: PROTOCOL_PLATFORMS,
    syntax: "readonly appLaunch.url: string | null",
    notes: [
      'Set when Windows started the app for a scheme registered with `registerProtocol()` or the `"protocols"` key. The URL is taken whole from the command line, so quotes and spaces in it arrive unchanged.',
      'With `singleInstance`, a link opened while the app runs arrives as a `launch` event with `source: "open-url"` and `url` instead.',
      UNTRUSTED,
    ],
    examples: [
      {
        title: "Route a deep link",
        lang: "ts",
        code: `import { appLaunch, type SkLaunchEvent } from "@skathi-native/runtime";

function route(link: string | null, go: (path: string) => void): void {
  if (!link || !URL.canParse(link)) return;
  const url = new URL(link);
  const id = url.pathname.slice(1);
  if (url.protocol === "notes:" && url.hostname === "note" && /^[\\w-]+$/.test(id)) go(\`/note/\${id}\`);
}

export function deepLinks(go: (path: string) => void): void {
  route(appLaunch.url, go);
  appLaunch.addEventListener("launch", (e) => route((e as SkLaunchEvent).url, go));
}`,
      },
    ],
    limitations: [
      'On macOS, the bundle declares the `"protocols"` schemes, but a link that opens the app is not delivered to `appLaunch` yet. On Linux, no scheme is registered.',
    ],
    related: ["launch-register-protocol", "launch-event", "app-launch"],
    aliases: ["deep link", "open-url", "protocol handler", "custom URL scheme"],
  },
  {
    slug: "launch-single-instance",
    name: "appLaunch.singleInstance",
    kind: "property",
    summary:
      'Whether this process holds the single-instance lock: `"singleInstance": true` is set and it was the first to start.',
    platforms: all(OK),
    platformSince: { macos: "0.3.0" },
    syntax: "readonly appLaunch.singleInstance: boolean",
    notes: [
      'With `"singleInstance": true` in `sk.config.json`, a second launch hands its command line, working folder and URL to the running process and exits before it opens a window. The running one gets a `launch` event and, unless `"raiseOnSecondInstance": false`, its window is brought to the front.',
      "`false` when the key is not set, when the environment variable `SK_SINGLE_INSTANCE` is `0` or `off`, in the devtools inspector, or when the lock could not be taken (the app then runs as a separate instance and says why on stderr).",
      'The lock is per user and per app identity (`"id"`, else the app name), so two development copies of one app share it.',
      "It works on Windows, Linux and, since 0.3.0, macOS.",
    ],
    examples: [
      {
        title: "Say how the app was started",
        lang: "ts",
        code: `import { appLaunch } from "@skathi-native/runtime";

console.log(appLaunch.singleInstance ? "Single instance: later launches come here." : "Running as a separate instance.");`,
      },
    ],
    edgeCases: [
      "A second launch that gets no answer within 5 s exits without starting a second copy; so does one that finds the lock held by a process that is not this app.",
      "A launch while the app is quitting waits for it to exit and then becomes the new instance.",
    ],
    related: ["launch-event", "app-launch"],
    aliases: ["requestSingleInstanceLock", "single instance", "singleInstance"],
  },
  {
    slug: "launch-event",
    name: "appLaunch launch event",
    kind: "event",
    summary: "Fires in the running app when the app is started again and the second launch is handed over.",
    platforms: all(OK),
    platformSince: { macos: "0.3.0" },
    syntax: `appLaunch.addEventListener("launch", (event: SkLaunchEvent) => void)

interface SkLaunchEvent extends Event {
  readonly argv: readonly string[];
  readonly cwd: string;
  readonly url: string | null;
  readonly paths: readonly string[];
  readonly source: "second-instance" | "open-url";
}`,
    notes: [
      "`argv`, `cwd` and `url` are the second launch's; `paths` are its file arguments resolved against its own working folder.",
      '`source` is `"open-url"` when the second launch was started for a link to a registered scheme (Windows; macOS planned), and `"second-instance"` otherwise. A click on the Dock icon with no window shows the window without an event.',
      "After the event, `launchQueue`'s consumer receives the same launch as `LaunchParams`.",
      'The window is also brought to the front, unless `"raiseOnSecondInstance"` is `false`, either process runs with `SK_WINDOW_FOCUS=never`, or `SK_LAUNCH_RAISE=off` is set.',
      UNTRUSTED,
    ],
    examples: [
      {
        title: "Show the window again when the user starts the app",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { appLaunch, appWindow, type SkLaunchEvent } from "@skathi-native/runtime";

const opened = ref<string[]>([]);
function onLaunch(event: Event) {
  opened.value.push(...(event as SkLaunchEvent).paths);
  appWindow.show();
}
appLaunch.addEventListener("launch", onLaunch);
onUnmounted(() => appLaunch.removeEventListener("launch", onLaunch));
</script>

<template>
  <li v-for="path in opened" :key="path">{{ path }}</li>
</template>`,
      },
    ],
    limitations: ['Fires only with `"singleInstance": true`: otherwise each launch is its own process.'],
    related: ["launch-single-instance", "launch-url", "app-launch"],
    aliases: ["second-instance", "SkLaunchEvent", "open-file"],
  },
  {
    slug: "launch-register-protocol",
    name: "appLaunch.registerProtocol()",
    kind: "method",
    summary: "Makes links with a custom scheme (`notes://…`) start this app, for the current user.",
    platforms: PROTOCOL_PLATFORMS,
    syntax: "appLaunch.registerProtocol(scheme: string): void",
    params: [
      {
        name: "scheme",
        type: "string",
        description:
          "A letter, then up to 62 letters, digits, `+`, `-` or `.`. Case, a trailing `:` and `//` are ignored.",
      },
    ],
    returns: "`undefined`.",
    notes: [
      "It writes the scheme under `HKEY_CURRENT_USER\\Software\\Classes`: no administrator rights. A link then starts the app with the URL, which arrives as `appLaunch.url`, or as a `launch` event when the app is already running with `singleInstance`.",
      'A packaged app registers the schemes listed in `"protocols"` in `sk.config.json` by itself at every start; call this only for a scheme the user turns on at run time.',
      "In a development host it points the scheme at the development build and prints an advisory; unregister it when done.",
    ],
    examples: [
      {
        title: "An opt-in link handler",
        lang: "ts",
        code: `import { appLaunch } from "@skathi-native/runtime";

export function setLinkHandler(on: boolean): boolean {
  if (on) appLaunch.registerProtocol("notes");
  else appLaunch.unregisterProtocol("notes");
  return appLaunch.isProtocolRegistered("notes");
}`,
      },
    ],
    edgeCases: [
      "It throws a `TypeError` for an invalid scheme; for a reserved one (`http`, `https`, `file`, `ftp`, `mailto`, `tel`, `data`, `blob`, `javascript`, `about`, `vbscript`, `search-ms`, any `ms-*`); for a scheme registered to another app, which Skathi never takes over; and in an MSIX-packaged app, which declares its protocols in its manifest instead.",
      'Without `"singleInstance": true`, every link starts a new process.',
    ],
    limitations: [PROTOCOL_ELSEWHERE],
    related: ["launch-unregister-protocol", "launch-is-protocol-registered", "launch-url"],
    aliases: ["setAsDefaultProtocolClient", "protocol handler", "custom URL scheme", "deep link"],
  },
  {
    slug: "launch-unregister-protocol",
    name: "appLaunch.unregisterProtocol()",
    kind: "method",
    summary: "Removes this app's registration of a custom scheme.",
    platforms: PROTOCOL_PLATFORMS,
    syntax: "appLaunch.unregisterProtocol(scheme: string): boolean",
    params: [{ name: "scheme", type: "string", description: "The scheme, as given to `registerProtocol()`." }],
    returns:
      "`true` when a registration of this app was removed, `false` when there was none, it belongs to another app, or the scheme is invalid.",
    notes: [
      "It never removes another app's registration.",
      "Running the executable with `--sk-unregister-protocols` removes every scheme registered to the app and exits, for an uninstaller.",
    ],
    examples: [
      {
        title: "Clean up a development registration",
        lang: "ts",
        code: `import { appLaunch } from "@skathi-native/runtime";

if (appLaunch.unregisterProtocol("notes")) console.log("notes:// no longer opens this build.");`,
      },
    ],
    limitations: [PROTOCOL_ELSEWHERE],
    related: ["launch-register-protocol", "launch-is-protocol-registered"],
    aliases: ["removeAsDefaultProtocolClient"],
  },
  {
    slug: "launch-is-protocol-registered",
    name: "appLaunch.isProtocolRegistered()",
    kind: "method",
    summary: "Whether a custom scheme is registered to this app for the current user.",
    platforms: PROTOCOL_PLATFORMS,
    syntax: "appLaunch.isProtocolRegistered(scheme: string): boolean",
    params: [{ name: "scheme", type: "string", description: "The scheme to check." }],
    returns:
      "`true` only when the scheme opens this app; `false` when it is free, belongs to another app, or is invalid.",
    examples: [
      {
        title: "A checkbox that shows the real state",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { appLaunch } from "@skathi-native/runtime";

const registered = ref(appLaunch.isProtocolRegistered("notes"));
function toggle() {
  if (registered.value) appLaunch.unregisterProtocol("notes");
  else appLaunch.registerProtocol("notes");
  registered.value = appLaunch.isProtocolRegistered("notes");
}
</script>

<template>
  <label><input type="checkbox" :checked="registered" @change="toggle" /> Open notes:// links with this app</label>
</template>`,
      },
    ],
    limitations: [PROTOCOL_ELSEWHERE],
    related: ["launch-register-protocol", "launch-unregister-protocol"],
    aliases: ["isDefaultProtocolClient"],
  },
]);
