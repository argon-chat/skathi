// Wire form of a `postMessage` payload between the host and the platform webview (another
// process). Structured-clone subset: JSON types, `undefined`, non-finite numbers, bigint, Date,
// ArrayBuffer and views, Map, Set, plain objects and arrays; class instances cross as plain
// objects. Functions, symbols, DOM nodes and cycles throw `DataCloneError` in the sender.
//
// crates/sk-webview/src/relay.js carries the same codec by hand; test/iframe-codec.test.ts
// runs every case through both.
//
// Every non-primitive is a tagged array:
//
//   null, boolean, string, finite number      themselves
//   ["u"]                                     undefined
//   ["f", "NaN" | "Infinity" | "-Infinity" | "-0"]
//   ["n", "123"]                              bigint
//   ["d", ms]                                 Date (NaN for an invalid one, as ["d", null])
//   ["a", [..]]                               array (holes become undefined)
//   ["o", [[key, value], ..]]                 plain object, own enumerable string keys
//   ["m", [[key, value], ..]]                 Map
//   ["s", [..]]                               Set
//   ["b", base64]                             ArrayBuffer
//   ["t", "Uint8Array", base64]               a view, by constructor name (DataView too)

const viewConstructors = {
  Int8Array: () => globalThis.Int8Array,
  Uint8Array: () => globalThis.Uint8Array,
  Uint8ClampedArray: () => globalThis.Uint8ClampedArray,
  Int16Array: () => globalThis.Int16Array,
  Uint16Array: () => globalThis.Uint16Array,
  Int32Array: () => globalThis.Int32Array,
  Uint32Array: () => globalThis.Uint32Array,
  Float32Array: () => globalThis.Float32Array,
  Float64Array: () => globalThis.Float64Array,
  BigInt64Array: () => globalThis.BigInt64Array,
  BigUint64Array: () => globalThis.BigUint64Array,
  DataView: () => globalThis.DataView,
};

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_INDEX = (() => {
  const t = new Uint8Array(128).fill(255);
  for (let i = 0; i < B64.length; i++) t[B64.charCodeAt(i)] = i;
  return t;
})();

export function toBase64(bytes: Uint8Array): string {
  const parts: string[] = [];
  let chunk = "";
  const n = bytes.length;
  for (let i = 0; i < n; i += 3) {
    const a = bytes[i]!;
    const b = i + 1 < n ? bytes[i + 1]! : 0;
    const c = i + 2 < n ? bytes[i + 2]! : 0;
    chunk +=
      B64[a >> 2]! +
      B64[((a & 3) << 4) | (b >> 4)]! +
      (i + 1 < n ? B64[((b & 15) << 2) | (c >> 6)] : "=") +
      (i + 2 < n ? B64[c & 63] : "=");
    // One string per 48 KB of input: a megabyte must not be a million concatenations deep.
    if (chunk.length >= 65536) {
      parts.push(chunk);
      chunk = "";
    }
  }
  parts.push(chunk);
  return parts.join("");
}

export function fromBase64(text: string): Uint8Array {
  let len = text.length;
  while (len > 0 && text.charCodeAt(len - 1) === 61) len--;
  const out = new Uint8Array((len * 3) >> 2);
  let o = 0;
  let acc = 0;
  let bits = 0;
  for (let i = 0; i < len; i++) {
    const v = B64_INDEX[text.charCodeAt(i) & 127]!;
    if (v === 255) throw new TypeError("iframe message: bad base64");
    acc = (acc << 6) | v;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[o++] = (acc >> bits) & 255;
    }
  }
  return out;
}

function cloneError(what: string): Error {
  const message = `Failed to execute 'postMessage': ${what} could not be cloned.`;
  if (typeof DOMException === "function") return new DOMException(message, "DataCloneError");
  const e = new Error(message);
  e.name = "DataCloneError";
  return e;
}

function describe(v: unknown): string {
  if (typeof v === "function") return `${(v as { name?: string }).name || "function"}()`;
  if (typeof v === "symbol") return String(v);
  const name = (v as { constructor?: { name?: string } } | null)?.constructor?.name;
  return name ? `#<${name}>` : "An object";
}

type Wire = null | boolean | number | string | unknown[];

export function encodeMessage(value: unknown, isPlatformObject?: (v: object) => boolean): Wire {
  const open = new Set<object>();
  const enc = (v: unknown): Wire => {
    switch (typeof v) {
      case "undefined":
        return ["u"];
      case "boolean":
      case "string":
        return v;
      case "number":
        if (Number.isFinite(v)) return Object.is(v, -0) ? ["f", "-0"] : v;
        return ["f", String(v)];
      case "bigint":
        return ["n", v.toString()];
      case "function":
      case "symbol":
        throw cloneError(describe(v));
    }
    if (v === null) return null;
    const obj = v as object;
    // A browser clones cycles; a text channel cannot without an id table on both sides.
    if (open.has(obj)) throw cloneError("An object with a cycle");
    if (obj instanceof Date) {
      const ms = obj.getTime();
      return ["d", Number.isNaN(ms) ? null : ms];
    }
    if (obj instanceof ArrayBuffer) return ["b", toBase64(new Uint8Array(obj))];
    if (ArrayBuffer.isView(obj)) {
      const name = obj.constructor?.name;
      if (!name || !Object.hasOwn(viewConstructors, name)) throw cloneError(describe(obj));
      return ["t", name, toBase64(new Uint8Array(obj.buffer, obj.byteOffset, obj.byteLength))];
    }
    open.add(obj);
    try {
      if (Array.isArray(obj)) {
        const items: Wire[] = [];
        for (let i = 0; i < obj.length; i++) items.push(enc(obj[i]));
        return ["a", items];
      }
      if (obj instanceof Map) {
        const entries: Wire[] = [];
        for (const [k, val] of obj) entries.push([enc(k), enc(val)]);
        return ["m", entries];
      }
      if (obj instanceof Set) {
        const items: Wire[] = [];
        for (const item of obj) items.push(enc(item));
        return ["s", items];
      }
      // Class left behind, as in a browser's clone; a platform object (DOM node) is refused.
      if (isPlatformObject?.(obj)) throw cloneError(describe(obj));
      const entries: Wire[] = [];
      for (const k of Object.keys(obj)) entries.push([k, enc((obj as Record<string, unknown>)[k])]);
      return ["o", entries];
    } finally {
      open.delete(obj);
    }
  };
  return enc(value);
}

export function decodeMessage(wire: unknown): unknown {
  if (wire === null || typeof wire !== "object") return wire;
  if (!Array.isArray(wire)) throw new TypeError("iframe message: not a wire value");
  const [tag, a, b] = wire as [string, unknown, unknown];
  switch (tag) {
    case "u":
      return undefined;
    case "f":
      return a === "-0" ? -0 : Number(a);
    case "n":
      return BigInt(a as string);
    case "d":
      return new Date(a === null ? NaN : (a as number));
    case "a":
      return (a as unknown[]).map(decodeMessage);
    case "o": {
      const out: Record<string, unknown> = {};
      for (const [k, v] of a as [string, unknown][]) {
        // `__proto__` as an own key, not as the prototype setter.
        Object.defineProperty(out, k, {
          value: decodeMessage(v),
          enumerable: true,
          writable: true,
          configurable: true,
        });
      }
      return out;
    }
    case "m":
      return new Map((a as [unknown, unknown][]).map(([k, v]) => [decodeMessage(k), decodeMessage(v)]));
    case "s":
      return new Set((a as unknown[]).map(decodeMessage));
    case "b":
      return fromBase64(a as string).buffer;
    case "t": {
      if (typeof a !== "string" || !Object.hasOwn(viewConstructors, a))
        throw new TypeError("iframe message: unknown view");
      const bytes = fromBase64(b as string);
      const Ctor: {
        new (buffer: ArrayBufferLike, offset: number, length: number): ArrayBufferView;
        readonly BYTES_PER_ELEMENT?: number;
      } = viewConstructors[a as keyof typeof viewConstructors]();
      const width = "BYTES_PER_ELEMENT" in Ctor ? Ctor.BYTES_PER_ELEMENT : 1;
      return new Ctor(bytes.buffer, 0, bytes.byteLength / width);
    }
  }
  throw new TypeError("iframe message: unknown tag");
}
