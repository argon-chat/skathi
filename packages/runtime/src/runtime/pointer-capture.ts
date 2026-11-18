import type { MirrorElement, MirrorNode } from "./types";
import { defineDom } from "./dom-members";
import { appRoot } from "./root";
import { hasListener, propagate, synthesize } from "./events";

export const MOUSE_ID = 1;
export const PEN_ID = 2;

/** One pen (or, positions only, mouse) reading the host reports: window CSS px, degrees. */
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
}

/** What drove a host mouse event: the mouse, or a pen with its readings (oldest first, the event's own last). */
export interface PointerSource {
  id: number;
  type: "mouse" | "pen";
  eraser: boolean;
  barrel: boolean;
  readings: PointerReading[];
}

export const MOUSE_SOURCE: PointerSource = { id: MOUSE_ID, type: "mouse", eraser: false, barrel: false, readings: [] };

// Lockstep with sk-tree's pen.rs.
const PEN_ERASER = 1;
const PEN_BARREL = 2;
const PEN_MOUSE = 8;
const READING_LEN = 9;

/** The host's pen packet (`[flags, ...readings]`); the mouse when there is none. */
export function pointerSource(raw: unknown): PointerSource {
  if (!Array.isArray(raw) || raw.length < 1 + READING_LEN) return MOUSE_SOURCE;
  const n = raw as number[];
  const flags = n[0] ?? 0;
  const readings: PointerReading[] = [];
  for (let i = 1; i + READING_LEN <= n.length; i += READING_LEN) {
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
    });
  }
  if (flags & PEN_MOUSE) return { ...MOUSE_SOURCE, readings };
  return { id: PEN_ID, type: "pen", eraser: !!(flags & PEN_ERASER), barrel: !!(flags & PEN_BARREL), readings };
}

// -- pointer capture: one owner per pointer; the host routes input to it even outside its hitbox.
export const pointerCaptures = new Map<number, MirrorElement>();
const disconnectingCaptureRoots = new Set<MirrorNode>();
/** The pointer the host last reported. `compatSuppressed`: a pen's `pointerdown` was cancelled, so its mouse events are not fired until it lifts. */
export const pointer = {
  buttons: 0,
  position: { x: 0, y: 0, button: -1, modifiers: 0 },
  source: MOUSE_SOURCE,
  compatSuppressed: false,
};

function captureEvent(owner: MirrorElement, name: string): void {
  if (hasListener(owner, name)) {
    const p = pointer.position;
    propagate(
      owner,
      name,
      synthesize(name, p.x, p.y, p.button, p.modifiers, undefined, undefined, undefined, pointer.source),
    );
  }
}
export function releaseCapturedPointer(pointerId: number, owner?: MirrorElement): void {
  const current = pointerCaptures.get(pointerId);
  if (!current || (owner && owner !== current)) return;
  pointerCaptures.delete(pointerId);
  __sk_pointer_capture(current.__skId, pointerId, false);
  captureEvent(current, "lostpointercapture");
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
defineDom("setPointerCapture", function (this: MirrorElement, pointerId: number) {
  if (pointerId !== MOUSE_ID && (pointerId !== PEN_ID || pointer.source.type !== "pen"))
    throw new DOMException("No active pointer with this id", "NotFoundError");
  if (!pointer.buttons || pointerId !== pointer.source.id) return;
  if (!this.__skId || (!this.parentNode && this !== appRoot))
    throw new DOMException("The capture target is not connected", "InvalidStateError");
  for (let n: MirrorNode | null = this; n; n = n.parentNode) {
    if (disconnectingCaptureRoots.has(n)) return;
  }
  const previous = pointerCaptures.get(pointerId);
  if (previous === this) return;
  if (!__sk_pointer_capture(this.__skId, pointerId, true)) return;
  pointerCaptures.set(pointerId, this);
  // A lost-capture listener may transfer capture again: publish the owner first, announce only if it stays.
  if (previous) captureEvent(previous, "lostpointercapture");
  if (pointerCaptures.get(pointerId) === this) captureEvent(this, "gotpointercapture");
});
defineDom("hasPointerCapture", function (this: MirrorElement, pointerId: number) {
  return pointerCaptures.get(pointerId) === this;
});
defineDom("releasePointerCapture", function (this: MirrorElement, pointerId: number) {
  releaseCapturedPointer(pointerId, this);
});
