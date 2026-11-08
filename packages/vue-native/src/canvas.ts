// `<canvas>`: its bitmap and native 2D or WebGPU context.
//
// Both contexts live in redjs (`webapi/canvas2d` and `webapi/webgpu`),
// alongside the OffscreenCanvas implementation. This module is the element half HTML puts on
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
// A mirror `<img>` is drawn from the pixels the host decoded for it
// (`__vn_img_bitmap`, crates/vn-tree/src/img.rs `pixels`), as an ImageBitmap
// kept per element and `src`: a cropper that redraws on every pointer move
// decodes once.
//
// runtime.ts calls `adoptCanvas` on every mirror `<canvas>`
// (`nodeOps.createElement`, which the renderer and `document.createElement`
// share). It must stay importable on its own: test/runtime-harness.ts
// evaluates it once and binds `adoptCanvas` into runtime.ts as a parameter, so
// nothing here may touch a host function at module scope — every one is read
// through `natives()` at call time.

/** Native contexts are opaque; rendering and capture stay in redjs. */
type Context2D = object;
type ContextGPU = object;
type ContextSelection = { kind: "2d" | "webgpu"; value: object | null };

/** The redjs natives behind the element half (redjs webapi/canvas2d/context.rs). */
interface CanvasNatives {
  __rjs_canvas_context(canvas: object, width: number, height: number, options: unknown, key: number): Context2D;
  __rjs_canvas_resize(ctx: Context2D, width: number, height: number): void;
  __rjs_canvas_data_url(ctx: Context2D | null, width: number, height: number, type: string, quality: unknown): string;
  __rjs_canvas_blob(ctx: Context2D | null, width: number, height: number, type: string, quality: unknown): unknown;
  __rjs_canvas_source_hook(resolve: (value: unknown) => unknown): void;
}

/** The WebGPU element entry points are present only in a WebGPU-enabled engine. */
interface WebGPUNatives {
  __rjs_webgpu_context(canvas: object, width: number, height: number, key: number): ContextGPU | null;
  __rjs_webgpu_resize(ctx: ContextGPU, width: number, height: number): void;
  __rjs_webgpu_data_url(ctx: ContextGPU, type: string, quality: unknown): string;
  __rjs_webgpu_blob(ctx: ContextGPU, type: string, quality: unknown): unknown;
}

/** The host's half (crates/vn-tree/src/lib.rs `vn_img_bitmap`). */
interface HostNatives {
  /** The `<img>`'s pixels as an ImageBitmap, `null` for nothing yet, or why not. */
  __vn_img_bitmap(id: number, originClean: boolean): ImageBitmapLike | null | string;
}

/** An ImageBitmap, as far as this module touches one. */
interface ImageBitmapLike {
  close(): void;
}

/** The slice of a mirror `<img>` read here: its DOM state, which runtime.ts keeps. */
interface ImageElement {
  __vnId: number;
  attrs: Record<string, string>;
  complete: boolean;
  currentSrc: string;
  naturalWidth: number;
  naturalHeight: number;
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

/** Look up canvas support directly; adapter availability does not imply presentation. */
function gpuNatives(): WebGPUNatives | null {
  const g = globalThis as unknown as Partial<WebGPUNatives>;
  return typeof g.__rjs_webgpu_context === "function" ? (g as WebGPUNatives) : null;
}

/** The host's natives, looked up per call (absent under bun). */
function host(): HostNatives | null {
  const g = globalThis as unknown as Partial<HostNatives>;
  return typeof g.__vn_img_bitmap === "function" ? (g as HostNatives) : null;
}

const warned = new Set<string>();
function advise(key: string, msg: string): void {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[vue-native compat] ${msg}`);
}

/** Context mode is fixed after a successful selection. Failed requests leave it unset. */
const contexts = new WeakMap<object, ContextSelection>();

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

/** Select one native context without converting a canvas already using another mode. */
function contextOf(el: CanvasElement, kind: "2d" | "webgpu", options?: unknown): object | null {
  const selected = contexts.get(el);
  if (selected) return selected.kind === kind ? selected.value : null;
  const n = kind === "2d" ? natives() : null;
  const gpu = kind === "webgpu" ? gpuNatives() : null;
  if (!n && !gpu) return null;
  // Native option getters may reenter getContext before creation completes.
  const pending: ContextSelection = { kind, value: null };
  contexts.set(el, pending);
  let w: number, h: number;
  try {
    w = dimension(el, "width");
    h = dimension(el, "height");
    pending.value = n
      ? n.__rjs_canvas_context(el, w, h, options, el.__vnId)
      : gpu!.__rjs_webgpu_context(el, w, h, el.__vnId);
  } catch (error) {
    contexts.delete(el);
    throw error;
  }
  if (!pending.value) {
    contexts.delete(el);
    return null;
  }
  // A getter may also have changed dimensions while no context was available.
  if (w !== dimension(el, "width") || h !== dimension(el, "height")) resized(el);
  return pending.value;
}

/** An `<img>`'s decoded pixels, for the `src` (and natural size) they were made for. */
const decoded = new WeakMap<object, { src: string; w: number; h: number; bitmap: ImageBitmapLike }>();

/**
 * HTML's origin-clean flag for an image: false for one fetched over http(s)
 * without a `crossorigin` attribute, which a browser's page origin never
 * shares with this runtime's `app://`. Recorded on the canvas that draws it,
 * not enforced (redjs canvas2d `Surface::readable` advises instead).
 */
function originClean(el: ImageElement, src: string): boolean {
  if (!/^https?:/i.test(src.trim())) return true;
  return el.attrs.crossorigin != null || el.attrs.crossOrigin != null;
}

/**
 * HTML "check the usability of the image argument" for an `<img>`, and its
 * pixels: still loading (or no `src`) draws nothing; a broken one throws
 * InvalidStateError, worded as Chromium words it; a loaded one is the bitmap
 * the host decoded for it.
 */
function imageSource(el: ImageElement): unknown {
  const src = el.attrs.src ?? "";
  if (src.trim() === "" || !el.complete) return null;
  // Complete with a src but no current one: the request answered `error`.
  if (el.currentSrc === "") {
    throw new DOMException("The HTMLImageElement provided is in the 'broken' state.", "InvalidStateError");
  }
  const w = el.naturalWidth;
  const h = el.naturalHeight;
  if (w === 0 || h === 0) return null;
  const hit = decoded.get(el);
  if (hit && hit.src === src && hit.w === w && hit.h === h) return hit.bitmap;
  const n = host();
  if (!n) return null;
  const got = n.__vn_img_bitmap(el.__vnId, originClean(el, src));
  if (typeof got === "string") {
    advise("canvas:img-decode", `drawImage(<img>) draws nothing: ${got}`);
    return null;
  }
  if (got === null) return null;
  hit?.bitmap.close();
  decoded.set(el, { src, w, h, bitmap: got });
  return got;
}

/**
 * What `drawImage` / `createImageBitmap` make of a mirror element: a canvas is
 * its context; an `<img>` is its decoded pixels (`imageSource`); a `<video>`
 * has no frames on this side yet (`null`: a valid source with nothing to
 * draw); anything else is not an image source (`undefined`: the engine's
 * TypeError).
 */
function resolveSource(v: unknown): unknown {
  // Reading a blank canvas does not select a rendering mode for it.
  if (isCanvas(v)) return contexts.get(v)?.value ?? null;
  const tag = tagOf(v);
  if (tag === "IMG" && typeof v === "object" && v !== null && "__vnId" in v) return imageSource(v as ImageElement);
  if (tag === "VIDEO") {
    // CV-4 (docs/RUNTIME.md §5.10).
    advise(
      "canvas:source-video",
      "drawImage(<video>) draws nothing yet: the host's video frames are not readable from a canvas in this build",
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

function getContext(this: CanvasElement, type: unknown, options?: unknown): object | null {
  const kind = String(type);
  if (kind === "2d" || kind === "webgpu") return contextOf(this, kind, options);
  advise(`canvas:${kind}`, `getContext('${kind}') is not available in this runtime; it returns null`);
  return null;
}

function toDataURL(this: CanvasElement, type?: unknown, quality?: unknown): string {
  const ty = type === undefined ? "image/png" : String(type);
  const selected = contexts.get(this);
  if (selected?.kind === "webgpu" && selected.value) return gpuNatives()?.__rjs_webgpu_data_url(selected.value, ty, quality) ?? "data:,";
  return natives()?.__rjs_canvas_data_url(selected?.value ?? null, this.width, this.height, ty, quality) ?? "data:,";
}

function toBlob(this: CanvasElement, callback: unknown, type?: unknown, quality?: unknown): void {
  if (typeof callback !== "function") {
    throw new TypeError("HTMLCanvasElement.toBlob: the callback is not a function");
  }
  const ty = type === undefined ? "image/png" : String(type);
  const selected = contexts.get(this);
  const blob = selected?.kind === "webgpu" && selected.value
    ? gpuNatives()?.__rjs_webgpu_blob(selected.value, ty, quality) ?? null
    : natives()?.__rjs_canvas_blob(selected?.value ?? null, this.width, this.height, ty, quality) ?? null;
  // HTML calls back on a later task, including a zero-size canvas's null result.
  setTimeout(() => (callback as (b: unknown) => void)(blob), 0);
}

/** A width/height attribute changed: a fresh bitmap, the context at its defaults. */
function resized(el: CanvasElement): void {
  const selected = contexts.get(el);
  if (!selected?.value) return;
  const w = dimension(el, "width");
  const h = dimension(el, "height");
  if (selected.kind === "webgpu") gpuNatives()?.__rjs_webgpu_resize(selected.value, w, h);
  else natives()?.__rjs_canvas_resize(selected.value, w, h);
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
