import type { MirrorElement } from "./types";
import { defineDom } from "./dom-members";
import { nodeById } from "./nodes";
import { mark, vacant } from "./realm";
import { propagateAtDocument } from "./events";

// The Pointer Lock API. The host keeps the lock and its rules (focus, gesture, Escape, release); a request is answered
// on the window's next frame by `pointerlockchange` or `pointerlockerror`, which carry the request they settle.

type Settle = { resolve: () => void; reject: (e: unknown) => void };
const pending = new Map<number, Settle>();

defineDom("requestPointerLock", function (this: MirrorElement, options?: { unadjustedMovement?: unknown }) {
  const request = __sk_pointer_lock_request(this.__skId, !!options?.unadjustedMovement);
  return new Promise<void>((resolve, reject) => pending.set(request, { resolve, reject }));
});

const doc = globalThis.document;
if (doc) {
  if (vacant(doc, "exitPointerLock"))
    Object.defineProperty(doc, "exitPointerLock", {
      value: mark(() => __sk_pointer_lock_exit()),
      configurable: true,
      writable: true,
    });
  if (vacant(doc, "pointerLockElement"))
    Object.defineProperty(doc, "pointerLockElement", {
      get: mark(() => nodeById(__sk_pointer_lock_element()) ?? null),
      configurable: true,
    });
}

/** The host's answer: fire the event at `document`, then settle the request it names. */
export function pointerLockEvent(name: "pointerlockchange" | "pointerlockerror", request: number, why?: string) {
  propagateAtDocument(name, new Event(name, { bubbles: true }));
  const settle = pending.get(request);
  if (!settle) return true;
  pending.delete(request);
  if (name === "pointerlockchange") settle.resolve();
  else {
    const text = why ?? "";
    const at = text.indexOf(": ");
    settle.reject(
      at > 0 ? new DOMException(text.slice(at + 2), text.slice(0, at)) : new DOMException(text, "NotAllowedError"),
    );
  }
  return true;
}
