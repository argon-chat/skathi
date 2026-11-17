import { NO, OK, PART, importLine, os, section } from "./shared";

const IMPORT = importLine("appCapture");
const PLATFORMS = os(OK, OK, NO);
const SOURCE = '`"microphone"`, `"camera"` or `"screen_share"`, the last one being the `display-capture` permission.';

export const capture = section("Screen capture", [
  {
    slug: "app-capture",
    name: "appCapture",
    kind: "property",
    summary:
      "Lists the screens and windows that can be captured, with thumbnails, and reads or requests the capture permissions.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

appCapture.getSources(types?: string[], options?: { thumbnailSize?: { width: number; height: number }; withIcons?: boolean }): Promise<ScreenCaptureSource[]>
appCapture.setPendingSource(sourceId: string, includeAudio?: boolean): boolean
appCapture.permission(source: CaptureSource): CapturePermission
appCapture.requestPermission(source: CaptureSource): Promise<PermissionState>
appCapture.permissionHint(source: CaptureSource): string`,
    notes: [
      "The building blocks of a screen-share picker: list the sources, show their thumbnails, remember the user's choice.",
      "The permission methods are [appPermissions](/app-api/app-permissions/) under capture names, with a hint text for a denial.",
      "With the environment variable `SK_MEDIA_CAPTURE=fake`, `getSources()` returns one synthetic 1920 × 1080 screen with a test pattern, for tests and machines without a display.",
    ],
    examples: [
      {
        title: "A screen picker",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";
import { appCapture, type ScreenCaptureSource } from "@skathi-native/runtime";

const emit = defineEmits<{ pick: [id: string] }>();
const sources = ref<ScreenCaptureSource[]>([]);
const error = ref("");

onMounted(async () => {
  try {
    sources.value = await appCapture.getSources(["screen", "window"], { thumbnailSize: { width: 240, height: 135 } });
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
});
</script>

<template>
  <p v-if="error">{{ error }}</p>
  <ul class="sources">
    <li v-for="source in sources" :key="source.id" @click="emit('pick', source.id)">
      <img v-if="source.thumbnailDataUrl" :src="source.thumbnailDataUrl" alt="" />
      <span>{{ source.name }}</span>
    </li>
  </ul>
</template>`,
      },
    ],
    limitations: [
      "On Linux, `getSources()` rejects with `NotSupportedError`; the microphone reads `granted`, the camera and screen capture `unsupported`.",
    ],
    related: ["capture-get-sources", "capture-set-pending-source", "capture-permission", "app-permissions"],
    aliases: ["desktopCapturer", "screen share", "screen picker", "getDisplayMedia"],
  },
  {
    slug: "capture-get-sources",
    name: "appCapture.getSources()",
    kind: "method",
    summary:
      "Lists the screens and the app windows that can be captured, each with a PNG thumbnail and, for windows, the app's icon.",
    platforms: PLATFORMS,
    syntax: `appCapture.getSources(
  types?: ("screen" | "window")[],
  options?: { thumbnailSize?: { width?: number; height?: number }; withIcons?: boolean },
): Promise<ScreenCaptureSource[]>

interface ScreenCaptureSource {
  id: string; // "screen:…" or "window:…"
  name: string;
  thumbnailDataUrl: string;
  appIconDataUrl: string | null;
  displayId: string;
  [key: string]: unknown;
}`,
    params: [
      {
        name: "types",
        type: '("screen" | "window")[]',
        description: "Which kinds to list.",
        optional: true,
        default: '["screen", "window"]',
      },
      {
        name: "options.thumbnailSize",
        type: "{ width?: number; height?: number }",
        description:
          "The box the thumbnail fits in, keeping its aspect ratio; each side 0 to 1024. 0 skips thumbnails.",
        optional: true,
        default: "320 × 180",
      },
      {
        name: "options.withIcons",
        type: "boolean",
        description: "Include the window's app icon.",
        optional: true,
        default: "true",
      },
    ],
    returns: "A promise for the sources: screens first, then windows.",
    notes: [
      "Every source has `kind` (`screen` or `window`), `bounds` (`x`, `y`, `width`, `height`), `width`, `height` and `thumbnailError` (why the thumbnail is empty, or `null`) beside the typed fields.",
      "A screen also has `deviceName`, `workArea` and `isPrimary`, and on macOS `scaleFactor`. Screens are named `Screen 1`, `Screen 2`, … and `appIconDataUrl` is `null`.",
      "A window has its title as `name` and `processId`, and on macOS `appName` and `bundleId`.",
      "On Windows, hidden and minimized windows, tool windows and owned windows such as dialogs are left out, and the list stops at 512 windows.",
      "On macOS, names and thumbnails count as screen recording: the first call shows the Screen Recording prompt and waits for the answer, on a thread of its own.",
      "The enumeration and the thumbnails run off the UI thread.",
    ],
    examples: [
      {
        title: "Pick the primary screen without asking",
        lang: "ts",
        code: `import { appCapture } from "@skathi-native/runtime";

export async function primaryScreenId(): Promise<string | undefined> {
  const screens = await appCapture.getSources(["screen"], { thumbnailSize: { width: 0, height: 0 }, withIcons: false });
  return (screens.find((s) => s["isPrimary"] === true) ?? screens[0])?.id;
}`,
      },
    ],
    edgeCases: [
      "`getSources([])` resolves with an empty list without asking anything.",
      'A type other than `"screen"` or `"window"`, or a thumbnail side above 1024, rejects with a `TypeError`.',
      'When screen capture is denied, it rejects with an `Error` named `NotAllowedError`: "Permission denied by system", followed on macOS by where to turn it on. A dismissed macOS prompt rejects with "Permission dismissed".',
    ],
    limitations: ["On Linux, it rejects with an `Error` named `NotSupportedError`."],
    related: ["capture-set-pending-source", "capture-permission", "app-capture"],
    aliases: ["desktopCapturer.getSources", "ScreenCaptureSource", "thumbnail"],
  },
  {
    slug: "capture-set-pending-source",
    name: "appCapture.setPendingSource()",
    kind: "method",
    summary: "Stores the screen or window the user picked, for the next `getDisplayMedia()` call to capture.",
    platforms: os(PART, PART, PART),
    syntax: "appCapture.setPendingSource(sourceId: string, includeAudio?: boolean): boolean",
    params: [
      {
        name: "sourceId",
        type: "string",
        description: "An `id` from `getSources()`: `screen:<number>` or `window:<number>`.",
      },
      {
        name: "includeAudio",
        type: "boolean",
        description: "Ask for system audio with it.",
        optional: true,
        default: "false",
      },
    ],
    returns: "`true`.",
    notes: [
      "The choice replaces any earlier one and is cleared by a reload.",
      "It only records the choice: the capture itself is done by `navigator.mediaDevices.getDisplayMedia()`.",
    ],
    examples: [
      {
        title: "Hand the picked source to the capture",
        lang: "ts",
        code: `import { appCapture } from "@skathi-native/runtime";

export function choose(sourceId: string, withAudio: boolean): void {
  appCapture.setPendingSource(sourceId, withAudio);
}`,
      },
    ],
    edgeCases: ["An ID that is not `screen:` or `window:` followed by digits throws a `TypeError`."],
    limitations: [
      "Skathi has no `getDisplayMedia()` of its own yet. The stored choice is read by an implementation that a program embedding Skathi provides.",
    ],
    related: ["capture-get-sources", "app-capture"],
    aliases: ["getDisplayMedia", "screen share source"],
  },
  {
    slug: "capture-permission",
    name: "appCapture.permission()",
    kind: "method",
    summary:
      "Reads the state of the microphone, camera or screen capture permission, with whether it can be requested.",
    platforms: PLATFORMS,
    syntax: `appCapture.permission(source: CaptureSource): CapturePermission

type CaptureSource = "microphone" | "camera" | "screen_share";

interface CapturePermission {
  state: "granted" | "denied" | "prompt" | "unsupported";
  canRequest: boolean;
  deniedBySystem: boolean;
}`,
    params: [{ name: "source", type: "CaptureSource", description: SOURCE }],
    returns: "A fresh object, as one row of `appPermissions.list()`.",
    notes: ["`deniedBySystem` tells a system switch apart from the user's Block in the app's own microphone dialog."],
    examples: [
      {
        title: "Grey out the share button",
        lang: "ts",
        code: `import { appCapture } from "@skathi-native/runtime";

export const canShareScreen = () => appCapture.permission("screen_share").state !== "denied";`,
      },
    ],
    edgeCases: ["Any other source throws a `TypeError`."],
    limitations: [
      "On Linux, the microphone is `granted` (the desktop does not gate it), the camera and screen capture `unsupported`.",
    ],
    related: ["capture-request-permission", "capture-permission-hint", "permissions-list"],
    aliases: ["CapturePermission", "CaptureSource"],
  },
  {
    slug: "capture-request-permission",
    name: "appCapture.requestPermission()",
    kind: "method",
    summary: "Asks for the microphone, camera or screen capture permission, as `appPermissions.request()` does.",
    platforms: PLATFORMS,
    syntax:
      'appCapture.requestPermission(source: CaptureSource): Promise<"granted" | "denied" | "prompt" | "unsupported">',
    params: [{ name: "source", type: "CaptureSource", description: SOURCE }],
    returns: "A promise for the state read back after the question or the settings page.",
    notes: [
      "The app's own prompt where Skathi asks itself, the system prompt on macOS, or the system's settings page after a system denial. See [appPermissions.request()](/app-api/permissions-request/).",
      "On macOS, a Screen Recording grant made in System Settings applies only after the app is relaunched; until then the state stays `denied`.",
    ],
    examples: [
      {
        title: "Ask for screen recording, then explain",
        lang: "ts",
        code: `import { appCapture } from "@skathi-native/runtime";

export async function askScreen(): Promise<string | null> {
  const state = await appCapture.requestPermission("screen_share");
  return state === "granted" || state === "unsupported" ? null : appCapture.permissionHint("screen_share");
}`,
      },
    ],
    edgeCases: ["Any other source rejects with a `TypeError`."],
    related: ["capture-permission", "capture-permission-hint", "permissions-request"],
  },
  {
    slug: "capture-permission-hint",
    name: "appCapture.permissionHint()",
    kind: "method",
    summary: "Returns a sentence that tells the user where to turn a capture permission back on, or an empty string.",
    platforms: os(OK, OK, NO),
    syntax: "appCapture.permissionHint(source: CaptureSource): string",
    params: [{ name: "source", type: "CaptureSource", description: SOURCE }],
    returns: 'The hint, or `""` when this platform has none for the source.',
    values: [
      {
        value: "microphone",
        status: OK,
        note: "Windows and macOS: the privacy page in Settings or System Settings, with its `ms-settings:` or `x-apple.systempreferences:` link.",
      },
      { value: "camera", status: OK, note: "Windows and macOS, as for the microphone." },
      {
        value: "screen_share",
        status: PART,
        note: "macOS only: the Screen & System Audio Recording page, and that the app must be reopened.",
      },
    ],
    notes: [
      "The sentence is in English and names the app with the window title. It is the same text a denied capture rejects with.",
    ],
    examples: [
      {
        title: "Show the hint under a denied permission",
        lang: "vue",
        code: `<script setup lang="ts">
import { appCapture } from "@skathi-native/runtime";

const mic = appCapture.permission("microphone");
const hint = mic.deniedBySystem ? appCapture.permissionHint("microphone") : "";
</script>

<template>
  <p v-if="mic.state === 'denied'">The microphone is off. {{ hint }}</p>
</template>`,
      },
    ],
    edgeCases: ["Any other source throws a `TypeError`."],
    related: ["capture-permission", "capture-request-permission"],
  },
]);
