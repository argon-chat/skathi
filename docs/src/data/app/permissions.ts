import { OK, PART, importLine, os, section } from "./shared";

const IMPORT = importLine("appPermissions");
const PLATFORMS = os(OK, PART, PART);

export const permissions = section("Permissions", [
  {
    slug: "app-permissions",
    name: "appPermissions",
    kind: "property",
    summary:
      "Reads and requests the app's access to the microphone, camera, screen capture, notifications, clipboard and location, with a fourth state, `unsupported`.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

readonly appPermissions.platform: string
readonly appPermissions.names: string[]
appPermissions.query(name: string): "granted" | "denied" | "prompt" | "unsupported"
appPermissions.list(): PermissionRow[]
appPermissions.request(name: string): Promise<"granted" | "denied" | "prompt" | "unsupported">
appPermissions.addEventListener("change", listener)`,
    values: [
      {
        value: "microphone",
        status: OK,
        note: "Windows: the app's own Allow or Block prompt, under the system's privacy switches. macOS: the system prompt. Linux: `prompt` until the first request, which grants without a dialog.",
      },
      {
        value: "camera",
        status: PART,
        note: "Windows: the system's privacy switches, `granted` or `denied`. macOS: the system prompt. Linux: `unsupported`.",
      },
      {
        value: "display-capture",
        status: PART,
        note: "Windows: the system's screen capture switch. macOS: Screen Recording; a grant applies after a relaunch. Linux: `unsupported`.",
      },
      {
        value: "notifications",
        status: PART,
        note: "Windows: the system's and the app's notification switches. macOS and Linux: `unsupported`.",
      },
      { value: "clipboard-read", status: PART, note: "Windows: always `granted`. macOS and Linux: `unsupported`." },
      { value: "clipboard-write", status: PART, note: "Windows: always `granted`. macOS and Linux: `unsupported`." },
      {
        value: "geolocation",
        status: PART,
        note: "Windows: the system's location switches. macOS and Linux: `unsupported`.",
      },
    ],
    notes: [
      "The same states as `navigator.permissions.query()`, read synchronously, plus `unsupported` for a permission this platform has no switch for.",
      "On Windows the system never asks a desktop app. For the microphone, Skathi asks itself with an Allow or Block dialog (Block is the default button) and keeps the answer in `permissions.json` in the storage folder. The other permissions read the system's privacy switches: `granted` or `denied`, never `prompt`.",
      "On macOS, the microphone, camera and screen recording use the system's own prompts and settings; Skathi stores no answer of its own.",
      "Group Policy that denies a capability on Windows reads as `denied`.",
      "A Svelte app imports it from `@skathi-native/runtime/svelte`. Nothing in it is reactive: listen for `change`.",
    ],
    examples: [
      {
        title: "A permissions page",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { appPermissions } from "@skathi-native/runtime";

const rows = ref(appPermissions.list());
const refresh = () => (rows.value = appPermissions.list());
appPermissions.addEventListener("change", refresh);
onUnmounted(() => appPermissions.removeEventListener("change", refresh));
</script>

<template>
  <table>
    <tr v-for="row in rows" :key="row.name">
      <td>{{ row.name }}</td>
      <td>{{ row.state }}{{ row.deniedBySystem ? " (by the system)" : "" }}</td>
      <td><button v-if="row.canRequest" @click="appPermissions.request(row.name)">Ask</button></td>
    </tr>
  </table>
</template>`,
      },
    ],
    edgeCases: [
      "`SK_MEDIA_CAPTURE=deny` makes the microphone `denied` and never shows a prompt, for tests.",
      "Reading a permission that is `unsupported` here prints an advisory once.",
    ],
    limitations: [
      "On macOS only the microphone, camera and screen recording have a state; on Linux only the microphone. The rest are `unsupported`.",
    ],
    related: [
      "permissions-query",
      "permissions-request",
      "permissions-list",
      "permissions-change",
      "capture-permission",
    ],
    aliases: ["navigator.permissions", "systemPreferences.getMediaAccessStatus", "askForMediaAccess", "privacy"],
  },
  {
    slug: "permissions-query",
    name: "appPermissions.query()",
    kind: "method",
    summary: "Returns the current state of one permission, synchronously.",
    platforms: PLATFORMS,
    syntax: 'appPermissions.query(name: string): "granted" | "denied" | "prompt" | "unsupported"',
    params: [
      {
        name: "name",
        type: "string",
        description:
          "`microphone`, `camera`, `display-capture`, `notifications`, `clipboard-read`, `clipboard-write` or `geolocation`.",
      },
    ],
    returns: "The state as the system and the app's stored answer give it now.",
    notes: [
      "`prompt` means asking would show a question: the app's own dialog on Windows and Linux, the system's on macOS.",
      "It reads the state each call; there is no cache to go stale.",
    ],
    examples: [
      {
        title: "Only show the call button when it can work",
        lang: "ts",
        code: `import { appPermissions } from "@skathi-native/runtime";

export function canJoinWithAudio(): boolean {
  const state = appPermissions.query("microphone");
  return state === "granted" || state === "prompt";
}`,
      },
    ],
    edgeCases: ["Any other name throws a `TypeError`."],
    related: ["permissions-list", "permissions-request", "app-permissions"],
    aliases: ["permissions.query", "PermissionState"],
  },
  {
    slug: "permissions-list",
    name: "appPermissions.list()",
    kind: "method",
    summary: "Returns every permission with its state, whether it can be requested, and whether the system denies it.",
    platforms: PLATFORMS,
    syntax: `appPermissions.list(): PermissionRow[]

interface PermissionRow {
  name: string;
  state: "granted" | "denied" | "prompt" | "unsupported";
  canRequest: boolean;
  deniedBySystem?: boolean;
}`,
    returns: "One row per name in `appPermissions.names`, in that order.",
    notes: [
      "`canRequest` is `false` for `granted` and `unsupported`, `true` otherwise: a request can show a prompt, or open the settings page where the user can change a denial.",
      "`deniedBySystem` is `true` when the state is `denied` because of a system switch rather than the user's Block in the app's own dialog.",
    ],
    examples: [
      {
        title: "Log what the app may use",
        lang: "ts",
        code: `import { appPermissions } from "@skathi-native/runtime";

for (const row of appPermissions.list()) {
  if (row.state !== "unsupported") console.log(\`\${row.name}: \${row.state}\`);
}`,
      },
    ],
    related: ["permissions-query", "permissions-names", "app-permissions"],
    aliases: ["PermissionRow"],
  },
  {
    slug: "permissions-request",
    name: "appPermissions.request()",
    kind: "method",
    summary: "Asks for a permission and resolves with the state read back afterwards, never optimistically.",
    platforms: PLATFORMS,
    syntax: 'appPermissions.request(name: string): Promise<"granted" | "denied" | "prompt" | "unsupported">',
    params: [{ name: "name", type: "string", description: "A name from `appPermissions.names`." }],
    returns: "A promise for the state once the question is answered, the settings page closed, or the wait over.",
    notes: [
      "Already `granted` or `unsupported`: it resolves at once and shows nothing.",
      "Microphone on Windows: when a system switch is off, it opens the Settings page and, once the switch is on, goes on to the app's dialog. Otherwise it shows the dialog. A stored Allow grants at once, and a stored Block asks again, so this is the way back from a Block.",
      "Other permissions on Windows: it opens the matching Settings page (`ms-settings:privacy-webcam`, `privacy-location`, `notifications`) and resolves when the switch moves, or with the unchanged state after 3 minutes.",
      "On macOS: the system prompt the first time; after a denial the state stays `denied` until the user changes it in System Settings.",
      "Concurrent requests for one permission share one question.",
      "Dismissing the app's dialog without answering resolves with `prompt` and stores nothing.",
    ],
    examples: [
      {
        title: "Ask before a call",
        lang: "ts",
        code: `import { appCapture, appPermissions } from "@skathi-native/runtime";

export async function ensureMicrophone(): Promise<string | null> {
  const state = await appPermissions.request("microphone");
  if (state === "granted") return null;
  return appCapture.permissionHint("microphone") || "The microphone is off for this app.";
}`,
        note: "Call it from a click: the user expects a question right after an action.",
      },
    ],
    edgeCases: ["An unknown name rejects with a `TypeError`."],
    related: ["permissions-query", "permissions-change", "capture-request-permission", "capture-permission-hint"],
    aliases: ["askForMediaAccess", "request permission"],
  },
  {
    slug: "permissions-change",
    name: "appPermissions change event",
    kind: "event",
    summary:
      "Fires on `appPermissions` when a permission's state changes, from a prompt, a Settings switch or the app's own store.",
    platforms: PLATFORMS,
    syntax: `appPermissions.addEventListener("change", (event: Event & { name: string; state: string }) => void)`,
    notes: [
      "`event.name` is the permission and `event.state` its new state.",
      "It mirrors the standard `PermissionStatus` `change` event of each permission, re-dispatched in one place.",
      "On Windows, a privacy switch flipped in Settings fires it while the app runs.",
    ],
    examples: [
      {
        title: "React to the microphone being switched off",
        lang: "ts",
        code: `import { appPermissions } from "@skathi-native/runtime";

appPermissions.addEventListener("change", (event) => {
  const { name, state } = event as Event & { name: string; state: string };
  if (name === "microphone" && state === "denied") console.warn("The microphone was switched off for this app.");
});`,
      },
    ],
    limitations: ["On Linux, the only change is the microphone answer stored by a request."],
    related: ["app-permissions", "permissions-query"],
    aliases: ["onchange", "PermissionStatus.onchange"],
  },
  {
    slug: "permissions-names",
    name: "appPermissions.names / platform",
    kind: "property",
    summary: "The permission names this build knows, and the name of the platform answering for them.",
    platforms: os(OK, OK, OK),
    syntax: "readonly appPermissions.names: string[]\nreadonly appPermissions.platform: string",
    notes: [
      "`names` is `microphone`, `camera`, `display-capture`, `notifications`, `clipboard-read`, `clipboard-write`, `geolocation` on every platform, also where some read `unsupported`.",
      '`platform` is `"Windows"` or `"macOS"`; on Linux it reads `"this platform"`.',
    ],
    examples: [
      {
        title: "Build a settings list from the names",
        lang: "ts",
        code: `import { appPermissions } from "@skathi-native/runtime";

export const supported = appPermissions.names.filter((name) => appPermissions.query(name) !== "unsupported");`,
      },
    ],
    related: ["permissions-list", "app-permissions"],
  },
]);
