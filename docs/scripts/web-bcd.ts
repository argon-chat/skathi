// Fills src/data/web/bcd.ts (MDN link, experimental, deprecated per entry) from @mdn/browser-compat-data.
// Usage: bun scripts/web-bcd.ts [--check] [path to BCD data.json or its package folder]
// An entry BCD does not know fails the run until KEYS names its key, or null for one the web does not have.
import { existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { entries, type WebEntry } from "../src/data/web/entries";

const OUT = join(import.meta.dir, "../src/data/web/bcd.ts");

const KEYS: Readonly<Record<string, string | null>> = {
  "request-animation-frame": "api.Window.requestAnimationFrame",
  "request-idle-callback": "api.Window.requestIdleCallback",
  onerror: "api.Window.error_event",
  "performance-now": "api.Performance.now",
  "performance-time-origin": "api.Performance.timeOrigin",
  "performance-mark": "api.Performance.mark",
  "performance-measure": "api.Performance.measure",
  "web-workers": "api.Worker",
  "audio-worklet": "api.AudioWorklet",
  "elements-and-attributes": "api.Element",
  "get-computed-style": "api.Window.getComputedStyle",
  "element-from-point": "api.Document.elementFromPoint",
  "scroll-into-view": "api.Element.scrollIntoView",
  "match-media": "api.Window.matchMedia",
  contenteditable: "html.global_attributes.contenteditable",
  "fullscreen-api": "api.Element.requestFullscreen",
  "web-animations": "api.Element.animate",
  "view-transitions": "api.Document.startViewTransition",
  "smooth-scrolling": "api.Element.scrollTo.options_behavior_parameter",
  "pointer-lock": "api.Element.requestPointerLock",
  "pointer-event-pointer-type": "api.PointerEvent.pointerType",
  "pointer-event-pressure": "api.PointerEvent.pressure",
  "pointer-event-tangential-pressure": "api.PointerEvent.tangentialPressure",
  "pointer-event-tilt-x": "api.PointerEvent.tiltX",
  "pointer-event-altitude-angle": "api.PointerEvent.altitudeAngle",
  "pointer-event-twist": "api.PointerEvent.twist",
  "pointer-event-get-coalesced-events": "api.PointerEvent.getCoalescedEvents",
  "pointer-event-get-predicted-events": "api.PointerEvent.getPredictedEvents",
  pointerrawupdate: "api.Element.pointerrawupdate_event",
  "document-fonts": "api.Document.fonts",
  "custom-elements": "api.CustomElementRegistry",
  "shadow-dom": "api.ShadowRoot",
  alert: "api.Window.alert",
  confirm: "api.Window.confirm",
  prompt: "api.Window.prompt",
  "window-open": "api.Window.open",
  "accept-language": "http.headers.Accept-Language",
  "navigator-send-beacon": "api.Navigator.sendBeacon",
  "navigator-connection": "api.Navigator.connection",
  "service-workers": "api.ServiceWorker",
  "cache-api": "api.Cache",
  "crypto-get-random-values": "api.Crypto.getRandomValues",
  "crypto-random-uuid": "api.Crypto.randomUUID",
  "local-storage": "api.Window.localStorage",
  "session-storage": "api.Window.sessionStorage",
  "indexed-db": "api.indexedDB",
  "navigator-storage": "api.Navigator.storage",
  "file-system-access": "api.Window.showOpenFilePicker",
  "input-type-file": "html.elements.input.type_file",
  "input-type-color": "html.elements.input.type_color",
  "html-input-element-show-picker": "api.HTMLInputElement.showPicker",
  showpicker: null,
  "a-download": "html.elements.a.download",
  "navigator-locks": "api.Navigator.locks",
  "canvas-2d": "api.CanvasRenderingContext2D",
  webgpu: "api.GPU",
  webgl: "api.WebGLRenderingContext",
  "webgl-2": "api.WebGL2RenderingContext",
  "web-audio": "api.AudioContext",
  "get-user-media": "api.MediaDevices.getUserMedia",
  video: "html.elements.video",
  "navigator-media-session": "api.Navigator.mediaSession",
  webcodecs: null,
  webrtc: "api.RTCPeerConnection",
  "navigator-language": "api.Navigator.language",
  "navigator-languages": "api.Navigator.languages",
  "navigator-user-agent-data": "api.Navigator.userAgentData",
  "navigator-permissions-query": "api.Permissions.query",
  "async-clipboard": "api.Clipboard",
  "navigator-get-battery": "api.Navigator.getBattery",
  "navigator-user-activation": "api.Navigator.userActivation",
  "navigator-window-controls-overlay": "api.Navigator.windowControlsOverlay",
  "launch-queue": "api.Window.launchQueue",
  "navigator-set-app-badge": "api.Navigator.setAppBadge",
  "navigator-clear-app-badge": "api.Navigator.clearAppBadge",
  "navigator-wake-lock": "api.Navigator.wakeLock",
  "navigator-share": "api.Navigator.share",
  "navigator-can-share": "api.Navigator.canShare",
  passkeys: "api.PublicKeyCredential",
  gamepad: "api.Navigator.getGamepads",
  geolocation: "api.Geolocation",
  vibration: "api.Navigator.vibrate",
  "web-bluetooth": "api.Bluetooth",
  webusb: "api.USB",
  webhid: "api.HID",
  "web-serial": "api.Serial",
};

type Json = { readonly [key: string]: unknown };
const isObject = (x: unknown): x is Json => typeof x === "object" && x !== null && !Array.isArray(x);

function candidates(e: WebEntry): string[] {
  if (e.slug in KEYS) {
    const k = KEYS[e.slug];
    return k === null || k === undefined ? [] : [k];
  }
  if (e.skathiOnly) return [];
  switch (e.kind) {
    case "interface":
    case "function":
      return [`api.${e.name}`];
    default:
      return [];
  }
}

interface Found {
  key: string;
  mdn?: string;
  experimental?: true;
  deprecated?: true;
}

function lookup(data: Json, key: string): Found | undefined {
  const parts = key.split(".");
  const chain: Json[] = [];
  let node: unknown = data;
  for (const p of parts) {
    if (!isObject(node)) return undefined;
    node = node[p];
    if (!isObject(node)) return undefined;
    chain.push(node);
  }
  const compat = (n: Json | undefined): Json | undefined =>
    n !== undefined && isObject(n["__compat"]) ? n["__compat"] : undefined;
  const own = compat(chain.at(-1));
  if (own === undefined) return undefined;
  const found: Found = { key };
  // A sub-feature without its own page links to the nearest feature that has one (`api.X` is the shallowest).
  for (let i = chain.length - 1; i >= 1; i--) {
    const url = compat(chain[i])?.["mdn_url"];
    if (typeof url === "string") {
      found.mdn = url;
      break;
    }
  }
  const status = own["status"];
  if (isObject(status)) {
    if (status["experimental"] === true) found.experimental = true;
    if (status["deprecated"] === true) found.deprecated = true;
  }
  return found;
}

function render(found: ReadonlyMap<string, Found>): string {
  // Expanded objects with unquoted identifier keys, so `bun fmt` leaves the file as written.
  const key = (k: string) => (/^[A-Za-z_$][\w$]*$/.test(k) ? k : JSON.stringify(k));
  const lines = [...found].flatMap(([slug, f]) => [
    `  ${key(slug)}: {`,
    ...Object.entries(f).map(([k, v]) => `    ${k}: ${JSON.stringify(v)},`),
    "  },",
  ]);
  return [
    "// Generated by scripts/web-bcd.ts from @mdn/browser-compat-data; do not edit by hand.",
    "export interface BcdInfo {",
    "  key: string;",
    "  mdn?: string;",
    "  experimental?: true;",
    "  deprecated?: true;",
    "}",
    "",
    "export const bcd: Readonly<Record<string, BcdInfo>> = {",
    ...lines,
    "};",
    "",
  ].join("\n");
}

const flags = process.argv.slice(2).filter((a) => a.startsWith("--"));
const source =
  process.argv.slice(2).find((a) => !a.startsWith("--")) ??
  Bun.resolveSync("@mdn/browser-compat-data", import.meta.dir);
const file = existsSync(source) && statSync(source).isDirectory() ? join(source, "data.json") : source;
const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
if (!isObject(parsed)) throw new Error(`${file} is not BCD data`);
const meta = parsed["__meta"];
const version = isObject(meta) && typeof meta["version"] === "string" ? meta["version"] : "unknown";

const found = new Map<string, Found>();
const unknown: WebEntry[] = [];
const unlisted: WebEntry[] = [];
const missingKeys: string[] = [];
for (const e of entries) {
  const keys = candidates(e);
  const hit = keys.map((k) => lookup(parsed, k)).find((f) => f !== undefined);
  if (hit === undefined) {
    if (!e.skathiOnly) unknown.push(e);
    if (!e.skathiOnly && !(e.slug in KEYS)) unlisted.push(e);
    if (keys.length > 0) missingKeys.push(`${e.slug} (${keys.join(", ")})`);
    continue;
  }
  found.set(e.slug, hit);
}

const stale = Object.keys(KEYS).filter((slug) => !entries.some((e) => e.slug === slug));
const text = render(found);
console.log(`BCD ${version}: ${found.size} of ${entries.length} entries matched`);
console.log(`not in BCD (no mdn): ${unknown.map((e) => e.slug).join(", ")}`);
console.log(
  `experimental: ${[...found]
    .filter(([, f]) => f.experimental)
    .map(([s]) => s)
    .join(", ")}`,
);
console.log(
  `deprecated on the web: ${[...found]
    .filter(([, f]) => f.deprecated)
    .map(([s]) => s)
    .join(", ")}`,
);
console.log(
  `matched without an MDN page: ${[...found]
    .filter(([, f]) => f.mdn === undefined)
    .map(([s]) => s)
    .join(", ")}`,
);
if (missingKeys.length > 0) console.log(`keys not found in BCD: ${missingKeys.join(", ")}`);
if (stale.length > 0) console.log(`key overrides for missing slugs: ${stale.join(", ")}`);

if (unlisted.length > 0) {
  console.error(
    `no BCD key for: ${unlisted.map((e) => e.slug).join(", ")}\n` +
      "Give each a key in KEYS of scripts/web-bcd.ts, or null when MDN has no page for it.",
  );
  process.exit(1);
}

if (flags.includes("--check")) {
  const current = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (current !== text) {
    console.error("src/data/web/bcd.ts is out of date; run `bun run bcd` in docs");
    process.exit(1);
  }
} else {
  writeFileSync(OUT, text);
  console.log(`wrote ${OUT}`);
}
