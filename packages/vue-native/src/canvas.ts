// `<canvas>`: its bitmap and its 2D context (the host+media stream's module).
//
// The context is redjs's own `CanvasRenderingContext2D` (vendor/redjs
// webapi/canvas2d): state, paths and pixels are native, and the same class
// backs `OffscreenCanvas`. This module is the ELEMENT half HTML puts on
// `HTMLCanvasElement` — `width`/`height` reflected from the attributes (300 ×
// 150 by default; setting either, even to the same value, clears the bitmap
// and resets the context), `getContext`, `toDataURL`, `toBlob` — and the map
// from a mirror `<canvas>` to its context that `drawImage(canvasElement)` and
// `createImageBitmap(canvasElement)` go through.
//
// The bitmap is published under the element's host node id (`__vnId`), which
// is how the host finds it to paint (crates/vn-tree/src/canvas.rs): a draw
// repaints the element on the next frame.
//
// runtime.ts calls `adoptCanvas` on every mirror `<canvas>`
// (`nodeOps.createElement`, which the renderer and `document.createElement`
// share). It must stay importable on its own: test/runtime-harness.ts
// evaluates it once and binds `adoptCanvas` into runtime.ts as a parameter, so
// nothing here may touch a host function at module scope — every one is read
// through `natives()` at call time.

/** A 2D context, as redjs makes it. Opaque here. */
type Context2D = object;

/** The redjs natives behind the element half (redjs webapi/canvas2d/context.rs). */
interface CanvasNatives {
  __rjs_canvas_context(canvas: object, width: number, height: number, options: unknown, key: number): Context2D;
  __rjs_canvas_resize(ctx: Context2D, width: number, height: number): void;
  __rjs_canvas_data_url(ctx: Context2D | null, width: number, height: number, type: string, quality: unknown): string;
  __rjs_canvas_blob(ctx: Context2D | null, width: number, height: number, type: string, quality: unknown): unknown;
  __rjs_canvas_source_hook(resolve: (value: unknown) => unknown): void;
}

/** The slice of a mirror element this module reads and extends. */
interface CanvasElement {
  __vnId: number;
  tagName: string;
  attrs: Record<string, string>;
  setAttribute(name: string, value: unknown): void;
  width: number;
  height: number;
}

/** The engine's canvas natives, looked up per call (absent under bun). */
function natives(): CanvasNatives | null {
  const g = globalThis as unknown as Partial<CanvasNatives>;
  return typeof g.__rjs_canvas_context === "function" ? (g as CanvasNatives) : null;
}

const warned = new Set<string>();
function advise(key: string, msg: string): void {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[vue-native compat] ${msg}`);
}

/** Element → its 2D context. Weak: a dropped canvas takes its bitmap with it. */
const contexts = new WeakMap<object, Context2D>();

/** HTML "rules for parsing non-negative integers", with the attribute's default. */
function dimension(el: CanvasElement, name: "width" | "height"): number {
  const fallback = name === "width" ? 300 : 150;
  const raw = el.attrs[name];
  if (raw == null) return fallback;
  const m = /^[\t\n\f\r ]*\+?(\d+)/.exec(raw);
  if (!m) return fallback;
  const v = Number(m[1]);
  return v <= 2147483647 ? v : fallback;
}

function isCanvas(v: unknown): v is CanvasElement {
  return typeof v === "object" && v !== null && (v as { tagName?: unknown }).tagName === "CANVAS" && "__vnId" in v;
}

function tagOf(v: unknown): string {
  return typeof v === "object" && v !== null && typeof (v as { tagName?: unknown }).tagName === "string"
    ? (v as { tagName: string }).tagName
    : "";
}

/** The element's context, made on first need (drawImage of a canvas nobody drew on yet). */
function contextOf(el: CanvasElement, options?: unknown): Context2D | null {
  let ctx = contexts.get(el);
  if (ctx) return ctx;
  const n = natives();
  if (!n) return null;
  ctx = n.__rjs_canvas_context(el, dimension(el, "width"), dimension(el, "height"), options, el.__vnId);
  contexts.set(el, ctx);
  return ctx;
}

/**
 * What `drawImage` / `createImageBitmap` make of a mirror element: a canvas is
 * its context; a `<video>` or `<img>` mirror has no pixels on this side yet
 * (`null`: a valid source with nothing to draw); anything else is not an image
 * source (`undefined`: the engine's TypeError).
 */
function resolveSource(v: unknown): unknown {
  if (isCanvas(v)) return contextOf(v);
  const tag = tagOf(v);
  if (tag === "VIDEO" || tag === "IMG") {
    advise(
      `canvas:source-${tag.toLowerCase()}`,
      `drawImage(<${tag.toLowerCase()}>) draws nothing yet: the host's ${tag === "VIDEO" ? "video frames" : "decoded <img> pixels"} ` +
        `are not readable from a canvas in this build` +
        (tag === "IMG" ? "; draw a `new Image()` with the same src instead" : ""),
    );
    return null;
  }
  return undefined;
}

/** The engine the hook went to (once per engine, not once per module). */
let hookedOn: unknown = null;
function installHook(): void {
  const n = natives();
  if (!n || hookedOn === n.__rjs_canvas_source_hook) return;
  n.__rjs_canvas_source_hook(resolveSource);
  hookedOn = n.__rjs_canvas_source_hook;
}

/** WebIDL `unsigned long` for the width/height setters. */
function toUnsignedLong(v: unknown): number {
  const x = Number(v);
  return Number.isFinite(x) ? Math.trunc(x) >>> 0 : 0;
}

function getContext(this: CanvasElement, type: unknown, options?: unknown): Context2D | null {
  const kind = String(type);
  if (kind === "2d") {
    const ctx = contextOf(this, options);
    if (!ctx) advise("canvas:engine", "getContext('2d'): this engine has no canvas support; it returns null");
    return ctx;
  }
  advise(`canvas:${kind}`, `getContext('${kind}') is not available in this runtime (only '2d'); it returns null`);
  return null;
}

function toDataURL(this: CanvasElement, type?: unknown, quality?: unknown): string {
  const n = natives();
  if (!n) return "data:,";
  const ty = type === undefined ? "image/png" : String(type);
  return n.__rjs_canvas_data_url(contexts.get(this) ?? null, this.width, this.height, ty, quality);
}

function toBlob(this: CanvasElement, callback: unknown, type?: unknown, quality?: unknown): void {
  if (typeof callback !== "function") {
    throw new TypeError("HTMLCanvasElement.toBlob: the callback is not a function");
  }
  const n = natives();
  const ty = type === undefined ? "image/png" : String(type);
  // HTML encodes "in parallel" and calls back on a task: never synchronously.
  const blob = n ? n.__rjs_canvas_blob(contexts.get(this) ?? null, this.width, this.height, ty, quality) : null;
  setTimeout(() => (callback as (b: unknown) => void)(blob), 0);
}

/** A width/height attribute changed: a fresh bitmap, the context at its defaults. */
function resized(el: CanvasElement): void {
  const ctx = contexts.get(el);
  const n = natives();
  if (ctx && n) n.__rjs_canvas_resize(ctx, dimension(el, "width"), dimension(el, "height"));
}

/** Gives a mirror `<canvas>` its canvas members. */
export function adoptCanvas(target: object): void {
  const el = target as CanvasElement;
  installHook();
  // `patchProp` (and so `setAttribute`) is the one writer of `attrs`; a write
  // to width/height is a resize, even to the same value (HTML: "set bitmap
  // dimensions" runs on every set).
  el.attrs = new Proxy(el.attrs, {
    set(t, k, v) {
      t[k as string] = v as string;
      if (k === "width" || k === "height") resized(el);
      return true;
    },
    deleteProperty(t, k) {
      const had = k in t;
      delete t[k as string];
      if (had && (k === "width" || k === "height")) resized(el);
      return true;
    },
  });
  Object.defineProperties(el, {
    width: {
      get: () => dimension(el, "width"),
      set: (v: unknown) => el.setAttribute("width", String(toUnsignedLong(v))),
      configurable: true,
    },
    height: {
      get: () => dimension(el, "height"),
      set: (v: unknown) => el.setAttribute("height", String(toUnsignedLong(v))),
      configurable: true,
    },
    getContext: { value: getContext, writable: true, configurable: true },
    toDataURL: { value: toDataURL, writable: true, configurable: true },
    toBlob: { value: toBlob, writable: true, configurable: true },
  });
}
