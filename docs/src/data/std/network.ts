import { OK, PART, importLine, os, section } from "./shared";

const IMPORT = importLine("skathi:network", "network");
const WINDOWS_2004 =
  'Windows needs Windows 10 2004 or later. Before it `online` reads `true`, `internet` reads `"unknown"`, the three flags read `null` and no event fires.';
const MAC =
  'On macOS, `internet` reads `"none"` without a network and `"unknown"` with one: the system has no connectivity check to ask. `roaming` and `overDataLimit` read `null`.';
const LINUX =
  'On Linux, `internet` and `metered` come from NetworkManager; on a system without it they read `"unknown"` and `null`. `roaming` and `overDataLimit` read `null`.';

export const network = section("skathi:network", [
  {
    slug: "network",
    name: "network",
    kind: "object",
    summary:
      "Reads what the OS knows about the connection: online, internet reachability, connection type, metered and roaming, with change events.",
    platforms: os(OK, PART, PART),
    syntax: `${IMPORT}

readonly network.online: boolean
readonly network.internet: "full" | "limited" | "portal" | "none" | "unknown"
readonly network.type: "ethernet" | "wifi" | "cellular" | "other" | "none" | "unknown"
readonly network.metered: boolean | null
readonly network.roaming: boolean | null
readonly network.overDataLimit: boolean | null

network.addEventListener(type: "change" | "online" | "offline", listener: (event: Event) => void): void`,
    notes: [
      "`network` is an `EventTarget`. Every property is read synchronously.",
      "`online` is `navigator.onLine`: the OS believes there is a network. A LAN without internet and a captive portal are both online.",
      "`internet` is the operating system's own connectivity check. Skathi sends no probe of its own; an app that needs its server checks its server. See [Internet](/std/internet/).",
      "`type` is the physical connection traffic takes. See [ConnectionType](/std/connection-type/).",
      "`navigator.connection` is the standard view of the same state: its `type` is always `network.type`. See [navigator.connection](/web-platform/navigator-connection/).",
      "`metered` says the connection is billed by use. `metered`, `roaming` and `overDataLimit` are `null` when the OS cannot tell.",
      "On Windows, `metered` is `true` for a fixed or variable-cost connection and `false` for an unrestricted one. `roaming` and `overDataLimit` come from the same system cost hint.",
      "On macOS, `metered` is `true` on a connection the system marks as expensive, such as cellular or a personal hotspot. On Linux it is NetworkManager's metered setting, its guess included.",
      "`online` and `offline` fire together with `window`'s events of the same name. Each is followed by a `change` on `network`.",
      "`change` also fires when `internet`, `metered`, `roaming` or `overDataLimit` changes, and on macOS and Linux when `type` does.",
      "A change towards online is announced after 500 ms and a change towards offline after 1.5 s. A switch between networks that passes through no network inside that time fires neither `offline` nor `online`.",
    ],
    limitations: [MAC, LINUX, WINDOWS_2004],
    examples: [
      {
        title: "Sync less on a metered connection",
        lang: "ts",
        code: `import { network } from "skathi:network";

type SyncPolicy = "wifi-only" | "always";

declare function applyPolicy(policy: SyncPolicy): void;

function syncPolicy(): SyncPolicy {
  return network.metered || network.roaming ? "wifi-only" : "always";
}

applyPolicy(syncPolicy());
network.addEventListener("change", () => applyPolicy(syncPolicy()));`,
      },
      {
        title: "Show an offline banner",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import { network } from "skathi:network";

const online = ref(network.online);
const portal = ref(network.internet === "portal");

function update(): void {
  online.value = network.online;
  portal.value = network.internet === "portal";
}

network.addEventListener("change", update);
onBeforeUnmount(() => network.removeEventListener("change", update));
</script>

<template>
  <p v-if="!online">You are offline. Changes are saved on this computer.</p>
  <p v-else-if="portal">This network asks you to sign in before it gives internet access.</p>
</template>`,
      },
    ],
    edgeCases: [
      'No property throws. A value the OS cannot give reads as `"unknown"` or `null`, so test `metered === true`, not `metered !== false`, when the answer must be certain.',
      "`online: true` does not mean a server is reachable. A request can still fail.",
    ],
    related: ["internet", "connection-type", "os"],
    aliases: [
      "skathi:network",
      "Network",
      "network.online",
      "network.internet",
      "network.type",
      "network.metered",
      "network.roaming",
      "network.overDataLimit",
      "navigator.onLine",
      "offline",
      "online event",
      "change event",
      "metered connection",
      "data saver",
    ],
  },
  {
    slug: "internet",
    name: "Internet",
    kind: "type",
    summary: "The values of `network.internet`: the result of the operating system's own connectivity check.",
    platforms: os(OK, PART, PART),
    syntax: `import type { Internet } from "skathi:network";

type Internet = "full" | "limited" | "portal" | "none" | "unknown";`,
    values: [
      { value: "full", status: OK, note: "The OS reached the internet." },
      { value: "limited", status: OK, note: "A network without internet." },
      { value: "portal", status: OK, note: "A captive portal: the network asks for a sign-in first." },
      { value: "none", status: OK, note: "No network." },
      { value: "unknown", status: OK, note: "The OS cannot tell." },
    ],
    notes: [
      "On Windows the value comes from the system's connectivity check, on Linux from NetworkManager's.",
      "Skathi sends no probe of its own. An app that needs its server checks its server.",
      '`"limited"` and `"portal"` are both online: `network.online` is `true` for them.',
    ],
    limitations: [
      'On macOS the value is `"none"` without a network and `"unknown"` with one: the system has no connectivity check to ask.',
      'On Linux without NetworkManager the value is `"unknown"`.',
      'On Windows before Windows 10 2004 the value is `"unknown"`.',
    ],
    examples: [
      {
        title: "Explain why a request may fail",
        lang: "ts",
        code: `import { network, type Internet } from "skathi:network";

const hints: Record<Internet, string | null> = {
  full: null,
  unknown: null,
  limited: "This network has no internet access.",
  portal: "Sign in to this network in a browser first.",
  none: "There is no network connection.",
};

export function connectionHint(): string | null {
  return hints[network.internet];
}`,
      },
    ],
    related: ["network", "connection-type"],
    aliases: ["network.internet", "captive portal", "connectivity", "reachability", "limited connectivity"],
  },
  {
    slug: "connection-type",
    name: "ConnectionType",
    kind: "type",
    summary: "The values of `network.type`: the physical connection traffic takes.",
    platforms: os(OK, OK, OK),
    syntax: `import type { ConnectionType } from "skathi:network";

type ConnectionType = "ethernet" | "wifi" | "cellular" | "other" | "none" | "unknown";`,
    values: [
      { value: "ethernet", status: OK },
      { value: "wifi", status: OK },
      { value: "cellular", status: OK },
      { value: "other", status: OK, note: "Another kind of adapter, or nothing but virtual adapters." },
      { value: "none", status: OK, note: "No connected adapter has a default gateway." },
      { value: "unknown", status: OK, note: "The OS cannot tell." },
    ],
    notes: [
      "On Windows it is the type of the connected adapter with a default gateway. With several, the one with the lowest route metric wins.",
      'Behind a VPN it is the adapter under the tunnel, and `"other"` only when there is nothing but virtual adapters.',
      "On macOS it is the interface of the system's current network path. On Linux it is the interface of the default route with the lowest metric.",
      "On Windows the first read after a change asks the OS, which takes milliseconds. Later reads return the stored value.",
    ],
    examples: [
      {
        title: "Hold large downloads on cellular",
        lang: "ts",
        code: `import { network } from "skathi:network";

export function canDownloadNow(bytes: number): boolean {
  if (network.type === "cellular" || network.metered === true) return bytes < 5_000_000;
  return network.online;
}`,
      },
    ],
    related: ["network", "internet"],
    aliases: ["network.type", "wifi", "ethernet", "cellular", "VPN", "adapter"],
  },
]);
