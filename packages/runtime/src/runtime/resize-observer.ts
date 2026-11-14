import type { MirrorElement, MirrorNode, MirrorRect } from "./types";
import { rectOf } from "./dom-members";
import { branded } from "./collections";
import { nativeEventClass } from "./forms";
import { asDomRect } from "./computed-style";

// ResizeObserver / IntersectionObserver run after layout and before paint (HTML "update the rendering"): a callback
// sees the size the frame is about to paint, and what it changes paints in that frame. The host runs one step per
// frame through `__sk_set_layout_hook`: it calls `layoutStep` once per op, drains microtasks after each call, and lays
// out again between rounds when callbacks changed anything.
// It has to be real: @floating-ui's `autoUpdate` waits for a ResizeObserver report before placing an element, so an
// inert observer leaves every popover at its pre-layout 0×0 position.
// Divergence: `contentRect`, `contentBoxSize` and `borderBoxSize` are all the BORDER box (padding and border widths
// are not published per node); `contentRect` is at (0, 0).
interface ResizeEntryLike {
  target: MirrorElement;
  contentRect: MirrorRect;
  borderBoxSize: readonly { inlineSize: number; blockSize: number }[];
  contentBoxSize: readonly { inlineSize: number; blockSize: number }[];
  devicePixelContentBoxSize: readonly { inlineSize: number; blockSize: number }[];
}
interface ResizeSize {
  w: number;
  h: number;
}
export interface ResizeObserverLike {
  _callback: (this: ResizeObserverLike, entries: ResizeEntryLike[], ob: ResizeObserverLike) => void;
  _targets: Map<MirrorElement, ResizeSize | null>;
  _active: [MirrorElement, ResizeSize, number][];
  _skipped: boolean;
}

export const resizeObservers: ResizeObserverLike[] = [];
export const resizeDue: ResizeObserverLike[] = [];
export let resizeDepth = Infinity;

/** Nodes from `node` to its tree's root, itself included: a root or detached node is 1, so the first gather (depth 0) takes every target. */
function nodeDepth(node: MirrorNode): number {
  let d = 1;
  for (let n = node.parentNode; n; n = n.parentNode) d++;
  return d;
}

export function gatherResizes(depth: number): number {
  resizeDue.length = 0;
  resizeDepth = Infinity;
  for (const ob of resizeObservers) {
    ob._active = [];
    ob._skipped = false;
    for (const [target, last] of ob._targets) {
      const r = rectOf(target);
      if (last && last.w === r.width && last.h === r.height) continue;
      const d = nodeDepth(target);
      if (d > depth) ob._active.push([target, { w: r.width, h: r.height }, d]);
      else ob._skipped = true;
    }
    if (ob._active.length) resizeDue.push(ob);
  }
  return resizeDue.length;
}

export function broadcastNextResize(): number {
  const ob = resizeDue.shift();
  if (ob && ob._active.length) {
    const entries: ResizeEntryLike[] = [];
    for (const [target, size, d] of ob._active) {
      entries.push(resizeEntry(target, size));
      if (ob._targets.has(target)) ob._targets.set(target, size);
      if (d < resizeDepth) resizeDepth = d;
    }
    ob._active = [];
    try {
      ob._callback.call(ob, entries, ob);
    } catch (e) {
      (globalThis.queueMicrotask ?? queueMicrotask)(() => {
        throw e;
      });
    }
  }
  return resizeDue.length;
}

function resizeEntry(target: MirrorElement, size: ResizeSize): ResizeEntryLike {
  const box = Object.freeze([Object.freeze(branded("ResizeObserverSize", { inlineSize: size.w, blockSize: size.h }))]);
  const dpr = Number((globalThis as { devicePixelRatio?: number }).devicePixelRatio) || 1;
  const device = Object.freeze([
    Object.freeze(
      branded("ResizeObserverSize", { inlineSize: Math.round(size.w * dpr), blockSize: Math.round(size.h * dpr) }),
    ),
  ]);
  const contentRect = asDomRect({
    x: 0,
    y: 0,
    width: size.w,
    height: size.h,
    top: 0,
    left: 0,
    right: size.w,
    bottom: size.h,
    toJSON() {
      return this;
    },
  });
  return branded("ResizeObserverEntry", {
    target,
    contentRect,
    borderBoxSize: box,
    contentBoxSize: box,
    devicePixelContentBoxSize: device,
  });
}

const RESIZE_LOOP_MESSAGE = "ResizeObserver loop completed with undelivered notifications.";

/**
 * Reports the resize loop error at the global: a cancelable `ErrorEvent`, and `window.onerror` called the HTML way
 * `(message, source, lineno, colno, error)`. The engine would hand `onerror` the event alone (breaking Sentry's
 * filter), so the attribute is held back from dispatch and called here.
 */
export function deliverResizeLoopError(): void {
  const g = globalThis as unknown as {
    onerror?: unknown;
    dispatchEvent?: (e: SkEvent) => boolean;
  };
  if (!ErrorEventClass) return;
  const ev = new ErrorEventClass("error", {
    cancelable: true,
    message: RESIZE_LOOP_MESSAGE,
    filename: "",
    lineno: 0,
    colno: 0,
    error: null,
  });
  const handler = g.onerror;
  const own = Object.getOwnPropertyDescriptor(g, "onerror");
  let hidden = false;
  if (typeof handler === "function" && own?.writable) {
    try {
      g.onerror = null;
      hidden = true;
    } catch {
      /* not writable after all: the engine calls it as an attribute */
    }
  }
  try {
    g.dispatchEvent?.(ev);
  } finally {
    if (hidden) g.onerror = handler;
  }
  if (hidden) {
    try {
      (handler as (...a: unknown[]) => unknown).call(globalThis, RESIZE_LOOP_MESSAGE, "", 0, 0, null);
    } catch (e) {
      (globalThis.queueMicrotask ?? queueMicrotask)(() => {
        throw e;
      });
    }
  }
}

type ErrorEventInit = {
  bubbles?: boolean;
  cancelable?: boolean;
  composed?: boolean;
  message?: string;
  filename?: string;
  lineno?: number;
  colno?: number;
  error?: unknown;
};
const ErrorEventClass = nativeEventClass<ErrorEventInit>("ErrorEvent");
