import type {
  Buckets,
  EventHandler,
  ExtraListener,
  HandlerMap,
  ListenerOptions,
  ListenerSignal,
  MirrorElement,
  MirrorNode,
} from "./types";
import { interceptedGlobals } from "./state";
import { EV_CODES, NAMES_BY_CODE, canonicalEvent, hostCode } from "./event-names";
import { handlers, nodeById } from "./nodes";
import { warnCompat } from "../compat";
import { appRoot } from "../runtime";
import { propagateAtDocument, propagateAtGlobal } from "./events";
import { RUNTIME_FIRED_EVENTS, blocksImplicitSubmission, isResetButton, isSubmitButton } from "./forms";
import { adviseJsSourced } from "./labels";
import { isColorInput, isFileInput, isLabel } from "./file-input";
import { isLink } from "./links";
import type { DndGesture } from "./dnd";

export function ensureBuckets(m: HandlerMap, name: string): Buckets {
  let buckets = m[name];
  if (!buckets)
    buckets = m[name] = {
      bubble: null,
      capture: null,
      onceB: false,
      onceC: false,
      extraB: [],
      extraC: [],
    };
  return buckets;
}

export function bucketsAlive(b: Buckets): boolean {
  return !!(b.bubble || b.capture || b.extraB.length || b.extraC.length);
}

// HTML drag and drop within the app: state here, the gesture by the dispatcher. Rides the app
// root's listener bits: a press once anything is `draggable="true"`, then moves/release/Escape
// while a press is pending or a drag is under way.
/** The drag-and-drop gesture in progress, and what the next event needs to know about the last one. */
export const dndState: { seen: boolean; gesture: DndGesture | null; swallowClick: boolean } = {
  seen: false,
  gesture: null,
  swallowClick: false,
};

function dndWants(code: number): boolean {
  if (code === EV_CODES.mousedown || code === EV_CODES.selectstart) return dndState.seen;
  if (code === EV_CODES.mousemove || code === EV_CODES.mouseup) return !!dndState.gesture;
  if (code === EV_CODES.keydown) return dndState.gesture?.phase === "dragging";
  return false;
}

export function dndRearm(): void {
  if (!appRoot) return;
  for (const name of ["mousedown", "selectstart", "mousemove", "mouseup", "keydown"]) syncListenerFlag(appRoot, name);
}

// Several event names share one host code (`pointerdown` rides `mousedown`): the flag is the OR over all of them.
export function syncListenerFlag(self: MirrorNode, name: string): void {
  const code = hostCode(name);
  if (!code) return;
  const id = self.__skId;
  const m = handlers.get(self);
  let on = NAMES_BY_CODE[code]!.some((n) => m?.[n] && bucketsAlive(m[n]!));
  if (!on && appRoot && id === appRoot.__skId) {
    on = NAMES_BY_CODE[code]!.some((n) => [...globalHandlers.values()].some((g) => g[n] && bucketsAlive(g[n]!)));
    if (!on && dndWants(code)) on = true;
  }
  // Activation behaviour: a link, `<label>`, file/color input and submit/reset button listen for their own click;
  // file/color inputs also hear Enter/Space, and a text-like `<input>` submits on Enter.
  if (!on && code === EV_CODES.click) {
    const n = self;
    if (n && (isLabel(n) || isFileInput(n) || isColorInput(n) || isLink(n) || isSubmitButton(n) || isResetButton(n)))
      on = true;
  }
  if (!on && (code === EV_CODES.keydown || code === EV_CODES.keyup)) {
    const n = self;
    if (isFileInput(n) || isColorInput(n) || (code === EV_CODES.keydown && blocksImplicitSubmission(n))) on = true;
  }
  // A text control hears its own `input`: the dispatcher writes `value` before propagating.
  if (!on && code === EV_CODES.input) {
    const n = self;
    if (n && n.kind === 1 && (n.tag === "input" || n.tag === "textarea")) on = true;
  }
  // Only tell the host when the answer changed (`set_listener` is idempotent; an `on*` re-patch would cross the
  // boundary per element). Test stand-ins carry no cache.
  const node = isMirrorNode(self) ? self : null;
  if (node) {
    const known = node._listenerOn;
    if (on) {
      if (known?.has(code)) return;
      (node._listenerOn ??= new Set()).add(code);
    } else {
      if (!known?.has(code)) return;
      known.delete(code);
    }
  }
  __sk_set_listener(id, code, on);
}

// Document / window listeners: not mirror nodes, so they have their own registry, joined onto the propagation walk
// one level above the app root. Without it, outside-click and Escape dismissal never fire.
export const globalHandlers = new Map<object, HandlerMap>(); // target object -> { [event name]: buckets }
export const isMirrorNode = (n: unknown): n is MirrorNode =>
  !!n && typeof (n as MirrorNode).__skId === "number" && nodeById((n as MirrorNode).__skId) === (n as MirrorNode);

type ListenerOptionsObject = { capture?: boolean; once?: boolean; passive?: boolean; signal?: ListenerSignal };

const PASSIVE_BY_DEFAULT = new Set(["wheel", "mousewheel", "touchstart", "touchmove"]);

/** Is this listener passive? An explicit `passive` decides; else `wheel`, `mousewheel`, `touchstart`, `touchmove` on `window`, `document` or `<body>` are, as the DOM's default says. */
export function passiveOf(opts: ListenerOptions | undefined, target: object, name: string): boolean {
  if (opts && typeof opts === "object" && typeof (opts as ListenerOptionsObject).passive === "boolean")
    return (opts as ListenerOptionsObject).passive!;
  if (!PASSIVE_BY_DEFAULT.has(name)) return false;
  return target === globalThis || target === globalThis.document || (appRoot != null && target === appRoot);
}

// `message`: the host delivers an `<iframe>` document's; the engine fires its own (`postMessage`).
const ALSO_NATIVE = new Set<string>(["load", "error", "message"]);

/** Does a `window`/`document` listener for this name live only in the runtime's registry (no engine EventTarget copy)? Then `dispatchEvent` there must walk the registry too. */
function keptByRuntime(name: string): boolean {
  if (ALSO_NATIVE.has(name)) return false;
  return !!hostCode(name) || JS_SOURCED_EVENTS.has(name) || RUNTIME_FIRED_EVENTS.has(name);
}

function addGlobalListener(
  target: object,
  rawName: string,
  fn: EventHandler | null | undefined,
  opts: ListenerOptions,
): boolean {
  if (!fn) return false;
  const name = canonicalEvent(rawName);
  // A file input's `change`/`cancel` bubble to the document, so a delegated listener takes the same walk.
  if (!hostCode(name) && !JS_SOURCED_EVENTS.has(name) && !RUNTIME_FIRED_EVENTS.has(name)) return false;
  const signal = !ALSO_NATIVE.has(name) && opts && typeof opts === "object" ? opts.signal : undefined;
  if (signal?.aborted) return true;
  let m = globalHandlers.get(target);
  if (!m) globalHandlers.set(target, (m = {}));
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const entries = capture ? buckets.extraC : buckets.extraB;
  const entry: ExtraListener = {
    fn,
    once: !!(opts && (opts as ListenerOptionsObject).once),
    passive: passiveOf(opts, target, name),
  };
  entries.push(entry);
  if (signal) {
    const abort = () => {
      const index = entries.indexOf(entry);
      if (index >= 0) entries.splice(index, 1);
      disposeGlobalListener(entry);
      if (appRoot) syncListenerFlag(appRoot, name);
    };
    entry.signalCleanup = () => signal.removeEventListener("abort", abort);
    signal.addEventListener("abort", abort, { once: true });
  }
  if (appRoot) syncListenerFlag(appRoot, name);
  // `load` and `error` are also the window's own events, so the listener goes to both the registry
  // and the engine's EventTarget.
  return !ALSO_NATIVE.has(name);
}

export function disposeGlobalListener(entry: ExtraListener): void {
  entry.removed = true;
  const cleanup = entry.signalCleanup;
  entry.signalCleanup = undefined;
  cleanup?.();
}

function removeGlobalListener(target: object, rawName: string, fn: EventHandler, opts: ListenerOptions): boolean {
  const name = canonicalEvent(rawName);
  const buckets = globalHandlers.get(target)?.[name];
  if (!buckets) return false;
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) {
    disposeGlobalListener(arr[i]!);
    arr.splice(i, 1);
  }
  if (appRoot) syncListenerFlag(appRoot, name);
  return !ALSO_NATIVE.has(name);
}

type GlobalTargetLike = {
  addEventListener?: (name: string, fn: EventHandler, opts?: ListenerOptions) => void;
  removeEventListener?: (name: string, fn: EventHandler, opts?: ListenerOptions) => void;
  dispatchEvent?: (event: Event) => boolean;
};

export type GlobalEventMethods = {
  add: GlobalTargetLike["addEventListener"];
  remove: GlobalTargetLike["removeEventListener"];
  dispatch: GlobalTargetLike["dispatchEvent"];
};
export type GlobalEventSlot = {
  native: GlobalEventMethods;
  set(delegate: GlobalEventMethods): void;
  clear(delegate: GlobalEventMethods): void;
};
function globalEventBridge(): { get(target: object): GlobalEventSlot | undefined } | undefined {
  return (globalThis as { __SK_GLOBAL_EVENTS__?: { get(target: object): GlobalEventSlot | undefined } })
    .__SK_GLOBAL_EVENTS__;
}

/** The target's own listener and dispatch methods as the FIRST runtime found them, so a reload wraps the same function, not the previous wrapper. */
export function nativeListenFns(target: object): {
  add: GlobalTargetLike["addEventListener"];
  remove: GlobalTargetLike["removeEventListener"];
  dispatch: GlobalTargetLike["dispatchEvent"];
} {
  const slot = globalEventBridge()?.get(target);
  if (slot) return slot.native;
  const g = globalThis as { __SK_NATIVE_LISTEN__?: WeakMap<object, ReturnType<typeof nativeListenFns>> };
  const table = (g.__SK_NATIVE_LISTEN__ ??= new WeakMap());
  let fns = table.get(target);
  const t = target as GlobalTargetLike;
  if (!fns) {
    fns = {
      add: typeof t.addEventListener === "function" ? t.addEventListener.bind(target) : undefined,
      remove: typeof t.removeEventListener === "function" ? t.removeEventListener.bind(target) : undefined,
      dispatch: typeof t.dispatchEvent === "function" ? t.dispatchEvent.bind(target) : undefined,
    };
    table.set(target, fns);
  } else if (!Object.prototype.hasOwnProperty.call(fns, "dispatch")) {
    fns.dispatch = typeof t.dispatchEvent === "function" ? t.dispatchEvent.bind(target) : undefined;
  }
  return fns;
}

export const passedThrough: { target: object; name: string; fn: EventHandler; opts: ListenerOptions }[] = [];

export function interceptGlobalTarget(target: object | undefined): void {
  if (!target) return;
  const bridge = globalEventBridge();
  const slot = bridge?.get(target);
  if (bridge && !slot) throw new TypeError("The host global-events bridge has no slot for this target");
  const { add: nativeAdd, remove: nativeRemove, dispatch: nativeDispatch } = nativeListenFns(target);
  const delegate: GlobalEventMethods = {
    dispatch: (event: Event) => {
      const name = event && typeof event.type === "string" ? canonicalEvent(event.type) : "";
      if (keptByRuntime(name)) {
        if (target === globalThis.document) return propagateAtDocument(name, event);
        if (target === globalThis) return propagateAtGlobal(target, name, event);
      }
      return nativeDispatch?.(event) ?? true;
    },
    add: (name: string, fn: EventHandler, opts: ListenerOptions) => {
      if (addGlobalListener(target, name, fn, opts)) return;
      nativeAdd?.(name, fn, opts);
      passedThrough.push({ target, name, fn, opts });
    },
    remove: (name: string, fn: EventHandler, opts: ListenerOptions) => {
      if (removeGlobalListener(target, name, fn, opts)) return;
      nativeRemove?.(name, fn, opts);
      const i = passedThrough.findIndex((p) => p.target === target && p.name === name && p.fn === fn);
      if (i >= 0) passedThrough.splice(i, 1);
    },
  };
  if (slot) {
    slot.set(delegate);
    interceptedGlobals.push({ slot, delegate });
    return;
  }
  // Older hosts have no bootstrap bridge; native AOT never redefines its sealed public methods.
  try {
    for (const [name, value] of [
      ["dispatchEvent", delegate.dispatch],
      ["addEventListener", delegate.add],
      ["removeEventListener", delegate.remove],
    ] as const) {
      Object.defineProperty(target, name, { value, configurable: true, writable: true });
    }
  } catch (e) {
    warnCompat(
      "global-listeners",
      "document/window event listeners could not be intercepted on this engine — a component " +
        `that closes itself on an outside click or Escape will stay open (${e})`,
    );
  }
}

// Does this name look like an event the platform sources? Only those get an advisory when the host cannot deliver
// them; a name with a dot or a capital (`"select.select"`) is a component's own event.
const looksNative = (name: string): boolean => /^[a-z]+$/.test(name);

/** Platform events the runtime fires itself (a file input's `change`/`cancel`, pointer capture): listeners are kept, no advisory. */
export const JS_SOURCED_EVENTS = new Set<string>(["change", "cancel", "gotpointercapture", "lostpointercapture"]);

// Events a browser fires only on `window`/`document`: no advisory for an element listener (dead code in Chrome too).
const WINDOW_ONLY_EVENTS = new Set<string>([
  "resize",
  "load",
  "unload",
  "beforeunload",
  "pageshow",
  "pagehide",
  "hashchange",
  "popstate",
  "online",
  "offline",
  "storage",
  "message",
  "languagechange",
  "rejectionhandled",
  "unhandledrejection",
]);

export function addExtraListener(
  node: MirrorElement,
  rawName: string,
  fn: EventHandler | null | undefined,
  opts: ListenerOptions,
): void {
  if (!fn) return;
  const name = canonicalEvent(rawName);
  const code = hostCode(name);
  if (!code && WINDOW_ONLY_EVENTS.has(name)) return;
  if (name === "selectionchange") {
    warnCompat(
      "event:selectionchange:element",
      `addEventListener("${rawName}") on an element — Skathi fires selectionchange at document only ` +
        "(a text field's own selection does not fire it); listen on document",
    );
  }
  if (!code && RUNTIME_FIRED_EVENTS.has(name)) {
  } else if (!code && JS_SOURCED_EVENTS.has(name)) {
    adviseJsSourced(node, name, `addEventListener("${rawName}")`);
  } else if (!code && looksNative(name)) {
    warnCompat(
      `event:${name}`,
      `addEventListener("${rawName}") — this event type is not delivered by the native host yet; ` +
        `the listener will never fire`,
    );
    return;
  }
  let m = handlers.get(node);
  if (!m) {
    m = {};
    handlers.set(node, m);
  }
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const entry = {
    fn,
    once: !!(opts && (opts as ListenerOptionsObject).once),
    passive: passiveOf(opts, node, name),
  };
  (capture ? buckets.extraC : buckets.extraB).push(entry);
  syncListenerFlag(node, name);
}

export function removeExtraListener(
  node: MirrorElement,
  rawName: string,
  fn: EventHandler,
  opts: ListenerOptions,
): void {
  const name = canonicalEvent(rawName);
  const buckets = handlers.get(node)?.[name];
  if (!buckets) return;
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  syncListenerFlag(node, name);
}
