import type { Details } from "../../details";

export const details: Details = {
  "navigator-language": {
    examples: [
      {
        title: "Format prices in the user's language",
        lang: "ts",
        code: `const price = new Intl.NumberFormat(navigator.language, {
  style: "currency",
  currency: "EUR",
});
totalLabel.value = price.format(order.total);`,
      },
    ],
    edgeCases: [
      '`navigator.language` is `navigator.languages[0]`, or `"en-US"` when the system reports none.',
      "It is read from the system once, when the app starts: a change in the system settings shows after a restart, and no `languagechange` event fires.",
      "`appLocale.language` does not change it. That override applies to the SDK's own dialogs and to `appLocale.t`.",
    ],
  },
  "navigator-languages": {
    examples: [
      {
        title: "Pick the best of your app's translations",
        lang: "ts",
        code: `const available = ["en", "de", "ru", "pt-BR"];

function pickLocale(): string {
  for (const tag of navigator.languages) {
    if (available.includes(tag)) return tag;
    const base = tag.split("-")[0];
    if (available.includes(base)) return base;
  }
  return "en";
}`,
      },
    ],
    edgeCases: [
      "The array is frozen and is the same object on every read.",
      "System spellings become BCP 47 tags: `pt_BR.UTF-8@euro` reads as `pt-BR`, and `C` or `POSIX` is skipped. Duplicates are dropped.",
    ],
  },
  "navigator-user-agent-data": {
    examples: [
      {
        title: "Detect Windows 11",
        lang: "ts",
        code: `async function isWindows11(): Promise<boolean> {
  const ua = navigator.userAgentData;
  if (ua.platform !== "Windows") return false;
  const { platformVersion } = await ua.getHighEntropyValues(["platformVersion"]);
  return Number(platformVersion.split(".")[0]) >= 13;
}`,
      },
      {
        title: "Attach platform details to a bug report",
        lang: "ts",
        code: `const details = await navigator.userAgentData.getHighEntropyValues([
  "architecture",
  "bitness",
  "platformVersion",
  "uaFullVersion",
]);
report.environment = JSON.stringify(details);`,
      },
    ],
    edgeCases: [
      '`platform` is `"Windows"`, `"macOS"` or `"Linux"`, and `mobile` is always `false`.',
      "On Windows, `platformVersion` follows Chromium: 13.0.0 and above is Windows 11, 1.0.0 to 10.0.0 is Windows 10.",
      "Unknown hints are left out of the result. `brands`, `mobile` and `platform` are always included.",
      "`brands` holds a GREASE entry (`Not)A;Brand`) and the runtime's own product, with major versions; `fullVersionList` has the full ones.",
      '`architecture` is `"x86"` or `"arm"`, `formFactors` is `["Desktop"]`, `model` is `""` and `wow64` is `false`.',
    ],
  },
  "navigator-permissions-query": {
    examples: [
      {
        title: "Explain the microphone permission in settings",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const mic = ref<PermissionState>("prompt");

onMounted(async () => {
  const status = await navigator.permissions.query({ name: "microphone" as PermissionName });
  mic.value = status.state;
  status.addEventListener("change", () => (mic.value = status.state));
});
</script>

<template>
  <p v-if="mic === 'denied'">Microphone access is blocked. Turn it on in Settings.</p>
  <p v-else-if="mic === 'prompt'">You will be asked the first time you join a call.</p>
</template>`,
      },
    ],
    edgeCases: [
      "The names are `microphone`, `camera`, `display-capture`, `notifications`, `clipboard-read`, `clipboard-write` and `geolocation`. Any other name rejects with a `TypeError`.",
      'A capability the platform has no consent setting for reads `"denied"`.',
      "`query()` returns the same `PermissionStatus` for the same name, so two `onchange` assignments overwrite each other. Use `addEventListener`.",
      'On Windows, `clipboard-read` and `clipboard-write` are always `"granted"`.',
      'Dismissing the microphone prompt keeps the state at `"prompt"`. Block makes it `"denied"`, and the answer is kept across restarts.',
    ],
  },
  "async-clipboard": {
    examples: [
      {
        title: "Copy a share link",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const props = defineProps<{ link: string }>();
const copied = ref(false);

async function copy() {
  await navigator.clipboard.writeText(props.link);
  copied.value = true;
  setTimeout(() => (copied.value = false), 1500);
}
</script>

<template>
  <button @click="copy">{{ copied ? "Copied" : "Copy link" }}</button>
</template>`,
      },
      {
        title: "Copy a chart as an image, with a text fallback",
        lang: "ts",
        code: `async function copyChart(canvas: HTMLCanvasElement, summary: string) {
  const png = await new Promise<Blob>((resolve) => canvas.toBlob((b) => resolve(b!), "image/png"));
  try {
    await navigator.clipboard.write([
      new ClipboardItem({ "image/png": png, "text/plain": new Blob([summary], { type: "text/plain" }) }),
    ]);
  } catch {
    await navigator.clipboard.writeText(summary);
  }
}`,
        note: "Item writes work on Windows. On macOS and Linux `write()` rejects and the text fallback runs.",
      },
    ],
    edgeCases: [
      "No user gesture or window focus is needed, for reading or for writing.",
      "`write()` takes one `ClipboardItem`; more rejects with `NotAllowedError`. A type the platform cannot write, such as `image/svg+xml`, rejects with `NotAllowedError` naming it.",
      "`image/png` data that is not a PNG rejects with `DataError`, and `read()` on an empty clipboard rejects with `DataError`.",
      '`document.execCommand("copy")` does nothing and returns `false`.',
    ],
  },
  "navigator-get-battery": {
    examples: [
      {
        title: "Pause background sync on low battery",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const lowPower = ref(false);

onMounted(async () => {
  const battery = await navigator.getBattery();
  const update = () => (lowPower.value = !battery.charging && battery.level < 0.2);
  update();
  battery.addEventListener("levelchange", update);
  battery.addEventListener("chargingchange", update);
});
</script>

<template>
  <p v-if="lowPower" class="hint">Battery is low: background sync is paused.</p>
</template>`,
      },
    ],
    edgeCases: [
      "A machine without a battery reports `charging: true`, `level: 1`, `chargingTime: 0` and `dischargingTime: Infinity`.",
      "On Windows, `chargingTime` is `Infinity` until the battery is full.",
      "Every call returns the same promise and the same `BatteryManager`.",
    ],
  },
  "navigator-user-activation": {
    examples: [
      {
        title: "Ask for a second click when the gesture has expired",
        lang: "ts",
        code: `async function shareReport(report: File) {
  if (!navigator.userActivation.isActive) {
    showToast("Click Share again to open the share sheet");
    return;
  }
  await navigator.share({ files: [report] });
}`,
      },
    ],
    edgeCases: [
      "`isActive` is `true` for 5 seconds after a key press, mouse press or click from the system. Escape and modifier keys alone do not count.",
      "`el.click()` and `dispatchEvent` never activate.",
      "Opening the color picker of an `<input type=color>` uses up the activation.",
      "`hasBeenActive` stays `true` after the first activation.",
    ],
  },
  "navigator-window-controls-overlay": {
    examples: [
      {
        title: "Keep the title bar clear of the window buttons",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const wco = navigator.windowControlsOverlay;
const width = ref(0);
const update = () => (width.value = wco.visible ? wco.getTitlebarAreaRect().width : 0);

onMounted(() => {
  update();
  wco.addEventListener("geometrychange", update);
});
onUnmounted(() => wco.removeEventListener("geometrychange", update));
</script>

<template>
  <header class="titlebar" :style="width ? { width: width + 'px' } : {}">
    <span class="title">Inbox</span>
  </header>
</template>`,
      },
      {
        title: "Fit the title bar with CSS alone",
        lang: "css",
        code: `.titlebar {
  position: fixed;
  left: env(titlebar-area-x, 0px);
  top: env(titlebar-area-y, 0px);
  width: env(titlebar-area-width, 100%);
  height: env(titlebar-area-height, 36px);
  app-region: drag;
}`,
      },
    ],
    edgeCases: [
      "`visible` is `false`, and `getTitlebarAreaRect()` is all zeros, in fullscreen, while minimized and while the host draws its fallback title bar.",
      "The rect keeps `x = 0` and loses the width of the window buttons, which sit at the right.",
      "`getTitlebarAreaRect()` returns a plain object with `x`, `y`, `width`, `height`, `top`, `right`, `bottom` and `left`, not a `DOMRect`.",
      "`ongeometrychange` runs before the listeners added with `addEventListener`.",
    ],
  },
  "launch-queue": {
    examples: [
      {
        title: "Open the files and links the app was launched with",
        lang: "ts",
        code: `launchQueue.setConsumer(async (params) => {
  if (params.targetURL) openDeepLink(new URL(params.targetURL));
  for (const handle of params.files) {
    const file = await handle.getFile();
    openDocument(file.name, await file.text());
  }
});`,
      },
    ],
    edgeCases: [
      "Launches wait in a queue until `setConsumer` is called, then arrive in order. The app's own launch is one of them, with `targetURL` `null` and no files when nothing was passed.",
      "With a single instance, every later launch of the app reaches the running instance's consumer.",
      "A consumer that throws is reported with `console.error`, and the next launch still arrives.",
      "A new consumer replaces the old one. Anything but a function throws a `TypeError`.",
      "`files` holds `FileSystemFileHandle`s.",
    ],
  },
  "navigator-set-app-badge": {
    examples: [
      {
        title: "Show the unread count on the app icon",
        lang: "ts",
        code: `import { watch } from "vue";

watch(unreadCount, (n) => {
  if (n > 0) navigator.setAppBadge(n);
  else navigator.clearAppBadge();
});`,
      },
      {
        title: "Flag that something needs attention",
        lang: "ts",
        code: `function onMention() {
  if (document.hidden) {
    navigator.setAppBadge();
  }
}`,
        note: "Without a count the badge is a dot.",
      },
    ],
    edgeCases: [
      "Counts above 99 show as `99+`, and fractions are truncated.",
      "`0` clears the badge.",
      "A negative, `NaN` or infinite count rejects with a `TypeError`.",
    ],
  },
  "navigator-clear-app-badge": {
    examples: [
      {
        title: "Clear the badge when the window gets focus",
        lang: "ts",
        code: `window.addEventListener("focus", () => {
  if (unreadCount.value === 0) {
    navigator.clearAppBadge();
  }
});`,
      },
    ],
    edgeCases: ["It does what `setAppBadge(0)` does, and resolves even when no badge is shown."],
  },
  "navigator-wake-lock": {
    examples: [
      {
        title: "Keep the screen on during a presentation",
        lang: "ts",
        code: `let lock: WakeLockSentinel | null = null;

async function keepAwake() {
  lock = await navigator.wakeLock.request("screen");
  lock.addEventListener("release", () => (lock = null));
}

document.addEventListener("visibilitychange", () => {
  if (!document.hidden && presenting.value && !lock) keepAwake();
});`,
        note: "The lock is released while the window is hidden, so it is requested again when the window comes back.",
      },
    ],
    edgeCases: [
      "Requesting while the window is hidden rejects with `NotAllowedError`, and so does a refusal from the system.",
      'Any type but `"screen"` rejects with a `TypeError`.',
      "All sentinels share one system request, which ends when the last one is released. A second `release()` does nothing.",
    ],
  },
  "navigator-share": {
    examples: [
      {
        title: "Share a link, or copy it if the user cancels",
        lang: "vue",
        code: `<script setup lang="ts">
const props = defineProps<{ title: string; url: string }>();

async function share() {
  try {
    await navigator.share({ title: props.title, url: props.url });
  } catch (e) {
    if ((e as DOMException).name !== "AbortError") await navigator.clipboard.writeText(props.url);
  }
}
</script>

<template>
  <button @click="share">Share</button>
</template>`,
      },
    ],
    edgeCases: [
      "It needs a user activation: outside one it rejects with `NotAllowedError`.",
      "Closing the share sheet rejects with `AbortError`.",
      "A second call while the sheet is open rejects with `InvalidStateError`.",
      "A relative `url` is resolved against `location.href`. An empty object rejects with a `TypeError`.",
    ],
  },
  "navigator-can-share": {
    examples: [
      {
        title: "Offer sharing only for data the system can take",
        lang: "ts",
        code: `const data = { files: [invoicePdf], title: "Invoice 2026-11" };
if (navigator.canShare(data)) {
  await navigator.share(data);
} else {
  saveCopy(invoicePdf);
}`,
      },
    ],
    edgeCases: [
      "It returns `false` for `{}`, for no argument, for `{ files: [] }`, for an invalid `url`, and for a `files` entry that is not a `File`.",
      "It does not check for a user activation, so `true` does not mean `share()` will open.",
    ],
  },
  passkeys: {
    examples: [
      {
        title: "Register a passkey",
        lang: "ts",
        code: `const AUTH = "https://auth.example.com";
const begin = await fetch(AUTH + "/passkeys/register/begin", { method: "POST", credentials: "include" });
const json = await begin.json();
const publicKey = PublicKeyCredential.parseCreationOptionsFromJSON({ ...json, rp: { ...json.rp, id: "example.com" } });
const credential = (await navigator.credentials.create({ publicKey })) as PublicKeyCredential;
await fetch(AUTH + "/passkeys/register/finish", {
  method: "POST",
  credentials: "include",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(credential.toJSON()),
});`,
      },
      {
        title: "Sign in with a passkey",
        lang: "ts",
        code: `const AUTH = "https://auth.example.com";
const begin = await fetch(AUTH + "/passkeys/login/begin", { method: "POST", credentials: "include" });
const publicKey = PublicKeyCredential.parseRequestOptionsFromJSON({ ...(await begin.json()), rpId: "example.com" });
const assertion = (await navigator.credentials.get({ publicKey })) as PublicKeyCredential;
const res = await fetch(AUTH + "/passkeys/login/finish", {
  method: "POST",
  credentials: "include",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(assertion.toJSON()),
});`,
      },
    ],
    edgeCases: [
      "Without `rp.id` or `rpId`, or with an IP address or an invalid domain, `create()` and `get()` reject with `SecurityError`.",
      "`user.id` must be 1 to 64 bytes, or `create()` rejects with a `TypeError`.",
      "Without `pubKeyCredParams`, ES256 (`-7`) and RS256 (`-257`) are offered.",
      "`timeout` is clamped to between 10 seconds and 10 minutes; the default is 5 minutes.",
      '`mediation: "conditional"`, password credentials and `store()` reject with `NotSupportedError`.',
    ],
  },
  notification: {
    edgeCases: [
      '`Notification.permission` is always `"denied"`, and `Notification.requestPermission()` resolves `"denied"`, calling a callback argument with it too.',
      "`new Notification(title)` shows nothing and logs an advisory once. Without a title it throws a `TypeError`.",
    ],
  },
  gamepad: {
    edgeCases: ["`navigator.getGamepads` is `undefined`."],
  },
  geolocation: {
    edgeCases: [
      '`navigator.geolocation` is `undefined`, although `navigator.permissions.query({ name: "geolocation" })` answers.',
    ],
  },
  vibration: {
    edgeCases: ["`navigator.vibrate` is `undefined`."],
  },
  "web-bluetooth": {
    edgeCases: ["`navigator.bluetooth` is `undefined`."],
  },
  webusb: {
    edgeCases: ["`navigator.usb` is `undefined`."],
  },
  webhid: {
    edgeCases: ["`navigator.hid` is `undefined`."],
  },
  "web-serial": {
    edgeCases: ["`navigator.serial` is `undefined`."],
  },
};
