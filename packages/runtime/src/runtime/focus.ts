import type { MirrorElement, MirrorNode } from "./types";
import { isMirrorNode } from "./listeners";
import { nodeById } from "./nodes";
import { isDisabledControl } from "./forms";
import { defineDom } from "./dom-members";
import { warnCompat } from "../compat";
import { appRoot } from "../runtime";
import { hasListener, propagate, synthesize } from "./events";

// Focus: the host moves focus (`__sk_focus`); the four events fire here in browser order. A node not yet built is
// queued and its events arrive a frame later (advised). `focus()` never scrolls.
// A handler that moves focus while another move's events run abandons that move, as in Chromium; during
// `blur`/`focusout` nothing is focused (`activeElement` is `body`). reka's FocusScope relies on this: its `focusout`
// handler pulls focus back into a dialog, and the abandoned target must not get `focus`.

let focusedNode: MirrorElement | null = null;

let focusChanges = 0;

/** The node a `focus()` is moving to while the old node hears `blur`/`focusout`: the host has focused it, a browser has not yet. */
let arriving: MirrorElement | null = null;

/** The host moved the focus to (`focus`) or off (`blur`) `target`; returns the move's number for {@link focusOvertaken}. */
export function noteHostFocus(name: "focus" | "blur", target: MirrorElement): number {
  arriving = null;
  if (name === "focus") focusedNode = target;
  else if (focusedNode === target) focusedNode = null;
  return ++focusChanges;
}

export const focusOvertaken = (change: number): boolean => focusChanges !== change;

function fireFocusEvent(target: MirrorElement, name: string, related: MirrorElement | null): void {
  if (!hasListener(target, name)) return;
  propagate(target, name, withRelatedTarget(synthesize(name, 0, 0, 0, 0, undefined), related));
}

export function withRelatedTarget(ev: SkEvent, related: MirrorNode | null): SkEvent {
  try {
    Object.defineProperty(ev, "relatedTarget", { value: related, configurable: true });
  } catch {
    /* the engine's own accessor: leave it */
  }
  return ev;
}

function focusMoved(next: MirrorElement, old: MirrorElement | null): void {
  const change = ++focusChanges;
  arriving = null;
  if (old) {
    focusedNode = null;
    arriving = next;
    try {
      fireFocusEvent(old, "blur", next);
      fireFocusEvent(old, "focusout", focusChanges === change ? next : null);
    } finally {
      arriving = null;
    }
    if (focusChanges !== change) return;
  }
  focusedNode = next;
  fireFocusEvent(next, "focus", old);
  if (focusChanges !== change) return;
  fireFocusEvent(next, "focusin", old);
}

const focusable = (el: MirrorElement): boolean =>
  isMirrorNode(el) && el.kind === 1 && nodeById(el.__skId) === el && !isDisabledControl(el);

defineDom("focus", function (this: MirrorElement) {
  if (!focusable(this)) return;
  if (typeof __sk_focus !== "function") {
    warnCompat("focus", "element.focus() is a no-op: this host has no programmatic focus (__sk_focus)");
    return;
  }
  if (this === arriving) {
    focusMoved(this, null);
    return;
  }
  const answer = __sk_focus(this.__skId, 0);
  if (answer.startsWith("moved:")) {
    const old = (nodeById(Number(answer.slice(6))) as MirrorElement | undefined) ?? null;
    // The host's old node may be the target of a move in progress: it never got focus, nothing to blur.
    focusMoved(this, old === arriving || old === this ? null : old);
  } else if (answer === "pending") {
    warnCompat(
      "focus:deferred",
      "element.focus() on an element that has not been painted yet (mounted in this same update) takes " +
        "effect on the next frame: until then document.activeElement is unchanged, and its focus/focusin " +
        "events fire then rather than before focus() returns",
    );
  }
});
defineDom("blur", function (this: MirrorElement) {
  if (!isMirrorNode(this) || typeof __sk_focus !== "function") return;
  if (this === arriving) return;
  if (__sk_focus(this.__skId, 1) === "blurred") {
    focusChanges++;
    arriving = null;
    if (focusedNode === this) focusedNode = null;
    fireFocusEvent(this, "blur", null);
    fireFocusEvent(this, "focusout", null);
  }
});

export function activeElement(): MirrorElement | null {
  if (focusedNode && nodeById(focusedNode.__skId) === focusedNode) return focusedNode;
  return appRoot;
}
