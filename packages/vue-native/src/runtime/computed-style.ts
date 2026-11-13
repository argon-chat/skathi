import type { MirrorNode, MirrorRect } from "./types";
import { styleDecls, theGlobal } from "./interfaces";
import { hyphenate } from "./inline-style";
import { warnCompat } from "../compat";
import { defineGlobal } from "./globals";
import { appRoot } from "../runtime";

export function asDomRect(r: MirrorRect): MirrorRect {
  const C = theGlobal("DOMRect");
  return typeof C === "function"
    ? new (C as new (x: number, y: number, w: number, h: number) => MirrorRect)(r.x, r.y, r.width, r.height)
    : r;
}

// -- getComputedStyle: values come from the real cascade (the properties components read, used width/height, every custom property).
//    A property nothing declared reads `""`, as an unsupported one would in a browser, with one advisory.
const computedCache = new WeakMap<object, unknown>();
function computedStyleFor(node: unknown) {
  const id = typeof (node as MirrorNode)?.__vnId === "number" ? (node as MirrorNode).__vnId : appRoot?.__vnId;
  const raw = id ? __vn_computed_style(id) : "";
  /** One property scanned out of the answer, not a parsed map (parsing cost three quarters of a read). The host call stays uncached (it flushes pending style). The `\n` before the key stops suffix matches (`width` vs `max-width`). */
  const lookup = (css: string): string | undefined => {
    const head = css + "=";
    const at = raw.startsWith(head) ? 0 : raw.indexOf("\n" + head);
    if (at < 0) return undefined;
    const from = at + head.length + (at === 0 ? 0 : 1);
    const end = raw.indexOf("\n", from);
    return end < 0 ? raw.slice(from) : raw.slice(from, end);
  };
  const read = (k: string | symbol): string | undefined => {
    const key = String(k);
    if (!/^[a-zA-Z-]/.test(key) || key.startsWith("__") || key === "then") return undefined;
    const css = key.startsWith("--") ? key : hyphenate(key);
    const value = lookup(css);
    if (value !== undefined) return value;
    // No answer: the node is not in the tree or there is no root yet. CSSOM gives a disconnected element an empty declaration, so "" is right.
    if (!raw) return "";
    if (css.startsWith("--")) return "";
    warnCompat(
      `getComputedStyle:${css}`,
      `getComputedStyle().${key} reads "" — vue-native's computed style publishes the properties ` +
        `a reused component actually reads (transition-*, animation-*, transform, display, position, ` +
        `overflow-*, width/height, direction, font-size, color) plus every custom property, ` +
        `not all 340 longhands`,
    );
    return "";
  };
  return new Proxy(
    {
      getPropertyValue: read,
      getPropertyPriority: () => "",
      get length() {
        return 0;
      },
    },
    {
      get: (t, k) => (k in t ? t[k as keyof typeof t] : typeof k === "string" ? read(k) : undefined),
      has: () => true,
    },
  );
}
defineGlobal("getComputedStyle", (node: unknown) => {
  let cached = computedCache.get(node as object);
  if (!cached) {
    cached = new Proxy(
      {},
      {
        get: (_t, k) => (computedStyleFor(node) as Record<string | symbol, unknown>)[k],
        has: () => true,
      },
    );
    if (node && typeof node === "object") computedCache.set(node, cached);
    styleDecls.add(cached as object);
  }
  return cached;
});
