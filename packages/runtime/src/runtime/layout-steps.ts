import {
  broadcastNextResize,
  deliverResizeLoopError,
  gatherResizes,
  resizeDepth,
  resizeObservers,
} from "./resize-observer";
import type { ResizeObserverLike } from "./resize-observer";
import { defineGlobal } from "./globals";
import type { MirrorElement } from "./types";
import { intersectionObservers, intersectionsQueued, updateIntersections } from "./intersection-observer";

const LAYOUT_START = 0;
const LAYOUT_NEXT = 1;
const LAYOUT_AGAIN = 2;
const LAYOUT_FINISH = 3;

function observingLayout(): boolean {
  for (const ob of resizeObservers) if (ob._targets.size) return true;
  for (const ob of intersectionObservers) if (ob._targets.size) return true;
  return false;
}

/** The runtime's half of the step: one op per host call. A throw would lose the step's state, so callback errors are reported from a microtask. */
function layoutStep(op: number): number {
  switch (op) {
    case LAYOUT_START:
      return gatherResizes(0);
    case LAYOUT_NEXT:
      return broadcastNextResize();
    case LAYOUT_AGAIN:
      return gatherResizes(resizeDepth);
    case LAYOUT_FINISH: {
      let raised = 0;
      if (resizeObservers.some((ob) => ob._skipped)) {
        for (const ob of resizeObservers) ob._skipped = false;
        deliverResizeLoopError();
        raised = 1;
      }
      updateIntersections();
      for (let i = resizeObservers.length - 1; i >= 0; i--)
        if (!resizeObservers[i]!._targets.size) resizeObservers.splice(i, 1);
      return raised | (observingLayout() ? 2 : 0);
    }
    default:
      return 0;
  }
}

// Without the host functions (older shell, test harness) the step falls back to
// requestAnimationFrame: before layout, no re-layout between rounds, and a
// 250 ms re-poll while nothing changes.
export const hostSteps = typeof __sk_set_layout_hook === "function" && typeof __sk_layout_watch === "function";
if (hostSteps) __sk_set_layout_hook(layoutStep);
const QUIET_STEPS = 2;
let fallbackQueued = false;
let fallbackQuiet = 0;
let fallbackTimer: unknown = null;
const layoutTimers = globalThis as {
  setTimeout?: (cb: () => void, ms: number) => unknown;
  clearTimeout?: (id: unknown) => void;
};

export function layoutObserved(): void {
  if (hostSteps) {
    __sk_layout_watch(true);
    return;
  }
  fallbackQuiet = 0;
  queueFallbackStep();
}

function queueFallbackStep(): void {
  if (fallbackTimer != null) layoutTimers.clearTimeout?.(fallbackTimer);
  fallbackTimer = null;
  if (fallbackQueued) return;
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf !== "function") return;
  fallbackQueued = true;
  raf(fallbackStep);
}

function fallbackStep(): void {
  fallbackQueued = false;
  let reported = false;
  let due = layoutStep(LAYOUT_START);
  while (due > 0) {
    reported = true;
    while (layoutStep(LAYOUT_NEXT) > 0);
    due = layoutStep(LAYOUT_AGAIN);
  }
  const state = layoutStep(LAYOUT_FINISH);
  if (intersectionsQueued) reported = true;
  if (!(state & 2)) return;
  fallbackQuiet = reported ? 0 : fallbackQuiet + 1;
  if (fallbackQuiet < QUIET_STEPS) queueFallbackStep();
  else if (fallbackTimer == null && typeof layoutTimers.setTimeout === "function") {
    fallbackTimer = layoutTimers.setTimeout(() => {
      fallbackTimer = null;
      queueFallbackStep();
    }, 250);
  }
}

defineGlobal(
  "ResizeObserver",
  class ResizeObserver implements ResizeObserverLike {
    declare _callback: ResizeObserverLike["_callback"];
    declare _targets: ResizeObserverLike["_targets"];
    declare _active: ResizeObserverLike["_active"];
    declare _skipped: boolean;
    constructor(callback: ResizeObserverLike["_callback"]) {
      if (typeof callback !== "function") {
        throw new TypeError("Failed to construct 'ResizeObserver': parameter 1 is not of type 'Function'.");
      }
      this._callback = callback;
      this._targets = new Map();
      this._active = [];
      this._skipped = false;
    }
    observe(node: MirrorElement) {
      if (!node || typeof node.__skId !== "number") return;
      this._targets.set(node, null);
      if (!resizeObservers.includes(this)) resizeObservers.push(this);
      layoutObserved();
    }
    unobserve(node: MirrorElement) {
      this._targets.delete(node);
    }
    disconnect() {
      this._targets.clear();
      this._active = [];
      this._skipped = false;
    }
  },
);
