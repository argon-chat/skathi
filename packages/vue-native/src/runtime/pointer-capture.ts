import type { MirrorElement, MirrorNode } from "./types";
import { defineDom } from "./dom-members";
import { appRoot } from "../runtime";
import { hasListener, propagate, synthesize } from "./events";

// -- pointer capture: one owner per pointer; the host routes input to it even outside its hitbox.
export const pointerCaptures = new Map<number, MirrorElement>();
const disconnectingCaptureRoots = new Set<MirrorNode>();
export const pointer = { buttons: 0, position: { x: 0, y: 0, button: -1, modifiers: 0 } };

function captureEvent(owner: MirrorElement, name: string): void {
  if (hasListener(owner, name)) {
    const p = pointer.position;
    propagate(owner, name, synthesize(name, p.x, p.y, p.button, p.modifiers, undefined));
  }
}
export function releaseCapturedPointer(pointerId: number, owner?: MirrorElement): void {
  const current = pointerCaptures.get(pointerId);
  if (!current || (owner && owner !== current)) return;
  pointerCaptures.delete(pointerId);
  __vn_pointer_capture(current.__vnId, pointerId, false);
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
  if (pointerId !== 1) throw new DOMException("No active pointer with this id", "NotFoundError");
  if (!pointer.buttons) return;
  if (!this.__vnId || (!this.parentNode && this !== appRoot))
    throw new DOMException("The capture target is not connected", "InvalidStateError");
  for (let n: MirrorNode | null = this; n; n = n.parentNode) {
    if (disconnectingCaptureRoots.has(n)) return;
  }
  const previous = pointerCaptures.get(pointerId);
  if (previous === this) return;
  if (!__vn_pointer_capture(this.__vnId, pointerId, true)) return;
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
