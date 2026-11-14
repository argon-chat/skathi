import type { MirrorElement, MirrorNode, MirrorRect } from "./types";
import { layoutObserved } from "./layout-steps";
import { boundsOf, paintedRectOf, viewportRect } from "./dom-members";
import { warnCompat } from "../compat";
import { mk } from "./node-ops";
import { I } from "./event-names";
import { fragments } from "./tree-ops";
import { looseCreated } from "./nodes";
import { ElementProto } from "./protos";
import { defineGlobal } from "./globals";
import { appRoot } from "../runtime";

// IntersectionObserver, from the host's painted bounds. redjs's is inert, which is not harmless: @vueuse/motion's
// `visible` variants wait for `isIntersecting`, so elements that start at `opacity: 0` stay invisible forever and lazy
// loaders never load.
// Runs the W3C update steps in the frame step after layout (FINISH op): queues an entry when the threshold index or
// `isIntersecting` moves (so the first update always reports), calls back from a task after paint. Rectangles are
// where boxes were DRAWN, as getBoundingClientRect.
// Divergence: clipped by the ROOT only, not by intermediate scroll ancestors (a host call per ancestor per frame), so
// a target scrolled out of an inner scroller but inside the window reads as intersecting.
interface IntersectionTargetState {
  prevIndex: number;
  prevIntersecting: boolean;
}
interface IntersectionEntryInit {
  time: number;
  target: MirrorElement;
  rootBounds: MirrorRect | null;
  boundingClientRect: MirrorRect;
  intersectionRect: MirrorRect;
  isIntersecting: boolean;
  intersectionRatio: number;
}
export const intersectionObservers = new Set<SkIntersectionObserver>();
let intersectionTaskQueued = false;
export let intersectionsQueued = false;

export function updateIntersections(): void {
  const now = (globalThis as { performance?: { now(): number } }).performance?.now() ?? Date.now();
  let queued = false;
  for (const ob of [...intersectionObservers]) {
    if (!ob._targets.size) {
      intersectionObservers.delete(ob);
      continue;
    }
    if (ob._update(now)) queued = true;
  }
  intersectionsQueued = queued;
  if (!queued || intersectionTaskQueued) return;
  // "Queue an intersection observer task": the next pump's timers, after this
  // frame has painted.
  const later =
    (globalThis as { setTimeout?: (cb: () => void, ms: number) => unknown }).setTimeout ??
    (typeof setTimeout === "function" ? setTimeout : undefined);
  if (typeof later !== "function") return;
  intersectionTaskQueued = true;
  later(notifyIntersectionObservers, 0);
}

function notifyIntersectionObservers(): void {
  intersectionTaskQueued = false;
  for (const ob of [...intersectionObservers]) {
    const records = ob.takeRecords();
    if (!records.length) continue;
    try {
      ob._callback.call(ob, records, ob);
    } catch (e) {
      (globalThis.queueMicrotask ?? queueMicrotask)(() => {
        throw e;
      });
    }
  }
}

const ioRect = (x: number, y: number, width: number, height: number): MirrorRect => ({
  x,
  y,
  width,
  height,
  top: y,
  left: x,
  right: x + width,
  bottom: y + height,
  toJSON() {
    return this;
  },
});

function parseIntersectionMargin(text: unknown, what: string): { v: number; pct: boolean }[] {
  const parts = String(text ?? "0px")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  const out = parts.map((p) => {
    const m = /^(-?\d*\.?\d+)(px|%)?$/.exec(p);
    if (!m || (!m[2] && Number(m[1]) !== 0)) {
      throw new DOMException(
        `Failed to construct 'IntersectionObserver': ${what} must be specified in pixels or percent.`,
        "SyntaxError",
      );
    }
    return { v: Number(m[1]), pct: m[2] === "%" };
  });
  if (out.length < 1 || out.length > 4) {
    throw new DOMException(
      `Failed to construct 'IntersectionObserver': ${what} must have 1 to 4 values.`,
      "SyntaxError",
    );
  }
  const [t, r = t, b = t, l = r] = out as [{ v: number; pct: boolean }, ...{ v: number; pct: boolean }[]];
  return [t, r!, b!, l!];
}

class SkIntersectionObserver {
  declare _callback: (entries: unknown[], ob: SkIntersectionObserver) => void;
  declare _targets: Map<MirrorElement, IntersectionTargetState>;
  declare _records: unknown[];
  declare _margin: { v: number; pct: boolean }[];
  declare root: MirrorElement | null;
  declare rootMargin: string;
  declare scrollMargin: string;
  declare thresholds: readonly number[];
  declare delay: number;
  declare trackVisibility: boolean;
  constructor(
    callback: (entries: unknown[], ob: SkIntersectionObserver) => void,
    options: Record<string, unknown> = {},
  ) {
    if (typeof callback !== "function") {
      throw new TypeError("Failed to construct 'IntersectionObserver': parameter 1 is not of type 'Function'.");
    }
    const opts = options ?? {};
    const root = opts.root ?? null;
    const isDocument =
      !!root &&
      (root === (globalThis as { document?: unknown }).document || (root as { nodeType?: number }).nodeType === 9);
    if (root !== null && !isDocument && typeof (root as { nodeType?: number }).nodeType !== "number") {
      throw new TypeError("Failed to construct 'IntersectionObserver': root must be an Element, a Document or null.");
    }
    const raw = opts.threshold === undefined ? [0] : Array.isArray(opts.threshold) ? opts.threshold : [opts.threshold];
    const thresholds = (raw as unknown[]).map(Number);
    for (const t of thresholds) {
      if (!(t >= 0 && t <= 1)) {
        throw new RangeError(
          "Failed to construct 'IntersectionObserver': Threshold values must be numbers between 0 and 1",
        );
      }
    }
    thresholds.sort((a, b) => a - b);
    this._callback = callback;
    this._targets = new Map();
    this._records = [];
    this._margin = parseIntersectionMargin(opts.rootMargin ?? "0px", "rootMargin");
    this.root = isDocument ? null : (root as MirrorElement | null);
    this.rootMargin = this._margin.map((m) => `${m.v}${m.pct ? "%" : "px"}`).join(" ");
    this.scrollMargin = parseIntersectionMargin(opts.scrollMargin ?? "0px", "scrollMargin")
      .map((m) => `${m.v}${m.pct ? "%" : "px"}`)
      .join(" ");
    this.thresholds = Object.freeze(thresholds.length ? thresholds : [0]);
    this.delay = Number(opts.delay ?? 0) || 0;
    this.trackVisibility = !!opts.trackVisibility;
  }
  observe(target: MirrorElement) {
    if (!target || typeof target !== "object") {
      throw new TypeError(
        "Failed to execute 'observe' on 'IntersectionObserver': parameter 1 is not of type 'Element'.",
      );
    }
    if (this._targets.has(target)) return;
    this._targets.set(target, { prevIndex: -1, prevIntersecting: false });
    intersectionObservers.add(this);
    layoutObserved();
  }
  unobserve(target: MirrorElement) {
    this._targets.delete(target);
  }
  disconnect() {
    this._targets.clear();
    this._records.length = 0;
    intersectionObservers.delete(this);
  }
  takeRecords(): unknown[] {
    return this._records.splice(0, this._records.length);
  }
  _rootBounds(): MirrorRect | null {
    let r: MirrorRect;
    if (this.root) {
      if (!boundsOf(this.root)) return null;
      r = paintedRectOf(this.root);
    } else {
      if (!appRoot) return null;
      r = viewportRect();
    }
    const [t, rt, b, l] = this._margin as [
      { v: number; pct: boolean },
      { v: number; pct: boolean },
      { v: number; pct: boolean },
      { v: number; pct: boolean },
    ];
    const px = (m: { v: number; pct: boolean }, of: number) => (m.pct ? (m.v / 100) * of : m.v);
    const top = r.top - px(t, r.height);
    const left = r.left - px(l, r.width);
    const right = r.right + px(rt, r.width);
    const bottom = r.bottom + px(b, r.height);
    return ioRect(left, top, Math.max(0, right - left), Math.max(0, bottom - top));
  }
  _update(time: number): boolean {
    const rootBounds = this._rootBounds();
    let queued = false;
    for (const [target, state] of this._targets) {
      const painted = !!boundsOf(target);
      let isIntersecting = false;
      let thresholdIndex = 0;
      let ratio = 0;
      let targetRect = ioRect(0, 0, 0, 0);
      let intersectionRect = ioRect(0, 0, 0, 0);
      const descendant = (() => {
        if (!this.root) return true;
        for (let n: MirrorNode | null | undefined = target.parentNode; n; n = n.parentNode)
          if (n === this.root) return true;
        return false;
      })();
      if (painted && rootBounds && descendant) {
        targetRect = paintedRectOf(target);
        const l = Math.max(targetRect.left, rootBounds.left);
        const t = Math.max(targetRect.top, rootBounds.top);
        const r = Math.min(targetRect.right, rootBounds.right);
        const b = Math.min(targetRect.bottom, rootBounds.bottom);
        isIntersecting = r >= l && b >= t;
        if (isIntersecting) intersectionRect = ioRect(l, t, Math.max(0, r - l), Math.max(0, b - t));
        const targetArea = targetRect.width * targetRect.height;
        const area = intersectionRect.width * intersectionRect.height;
        ratio = targetArea > 0 ? area / targetArea : isIntersecting ? 1 : 0;
        const at = this.thresholds.findIndex((th) => th > ratio);
        thresholdIndex = at < 0 ? this.thresholds.length : at;
      }
      if (thresholdIndex === state.prevIndex && isIntersecting === state.prevIntersecting) continue;
      state.prevIndex = thresholdIndex;
      state.prevIntersecting = isIntersecting;
      this._records.push(
        new SkIntersectionObserverEntry({
          time,
          target,
          rootBounds,
          boundingClientRect: targetRect,
          intersectionRect,
          isIntersecting,
          intersectionRatio: ratio,
        }),
      );
      queued = true;
    }
    return queued;
  }
}

class SkIntersectionObserverEntry {
  declare time: number;
  declare target: MirrorElement;
  declare rootBounds: MirrorRect | null;
  declare boundingClientRect: MirrorRect;
  declare intersectionRect: MirrorRect;
  declare isIntersecting: boolean;
  declare intersectionRatio: number;
  declare isVisible: boolean;
  constructor(init: IntersectionEntryInit) {
    this.time = init.time;
    this.target = init.target;
    this.rootBounds = init.rootBounds;
    this.boundingClientRect = init.boundingClientRect;
    this.intersectionRect = init.intersectionRect;
    this.isIntersecting = init.isIntersecting;
    this.intersectionRatio = init.intersectionRatio;
    this.isVisible = false;
  }
}

// OVER redjs's inert pair, not beside it — `defineGlobal` only fills a hole.
for (const [name, value] of [
  ["IntersectionObserver", SkIntersectionObserver],
  ["IntersectionObserverEntry", SkIntersectionObserverEntry],
] as const) {
  try {
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  } catch {
    try {
      (globalThis as Record<string, unknown>)[name] = value;
    } catch {
      warnCompat(`global:${name}`, `${name} could not be installed over the engine's own; it stays inert`);
    }
  }
}

// DocumentFragment: Vue's Teleport accepts one as a target and reka-ui's Select constructs one even while closed.
// Ours is a detached mirror element (loose, `looseCreated`); inserting it inserts its children instead (`insertAll`).
{
  class DocumentFragment {
    constructor() {
      const node = mk(__sk_create_element(I("fragment")), 1, "fragment");
      Object.setPrototypeOf(node, DocumentFragment.prototype);
      fragments.add(node);
      // `new DocumentFragment()` deliberately returns a different object: a detached mirror element.
      return looseCreated(node) as unknown as DocumentFragment;
    }
  }
  Object.setPrototypeOf(DocumentFragment.prototype, ElementProto);
  for (const [k, v] of [
    ["nodeType", 11],
    ["nodeName", "#document-fragment"],
  ] as const) {
    Object.defineProperty(DocumentFragment.prototype, k, { get: () => v, configurable: true });
  }
  defineGlobal("DocumentFragment", DocumentFragment);
}
