import { NO, OK, importLine, os, section } from "./shared";

const IMPORT = importLine("appHotkeys");
const PLATFORMS = os(OK, OK, NO);
const LINUX =
  'On Linux, `available` is `false`: the hooks cannot start, `status().error` says so, and `captureOnce()` rejects with "hotkeys are not running".';
const MAC_AX =
  "On macOS, the hooks need Accessibility access. Without it they do not start, and `status().error` says where to grant it; call `restart()` once it is granted.";

export const hotkeys = section("Global hotkeys", [
  {
    slug: "app-hotkeys",
    name: "appHotkeys",
    kind: "property",
    summary:
      "System-wide key and mouse-button chords that fire while another app has the focus, with a down and an up edge for push-to-talk.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

appHotkeys.available: boolean
appHotkeys.setBindings(bindings: readonly HotkeyBinding[]): boolean
appHotkeys.onFired(listener: (event: HotkeyEvent) => void): () => void
appHotkeys.captureOnce(): Promise<HotkeyChord>
appHotkeys.cancelCapture(): void
appHotkeys.status(): HotkeyStatus
appHotkeys.requestAccessibility(): boolean
appHotkeys.restart(): boolean`,
    notes: [
      "A low-level keyboard and mouse hook on Windows and a Quartz event tap on macOS watch every key. They start on the first `setBindings()` or `captureOnce()`, not at launch.",
      "Keys are Windows virtual-key codes on every platform, so a binding saved on one OS works on the other. See [HotkeyChord](/app-api/hotkeys-chord/).",
      "Events arrive in a later frame on the JS thread. Every `down` an app received gets its `up`, also when the bindings change, a capture starts, the hooks restart or the page reloads.",
      "A full reload drops the listeners and cancels a capture; the bindings stay with the host.",
      "`available` is `true` on Windows and macOS. Nothing in it is reactive.",
    ],
    examples: [
      {
        title: "Push to talk on F9",
        lang: "ts",
        code: `import { appHotkeys } from "skathi:app";

const F9 = 0x78;

export function pushToTalk(setTalking: (on: boolean) => void): () => void {
  appHotkeys.setBindings([{ id: "ptt", chord: { buttons: [{ device: 0, code: F9 }] } }]);
  return appHotkeys.onFired((event) => {
    if (event.id === "ptt") setTalking(event.phase === "down");
  });
}`,
      },
    ],
    limitations: [MAC_AX, LINUX],
    related: ["hotkeys-set-bindings", "hotkeys-on-fired", "hotkeys-capture-once", "hotkeys-chord", "hotkeys-status"],
    aliases: ["globalShortcut", "global hotkey", "global shortcut", "push to talk", "keyboard hook"],
  },
  {
    slug: "hotkeys-set-bindings",
    name: "appHotkeys.setBindings()",
    kind: "method",
    summary: "Replaces every global hotkey binding at once, and starts the system hooks on the first call.",
    platforms: PLATFORMS,
    syntax: `appHotkeys.setBindings(bindings: readonly HotkeyBinding[]): boolean

interface HotkeyBinding {
  readonly id: string;
  readonly chord: HotkeyChord;
  readonly suppress?: boolean;
  readonly triggerCooldownMs?: number;
}`,
    params: [
      { name: "id", type: "string", description: "Unique, not empty, at most 128 bytes. `onFired` reports it." },
      {
        name: "chord",
        type: "HotkeyChord",
        description: "Up to 16 buttons that must all be held. See [HotkeyChord](/app-api/hotkeys-chord/).",
      },
      {
        name: "suppress",
        type: "boolean",
        description: "Swallow the key presses that complete the chord, so the focused app never sees them.",
        optional: true,
        default: "false",
      },
      {
        name: "triggerCooldownMs",
        type: "number",
        description: "The shortest time between two `down` edges of this binding, at most 60000.",
        optional: true,
        default: "0",
      },
    ],
    returns: "`true` when the bindings changed, `false` when they equal the current ones.",
    notes: [
      "A chord fires when its last button goes down while the others are held, in any order. Holding a key that is not in the chord does not block it, but holding an extra modifier (Shift, Ctrl, Alt, Win or Cmd) does: Ctrl+K does not fire on Ctrl+Shift+K.",
      "When several bindings match one press, the one with the most buttons wins; equal sizes all fire.",
      "Key repeat does not fire again: a binding fires once per press.",
      "A changed set releases what was held first: a `down` already delivered gets its `up` right away, inside the call.",
      "A binding with an empty `buttons` list is ignored. Buttons listed twice count once.",
      "A Vue `reactive()` or `ref()` array works as `bindings`.",
    ],
    examples: [
      {
        title: "Bindings the user edits in settings",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watch } from "vue";
import type { HotkeyBinding } from "@skathi-native/runtime";
import { appHotkeys } from "skathi:app";

const CTRL = 0xa2;
const SHIFT = 0xa0;
const bindings = ref<HotkeyBinding[]>([
  { id: "mute", chord: { buttons: [{ device: 0, code: CTRL }, { device: 0, code: SHIFT }, { device: 0, code: 0x4d }] } },
  { id: "ptt", chord: { buttons: [{ device: 1, code: 4 }] }, suppress: true },
]);

watch(bindings, (next) => appHotkeys.setBindings(next), { deep: true, immediate: true });
</script>

<template>
  <p v-if="!appHotkeys.available">Global shortcuts are not available on this system.</p>
</template>`,
        note: "The first binding is left Ctrl+Shift+M; the second is the mouse's back side button, swallowed so the browser in front does not navigate back.",
      },
    ],
    edgeCases: [
      "`0x10`, `0x11` and `0x12` (Shift, Ctrl, Alt without a side) mean the left key. Bind `0xa1`, `0xa3` or `0xa5` for the right one.",
      "`suppress` swallows only the presses that complete the chord, and their releases; the other keys of the chord still reach the focused app.",
      "Calling it on Linux returns normally and binds nothing.",
    ],
    limitations: [
      "It throws a `TypeError` for a list that is not an array, more than 128 bindings, a duplicate or empty `id`, an unknown field in a binding, more than 16 buttons, a cooldown over 60000, or a button code outside the ranges in [HotkeyChord](/app-api/hotkeys-chord/).",
      MAC_AX,
      LINUX,
    ],
    related: ["hotkeys-chord", "hotkeys-on-fired", "hotkeys-capture-once", "app-hotkeys"],
    aliases: ["HotkeyBinding", "register", "globalShortcut.register"],
  },
  {
    slug: "hotkeys-chord",
    name: "HotkeyChord",
    kind: "interface",
    summary:
      "The buttons of a hotkey: keyboard keys as Windows virtual-key codes or scan codes, and mouse buttons, on every platform.",
    platforms: PLATFORMS,
    syntax: `interface HotkeyChord {
  readonly buttons: readonly HotkeyButton[];
}

interface HotkeyButton {
  readonly device: number; // 0 keyboard, 1 mouse
  readonly code: number;
}`,
    notes: [
      "Keyboard (`device: 0`): a Windows virtual-key code from 1 to 255 (`0x41` is A, `0x70` is F1, `0xa2` is left Ctrl, `0x5b` is the left Windows or Cmd key), or `0x10000 | scanCode` for a key that no virtual-key code tells apart.",
      "Mouse (`device: 1`): 1 left, 2 right, 3 middle, 4 and 5 the side buttons.",
      "On macOS, key codes are translated to the same virtual-key codes, so a chord stored as JSON moves between platforms.",
      "`captureOnce()` returns a chord in this shape; store it and pass it back in a binding.",
    ],
    examples: [
      {
        title: "Show a chord to the user",
        lang: "ts",
        code: `import type { HotkeyChord } from "@skathi-native/runtime";

const names: Record<number, string> = { 0xa0: "Shift", 0xa1: "Right Shift", 0xa2: "Ctrl", 0xa3: "Right Ctrl", 0xa4: "Alt", 0xa5: "Right Alt", 0x5b: "Win", 0x20: "Space" };
const mouse = ["", "Left button", "Right button", "Middle button", "Back", "Forward"];

export function describe(chord: HotkeyChord): string {
  return chord.buttons
    .map(({ device, code }) => {
      if (device === 1) return mouse[code] ?? \`Mouse \${code}\`;
      if (code >= 0x70 && code <= 0x87) return \`F\${code - 0x6f}\`;
      if ((code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a)) return String.fromCharCode(code);
      return names[code] ?? \`Key \${code.toString(16)}\`;
    })
    .join(" + ");
}`,
      },
    ],
    edgeCases: [
      "A device other than 0 or 1, a keyboard code of 0 or above 255 that is not a scan code, or a mouse code outside 1 to 5 throws a `TypeError` in `setBindings()`.",
    ],
    related: ["hotkeys-set-bindings", "hotkeys-capture-once"],
    aliases: ["HotkeyButton", "virtual-key code", "VK code", "scan code"],
  },
  {
    slug: "hotkeys-on-fired",
    name: "appHotkeys.onFired()",
    kind: "method",
    summary:
      "Calls a listener with the binding's `id` and `down` or `up` each time a global hotkey is pressed or released.",
    platforms: PLATFORMS,
    syntax: `appHotkeys.onFired(listener: (event: HotkeyEvent) => void): () => void

interface HotkeyEvent {
  readonly id: string;
  readonly phase: "down" | "up";
  readonly cancelled?: true;
}`,
    returns: "A function that removes the listener. Calling it twice does nothing.",
    notes: [
      "`down` fires when the chord completes, `up` when one of its buttons is released.",
      "A synthetic `up` carries `cancelled: true` when a binding changes, capture starts, or the service interrupts a hold. Use it to stop a held action and skip actions that require a physical release.",
      "Every listener gets the same event object. A listener that throws is logged and the others still run.",
      "Events produced before a `setBindings()` that changed the set are dropped, except the `up` edges, so a held key is always released.",
      "In Svelte, return the unsubscribe function from `$effect`; in Vue, call it in `onUnmounted`.",
    ],
    examples: [
      {
        title: "A microphone indicator",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { appHotkeys } from "skathi:app";

const talking = ref(false);
const stop = appHotkeys.onFired((event) => {
  if (event.id === "ptt") talking.value = event.phase === "down";
});
onUnmounted(stop);
</script>

<template>
  <span class="mic" :class="{ live: talking }">{{ talking ? "Talking" : "Muted" }}</span>
</template>`,
      },
      {
        title: "The same in Svelte",
        lang: "svelte",
        code: `<script lang="ts">
  import { appHotkeys } from "skathi:app";

  let talking = $state(false);
  $effect(() =>
    appHotkeys.onFired((event) => {
      if (event.id === "ptt") talking = event.phase === "down";
    }),
  );
</script>

<span class="mic" class:live={talking}>{talking ? "Talking" : "Muted"}</span>`,
      },
    ],
    edgeCases: [
      "Not a function throws a `TypeError`. More than 1024 listeners at once throws an `Error`.",
      "A full reload removes every listener.",
    ],
    limitations: [LINUX],
    related: ["hotkeys-set-bindings", "app-hotkeys"],
    aliases: ["HotkeyEvent", "keydown", "keyup", "push to talk"],
  },
  {
    slug: "hotkeys-capture-once",
    name: "appHotkeys.captureOnce()",
    kind: "method",
    summary: 'Records the next chord the user presses anywhere on the system, for a "press a shortcut" field.',
    platforms: PLATFORMS,
    syntax: "appHotkeys.captureOnce(): Promise<HotkeyChord>",
    returns: "A promise for the chord, settled when every key of it has been released.",
    notes: [
      "Every key or button pressed counts, in the order first pressed, up to 16; the chord is complete when all of them are up again. Modifiers already held when the capture starts are part of it.",
      "Keys pressed during a capture are swallowed: the focused app does not see them, and no binding fires. A `down` already delivered gets its `up` when the capture starts.",
      "The left and right mouse buttons are not recorded, so the user can still click; the middle and side buttons are.",
      "Esc cancels. After 10 s the capture ends: with what was pressed so far, or rejected when nothing was.",
      "It starts the hooks if they are not running.",
    ],
    examples: [
      {
        title: "A shortcut field",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import type { HotkeyChord } from "@skathi-native/runtime";
import { appHotkeys } from "skathi:app";

const emit = defineEmits<{ change: [chord: HotkeyChord] }>();
const recording = ref(false);
const message = ref("");

async function record() {
  recording.value = true;
  message.value = "Press the shortcut, Esc to cancel";
  try {
    emit("change", await appHotkeys.captureOnce());
    message.value = "";
  } catch (error) {
    message.value = String(error).includes("timeout") ? "Nothing was pressed" : "";
  } finally {
    recording.value = false;
  }
}
</script>

<template>
  <button :disabled="recording" @click="record">{{ recording ? "Listening…" : "Set shortcut" }}</button>
  <button v-if="recording" @click="appHotkeys.cancelCapture()">Cancel</button>
  <small>{{ message }}</small>
</template>`,
      },
    ],
    edgeCases: [
      'It rejects with an `Error` whose message is "hotkey capture cancelled" (Esc, `cancelCapture()` or a reload), "hotkey capture timeout" (10 s and nothing pressed) or "hotkey capture superseded" (another `captureOnce()` started).',
      'Where the hooks cannot run it rejects with "hotkeys are not running".',
    ],
    limitations: [MAC_AX, LINUX],
    related: ["hotkeys-cancel-capture", "hotkeys-chord", "hotkeys-set-bindings"],
    aliases: ["record shortcut", "shortcut recorder"],
  },
  {
    slug: "hotkeys-cancel-capture",
    name: "appHotkeys.cancelCapture()",
    kind: "method",
    summary: 'Ends a running `captureOnce()`, which rejects with "hotkey capture cancelled".',
    platforms: PLATFORMS,
    syntax: "appHotkeys.cancelCapture(): void",
    returns: "`undefined`.",
    notes: ["The promise is rejected inside the call. With no capture running, it does nothing."],
    examples: [
      {
        title: "Stop recording when the settings panel closes",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted } from "vue";
import { appHotkeys } from "skathi:app";

onUnmounted(() => appHotkeys.cancelCapture());
</script>

<template>
  <slot />
</template>`,
      },
    ],
    related: ["hotkeys-capture-once"],
  },
  {
    slug: "hotkeys-status",
    name: "appHotkeys.status()",
    kind: "method",
    summary:
      "Reports whether the system hooks are running, why not, and whether macOS Accessibility access is granted.",
    platforms: PLATFORMS,
    syntax: `appHotkeys.status(): HotkeyStatus

interface HotkeyStatus {
  readonly running: boolean;
  readonly platform: string; // "windows", "macos", "linux"
  readonly accessibilityGranted: boolean;
  readonly error: string | null;
  readonly restarts: number;
}`,
    returns: "A fresh object on each call.",
    notes: [
      "`running` is `false` until the first `setBindings()` or `captureOnce()`.",
      "`error` is the last reason the hooks stopped or did not start, or `null`.",
      "`accessibilityGranted` is always `true` on Windows and `false` on Linux.",
      "`restarts` counts the successful `restart()` calls.",
    ],
    examples: [
      {
        title: "Tell the user why shortcuts do nothing",
        lang: "ts",
        code: `import { appHotkeys } from "skathi:app";

export function hotkeyProblem(): string | null {
  const status = appHotkeys.status();
  if (status.running) return null;
  if (status.platform === "macos" && !status.accessibilityGranted)
    return "Allow this app under Privacy & Security › Accessibility, then try again.";
  return status.error;
}`,
      },
    ],
    edgeCases: [
      "On macOS, the system can turn the event tap off later, for example when Accessibility access is revoked; `running` turns `false` and `error` says to restart.",
    ],
    related: ["hotkeys-restart", "hotkeys-request-accessibility", "app-hotkeys"],
    aliases: ["HotkeyStatus"],
  },
  {
    slug: "hotkeys-request-accessibility",
    name: "appHotkeys.requestAccessibility()",
    kind: "method",
    summary:
      "Shows the macOS prompt that sends the user to grant Accessibility access, which global hotkeys need there.",
    platforms: PLATFORMS,
    syntax: "appHotkeys.requestAccessibility(): boolean",
    returns: "Whether access is granted right now: always `true` on Windows, `false` on Linux.",
    notes: [
      "Call it from a user action, such as a button. The grant does not start the hooks: call `restart()` once `status().accessibilityGranted` is `true`.",
      "On Windows, nothing needs granting; it returns `true` and shows nothing.",
    ],
    examples: [
      {
        title: "Grant, then restart",
        lang: "ts",
        code: `import { appHotkeys } from "skathi:app";

export async function enableGlobalShortcuts(): Promise<boolean> {
  if (appHotkeys.requestAccessibility()) return appHotkeys.restart();
  for (let i = 0; i < 60; i++) {
    await new Promise<void>((r) => setTimeout(r, 1000));
    if (appHotkeys.status().accessibilityGranted) return appHotkeys.restart();
  }
  return false;
}`,
      },
    ],
    limitations: [LINUX],
    related: ["hotkeys-status", "hotkeys-restart"],
    aliases: ["Accessibility", "AXIsProcessTrusted", "TCC"],
  },
  {
    slug: "hotkeys-restart",
    name: "appHotkeys.restart()",
    kind: "method",
    summary:
      "Stops and starts the system hooks again, keeping the bindings: after a macOS Accessibility grant, or after a failure.",
    platforms: PLATFORMS,
    syntax: "appHotkeys.restart(): boolean",
    returns: "`true` once the hooks are running again.",
    notes: [
      "Held chords get their `up` first, and a running capture is cancelled.",
      "It counts in `status().restarts`.",
    ],
    examples: [
      {
        title: "A retry button",
        lang: "ts",
        code: `import { appHotkeys } from "skathi:app";

export function retryHotkeys(): string | null {
  try {
    appHotkeys.restart();
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}`,
      },
    ],
    edgeCases: [
      "When the hooks cannot start (macOS without Accessibility, Linux), it throws an `Error` with the reason.",
    ],
    limitations: [LINUX],
    related: ["hotkeys-status", "hotkeys-request-accessibility"],
  },
]);
