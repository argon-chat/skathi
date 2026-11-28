import type { MirrorElement, MirrorNode } from "./types";
import { defineDom } from "./dom-members";
import { appRoot } from "./root";
import { hasListener, propagate, synthesize } from "./events";

export const MOUSE_ID = 1;
export const PEN_ID = 2;

/** One pen, touch (or, positions only, mouse) reading the host reports: window CSS px, degrees. */
export interface PointerReading {
  x: number;
  y: number;
  pressure: number;
  tangentialPressure: number;
  tiltX: number;
  tiltY: number;
  twist: number;
  width: number;
  height: number;
  /** When the hardware took it, on the `performance.now()` clock. */
  time: number;
}

/** Where the pointer is predicted to be at `time`. */
export interface PredictedReading {
  x: number;
  y: number;
  time: number;
}

/** What drove a host mouse event: the mouse, a pen or a touch contact, with its readings (oldest first, the event's own last). */
export interface PointerSource {
  id: number;
  type: "mouse" | "pen" | "touch";
  /** The first contact of a touch sequence; the mouse and a pen always are. */
  primary: boolean;
  /** A touch's boundary event fired in two parts: only its pointer events, or only its mouse event. */
  part?: "pointer" | "compat";
  eraser: boolean;
  barrel: boolean;
  mousePressure?: boolean;
  readings: PointerReading[];
  predicted: PredictedReading[];
}

export const MOUSE_SOURCE: PointerSource = {
  id: MOUSE_ID,
  type: "mouse",
  primary: true,
  eraser: false,
  barrel: false,
  readings: [],
  predicted: [],
};

// Lockstep with sk-tree's pen.rs.
const PEN_ERASER = 1;
const PEN_BARREL = 2;
const PEN_MOUSE = 8;
const MOUSE_PRESSURE = 16;
const TOUCH = 32;
const TOUCH_PRIMARY = 64;
const POINTER_ONLY = 128;
const COMPAT_ONLY = 256;
const ID_SHIFT = 512;
const READING_LEN = 10;
const PREDICTED_LEN = 3;

/** The host's pointer packet (`[flags, count, ...readings, ...predicted]`); the mouse when there is none. */
export function pointerSource(raw: unknown): PointerSource {
  if (!Array.isArray(raw) || raw.length < 2 + READING_LEN) return MOUSE_SOURCE;
  const n = raw as number[];
  const flags = n[0] ?? 0;
  const count = n[1] ?? 0;
  const readings: PointerReading[] = [];
  let i = 2;
  for (; readings.length < count && i + READING_LEN <= n.length; i += READING_LEN) {
    readings.push({
      x: n[i]!,
      y: n[i + 1]!,
      pressure: n[i + 2]!,
      tangentialPressure: n[i + 3]!,
      tiltX: n[i + 4]!,
      tiltY: n[i + 5]!,
      twist: n[i + 6]!,
      width: n[i + 7]!,
      height: n[i + 8]!,
      time: n[i + 9]!,
    });
  }
  const predicted: PredictedReading[] = [];
  for (; i + PREDICTED_LEN <= n.length; i += PREDICTED_LEN) predicted.push({ x: n[i]!, y: n[i + 1]!, time: n[i + 2]! });
  if (flags & TOUCH)
    return {
      id: Math.floor(flags / ID_SHIFT),
      type: "touch",
      primary: !!(flags & TOUCH_PRIMARY),
      ...(flags & POINTER_ONLY ? { part: "pointer" as const } : flags & COMPAT_ONLY ? { part: "compat" as const } : {}),
      eraser: false,
      barrel: false,
      readings,
      predicted,
    };
  if (flags & PEN_MOUSE) return { ...MOUSE_SOURCE, mousePressure: !!(flags & MOUSE_PRESSURE), readings, predicted };
  return {
    id: PEN_ID,
    type: "pen",
    primary: true,
    eraser: !!(flags & PEN_ERASER),
    barrel: !!(flags & PEN_BARREL),
    readings,
    predicted,
  };
}

interface PointerState {
  buttons: number;
  position: { x: number; y: number; button: number; modifiers: number };
  source: PointerSource;
  compatSuppressed: boolean;
}

// -- pointer capture: one owner per pointer; the host routes input to it even outside its hitbox.
export const pointerCaptures = new Map<number, MirrorElement>();
const disconnectingCaptureRoots = new Set<MirrorNode>();
/** The pointer the host last reported. `compatSuppressed`: a pen's `pointerdown` was cancelled, so its mouse events are not fired until it lifts. */
export const pointer: PointerState = {
  buttons: 0,
  position: { x: 0, y: 0, button: -1, modifiers: 0 },
  source: MOUSE_SOURCE,
  compatSuppressed: false,
};
/** Touch contacts that are down, by `pointerId`: each is a pointer of its own. */
export const touches = new Map<number, PointerState>();
/** Implicit captures not announced yet: a touch's, taken before its `pointerdown` listeners run. */
const pendingImplicit = new Set<number>();

function captureEvent(owner: MirrorElement, name: string, pointerId: number): void {
  if (pendingImplicit.has(pointerId)) return;
  if (hasListener(owner, name)) {
    const state = touches.get(pointerId) ?? pointer;
    const p = state.position;
    propagate(
      owner,
      name,
      synthesize(name, p.x, p.y, p.button, p.modifiers, undefined, undefined, undefined, state.source),
    );
  }
}
export function releaseCapturedPointer(pointerId: number, owner?: MirrorElement): void {
  const current = pointerCaptures.get(pointerId);
  if (!current || (owner && owner !== current)) return;
  pointerCaptures.delete(pointerId);
  __sk_pointer_capture(current.__skId, pointerId, false);
  captureEvent(current, "lostpointercapture", pointerId);
  pendingImplicit.delete(pointerId);
}
export function releaseCapturesIn(node: MirrorNode): void {
  disconnectingCaptureRoots.add(node);
  try {
    for (const [pointerId, owner] of Array.from(pointerCaptures)) {
      for (let n: MirrorNode | null = owner; n; n = n.parentNode) {
        if (n === node) {
          releaseCapturedPointer(pointerId, owner);
          break;
        }
      }
    }
  } finally {
    disconnectingCaptureRoots.delete(node);
  }
}

function capturePointer(el: MirrorElement, pointerId: number): void {
  if (!el.__skId || (!el.parentNode && el !== appRoot))
    throw new DOMException("The capture target is not connected", "InvalidStateError");
  for (let n: MirrorNode | null = el; n; n = n.parentNode) {
    if (disconnectingCaptureRoots.has(n)) return;
  }
  const previous = pointerCaptures.get(pointerId);
  if (previous === el) return;
  if (!__sk_pointer_capture(el.__skId, pointerId, true)) return;
  pointerCaptures.set(pointerId, el);
  // A lost-capture listener may transfer capture again: publish the owner first, announce only if it stays.
  if (previous) captureEvent(previous, "lostpointercapture", pointerId);
  pendingImplicit.delete(pointerId);
  if (pointerCaptures.get(pointerId) === el) captureEvent(el, "gotpointercapture", pointerId);
}

/**
 * A touch contact's implicit capture (Pointer Events §9.5): as if `setPointerCapture` were called on the target just
 * before its `pointerdown` listeners run; `gotpointercapture` follows them unless one released or moved it.
 */
export function beginImplicitCapture(target: MirrorNode, pointerId: number): void {
  if (target.kind !== 1 || !target.__skId) return;
  const el = target as MirrorElement;
  if (!__sk_pointer_capture(el.__skId, pointerId, true)) return;
  pointerCaptures.set(pointerId, el);
  pendingImplicit.add(pointerId);
}
export function announceImplicitCapture(pointerId: number): void {
  if (!pendingImplicit.delete(pointerId)) return;
  const owner = pointerCaptures.get(pointerId);
  if (owner) captureEvent(owner, "gotpointercapture", pointerId);
}

defineDom("setPointerCapture", function (this: MirrorElement, pointerId: number) {
  const touch = touches.get(pointerId);
  if (!touch && pointerId !== MOUSE_ID && (pointerId !== PEN_ID || pointer.source.type !== "pen"))
    throw new DOMException("No active pointer with this id", "NotFoundError");
  if (touch ? !touch.buttons : !pointer.buttons || pointerId !== pointer.source.id) return;
  capturePointer(this, pointerId);
});
defineDom("hasPointerCapture", function (this: MirrorElement, pointerId: number) {
  return pointerCaptures.get(pointerId) === this;
});
defineDom("releasePointerCapture", function (this: MirrorElement, pointerId: number) {
  releaseCapturedPointer(pointerId, this);
});
