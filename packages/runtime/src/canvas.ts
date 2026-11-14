// `<canvas>`: the element half (width/height reflection, `getContext`, `toDataURL`, `toBlob`) over the native
// 2D/WebGPU contexts that live in redjs. Setting width or height, even to the same value, clears the bitmap and
// resets the context. The bitmap is published under the element's host node id (`__skId`) so the host can paint it.
// runtime.ts calls `adoptCanvas` on every mirror `<canvas>`; test/runtime-harness.ts evaluates this module
// standalone, so host functions are read through `natives()` at call time, never at module scope.

type Context2D = object;
type ContextGPU = object;
type ContextSelection = { kind: "2d" | "webgpu"; value: object | null };

interface CanvasNatives {
  __rjs_canvas_context(canvas: object, width: number, height: number, options: unknown, key: number): Context2D;
  __rjs_canvas_resize(ctx: Context2D, width: number, height: number): void;
  __rjs_canvas_data_url(ctx: Context2D | null, width: number, height: number, type: string, quality: unknown): string;
  __rjs_canvas_blob(ctx: Context2D | null, width: number, height: number, type: string, quality: unknown): unknown;
  __rjs_canvas_source_hook(resolve: (value: unknown) => unknown): void;
}

interface WebGPUNatives {
  __rjs_webgpu_context(canvas: object, width: number, height: number, key: number): ContextGPU | null;
  __rjs_webgpu_resize(ctx: ContextGPU, width: number, height: number): void;
  __rjs_webgpu_data_url(ctx: ContextGPU, type: string, quality: unknown): string;
  __rjs_webgpu_blob(ctx: ContextGPU, type: string, quality: unknown): unknown;
}

interface HostNatives {
  __sk_img_bitmap(id: number, originClean: boolean): ImageBitmapLike | null | string;
}

interface ImageBitmapLike {
  close(): void;
}

interface ImageElement {
  __skId: number;
  attrs: Record<string, string>;
  complete: boolean;
  currentSrc: string;
  naturalWidth: number;
  naturalHeight: number;
}

interface CanvasElement {
  __skId: number;
  tagName: string;
  attrs: Record<string, string>;
  setAttribute(name: string, value: unknown): void;
  width: number;
  height: number;
}

function natives(): CanvasNatives | null {
  const g = globalThis as unknown as Partial<CanvasNatives>;
  return typeof g.__rjs_canvas_context === "function" ? (g as CanvasNatives) : null;
}

function gpuNatives(): WebGPUNatives | null {
  const g = globalThis as unknown as Partial<WebGPUNatives>;
  return typeof g.__rjs_webgpu_context === "function" ? (g as WebGPUNatives) : null;
}

function host(): HostNatives | null {
  const g = globalThis as unknown as Partial<HostNatives>;
  return typeof g.__sk_img_bitmap === "function" ? (g as HostNatives) : null;
}

const warned = new Set<string>();
function advise(key: string, msg: string): void {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[skathi compat] ${msg}`);
}

const contexts = new WeakMap<object, ContextSelection>();

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
  return typeof v === "object" && v !== null && (v as { tagName?: unknown }).tagName === "CANVAS" && "__skId" in v;
}

function tagOf(v: unknown): string {
  return typeof v === "object" && v !== null && typeof (v as { tagName?: unknown }).tagName === "string"
    ? (v as { tagName: string }).tagName
    : "";
}

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
      ? n.__rjs_canvas_context(el, w, h, options, el.__skId)
      : gpu!.__rjs_webgpu_context(el, w, h, el.__skId);
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

const decoded = new WeakMap<object, { src: string; w: number; h: number; bitmap: ImageBitmapLike }>();

/** HTML origin-clean flag: false for an http(s) image without `crossorigin`. Recorded on the canvas, not enforced. */
function originClean(el: ImageElement, src: string): boolean {
  if (!/^https?:/i.test(src.trim())) return true;
  return el.attrs.crossorigin != null || el.attrs.crossOrigin != null;
}

function imageSource(el: ImageElement): unknown {
  const src = el.attrs.src ?? "";
  if (src.trim() === "" || !el.complete) return null;
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
  const got = n.__sk_img_bitmap(el.__skId, originClean(el, src));
  if (typeof got === "string") {
    advise("canvas:img-decode", `drawImage(<img>) draws nothing: ${got}`);
    return null;
  }
  if (got === null) return null;
  hit?.bitmap.close();
  decoded.set(el, { src, w, h, bitmap: got });
  return got;
}

function resolveSource(v: unknown): unknown {
  if (isCanvas(v)) return contexts.get(v)?.value ?? null;
  const tag = tagOf(v);
  if (tag === "IMG" && typeof v === "object" && v !== null && "__skId" in v) return imageSource(v as ImageElement);
  if (tag === "VIDEO") {
    advise(
      "canvas:source-video",
      "drawImage(<video>) draws nothing yet: the host's video frames are not readable from a canvas in this build",
    );
    return null;
  }
  return undefined;
}

let hookedOn: unknown = null;
function installHook(): void {
  const n = natives();
  if (!n || hookedOn === n.__rjs_canvas_source_hook) return;
  n.__rjs_canvas_source_hook(resolveSource);
  hookedOn = n.__rjs_canvas_source_hook;
}

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
  if (selected?.kind === "webgpu" && selected.value)
    return gpuNatives()?.__rjs_webgpu_data_url(selected.value, ty, quality) ?? "data:,";
  return natives()?.__rjs_canvas_data_url(selected?.value ?? null, this.width, this.height, ty, quality) ?? "data:,";
}

function toBlob(this: CanvasElement, callback: unknown, type?: unknown, quality?: unknown): void {
  if (typeof callback !== "function") {
    throw new TypeError("HTMLCanvasElement.toBlob: the callback is not a function");
  }
  const ty = type === undefined ? "image/png" : String(type);
  const selected = contexts.get(this);
  const blob =
    selected?.kind === "webgpu" && selected.value
      ? (gpuNatives()?.__rjs_webgpu_blob(selected.value, ty, quality) ?? null)
      : (natives()?.__rjs_canvas_blob(selected?.value ?? null, this.width, this.height, ty, quality) ?? null);
  // HTML calls back on a later task, including a zero-size canvas's null result.
  setTimeout(() => (callback as (b: unknown) => void)(blob), 0);
}

function resized(el: CanvasElement): void {
  const selected = contexts.get(el);
  if (!selected?.value) return;
  const w = dimension(el, "width");
  const h = dimension(el, "height");
  if (selected.kind === "webgpu") gpuNatives()?.__rjs_webgpu_resize(selected.value, w, h);
  else natives()?.__rjs_canvas_resize(selected.value, w, h);
}

export function adoptCanvas(target: object): void {
  const el = target as CanvasElement;
  installHook();
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
