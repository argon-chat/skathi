import type { MirrorElement, MirrorNode } from "./types";
import { isFileInput, isTextWidget } from "./file-input";
import {
  ACTIVATING_EVENTS,
  CODE_TO_NAME,
  activatableOnPath,
  buttonsFor,
  detailFor,
  hasListener,
  mods,
  propagate,
  propagateAtDocument,
  synthesize,
  withProps,
} from "./events";
import { focusOvertaken, noteHostFocus, withRelatedTarget } from "./focus";
import { dndRearm, dndState } from "./listeners";
import { pointer, pointerCaptures, releaseCapturedPointer } from "./pointer-capture";
import { nodeById } from "./nodes";
import { deliverFrameMessage, isFrameEl } from "./media";
import { DERIVED, DERIVED_OF } from "./event-names";
import type { DerivedEventName } from "./event-names";
import { activate } from "./links";
import { warnCompat } from "../compat";
import { blocksImplicitSubmission, implicitSubmit } from "./forms";
import { noteActivation } from "./activation";
import { theGlobal } from "./interfaces";
import { isSelectEl, pickOption } from "./select";
import { isRangeInput, rangeValue } from "./controls";

// HTML drag and drop: gpui has none, so the runtime runs the gesture off the host's mouse events: a left press on a
// `draggable="true"` element then a move past 4 px starts a drag; each move fires `drag`, `dragenter` / `dragleave`
// and `dragover` (cancelled: a drop target); release fires `drop` then `dragend` and swallows its `click`; Escape
// cancels. Divergences: DataTransfer stays read/write for the whole drag; no drag image; a drag ends at the window's
// edge.

const DRAG_THRESHOLD = 4;

export interface DndGesture {
  /** `pressed`: pressed on a draggable, not yet moved far enough. `done`: cancelled while the button is still down; its release is swallowed. */
  phase: "pressed" | "dragging" | "done";
  source: MirrorElement;
  x: number;
  y: number;
  dt: DataTransferLike | null;
  over: MirrorElement | null;
  op: string;
}

interface DataTransferLike {
  dropEffect: string;
  effectAllowed: string;
}

function draggableAt(n: MirrorNode | null | undefined): MirrorElement | null {
  // A press in a text field selects text in it, whatever is around it.
  if (!n || isTextWidget(n)) return null;
  for (let p: MirrorNode | null = n; p; p = p.parentNode) {
    if (p.kind === 1 && (p as MirrorElement).attrs?.draggable === "true") return p as MirrorElement;
  }
  return null;
}

const DRAG_CANCELABLE = new Set(["dragstart", "drag", "dragenter", "dragover", "drop"]);

function dragEvent(
  name: string,
  x: number,
  y: number,
  d: number,
  dt: DataTransferLike,
  related: MirrorNode | null,
): Event {
  const init = {
    bubbles: true,
    cancelable: DRAG_CANCELABLE.has(name),
    composed: true,
    ...mods(d),
    clientX: x,
    clientY: y,
    button: 0,
    buttons: name === "drop" || name === "dragend" ? 0 : 1,
    dataTransfer: dt,
  };
  let ev: Event;
  try {
    const Drag = theGlobal("DragEvent") as (new (type: string, init: Record<string, unknown>) => Event) | undefined;
    ev =
      typeof Drag === "function" ? new Drag(name, init) : withProps(new MouseEvent(name, init), { dataTransfer: dt });
  } catch {
    ev = withProps(new CustomEvent(name, init), {
      ...mods(d),
      clientX: x,
      clientY: y,
      button: 0,
      buttons: init.buttons,
      dataTransfer: dt,
    });
  }
  return withRelatedTarget(ev, related);
}

function initialDropEffect(effectAllowed: string): string {
  switch (effectAllowed) {
    case "none":
      return "none";
    case "link":
    case "linkMove":
      return "link";
    case "move":
      return "move";
    default:
      return "copy";
  }
}

function dropAllowed(op: string, effectAllowed: string): boolean {
  if (op === "none") return false;
  if (effectAllowed === "all" || effectAllowed === "uninitialized") return true;
  return effectAllowed.toLowerCase().includes(op);
}

function dndPress(target: MirrorNode, x: number, y: number, button: number, proceed: boolean): void {
  const source = button === 0 && proceed ? draggableAt(target) : null;
  if (!source) return;
  dndState.gesture = { phase: "pressed", source, x, y, dt: null, over: null, op: "none" };
  dndRearm();
}

function dndFinish(swallowClick: boolean): void {
  dndState.gesture = null;
  dndState.swallowClick = swallowClick;
  dndRearm();
}

function dndStart(g: DndGesture, x: number, y: number, d: number): boolean {
  // A browser cancels the pointer a drag takes over (Pointer Events §4.1.3.3).
  const captured = pointerCaptures.get(1);
  if (captured) {
    propagate(captured, "pointercancel", synthesize("pointercancel", x, y, -1, d, undefined));
    releaseCapturedPointer(1);
  }
  const DT = theGlobal("DataTransfer") as (new () => DataTransferLike) | undefined;
  const dt: DataTransferLike = typeof DT === "function" ? new DT() : { dropEffect: "none", effectAllowed: "none" };
  dt.effectAllowed = "uninitialized";
  if (!propagate(g.source, "dragstart", dragEvent("dragstart", x, y, d, dt, null))) {
    dndFinish(false);
    return false;
  }
  g.phase = "dragging";
  g.dt = dt;
  dndRearm();
  return true;
}

function dndMove(g: DndGesture, under: MirrorNode | null | undefined, x: number, y: number, d: number): void {
  const dt = g.dt!;
  if (!propagate(g.source, "drag", dragEvent("drag", x, y, d, dt, null))) {
    dndCancel(g, x, y, d, true);
    return;
  }
  const over =
    under && under.kind === 1 ? (under as MirrorElement) : ((under?.parentNode as MirrorElement | null) ?? null);
  if (over !== g.over) {
    // The new element hears its `dragenter` before the old one its `dragleave`.
    const prev = g.over;
    g.over = over;
    if (over) {
      dt.dropEffect = initialDropEffect(dt.effectAllowed);
      propagate(over, "dragenter", dragEvent("dragenter", x, y, d, dt, prev));
    }
    if (prev) propagate(prev, "dragleave", dragEvent("dragleave", x, y, d, dt, over));
  }
  g.op = "none";
  if (g.over) {
    dt.dropEffect = initialDropEffect(dt.effectAllowed);
    if (
      !propagate(g.over, "dragover", dragEvent("dragover", x, y, d, dt, null)) &&
      dropAllowed(dt.dropEffect, dt.effectAllowed)
    )
      g.op = dt.dropEffect;
  }
}

function dndDrop(g: DndGesture, x: number, y: number, d: number): void {
  const dt = g.dt!;
  let effect = "none";
  if (g.over && g.op !== "none") {
    dt.dropEffect = g.op;
    if (!propagate(g.over, "drop", dragEvent("drop", x, y, d, dt, null))) effect = dt.dropEffect;
  } else if (g.over) {
    propagate(g.over, "dragleave", dragEvent("dragleave", x, y, d, dt, null));
  }
  dt.dropEffect = effect;
  dndFinish(true);
  propagate(g.source, "dragend", dragEvent("dragend", x, y, d, dt, null));
}

function dndCancel(g: DndGesture, x: number, y: number, d: number, held: boolean): void {
  const dt = g.dt!;
  if (g.over) propagate(g.over, "dragleave", dragEvent("dragleave", x, y, d, dt, null));
  dt.dropEffect = "none";
  if (held) {
    g.phase = "done";
    g.over = null;
    dndRearm();
  } else dndFinish(false);
  propagate(g.source, "dragend", dragEvent("dragend", x, y, d, dt, null));
}

/** The drag's share of a host event: its answer when the drag consumed it, undefined otherwise. */
function dndDispatch(
  name: string,
  nodeId: SkNodeId,
  x: number,
  y: number,
  d: number,
  key: string | undefined,
): boolean | undefined {
  if (dndState.swallowClick && (name === "click" || name === "mousedown" || name === "keydown")) {
    dndState.swallowClick = false;
    if (name === "click") return false;
  }
  if (name === "selectstart") return draggableAt(nodeById(nodeId)) ? false : undefined;
  const g = dndState.gesture;
  if (!g) return undefined;
  const held = !!(pointer.buttons & 1);
  if (g.phase === "pressed") {
    if (name === "mouseup" || (name === "mousemove" && !held)) dndFinish(false);
    else if (name === "mousemove" && (Math.abs(x - g.x) > DRAG_THRESHOLD || Math.abs(y - g.y) > DRAG_THRESHOLD)) {
      if (!dndStart(g, x, y, d)) return undefined;
      dndMove(g, nodeById(nodeId), x, y, d);
      return true;
    }
    return undefined;
  }
  if (g.phase === "done") {
    if (name === "mouseup") dndFinish(true);
    else if (name === "mousemove" && !held) dndFinish(false);
    return name === "mouseup" || name === "mousemove" ? true : undefined;
  }
  switch (name) {
    case "mousemove":
      if (held) dndMove(g, nodeById(nodeId), x, y, d);
      else dndCancel(g, x, y, d, false);
      return true;
    case "mouseup":
      dndDrop(g, x, y, d);
      return true;
    case "keydown":
      if (key !== "Escape") return undefined;
      dndCancel(g, pointer.position.x, pointer.position.y, pointer.position.modifiers, true);
      return false;
  }
  return undefined;
}

__sk_set_dispatcher((nodeId, evCode, a, b, c, d, key, data, x, y) => {
  const name = CODE_TO_NAME[evCode];
  if (!name) return true;
  if (name === "mousedown" || name === "mousemove" || name === "mouseup" || name === "pointercancel") {
    pointer.position = { x: a, y: b, button: c, modifiers: d };
    pointer.buttons = buttonsFor(name, c);
    // A lost native release (capture stolen by another window, for example)
    // must terminate the old gesture before an unpressed move is delivered.
    if (name === "mousemove" && !pointer.buttons) {
      const captured = pointerCaptures.get(1);
      if (captured) {
        propagate(captured, "pointercancel", synthesize("pointercancel", a, b, -1, d, key));
        releaseCapturedPointer(1);
      }
    }
  }
  if (dndState.gesture || dndState.swallowClick || (name === "selectstart" && dndState.seen)) {
    const answer = dndDispatch(name, nodeId, a, b, d, key);
    if (answer !== undefined) return answer;
  }
  const retarget = name === "mousemove" || name === "mouseup" || name === "pointercancel";
  const target = (retarget && pointerCaptures.get(1)) || nodeById(nodeId);
  if (!target) return true;
  if (name === "keydown" && key === "Escape" && globalThis.__SK_WINDOW__?.escapeFullscreen?.()) return false;
  if (name === "wheel" && x !== undefined) data = { clientX: x, clientY: y ?? 0 };
  // The host delivers `selectionchange` at the root, whose listener bit the
  // document's listeners arm; a browser fires it AT the document.
  if (name === "selectionchange") return propagateAtDocument(name, synthesize(name, a, b, c, d, key));
  if (name === "message") return deliverFrameMessage(target, key);
  // `document.activeElement` follows the host's focus moves; blur arrives before the next focus. `a` is the
  // move's other node (0: none), the events' `relatedTarget`. A handler that moves the focus on cancels the rest
  // of the move: the host skips `focus`, here `focusin` is skipped and `focusout` loses its `relatedTarget`.
  let focusChange = 0;
  let focusRelated: MirrorNode | null = null;
  if (name === "focus" || name === "blur") {
    focusChange = noteHostFocus(name, target as MirrorElement);
    focusRelated = (a && nodeById(a)) || null;
  }
  // Sticky activation: the windowing prelude arms it from the engine's own EventTarget, which gpui events never
  // reach, so the host's events call its hook here; without it `beforeunload` could never veto a close. The host
  // forwards only events something listens for, so a host `click`, `mouseup` or `pointerup` stands in for the
  // press. `el.click()` and `dispatchEvent` never come through here.
  if (ACTIVATING_EVENTS.has(name)) {
    globalThis.__skStickyActivation?.();
    noteActivation(name, key ?? undefined);
  }
  // input: mirror the payload (the new value) onto the node BEFORE dispatch, so `e.target.value` reads like a browser's.
  if (name === "input" && isSelectEl(target)) return pickOption(target as MirrorElement, a);
  // Range controls use the input channel twice: b=0 is a live edit,
  // b=1 commits the gesture. Only the latter dispatches change.
  if (name === "input" && isRangeInput(target as MirrorElement)) (target as MirrorElement)._rangeDirty = true;
  if (name === "input" && isRangeInput(target as MirrorElement) && b === 1) {
    if (key != null) (target as MirrorElement)._value = rangeValue(target as MirrorElement, key);
    return propagate(target, "change", synthesize("change", 0, 0, 0, 0, undefined));
  }
  if ((name === "input" || name === "change") && key != null)
    // The storage, not the accessor: the host already holds this value, and
    // the accessor would push it straight back.
    (target as MirrorElement)._value = key;
  // An image's answer is recorded BEFORE the event so `onload` sees the size; one for a `src` the element no
  // longer has is stale and dropped.
  const isImg = target.kind === 1 && (target as MirrorElement).tag === "img";
  if (isImg && (name === "load" || name === "error")) {
    const img = target as MirrorElement;
    if (key == null || key !== img.attrs.src) return true;
    img._img = { src: key, ok: name === "load", w: name === "load" ? a : 0, h: name === "load" ? b : 0 };
  }
  // `preventDefault()` on a derived pointer event cancels its source.
  const derived = DERIVED_OF[name];
  let proceed = true;
  if (derived) {
    for (const dn of derived) {
      if (!DERIVED[dn as DerivedEventName].pre) continue;
      if (!hasListener(target, dn)) continue;
      if (!propagate(target, dn, synthesize(dn, a, b, c, d, key))) proceed = false;
    }
  }
  // Skipping the source event when nothing listens is safe: the only `preventDefault()` that could set the flag
  // is the one below, which already made `proceed` false. A click whose path holds a link, label or file input
  // must be built regardless: a `preventDefault()` on the path cancels the activation.
  const link = name === "click" && activatableOnPath(target);
  if (link || hasListener(target, name)) {
    const ev = synthesize(name, a, b, c, d, key, data, isImg || isFrameEl(target) ? "img" : undefined);
    if (focusChange) withRelatedTarget(ev, focusRelated);
    if (!proceed && ev.preventDefault) ev.preventDefault();
    proceed = propagate(target, name, ev) && proceed;
    if (link && proceed) activate(target, true);
    if (name === "contextmenu" && proceed) {
      warnCompat(
        "contextmenu:default",
        "an uncancelled `contextmenu` — a browser would open its own context menu here; Skathi has " +
          "none (not even in a text field), so nothing opens. Call e.preventDefault() and show your own",
      );
    }
  }
  if (
    proceed &&
    isFileInput(target) &&
    ((name === "keydown" && key === "Enter") || (name === "keyup" && key === " "))
  ) {
    (target as MirrorElement).click();
  }
  if (proceed && name === "keydown" && key === "Enter" && blocksImplicitSubmission(target)) {
    implicitSubmit(target as MirrorElement);
  }
  if (derived) {
    for (const dn of derived) {
      if (DERIVED[dn as DerivedEventName].pre) continue;
      if (dn === "dblclick" && detailFor("click", d) !== 2) continue;
      if (!hasListener(target, dn)) continue;
      if (focusChange) {
        const overtaken = focusOvertaken(focusChange);
        // A `focus` handler moved the focus on: no `focusin` (Chromium).
        if (dn === "focusin" && overtaken) continue;
        propagate(target, dn, withRelatedTarget(synthesize(dn, a, b, c, d, key), overtaken ? null : focusRelated));
        continue;
      }
      propagate(target, dn, synthesize(dn, a, b, c, d, key));
    }
  }
  if (!proceed && name === "mousedown" && isTextWidget(target)) {
    warnCompat(
      "mousedown-prevent:text-field",
      "`mousedown.preventDefault()` on an <input> or <textarea> — a browser leaves the focus where it was; " +
        "Skathi's text field has already focused itself and placed the caret",
    );
  }
  if (!proceed && name === "wheel" && (target as MirrorElement).tag === "textarea") {
    warnCompat(
      "wheel-prevent:textarea",
      "`wheel.preventDefault()` on a <textarea> — a browser does not scroll it; Skathi's textarea has " +
        "already scrolled its own content (the boxes around it stay put)",
    );
  }
  if (name === "mouseup" || name === "pointercancel") releaseCapturedPointer(1);
  if (name === "mousedown" && dndState.seen && !dndState.gesture) dndPress(target, a, b, c, proceed);
  return proceed;
});
