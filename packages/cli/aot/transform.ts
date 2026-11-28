// opaque-host-v1: the production bundle, wrapped so the host's globals are opaque inputs to the
// native frontend. It must not assume that this host's URL, fetch, WebSocket or Blob have Bun's
// layouts or helpers; the host binds `__SK_AOT_HOST__` to the realm's globalThis before the entry runs.
import HOST_TABLE from "./host-globals.json";

export const TRANSFORM = "opaque-host-v1";
export const PRELUDES_TRANSFORM = "opaque-host-preludes-v2";
export const CLOSED_PRELUDES_TRANSFORM = "opaque-host-closed-preludes-v3";
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
  "PressureObserver",
  "PressureRecord",
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

// Browser bundles can probe CommonJS, including Bun's own helper for dynamic require().
// Resolve those probes on the embedding realm. A bare require in the native frontend
// selects Bun's ModuleRequire builtin, whose module loader a browser host does not install.
const COMMONJS_GLOBALS = ["require", "module", "exports"];

export const TABLE_ID: string = HOST_TABLE.table;

export function opaqueGlobals(tableGlobals: readonly string[] = HOST_TABLE.names): string[] {
  return [...new Set([...tableGlobals, ...BROWSER_GLOBALS, ...COMMONJS_GLOBALS])].sort();
}

/** A builtin global stays read-only unless the image spells out an assignment to it; writes through
 * `__sk_host` (the runtime wraps setTimeout and WebSocket) are not spelled out, so this dead branch is. */
function openHostGlobals(names: readonly string[]): string {
  const host = new Set(HOST_TABLE.names);
  const opened = names.filter((name) => host.has(name));
  return `if (__SK_AOT_HOST__ === undefined) {\n${opened.map((name) => `globalThis.${name} = undefined;`).join("\n")}\n}\n`;
}

function hostScope(names: readonly string[], classic = false): [string, string] {
  const start = classic ? "(function(__sk_host) {" : "{\nconst __sk_host = __SK_AOT_HOST__;";
  const prefix = `// ${TRANSFORM}: stable host API references are opaque inputs.\n${openHostGlobals(names)}${start}\nconst globalThis = __sk_host;\nconst {${names.join(",")}} = __sk_host;\n{\n`;
  return [prefix, classic ? "\n}\n})(__SK_AOT_HOST__);\n" : "\n}\n}\n"];
}

export function opaqueHost(source: string, names: readonly string[] = opaqueGlobals()): string {
  const [prefix, suffix] = hostScope(names);
  return prefix + source + suffix;
}

/** The host binds globals before the entry that the preludes probe with `typeof`. An open image admits them only
 * if it may load code it has not seen, which an app that evaluates no text of its own would not; this dead branch
 * does, as every app that imports the runtime already does. */
const admitHostCode = "if (__SK_AOT_HOST__ === undefined) Function();\n";

/** Each script captures its host globals after the preceding script has installed them. */
export function nativeInput(source: string, preludes?: PreludeScripts, closed = false) {
  const scripts = preludes
    ? [
        { file: "sk://prelude.js", source: preludes[0] },
        { file: "sk://window.js", source: preludes[1] },
      ]
    : [];
  scripts.push({ file: "app.prod.js", source });
  const names = opaqueGlobals();
  let code = closed ? "" : admitHostCode;
  let byteOffset = Buffer.byteLength(code);
  const sections = scripts.map((script) => {
    const [prefix, suffix] = hostScope(names, script.file !== "app.prod.js");
    byteOffset += Buffer.byteLength(prefix);
    const section = { ...script, byteOffset };
    code += prefix + script.source + suffix;
    byteOffset += Buffer.byteLength(script.source) + Buffer.byteLength(suffix);
    return section;
  });
  return { code, sections };
}
