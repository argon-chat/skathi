// opaque-host-v1: the production bundle, wrapped so the host's globals are opaque inputs to the
// native frontend. It must not assume that this host's URL, fetch, WebSocket or Blob have Bun's
// layouts or helpers; the host binds `__SK_AOT_HOST__` to the realm's globalThis before the entry runs.
import HOST_TABLE from "./host-globals.json";

export const TRANSFORM = "opaque-host-v1";
export const PRELUDES_TRANSFORM = "opaque-host-preludes-v2";
export type PreludeScripts = readonly [string, string];

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

/** A builtin global stays read-only unless the image spells out an assignment to it; writes through
 * `__sk_host` (the runtime wraps setTimeout and WebSocket) are not spelled out, so this dead branch is. */
function openHostGlobals(names: readonly string[]): string {
  const host = new Set(HOST_TABLE.names);
  const opened = names.filter((name) => host.has(name));
  return `if (__SK_AOT_HOST__ === undefined) {\n${opened.map((name) => `globalThis.${name} = undefined;`).join("\n")}\n}\n`;
}

function hostPrefix(names: readonly string[]): string {
  return `// ${TRANSFORM}: stable host API references are opaque inputs.\n${openHostGlobals(names)}(function(__sk_host) {\nconst globalThis = __sk_host;\nconst {${names.join(",")}} = __sk_host;\n`;
}

const hostSuffix = "\n})(__SK_AOT_HOST__);\n";

export function opaqueHost(source: string, names: readonly string[] = opaqueGlobals()): string {
  return hostPrefix(names) + source + hostSuffix;
}

/** Each script captures its host globals after the preceding script has installed them. */
export function nativeInput(source: string, preludes?: PreludeScripts) {
  const scripts = preludes
    ? [
        { file: "sk://prelude.js", source: preludes[0] },
        { file: "sk://window.js", source: preludes[1] },
      ]
    : [];
  scripts.push({ file: "app.prod.js", source });
  const prefix = hostPrefix(opaqueGlobals());
  let code = "";
  let byteOffset = Buffer.byteLength(prefix);
  const sections = scripts.map((script) => {
    const section = { ...script, byteOffset };
    code += prefix + script.source + hostSuffix;
    byteOffset += Buffer.byteLength(script.source) + Buffer.byteLength(hostSuffix) + Buffer.byteLength(prefix);
    return section;
  });
  return { code, sections };
}
