import { defineDomGet } from "./dom-members";
import type { Buckets, MirrorElement, MirrorNode, PropagationTarget } from "./types";
import { MOUSE_SOURCE, pointer } from "./pointer-capture";
import type { PointerReading, PointerSource } from "./pointer-capture";
import { EDIT_TYPES, EV_CODES, MEDIA_EVENTS, NOT_CANCELABLE, NO_BUBBLE, POINTER_EVENTS } from "./event-names";
import { ClipboardEventClass, isResetButton, isSubmitButton } from "./forms";
import { traceCallbackBegin, traceCallbackEnd } from "./realm";
import { bucketsAlive, disposeGlobalListener, globalHandlers, isMirrorNode, syncListenerFlag } from "./listeners";
import { handlers } from "./nodes";
import { fileEvent, isColorInput, isFileInput, isLabel } from "./file-input";
import { isActivationEvent, isLink } from "./links";
import { appRoot } from "./root";
import { theGlobal } from "./interfaces";
import { isMirrorEl, isToggleInput, ownExpando } from "./controls";

// `on<event>` IDL handlers: one listener per element and type, kept in its place in the listener order while
// the handler changes; `null` removes it; a handler returning `false` cancels the event. On a non-mirror
// object the property stays a plain expando.
const HANDLER_ATTRS = [
  "abort",
  "animationcancel",
  "animationend",
  "animationiteration",
  "animationstart",
  "auxclick",
  "beforeinput",
  "blur",
  "cancel",
  "canplay",
  "canplaythrough",
  "change",
  "click",
  "close",
  "contextmenu",
  "copy",
  "cut",
  "dblclick",
  "durationchange",
  "emptied",
  "ended",
  "error",
  "focus",
  "input",
  "invalid",
  "keydown",
  "keypress",
  "keyup",
  "load",
  "loadeddata",
  "loadedmetadata",
  "loadstart",
  "mousedown",
  "mouseenter",
  "mouseleave",
  "mousemove",
  "mouseout",
  "mouseover",
  "mouseup",
  "paste",
  "pause",
  "play",
  "playing",
  "pointercancel",
  "pointerdown",
  "pointerenter",
  "pointerleave",
  "pointermove",
  "pointerout",
  "pointerover",
  "pointerup",
  "progress",
  "ratechange",
  "reset",
  "scroll",
  "scrollend",
  "seeked",
  "seeking",
  "select",
  "selectionchange",
  "selectstart",
  "submit",
  "timeupdate",
  "toggle",
  "transitioncancel",
  "transitionend",
  "transitionrun",
  "transitionstart",
  "volumechange",
  "waiting",
  "wheel",
];
for (const type of HANDLER_ATTRS) {
  const prop = `on${type}`;
  defineDomGet(
    prop,
    function (this: MirrorElement) {
      if (!isMirrorEl(this)) return undefined;
      return this._on?.[type]?.fn ?? null;
    },
    function (this: MirrorElement, v: unknown) {
      if (!isMirrorEl(this)) return ownExpando(this, prop, v);
      const fn = v !== null && (typeof v === "function" || typeof v === "object") ? v : null;
      const on = this._on ?? (this._on = {});
      const slot = on[type];
      if (!fn) {
        if (slot) {
          this.removeEventListener(type, slot.listener);
          delete on[type];
        }
        return;
      }
      if (slot) {
        slot.fn = fn;
        return;
      }
      const el = this;
      const entry = {
        fn,
        listener: (ev: Event) => {
          if (typeof entry.fn !== "function") return;
          const r = (entry.fn as (this: unknown, ev: Event) => unknown).call(el, ev);
          if (r === false) ev.preventDefault?.();
        },
      };
      on[type] = entry;
      this.addEventListener(type, entry.listener);
    } as (value: never) => void,
  );
}

// W3C event synthesis + propagation. The host sends one payload per input event at the deepest hit node; the
// mirror tree provides the ancestor path. Events use the engine's native classes when present, else CustomEvent.
export const mods = (d: number) => ({
  ctrlKey: !!(d & 1),
  shiftKey: !!(d & 2),
  altKey: !!(d & 4),
  metaKey: !!(d & 8),
});

const POINTER_INIT = {
  pointerId: 1,
  pointerType: "mouse",
  isPrimary: true,
  width: 1,
  height: 1,
};

// Pointer Events §"The button property": a pen's eraser is button 5, buttons bit 32.
const ERASER_BUTTON = 5;
const ERASER_BUTTONS = 32;

/** `button` and `buttons` of a pointer event from `src`: the eraser in place of the tip. */
function penButtons(src: PointerSource, button: number, buttons: number): { button: number; buttons: number } {
  if (!src.eraser) return { button, buttons };
  return {
    button: button === 0 ? ERASER_BUTTON : button,
    buttons: buttons & 1 ? (buttons & ~1) | ERASER_BUTTONS : buttons,
  };
}

/** The members a reading sets; a pen's pressure is 0 with no button down, as the spec has it. */
function readingInit(src: PointerSource, r: PointerReading | undefined, buttons: number) {
  if (src.type !== "pen" || !r)
    return {
      pointerId: src.id,
      pointerType: "mouse",
      isPrimary: true,
      width: 1,
      height: 1,
      pressure: buttons ? 0.5 : 0,
    };
  return {
    pointerId: src.id,
    pointerType: "pen",
    isPrimary: true,
    width: r.width > 0 ? r.width : 1,
    height: r.height > 0 ? r.height : 1,
    pressure: buttons ? r.pressure : 0,
    tangentialPressure: r.tangentialPressure,
    tiltX: r.tiltX,
    tiltY: r.tiltY,
    twist: r.twist,
  };
}

type MoveInit = Record<string, unknown> & { clientX: number; clientY: number; movementX?: number; movementY?: number };

/** An event for one reading: its `timeStamp` is the reading's time. */
function stamped(ev: PointerEvent, time: number | undefined): PointerEvent {
  if (time !== undefined && Number.isFinite(time))
    Object.defineProperty(ev, "timeStamp", { value: time, configurable: true });
  return ev;
}

/** A `pointermove`'s coalesced events: one per reading since the last move, or the event alone. */
function coalescedMoves(init: MoveInit, src: PointerSource, buttons: number): PointerEvent[] {
  const readings = src.readings;
  const base = { ...init, bubbles: false, cancelable: false };
  if (readings.length < 2) return [stamped(new PointerEvent("pointermove", base), readings[0]?.time)];
  const first = readings[0]!;
  const last = readings[readings.length - 1]!;
  let px = first.x - ((init.movementX ?? 0) - (last.x - first.x));
  let py = first.y - ((init.movementY ?? 0) - (last.y - first.y));
  return readings.map((r) => {
    const ev = new PointerEvent("pointermove", {
      ...base,
      ...readingInit(src, r, buttons),
      clientX: r.x,
      clientY: r.y,
      movementX: r.x - px,
      movementY: r.y - py,
    });
    px = r.x;
    py = r.y;
    return stamped(ev, r.time);
  });
}

/** A `pointermove`'s predicted events: the event at each predicted position, as Chromium makes them. */
function predictedMoves(init: MoveInit, src: PointerSource): PointerEvent[] {
  let px = init.clientX;
  let py = init.clientY;
  return src.predicted.map((p) => {
    const ev = new PointerEvent("pointermove", {
      ...init,
      bubbles: false,
      cancelable: false,
      clientX: p.x,
      clientY: p.y,
      movementX: p.x - px,
      movementY: p.y - py,
    });
    px = p.x;
    py = p.y;
    return stamped(ev, p.time);
  });
}

/**
 * `offsetX` / `offsetY`: from the target's padding edge, measured when first read. Coalesced events
 * measure from the event's target.
 */
function withOffsets(ev: Event, x: number, y: number, of: () => unknown = () => ev.target): Event {
  const edge = (): { left: number; top: number } => {
    const t = of() as Partial<MirrorElement> | null | undefined;
    if (!t || typeof t.getBoundingClientRect !== "function") return { left: 0, top: 0 };
    const r = t.getBoundingClientRect();
    return { left: r.left + (t.clientLeft ?? 0), top: r.top + (t.clientTop ?? 0) };
  };
  let at: { left: number; top: number } | undefined;
  Object.defineProperty(ev, "offsetX", { get: () => x - (at ??= edge()).left, configurable: true });
  Object.defineProperty(ev, "offsetY", { get: () => y - (at ??= edge()).top, configurable: true });
  return ev;
}

const BUTTON_BITS = [1, 4, 2, 8, 16];
/** `buttons`: which buttons are DOWN as the event fires (already up by `mouseup`, `click`, `dblclick`, `auxclick`, `contextmenu`). */
export function buttonsFor(name: string, c: number): number {
  switch (name) {
    case "gotpointercapture":
    case "lostpointercapture":
      return pointer.buttons;
    case "mousedown":
    case "pointerdown":
    case "mousemove":
    case "pointermove":
    case "mouseenter":
    case "mouseleave":
    case "pointerenter":
    case "pointerleave":
    case "mouseover":
    case "mouseout":
    case "pointerover":
    case "pointerout":
      return c >= 0 && c < BUTTON_BITS.length ? BUTTON_BITS[c]! : 0;
    default:
      return 0;
  }
}
/** `button`: the button whose state CHANGED (a move or enter/leave: 0 for a MouseEvent, -1 for a PointerEvent). The host passes the HELD button in `c` for a move. */
function buttonFor(name: string, c: number): number {
  switch (name) {
    case "mousemove":
    case "mouseenter":
    case "mouseleave":
    case "mouseover":
    case "mouseout":
      return 0;
    case "pointermove":
    case "pointerenter":
    case "pointerleave":
    case "pointerover":
    case "pointerout":
    case "pointercancel":
      return -1;
    case "dblclick":
      return 0;
    default:
      return c < 0 ? 0 : c;
  }
}
/** `detail`: the click count, carried above the four modifier bits of `d`; `dblclick` is 2, everything else 0. */
export function detailFor(name: string, d: number): number {
  if (name === "click" || name === "auxclick" || name === "mousedown" || name === "mouseup") return (d >>> 4) & 0xff;
  return name === "dblclick" ? 2 : 0;
}

/** Init for `input` / `beforeinput`: both bubble; only `beforeinput` cancels, and not while composing. */
function inputInit(name: string, a: number, b: number, data: unknown) {
  return {
    bubbles: true,
    cancelable: name === "beforeinput" && !b,
    composed: true,
    inputType: EDIT_TYPES[a] ?? "",
    data: typeof data === "string" ? data : null,
    isComposing: !!b,
  };
}

/** A `mousemove` / `pointermove`'s `movementX` / `movementY`, passed to `synthesize` as `data`; nothing for any other event. */
function movementOf(name: string, data: unknown): { movementX?: number; movementY?: number } {
  if (name !== "mousemove" && name !== "pointermove") return {};
  const m = data as { movementX?: unknown; movementY?: unknown } | null | undefined;
  return {
    movementX: typeof m?.movementX === "number" ? m.movementX : 0,
    movementY: typeof m?.movementY === "number" ? m.movementY : 0,
  };
}

/** A wheel's `clientX` / `clientY`, passed to `synthesize` as `data` (`a`/`b` are the deltas); 0, 0 if absent. */
function wheelAt(data: unknown): { clientX: number; clientY: number } {
  const at = data as { clientX?: unknown; clientY?: unknown } | null | undefined;
  return {
    clientX: typeof at?.clientX === "number" ? at.clientX : 0,
    clientY: typeof at?.clientY === "number" ? at.clientY : 0,
  };
}

export function synthesize(
  name: string,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
  data?: unknown,
  tag?: string,
  src: PointerSource = MOUSE_SOURCE,
): Event {
  const common = { bubbles: !NO_BUBBLE.has(name), cancelable: !NO_BUBBLE.has(name) && !NOT_CANCELABLE.has(name) };
  const M = mods(d);
  const moved = movementOf(name, data);
  try {
    if ((name === "copy" || name === "cut" || name === "paste") && ClipboardEventClass)
      return new ClipboardEventClass(name, { ...common, composed: true, clipboardData: data ?? null });
    if (tag === "img" && (name === "load" || name === "error")) return new Event(name, common);
    if (POINTER_EVENTS.has(name)) {
      const { button, buttons } = penButtons(src, buttonFor(name, c), buttonsFor(name, c));
      const fields = readingInit(src, src.readings[src.readings.length - 1], buttons);
      const init = { ...common, ...M, clientX: a, clientY: b, ...moved, button, buttons, ...fields };
      if (typeof PointerEvent === "function") {
        const move = name === "pointermove";
        const coalescedEvents = move ? coalescedMoves(init, src, buttons) : [];
        const predictedEvents = move ? predictedMoves(init, src) : [];
        const ev = withOffsets(new PointerEvent(name, { ...init, coalescedEvents, predictedEvents }), a, b);
        for (const e of [...coalescedEvents, ...predictedEvents]) withOffsets(e, e.clientX, e.clientY, () => ev.target);
        return ev;
      }
      // No PointerEvent class: a MouseEvent carrying the pointer fields reads the same.
      return withProps(new MouseEvent(name, init), fields);
    }
    switch (name) {
      case "click":
      case "dblclick":
      case "auxclick":
      case "contextmenu":
      case "mousedown":
      case "mouseup":
      case "mousemove":
      case "mouseenter":
      case "mouseleave":
      case "mouseover":
      case "mouseout":
        return withOffsets(
          new MouseEvent(name, {
            ...common,
            ...M,
            clientX: a,
            clientY: b,
            ...moved,
            button: buttonFor(name, c),
            buttons: buttonsFor(name, c),
            detail: detailFor(name, d),
          }),
          a,
          b,
        );
      case "load":
      case "selectstart":
      case "selectionchange":
        return new Event(name, common);
      case "focusin":
      case "focusout":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "wheel":
        if (typeof WheelEvent === "function")
          return new WheelEvent(name, { ...common, ...M, ...wheelAt(data), deltaX: a, deltaY: b });
        break;
      case "keydown":
      case "keyup":
        if (typeof KeyboardEvent === "function") return new KeyboardEvent(name, { ...common, ...M, key: key ?? "" });
        break;
      case "focus":
      case "blur":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "input":
      case "beforeinput":
        if (typeof InputEvent === "function") return new InputEvent(name, inputInit(name, a, b, data));
        break;
      case "compositionstart":
      case "compositionupdate":
      case "compositionend": {
        // Chromium makes all three cancelable; cancelling one does nothing.
        const Composition = theGlobal("CompositionEvent");
        if (typeof Composition === "function")
          return new (Composition as new (type: string, init: Record<string, unknown>) => Event)(name, {
            bubbles: true,
            cancelable: true,
            composed: true,
            data: typeof data === "string" ? data : "",
          });
        break;
      }
      case "scroll":
        if (typeof Event === "function") return new Event(name, common);
        break;
      case "transitionend":
        if (typeof TransitionEvent === "function")
          return new TransitionEvent(name, { ...common, propertyName: key ?? "", elapsedTime: a });
        break;
      case "animationstart":
      case "animationend":
      case "animationcancel":
        if (typeof AnimationEvent === "function")
          return new AnimationEvent(name, { ...common, animationName: key ?? "", elapsedTime: a });
        break;
      case "surfaceresize":
      case "surfaceready":
      case "surfacelost":
      case "surfacedisplaychange": {
        const detail = surfaceDetail(name, a, b, c, d, key);
        return withProps(new CustomEvent(name, { ...common, detail }), detail);
      }
      case "complete":
      case "loopComplete":
      case "enterFrame":
        // Both shapes are served (lottie-player CustomEvent, lottie-web plain object): numbers on `detail` and the event.
        return withProps(
          new CustomEvent(name, { ...common, detail: lottieDetail(name, a, b, c, d) }),
          lottieDetail(name, a, b, c, d),
        );
    }
    if (MEDIA_EVENTS.has(name)) {
      // Plain Events in a browser, read off `e.target`; the numbers also ride on the event and `detail` so a
      // handler needs no host call per frame.
      const detail = mediaDetail(a, b, c, d);
      return withProps(new CustomEvent(name, { ...common, detail }), detail);
    }
  } catch (e) {
    /* fall through to the generic event */
  }
  const ev = new CustomEvent(name, name === "input" || name === "beforeinput" ? inputInit(name, a, b, data) : common);
  const extra =
    name === "wheel"
      ? { deltaX: a, deltaY: b, ...wheelAt(data), ...M }
      : name === "keydown" || name === "keyup"
        ? { key: key ?? "", ...M }
        : name === "input" || name === "beforeinput"
          ? (({ data: d, inputType, isComposing }) => ({ data: d, inputType, isComposing }))(
              inputInit(name, a, b, data),
            )
          : name === "compositionstart" || name === "compositionupdate" || name === "compositionend"
            ? { data: typeof data === "string" ? data : "" }
            : name === "transitionend"
              ? { propertyName: key ?? "", elapsedTime: a, pseudoElement: "" }
              : name === "animationstart" || name === "animationend" || name === "animationcancel"
                ? { animationName: key ?? "", elapsedTime: a, pseudoElement: "" }
                : name === "focus" || name === "blur" || name === "focusin" || name === "focusout" || name === "scroll"
                  ? {}
                  : name === "complete" || name === "loopComplete" || name === "enterFrame"
                    ? lottieDetail(name, a, b, c, d)
                    : MEDIA_EVENTS.has(name) && !(tag === "img" && name === "error")
                      ? mediaDetail(a, b, c, d)
                      : name === "load" || name === "error"
                        ? {}
                        : POINTER_EVENTS.has(name)
                          ? {
                              clientX: a,
                              clientY: b,
                              ...moved,
                              button: buttonFor(name, c),
                              buttons: buttonsFor(name, c),
                              ...POINTER_INIT,
                              ...M,
                            }
                          : {
                              clientX: a,
                              clientY: b,
                              button: buttonFor(name, c),
                              buttons: buttonsFor(name, c),
                              detail: detailFor(name, d),
                              ...moved,
                              ...M,
                            };
  for (const k in extra) Object.defineProperty(ev, k, { value: (extra as ExtraProps)[k], configurable: true });
  return ev;
}

type ExtraProps = Record<string, unknown>;

export function withProps(ev: Event, props: Record<string, unknown>): Event {
  for (const k in props) Object.defineProperty(ev, k, { value: props[k], configurable: true });
  return ev;
}

// Media event payload: a = currentTime, b = duration, c = readyState, d = playbackRate. For `error`, a is
// MediaError.code and b is currentTime; `e.target.error` is served live off `__sk_media_state`.
function mediaDetail(a: number, b: number, c: number, d: number) {
  return { currentTime: a, duration: b, readyState: c, playbackRate: d };
}

// `<sk-surface>`: `surfaceresize` a × b physical px at scale c, `surfaceready` a × b the first frame, `surfacelost`
// `key` the reason, `surfacedisplaychange` the display (tree.rs `EV_SURFACEDISPLAYCHANGE`).
function surfaceDetail(name: string, a: number, b: number, c: number, d: number, key: string | undefined) {
  if (name === "surfacelost") return { reason: key ?? "" };
  if (name === "surfacedisplaychange") {
    const flags = (key ?? "").split(" ");
    return {
      headroom: a,
      potentialHeadroom: b,
      refreshRate: c > 0 ? c : null,
      sdrWhiteNits: d > 0 ? d : null,
      hdr: flags.includes("hdr"),
      outputHdr: flags.includes("output-hdr"),
      variableRefresh: flags.includes("vrr") ? true : flags.includes("fixed") ? false : null,
    };
  }
  if (name === "surfaceready") return { width: a, height: b };
  return { width: a, height: b, devicePixelRatio: c };
}

function lottieDetail(name: string, a: number, b: number, c: number, d: number) {
  if (name === "enterFrame") return { currentTime: a, totalTime: b, elapsed: c, progress: d };
  if (name === "loopComplete") return { currentTime: a, totalTime: b, currentLoop: c };
  return { currentTime: a, totalTime: b, loops: c };
}

type Phase = "capture" | "bubble";
interface PropagationState {
  stopped: boolean;
  immediate: boolean;
}

const EVENT_PHASE = { none: 0, capturing: 1, atTarget: 2, bubbling: 3 } as const;

/** HTML's "report an exception": the window's `error` event and `onerror`, then the console unless cancelled. */
export function reportException(e: unknown): void {
  const report = (globalThis as { reportError?: unknown }).reportError;
  if (typeof report === "function") return void (report as (e: unknown) => void)(e);
  (globalThis.queueMicrotask ?? queueMicrotask)(() => {
    throw e;
  });
}

/** One listener call. A throwing listener does not end the dispatch: the error is reported as uncaught and the next listener runs. */
export function callListener(
  fn: (this: unknown, ev: Event) => unknown,
  self: unknown,
  ev: Event,
  passive = false,
): void {
  // A passive listener's `preventDefault()` does nothing (`passiveOf`). An own
  // no-op shadows the prototype's for the length of the call.
  const own = passive ? Object.getOwnPropertyDescriptor(ev, "preventDefault") : undefined;
  if (passive) Object.defineProperty(ev, "preventDefault", { value: () => {}, configurable: true, writable: true });
  let traceToken = 0;
  try {
    traceToken = traceCallbackBegin?.(fn) ?? 0;
    fn.call(self, ev);
  } catch (e) {
    reportException(e);
  } finally {
    if (traceToken !== 0) traceCallbackEnd!(traceToken);
    if (passive) {
      if (own) Object.defineProperty(ev, "preventDefault", own);
      else delete (ev as { preventDefault?: unknown }).preventDefault;
    }
  }
}

function invoke(
  buckets: Buckets,
  phase: Phase,
  eventPhase: number,
  ev: Event,
  node: PropagationTarget,
  name: string,
  state: PropagationState,
): void {
  // The walk is ours, so the engine's `currentTarget` / `eventPhase` getters would say "no dispatch"; own
  // properties stand in.
  Object.defineProperty(ev, "currentTarget", { value: node, configurable: true });
  Object.defineProperty(ev, "eventPhase", { value: eventPhase, configurable: true });
  const fn = phase === "capture" ? buckets.capture : buckets.bubble;
  if (fn) {
    const passive = !!buckets[phase === "capture" ? "passiveC" : "passiveB"];
    if (buckets[phase === "capture" ? "onceC" : "onceB"]) {
      if (phase === "capture") buckets.capture = null;
      else buckets.bubble = null;
    }
    if (Array.isArray(fn)) {
      for (const f of fn) {
        if (state.immediate) break;
        callListener(f, undefined, ev, passive);
      }
    } else callListener(fn, undefined, ev, passive);
  }
  const extra = phase === "capture" ? buckets.extraC : buckets.extraB;
  for (const entry of [...extra]) {
    if (state.immediate) break;
    if (entry.removed) continue;
    if (entry.once) {
      const i = extra.indexOf(entry);
      if (i >= 0) extra.splice(i, 1);
      if (entry.signalCleanup) disposeGlobalListener(entry);
    }
    callListener(entry.fn, node, ev, !!entry.passive);
  }
  if (isMirrorNode(node)) syncListenerFlag(node, name);
  else if (appRoot) syncListenerFlag(appRoot, name);
}

export function propagate(target: MirrorNode, name: string, ev: Event): boolean {
  const path: PropagationTarget[] = [];
  for (let n: MirrorNode | null = target; n; n = n.parentNode) path.push(n);
  if (globalHandlers.size) {
    if (globalThis.document) path.push(globalThis.document);
    path.push(globalThis);
  }
  Object.defineProperty(ev, "target", { value: target, configurable: true });
  const state = { stopped: false, immediate: false };
  const origStop = ev.stopPropagation?.bind(ev);
  const origStopIm = ev.stopImmediatePropagation?.bind(ev);
  ev.stopPropagation = () => {
    state.stopped = true;
    origStop?.();
  };
  ev.stopImmediatePropagation = () => {
    state.stopped = true;
    state.immediate = true;
    origStopIm?.();
  };
  const at = (node: PropagationTarget, phase: Phase) => {
    const buckets = isMirrorNode(node) ? handlers.get(node)?.[name] : globalHandlers.get(node)?.[name];
    const eventPhase =
      node === target ? EVENT_PHASE.atTarget : phase === "capture" ? EVENT_PHASE.capturing : EVENT_PHASE.bubbling;
    if (buckets) invoke(buckets, phase, eventPhase, ev, node, name, state);
  };
  const composed = path.slice();
  Object.defineProperty(ev, "composedPath", { value: () => composed.slice(), configurable: true });
  try {
    for (let i = path.length - 1; i > 0 && !state.stopped; i--) at(path[i]!, "capture");
    if (!state.stopped) at(target, "capture");
    if (!state.stopped) at(target, "bubble");
    if (ev.bubbles) for (let i = 1; i < path.length && !state.stopped; i++) at(path[i]!, "bubble");
  } finally {
    Object.defineProperty(ev, "currentTarget", { value: null, configurable: true });
    Object.defineProperty(ev, "eventPhase", { value: EVENT_PHASE.none, configurable: true });
    Object.defineProperty(ev, "composedPath", { value: () => [], configurable: true });
  }
  return !ev.defaultPrevented;
}

/** A click on a checkbox or radio toggles it before the listeners run and is put back if one cancels it; otherwise `input` and `change` follow. */
export function propagateClick(target: MirrorNode, ev: Event): boolean {
  const token = isToggleInput(target) && isActivationEvent(ev) ? __sk_toggle_begin(target.__skId) : -1;
  const proceed = propagate(target, "click", ev);
  if (token !== -1 && __sk_toggle_end(target.__skId, token, !proceed)) {
    propagate(target, "input", fileEvent("input"));
    propagate(target, "change", fileEvent("change"));
  }
  return proceed;
}

/** An event targeted at the document (`selectionchange`): window capture, the document's own, then window bubble if it bubbles. */
export function propagateAtDocument(name: string, ev: Event): boolean {
  const document = globalThis.document as object | undefined;
  return document ? propagateAtGlobal(document, name, ev) : true;
}

export function propagateAtGlobal(target: object, name: string, ev: Event): boolean {
  const windowTarget = target === globalThis;
  Object.defineProperty(ev, "target", { value: target, configurable: true });
  const state = { stopped: false, immediate: false };
  const origStop = ev.stopPropagation?.bind(ev);
  const origStopIm = ev.stopImmediatePropagation?.bind(ev);
  ev.stopPropagation = () => {
    state.stopped = true;
    origStop?.();
  };
  ev.stopImmediatePropagation = () => {
    state.stopped = true;
    state.immediate = true;
    origStopIm?.();
  };
  const composed = windowTarget ? [globalThis] : [target, globalThis];
  Object.defineProperty(ev, "composedPath", { value: () => composed.slice(), configurable: true });
  const win = globalHandlers.get(globalThis)?.[name];
  const at = globalHandlers.get(target)?.[name];
  try {
    if (!windowTarget && win) invoke(win, "capture", EVENT_PHASE.capturing, ev, globalThis, name, state);
    if (!state.stopped) {
      if (at) invoke(at, "capture", EVENT_PHASE.atTarget, ev, target, name, state);
      if (at && !state.immediate) invoke(at, "bubble", EVENT_PHASE.atTarget, ev, target, name, state);
    }
    if (!windowTarget && ev.bubbles && win && !state.stopped)
      invoke(win, "bubble", EVENT_PHASE.bubbling, ev, globalThis, name, state);
  } finally {
    Object.defineProperty(ev, "currentTarget", { value: null, configurable: true });
    Object.defineProperty(ev, "eventPhase", { value: EVENT_PHASE.none, configurable: true });
    Object.defineProperty(ev, "composedPath", { value: () => [], configurable: true });
  }
  return !ev.defaultPrevented;
}

export const CODE_TO_NAME: Record<number, string | undefined> = Object.fromEntries(
  Object.entries(EV_CODES).map(([k, v]) => [v, k]),
);

/**
 * Is anything on this path going to be CALLED for `name`? Several names share one host code (a `pointermove` listener
 * arms `mousemove`), and building a redjs `new MouseEvent(...)` is most of dispatch cost, so skip it when nothing
 * listens. Ignores `bubbles`: an extra event is cheaper than a dropped handler.
 */
export function hasListener(target: MirrorNode, name: string): boolean {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    const buckets = handlers.get(n)?.[name];
    if (buckets && bucketsAlive(buckets)) return true;
  }
  for (const m of globalHandlers.values()) {
    const buckets = m[name];
    if (buckets && bucketsAlive(buckets)) return true;
  }
  return false;
}

export const ACTIVATING_EVENTS = new Set([
  "keydown",
  "mousedown",
  "pointerdown",
  "mouseup",
  "pointerup",
  "click",
  "auxclick",
]);

export function activatableOnPath(target: MirrorNode): boolean {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (
      isLabel(n) ||
      isFileInput(n) ||
      isColorInput(n) ||
      isToggleInput(n) ||
      isLink(n) ||
      isSubmitButton(n) ||
      isResetButton(n)
    )
      return true;
  }
  return false;
}
