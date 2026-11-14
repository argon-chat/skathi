// opaque-host-v1: the production bundle, wrapped so the host's globals are opaque inputs to the
// native frontend. It must not assume that this host's URL, fetch, WebSocket or Blob have Bun's
// layouts or helpers; the host binds `__SK_AOT_HOST__` to the realm's globalThis before the entry runs.
import HOST_TABLE from "./host-globals.json";

export const TRANSFORM = "opaque-host-v1";

export interface BuiltinEntry {
  holder: string;
  origin: string;
  isSymbolName: boolean;
  name: string;
}

/** The host-implemented globals of a builtin table dump: what host-globals.json records. */
export function hostGlobalsFromTable(entries: BuiltinEntry[]): string[] {
  return [
    ...new Set(
      entries
        .filter(
          (entry) =>
            entry.holder === "GlobalThis" &&
            entry.origin === "Host" &&
            !entry.isSymbolName &&
            /^[A-Za-z_$][\w$]*$/.test(entry.name),
        )
        .map((entry) => entry.name),
    ),
  ].sort();
}

/** Browser globals this host provides that the table does not describe as host APIs. */
const BROWSER_GLOBALS = [
  "Object",
  "console",
  "window",
  "self",
  "document",
  "navigator",
  "location",
  "performance",
  "requestAnimationFrame",
  "cancelAnimationFrame",
  "ResizeObserver",
  "MutationObserver",
  "IntersectionObserver",
  "Image",
  "Audio",
  "AudioContext",
  "OffscreenCanvas",
  "Path2D",
  "ImageData",
  "createImageBitmap",
  "localStorage",
  "sessionStorage",
  "indexedDB",
  "matchMedia",
  "getComputedStyle",
];

export const TABLE_ID: string = HOST_TABLE.table;

export function opaqueGlobals(tableGlobals: readonly string[] = HOST_TABLE.names): string[] {
  return [...new Set([...tableGlobals, ...BROWSER_GLOBALS])].sort();
}

export function opaqueHost(source: string, names: readonly string[] = opaqueGlobals()): string {
  return `// ${TRANSFORM}: stable host API references are opaque inputs.\n(function(__sk_host) {\nconst globalThis = __sk_host;\nconst {${names.join(",")}} = __sk_host;\n${source}\n})(__SK_AOT_HOST__);\n`;
}
