// vue-native JS runtime: a @vue/runtime-core custom renderer over the __vn_* host protocol.
// Apps import `createApp` from "vue-native"; compiled SFCs import their helpers from "vue".

import {
  BaseTransition,
  BaseTransitionPropsValidators,
  createRenderer,
  createVNode,
  defineComponent,
  Fragment,
  getCurrentInstance,
  getTransitionRawChildren,
  h,
  nextTick,
  onBeforeUpdate,
  onMounted,
  onUnmounted,
  queuePostFlushCb,
  resolveTransitionHooks,
  setTransitionHooks,
  Static,
  Teleport,
  toRaw,
  useTransitionState,
  version,
  watch,
} from "@vue/runtime-core";
import type { App, ComponentPublicInstance, RendererOptions, VNode } from "@vue/runtime-core";
import { installDesktop } from "./desktop";
import { installBadge } from "./badge";
import { installCredentials } from "./credentials";
import { installMediaSession } from "./mediasession";
import { installShare } from "./share";
import { installWakeLock } from "./wakelock";
import type { WakeLockDeps } from "./wakelock";
import "./apphost";
import type { DesktopApi, DesktopEngine, DesktopFsHost, DesktopSettle, HandleWire } from "./desktop";
import { adoptCanvas } from "./canvas";
import { decodeMessage, encodeMessage } from "./iframe";
import { installFonts } from "./fonts";
import { installSelection } from "./selection";
import { installWaapi } from "./waapi";
import { VnWindow, WINDOW_ROOT, windowRoots } from "./vnwindow";

// Cached before user code can replace the globals. A zero token is the inactive path.
const traceCallbackGlobals = globalThis as unknown as {
  __vn_trace_callback_begin?: (callback: unknown) => number;
  __vn_trace_callback_end?: (token: number) => void;
};
const traceCallbackBegin =
  typeof traceCallbackGlobals.__vn_trace_callback_begin === "function" &&
  typeof traceCallbackGlobals.__vn_trace_callback_end === "function"
    ? traceCallbackGlobals.__vn_trace_callback_begin
    : undefined;
const traceCallbackEnd = traceCallbackBegin ? traceCallbackGlobals.__vn_trace_callback_end : undefined;

// `location.reload()` and the dev host's cold restart re-evaluate this module in the SAME realm.
// REALM.installed: what a runtime installed on engine-owned objects. `vacant()` counts it as a hole, so the
//   next runtime installs its OWN class over it (a kept class runs against the old runtime's state).
interface RealmRecord {
  base: Set<PropertyKey>;
  installed: WeakSet<object>;
  runs: number;
  originals?: WeakMap<object, Map<PropertyKey, unknown>>;
}
const REALM: RealmRecord = ((globalThis as { __VN_REALM__?: RealmRecord }).__VN_REALM__ ??= {
  base: new Set(Reflect.ownKeys(globalThis)),
  installed: new WeakSet(),
  runs: 0,
});
REALM.runs++;

function mark<T>(v: T): T {
  if ((typeof v === "object" && v !== null) || typeof v === "function") REALM.installed.add(v as object);
  return v;
}

/** `obj[name]` as the FIRST runtime in this realm found it: after a reload, wrap the original, not the old wrapper. */
function original<T>(obj: object, name: PropertyKey, now: T): T {
  const table = (REALM.originals ??= new WeakMap());
  let byName = table.get(obj);
  if (!byName) table.set(obj, (byName = new Map()));
  if (!byName.has(name)) byName.set(name, now);
  return byName.get(name) as T;
}

/** Is `obj[name]` missing or a previous runtime's install? Walks the prototype chain by descriptor, so no getter runs. */
function vacant(obj: object, name: PropertyKey): boolean {
  for (let o: object | null = obj; o; o = Object.getPrototypeOf(o)) {
    const d = Object.getOwnPropertyDescriptor(o, name);
    if (!d) continue;
    const v = d.get ?? d.value;
    if (v === undefined) return true;
    return (typeof v === "object" && v !== null) || typeof v === "function" ? REALM.installed.has(v) : false;
  }
  return true;
}

/** Deletes configurable globals added since the first runtime, except `__vn*`. Returns how many could not be deleted. */
function resetGlobals(): number {
  let kept = 0;
  for (const k of Reflect.ownKeys(globalThis)) {
    if (REALM.base.has(k) || (typeof k === "string" && /^__vn/i.test(k))) continue;
    const d = Object.getOwnPropertyDescriptor(globalThis, k);
    if (!d) continue;
    if (!d.configurable) {
      kept++;
      continue;
    }
    try {
      delete (globalThis as Record<PropertyKey, unknown>)[k];
    } catch {
      kept++;
    }
  }
  return kept;
}

// Members not listed here do not exist on a mirror node: reused browser code gets the TypeError an engine without them would give.

/** 1 element, 2 text, 3 comment. The mirror's own tag — not `nodeType`. */
type MirrorKind = 1 | 2 | 3;

type EventHandler = (ev: VnEvent) => void;
/** …and what Vue passes for `@click="[a, b]"` — several, in order. */
type EventHandlerOrList = EventHandler | EventHandler[];
type ListenerSignal = {
  readonly aborted: boolean;
  addEventListener(type: string, listener: () => void, options?: { once?: boolean }): void;
  removeEventListener(type: string, listener: () => void): void;
};
type ListenerOptions =
  boolean | { capture?: boolean; once?: boolean; passive?: boolean; signal?: ListenerSignal } | undefined;

interface ExtraListener {
  fn: EventHandler;
  once: boolean;
  passive?: boolean;
  signalCleanup?: (() => void) | undefined;
  removed?: boolean;
}

/**
 * Listeners for one event name on one target. `bubble`/`capture` are the RENDERER's single slots (Vue
 * re-patches a prop); `extraB`/`extraC` are `addEventListener`'s and accumulate.
 */
interface Buckets {
  bubble: EventHandlerOrList | null;
  capture: EventHandlerOrList | null;
  onceB: boolean;
  onceC: boolean;
  passiveB?: boolean;
  passiveC?: boolean;
  extraB: ExtraListener[];
  extraC: ExtraListener[];
}

type HandlerMap = Record<string, Buckets | undefined>;

interface MirrorRect {
  x: number;
  y: number;
  top: number;
  left: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
  toJSON?(): MirrorRect;
}

interface MirrorCollection<T> extends ArrayLike<T>, Iterable<T> {
  item(index: number): T | null;
}

interface MirrorClassList {
  add(...names: string[]): void;
  remove(...names: string[]): void;
  toggle(name: string, force?: boolean): boolean;
  contains(name: string): boolean;
  readonly length: number;
  item(i: number): string | null;
  toString(): string;
  [Symbol.iterator](): IterableIterator<string>;
}

/** `el.style`: a Proxy. Unnamed property writes go straight to `__vn_set_style`, custom properties included (reka-ui's `--reka-popper-*`). */
interface MirrorStyle {
  setProperty(key: string, value: string | number | null | undefined, priority?: string): void;
  removeProperty(key: string): string;
  getPropertyValue(key: string): string;
  getPropertyPriority(key: string): string;
  item(i: number): string;
  readonly length: number;
  cssText: string;
  [property: string]: unknown;
}

interface InlineDecl {
  value: string;
  important: boolean;
}

/** The declaration block behind `el.style` AND the `style` attribute: one object, so every spelling reads and writes the same declarations. */
interface InlineStyleState {
  decls: Map<string, InlineDecl>;
  present: boolean;
  /** The text `setAttribute("style", …)` wrote, verbatim, until a CSSOM change re-serializes it. */
  raw: string | null;
  /** runtime-dom's `CSS_VAR_TEXT`: what `useCssVars` set, kept across a string `:style`. */
  cssVarText: string;
}

interface MirrorNodeBase {
  /**
   * The host node id (`VnNodeId`); `el.id` is the `id` ATTRIBUTE. 0 once the node is freed (`forgetDeep`):
   * host ids are recycled, so a stale reference must not name the next node.
   */
  __vnId: VnNodeId;
  kind: MirrorKind;
  tag: string;
  parentNode: MirrorElement | null;
  childNodes: MirrorNode[];
  text?: string;
  contains(other: MirrorNode | null | undefined): boolean;
  readonly ownerDocument: VnDocument | undefined;
  readonly nodeType: 1 | 3 | 8 | 11;
  readonly nodeName: string;
  readonly isConnected: boolean;
  readonly parentElement: MirrorElement | null;
  readonly firstChild: MirrorNode | null;
  readonly lastChild: MirrorNode | null;
  readonly nextSibling: MirrorNode | null;
  readonly previousSibling: MirrorNode | null;
  readonly nextElementSibling: MirrorElement | null;
  readonly previousElementSibling: MirrorElement | null;
  hasChildNodes(): boolean;
  getRootNode(): unknown;
  dispatchEvent(ev: VnEvent): boolean;
  /**
   * Event codes whose `__vn_set_listener` bit is ON in the host (see `syncListenerFlag`). Kept on the node,
   * not in a map by id: `__vn_remove` recycles ids, so a map would answer for a slot's previous occupant.
   */
  _listenerOn?: Set<VnEventCode>;
  /** Vue's symbol-keyed bookkeeping lands here; `__v_skip` on the prototype stops `reactive()` proxying the node. */
  [key: symbol]: unknown;
}

interface MirrorText extends MirrorNodeBase {
  kind: 2 | 3;
  data: string;
  nodeValue: string;
  textContent: string;
}

interface MirrorElement extends MirrorNodeBase {
  kind: 1;
  tagName: string;
  value: string;
  _value: string;
  _checked?: boolean;
  /** Range: IDL/user edits stop value/defaultValue attributes replacing the live value. */
  _rangeDirty?: boolean;
  _trueValue?: unknown;
  _falseValue?: unknown;
  composing?: boolean;
  _selected?: boolean;
  _dirty?: boolean;
  _rawValue?: unknown;
  _shown?: number;
  _modelValue?: unknown;
  _assigning?: boolean;
  _img?: { src: string; ok: boolean; w: number; h: number };
  _on?: Record<string, { fn: unknown; listener: (ev: VnEvent) => void }>;
  attrs: Record<string, string>;
  className: string;
  /** The class Vue or a script assigned, kept apart from `<Transition>`'s `_vtc`. */
  _class: string;
  ownText?: string;
  addEventListener(name: string, fn: EventHandler, opts?: ListenerOptions): void;
  removeEventListener(name: string, fn: EventHandler, opts?: ListenerOptions): void;

  _ns?: "svg" | "mathml";

  readonly children: MirrorCollection<MirrorElement>;
  readonly firstElementChild: MirrorElement | null;
  readonly lastElementChild: MirrorElement | null;
  readonly childElementCount: number;
  textContent: string;

  matches(selector: string): boolean;
  closest(selector: string): MirrorElement | null;
  querySelector(selector: string): MirrorElement | null;
  querySelectorAll(selector: string): MirrorCollection<MirrorElement>;

  getBoundingClientRect(): MirrorRect;
  getClientRects(): MirrorCollection<MirrorRect>;
  readonly offsetWidth: number;
  readonly offsetHeight: number;
  readonly clientWidth: number;
  readonly clientHeight: number;
  readonly offsetTop: number;
  readonly offsetLeft: number;
  readonly offsetParent: null;
  readonly clientTop: number;
  readonly clientLeft: number;

  /** The CONTENT box of a scroll container; the border box of anything else. */
  readonly scrollWidth: number;
  readonly scrollHeight: number;
  /** Read and WRITTEN: assigning clamps and is visible to the getter at once. */
  scrollLeft: number;
  scrollTop: number;
  scrollTo(x?: unknown, y?: unknown): void;
  scroll(x?: unknown, y?: unknown): void;
  scrollBy(x?: unknown, y?: unknown): void;
  scrollIntoView(arg?: unknown): void;

  getAttribute(name: string): string | null;
  hasAttribute(name: string): boolean;
  getAttributeNames(): string[];
  setAttribute(name: string, value: unknown): void;
  removeAttribute(name: string): void;
  toggleAttribute(name: string, force?: boolean): boolean;
  readonly dataset: Record<string, string | undefined>;
  readonly classList: MirrorClassList;
  readonly style: MirrorStyle;

  click(): void;
  showPicker(): void;

  focus(): void;
  blur(): void;
  select(): void;
  setPointerCapture(pointerId: number): void;
  hasPointerCapture(pointerId: number): boolean;
  releasePointerCapture(pointerId: number): void;

  // The retained tree has no copy and no insert from outside the renderer.
  insertAdjacentElement(where: string, el: MirrorElement): MirrorElement;
  append(...nodes: unknown[]): void;
  remove(): void;
  cloneNode(): null;

  currentTime: number;
  readonly duration: number;
  readonly paused: boolean;
  readonly ended: boolean;
  readonly seeking: boolean;
  readonly readyState: number;
  readonly networkState: number;
  volume: number;
  muted: boolean;
  playbackRate: number;
  loop: boolean;
  readonly videoWidth: number;
  readonly videoHeight: number;
  srcObject: unknown;
  readonly buffered: MirrorTimeRanges;
  readonly seekable: MirrorTimeRanges;
  readonly error: { code: number; message: string } | null;
  play(): Promise<void>;
  pause(): void;
  load(): void;
  canPlayType(type: string): string;

  _inline?: InlineStyleState;
  _vtc?: Set<string> | undefined;
  _isLeaving?: boolean;
  _endId?: number;
}

type MirrorNode = MirrorElement | MirrorText;

type MirrorNodeInit = Omit<MirrorElement, "kind"> & Omit<MirrorText, "kind"> & { kind: MirrorKind };

interface MirrorTimeRanges {
  length: number;
  start(i: number): number;
  end(i: number): number;
  ranges: { start: number; end: number }[];
}

type PropagationTarget = MirrorNode | object;

// Names cross the host boundary once; ids after.
const interned = new Map<string, VnNameId>();
const I = (s: string): VnNameId => {
  let v = interned.get(s);
  if (v === undefined) {
    v = __vn_intern(s);
    interned.set(s, v);
  }
  return v;
};

// Keep in lockstep with EV_* in vn-tree's tree.rs.
const EV_CODES = {
  click: 1,
  mousedown: 2,
  mouseup: 3,
  wheel: 4,
  keydown: 5,
  keyup: 6,
  focus: 7,
  blur: 8,
  input: 9,
  scroll: 10,
  complete: 11,
  loopComplete: 12,
  enterFrame: 13,
  // Pointer family: the host sources only mousemove/mouseenter/mouseleave; the rest of the W3C set is
  // DERIVED from them and mousedown/mouseup (`DERIVED`).
  mousemove: 14,
  mouseenter: 15,
  mouseleave: 16,
  loadstart: 17,
  loadedmetadata: 18,
  loadeddata: 19,
  canplay: 20,
  canplaythrough: 21,
  play: 22,
  playing: 23,
  pause: 24,
  seeking: 25,
  seeked: 26,
  // The host throttles it to the spec's 250 ms (MF raises it at the display's rate).
  timeupdate: 27,
  durationchange: 28,
  ratechange: 29,
  volumechange: 30,
  waiting: 31,
  ended: 32,
  // `a` is MediaError.code (1-4), `key` the advisory naming the missing codec.
  error: 33,
  emptied: 34,
  // `a` = elapsedTime, `key` = the property / @keyframes name.
  transitionend: 35,
  animationstart: 36,
  animationend: 37,
  animationcancel: 38,
  paste: 39,
  copy: 40,
  cut: 41,
  // `contextmenu`: right-button release, or Menu key / Shift+F10. `load`: an `<img>` has its picture (a/b = natural
  // size, `key` = src; a failure is `error` with the same `key`). `pointercancel`: the host gave up on a press.
  contextmenu: 42,
  load: 43,
  pointercancel: 44,
  // Document text selection. `selectstart` is cancelable, at the press target. `selectionchange` arrives at the
  // ROOT and is retargeted to `document`, where a browser fires it.
  selectstart: 45,
  selectionchange: 46,
  auxclick: 47,
  // Text-field edits: `beforeinput` precedes an edit and cancels it (an IME's excepted); `a` is the `EDIT_TYPES` code,
  // `b` 1 while an IME composes, the eighth argument the `data` string. `input` (9) adds the new value in `key`.
  beforeinput: 48,
  compositionstart: 49,
  compositionupdate: 50,
  compositionend: 51,
  // An `<iframe>` document's postMessage, delivered at the iframe node and retargeted to `window`. `key` is
  // `{"o": origin, "d": wire}`, the payload in `iframe.ts`'s wire form.
  message: 52,
} as const;

/** `InputEvent.inputType` by host code: indexes vn-elements' `text_input::InputType` (lockstep, test/protocol-codes.test.ts). 0: none. */
const EDIT_TYPES = [
  "",
  "insertText",
  "insertLineBreak",
  "insertFromPaste",
  "insertCompositionText",
  "deleteContentBackward",
  "deleteContentForward",
  "deleteWordBackward",
  "deleteWordForward",
  "deleteSoftLineBackward",
  "deleteSoftLineForward",
  "deleteByCut",
  "historyUndo",
  "historyRedo",
] as const;

type HostEventName = keyof typeof EV_CODES;
// Events synthesized in JS from one the host delivers. `pre: true` fires BEFORE its source (W3C pointer order:
// `pointerdown` precedes `mousedown`); `focusin`/`focusout` bubble and fire after `focus`/`blur`.
// A `pointerdown` listener registers the host's `mousedown` bit and receives the mouse event's payload.
const DERIVED = {
  pointerdown: { from: "mousedown", pre: true },
  pointerup: { from: "mouseup", pre: true },
  pointermove: { from: "mousemove", pre: true },
  pointerenter: { from: "mouseenter", pre: true },
  pointerleave: { from: "mouseleave", pre: true },
  focusin: { from: "focus", pre: false },
  focusout: { from: "blur", pre: false },
  // After the click with `detail` 2 and the primary button: only the second click of a pair makes one.
  dblclick: { from: "click", pre: false },
} as const satisfies Record<string, { from: HostEventName; pre: boolean }>;

type DerivedEventName = keyof typeof DERIVED;

const hostCode = (name: string): VnEventCode | undefined =>
  (EV_CODES as Record<string, VnEventCode | undefined>)[name] ??
  (EV_CODES as Record<string, VnEventCode | undefined>)[
    (DERIVED as Record<string, { from: HostEventName } | undefined>)[name]?.from as string
  ];
const NAMES_BY_CODE: Record<number, string[]> = (() => {
  const out: Record<number, string[]> = {};
  const add = (code: VnEventCode, name: string) => (out[code] ??= []).push(name);
  for (const name in EV_CODES) add(EV_CODES[name as HostEventName], name);
  for (const name in DERIVED) add(EV_CODES[DERIVED[name as DerivedEventName].from], name);
  return out;
})();
const DERIVED_OF: Record<string, string[] | undefined> = (() => {
  const out: Record<string, string[]> = {};
  for (const name in DERIVED) (out[DERIVED[name as DerivedEventName].from] ??= []).push(name);
  return out;
})();
const POINTER_EVENTS = new Set<string>([
  "pointerdown",
  "pointerup",
  "pointermove",
  "pointerenter",
  "pointerleave",
  "pointercancel",
  "gotpointercapture",
  "lostpointercapture",
]);
const NOT_CANCELABLE = new Set<string>(["pointercancel", "gotpointercapture", "lostpointercapture", "select"]);
// Vue hyphenates an `@camelCase` listener prop (`onLoopComplete` -> "loop-complete"); accept both spellings and
// fold them onto the camelCase name.
const EVENT_ALIASES: Record<string, string | undefined> = {
  "loop-complete": "loopComplete",
  loopcomplete: "loopComplete",
  "enter-frame": "enterFrame",
  enterframe: "enterFrame",
};
const canonicalEvent = (name: string): string => EVENT_ALIASES[name] ?? name;
// These fire at the target only (`scroll` bubbles from the document, never from an element).
const NO_BUBBLE = new Set<string>([
  "focus",
  "blur",
  "scroll",
  "selectionchange",
  "complete",
  "loopComplete",
  "enterFrame",
  // enter/leave do not bubble; getting that wrong closes a menu as soon as the pointer crosses one of its items.
  "mouseenter",
  "mouseleave",
  "pointerenter",
  "pointerleave",
  // Media events fire at the target only; else a wrapper's `@error` would catch a `<video>`'s failed load.
  "loadstart",
  "loadedmetadata",
  "loadeddata",
  "canplay",
  "canplaythrough",
  "play",
  "playing",
  "pause",
  "seeking",
  "seeked",
  "timeupdate",
  "durationchange",
  "ratechange",
  "volumechange",
  "waiting",
  "ended",
  "error",
  "emptied",
  "load",
]);
const MEDIA_EVENTS = new Set<string>([
  "loadstart",
  "loadedmetadata",
  "loadeddata",
  "canplay",
  "canplaythrough",
  "play",
  "playing",
  "pause",
  "seeking",
  "seeked",
  "timeupdate",
  "durationchange",
  "ratechange",
  "volumechange",
  "waiting",
  "ended",
  "error",
  "emptied",
]);
const nodesById = new Map<VnNodeId, MirrorNode>();

// Loose nodes (made by `document.createElement`, `createTextNode`, `new DocumentFragment()`, or taken out of the tree)
// are held weakly; `sweepLoose` frees the host nodes of collected subtrees. The host has no "detach", so a removed
// subtree is parked under an unattached "box" node, as <KeepAlive> parks a deactivated one.
// Invariant: a node is loose exactly when its topmost ancestor is in `looseRoots`; `nodeOps.insert` moves a subtree
// between the registries when it changes sides.
const looseById = new Map<VnNodeId, WeakRef<MirrorNode>>();
/** The root of each loose subtree, and the box a removed one is parked in (0: none, it never had a parent). */
const looseRoots = new Map<VnNodeId, { ref: WeakRef<MirrorNode>; box: VnNodeId }>();
function nodeById(id: VnNodeId): MirrorNode | undefined {
  return nodesById.get(id) ?? (looseById.size ? looseById.get(id)?.deref() : undefined);
}
const isLoose = (n: MirrorNode): boolean => looseById.size !== 0 && looseById.get(n.__vnId)?.deref() === n;
function loosen(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    if (nodesById.get(d.__vnId) === d) nodesById.delete(d.__vnId);
    looseById.set(d.__vnId, new WeakRef(d));
  });
}
function tighten(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    looseById.delete(d.__vnId);
    nodesById.set(d.__vnId, d);
  });
}
let sweepQueued = false;
const looseFinalizer: FinalizationRegistry<VnNodeId> | null =
  typeof FinalizationRegistry === "function"
    ? new FinalizationRegistry<VnNodeId>(() => {
        if (sweepQueued) return;
        sweepQueued = true;
        queueMicrotask(sweepLoose);
      })
    : null;
/** Without a FinalizationRegistry the sweep runs every this many new loose roots. */
let rootsSinceSweep = 0;
function addLooseRoot(n: MirrorNode, box: VnNodeId): void {
  looseRoots.set(n.__vnId, { ref: new WeakRef(n), box });
  if (looseFinalizer) looseFinalizer.register(n, n.__vnId, n);
  else if (++rootsSinceSweep >= 256) {
    rootsSinceSweep = 0;
    sweepLoose();
  }
}
function dropLooseRoot(n: MirrorNode): VnNodeId {
  if (looseRoots.size === 0) return 0;
  const r = looseRoots.get(n.__vnId);
  if (!r || r.ref.deref() !== n) return 0;
  looseRoots.delete(n.__vnId);
  looseFinalizer?.unregister(n);
  return r.box;
}
function sweepLoose(): void {
  sweepQueued = false;
  for (const [id, r] of looseRoots) {
    if (r.ref.deref()) continue;
    looseRoots.delete(id);
    __vn_remove(r.box || id);
  }
  for (const [id, ref] of looseById) if (!ref.deref()) looseById.delete(id);
}
function looseCreated<T extends MirrorNode>(n: T): T {
  loosen(n);
  addLooseRoot(n, 0);
  return n;
}
/** Takes `child` out of its parent WITHOUT freeing it (a script may re-insert it); its subtree becomes loose, parked in a box. */
function detachNode(child: MirrorNode, notify = true): void {
  const p = child.parentNode;
  if (!p) return;
  releaseCapturesIn(child);
  const i = p.childNodes.indexOf(child);
  if (i >= 0) p.childNodes.splice(i, 1);
  child.parentNode = null;
  domEpoch++;
  optionsChangedIn(p);
  const box = __vn_create_element(I("vn-detached"));
  __vn_insert(child.__vnId, box, 0);
  if (!isLoose(child)) loosen(child);
  addLooseRoot(child, box);
  if (notify) notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
}
/** Forgets a subtree whose host nodes are freed: registries, handlers, and its ids (set to 0, since host ids are recycled). */
function forgetDeep(n: MirrorNode): void {
  releaseCapturesIn(n);
  forEachDeep(n, (d) => {
    handlers.delete(d);
    if (nodesById.get(d.__vnId) === d) nodesById.delete(d.__vnId);
    else if (looseById.size) looseById.delete(d.__vnId);
    d.__vnId = 0;
  });
}

// One-shot compat advisories: never silently drop behavior a browser has; say what to use instead.
const warned = new Set<string>();
function warnCompat(key: string, msg: string) {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[vue-native compat] ${msg}`);
}

// Mirrors sit on HTMLElement.prototype so `instanceof HTMLElement`/`Node` checks in reused browser components pass.
// The bun test harness has no HTMLElement; a private prototype stands in so the DOM surface never lands on Object.prototype.
const ElementProto: object =
  typeof HTMLElement === "function" ? HTMLElement.prototype : Object.create(Object.prototype);
const TextProto: object = Object.create(Object.prototype);

// NodeList, HTMLCollection and DOMRectList: code tests for them by name (motion's `subject instanceof NodeList`).
// `childNodes` IS the renderer's child array (live by construction) and stays a plain Array: re-parenting it onto
// NodeList.prototype made JavaScriptCore's `indexOf` 30 times slower. `childList` gives it `item` and the toString
// tag; `Array.isArray(el.childNodes)` stays true. Static lists (`querySelectorAll()`, `getClientRects()`) are
// read-only snapshots; live collections (`children`, `getElementsByTagName()`) are Proxies recomputed when
// `domEpoch` changes.

/** Bumped by every change a live collection could see: an insert, a removal, an `id`/`name`/`class` change. */
let domEpoch = 0;

const staticItems = new WeakMap<object, readonly unknown[]>();

interface LiveState {
  compute: () => readonly unknown[];
  items: readonly unknown[];
  epoch: number;
  named: boolean;
}
const liveStates = new WeakMap<object, LiveState>();

function liveItems(s: LiveState): readonly unknown[] {
  if (s.epoch !== domEpoch) {
    s.items = s.compute();
    s.epoch = domEpoch;
  }
  return s.items;
}

function listItems(self: unknown): ArrayLike<unknown> {
  if (self && typeof self === "object") {
    const fixed = staticItems.get(self);
    if (fixed) return fixed;
    const live = liveStates.get(self);
    if (live) return liveItems(live);
    if (isChildList(self)) return self;
  }
  throw new TypeError("Illegal invocation");
}

function namedMember(items: ArrayLike<unknown>, key: string): unknown {
  if (key === "") return null;
  for (let i = 0; i < items.length; i++) {
    const e = items[i] as Partial<MirrorElement> | undefined;
    const a = e?.attrs;
    if (a && (a.id === key || (a.name === key && !e._ns))) return e;
  }
  return null;
}
function namedKeys(items: ArrayLike<unknown>): string[] {
  const out = new Set<string>();
  for (let i = 0; i < items.length; i++) {
    const e = items[i] as Partial<MirrorElement> | undefined;
    const a = e?.attrs;
    if (!a) continue;
    if (a.id) out.add(a.id);
    if (a.name && !e._ns) out.add(a.name);
  }
  return [...out];
}

function indexOfKey(k: string): number {
  const c = k.charCodeAt(0);
  if (!(c >= 48 && c <= 57)) return -1;
  const n = Number(k);
  return n <= 4294967294 && String(n) === k ? n : -1;
}

/**
 * WebIDL's legacy platform object over a live list: index properties are read-only and enumerable, named ones
 * (an HTMLCollection's) read-only and not enumerable; a name the prototype chain has (`item`, `length`) wins.
 */
const LIVE_HANDLER: ProxyHandler<object> = {
  get(t, k, recv) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) return liveItems(s)[i];
      if (k === "length") return liveItems(s).length;
      if (s.named && !(k in t)) {
        const n = namedMember(liveItems(s), k);
        if (n) return n;
      }
    }
    return Reflect.get(t, k, recv);
  },
  has(t, k) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) return i < liveItems(s).length;
      if (s.named && !(k in t) && namedMember(liveItems(s), k)) return true;
    }
    return Reflect.has(t, k);
  },
  ownKeys(t) {
    const s = liveStates.get(t)!;
    const items = liveItems(s);
    const keys: (string | symbol)[] = [];
    for (let i = 0; i < items.length; i++) keys.push(String(i));
    if (s.named) for (const n of namedKeys(items)) if (indexOfKey(n) < 0 && !(n in t)) keys.push(n);
    for (const k of Reflect.ownKeys(t)) keys.push(k);
    return keys;
  },
  getOwnPropertyDescriptor(t, k) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) {
        const items = liveItems(s);
        return i < items.length
          ? { value: items[i], writable: false, enumerable: true, configurable: true }
          : undefined;
      }
      if (s.named && !(k in t)) {
        const n = namedMember(liveItems(s), k);
        if (n) return { value: n, writable: false, enumerable: false, configurable: true };
      }
    }
    return Reflect.getOwnPropertyDescriptor(t, k);
  },
  defineProperty(t, k, d) {
    if (typeof k === "string") {
      if (indexOfKey(k) >= 0) return false;
      const s = liveStates.get(t)!;
      if (s.named && !Object.prototype.hasOwnProperty.call(t, k) && namedMember(liveItems(s), k)) return false;
    }
    return Reflect.defineProperty(t, k, d);
  },
  deleteProperty(t, k) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) return i >= liveItems(s).length;
      if (s.named && !(k in t) && namedMember(liveItems(s), k)) return false;
    }
    return Reflect.deleteProperty(t, k);
  },
  preventExtensions: () => false,
};

function liveCollection<T>(proto: object, named: boolean, compute: () => readonly T[]): MirrorCollection<T> {
  const target = Object.create(proto) as object;
  const s: LiveState = { compute, items: [], epoch: -1, named };
  const list = new Proxy(target, LIVE_HANDLER);
  liveStates.set(target, s);
  liveStates.set(list, s);
  return list as MirrorCollection<T>;
}

function staticList<T>(proto: object, items: readonly T[]): MirrorCollection<T> {
  const list = Object.create(proto) as Record<number, unknown>;
  for (let i = 0; i < items.length; i++) {
    Object.defineProperty(list, i, { value: items[i], enumerable: true, configurable: true });
  }
  staticItems.set(list, items);
  return list as unknown as MirrorCollection<T>;
}

function collectionProto(name: string, opts: { named?: boolean; iterable?: boolean } = {}): object {
  const proto: object = Object.create(Object.prototype);
  const required = (member: string, n: number) => {
    if (n < 1)
      throw new TypeError(`Failed to execute '${member}' on '${name}': 1 argument required, but only 0 present.`);
  };
  const members: Record<string, PropertyDescriptor> = {
    item: {
      value: function item(this: unknown, index: unknown) {
        required("item", arguments.length);
        const items = listItems(this);
        const i = Number(index) >>> 0;
        return i < items.length ? items[i] : null;
      },
    },
    length: {
      get: function length(this: unknown) {
        return listItems(this).length;
      },
    },
  };
  if (opts.named) {
    members.namedItem = {
      value: function namedItem(this: unknown, key: unknown) {
        required("namedItem", arguments.length);
        return namedMember(listItems(this), String(key));
      },
    };
  }
  if (opts.iterable) {
    for (const k of ["entries", "keys", "values", "forEach"] as const) members[k] = { value: Array.prototype[k] };
  }
  for (const [k, d] of Object.entries(members)) {
    Object.defineProperty(proto, k, {
      ...d,
      enumerable: true,
      configurable: true,
      ...("value" in d ? { writable: true } : {}),
    });
  }
  Object.defineProperty(proto, Symbol.iterator, { value: Array.prototype.values, writable: true, configurable: true });
  Object.defineProperty(proto, Symbol.toStringTag, { value: name, configurable: true });
  return proto;
}
const NodeListProto = collectionProto("NodeList", { iterable: true });
const HTMLCollectionProto = collectionProto("HTMLCollection", { named: true });
const DOMRectListProto = collectionProto("DOMRectList");

// NodeList's own `item` and toString tag for each `childNodes` array, so `Object.prototype.toString` says
// "[object NodeList]" and `reactive()` leaves the list alone.
const CHILD_ITEM: PropertyDescriptor = {
  value: (NodeListProto as { item: unknown }).item,
  writable: true,
  configurable: true,
};
const CHILD_TAG: PropertyDescriptor = { value: "NodeList", configurable: true };
function childList(): MirrorNode[] {
  const list: MirrorNode[] = [];
  Object.defineProperty(list, "item", CHILD_ITEM);
  Object.defineProperty(list, Symbol.toStringTag, CHILD_TAG);
  return list;
}
function isChildList(x: object): x is MirrorNode[] {
  return Array.isArray(x) && (x as { item?: unknown }).item === CHILD_ITEM.value;
}

const NO_NODES: readonly MirrorNode[] = Object.freeze([]);

const BRANDS = {
  DOMTokenList: new WeakSet<object>(),
  DOMStringMap: new WeakSet<object>(),
  MutationRecord: new WeakSet<object>(),
  ResizeObserverEntry: new WeakSet<object>(),
  ResizeObserverSize: new WeakSet<object>(),
  TreeWalker: new WeakSet<object>(),
};
function branded<T extends object>(kind: keyof typeof BRANDS, obj: T): T {
  BRANDS[kind].add(obj);
  return obj;
}

const HTML_NS = "http://www.w3.org/1999/xhtml";
const SVG_NS = "http://www.w3.org/2000/svg";
const MATHML_NS = "http://www.w3.org/1998/Math/MathML";

// A mirror node must never become reactive state: ours stringify as `[object Object]`, which Vue's `getTargetType`
// would Proxy-wrap, so `set.has`, `indexOf` and `===` would disagree about one node (reka-ui's layer stack and focus
// guards compare identity). `__v_skip` (markRaw) on the prototypes stops it.
for (const proto of [ElementProto, TextProto]) {
  Object.defineProperty(proto, "__v_skip", { value: true, configurable: true });
}

function mk(id: VnNodeId, kind: 1, tag: string): MirrorElement;
function mk(id: VnNodeId, kind: 2 | 3, tag: string): MirrorText;
function mk(id: VnNodeId, kind: MirrorKind, tag: string): MirrorNode;
function mk(id: VnNodeId, kind: MirrorKind, tag: string): MirrorNode {
  const n = Object.create(kind === 1 ? ElementProto : TextProto) as MirrorNodeInit;
  n.__vnId = id;
  n.kind = kind;
  n.tag = tag;
  n.parentNode = null;
  n.childNodes = childList();
  if (kind === 1) {
    n.tagName = String(tag).toUpperCase();
    // Storage under the prototype's `value` accessor; being OWN also tells a mirror element from a detached engine one.
    n._value = "";
    // Written only by `patchProp`. reka-ui reads `data-*`/`aria-*` back off nodes, so the mirror must remember them.
    n.attrs = Object.create(null);
    // Kept apart from `<Transition>`'s classes so a re-render does not wipe an in-flight `v-enter-active`.
    n._class = "";
    // `scrollTop`/`scrollLeft` are prototype accessors over the host's `ScrollHandle`; an own data property would shadow them.
    // Own methods shadow the prototype's: they route into the registry the renderer's props use, so propagation sees both.
    n.addEventListener = (name, fn, opts) => addExtraListener(n as MirrorElement, name, fn, opts);
    n.removeEventListener = (name, fn, opts) => removeExtraListener(n as MirrorElement, name, fn, opts);
  }
  nodesById.set(id, n as MirrorNode);
  return n as MirrorNode;
}

function forEachDeep(n: MirrorNode, f: (n: MirrorNode) => void) {
  f(n);
  for (const c of n.childNodes) forEachDeep(c, f);
}

// node -> { click: fn }. Weak, so a loose node is not kept alive by its own listeners.
const handlers = new WeakMap<MirrorNode, HandlerMap>();

const nodeOps: Omit<RendererOptions<MirrorNode, MirrorElement>, "patchProp"> = {
  createElement: (type, namespace, _is, props) => {
    const el = mk(__vn_create_element(I(type)), 1, type);
    // `<VnWindow>`'s root: registered before its content mounts (vnwindow.ts).
    if (props && (props as Record<symbol, unknown>)[WINDOW_ROOT]) windowRoots.add(el);
    // `<svg>`/`<math>` and their content keep their tag's case (`linearGradient`), and `instanceof SVGElement`/`MathMLElement` hold.
    if (namespace === "svg" || namespace === "mathml") {
      el._ns = namespace;
      el.tagName = type;
    }
    // The host must deliver `click` for `<label>` (it activates its control) and `<button>` (it submits), and `keydown`
    // for `<input>` (implicit submission), whether or not anything listens.
    if (type === "label" || type === "button") syncListenerFlag(el, "click");
    if (type === "input") syncListenerFlag(el, "keydown");
    if (type === "input" || type === "textarea") syncListenerFlag(el, "input");
    if (type === "canvas") adoptCanvas(el);
    if (!namespace && type === "select") defineSelectMembers(el);
    else if (!namespace && type === "option") defineOptionMembers(el);
    return el;
  },
  createText: (text) => {
    const n = mk(__vn_create_text(), 2, "#text");
    n.text = String(text);
    __vn_set_text(n.__vnId, n.text);
    return n;
  },
  createComment: () => mk(__vn_create_anchor(), 3, "#comment"),
  setText: (node, text) => {
    node.text = String(text);
    __vn_set_text(node.__vnId, node.text);
  },
  setElementText: (node, text) => {
    // DETACH the children, do not free them, as `el.textContent = text` does: a leaving <Transition> child stays until
    // its leave ends, and a script-appended one is still the script's. Freeing either leaves a mirror over a recycled id.
    const kids = node.childNodes.length ? node.childNodes.slice() : null;
    if (kids) for (const c of kids) detachNode(c, false);
    node.ownText = String(text);
    __vn_set_element_text(node.__vnId, String(text));
    if (kids) optionsChangedIn(node);
    notifyMutation(node, "childList", { addedNodes: [], removedNodes: kids ?? [] });
  },
  insert: (child, parent, anchor) => insertNode(child, parent, anchor ?? null, true),
  remove: (child) => {
    const p = child.parentNode;
    if (p) {
      const i = p.childNodes.indexOf(child);
      if (i >= 0) p.childNodes.splice(i, 1);
      child.parentNode = null;
      domEpoch++;
      optionsChangedIn(p);
      notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
    }
    if (nodeById(child.__vnId) !== child) return;
    const id = child.__vnId;
    const box = dropLooseRoot(child);
    forgetDeep(child);
    __vn_remove(box || id);
  },
  parentNode: (node) => node.parentNode,
  nextSibling: (node) => {
    const p = node.parentNode;
    if (!p) return null;
    const i = p.childNodes.indexOf(node);
    return i >= 0 ? (p.childNodes[i + 1] ?? null) : null;
  },
  // <style scoped>: the Rust matcher buckets on the data-v id Vue stamps on scoped elements.
  setScopeId: (el, id) => __vn_add_scope(el.__vnId, id),
  // Resolves a STRING Teleport `to` (reka-ui portals use "body"). Document roots mean the app root here; anything else
  // goes through the real matcher. A Teleport inside a `<VnWindow>` asks `windowTeleport` instead.
  querySelector: (sel) => {
    if (DOCUMENT_ROOTS.has(sel) && sel !== "#app") return appRoot;
    return queryOne(sel, appRoot ? appRoot.__vnId : 0) ?? (sel === "#app" ? appRoot : null);
  },
};

const DOCUMENT_ROOTS = new Set(["body", "html", ":root", "#app"]);

/** The `<VnWindow>` root `node` paints under, or `null` for the main window (a pop-out's content stays in the one tree). */
function windowRootOf(node: MirrorNode | null | undefined): MirrorElement | null {
  if (windowRoots.size === 0) return null;
  for (let n = node; n; n = n.parentNode) if (windowRoots.has(n)) return n as MirrorElement;
  return null;
}

interface MountingInstance {
  type: unknown;
  parent: MountingInstance | null;
  subTree?: { el?: unknown } | null;
}

/**
 * The `<VnWindow>` root that `container`, not in the document yet, will paint under, or `null`. Vue mounts children
 * before inserting the element, so a `<Teleport>` gets a container whose parents stop short of the window root; the
 * nearest `<VnWindow>` above the Teleport says which window it is in.
 */
function mountingWindowRoot(
  container: MirrorNode | null | undefined,
  parent: MountingInstance | null | undefined,
): MirrorElement | null {
  if (windowRoots.size === 0 || !container) return null;
  for (let n: MirrorNode | null = container; n; n = n.parentNode) if (n === appRoot) return null;
  for (let i = parent; i; i = i.parent) {
    if (i.type !== VnWindow) continue;
    const el = i.subTree?.el as MirrorElement | null | undefined;
    return el && windowRoots.has(el) ? el : null;
  }
  return null;
}

/**
 * `<Teleport to="body">` inside a `<VnWindow>` lands in THAT window. Teleport resolves a string `to` through the
 * renderer options it is handed (`internals.o.querySelector`), so it gets options whose document roots are the
 * window root its placeholder sits under.
 */
const TELEPORT_ORIGINAL = Symbol.for("vn.teleport.original");
type TeleportProcess = ((this: unknown, ...args: unknown[]) => unknown) & { [TELEPORT_ORIGINAL]?: TeleportProcess };
interface TeleportInternals {
  o: { querySelector?: (sel: string) => unknown };
}
{
  const impl = Teleport as unknown as { process: TeleportProcess };
  // A reload may re-evaluate this module over the same Teleport: wrap Vue's own `process`, never a predecessor's
  // wrapper, whose window registry is the old app's.
  let process = impl.process;
  while (process[TELEPORT_ORIGINAL]) process = process[TELEPORT_ORIGINAL];
  const wrapped: TeleportProcess = function (this: unknown, ...args: unknown[]) {
    const container = args[2] as MirrorNode | null;
    const root = windowRootOf(container) ?? mountingWindowRoot(container, args[4] as MountingInstance | null);
    const internals = args[9] as TeleportInternals | undefined;
    if (root && internals?.o) {
      const select = internals.o.querySelector;
      args[9] = {
        ...internals,
        o: {
          ...internals.o,
          querySelector: (sel: string) => (DOCUMENT_ROOTS.has(sel) ? root : select ? select(sel) : null),
        },
      };
    }
    return process.apply(this, args);
  };
  wrapped[TELEPORT_ORIGINAL] = process;
  impl.process = wrapped;
}

/**
 * The document a node in a `<VnWindow>` belongs to (`ownerDocument`). floating-ui measures the viewport as
 * `el.ownerDocument.documentElement.clientWidth`, so the root elements answer with the window's root; the rest is
 * the real document. Not `=== document`, like a browser popup's.
 */
const WINDOW_DOCUMENTS = new WeakMap<object, object>();
function windowDocument(root: MirrorElement): object {
  const known = WINDOW_DOCUMENTS.get(root);
  if (known) return known;
  const real = globalThis.document as unknown as object;
  const bound = new WeakMap<object, unknown>();
  const doc = new Proxy(real, {
    get(target, key) {
      if (key === "documentElement" || key === "body" || key === "scrollingElement") return root;
      const v: unknown = Reflect.get(target, key, target);
      if (typeof v !== "function") return v;
      // Methods answer for the real document (native `this`), the same function on every read.
      let b = bound.get(v);
      if (!b) bound.set(v, (b = (v as (...a: unknown[]) => unknown).bind(target)));
      return b;
    },
  });
  WINDOW_DOCUMENTS.set(root, doc);
  return doc;
}

/** Insert or move. `notifyAdd: false` leaves the addition record to the caller (one per fragment); the old parent always hears the removal. */
function insertNode(child: MirrorNode, parent: MirrorElement, anchor: MirrorNode | null, notifyAdd: boolean): void {
  const old = child.parentNode;
  if (old && (isLoose(parent) || windowRootOf(child) !== windowRootOf(parent))) releaseCapturesIn(child);
  if (old) {
    const cs = old.childNodes;
    const i = cs.indexOf(child);
    if (i >= 0) cs.splice(i, 1);
  }
  let idx = parent.childNodes.length;
  if (anchor != null) {
    const i = parent.childNodes.indexOf(anchor);
    if (i >= 0) idx = i;
  }
  parent.childNodes.splice(idx, 0, child);
  child.parentNode = parent;
  domEpoch++;
  if (parent.ownText) parent.ownText = "";
  __vn_insert(child.__vnId, parent.__vnId, anchor ? anchor.__vnId : 0);
  if (looseById.size) {
    const box = dropLooseRoot(child);
    if (box) __vn_remove(box);
    const now = isLoose(parent);
    if (now !== isLoose(child)) (now ? loosen : tighten)(child);
  }
  if (old && old !== parent) optionsChangedIn(old);
  optionsChangedIn(parent);
  if (old) notifyMutation(old, "childList", { addedNodes: [], removedNodes: [child] });
  if (notifyAdd) notifyMutation(parent, "childList", { addedNodes: [child], removedNodes: [] });
}

// `\B` so a leading capital is only lowercased, never prefixed: `withKeys` runs this over
// `KeyboardEvent.key`, and "Enter" must become "enter", not "-enter".
const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

/** A `:style` key or `CSSStyleDeclaration` name as CSS spells it (`backgroundColor` -> `background-color`); custom properties stay as written. */
function cssPropName(k: string): string {
  if (k.startsWith("--")) return k;
  if (k === "cssFloat") return "float";
  if (k.includes("-")) return k.toLowerCase();
  const h = hyphenate(k);
  return /^(webkit|moz|ms)-/.test(h) ? "-" + h : h;
}
const cssomName = (k: string): string => (k.startsWith("--") ? k : k.toLowerCase());
const IMPORTANT_RE = /\s*!\s*important\s*$/i;
const PROP_NAME_RE = /^(?:--[^\s:;]+|-?[A-Za-z_\u0080-￿][\w\u0080-￿-]*)$/;

/** A declaration's value as stored, trimmed. An empty custom property is valid CSS but "" means remove on the host channel, so it is kept as one space. */
const inlineValue = (v: string, custom: boolean): string => {
  const t = v.trim();
  return t === "" && custom ? " " : t;
};

function inlineOf(el: MirrorElement): InlineStyleState {
  return (el._inline ??= { decls: new Map(), present: false, raw: null, cssVarText: "" });
}
const hostDecl = (d: InlineDecl): string => (d.important ? `${d.value} !important` : d.value);
function serializeInline(st: InlineStyleState | undefined): string {
  let out = "";
  if (st) {
    for (const [name, d] of st.decls) {
      out += `${out ? " " : ""}${name}: ${d.value}${d.important ? " !important" : ""};`;
    }
  }
  return out;
}
/** Said once, at the first read: unlike a browser, serialization keeps values as written (no re-spelling, no shorthand folding, rejected values kept). */
function noteInlineSerialization(): void {
  warnCompat(
    "inline-style-serialize",
    "el.style.cssText / getAttribute('style') serialize the inline declarations as they were written: " +
      "values are not re-spelled (`#f00` stays `#f00`), a shorthand is not expanded into its longhands " +
      '(after `margin: 0`, `el.style.marginTop` reads ""), and a value the engine rejects is still listed',
  );
}
function styleAttribute(el: MirrorElement): string | null {
  const st = el._inline;
  if (!st || !st.present) return null;
  if (st.raw !== null) return st.raw;
  const text = serializeInline(st);
  if (text) noteInlineSerialization();
  return text;
}
const styleBefore = (el: MirrorElement): string | null => (observers.size ? styleAttribute(el) : null);
function styleTouched(el: MirrorElement, st: InlineStyleState, before: string | null): void {
  st.present = true;
  st.raw = null;
  if (el.attrs) el.attrs.style = "";
  notifyMutation(el, "attributes", { attributeName: "style", oldValue: before });
}

function setInlineDecl(el: MirrorElement, name: string, value: string, important: boolean): boolean {
  if (value === "") return removeInlineDecl(el, name) !== null;
  const st = inlineOf(el);
  const prev = st.decls.get(name);
  if (prev && prev.value === value && prev.important === important) return false;
  const before = styleBefore(el);
  const d = { value, important };
  st.decls.set(name, d);
  __vn_set_style(el.__vnId, I(name), hostDecl(d));
  styleTouched(el, st, before);
  return true;
}
function removeInlineDecl(el: MirrorElement, name: string): string | null {
  const st = el._inline;
  const prev = st?.decls.get(name);
  if (!st || !prev) return null;
  const before = styleBefore(el);
  st.decls.delete(name);
  __vn_set_style(el.__vnId, I(name), "");
  styleTouched(el, st, before);
  return prev.value;
}

/** CSS "parse a list of declarations": `;` ends one only outside strings, comments and brackets; a repeated property keeps its last, but a normal one never overrides an earlier `!important`. */
function parseInlineDecls(text: string): Map<string, InlineDecl> {
  const out = new Map<string, InlineDecl>();
  const s = String(text);
  let seg = "";
  let depth = 0;
  let quote = "";
  const flush = () => {
    const colon = seg.indexOf(":");
    if (colon > 0) {
      const raw = seg.slice(0, colon).trim();
      let value = seg.slice(colon + 1);
      const m = IMPORTANT_RE.exec(value);
      const important = !!m;
      if (m) value = value.slice(0, m.index);
      value = value.trim();
      const custom = raw.startsWith("--");
      if (PROP_NAME_RE.test(raw) && (value !== "" || custom)) {
        const name = cssomName(raw);
        value = inlineValue(value, custom);
        const prev = out.get(name);
        if (!(prev?.important && !important)) {
          out.delete(name);
          out.set(name, { value, important });
        }
      }
    }
    seg = "";
  };
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quote) {
      seg += c;
      if (c === "\\" && i + 1 < s.length) seg += s[++i];
      else if (c === quote) quote = "";
      continue;
    }
    if (c === "/" && s[i + 1] === "*") {
      const end = s.indexOf("*/", i + 2);
      i = end < 0 ? s.length : end + 1;
      seg += " ";
      continue;
    }
    if (c === "\\" && i + 1 < s.length) {
      seg += c + s[++i];
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    else if (c === "(" || c === "[" || c === "{") depth++;
    else if ((c === ")" || c === "]" || c === "}") && depth > 0) depth--;
    else if (c === ";" && depth === 0) {
      flush();
      continue;
    }
    seg += c;
  }
  flush();
  return out;
}

/**
 * `style.cssText = text`, or `setAttribute("style", text)` (`asAttribute`, which keeps `text`
 * verbatim). Unchanged declarations cost no host call while the host's order stays the new order;
 * otherwise the block is re-sent, because order decides a shorthand against its longhand.
 */
function replaceInline(el: MirrorElement, text: string, asAttribute: boolean): void {
  const st = inlineOf(el);
  const before = styleBefore(el);
  const next = parseInlineDecls(text);
  const old = st.decls;
  const order = [...old.keys()].filter((k) => next.has(k));
  for (const k of next.keys()) if (!old.has(k)) order.push(k);
  let i = 0;
  let inPlace = true;
  for (const k of next.keys()) {
    if (order[i++] !== k) {
      inPlace = false;
      break;
    }
  }
  for (const k of old.keys()) if (!inPlace || !next.has(k)) __vn_set_style(el.__vnId, I(k), "");
  for (const [k, d] of next) {
    const p = old.get(k);
    if (inPlace && p && p.value === d.value && p.important === d.important) continue;
    __vn_set_style(el.__vnId, I(k), hostDecl(d));
  }
  st.decls = next;
  styleTouched(el, st, before);
  if (asAttribute) st.raw = String(text);
}

const vShowOriginalDisplay = Symbol("_vod");
const vShowHidden = Symbol("_vsh");
const DISPLAY_RE = /(^|;)\s*display\s*:/;
const RD_IMPORTANT_RE = /\s*!important$/;

/** runtime-dom's `patchStyle`: a string is the whole block (`useCssVars` variables kept), a record is diffed per property, null clears. */
function patchStyle(el: MirrorElement, prev: unknown, next: unknown): void {
  const isCssString = typeof next === "string";
  let hasControlledDisplay = false;
  if (next && !isCssString) {
    const n = next as Record<string, unknown>;
    if (prev) {
      if (typeof prev !== "string") {
        for (const key in prev as Record<string, unknown>) if (n[key] == null) setStyleValue(el, key, "");
      } else {
        for (const prevStyle of prev.split(";")) {
          const key = prevStyle.slice(0, prevStyle.indexOf(":")).trim();
          if (key && n[key] == null) setStyleValue(el, key, "");
        }
      }
    }
    for (const key in n) {
      if (key === "display") hasControlledDisplay = true;
      setStyleValue(el, key, n[key]);
    }
  } else if (isCssString) {
    if (prev !== next) {
      const vars = el._inline?.cssVarText;
      replaceInline(el, vars ? `${next};${vars}` : next, false);
      hasControlledDisplay = DISPLAY_RE.test(next);
    }
  } else if (prev) {
    clearInline(el);
  }
  if (vShowOriginalDisplay in el) {
    el[vShowOriginalDisplay] = hasControlledDisplay ? (el._inline?.decls.get("display")?.value ?? "") : "";
    if (el[vShowHidden]) setInlineDecl(el, "display", "none", false);
  }
}

function setStyleValue(el: MirrorElement, name: string, val: unknown): void {
  if (Array.isArray(val)) {
    setStyleFallbacks(el, name, val);
    return;
  }
  const v = val == null ? "" : String(val);
  const prop = cssPropName(name);
  if (v === "") {
    removeInlineDecl(el, prop);
  } else if (prop.startsWith("--")) {
    setInlineDecl(el, prop, inlineValue(v, true), false);
  } else if (RD_IMPORTANT_RE.test(v)) {
    const bare = v.replace(RD_IMPORTANT_RE, "");
    if (bare.trim() === "") removeInlineDecl(el, prop);
    else setInlineDecl(el, prop, inlineValue(bare, false), true);
  } else {
    setInlineDecl(el, prop, inlineValue(v, false), false);
  }
}

/** `{ display: ["-webkit-box", "flex"] }`: the last entry the engine accepts (`__vn_style_accepts`) wins; with none, the declaration is left as it was. */
function setStyleFallbacks(el: MirrorElement, name: string, list: unknown[]): void {
  const flat: unknown[] = list.flat(Infinity);
  const prop = cssPropName(name);
  for (let i = flat.length - 1; i >= 0; i--) {
    const v = flat[i] == null ? "" : String(flat[i]);
    const bare = v.replace(RD_IMPORTANT_RE, "");
    if (bare.trim() === "" || styleAccepts(prop, bare.trim())) {
      if (i < flat.length - 1) {
        warnCompat(
          `style-array:${prop}`,
          `:style ${prop}: [${flat.map((x) => JSON.stringify(x)).join(", ")}] uses ${JSON.stringify(flat[i])}: ` +
            `the engine does not apply the later ${flat.length - 1 - i === 1 ? "entry" : "entries"}. A browser keeps ` +
            `the last entry IT accepts, so one that supports a later entry would use that one`,
        );
      }
      setStyleValue(el, name, flat[i]);
      return;
    }
  }
}

function styleAccepts(prop: string, value: string): boolean {
  if (prop.startsWith("--")) return true;
  if (typeof __vn_style_accepts !== "function") {
    warnCompat(
      "style-array:host",
      "an array :style value keeps its LAST entry here: this host cannot say which values the engine " +
        "accepts (__vn_style_accepts is missing), where a browser keeps the last entry it accepts",
    );
    return true;
  }
  return __vn_style_accepts(I(prop), value);
}

function clearInline(el: MirrorElement): void {
  const st = el._inline;
  if (!st || !st.present) return;
  const before = styleBefore(el);
  for (const k of st.decls.keys()) __vn_set_style(el.__vnId, I(k), "");
  st.decls = new Map();
  st.present = false;
  st.raw = null;
  if (el.attrs) delete el.attrs.style;
  notifyMutation(el, "attributes", { attributeName: "style", oldValue: before });
}

// Reflected IDL attributes (`el.hidden`, `input.placeholder`, `el.tabIndex`): tags (`null`: every HTML element),
// attribute and IDL type. Drives `attributeOf` and `defineReflected`.
type ReflectKind = "bool" | "string" | "long";
interface Reflected {
  attr: string;
  kind: ReflectKind;
  tags: ReadonlySet<string> | null;
}
const REFLECTED: Record<string, Reflected> = Object.create(null);
/** `names`: space-separated properties, `prop=attr` where the attribute is not the lowercased name. */
function reflect(kind: ReflectKind, names: string, tags: string | null): void {
  const set = tags ? new Set(tags.split(" ")) : null;
  for (const n of names.split(" ")) {
    const [prop = n, attr = prop.toLowerCase()] = n.split("=");
    REFLECTED[prop] = { attr, kind, tags: set };
  }
}
reflect("bool", "hidden inert autofocus", null);
reflect("bool", "disabled", "button fieldset input link optgroup option select textarea");
reflect("bool", "required", "input select textarea");
reflect("bool", "multiple", "input select");
reflect("bool", "readOnly", "input textarea");
reflect("bool", "open", "details dialog");
reflect("bool", "noValidate", "form");
reflect("bool", "formNoValidate", "button input");
reflect("bool", "reversed", "ol");
reflect("bool", "allowFullscreen", "iframe");
reflect("bool", "isMap", "img");
reflect("bool", "autoplay controls loop", "audio video"); // `loop`'s getter is the media state's (`defineMediaGet`)
reflect("bool", "playsInline", "video");
reflect("bool", "webkitdirectory", "input");
reflect("bool", "defaultSelected=selected", "option");
reflect("string", "title lang accessKey", null);
reflect("string", "placeholder", "input textarea");
reflect("string", "name", "button fieldset form iframe input map meta object output select slot textarea");
reflect("string", "alt", "area img input");
reflect("string", "allow srcdoc", "iframe");
reflect("string", "referrerPolicy loading", "iframe img");
reflect("string", "accept min max step pattern", "input");
reflect("string", "htmlFor=for", "label output");
reflect("long", "tabIndex", null);
reflect("long", "maxLength minLength", "input textarea");

function reflectedOf(el: MirrorElement, key: string): Reflected | null {
  const r = REFLECTED[key];
  return r && (!r.tags || r.tags.has(el.tag)) ? r : null;
}

/** WebIDL `long`: ToNumber, then truncated into 32 bits; NaN and the infinities are 0. */
function toLong(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) ? Math.trunc(n) | 0 : 0;
}

/** `@vue/shared`'s `isOn`: `on` then anything but a lowercase letter. `once`, `online` are attributes. */
function isOn(key: string): boolean {
  const c = key.charCodeAt(2);
  return key.charCodeAt(0) === 111 && key.charCodeAt(1) === 110 && (c > 122 || c < 97);
}

/** The renderer's prop hook; `prev`/`next` are `unknown` since a template can bind anything. `namespace` is set inside `<svg>`/`<math>`, where every key is an attribute. */
function patchProp(el: MirrorElement, key: string, prev: unknown, next: unknown, namespace?: unknown): void {
  if (key === "srcObject" && el.tag === "video") {
    el.srcObject = next;
    return;
  }
  if (key === "style") {
    // A null/undefined value in a record clears its property (@floating-ui/vue's
    // `floatingStyles` is undefined before its first measurement).
    patchStyle(el, prev, next);
  } else if (key === "class") {
    if (prev !== next) {
      el._class = next == null ? "" : String(next);
      writeClass(el, next == null);
    }
  } else if (isOn(key)) {
    // runtime-dom's `parseName`: strip the option suffixes, then read the name. `on:loopComplete`
    // is the case-preserving spelling compiler-dom emits for a custom element's camelCase event.
    let rest = key.slice(2);
    let capture = false;
    let once = false;
    let passive = false;
    for (;;) {
      if (rest.endsWith("Once")) {
        once = true;
        rest = rest.slice(0, -4);
      } else if (rest.endsWith("Capture")) {
        capture = true;
        rest = rest.slice(0, -7);
      } else if (rest.endsWith("Passive")) {
        passive = true;
        rest = rest.slice(0, -7);
      } else break;
    }
    const namespaced = rest.startsWith(":");
    // `\B` keeps a leading capital bare ("Click" -> "click", "LoopComplete" -> "loop-complete");
    // `canonicalEvent` folds that back onto the host's camelCase name.
    const name = namespaced ? rest.slice(1) : canonicalEvent(hyphenate(rest));
    const code = hostCode(name);
    if (!code && RUNTIME_FIRED_EVENTS.has(name)) {
    } else if (!code && JS_SOURCED_EVENTS.has(name)) {
      // `change` / `cancel`: the runtime fires them on a file input (they bubble), so the handler is kept.
      if (next) adviseJsSourced(el, name, `@${name}`);
    } else if (!code) {
      // A listener the host will never deliver must not vanish silently. A `:` inside the name
      // marks a component-emit prop that leaked onto an element; that stays silent.
      if (namespaced || !rest.includes(":")) {
        warnCompat(
          `event:${name}`,
          `@${name} — this event type is not delivered by the native host yet; ` + `the handler will never fire`,
        );
      }
      return;
    }
    let m = handlers.get(el);
    if (!m) {
      m = {};
      handlers.set(el, m);
    }
    let buckets = ensureBuckets(m, name);
    const pass = passiveOf(passive ? { passive } : undefined, el, name);
    if (capture) {
      buckets.capture = (next as EventHandlerOrList) || null;
      buckets.onceC = once;
      buckets.passiveC = pass;
    } else {
      buckets.bubble = (next as EventHandlerOrList) || null;
      buckets.onceB = once;
      buckets.passiveB = pass;
    }
    syncListenerFlag(el, name);
  } else if (key === "key" || key === "ref") {
  } else if (key === "textContent" && !foreignNamespace(namespace)) {
    if (next != null) (el as { textContent: unknown }).textContent = next;
  } else if (key === "value" && isRangeInput(el) && !foreignNamespace(namespace)) {
    writeControlValue(el, next);
  } else if (key === "value" && (isSelectEl(el) || isOptionEl(el)) && !foreignNamespace(namespace)) {
    patchSelectValue(el, next);
  } else if (key === "selected" && isOptionEl(el) && !foreignNamespace(namespace)) {
    patchAttribute(el, ...attributeOf(el, key, next, namespace));
    setOptionSelected(el, next === "" || !!next);
  } else {
    if (key === "true-value") el._trueValue = next;
    else if (key === "false-value") el._falseValue = next;
    else if (key === "checked" && el.tag === "input" && !foreignNamespace(namespace))
      el._checked = next === "" || !!next;
    patchAttribute(el, ...attributeOf(el, key, next, namespace));
  }
}

const foreignNamespace = (ns: unknown): boolean => ns === "svg" || ns === "mathml";

/** runtime-dom's `patchProp` for a non-style/class/listener key, reduced to the attribute it leaves: booleans are `""` or absent (writing "false" would make `[hidden]` match), camelCase names map to their attribute (`tabIndex` -> tabindex), a `long` is an integer, the rest verbatim. Returns `[attribute, value]`; `null` removes. */
function attributeOf(el: MirrorElement, key: string, next: unknown, namespace: unknown): [string, unknown] {
  const on = (): string | null => (next === "" || !!next ? "" : null); // runtime-dom's includeBooleanAttr
  if (SPECIAL_BOOLEAN_ATTRS.has(key)) return [key, on()];
  if (foreignNamespace(namespace)) return [key, next];
  if ((key === "checked" && el.tag === "input") || (key === "selected" && el.tag === "option")) return [key, on()];
  const r = reflectedOf(el, key);
  if (!r) return [key, next];
  if (r.kind === "bool") return [r.attr, on()];
  if (r.attr === key || next == null) return [r.attr, next];
  return [r.attr, r.kind === "long" ? String(toLong(next)) : String(next)];
}

const SPECIAL_BOOLEAN_ATTRS = new Set([
  "itemscope",
  "allowfullscreen",
  "formnovalidate",
  "ismap",
  "nomodule",
  "novalidate",
  "readonly",
]);

function patchAttribute(el: MirrorElement, key: string, next: unknown): void {
  if (next == null) __vn_set_prop(el.__vnId, I(key), undefined);
  else if (typeof next === "string" || typeof next === "number" || typeof next === "boolean")
    __vn_set_prop(el.__vnId, I(key), String(next));
  else
    warnCompat(
      `prop:${key}`,
      `prop "${key}" with a non-primitive value is not supported by the native host yet; ` +
        `it was dropped — pass a string/number/boolean or handle it in the component`,
    );
  if (el.attrs) {
    const before = el.attrs[key];
    if (next == null) delete el.attrs[key];
    else if (typeof next !== "object" && typeof next !== "function") el.attrs[key] = String(next);
    if (before !== el.attrs[key]) {
      notifyMutation(el, "attributes", { attributeName: key, oldValue: before ?? null });
    }
  }
  if (key === "draggable" && !dndSeen && next != null && String(next) === "true") {
    dndSeen = true;
    dndRearm();
  }
  if (key === "value" && !(isRangeInput(el) && el._rangeDirty)) el._value = next == null ? "" : String(next);
  if (isRangeInput(el) && ["value", "type", "min", "max", "step"].includes(key)) syncRangeValue(el);
  if (isColorInput(el) && (key === "value" || key === "type")) writeControlValue(el, el._value);
  if (key === "selected" && isOptionEl(el) && !el._dirty) {
    el._selected = next != null;
    selectednessChanged(el);
  }
  if ((key === "multiple" || key === "size") && isSelectEl(el)) {
    resetSelect(el);
    adviseListBox(el);
  }
  // An `<a href>` is activatable whether or not anything listens, and the host only delivers a
  // click to a node whose click bit is on, so the bit follows the href.
  if (key === "href" && el.tag === "a") syncListenerFlag(el, "click");
  if (key === "type" && el.tag === "input") syncFileInput(el);
  if (key === "type" && (el.tag === "input" || el.tag === "button")) {
    syncListenerFlag(el, "click");
    if (el.tag === "input") syncListenerFlag(el, "keydown");
  }
  // The runtime honours a disabled fieldset; the host's native widgets read only their own `disabled`.
  if (key === "disabled" && el.tag === "fieldset" && next != null && next !== false && next !== "false") {
    warnCompat(
      "fieldset:disabled",
      "<fieldset disabled>: clicks, focus() and form submission treat the controls inside as disabled, " +
        "as a browser does, but the native <input>s inside still accept typing; disable each one as well",
    );
  }
}

function writeAttribute(el: MirrorElement, key: string, value: string | null): void {
  if (key === "class") {
    if (value != null) return setClassText(el, value);
    el._class = "";
    el._vtc = undefined;
    writeClass(el, true);
  } else if (key.startsWith("on") && key.length > 2) patchProp(el, key, undefined, value);
  else patchAttribute(el, key, value);
}

// The class attribute is what Vue assigned plus what `<Transition>` layered on (`el._vtc`), so a re-render
// mid-transition keeps the phase class; `getAttribute`, `className` and `classList` read the merged value.
function writeClass(el: MirrorElement, remove = false): void {
  const base = el._class || "";
  const extra = el._vtc;
  const value = extra && extra.size ? [base, ...extra].filter(Boolean).join(" ") : base;
  __vn_set_class(el.__vnId, value);
  if (!el.attrs) return;
  const before = el.attrs.class;
  if (remove && !value) delete el.attrs.class;
  else el.attrs.class = value;
  if (before !== el.attrs.class) notifyMutation(el, "attributes", { attributeName: "class", oldValue: before ?? null });
}

/**
 * The class attribute becomes `text`, as `className = text` does. A transition class `text` still
 * names stays one (a read-modify-write must not bake `v-enter-active` in); one it drops is dropped.
 */
function setClassText(el: MirrorElement, text: string): void {
  const vtc = el._vtc;
  if (vtc && vtc.size) {
    const tokens = text.split(/\s+/).filter(Boolean);
    for (const c of [...vtc]) if (!tokens.includes(c)) vtc.delete(c);
    if (!vtc.size) el._vtc = undefined;
    el._class = tokens.filter((c) => !vtc.has(c)).join(" ");
  } else {
    el._class = text;
  }
  writeClass(el);
}

function ensureBuckets(m: HandlerMap, name: string): Buckets {
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

function bucketsAlive(b: Buckets): boolean {
  return !!(b.bubble || b.capture || b.extraB.length || b.extraC.length);
}

// HTML drag and drop within the app: state here, the gesture by the dispatcher. Rides the app
// root's listener bits: a press once anything is `draggable="true"`, then moves/release/Escape
// while a press is pending or a drag is under way.
let dndSeen = false;
let dnd: DndGesture | null = null;
let dndSwallowClick = false;

function dndWants(code: number): boolean {
  if (code === EV_CODES.mousedown || code === EV_CODES.selectstart) return dndSeen;
  if (code === EV_CODES.mousemove || code === EV_CODES.mouseup) return !!dnd;
  if (code === EV_CODES.keydown) return dnd?.phase === "dragging";
  return false;
}

function dndRearm(): void {
  if (!appRoot) return;
  for (const name of ["mousedown", "selectstart", "mousemove", "mouseup", "keydown"]) syncListenerFlag(appRoot, name);
}

// Several event names share one host code (`pointerdown` rides `mousedown`): the flag is the OR over all of them.
function syncListenerFlag(self: MirrorNode, name: string): void {
  const code = hostCode(name);
  if (!code) return;
  const id = self.__vnId;
  const m = handlers.get(self);
  let on = NAMES_BY_CODE[code]!.some((n) => m?.[n] && bucketsAlive(m[n]!));
  if (!on && appRoot && id === appRoot.__vnId) {
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
  __vn_set_listener(id, code, on);
}

// Document / window listeners: not mirror nodes, so they have their own registry, joined onto the propagation walk
// one level above the app root. Without it, outside-click and Escape dismissal never fire.
const globalHandlers = new Map<object, HandlerMap>(); // target object -> { [event name]: buckets }
const isMirrorNode = (n: unknown): n is MirrorNode =>
  !!n && typeof (n as MirrorNode).__vnId === "number" && nodeById((n as MirrorNode).__vnId) === (n as MirrorNode);

type ListenerOptionsObject = { capture?: boolean; once?: boolean; passive?: boolean; signal?: ListenerSignal };

const PASSIVE_BY_DEFAULT = new Set(["wheel", "mousewheel", "touchstart", "touchmove"]);

/** Is this listener passive? An explicit `passive` decides; else `wheel`, `mousewheel`, `touchstart`, `touchmove` on `window`, `document` or `<body>` are, as the DOM's default says. */
function passiveOf(opts: ListenerOptions | undefined, target: object, name: string): boolean {
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

function disposeGlobalListener(entry: ExtraListener): void {
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
  dispatchEvent?: (event: VnEvent) => boolean;
};

type GlobalEventMethods = {
  add: GlobalTargetLike["addEventListener"];
  remove: GlobalTargetLike["removeEventListener"];
  dispatch: GlobalTargetLike["dispatchEvent"];
};
type GlobalEventSlot = {
  native: GlobalEventMethods;
  set(delegate: GlobalEventMethods): void;
  clear(delegate: GlobalEventMethods): void;
};
function globalEventBridge(): { get(target: object): GlobalEventSlot | undefined } | undefined {
  return (globalThis as { __VN_GLOBAL_EVENTS__?: { get(target: object): GlobalEventSlot | undefined } })
    .__VN_GLOBAL_EVENTS__;
}

/** The target's own listener and dispatch methods as the FIRST runtime found them, so a reload wraps the same function, not the previous wrapper. */
function nativeListenFns(target: object): {
  add: GlobalTargetLike["addEventListener"];
  remove: GlobalTargetLike["removeEventListener"];
  dispatch: GlobalTargetLike["dispatchEvent"];
} {
  const slot = globalEventBridge()?.get(target);
  if (slot) return slot.native;
  const g = globalThis as { __VN_NATIVE_LISTEN__?: WeakMap<object, ReturnType<typeof nativeListenFns>> };
  const table = (g.__VN_NATIVE_LISTEN__ ??= new WeakMap());
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

const passedThrough: { target: object; name: string; fn: EventHandler; opts: ListenerOptions }[] = [];
const interceptedGlobals: { slot: GlobalEventSlot; delegate: GlobalEventMethods }[] = [];

function interceptGlobalTarget(target: object | undefined): void {
  if (!target) return;
  const bridge = globalEventBridge();
  const slot = bridge?.get(target);
  if (bridge && !slot) throw new TypeError("The host global-events bridge has no slot for this target");
  const { add: nativeAdd, remove: nativeRemove, dispatch: nativeDispatch } = nativeListenFns(target);
  const delegate: GlobalEventMethods = {
    dispatch: (event: VnEvent) => {
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
const JS_SOURCED_EVENTS = new Set<string>(["change", "cancel", "gotpointercapture", "lostpointercapture"]);

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

function addExtraListener(
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
      `addEventListener("${rawName}") on an element — vue-native fires selectionchange at document only ` +
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

function removeExtraListener(node: MirrorElement, rawName: string, fn: EventHandler, opts: ListenerOptions): void {
  const name = canonicalEvent(rawName);
  const buckets = handlers.get(node)?.[name];
  if (!buckets) return;
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  syncListenerFlag(node, name);
}

// The DOM surface on mirror nodes: reused browser components call element methods unconditionally, so a missing one
// is a TypeError. Members answer truthfully (mirror tree, `__vn_query`, `__vn_bounds`, `__vn_computed_style`) or
// return a degraded constant with one advisory.

let appRoot: MirrorElement | null = null; // created on mount or a pre-mount body insertion
// Replays pre-mount `documentElement` writes onto the real root; installed by the document surface below.
let flushDocumentElement: ((root: MirrorElement) => void) | null = null;

/** Any function installed on a mirror prototype; the receiver is annotated per call site so `noImplicitThis` catches a member on the wrong prototype. */
type DomMember = (...args: never[]) => unknown;

function defineDom(name: string, value: DomMember, protos: object[] = [ElementProto]): void {
  for (const proto of protos) {
    Object.defineProperty(proto, name, { value, configurable: true, writable: true });
  }
}
function defineDomGet(
  name: string,
  get: () => unknown,
  set?: ((value: never) => void) | undefined,
  protos: object[] = [ElementProto],
): void {
  for (const proto of protos) Object.defineProperty(proto, name, { get, ...(set ? { set } : {}), configurable: true });
}
type TreeWalkerFilter = ((node: MirrorNode) => number) | { acceptNode?: (node: MirrorNode) => number } | null;

const NODE_MEMBERS: object[] = [ElementProto, TextProto];

/// `__vn_query`, decoded. The host answers `"!<advisory>"` for a selector the engine cannot run
/// (`:has()`, `::before`), so an unanswerable query explains itself instead of reporting "no match".
function hostQuery(selector: string, rootId: VnNodeId, mode: VnQueryMode): MirrorElement[] {
  if (rootId === 0) return [];
  const raw = __vn_query(String(selector), rootId, mode);
  if (raw === "") return [];
  if (raw.charCodeAt(0) === 33 /* ! */) {
    warnCompat(
      `selector:${selector}`,
      `querySelector/closest/matches(${JSON.stringify(String(selector))}) — ${raw.slice(1)}`,
    );
    return [];
  }
  const out: MirrorElement[] = [];
  for (const part of raw.split(",")) {
    const node = nodeById(Number(part));
    if (node) out.push(node as MirrorElement);
  }
  return out;
}
const queryOne = (selector: string, rootId: VnNodeId, mode: VnQueryMode = 0): MirrorElement | null =>
  hostQuery(selector, rootId, mode)[0] ?? null;

const ZERO_RECT = {
  x: 0,
  y: 0,
  top: 0,
  left: 0,
  right: 0,
  bottom: 0,
  width: 0,
  height: 0,
};

/// The viewport as `document.documentElement` reports it: from the client-area origin (the space
/// all rects live in) to the far corner of the app root, which is inset by host chrome. The root's
/// own size would make @floating-ui clip against an offset space and shift every popover left.
function viewportRect(): MirrorRect {
  if (!appRoot) return { ...ZERO_RECT };
  const r = rectOf(appRoot);
  const width = Math.round(r.x + r.width);
  const height = Math.round(r.y + r.height);
  return { x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height };
}
/** `__vn_bounds`, with `"stale"` (the host cannot force layout inside an input handler) folded into `""`. A script-facing read (`kind`) leaves one advisory per call site and kind. */
function boundsOf(node: MirrorNode, kind?: string): VnRect {
  if (!node.__vnId) return "";
  const raw = __vn_bounds(node.__vnId);
  if (raw !== "stale") return raw;
  if (kind) {
    // Frames only (V8 and bun put an `Error` line first, JSC does not): [0] this
    // function, [1] rectOf, [2] the accessor, [3] its caller.
    const frames = (new Error().stack ?? "").split("\n").filter((l) => /^\s*at |@/.test(l));
    const site = frames[3]?.trim() ?? "";
    warnCompat(
      `layout-stale:${kind}:${site}`,
      `${kind} answered 0 for an element that has not been laid out yet` +
        (site ? ` (${site})` : "") +
        ": vue-native cannot lay out while the platform is dispatching mouse or keyboard input, " +
        "so a node mounted in this same event has no size; a browser would lay out and answer it. " +
        "Measure from requestAnimationFrame() or a ResizeObserver callback",
    );
  }
  return "";
}
/** The node's LAYOUT box, before any CSS `transform`: what `offsetWidth`, `clientWidth` and ResizeObserver report. */
function rectOf(node: MirrorNode, kind?: string): MirrorRect {
  return rectFrom(boundsOf(node, kind), 0);
}
/**
 * Where the node was DRAWN: its border box under its own and its ancestors' `transform`, as `getBoundingClientRect()`
 * reports. Poppers sit at (0, 0) and move by `translate()`, so layout boxes would anchor to the window corner. The
 * host appends these four numbers to `__vn_bounds` only when a transform moved the box.
 */
function paintedRectOf(node: MirrorNode, kind?: string): MirrorRect {
  return rectFrom(boundsOf(node, kind), 4);
}
function rectFrom(raw: VnRect, at: 0 | 4): MirrorRect {
  if (!raw)
    return {
      ...ZERO_RECT,
      toJSON() {
        return this;
      },
    };
  const n = raw.split(",").map(Number);
  const [x = NaN, y = NaN, w = NaN, h = NaN] = n.length >= at + 4 ? n.slice(at, at + 4) : n;
  return {
    x,
    y,
    width: w,
    height: h,
    top: y,
    left: x,
    right: x + w,
    bottom: y + h,
    toJSON() {
      return this;
    },
  };
}

defineDom(
  "contains",
  function (this: MirrorNode, other: MirrorNode | null | undefined) {
    for (let n: MirrorNode | null | undefined = other; n; n = n.parentNode) if (n === this) return true;
    return false;
  },
  NODE_MEMBERS,
);
defineDomGet(
  "ownerDocument",
  function (this: MirrorNode) {
    const root = windowRootOf(this);
    return root ? windowDocument(root) : globalThis.document;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "nodeType",
  function (this: MirrorNode) {
    return this.kind === 2 ? 3 : this.kind === 3 ? 8 : 1;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "nodeName",
  function (this: MirrorNode) {
    return (this as Partial<MirrorElement>).tagName ?? (this.kind === 2 ? "#text" : "#comment");
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "isConnected",
  function (this: MirrorNode) {
    for (let n: MirrorNode | null = this; n; n = n.parentNode) if (n === appRoot) return true;
    return false;
  },
  undefined,
  NODE_MEMBERS,
);
for (const alias of ["data", "nodeValue"]) {
  defineDomGet(
    alias,
    function (this: MirrorText) {
      return this.text ?? "";
    },
    function (this: MirrorText, value: unknown) {
      nodeOps.setText(this, value == null ? "" : String(value));
    },
    [TextProto],
  );
}
defineDomGet(
  "textContent",
  function (this: MirrorText) {
    return this.text ?? "";
  },
  function (this: MirrorText, value: unknown) {
    nodeOps.setText(this, value == null ? "" : String(value));
  },
  [TextProto],
);
// `parentElement`: the parent when it is an Element. The one non-mirror parent is `document.head`
// (an element in a browser), so the test is "an element node or an object shaped like one".
defineDomGet(
  "parentElement",
  function (this: MirrorNode) {
    const p = this.parentNode as unknown as { nodeType?: unknown; tagName?: unknown } | null;
    if (!p) return null;
    return p.nodeType === 1 || (p.nodeType === undefined && typeof p.tagName === "string") ? p : null;
  },
  undefined,
  NODE_MEMBERS,
);
function siblingOf(node: MirrorNode, step: 1 | -1): MirrorNode | null {
  const p = node.parentNode;
  if (!p || !p.childNodes) return null;
  // `document.head`'s list is a NodeList with no `indexOf`; a mirror's is an Array.
  const i = Array.prototype.indexOf.call(p.childNodes, node);
  return i >= 0 ? (p.childNodes[i + step] ?? null) : null;
}
defineDomGet(
  "nextSibling",
  function (this: MirrorNode) {
    return siblingOf(this, 1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "previousSibling",
  function (this: MirrorNode) {
    return siblingOf(this, -1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "firstChild",
  function (this: MirrorNode) {
    return this.childNodes?.[0] ?? null;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "lastChild",
  function (this: MirrorNode) {
    const kids = this.childNodes;
    return kids && kids.length ? kids[kids.length - 1] : null;
  },
  undefined,
  NODE_MEMBERS,
);
defineDom(
  "hasChildNodes",
  function (this: MirrorNode) {
    return !!this.childNodes?.length;
  },
  NODE_MEMBERS,
);
defineDom(
  "getRootNode",
  function (this: MirrorNode) {
    let n: MirrorNode = this;
    while (n.parentNode) n = n.parentNode;
    return n === appRoot ? (globalThis.document ?? n) : n;
  },
  NODE_MEMBERS,
);
// `children` is the element-only view of `childNodes`; `aria-hidden` (reka's Dialog) walks it and
// calls `getAttribute` on each, so text and comment mirrors must not appear.
const elementChildren = (n: MirrorNode): MirrorElement[] =>
  (n.childNodes || []).filter((c): c is MirrorElement => c.kind === 1);
const childrenOf = new WeakMap<object, MirrorCollection<MirrorElement>>();
defineDomGet("children", function (this: MirrorElement) {
  let kids = childrenOf.get(this);
  if (!kids) childrenOf.set(this, (kids = liveCollection(HTMLCollectionProto, true, () => elementChildren(this))));
  return kids;
});
defineDomGet("firstElementChild", function (this: MirrorElement) {
  return elementChildren(this)[0] ?? null;
});
defineDomGet("lastElementChild", function (this: MirrorElement) {
  const kids = elementChildren(this);
  return kids[kids.length - 1] ?? null;
});
defineDomGet("childElementCount", function (this: MirrorElement) {
  return elementChildren(this).length;
});
function elementSibling(node: MirrorNode, step: number): MirrorElement | null {
  const p = node.parentNode;
  if (!p || !p.childNodes) return null;
  for (let i = Array.prototype.indexOf.call(p.childNodes, node) + step; i >= 0 && i < p.childNodes.length; i += step) {
    if (p.childNodes[i]!.kind === 1) return p.childNodes[i] as MirrorElement;
  }
  return null;
}
defineDomGet(
  "nextElementSibling",
  function (this: MirrorNode) {
    return elementSibling(this, 1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "previousElementSibling",
  function (this: MirrorNode) {
    return elementSibling(this, -1);
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "textContent",
  function (this: MirrorElement) {
    let out = "";
    const walk = (n: MirrorNode) => {
      if (n.kind === 2) out += n.text ?? "";
      else if (n.kind === 1 && n.ownText) out += n.ownText;
      for (const c of n.childNodes || []) walk(c);
    };
    walk(this);
    return out;
  },
  function (this: MirrorElement, value: unknown) {
    // The engine's detached elements share this prototype and have no tree:
    // the text is theirs to keep (a `<style>` from it is read on insertion).
    if (!isMirrorNode(this)) return ownExpando(this, "textContent", value == null ? "" : String(value));
    nodeOps.setElementText(this, value == null ? "" : String(value));
  } as (value: never) => void,
);

defineDom("matches", function (this: MirrorElement, selector: string) {
  return hostQuery(selector, this.__vnId, 1).length > 0;
});
defineDom("closest", function (this: MirrorElement, selector: string) {
  return queryOne(selector, this.__vnId, 2);
});
defineDom("querySelector", function (this: MirrorElement, selector: string) {
  return queryOne(selector, this.__vnId, 0);
});
defineDom("querySelectorAll", function (this: MirrorElement, selector: string) {
  return staticList(NodeListProto, hostQuery(selector, this.__vnId, 0));
});

// Live collections walk the mirror tree, not the selector matcher: `foo:bar` is a tag name here and an invalid selector there.
function elementsUnder(
  root: MirrorNode | null,
  inclusive: boolean,
  test: (el: MirrorElement) => boolean,
): MirrorElement[] {
  const out: MirrorElement[] = [];
  if (!root) return out;
  const walk = (n: MirrorNode) => {
    const kids = n.childNodes;
    for (let i = 0; i < kids.length; i++) {
      const c = kids[i]!;
      if (c.kind !== 1) continue;
      if (test(c)) out.push(c);
      walk(c);
    }
  };
  if (inclusive && root.kind === 1 && test(root)) out.push(root);
  walk(root);
  return out;
}
function tagTest(qualifiedName: unknown): (el: MirrorElement) => boolean {
  const q = String(qualifiedName);
  if (q === "*") return () => true;
  const lower = q.toLowerCase();
  return (el) => (el._ns ? el.tag === q : el.tag.toLowerCase() === lower);
}
function classTest(classNames: unknown): (el: MirrorElement) => boolean {
  const want = String(classNames)
    .split(/[ \t\n\f\r]+/)
    .filter(Boolean);
  if (!want.length) return () => false;
  return (el) => {
    const have = el.attrs.class;
    if (!have) return false;
    const tokens = have.split(/[ \t\n\f\r]+/);
    return want.every((w) => tokens.includes(w));
  };
}
defineDom("getElementsByTagName", function (this: MirrorElement, qualifiedName: unknown) {
  const test = tagTest(qualifiedName);
  return liveCollection(HTMLCollectionProto, true, () => elementsUnder(this, false, test));
});
defineDom("getElementsByClassName", function (this: MirrorElement, classNames: unknown) {
  const test = classTest(classNames);
  return liveCollection(HTMLCollectionProto, true, () => elementsUnder(this, false, test));
});

// A mirror's namespace is `_ns`; any other element on this prototype (the engine's detached ones) is HTML.
defineDomGet("namespaceURI", function (this: MirrorElement) {
  return this._ns === "svg" ? SVG_NS : this._ns === "mathml" ? MATHML_NS : HTML_NS;
});
defineDomGet("localName", function (this: MirrorElement) {
  const name = typeof this.tag === "string" ? this.tag : String(this.tagName ?? "").toLowerCase();
  return this._ns ? name.slice(name.indexOf(":") + 1) : name.toLowerCase();
});
defineDomGet("prefix", function (this: MirrorElement) {
  return null;
});

// Bounds come from the last painted frame. The rect methods answer where the box was DRAWN (transforms applied); the size accessors answer the layout box.
defineDom("getBoundingClientRect", function (this: MirrorElement) {
  return asDomRect(paintedRectOf(this, "getBoundingClientRect()"));
});
defineDom("getClientRects", function (this: MirrorElement) {
  const r = paintedRectOf(this, "getClientRects()");
  return staticList(DOMRectListProto, r.width || r.height ? [asDomRect(r)] : []);
});
for (const [name, pick] of [
  ["offsetWidth", (r: MirrorRect) => r.width],
  ["offsetHeight", (r: MirrorRect) => r.height],
  ["clientWidth", (r: MirrorRect) => r.width],
  ["clientHeight", (r: MirrorRect) => r.height],
] as [string, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    return Math.round(pick(rectOf(this, name)));
  });
}
// `scrollWidth` / `scrollHeight` are the CONTENT box: `el.scrollTop = el.scrollHeight` is wrong by the whole overflow if this answers the viewport.
for (const [name, pick, fallback] of [
  ["scrollWidth", (s: ScrollPos) => s.scrollWidth, (r: MirrorRect) => r.width],
  ["scrollHeight", (s: ScrollPos) => s.scrollHeight, (r: MirrorRect) => r.height],
] as [string, (s: ScrollPos) => number, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    const s = hostScroll(this, 0, NaN, NaN, true);
    return Math.round(s ? pick(s) : fallback(rectOf(this, name)));
  });
}
// `offset*` is measured from the offsetParent in the DOM; with no positioned-ancestor chain these are window-relative.
for (const [name, pick] of [
  ["offsetTop", (r: MirrorRect) => r.top],
  ["offsetLeft", (r: MirrorRect) => r.left],
] as [string, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    warnCompat(
      "offsetParent",
      "offsetTop/offsetLeft are measured from the WINDOW, not from an offsetParent — " +
        "vue-native has no positioned-ancestor chain; use getBoundingClientRect(), which is exact",
    );
    return Math.round(pick(rectOf(this, name)));
  });
}
defineDomGet("offsetParent", () => null);
// Border widths are not published (the painted rect is the border box); 0, because @floating-ui adds these to every clipping rect and `undefined` makes it NaN.
defineDomGet("clientTop", () => 0);
defineDomGet("clientLeft", () => 0);

// Programmatic scrolling drives the host's real `ScrollHandle`. A write is CLAMPED and the
// getter answers the clamped value synchronously; the `scroll` EVENT is asynchronous, as in CSSOM-View.

interface ScrollPos {
  left: number;
  top: number;
  scrollWidth: number;
  scrollHeight: number;
  stale: boolean;
}

/** One host call for every scroll accessor; `null` when not a scroll container. `sizeMatters` enables the `noteStale` advisory. */
function hostScroll(node: MirrorElement, op: 0 | 1 | 2, x = NaN, y = NaN, sizeMatters = false): ScrollPos | null {
  if (!node.__vnId) return null;
  const raw = __vn_scroll(node.__vnId, op, x, y);
  if (!raw) return null;
  const [left = NaN, top = NaN, scrollWidth = NaN, scrollHeight = NaN, stale] = raw.split(",").map(Number);
  if (stale && sizeMatters) noteStale();
  return { left, top, scrollWidth, scrollHeight, stale: !!stale };
}

/**
 * The host forces layout on read wherever a draw is legal, but not inside handlers gpui calls directly (keydown/keyup,
 * mousedown, wheel, clipboard, focus/blur: `vn_tree::in_input_dispatch`). There a content-size read in the same JS
 * wave as a DOM change answers the last painted frame.
 */
function noteStale(): void {
  warnCompat(
    "scroll-layout",
    "scrollWidth/scrollHeight (and the clamp on a scroll) answered from the LAST PAINTED frame: " +
      "the DOM changed in this same wave, and a layout cannot be forced inside a keydown/keyup " +
      "(or the click Enter/Space makes), mousedown, wheel, clipboard or focus/blur handler here " +
      "(it can everywhere else: timers, rAF, fetch callbacks, mouseup/click, input). " +
      "`el.scrollTop = el.scrollHeight` right after adding content from such a handler therefore " +
      "stops one item short — do it inside requestAnimationFrame(), which runs after the frame " +
      "that laid the new content out",
  );
}

/** `behavior: "smooth"` is performed instantly, with an advisory: a running CSS animation switches the retained-render cache off document-wide. */
function noteBehavior(behavior: unknown): void {
  if (behavior !== "smooth") return;
  warnCompat(
    "scroll-behavior",
    'scroll behavior: "smooth" is performed INSTANTLY — the final position is exact and readable ' +
      "on the next line, but there is no interpolation and no `scrollend` event; " +
      'use behavior: "auto" (or "instant") to say so explicitly',
  );
}

function scrollArgs(a: unknown, b: unknown): { left: number; top: number } {
  if (a !== null && typeof a === "object") {
    const o = a as { left?: unknown; top?: unknown; behavior?: unknown };
    noteBehavior(o.behavior);
    return { left: o.left === undefined ? NaN : Number(o.left), top: o.top === undefined ? NaN : Number(o.top) };
  }
  return { left: a === undefined ? NaN : Number(a), top: b === undefined ? NaN : Number(b) };
}

for (const [name, axis] of [
  ["scrollLeft", "left"],
  ["scrollTop", "top"],
] as [string, "left" | "top"][]) {
  defineDomGet(
    name,
    function (this: MirrorElement) {
      const s = hostScroll(this, 0);
      return s ? s[axis] : 0;
    },
    function (this: MirrorElement, value: never) {
      const v = Number(value);
      if (!Number.isFinite(v)) return;
      // The write is clamped against the content size, so a stale size lands stale.
      hostScroll(this, 1, axis === "left" ? v : NaN, axis === "top" ? v : NaN, true);
    },
  );
}

const scrollToImpl = function (this: MirrorElement, a: unknown, b: unknown) {
  const { left, top } = scrollArgs(a, b);
  hostScroll(this, 1, left, top, true);
};
defineDom("scrollTo", scrollToImpl);
defineDom("scroll", scrollToImpl);
defineDom("scrollBy", function (this: MirrorElement, a: unknown, b: unknown) {
  const { left, top } = scrollArgs(a, b);
  hostScroll(this, 2, Number.isFinite(left) ? left : 0, Number.isFinite(top) ? top : 0, true);
});

const SCROLL_ALIGN: Record<string, VnScrollAlign> = {
  start: 0,
  center: 1,
  end: 2,
  nearest: 3,
};

defineDom("scrollIntoView", function (this: MirrorElement, arg: unknown) {
  if (!this.__vnId) return;
  let block: VnScrollAlign = 0;
  let inline: VnScrollAlign = 3;
  if (arg === false) block = 2;
  else if (arg !== null && typeof arg === "object") {
    const o = arg as { block?: unknown; inline?: unknown; behavior?: unknown };
    noteBehavior(o.behavior);
    if (o.block !== undefined) block = SCROLL_ALIGN[String(o.block)] ?? 0;
    if (o.inline !== undefined) inline = SCROLL_ALIGN[String(o.inline)] ?? 3;
  }
  // Only horizontal-tb ltr exists: block is vertical, inline horizontal (`direction: rtl` would not flip it).
  const out = __vn_scroll_into_view(this.__vnId, block, inline);
  if (out.charCodeAt(0) === 33 /* ! */) warnCompat("scrollIntoView", out.slice(1));
  else if (out === "stale") noteStale();
});

// `style` is not stored as text: it IS the inline declaration block, so `getAttribute("style")` reads what
// `:style` and `el.style` wrote. `attrs.style` is only a marker for `hasAttribute` / `getAttributeNames`.
const isStyleAttr = (name: unknown): boolean => String(name).toLowerCase() === "style";
defineDom("getAttribute", function (this: MirrorElement, name: unknown) {
  if (isStyleAttr(name)) return styleAttribute(this);
  const v = this.attrs?.[String(name)];
  return v === undefined ? null : v;
});
defineDom("hasAttribute", function (this: MirrorElement, name: unknown) {
  if (isStyleAttr(name)) return !!this._inline?.present;
  return this.attrs ? String(name) in this.attrs : false;
});
defineDom("getAttributeNames", function (this: MirrorElement) {
  return this.attrs ? Object.keys(this.attrs) : [];
});
defineDom("setAttribute", function (this: MirrorElement, name: unknown, value: unknown) {
  const key = String(name);
  if (isStyleAttr(key)) {
    replaceInline(this, String(value), true);
    return;
  }
  writeAttribute(this, key, value == null ? "" : String(value));
});
defineDom("removeAttribute", function (this: MirrorElement, name: unknown) {
  if (isStyleAttr(name)) {
    clearInline(this);
    return;
  }
  writeAttribute(this, String(name), null);
});
defineDomGet(
  "id",
  function (this: MirrorElement) {
    return isMirrorEl(this) ? (this.attrs.id ?? "") : undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "id", v);
    this.setAttribute("id", String(v));
  } as (value: never) => void,
);

// `REFLECTED` properties as accessors over their attributes. An element whose tag lacks the property
// reads `undefined` and keeps an assignment as an expando.
const TAB_INDEX_ZERO = new Set(["a", "area", "button", "frame", "iframe", "input", "object", "select", "textarea"]);
function parseHtmlInt(s: string | undefined): number | null {
  const m = s === undefined ? null : /^[\t\n\f\r ]*([-+]?\d+)/.exec(s);
  if (!m) return null;
  const n = Number(m[1]);
  return n >= -2147483648 && n <= 2147483647 ? n : null;
}
function isDetailsSummary(el: MirrorElement): boolean {
  const p = el.parentNode as MirrorElement | null;
  if (el.tag !== "summary" || !p || p.kind !== 1 || p.tag !== "details") return false;
  return p.childNodes.find((c) => c.kind === 1 && (c as MirrorElement).tag === "summary") === el;
}
function readReflected(el: MirrorElement, prop: string, r: Reflected): unknown {
  const raw = el.attrs[r.attr];
  if (r.kind === "bool") return raw !== undefined;
  if (r.kind === "string") return raw ?? "";
  const n = parseHtmlInt(raw);
  if (prop === "tabIndex") return n ?? (TAB_INDEX_ZERO.has(el.tag) || isDetailsSummary(el) ? 0 : -1);
  return n !== null && n >= 0 ? n : -1;
}
function writeReflected(el: MirrorElement, prop: string, r: Reflected, v: unknown): void {
  if (r.kind === "bool") {
    if (!v) el.removeAttribute(r.attr);
    else if (el.attrs[r.attr] === undefined) el.setAttribute(r.attr, "");
    return;
  }
  if (r.kind === "string") return el.setAttribute(r.attr, String(v));
  const n = toLong(v);
  if (n < 0 && prop !== "tabIndex") {
    throw new DOMException(
      `Failed to set the '${prop}' property: The value provided (${n}) is negative.`,
      "IndexSizeError",
    );
  }
  el.setAttribute(r.attr, String(n));
}
for (const [prop, r] of Object.entries(REFLECTED)) {
  defineDomGet(
    prop,
    function (this: MirrorElement) {
      return isMirrorEl(this) && (!r.tags || r.tags.has(this.tag)) ? readReflected(this, prop, r) : undefined;
    },
    function (this: MirrorElement, v: unknown) {
      if (!isMirrorEl(this) || (r.tags && !r.tags.has(this.tag))) return ownExpando(this, prop, v);
      writeReflected(this, prop, r, v);
    } as (value: never) => void,
  );
}
defineDomGet(
  "hidden",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    const a = this.attrs.hidden;
    return a === undefined ? false : a.toLowerCase() === "until-found" ? "until-found" : true;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "hidden", v);
    if (typeof v === "string" && v.toLowerCase() === "until-found") this.setAttribute("hidden", "until-found");
    else writeReflected(this, "hidden", REFLECTED.hidden!, v);
  } as (value: never) => void,
);
const INPUT_TYPES = new Set(
  (
    "hidden text search tel url email password date month week time datetime-local number range color " +
    "checkbox radio file submit image reset button"
  ).split(" "),
);
const TYPE_STRING_TAGS = new Set(["a", "link", "ol", "li", "source", "embed", "object", "script", "style"]);
// `type`: enumerated on <input> / <button>, read-only and derived on <select> / <textarea>, a plain string elsewhere.
defineDomGet(
  "type",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    const a = this.attrs.type;
    const low = a?.toLowerCase();
    switch (this.tag) {
      case "input":
        return low && INPUT_TYPES.has(low) ? low : "text";
      case "button":
        return low === "reset" || low === "button" ? low : "submit";
      case "select":
        return this.attrs.multiple !== undefined ? "select-multiple" : "select-one";
      case "textarea":
      case "fieldset":
      case "output":
        return this.tag;
      default:
        return TYPE_STRING_TAGS.has(this.tag) ? (a ?? "") : undefined;
    }
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "type", v);
    if (this.tag === "input" || this.tag === "button" || TYPE_STRING_TAGS.has(this.tag))
      this.setAttribute("type", String(v));
    else if (!["select", "textarea", "fieldset", "output"].includes(this.tag)) ownExpando(this, "type", v);
    // (read-only on those four: the assignment is ignored, as in sloppy-mode JS)
  } as (value: never) => void,
);
defineDomGet(
  "dir",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    const low = this.attrs.dir?.toLowerCase();
    return low === "ltr" || low === "rtl" || low === "auto" ? low : "";
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "dir", v);
    this.setAttribute("dir", String(v));
  } as (value: never) => void,
);
defineDom("toggleAttribute", function (this: MirrorElement, name: string, force?: boolean) {
  const has = this.hasAttribute(name);
  const on = force === undefined ? !has : !!force;
  if (on && !has) this.setAttribute(name, "");
  else if (!on && has) this.removeAttribute(name);
  return on;
});

// `el.attributes` is a live `NamedNodeMap` over `attrs`; each attribute is one `Attr` while the element has it
// (removing it detaches the Attr, which keeps its value). Names are kept as written; namespaces come from the
// prefix (`xlink:`, `xml:`, `xmlns`).

const XLINK_NS = "http://www.w3.org/1999/xlink";
const XML_NS = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS = "http://www.w3.org/2000/xmlns/";

function attrParts(name: string): { ns: string | null; prefix: string | null; local: string } {
  if (name === "xmlns") return { ns: XMLNS_NS, prefix: null, local: name };
  const i = name.indexOf(":");
  if (i > 0) {
    const prefix = name.slice(0, i);
    const ns = prefix === "xlink" ? XLINK_NS : prefix === "xml" ? XML_NS : prefix === "xmlns" ? XMLNS_NS : null;
    if (ns) return { ns, prefix, local: name.slice(i + 1) };
  }
  return { ns: null, prefix: null, local: name };
}

function attrNameNS(el: MirrorElement, ns: unknown, local: unknown): string | null {
  const want = ns == null || ns === "" ? null : String(ns);
  const l = String(local);
  for (const name of Object.keys(el.attrs ?? {})) {
    const p = attrParts(name);
    if (p.ns === want && p.local === l) return name;
  }
  return null;
}

class AttrImpl {
  _el: MirrorElement | null = null;
  _name: string;
  _value: string;
  constructor(name: string, value: string) {
    this._name = name;
    this._value = value;
  }
  get name(): string {
    return this._name;
  }
  get nodeName(): string {
    return this._name;
  }
  get localName(): string {
    return attrParts(this._name).local;
  }
  get prefix(): string | null {
    return attrParts(this._name).prefix;
  }
  get namespaceURI(): string | null {
    return attrParts(this._name).ns;
  }
  get ownerElement(): MirrorElement | null {
    return this._el;
  }
  get value(): string {
    if (this._el) this._value = this._el.getAttribute(this._name) ?? this._value;
    return this._value;
  }
  set value(v: string) {
    const s = String(v);
    this._value = s;
    if (this._el) this._el.setAttribute(this._name, s);
  }
  get nodeValue(): string {
    return this.value;
  }
  set nodeValue(v: string) {
    this.value = v == null ? "" : v;
  }
  get textContent(): string {
    return this.value;
  }
  set textContent(v: string) {
    this.value = v == null ? "" : v;
  }
  get nodeType(): number {
    return 2;
  }
  get specified(): boolean {
    return true;
  }
  get isConnected(): boolean {
    return false;
  }
  get parentNode(): null {
    return null;
  }
  get ownerDocument(): unknown {
    return (globalThis as { document?: unknown }).document ?? null;
  }
  cloneNode(): AttrImpl {
    return new AttrImpl(this._name, this.value);
  }
}
Object.defineProperty(AttrImpl, "name", { value: "Attr" });
for (const [k, v] of [
  ["ATTRIBUTE_NODE", 2],
  ["ELEMENT_NODE", 1],
  ["TEXT_NODE", 3],
  ["COMMENT_NODE", 8],
] as const) {
  Object.defineProperty(AttrImpl.prototype, k, { value: v, enumerable: true });
}

const attrNodes = new WeakMap<MirrorElement, Map<string, AttrImpl>>();
let noAttrNodes = true;

function attrNode(el: MirrorElement, name: string): AttrImpl | null {
  if (!el.attrs || !(name in el.attrs)) return null;
  let map = attrNodes.get(el);
  if (!map) attrNodes.set(el, (map = new Map()));
  let a = map.get(name);
  if (!a) {
    noAttrNodes = false;
    a = new AttrImpl(name, "");
    a._el = el;
    map.set(name, a);
  }
  return a;
}

function attrChanged(el: MirrorElement, name: string, oldValue: string | null): void {
  if (noAttrNodes || (el.attrs && name in el.attrs)) return;
  const map = attrNodes.get(el);
  const a = map?.get(name);
  if (!a) return;
  if (oldValue !== null) a._value = oldValue;
  a._el = null;
  map!.delete(name);
}

function setAttrNode(el: MirrorElement, attr: unknown, method: string): AttrImpl | null {
  if (!(attr instanceof AttrImpl)) {
    throw new TypeError(`Failed to execute '${method}' on 'Element': parameter 1 is not of type 'Attr'.`);
  }
  if (attr._el && attr._el !== el) {
    throw domError(
      method,
      "Element",
      "InUseAttributeError",
      "The node provided is an attribute node that is already an attribute of another Element; attribute nodes must be explicitly cloned.",
    );
  }
  if (attr._el === el) return attr;
  const old = attrNode(el, attr._name);
  if (old) old._value = old.value;
  el.setAttribute(attr._name, attr._value);
  if (old) old._el = null;
  noAttrNodes = false;
  let map = attrNodes.get(el);
  if (!map) attrNodes.set(el, (map = new Map()));
  attr._el = el;
  map.set(attr._name, attr);
  return old;
}

function removeAttrNode(el: MirrorElement, attr: unknown, method: string, iface: string): AttrImpl {
  if (!(attr instanceof AttrImpl) || attr._el !== el) {
    throw domError(method, iface, "NotFoundError", "The node provided is owned by another element.");
  }
  attr._value = attr.value;
  el.removeAttribute(attr._name);
  return attr;
}

const namedNodeMaps = new WeakMap<MirrorElement, object>();

class NamedNodeMapImpl {
  declare _el: MirrorElement;
  get length(): number {
    return Object.keys(this._el.attrs ?? {}).length;
  }
  item(i: unknown): AttrImpl | null {
    const n = Number(i) >>> 0;
    const name = Object.keys(this._el.attrs ?? {})[n];
    return name === undefined ? null : attrNode(this._el, name);
  }
  getNamedItem(name: unknown): AttrImpl | null {
    return attrNode(this._el, String(name));
  }
  getNamedItemNS(ns: unknown, local: unknown): AttrImpl | null {
    const name = attrNameNS(this._el, ns, local);
    return name === null ? null : attrNode(this._el, name);
  }
  setNamedItem(attr: unknown): AttrImpl | null {
    return setAttrNode(this._el, attr, "setNamedItem");
  }
  setNamedItemNS(attr: unknown): AttrImpl | null {
    return setAttrNode(this._el, attr, "setNamedItemNS");
  }
  removeNamedItem(name: unknown): AttrImpl {
    const a = attrNode(this._el, String(name));
    if (!a)
      throw domError(
        "removeNamedItem",
        "NamedNodeMap",
        "NotFoundError",
        `No item with name '${String(name)}' was found.`,
      );
    return removeAttrNode(this._el, a, "removeNamedItem", "NamedNodeMap");
  }
  removeNamedItemNS(ns: unknown, local: unknown): AttrImpl {
    const name = attrNameNS(this._el, ns, local);
    if (name === null) {
      throw domError(
        "removeNamedItemNS",
        "NamedNodeMap",
        "NotFoundError",
        `No item with name '${String(ns)}::${String(local)}' was found.`,
      );
    }
    return removeAttrNode(this._el, attrNode(this._el, name), "removeNamedItemNS", "NamedNodeMap");
  }
  *[Symbol.iterator](): IterableIterator<AttrImpl> {
    for (const name of Object.keys(this._el.attrs ?? {})) {
      const a = attrNode(this._el, name);
      if (a) yield a;
    }
  }
}
Object.defineProperty(NamedNodeMapImpl, "name", { value: "NamedNodeMap" });

function namedNodeMapOf(el: MirrorElement): object {
  let m = namedNodeMaps.get(el);
  if (m) return m;
  const target = Object.create(NamedNodeMapImpl.prototype) as NamedNodeMapImpl;
  Object.defineProperty(target, "_el", { value: el });
  const index = (k: PropertyKey): number => (typeof k === "string" && /^(0|[1-9]\d*)$/.test(k) ? Number(k) : -1);
  const named = (k: PropertyKey): boolean => typeof k === "string" && !(k in target) && !!el.attrs && k in el.attrs;
  m = new Proxy(target, {
    get(t, k, receiver) {
      const i = index(k);
      if (i >= 0) return t.item(i) ?? undefined;
      if (named(k)) return attrNode(el, k as string);
      return Reflect.get(t, k, receiver);
    },
    has(t, k) {
      const i = index(k);
      if (i >= 0) return i < t.length;
      return named(k) || Reflect.has(t, k);
    },
    ownKeys(t) {
      return [...Object.keys(el.attrs ?? {}).map((_, i) => String(i)), ...Reflect.ownKeys(t)];
    },
    getOwnPropertyDescriptor(t, k) {
      const i = index(k);
      if (i >= 0)
        return i < t.length ? { value: t.item(i), enumerable: true, configurable: true, writable: false } : undefined;
      return Reflect.getOwnPropertyDescriptor(t, k);
    },
    set(t, k, v, receiver) {
      if (index(k) >= 0 || named(k)) return true;
      return Reflect.set(t, k, v, receiver);
    },
  });
  namedNodeMaps.set(el, m);
  return m;
}

defineDomGet("attributes", function (this: MirrorElement) {
  return isMirrorEl(this) ? namedNodeMapOf(this) : undefined;
});
defineDom("hasAttributes", function (this: MirrorElement) {
  return !!this.attrs && Object.keys(this.attrs).length > 0;
});
defineDom("getAttributeNode", function (this: MirrorElement, name: unknown) {
  return attrNode(this, String(name));
});
defineDom("getAttributeNodeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  const name = attrNameNS(this, ns, local);
  return name === null ? null : attrNode(this, name);
});
defineDom("setAttributeNode", function (this: MirrorElement, attr: unknown) {
  return setAttrNode(this, attr, "setAttributeNode");
});
defineDom("setAttributeNodeNS", function (this: MirrorElement, attr: unknown) {
  return setAttrNode(this, attr, "setAttributeNodeNS");
});
defineDom("removeAttributeNode", function (this: MirrorElement, attr: unknown) {
  return removeAttrNode(this, attr, "removeAttributeNode", "Element");
});
defineDom("getAttributeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  const name = attrNameNS(this, ns, local);
  return name === null ? null : this.getAttribute(name);
});
defineDom("hasAttributeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  return attrNameNS(this, ns, local) !== null;
});
// The qualified name is the key, so `setAttributeNS(XLINK_NS, "xlink:href", v)` and `getAttribute("xlink:href")` meet.
defineDom("setAttributeNS", function (this: MirrorElement, ns: unknown, qualifiedName: unknown, value: unknown) {
  const q = String(qualifiedName);
  const existing = attrNameNS(this, ns, q.slice(q.indexOf(":") + 1));
  this.setAttribute(existing ?? q, value);
});
defineDom("removeAttributeNS", function (this: MirrorElement, ns: unknown, local: unknown) {
  const name = attrNameNS(this, ns, local);
  if (name !== null) this.removeAttribute(name);
});
const camelToData = (k: string): string => "data-" + hyphenate(k);
defineDomGet("dataset", function (this: MirrorElement) {
  const node = this;
  const proxy = new Proxy(Object.create(null) as Record<string, string | undefined>, {
    get: (_t, k) => (typeof k === "string" ? node.attrs?.[camelToData(k)] : undefined),
    has: (_t, k) => typeof k === "string" && camelToData(k) in (node.attrs ?? {}),
    set: (_t, k, v) => {
      node.setAttribute(camelToData(k as string), v);
      return true;
    },
    deleteProperty: (_t, k) => {
      node.removeAttribute(camelToData(k as string));
      return true;
    },
    ownKeys: () =>
      Object.keys(node.attrs ?? {})
        .filter((k) => k.startsWith("data-"))
        .map((k) => k.slice(5).replace(/-([a-z])/g, (_, c: string) => c.toUpperCase())),
    getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
  });
  Object.defineProperty(this, "dataset", { value: branded("DOMStringMap", proxy), configurable: true });
  return proxy;
});

defineDomGet(
  "className",
  function (this: MirrorElement) {
    return isMirrorEl(this) ? (this.attrs.class ?? "") : undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "className", v);
    setClassText(this, String(v));
  } as (value: never) => void,
);
defineDomGet("classList", function (this: MirrorElement) {
  const node = this;
  const parts = () => (node.attrs.class ?? "").split(/\s+/).filter(Boolean);
  const commit = (list: string[]) => setClassText(node, list.join(" "));
  const list: MirrorClassList = {
    add(...names: string[]) {
      const cur = parts();
      for (const n of names) if (n && !cur.includes(n)) cur.push(n);
      commit(cur);
    },
    remove(...names: string[]) {
      commit(parts().filter((c) => !names.includes(c)));
    },
    toggle(name: string, force?: boolean) {
      const on = force === undefined ? !parts().includes(name) : !!force;
      if (on) list.add(name);
      else list.remove(name);
      return on;
    },
    contains: (name: string) => parts().includes(name),
    get length() {
      return parts().length;
    },
    item: (i: number) => parts()[i] ?? null,
    toString: () => node.className,
    [Symbol.iterator]: () => parts()[Symbol.iterator](),
  };
  Object.defineProperty(this, "classList", { value: branded("DOMTokenList", list), configurable: true });
  return list;
});

// `el.style` is a CSSStyleDeclaration over `InlineStyleState`; writes take the same host call as `:style`.

/** `style.backgroundColor = v`: `null` removes; `undefined` and a value carrying `!important` are ignored. `style["--x"] = v` sets the custom property (an expando in a browser). */
function assignInlineProperty(node: MirrorElement, key: string, v: unknown): void {
  if (v === undefined) return;
  const name = cssPropName(key);
  const custom = name.startsWith("--");
  if (custom) {
    warnCompat(
      "style-custom-assign",
      `el.style["${name}"] = … sets the custom property here, but in a browser it is only an expando on ` +
        `the declaration object and changes nothing; use el.style.setProperty("${name}", …), which works in both`,
    );
  }
  const value = v === null ? "" : String(v);
  if (value === "") {
    removeInlineDecl(node, name);
    return;
  }
  if (!custom && IMPORTANT_RE.test(value)) return;
  setInlineDecl(node, name, inlineValue(value, custom), false);
}

defineDomGet("style", function (this: MirrorElement) {
  const node = this;
  const decls = () => node._inline?.decls;
  // The target carries the real members; the handler routes the rest to the declarations (the `as MirrorStyle` cast joins the halves).
  const target = {
    setProperty(k: unknown, v: unknown, priority?: unknown) {
      const name = cssomName(String(k));
      const custom = name.startsWith("--");
      if (v === undefined && !custom) return;
      const value = v === null ? "" : String(v);
      if (value === "") {
        removeInlineDecl(node, name);
        return;
      }
      const p = priority == null ? "" : String(priority);
      if (p !== "" && p.toLowerCase() !== "important") return;
      if (!custom && IMPORTANT_RE.test(value)) return;
      setInlineDecl(node, name, inlineValue(value, custom), p !== "");
    },
    removeProperty(k: unknown) {
      return removeInlineDecl(node, cssomName(String(k))) ?? "";
    },
    getPropertyValue(k: unknown) {
      return decls()?.get(cssomName(String(k)))?.value ?? "";
    },
    getPropertyPriority(k: unknown) {
      return decls()?.get(cssomName(String(k)))?.important ? "important" : "";
    },
    item(i: unknown) {
      return [...(decls()?.keys() ?? [])][Number(i)] ?? "";
    },
    get length() {
      return decls()?.size ?? 0;
    },
    get cssText() {
      const text = serializeInline(node._inline);
      if (text) noteInlineSerialization();
      return text;
    },
    set cssText(v: unknown) {
      replaceInline(node, v == null ? "" : String(v), false);
    },
    get cssFloat() {
      return decls()?.get("float")?.value ?? "";
    },
    set cssFloat(v: unknown) {
      assignInlineProperty(node, "float", v);
    },
    parentRule: null,
    [Symbol.iterator]() {
      return [...(decls()?.keys() ?? [])][Symbol.iterator]();
    },
  };
  const style = new Proxy(target as unknown as MirrorStyle, {
    get: (t, k) => {
      if (typeof k === "symbol" || k in t) return Reflect.get(t, k);
      if (/^\d+$/.test(k)) {
        const n = Number(k);
        return n < (decls()?.size ?? 0) ? t.item(n) : undefined;
      }
      return decls()?.get(cssPropName(k))?.value ?? "";
    },
    set: (t, k, v) => {
      if (typeof k === "symbol" || k in t) return Reflect.set(t, k, v);
      if (/^\d+$/.test(k)) return true; // the indices are read-only
      assignInlineProperty(node, k, v);
      return true;
    },
  });
  Object.defineProperty(this, "style", { value: style, configurable: true });
  styleDecls.add(style);
  return style;
});

// dispatchEvent runs a component's own events through the capture/bubble walk of host events (reka-ui's `select.select`, `menu.itemSelect`).
defineDom(
  "dispatchEvent",
  function (this: MirrorNode, ev: VnEvent) {
    if (!ev || typeof ev.type !== "string") return true;
    const name = canonicalEvent(ev.type);
    const proceed = propagate(this, name, ev);
    // DOM activation step: an uncancelled dispatched click acts like a real one (FileSaver.js; `el.click()` is built on it).
    if (proceed && name === "click" && isActivationEvent(ev)) activate(this, !!ev.bubbles);
    return proceed;
  },
  NODE_MEMBERS,
);

// There is no document to navigate, so a link hands its URL to the system. Links are reachable by
// CONTENT, so the scheme allowlist (http, https, mailto) is the security line; other schemes are
// refused with an advisory naming `appShell.openExternal`, the call for URLs the app trusts.

const CONTENT_SCHEMES = new Set(["http", "https", "mailto"]);
const SCHEME_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*):/;

function isLink(n: MirrorNode): n is MirrorElement {
  return n.kind === 1 && (n.tag === "a" || n.tag === "area") && n.attrs != null && n.attrs.href != null;
}

const ACTIVATABLE = new Set(["button", "input", "label", "summary"]);
const hasActivationBehaviour = (n: MirrorNode): n is MirrorElement =>
  n.kind === 1 && (ACTIVATABLE.has(n.tag) || isLink(n));

/** DOM "dispatch" step 5.5: only a `MouseEvent` click activates; an engine without MouseEvent (unit tests) counts any click. */
function isActivationEvent(ev: VnEvent): boolean {
  return typeof MouseEvent !== "function" || ev instanceof MouseEvent;
}

function activate(target: MirrorNode, bubbles: boolean): void {
  // Detached `document.createElement("a" | "input")` elements are not in the tree; FileSaver.js and `useFileDialog` click them.
  if (!isMirrorNode(target)) return activateDetached(target);
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (hasActivationBehaviour(n)) {
      if (isFileInput(n)) showFilePicker(fileInputOps(n), null);
      else if (isColorInput(n)) showColorPicker(n, null);
      else if (isLabel(n)) activateLabel(n, target);
      else if (isLink(n)) followLink(n);
      else if (isSubmitButton(n) || isResetButton(n)) activateFormButton(n);
      return;
    }
    if (!bubbles) return;
  }
}

function followLink(a: MirrorElement): void {
  followHref(String(a.attrs.href ?? ""), a.attrs.download ?? null);
}

function followHref(href: string, download: string | null): void {
  const raw = href.trim();
  if (raw === "" || raw.startsWith("#")) return;
  const m = SCHEME_RE.exec(raw);
  if (!m) {
    warnCompat(
      "link:relative",
      `<a href="${raw}"> — a relative link has no document to navigate in vue-native, so ` +
        `activating it does nothing. In-app links belong to vue-router's <RouterLink> (which ` +
        `handles the click itself); an external one needs an absolute https: URL`,
    );
    return;
  }
  const scheme = m[1]!.toLowerCase();
  if (scheme === "javascript") {
    // `javascript:void(0)` is the "link with only a click handler" idiom; anything else asked for code to run.
    if (!/^javascript:\s*(void\s*\(?\s*0\s*\)?\s*;?\s*|;?)$/i.test(raw)) {
      warnCompat(
        "link:javascript",
        `<a href="javascript:…"> — javascript: URLs are not executed (CSP-style); put the code ` +
          `in a @click handler`,
      );
    }
    return;
  }
  if (download != null && (scheme === "blob" || scheme === "data")) {
    // Resolved now, at the click, so `a.click(); URL.revokeObjectURL(url)` still saves.
    desktopApi.download(raw, download);
    return;
  }
  if (!CONTENT_SCHEMES.has(scheme)) {
    adviseScheme(scheme, "<a href>");
    return;
  }
  openContentUrl(raw, "<a href>");
}

function adviseScheme(scheme: string, via: string): void {
  warnCompat(
    `link-scheme:${scheme}`,
    `${via} with a ${scheme}: URL was refused: links and window.open() only hand http, https ` +
      `and mailto to the system, because content can reach them. If the app trusts this URL, ` +
      `open it from your own code with appShell.openExternal(url)`,
  );
}

// A file input's click opens the picker via desktop.ts; its `files`, `value`, `accept`, `multiple` and `webkitdirectory`
// are own accessors installed when `type` becomes "file". Detached `document.createElement("input")` (VueUse's
// `useFileDialog`) gets the same.

function fileStatus(files: ArrayLike<{ name?: unknown }> | null): string {
  const n = files?.length ?? 0;
  if (n === 0) return "No file chosen";
  return n === 1 ? String(files![0]?.name ?? "") : `${n} files`;
}

function isFileInput(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return (
    !!e &&
    e.kind === 1 &&
    e.tag === "input" &&
    String(e.attrs?.type ?? "")
      .trim()
      .toLowerCase() === "file"
  );
}

function isColorInput(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return (
    !!e &&
    e.kind === 1 &&
    e.tag === "input" &&
    String(e.attrs?.type ?? "")
      .trim()
      .toLowerCase() === "color"
  );
}

function colorValue(value: unknown): string {
  const text = String(value ?? "")
    .trim()
    .toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(text)) return text;
  if (/^#[0-9a-f]{3,4}$/.test(text)) return "#" + [...text.slice(1, 4)].map((c) => c + c).join("");
  if (/^#[0-9a-f]{8}$/.test(text)) return text.slice(0, 7);
  if (text)
    warnCompat(
      "input-color:value",
      "<input type=color> currently accepts hex colors (#rgb, #rgba, #rrggbb, #rrggbbaa); other CSS color values normalize to #000000",
    );
  return "#000000";
}

// A picker is asynchronous. An already-open picker cannot be replaced by a
// second click, and its result belongs to the original control.
const colorPickers = new WeakSet<object>();
function showColorPicker(el: MirrorElement | Detached, via: "showPicker" | null): void {
  const mirror = isMirrorNode(el);
  if (mirror ? isDisabledControl(el as MirrorElement) : !!el.disabled) return;
  if (colorPickers.has(el)) return;
  if (via === null && !activation.isActive()) {
    console.warn("Color chooser dialog can only be shown with a user activation.");
    return;
  }
  const before = colorValue(el.value);
  const id = mirror ? (el as MirrorElement).__vnId : 0;
  activation.consume();
  colorPickers.add(el);
  void desktopApi
    .pickColorForInput(before)
    .then((selected) => {
      if (selected == null) return;
      const next = colorValue(selected);
      if (mirror) {
        const control = el as MirrorElement;
        if (control.__vnId !== id || !isColorInput(control) || next === colorValue(control.value)) return;
        writeControlValue(control, next);
        propagate(control, "input", fileEvent("input"));
        propagate(control, "change", fileEvent("change"));
      } else {
        if (String(el.type).trim().toLowerCase() !== "color" || next === colorValue(el.value)) return;
        el.value = next;
        fireDetached(el, "input");
        fireDetached(el, "change");
      }
    })
    .finally(() => colorPickers.delete(el));
}

/** Is this one of the host's text widgets? An `<input>` other than file/range/color, or a `<textarea>` (`vn-tree` `build_node`). */
function isTextWidget(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return (
    !!e &&
    e.kind === 1 &&
    (e.tag === "textarea" || (e.tag === "input" && !isFileInput(e) && !isRangeInput(e) && !isColorInput(e)))
  );
}

/** Select a field through its native editor, so the next keystroke, IME commit or clipboard operation does not replace a JS-only range. */
defineDom("select", function (this: MirrorElement) {
  if (!isMirrorEl(this) || (this.tag !== "input" && this.tag !== "textarea")) throw new TypeError("Illegal invocation");
  if (activeElement() !== this) this.focus();
  if (!isTextWidget(this) || !__vn_input_select(this.__vnId)) return;
  // HTML queues a bubbling, non-cancelable select event. It is not an edit:
  // no input/change event or value assignment is manufactured here.
  queueMicrotask(() => {
    if (this.__vnId) propagate(this, "select", synthesize("select", 0, 0, 0, 0, undefined));
  });
});

function isLabel(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "label";
}

function isFileList(v: unknown): boolean {
  const FL = (globalThis as { FileList?: unknown }).FileList;
  if (typeof FL === "function") return v instanceof (FL as new () => object);
  return (
    !!v &&
    typeof v === "object" &&
    typeof (v as { length?: unknown }).length === "number" &&
    typeof (v as { item?: unknown }).item === "function"
  );
}

function emptyFileList(): unknown {
  const DT = (globalThis as { DataTransfer?: new () => { files: unknown } }).DataTransfer;
  if (typeof DT === "function") {
    try {
      return new DT().files;
    } catch {
      /* fall through to the array-like below */
    }
  }
  return Object.freeze(Object.assign([], { item: () => null }));
}

interface FileState {
  files: unknown;
  changed: () => void;
}
const fileStates = new WeakMap<object, FileState>();

const FAKEPATH = "C:\\fakepath\\";
const VALUE_REFUSED =
  "Failed to set the 'value' property on 'HTMLInputElement': This input element accepts a filename, " +
  "which may only be programmatically set to the empty string.";
const FILES_REFUSED =
  "Failed to set the 'files' property on 'HTMLInputElement': The provided value is not of type 'FileList'.";

/** HTML's `files` and filename-mode `value` over one state: `files` is always a FileList (setting it fires nothing); `value` is `C:\fakepath\<name>`, "" clears, anything else throws. */
function fileAccessors(state: FileState, isFile: () => boolean, other?: { value: string }) {
  const list = () => (state.files ??= emptyFileList()) as ArrayLike<{ name?: unknown }>;
  return {
    files: {
      get: () => (isFile() ? list() : null),
      set: (v: unknown) => {
        if (!isFile() || v == null) return;
        if (!isFileList(v)) throw new TypeError(FILES_REFUSED);
        state.files = v;
        state.changed();
      },
      configurable: true,
      enumerable: true,
    },
    value: {
      get: () => {
        if (!isFile()) return other?.value ?? "";
        const f = list();
        return f.length ? FAKEPATH + String(f[0]?.name ?? "") : "";
      },
      set: (v: unknown) => {
        if (!isFile()) {
          if (other) other.value = v == null ? "" : String(v);
          return;
        }
        if (v != null && String(v) !== "") throw new DOMException(VALUE_REFUSED, "InvalidStateError");
        state.files = emptyFileList();
        state.changed();
      },
      configurable: true,
      enumerable: true,
    },
  };
}

let advisedFileLook = false;

function syncFileInput(el: MirrorElement): void {
  const file = isFileInput(el);
  const had = fileStates.has(el);
  if (file && !had) {
    const state: FileState = { files: null, changed: () => {} };
    state.changed = () =>
      __vn_set_prop(el.__vnId, I(":file-label"), fileStatus(state.files as ArrayLike<{ name?: unknown }> | null));
    fileStates.set(el, state);
    const acc = fileAccessors(state, () => isFileInput(el));
    const flag = (k: string) => ({
      get: () => el.attrs[k] != null && el.attrs[k] !== "false",
      set: (v: unknown) => patchProp(el, k, el.attrs[k], v ? "" : null),
      configurable: true,
      enumerable: true,
    });
    Object.defineProperties(el, {
      files: acc.files,
      value: acc.value,
      accept: {
        get: () => el.attrs.accept ?? "",
        set: (v: unknown) => patchProp(el, "accept", el.attrs.accept, String(v)),
        configurable: true,
        enumerable: true,
      },
      multiple: flag("multiple"),
      webkitdirectory: flag("webkitdirectory"),
    });
    state.changed();
    if (!advisedFileLook) {
      advisedFileLook = true;
      warnCompat(
        "input-file:look",
        "<input type=file>: the button and status are an approximation of Chromium's, with its " +
          'English strings ("Choose File", "No file chosen"); restyle it with ::file-selector-button',
      );
    }
  } else if (!file && had) {
    fileStates.delete(el);
    for (const k of ["files", "value", "accept", "multiple", "webkitdirectory"])
      delete (el as unknown as Record<string, unknown>)[k];
    el._value = "";
    __vn_set_prop(el.__vnId, I(":file-label"), undefined);
  }
  for (const n of ["click", "keydown", "keyup"]) syncListenerFlag(el, n);
}

interface FileInputOps {
  accept: string;
  multiple: boolean;
  directory: boolean;
  disabled: boolean;
  current: unknown;
  setFiles(list: unknown): void;
  fire(type: "input" | "change" | "cancel"): void;
}

const fileEvent = (type: string): VnEvent => new Event(type, { bubbles: true, composed: type === "input" }) as VnEvent;

function fileInputOps(el: MirrorElement): FileInputOps {
  const flag = (k: string) => el.attrs[k] != null && el.attrs[k] !== "false";
  if (!fileStates.has(el)) syncFileInput(el);
  const state = fileStates.get(el)!;
  return {
    accept: el.attrs.accept ?? "",
    multiple: flag("multiple"),
    directory: flag("webkitdirectory"),
    disabled: flag("disabled"),
    current: state.files,
    setFiles: (list) => {
      state.files = list;
      state.changed();
    },
    fire: (type) => void propagate(el, type, fileEvent(type)),
  };
}

/** HTML "show the picker": a click without transient activation only warns, in Chromium's words. Picked: `files` changes, then `input` and `change`; dismissed or same selection: `cancel`. */
function showFilePicker(ops: FileInputOps, via: "showPicker" | null): void {
  if (ops.disabled) return;
  if (via === null && !activation.isActive()) {
    console.warn("File chooser dialog can only be shown with a user activation.");
    return;
  }
  void desktopApi
    .pickForInput({ accept: ops.accept, multiple: ops.multiple, directory: ops.directory, current: ops.current })
    .then((list) => {
      if (list === undefined) return;
      if (list === null) return ops.fire("cancel");
      ops.setFiles(list);
      ops.fire("input");
      ops.fire("change");
    });
}

const LABELABLE = new Set(["button", "meter", "output", "progress", "select", "textarea"]);
const isLabelable = (n: MirrorNode): n is MirrorElement =>
  n.kind === 1 &&
  (LABELABLE.has(n.tag) ||
    (n.tag === "input" &&
      String(n.attrs.type ?? "")
        .trim()
        .toLowerCase() !== "hidden"));

function firstInTree(root: MirrorNode, pred: (n: MirrorElement) => boolean): MirrorElement | null {
  if (root.kind === 1 && pred(root)) return root;
  for (const c of root.childNodes ?? []) {
    const hit = firstInTree(c, pred);
    if (hit) return hit;
  }
  return null;
}

/** A label's control: the element its `for` names if labelable, else its first labelable descendant. */
function labelControl(label: MirrorElement): MirrorElement | null {
  const forId = label.attrs.for;
  if (forId != null) {
    let root: MirrorNode = label;
    while (root.parentNode) root = root.parentNode;
    const found = firstInTree(root, (n) => n.attrs?.id === forId);
    return found && isLabelable(found) ? found : null;
  }
  return firstInTree(label, (n) => n !== label && isLabelable(n));
}

const FOCUS_ONLY_TYPES = new Set(["button", "submit", "reset", "image", "file", "checkbox", "radio", "range", "color"]);

function activateLabel(label: MirrorElement, target: MirrorNode): void {
  const control = labelControl(label);
  if (!control) return;
  for (let n: MirrorNode | null = target; n; n = n.parentNode) if (n === control) return;
  const textual =
    control.tag === "textarea" ||
    control.tag === "select" ||
    (control.tag === "input" &&
      !FOCUS_ONLY_TYPES.has(
        String(control.attrs.type ?? "text")
          .trim()
          .toLowerCase(),
      ));
  if (textual) control.focus();
  control.click();
}

function adviseJsSourced(el: MirrorElement, name: string, via: string): void {
  const key = `event:${name}`;
  if (warned.has(key)) return;
  queueMicrotask(() => {
    if (
      warned.has(key) ||
      firstInTree(
        el,
        (n) =>
          isFileInput(n) ||
          (name === "change" &&
            (isSelectEl(n) ||
              isColorInput(n) ||
              (n.tag === "input" && String(n.attrs.type).toLowerCase() === "range"))),
      )
    )
      return;
    warnCompat(
      key,
      `${via} — this event type is not delivered by the native host yet: only <input type=file> ` +
        `(and <select>, <input type=range/color>, for \`change\`) fires it here, so on this element it will never fire`,
    );
  });
}

// Detached `document.createElement("a" | "input")` are plain objects on HTMLElement.prototype with no-op
// attribute methods; `wrapDetached` gives them a reflected attribute map and an input the file accessors.
// Their events go through the engine's own EventTarget.

const REFLECTED_BOOL = new Set(["multiple", "webkitdirectory", "disabled"]);
const REFLECTED_STR = new Set(["href", "download", "type", "accept", "rel", "target"]);
const nativeDispatch =
  typeof EventTarget === "function"
    ? (EventTarget.prototype.dispatchEvent as (this: object, ev: VnEvent) => boolean)
    : null;

type Detached = Record<string, unknown>;

function wrapDetached(el: Detached, tag: string): void {
  const attrs: Record<string, string> = Object.create(null);
  const define = (name: string, value: unknown) => {
    try {
      Object.defineProperty(el, name, { value, configurable: true, writable: true });
    } catch {
      /* a non-configurable engine slot keeps the engine's method */
    }
  };
  define("setAttribute", (name: unknown, value: unknown) => {
    const k = String(name).toLowerCase();
    const v = value == null ? "" : String(value);
    attrs[k] = v;
    if (REFLECTED_BOOL.has(k)) el[k] = true;
    else if (REFLECTED_STR.has(k)) el[k] = v;
  });
  define("getAttribute", (name: unknown) => {
    const k = String(name).toLowerCase();
    if (REFLECTED_BOOL.has(k)) return el[k] ? (attrs[k] ?? "") : null;
    if (REFLECTED_STR.has(k) && el[k] != null) return String(el[k]);
    return attrs[k] ?? null;
  });
  define("hasAttribute", (name: unknown) => (el.getAttribute as (n: unknown) => string | null)(name) !== null);
  define("removeAttribute", (name: unknown) => {
    const k = String(name).toLowerCase();
    delete attrs[k];
    if (REFLECTED_BOOL.has(k)) el[k] = false;
    else if (REFLECTED_STR.has(k)) delete el[k];
  });
  if (tag !== "input") return;
  const state: FileState = { files: null, changed: () => {} };
  fileStates.set(el, state);
  const other = { value: String(el.value ?? "") };
  const acc = fileAccessors(
    state,
    () =>
      String(el.type ?? "")
        .trim()
        .toLowerCase() === "file",
    other,
  );
  try {
    Object.defineProperties(el, {
      files: acc.files,
      value: {
        ...acc.value,
        get: () => (String(el.type ?? "").toLowerCase() === "color" ? colorValue(other.value) : acc.value.get()),
        set: (v: unknown) => acc.value.set(String(el.type ?? "").toLowerCase() === "color" ? colorValue(v) : v),
      },
    });
  } catch {
    /* as above */
  }
}

function fireDetached(el: Detached, type: string): void {
  const ev = fileEvent(type);
  try {
    if (!nativeDispatch) throw new TypeError("no EventTarget");
    nativeDispatch.call(el, ev);
  } catch {
    // Not dispatchable: call the handler attribute with a target (VueUse reads `event.target.files`).
    const h = el[`on${type}`];
    Object.defineProperty(ev, "target", { value: el, configurable: true });
    if (typeof h === "function") callListener(h as (this: unknown, ev: VnEvent) => unknown, el, ev);
  }
}

function detachedFileOps(el: Detached): FileInputOps {
  if (!fileStates.has(el)) wrapDetached(el, "input");
  const state = fileStates.get(el)!;
  return {
    accept: String(el.accept ?? ""),
    multiple: !!el.multiple,
    directory: !!el.webkitdirectory,
    disabled: !!el.disabled,
    current: state.files,
    setFiles: (list) => {
      state.files = list;
    },
    fire: (type) => fireDetached(el, type),
  };
}

function activateDetached(target: object): void {
  const el = target as Detached;
  const tag = String(el.tagName ?? "").toUpperCase();
  if (tag === "A" && el.href != null) {
    followHref(String(el.href), el.download == null ? null : String(el.download));
  } else if (
    tag === "INPUT" &&
    String(el.type ?? "")
      .trim()
      .toLowerCase() === "file"
  ) {
    showFilePicker(detachedFileOps(el), null);
  } else if (
    tag === "INPUT" &&
    String(el.type ?? "")
      .trim()
      .toLowerCase() === "color"
  ) {
    showColorPicker(el, null);
  }
}

// showPicker(): Chromium's checks (immutable -> InvalidStateError, no user activation -> NotAllowedError).
defineDom("showPicker", function (this: MirrorElement) {
  const mirror = isMirrorNode(this);
  const raw = this as unknown as Detached;
  const tag = mirror ? this.tag : String(raw.tagName ?? "").toLowerCase();
  if (tag !== "input") {
    throw new TypeError("Failed to execute 'showPicker': this element has no picker (only <input> has one here).");
  }
  const attr = (k: string) => (mirror ? this.attrs[k] != null && this.attrs[k] !== "false" : !!raw[k]);
  const type = String((mirror ? this.attrs.type : raw.type) ?? "text")
    .trim()
    .toLowerCase();
  const fail = (name: string, msg: string) =>
    new DOMException(`Failed to execute 'showPicker' on 'HTMLInputElement': ${msg}`, name);
  if (
    (mirror ? isDisabledControl(this) : attr("disabled")) ||
    (type !== "file" && type !== "color" && attr("readonly"))
  ) {
    throw fail("InvalidStateError", "HTMLInputElement::showPicker() cannot be used on immutable controls.");
  }
  if (!activation.isActive()) throw fail("NotAllowedError", "HTMLInputElement::showPicker() requires a user gesture.");
  if (type === "color") return showColorPicker(mirror ? this : raw, "showPicker");
  if (type !== "file") {
    warnCompat(
      `showPicker:${type}`,
      `showPicker() on <input type=${type}> is a no-op: only file and color pickers exist here`,
    );
    return;
  }
  showFilePicker(mirror ? fileInputOps(this) : detachedFileOps(raw), "showPicker");
});

// el.click(): a MouseEvent through the host-click walk, then activation unless cancelled.
// A disabled control, or a click() inside its own click handler (HTML's "click in progress"), does nothing.
const DISABLEABLE = new Set(["button", "input", "select", "textarea", "fieldset", "optgroup", "option"]);
const clicking = new Set<MirrorElement>();
defineDom("click", function (this: MirrorElement) {
  const mirror: boolean = isMirrorNode(this);
  if (
    mirror
      ? isDisabledControl(this)
      : DISABLEABLE.has(this.tag) && this.attrs.disabled != null && this.attrs.disabled !== "false"
  )
    return;
  if (clicking.has(this)) return;
  clicking.add(this);
  try {
    let ev: VnEvent;
    const init = { bubbles: true, cancelable: true, composed: true, detail: 1 };
    try {
      ev = new MouseEvent("click", init);
    } catch {
      ev = new CustomEvent("click", init);
    }
    if (propagate(this, "click", ev)) activate(this, true);
  } finally {
    clicking.delete(this);
  }
});

// Forms: submission. Divergences, each advised once: no constraint validation, no navigation after an uncancelled
// submit, and reset does not restore control values.

const RUNTIME_FIRED_EVENTS = new Set<string>([
  "submit",
  "reset",
  "fullscreenchange",
  "gotpointercapture",
  "lostpointercapture",
  "select",
  "dragstart",
  "drag",
  "dragenter",
  "dragover",
  "dragleave",
  "drop",
  "dragend",
]);

const hasFlag = (n: MirrorElement, name: string): boolean => n.attrs?.[name] != null && n.attrs[name] !== "false";
const FIELDSET_DISABLES = new Set(["button", "input", "select", "textarea", "fieldset"]);
function isDisabledControl(n: MirrorElement): boolean {
  if (!DISABLEABLE.has(n.tag)) return false;
  if (hasFlag(n, "disabled")) return true;
  if (!FIELDSET_DISABLES.has(n.tag)) return false;
  let child: MirrorNode = n;
  for (let p = n.parentNode; p; child = p, p = p.parentNode) {
    if (p.kind === 1 && p.tag === "fieldset" && hasFlag(p as MirrorElement, "disabled")) {
      const legend = p.childNodes.find((c) => c.kind === 1 && c.tag === "legend");
      if (child !== legend) return true;
    }
  }
  return false;
}
const typeOf = (n: MirrorElement): string =>
  String(n.attrs?.type ?? "")
    .trim()
    .toLowerCase();

function isSubmitButton(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  if (!e || e.kind !== 1) return false;
  if (e.tag === "button") return typeOf(e) !== "button" && typeOf(e) !== "reset";
  return e.tag === "input" && (typeOf(e) === "submit" || typeOf(e) === "image");
}

function isResetButton(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && (e.tag === "button" || e.tag === "input") && typeOf(e) === "reset";
}

const IMPLICIT_SUBMIT_TYPES = new Set([
  "text",
  "search",
  "url",
  "tel",
  "email",
  "password",
  "date",
  "month",
  "week",
  "time",
  "datetime-local",
  "number",
]);
function blocksImplicitSubmission(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "input" && IMPLICIT_SUBMIT_TYPES.has(typeOf(e) || "text");
}

const LISTED = new Set(["button", "fieldset", "input", "object", "output", "select", "textarea"]);

function topOf(n: MirrorNode): MirrorNode {
  while (n.parentNode) n = n.parentNode;
  return n;
}

function formOwner(n: MirrorElement): MirrorElement | null {
  const fid = n.attrs?.form;
  if (fid != null && LISTED.has(n.tag)) {
    const f = firstInTree(topOf(n), (x) => x.attrs?.id === fid);
    return f && f.tag === "form" ? f : null;
  }
  for (let p = n.parentNode; p; p = p.parentNode) if (p.kind === 1 && p.tag === "form") return p as MirrorElement;
  return null;
}

function formControls(form: MirrorElement): MirrorElement[] {
  const out: MirrorElement[] = [];
  forEachDeep(topOf(form), (n) => {
    if (n.kind === 1 && LISTED.has(n.tag) && formOwner(n as MirrorElement) === form) out.push(n as MirrorElement);
  });
  return out;
}

type SubmitEventInit = { bubbles?: boolean; cancelable?: boolean; composed?: boolean; submitter?: unknown };
const SubmitEventClass: (new (type: string, init?: SubmitEventInit) => VnEvent) | null = (() => {
  const g = globalThis as { SubmitEvent?: unknown };
  if (typeof g.SubmitEvent === "function" && !vacant(g, "SubmitEvent"))
    return g.SubmitEvent as new (type: string, init?: SubmitEventInit) => VnEvent;
  if (typeof Event !== "function") return null;
  class SubmitEvent extends Event {
    readonly submitter: unknown;
    constructor(type: string, init: SubmitEventInit = {}) {
      super(type, init);
      this.submitter = init.submitter ?? null;
    }
  }
  try {
    Object.defineProperty(globalThis, "SubmitEvent", { value: mark(SubmitEvent), configurable: true, writable: true });
  } catch {
    /* a frozen global: the class still serves this module */
  }
  return SubmitEvent as unknown as new (type: string, init?: SubmitEventInit) => VnEvent;
})();

type ClipboardEventInit = { bubbles?: boolean; cancelable?: boolean; composed?: boolean; clipboardData?: unknown };
/** The engine's `ClipboardEvent`, or ours, installed on the global when it lacks one; `clipboardData` is a prototype getter, as WebIDL puts a readonly attribute. */
const ClipboardEventClass: (new (type: string, init?: ClipboardEventInit) => VnEvent) | null = (() => {
  const g = globalThis as { ClipboardEvent?: unknown };
  if (typeof g.ClipboardEvent === "function" && !vacant(g, "ClipboardEvent"))
    return g.ClipboardEvent as new (type: string, init?: ClipboardEventInit) => VnEvent;
  if (typeof Event !== "function") return null;
  const data = new WeakMap<object, unknown>();
  class ClipboardEvent extends Event {
    constructor(type: string, init: ClipboardEventInit = {}) {
      super(type, init);
      data.set(this, init.clipboardData ?? null);
    }
    get clipboardData(): unknown {
      return data.has(this) ? data.get(this) : null;
    }
  }
  try {
    Object.defineProperty(globalThis, "ClipboardEvent", {
      value: mark(ClipboardEvent),
      configurable: true,
      writable: true,
    });
  } catch {
    /* a frozen global: the class still serves this module */
  }
  return ClipboardEvent as unknown as new (type: string, init?: ClipboardEventInit) => VnEvent;
})();

function adviseNoNavigation(via: string): void {
  warnCompat(
    "form:navigate",
    `${via}: in a browser an uncancelled form submission navigates to the form's action. vue-native has ` +
      `no document to navigate, so it does nothing. Cancel it (@submit.prevent) and send the data from JS`,
  );
}

function adviseNoValidation(form: MirrorElement): void {
  if (warned.has("form:validation")) return;
  const constrained = formControls(form).some(
    (c) =>
      ["required", "pattern", "min", "max", "minlength", "maxlength"].some((a) => c.attrs?.[a] != null) ||
      (c.tag === "input" && ["email", "url", "number"].includes(typeOf(c))),
  );
  if (!constrained) return;
  warnCompat(
    "form:validation",
    "a <form> without novalidate is submitted WITHOUT constraint validation: required, pattern, " +
      "type=email and the rest are not checked, no validation bubble is shown, and the submit event " +
      "fires anyway. Validate in the submit handler (and add novalidate to say so)",
  );
}

const submitting = new WeakSet<MirrorElement>();

function submitForm(form: MirrorElement, submitter: MirrorElement | null, fromSubmitMethod: boolean): void {
  if (fromSubmitMethod) {
    // `form.submit()` fires no event and skips validation, then navigates.
    adviseNoNavigation("form.submit()");
    return;
  }
  if (submitting.has(form)) return;
  if (!hasFlag(form, "novalidate") && !(submitter && hasFlag(submitter, "formnovalidate"))) adviseNoValidation(form);
  submitting.add(form);
  let proceed = true;
  try {
    const init = { bubbles: true, cancelable: true, submitter };
    const ev = SubmitEventClass
      ? new SubmitEventClass("submit", init)
      : withProps(new CustomEvent("submit", init), { submitter });
    proceed = propagate(form, "submit", ev);
  } finally {
    submitting.delete(form);
  }
  if (proceed) adviseNoNavigation("a submit event nobody cancelled");
}

function resetForm(form: MirrorElement): void {
  const init = { bubbles: true, cancelable: true };
  const ev = typeof Event === "function" ? new Event("reset", init) : new CustomEvent("reset", init);
  if (propagate(form, "reset", ev)) {
    const controls = formControls(form);
    for (const control of controls) {
      if (!isRangeInput(control)) continue;
      control._rangeDirty = false;
      control._value = control.attrs.value ?? "";
      syncRangeValue(control);
    }
    if (!controls.some((c) => !isRangeInput(c) && (c.tag === "input" || c.tag === "textarea" || c.tag === "select")))
      return;
    warnCompat(
      "form:reset",
      "form reset (a reset button or form.reset()) fired `reset` but cannot restore the controls: the " +
        "mirror keeps no default values. Reset the model in the handler",
    );
  }
}

function activateFormButton(n: MirrorElement): void {
  if (isDisabledControl(n)) return;
  const form = formOwner(n);
  if (!form) return;
  if (isSubmitButton(n)) submitForm(form, n, false);
  else if (isResetButton(n)) resetForm(form);
}

function implicitSubmit(field: MirrorElement): void {
  const form = formOwner(field);
  if (!form) return;
  const controls = formControls(form);
  const defaultButton = controls.find(isSubmitButton);
  if (defaultButton) {
    if (!isDisabledControl(defaultButton)) defaultButton.click();
    return;
  }
  if (controls.filter(blocksImplicitSubmission).length > 1) return;
  submitForm(form, null, false);
}

const asForm = (el: MirrorElement, method: string): MirrorElement => {
  if (!isMirrorNode(el) || el.tag !== "form") throw new TypeError(`${method} is not a function`);
  return el;
};
defineDom("requestSubmit", function (this: MirrorElement, submitter?: unknown) {
  const form = asForm(this, "requestSubmit");
  if (submitter != null) {
    if (!isSubmitButton(submitter)) {
      throw new TypeError(
        "Failed to execute 'requestSubmit' on 'HTMLFormElement': The specified element is not a submit button.",
      );
    }
    if (formOwner(submitter) !== form) {
      throw new DOMException(
        "Failed to execute 'requestSubmit' on 'HTMLFormElement': The specified element is not owned by this form element.",
        "NotFoundError",
      );
    }
  }
  submitForm(form, (submitter as MirrorElement | null | undefined) ?? null, false);
});
defineDom("submit", function (this: MirrorElement) {
  submitForm(asForm(this, "submit"), null, true);
});
defineDom("reset", function (this: MirrorElement) {
  resetForm(asForm(this, "reset"));
});

// Focus: the host moves focus (`__vn_focus`); the four events fire here in browser order. A node not yet built is
// queued and its events arrive a frame later (advised). `focus()` never scrolls.
// A handler that moves focus while another move's events run abandons that move, as in Chromium; during
// `blur`/`focusout` nothing is focused (`activeElement` is `body`). reka's FocusScope relies on this: its `focusout`
// handler pulls focus back into a dialog, and the abandoned target must not get `focus`.

let focusedNode: MirrorElement | null = null;

let focusChanges = 0;

/** The node a `focus()` is moving to while the old node hears `blur`/`focusout`: the host has focused it, a browser has not yet. */
let arriving: MirrorElement | null = null;

function fireFocusEvent(target: MirrorElement, name: string, related: MirrorElement | null): void {
  if (!hasListener(target, name)) return;
  propagate(target, name, withRelatedTarget(synthesize(name, 0, 0, 0, 0, undefined), related));
}

function withRelatedTarget(ev: VnEvent, related: MirrorNode | null): VnEvent {
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
  isMirrorNode(el) && el.kind === 1 && nodeById(el.__vnId) === el && !isDisabledControl(el);

defineDom("focus", function (this: MirrorElement) {
  if (!focusable(this)) return;
  if (typeof __vn_focus !== "function") {
    warnCompat("focus", "element.focus() is a no-op: this host has no programmatic focus (__vn_focus)");
    return;
  }
  if (this === arriving) {
    focusMoved(this, null);
    return;
  }
  const answer = __vn_focus(this.__vnId, 0);
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
  if (!isMirrorNode(this) || typeof __vn_focus !== "function") return;
  if (this === arriving) return;
  if (__vn_focus(this.__vnId, 1) === "blurred") {
    focusChanges++;
    arriving = null;
    if (focusedNode === this) focusedNode = null;
    fireFocusEvent(this, "blur", null);
    fireFocusEvent(this, "focusout", null);
  }
});

function activeElement(): MirrorElement | null {
  if (focusedNode && nodeById(focusedNode.__vnId) === focusedNode) return focusedNode;
  return appRoot;
}

// location.reload(): the host (`__vn_reload`) calls `__VN_UNLOAD__`, then re-evaluates the bundle in the same realm,
// so the outgoing app's timers, frames, idle callbacks, WebSockets (listeners removed first, so reconnect logic
// cannot run) and runtime-sourced window/document listeners are undone here, after `pagehide` and the unmount.

type Fn = (...args: unknown[]) => unknown;
type SocketLike = {
  close(): void;
  addEventListener?: Fn;
  removeEventListener?: Fn;
  [handler: string]: unknown;
};

/// Called with each frame's timestamp before a rAF callback runs, so `document.timeline` follows it.
const frameStampers: ((timestamp: number) => void)[] = [];

const realm = (() => {
  const g = globalThis as unknown as Record<string, unknown>;
  // The engine's own functions, kept ONCE per process (see `nativeListenFns`).
  const native = (g.__VN_NATIVE_SCHED__ ??= {
    setTimeout: g.setTimeout,
    clearTimeout: g.clearTimeout,
    setInterval: g.setInterval,
    clearInterval: g.clearInterval,
    requestAnimationFrame: g.requestAnimationFrame,
    cancelAnimationFrame: g.cancelAnimationFrame,
    requestIdleCallback: g.requestIdleCallback,
    cancelIdleCallback: g.cancelIdleCallback,
    WebSocket: g.WebSocket,
  }) as Record<string, Fn | undefined>;
  const timers = new Set<unknown>();
  const frames = new Set<unknown>();
  const idles = new Set<unknown>();
  const sockets = new Map<SocketLike, { type: string; fn: unknown; opts: unknown }[]>();

  const track = (setName: string, live: Set<unknown>, once: boolean) => {
    const set = native[setName];
    if (typeof set !== "function") return;
    g[setName] = function (cb: unknown, ...rest: unknown[]) {
      if (typeof cb !== "function") return set.call(g, cb, ...rest); // the string form
      let id: unknown;
      const run = once
        ? function (this: unknown, ...a: unknown[]) {
            live.delete(id);
            if (live === frames && typeof a[0] === "number") for (const stamp of frameStampers) stamp(a[0]);
            const token = traceCallbackBegin?.(cb) ?? 0;
            if (token === 0) return (cb as Fn).apply(this, a);
            try {
              return (cb as Fn).apply(this, a);
            } finally {
              traceCallbackEnd!(token);
            }
          }
        : cb;
      id = set.call(g, run, ...rest);
      live.add(id);
      return id;
    };
  };
  const untrack = (clearName: string, ...lives: Set<unknown>[]) => {
    const clear = native[clearName];
    if (typeof clear !== "function") return;
    g[clearName] = function (id: unknown) {
      for (const live of lives) live.delete(id);
      return clear.call(g, id);
    };
  };
  track("setTimeout", timers, true);
  track("setInterval", timers, false);
  // One id space for both (HTML): either clear cancels either.
  untrack("clearTimeout", timers);
  untrack("clearInterval", timers);
  track("requestAnimationFrame", frames, true);
  untrack("cancelAnimationFrame", frames);
  track("requestIdleCallback", idles, true);
  untrack("cancelIdleCallback", idles);

  const NativeWS = native.WebSocket;
  if (typeof NativeWS === "function") {
    g.WebSocket = new Proxy(NativeWS, {
      construct(target, args, newTarget) {
        const ws = Reflect.construct(target, args, newTarget) as SocketLike;
        const added: { type: string; fn: unknown; opts: unknown }[] = [];
        sockets.set(ws, added);
        const add = ws.addEventListener;
        const remove = ws.removeEventListener;
        if (typeof add === "function") {
          Object.defineProperty(ws, "addEventListener", {
            value(type: string, fn: unknown, opts?: unknown) {
              added.push({ type, fn, opts });
              return add.call(ws, type, fn, opts);
            },
            configurable: true,
            writable: true,
          });
          add.call(ws, "close", () => sockets.delete(ws));
        }
        if (typeof remove === "function") {
          Object.defineProperty(ws, "removeEventListener", {
            value(type: string, fn: unknown, opts?: unknown) {
              const i = added.findIndex((a) => a.type === type && a.fn === fn);
              if (i >= 0) added.splice(i, 1);
              return remove.call(ws, type, fn, opts);
            },
            configurable: true,
            writable: true,
          });
        }
        return ws as object;
      },
    });
  }

  const close = () => {
    for (const { target, name, fn, opts } of passedThrough.splice(0)) {
      try {
        nativeListenFns(target).remove?.(name, fn, opts);
      } catch {
        /* already gone */
      }
    }
    for (const { slot, delegate } of interceptedGlobals.splice(0)) slot.clear(delegate);
    for (const [ws, added] of sockets) {
      const remove = Object.getOwnPropertyDescriptor(ws, "removeEventListener")?.value as Fn | undefined;
      for (const a of added.splice(0)) remove?.call(ws, a.type, a.fn, a.opts);
      for (const h of ["onopen", "onmessage", "onerror", "onclose"]) ws[h] = null;
      try {
        ws.close();
      } catch {
        /* closing a closed socket */
      }
    }
    sockets.clear();
    for (const id of timers) native.clearTimeout?.call(g, id);
    for (const id of frames) native.cancelAnimationFrame?.call(g, id);
    for (const id of idles) native.cancelIdleCallback?.call(g, id);
    timers.clear();
    frames.clear();
    idles.clear();
  };
  return { close };
})();

{
  const g = globalThis as unknown as {
    location?: Record<string, unknown>;
    __VN_UNLOAD__?: (() => void) | undefined;
    __VN_TEARDOWN__?: () => void;
  };
  if (g.location && typeof __vn_reload === "function") {
    try {
      Object.defineProperty(g.location, "reload", {
        value: function reload() {
          __vn_reload();
        },
        configurable: true,
        writable: true,
      });
    } catch (e) {
      warnCompat(
        "location.reload:install",
        `location.reload() could not be installed on this engine and does nothing (${e})`,
      );
    }
  }
  g.__VN_UNLOAD__ = () => {
    g.__VN_UNLOAD__ = undefined;
    delete (globalThis as { __VN_SCREEN_SOURCE__?: unknown }).__VN_SCREEN_SOURCE__;
    warnCompat(
      "location.reload",
      "location.reload() re-runs the app in the SAME JavaScript realm: its timers, animation frames, " +
        "WebSockets, window/document listeners, media tracks " +
        "and AudioContexts are closed and the globals it added are deleted, but " +
        "changes it made to built-in objects and to globals that were already there stay, fetches in " +
        "flight settle into the old code, IndexedDB connections stay until the old code " +
        "ends them, beforeunload is not asked, and DOMContentLoaded / load do not fire again",
    );
    try {
      const ev = typeof Event === "function" ? new Event("pagehide") : null;
      if (ev) {
        Object.defineProperty(ev, "persisted", { value: false });
        (globalThis as unknown as { dispatchEvent?: (e: VnEvent) => boolean }).dispatchEvent?.(ev);
      }
    } catch (e) {
      __vn_log("reload: pagehide: " + e);
    }
    try {
      g.__VN_TEARDOWN__?.();
    } catch (e) {
      __vn_log("reload: teardown: " + e);
    }
    (globalThis as { __VN_HOST_LIFECYCLE__?: { reset: () => void } }).__VN_HOST_LIFECYCLE__?.reset();
    realm.close();
    // The host keeps calling the old observer step until the next runtime installs its own; tell it to stop.
    resizeObservers.length = 0;
    resizeDue = [];
    intersectionObservers.clear();
    if (hostSteps) __vn_layout_watch(false);
    // Last: the teardown above still calls through the globals this deletes.
    const kept = resetGlobals();
    if (kept)
      warnCompat(
        "location.reload:globals",
        `location.reload(): ${kept} global(s) the old app declared cannot be deleted (a top-level var or ` +
          "function in a classic script, or a non-configurable property), so the new app finds them set",
      );
  };
}

// -- pointer capture: one owner per pointer; the host routes input to it even outside its hitbox.
const pointerCaptures = new Map<number, MirrorElement>();
const disconnectingCaptureRoots = new Set<MirrorNode>();
let pointerButtons = 0;
let pointerPosition = { x: 0, y: 0, button: -1, modifiers: 0 };

function captureEvent(owner: MirrorElement, name: string): void {
  if (hasListener(owner, name)) {
    const p = pointerPosition;
    propagate(owner, name, synthesize(name, p.x, p.y, p.button, p.modifiers, undefined));
  }
}
function releaseCapturedPointer(pointerId: number, owner?: MirrorElement): void {
  const current = pointerCaptures.get(pointerId);
  if (!current || (owner && owner !== current)) return;
  pointerCaptures.delete(pointerId);
  __vn_pointer_capture(current.__vnId, pointerId, false);
  captureEvent(current, "lostpointercapture");
}
function releaseCapturesIn(node: MirrorNode): void {
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
  if (!pointerButtons) return;
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

// Imperative tree mutation runs through `nodeOps.insert`, so the host sees the ops a Vue patch sends; a removal
// DETACHES (see `detachNode`). The engine's own detached `document.createElement(...)` objects (`ENGINE_TAGS`) may be
// passed too: `<style>`/`<link>` join the stylesheet (`foreignNode`), others are left out, advised.

const ENGINE_TAGS = new Set(["a", "input", "style", "link", "script", "meta", "title", "base"]);
function stylesheetText(node: unknown): string {
  if (node == null) return "";
  if (typeof node === "string") return node;
  const text = node as { textContent?: unknown; data?: unknown; nodeValue?: unknown; text?: unknown };
  return String(text.textContent ?? text.data ?? text.nodeValue ?? text.text ?? "");
}
/** The engine's detached style element has inert child methods: buffer its text from creation. */
function prepareStylesheetElement(element: Record<string, unknown>): void {
  let text = stylesheetText(element);
  for (const property of ["textContent", "innerHTML", "innerText", "text"]) {
    Object.defineProperty(element, property, {
      get: () => text,
      set: (value) => {
        text = value == null ? "" : String(value);
      },
      configurable: true,
    });
  }
  const append = (child: unknown) => {
    text += stylesheetText(child);
    return child;
  };
  Object.defineProperty(element, "appendChild", { value: append, configurable: true, writable: true });
  Object.defineProperty(element, "append", {
    value: (...children: unknown[]) => {
      children.forEach(append);
    },
    configurable: true,
    writable: true,
  });
}
/** DOM's "valid element local name" (the 2025 relaxed rule): no whitespace, NUL, `/` or `>`, and a letter, `:`, `_` or non-ASCII first. */
const VALID_ELEMENT_NAME = /^[A-Za-z:_\u0080-￿][^\t\n\f\r \/>\0]*$/;
let foreignStyle: ((n: object, inserted: boolean) => boolean) | null = null;
const fragments = new WeakSet<MirrorNode>();

const domError = (method: string, iface: string, name: string, msg: string): Error =>
  new DOMException(`Failed to execute '${method}' on '${iface}': ${msg}`, name);

/** A Node argument: a live mirror, or `null` for a node that is not ours (an engine element, or a freed mirror: `foreignNode`). Anything else is WebIDL's TypeError. */
function nodeArg(x: unknown, method: string, iface: string, pos: number, type = "Node"): MirrorNode | null {
  if (isMirrorNode(x)) {
    if (type === "Element" && x.kind !== 1)
      throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter ${pos} is not of type 'Element'.`);
    return x;
  }
  if (x && typeof x === "object" && (typeof (x as { tagName?: unknown }).tagName === "string" || "__vnId" in x))
    return null;
  throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter ${pos} is not of type '${type}'.`);
}

function foreignNode(x: object, inserted: boolean, method: string): void {
  if (foreignStyle?.(x, inserted)) return;
  if ("__vnId" in x) {
    warnCompat(
      "dom:freed",
      `${method}() was given a node the Vue renderer already freed (its component unmounted), so it is not ` +
        "inserted again — in a browser it would be. Keep a node you move by hand out of the renderer's own subtrees",
    );
    return;
  }
  if (inserted) {
    const tag = String((x as { tagName?: unknown }).tagName).toLowerCase();
    warnCompat(
      `dom:engine-${tag}`,
      `${method}() — a <${tag}> from document.createElement("${tag}") is the engine's detached element, ` +
        "not a node of the retained tree, so it is not inserted and never painted (its click, href and files still work)",
    );
  }
}

function preInsertCheck(parent: MirrorNode, node: MirrorNode, method: string): void {
  if (parent.kind !== 1)
    throw domError(method, "Node", "HierarchyRequestError", "This node type does not support this method.");
  for (let a: MirrorNode | null = parent; a; a = a.parentNode) {
    if (a === node)
      throw domError(method, "Node", "HierarchyRequestError", "The new child element contains the parent.");
  }
}

/** An element's own text becomes a real Text child before it gains a sibling, so the text is kept, as in a browser. */
function materializeOwnText(el: MirrorElement): void {
  if (!el.ownText || el.childNodes.length) return;
  const t = nodeOps.createText(el.ownText);
  el.ownText = "";
  insertNode(t, el, null, false);
  if (typeof __vn_selection_rebase === "function") __vn_selection_rebase(el.__vnId, t.__vnId);
}

function insertAll(parent: MirrorElement, items: (MirrorNode | object)[], ref: MirrorNode | null): void {
  const added: MirrorNode[] = [];
  for (const it of items) {
    if (!isMirrorNode(it)) {
      foreignNode(it, true, "insert");
      continue;
    }
    for (const n of fragments.has(it) ? it.childNodes.slice() : [it]) {
      if (!added.length) materializeOwnText(parent);
      insertNode(n, parent, ref, false);
      added.push(n);
    }
  }
  if (added.length) notifyMutation(parent, "childList", { addedNodes: added, removedNodes: [] });
}

function preInsert(parent: MirrorNode, node: unknown, child: unknown, method: string): unknown {
  const n = nodeArg(node, method, "Node", 1);
  const c = child == null ? null : nodeArg(child, method, "Node", 2);
  if (!n) {
    foreignNode(node as object, true, method);
    return node;
  }
  preInsertCheck(parent, n, method);
  if (child != null && (!c || c.parentNode !== parent)) {
    throw domError(
      method,
      "Node",
      "NotFoundError",
      "The node before which the new node is to be inserted is not a child of this node.",
    );
  }
  insertAll(parent as MirrorElement, [n], c === n ? n.nextSibling : c);
  return n;
}

function convertNodes(parent: MirrorNode, args: unknown[], method: string, iface: string): (MirrorNode | object)[] {
  const items = args.map((a) => {
    if (isMirrorNode(a)) return a;
    if (a && typeof a === "object" && (typeof (a as { tagName?: unknown }).tagName === "string" || "__vnId" in a))
      return a;
    return looseCreated(nodeOps.createText(String(a)));
  });
  for (const it of items) if (isMirrorNode(it)) preInsertCheckAs(parent, it, method, iface);
  return items;
}
function preInsertCheckAs(parent: MirrorNode, node: MirrorNode, method: string, iface: string): void {
  try {
    preInsertCheck(parent, node, method);
  } catch (e) {
    throw domError(method, iface, (e as { name: string }).name, String((e as Error).message).replace(/^.*?': /, ""));
  }
}

function viable(from: MirrorNode | null, items: unknown[], step: 1 | -1): MirrorNode | null {
  let n = from;
  while (n && items.includes(n)) n = siblingOf(n, step);
  return n;
}

defineDom(
  "appendChild",
  function (this: MirrorNode, node: unknown) {
    if (!isMirrorNode(this)) return node;
    return preInsert(this, node, null, "appendChild");
  },
  NODE_MEMBERS,
);
defineDom(
  "insertBefore",
  function (this: MirrorNode, node: unknown, child: unknown) {
    if (!isMirrorNode(this)) return node;
    if (arguments.length < 2) {
      throw new TypeError("Failed to execute 'insertBefore' on 'Node': 2 arguments required, but only 1 present.");
    }
    return preInsert(this, node, child, "insertBefore");
  },
  NODE_MEMBERS,
);
defineDom(
  "removeChild",
  function (this: MirrorNode, child: unknown) {
    const c = nodeArg(child, "removeChild", "Node", 1);
    if (!c || !isMirrorNode(this)) {
      foreignNode(child as object, false, "removeChild");
      return child;
    }
    if (c.parentNode !== this) {
      throw domError("removeChild", "Node", "NotFoundError", "The node to be removed is not a child of this node.");
    }
    detachNode(c);
    return c;
  },
  NODE_MEMBERS,
);
defineDom(
  "replaceChild",
  function (this: MirrorNode, node: unknown, child: unknown) {
    const n = nodeArg(node, "replaceChild", "Node", 1);
    const c = nodeArg(child, "replaceChild", "Node", 2);
    if (!isMirrorNode(this) || !n || !c) {
      if (!n) foreignNode(node as object, true, "replaceChild");
      if (!c) foreignNode(child as object, false, "replaceChild");
      return child;
    }
    preInsertCheck(this, n, "replaceChild");
    if (c.parentNode !== this) {
      throw domError("replaceChild", "Node", "NotFoundError", "The node to be replaced is not a child of this node.");
    }
    if (n === c) return c;
    let ref = c.nextSibling;
    if (ref === n) ref = n.nextSibling;
    detachNode(c);
    insertAll(this as MirrorElement, [n], ref);
    return c;
  },
  NODE_MEMBERS,
);

defineDom(
  "remove",
  function (this: MirrorNode) {
    if (!isMirrorNode(this)) return void foreignNode(this, false, "remove");
    if (this.parentNode) detachNode(this);
  },
  NODE_MEMBERS,
);
defineDom(
  "before",
  function (this: MirrorNode, ...nodes: unknown[]) {
    const parent = this.parentNode;
    if (!isMirrorNode(this) || !parent) return;
    const prev = viable(siblingOf(this, -1), nodes, -1);
    const items = convertNodes(parent, nodes, "before", "Element");
    insertAll(parent, items, viable(prev ? siblingOf(prev, 1) : (parent.childNodes[0] ?? null), nodes, 1));
  },
  NODE_MEMBERS,
);
defineDom(
  "after",
  function (this: MirrorNode, ...nodes: unknown[]) {
    const parent = this.parentNode;
    if (!isMirrorNode(this) || !parent) return;
    const next = viable(siblingOf(this, 1), nodes, 1);
    insertAll(parent, convertNodes(parent, nodes, "after", "Element"), next);
  },
  NODE_MEMBERS,
);
defineDom(
  "replaceWith",
  function (this: MirrorNode, ...nodes: unknown[]) {
    const parent = this.parentNode;
    if (!isMirrorNode(this) || !parent) return;
    const next = viable(siblingOf(this, 1), nodes, 1);
    const items = convertNodes(parent, nodes, "replaceWith", "Element");
    if (items.includes(this)) return insertAll(parent, items, next);
    insertAll(parent, items, this);
    detachNode(this);
  },
  NODE_MEMBERS,
);

defineDom("append", function (this: MirrorElement, ...nodes: unknown[]) {
  if (!isMirrorNode(this)) return;
  insertAll(this, convertNodes(this, nodes, "append", "Element"), null);
});
defineDom("prepend", function (this: MirrorElement, ...nodes: unknown[]) {
  if (!isMirrorNode(this)) return;
  const items = convertNodes(this, nodes, "prepend", "Element");
  insertAll(this, items, viable(this.childNodes[0] ?? null, nodes, 1));
});
defineDom("replaceChildren", function (this: MirrorElement, ...nodes: unknown[]) {
  if (!isMirrorNode(this)) return;
  const items = convertNodes(this, nodes, "replaceChildren", "Element");
  const gone = this.childNodes.filter((c) => !items.includes(c));
  for (const c of gone) detachNode(c, false);
  if (this.ownText) nodeOps.setElementText(this, "");
  if (gone.length) notifyMutation(this, "childList", { addedNodes: [], removedNodes: gone });
  insertAll(this, items, null);
});

const ADJACENT = ["beforebegin", "afterbegin", "beforeend", "afterend"] as const;
function adjacentSlot(el: MirrorElement, where: unknown, method: string): [MirrorElement, MirrorNode | null] | null {
  const w = String(where).toLowerCase();
  if (!(ADJACENT as readonly string[]).includes(w)) {
    throw domError(
      method,
      "Element",
      "SyntaxError",
      `The value provided ('${String(where)}') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or 'afterEnd'.`,
    );
  }
  if (w === "afterbegin") return [el, el.childNodes[0] ?? null];
  if (w === "beforeend") return [el, null];
  const parent = el.parentNode;
  if (!parent) return null;
  return [parent, w === "beforebegin" ? el : siblingOf(el, 1)];
}
defineDom("insertAdjacentElement", function (this: MirrorElement, where: unknown, element: unknown) {
  const n = nodeArg(element, "insertAdjacentElement", "Element", 2, "Element");
  if (!isMirrorNode(this)) return element;
  const slot = adjacentSlot(this, where, "insertAdjacentElement");
  if (!slot) return null;
  if (!n) {
    foreignNode(element as object, true, "insertAdjacentElement");
    return element;
  }
  preInsertCheckAs(slot[0], n, "insertAdjacentElement", "Element");
  insertAll(slot[0], [n], slot[1] === n ? n.nextSibling : slot[1]);
  return n;
});
defineDom("insertAdjacentText", function (this: MirrorElement, where: unknown, data: unknown) {
  if (!isMirrorNode(this)) return;
  const slot = adjacentSlot(this, where, "insertAdjacentText");
  if (slot) insertAll(slot[0], [looseCreated(nodeOps.createText(String(data)))], slot[1]);
});

defineDom("cloneNode", function () {
  warnCompat(
    "cloneNode",
    "element.cloneNode() is not implemented — the retained tree has no copy operation; render a " +
      "second element instead",
  );
  return null;
});

// HTMLMediaElement: media state lives in the HOST (`currentTime` moves in the OS pipeline), so accessors read
// `__vn_media_state` live, uncached; verbs go through `__vn_media_call`.
const MEDIA_CALL = {
  play: 0,
  pause: 1,
  currentTime: 2,
  volume: 3,
  muted: 4,
  playbackRate: 5,
  loop: 6,
} as const satisfies Record<string, VnMediaVerb>;
const MEDIA_TAGS = new Set<string>(["VIDEO", "AUDIO"]);
const mediaStreams = new WeakMap<MirrorElement, unknown>();
defineDomGet(
  "srcObject",
  function (this: MirrorElement) {
    return isMediaEl(this) ? (mediaStreams.get(this) ?? null) : undefined;
  },
  function (this: MirrorElement, value: unknown) {
    if (!isMediaEl(this)) return;
    if (value == null) {
      mediaStreams.delete(this);
      if (this.getAttribute("src")?.startsWith("vn-stream://")) this.removeAttribute("src");
      return;
    }
    const stream = value as { getVideoTracks?: () => { __vnVideoSrc?: string }[] };
    const track = typeof stream.getVideoTracks === "function" ? stream.getVideoTracks()[0] : undefined;
    if (!track?.__vnVideoSrc?.startsWith("vn-stream://")) {
      // An <audio> given an audio stream (kept alive while Web Audio plays it) holds the stream and plays nothing.
      if (this.tagName === "AUDIO") {
        mediaStreams.set(this, value);
        if (this.getAttribute("src")?.startsWith("vn-stream://")) this.removeAttribute("src");
        return;
      }
      throw new TypeError("video.srcObject requires a native video MediaStream");
    }
    mediaStreams.set(this, value);
    this.setAttribute("src", track.__vnVideoSrc);
  },
);
const isMediaEl = (n: MirrorNode | null | undefined): n is MirrorElement =>
  !!n && MEDIA_TAGS.has((n as MirrorElement).tagName);
function mediaState(node: MirrorNode | null | undefined): Record<string, string> | null {
  if (!isMediaEl(node) || !node.__vnId) return null;
  const raw = __vn_media_state(node.__vnId);
  if (!raw) return null;
  const out: Record<string, string> = {};
  for (const line of raw.split("\n")) {
    if (!line) continue;
    const eq = line.indexOf("=");
    if (eq > 0) out[line.slice(0, eq)] = line.slice(eq + 1);
  }
  return out;
}
const mediaCall = (node: MirrorNode, verb: VnMediaVerb, arg = 0): boolean =>
  isMediaEl(node) && node.__vnId ? __vn_media_call(node.__vnId, verb, arg) : false;
function timeRanges(raw: string | undefined): MirrorTimeRanges {
  const list = raw
    ? raw
        .split(",")
        .filter(Boolean)
        .map((pair) => {
          const [start = NaN, end = NaN] = pair.split(":").map(Number);
          return { start, end };
        })
    : [];
  return {
    length: list.length,
    start: (i: number) => list[i]?.start ?? NaN,
    end: (i: number) => list[i]?.end ?? NaN,
    ranges: list,
  };
}
function defineMediaGet(
  name: string,
  read: (s: Record<string, string>) => unknown,
  write?: (value: never) => void,
): void {
  defineDomGet(
    name,
    function (this: MirrorElement) {
      const s = mediaState(this);
      return s ? read(s) : isMediaEl(this) ? read({}) : undefined;
    },
    write,
  );
}
const num = (s: Record<string, string>, k: string, fallback: number): number =>
  s[k] === undefined ? fallback : Number(s[k]);
const bool = (s: Record<string, string>, k: string, fallback: boolean): boolean =>
  s[k] === undefined ? fallback : s[k] === "true";

/** A SCALAR media property: one host call (`__vn_media_prop`), since the whole state string costs ~0.2 ms per read on Media Foundation. */
function defineMediaScalar(
  name: string,
  prop: VnMediaProp,
  fallback: number | boolean,
  write?: (value: never) => void,
): void {
  const isBool = typeof fallback === "boolean";
  defineDomGet(
    name,
    function (this: MirrorElement) {
      if (!isMediaEl(this)) return undefined;
      if (typeof __vn_media_prop === "function") {
        const v = this.__vnId ? __vn_media_prop(this.__vnId, prop) : undefined;
        if (v === undefined) return fallback;
        return isBool ? v !== 0 : v;
      }
      const s = mediaState(this) ?? {};
      return isBool ? bool(s, name, fallback as boolean) : num(s, name, fallback as number);
    },
    write,
  );
}

defineMediaScalar("currentTime", 0, 0, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.currentTime, Number(v));
});
defineMediaScalar("duration", 1, NaN);
defineMediaScalar("paused", 2, true);
defineMediaScalar("ended", 3, false);
defineMediaScalar("seeking", 4, false);
defineMediaScalar("readyState", 5, 0);
defineMediaScalar("networkState", 6, 0);
defineMediaScalar("volume", 7, 1, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.volume, Number(v));
});
defineMediaScalar("muted", 8, false, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.muted, v ? 1 : 0);
});
defineMediaScalar("playbackRate", 9, 1, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.playbackRate, Number(v));
});
defineMediaScalar("loop", 10, false, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.loop, v ? 1 : 0);
});
defineMediaScalar("videoWidth", 11, 0);
defineMediaScalar("videoHeight", 12, 0);
defineMediaGet("buffered", (s) => timeRanges(s.buffered));
defineMediaGet("seekable", (s) => timeRanges(s.seekable));
defineMediaGet("error", (s) => {
  const code = num(s, "error", 0);
  return code ? { code, message: s.errorMessage ?? "" } : null;
});

defineDom("play", function (this: MirrorElement) {
  mediaCall(this, MEDIA_CALL.play);
  // Resolves immediately (the host's play is synchronous), so `el.play().catch(…)` does not throw.
  return Promise.resolve();
});
defineDom("pause", function (this: MirrorElement) {
  mediaCall(this, MEDIA_CALL.pause);
});
defineDom("load", function (this: MirrorElement) {
  // The host loads off the `src` prop, edge-triggered; there is no load verb.
  if (isMediaEl(this)) {
    warnCompat(
      "media-load",
      "video.load() is a no-op — the host loads from the `src` prop, edge-triggered; " +
        "to force a reload, clear `src` and set it again",
    );
  }
});
defineDom("canPlayType", function (this: MirrorElement, type: unknown) {
  if (!isMediaEl(this) || !this.__vnId) return "";
  return __vn_media_can_play(this.__vnId, String(type));
});

// HTMLIFrameElement: the document lives in a host-owned platform webview, so it is cross-origin: post to it and hear
// from it. `contentWindow` is one object per element, so `event.source === iframe.contentWindow` holds.
const isFrameEl = (x: unknown): x is MirrorElement => isMirrorEl(x) && x.tag === "iframe";
const frameWindows = new WeakMap<MirrorElement, object>();
const frameOf = new WeakMap<object, MirrorElement>();

function frameTargetOrigin(arg: unknown): string {
  if (arg == null) return "/"; // the WindowPostMessageOptions default: same origin only
  if (typeof arg === "object") {
    const t = (arg as { targetOrigin?: unknown; transfer?: unknown[] }).targetOrigin;
    if ((arg as { transfer?: unknown[] }).transfer?.length) {
      throw new DOMException(
        "Failed to execute 'postMessage': transferring is not supported for an <iframe>.",
        "DataCloneError",
      );
    }
    return t == null ? "/" : String(t);
  }
  return String(arg);
}

function frameWindow(el: MirrorElement): object {
  let win = frameWindows.get(el);
  if (win) return win;
  // No prototype: nothing of this realm is reachable through another origin's window handle.
  win = Object.freeze(
    Object.assign(Object.create(null) as Record<string, unknown>, {
      postMessage(message: unknown, target?: unknown, transfer?: unknown[]) {
        if (arguments.length === 0)
          throw new TypeError("Failed to execute 'postMessage': 1 argument required, but only 0 present.");
        if (transfer?.length) {
          throw new DOMException(
            "Failed to execute 'postMessage': transferring is not supported for an <iframe>.",
            "DataCloneError",
          );
        }
        const origin = frameTargetOrigin(target);
        // Encoded now, in the sender, so a value that cannot cross throws HERE.
        const wire = JSON.stringify(encodeMessage(message, (o) => isMirrorNode(o)));
        if (!el.__vnId || !el.isConnected || typeof __vn_iframe_post !== "function") return;
        __vn_iframe_post(el.__vnId, wire, origin);
      },
      toString: () => "[object Window]",
    }),
  );
  frameWindows.set(el, win);
  frameOf.set(win, el);
  return win;
}

defineDomGet("contentWindow", function (this: MirrorElement) {
  if (!isFrameEl(this)) return undefined;
  return this.isConnected ? frameWindow(this) : null;
});
defineDomGet("contentDocument", function (this: MirrorElement) {
  return isFrameEl(this) ? null : undefined;
});
defineDom("getSVGDocument", function (this: MirrorElement) {
  return null;
});

defineDomGet(
  "sandbox",
  function (this: MirrorElement) {
    if (!isFrameEl(this)) return undefined;
    const node = this;
    const parts = () => (node.attrs.sandbox ?? "").split(/\s+/).filter(Boolean);
    const write = (tokens: string[]) => node.setAttribute("sandbox", tokens.join(" "));
    const list = {
      get value() {
        return node.attrs.sandbox ?? "";
      },
      set value(v: string) {
        node.setAttribute("sandbox", String(v));
      },
      add(...names: string[]) {
        const now = parts();
        for (const n of names) if (!now.includes(n)) now.push(n);
        write(now);
      },
      remove(...names: string[]) {
        write(parts().filter((p) => !names.includes(p)));
      },
      toggle(name: string, force?: boolean) {
        const on = force === undefined ? !parts().includes(name) : !!force;
        if (on) list.add(name);
        else list.remove(name);
        return on;
      },
      replace(from: string, to: string) {
        const now = parts();
        const i = now.indexOf(from);
        if (i < 0) return false;
        now[i] = to;
        write([...new Set(now)]);
        return true;
      },
      contains: (name: string) => parts().includes(name),
      supports: (name: string) => FRAME_SANDBOX_TOKENS.has(String(name).toLowerCase()),
      get length() {
        return parts().length;
      },
      item: (i: number) => parts()[i] ?? null,
      forEach(fn: (token: string, index: number) => void) {
        parts().forEach((t, i) => fn(t, i));
      },
      toString: () => node.attrs.sandbox ?? "",
      [Symbol.iterator]: () => parts()[Symbol.iterator](),
    };
    Object.defineProperty(this, "sandbox", {
      get: () => list,
      set: (v: unknown) => node.setAttribute("sandbox", String(v)),
      configurable: true,
    });
    return branded("DOMTokenList", list);
  },
  function (this: MirrorElement, v: unknown) {
    if (!isFrameEl(this)) return ownExpando(this, "sandbox", v);
    this.setAttribute("sandbox", String(v));
  } as (value: never) => void,
);
const FRAME_SANDBOX_TOKENS = new Set([
  "allow-downloads",
  "allow-forms",
  "allow-modals",
  "allow-orientation-lock",
  "allow-pointer-lock",
  "allow-popups",
  "allow-popups-to-escape-sandbox",
  "allow-presentation",
  "allow-same-origin",
  "allow-scripts",
  "allow-top-navigation",
  "allow-top-navigation-by-user-activation",
  "allow-top-navigation-to-custom-protocols",
]);

function deliverFrameMessage(target: MirrorNode, key: string | undefined): boolean {
  if (!isFrameEl(target) || key == null) return true;
  let origin = "null";
  let data: unknown;
  try {
    const parsed = JSON.parse(key) as { o?: string; d?: unknown };
    origin = String(parsed.o ?? "null");
    data = decodeMessage(parsed.d);
  } catch (e) {
    console.error("[vue-native] an <iframe> message that could not be decoded was dropped:", e);
    return true;
  }
  const source = frameWindow(target);
  const init = { data, origin, source, lastEventId: "", ports: [] as unknown[] };
  let ev: VnEvent;
  const Ctor = (globalThis as { MessageEvent?: new (type: string, init: object) => VnEvent }).MessageEvent;
  try {
    // A real MessageEvent refuses a foreign `source`, so the fields are defined on it below.
    ev = Ctor ? new Ctor("message", { data, origin, lastEventId: "" }) : (new Event("message") as unknown as VnEvent);
  } catch {
    ev = new Event("message") as unknown as VnEvent;
  }
  for (const k of Object.keys(init) as (keyof typeof init)[]) {
    Object.defineProperty(ev, k, { value: init[k], configurable: true, enumerable: true });
  }
  propagateAtGlobal(globalThis, "message", ev);
  return true;
}

/** `vnFeatures`: host capabilities a page should check; `iframe` is false without a webview backend (Linux, or Windows without WebView2), where every `src` answers `error`. */
Object.defineProperty(globalThis, "vnFeatures", {
  value: Object.freeze(
    Object.defineProperty({}, "iframe", {
      get: () => typeof __vn_iframe_supported === "function" && __vn_iframe_supported(),
      enumerable: true,
    }),
  ),
  configurable: true,
});

// HTMLImageElement: the host answers every `src` with `load` or `error`; the dispatcher records it on the element
// (`_img`) before the event runs, and discards one for a `src` the element has since dropped.
const isImgEl = (x: unknown): x is MirrorElement => isMirrorEl(x) && x.tag === "img";
const imgAnswer = (el: MirrorElement) => (el._img && el._img.src === el.attrs.src ? el._img : undefined);
const SRC_TAGS = new Set(["img", "video", "audio", "source", "iframe", "lottie"]);

defineDomGet(
  "src",
  function (this: MirrorElement) {
    if (!isMirrorEl(this) || !SRC_TAGS.has(this.tag)) return undefined;
    return this.attrs.src ?? "";
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this) || !SRC_TAGS.has(this.tag)) return ownExpando(this, "src", v);
    patchProp(this, "src", undefined, String(v));
  } as (value: never) => void,
);
defineDomGet("currentSrc", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  return imgAnswer(this)?.ok ? this.attrs.src : "";
});
defineDomGet("complete", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  const src = this.attrs.src;
  return src == null || src === "" || !!imgAnswer(this);
});
defineDomGet("naturalWidth", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  return imgAnswer(this)?.w ?? 0;
});
defineDomGet("naturalHeight", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  return imgAnswer(this)?.h ?? 0;
});
defineDom("decode", function (this: MirrorElement) {
  if (!isImgEl(this)) return Promise.resolve();
  const el = this;
  const settled = (): Promise<void> | null => {
    const answer = imgAnswer(el);
    if (answer?.ok) return Promise.resolve();
    if (answer || !el.attrs.src)
      return Promise.reject(new DOMException("The source image cannot be decoded.", "EncodingError"));
    return null;
  };
  return (
    settled() ??
    new Promise<void>((resolve, reject) => {
      const done = () => {
        el.removeEventListener("load", done);
        el.removeEventListener("error", done);
        (settled() ?? Promise.reject(new DOMException("The source image cannot be decoded.", "EncodingError"))).then(
          resolve,
          reject,
        );
      };
      el.addEventListener("load", done);
      el.addEventListener("error", done);
    })
  );
});

// `on<event>` IDL handlers: one listener per element and type, kept in its place in the listener order while
// the handler changes; `null` removes it; a handler returning `false` cancels the event. On a non-mirror
// object the property stays a plain expando (a detached `<input>`'s `onchange` is read by `fireDetached`).
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
        listener: (ev: VnEvent) => {
          if (typeof entry.fn !== "function") return;
          const r = (entry.fn as (this: unknown, ev: VnEvent) => unknown).call(el, ev);
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
const mods = (d: number) => ({
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

const BUTTON_BITS = [1, 4, 2, 8, 16];
/** `buttons`: which buttons are DOWN as the event fires (already up by `mouseup`, `click`, `dblclick`, `auxclick`, `contextmenu`). */
function buttonsFor(name: string, c: number): number {
  switch (name) {
    case "gotpointercapture":
    case "lostpointercapture":
      return pointerButtons;
    case "mousedown":
    case "pointerdown":
    case "mousemove":
    case "pointermove":
    case "mouseenter":
    case "mouseleave":
    case "pointerenter":
    case "pointerleave":
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
      return 0;
    case "pointermove":
    case "pointerenter":
    case "pointerleave":
    case "pointercancel":
      return -1;
    case "dblclick":
      return 0;
    default:
      return c < 0 ? 0 : c;
  }
}
/** `detail`: the click count, carried above the four modifier bits of `d`; `dblclick` is 2, everything else 0. */
function detailFor(name: string, d: number): number {
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

/** A wheel's `clientX` / `clientY`, passed to `synthesize` as `data` (`a`/`b` are the deltas); 0, 0 if absent. */
function wheelAt(data: unknown): { clientX: number; clientY: number } {
  const at = data as { clientX?: unknown; clientY?: unknown } | null | undefined;
  return {
    clientX: typeof at?.clientX === "number" ? at.clientX : 0,
    clientY: typeof at?.clientY === "number" ? at.clientY : 0,
  };
}

function synthesize(
  name: string,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
  data?: unknown,
  tag?: string,
): VnEvent {
  const common = { bubbles: !NO_BUBBLE.has(name), cancelable: !NO_BUBBLE.has(name) && !NOT_CANCELABLE.has(name) };
  const M = mods(d);
  try {
    if ((name === "copy" || name === "cut" || name === "paste") && ClipboardEventClass)
      return new ClipboardEventClass(name, { ...common, composed: true, clipboardData: data ?? null });
    if (tag === "img" && (name === "load" || name === "error")) return new Event(name, common);
    if (POINTER_EVENTS.has(name)) {
      const buttons = buttonsFor(name, c);
      const init = {
        ...common,
        ...M,
        ...POINTER_INIT,
        clientX: a,
        clientY: b,
        button: buttonFor(name, c),
        buttons,
        pressure: buttons ? 0.5 : 0,
      };
      if (typeof PointerEvent === "function") return new PointerEvent(name, init);
      // No PointerEvent class: a MouseEvent carrying the pointer fields reads the same.
      return withProps(new MouseEvent(name, init), { ...POINTER_INIT, pressure: init.pressure });
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
        return new MouseEvent(name, {
          ...common,
          ...M,
          clientX: a,
          clientY: b,
          button: buttonFor(name, c),
          buttons: buttonsFor(name, c),
          detail: detailFor(name, d),
        });
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
          return new (Composition as new (type: string, init: Record<string, unknown>) => VnEvent)(name, {
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
                              ...M,
                            };
  for (const k in extra) Object.defineProperty(ev, k, { value: (extra as ExtraProps)[k], configurable: true });
  return ev;
}

type ExtraProps = Record<string, unknown>;

function withProps(ev: VnEvent, props: Record<string, unknown>): VnEvent {
  for (const k in props) Object.defineProperty(ev, k, { value: props[k], configurable: true });
  return ev;
}

// Media event payload: a = currentTime, b = duration, c = readyState, d = playbackRate. For `error`, a is
// MediaError.code and b is currentTime; `e.target.error` is served live off `__vn_media_state`.
function mediaDetail(a: number, b: number, c: number, d: number) {
  return { currentTime: a, duration: b, readyState: c, playbackRate: d };
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

/** One listener call. A throwing listener does not end the dispatch: the error is reported from a microtask, after the walk. */
function callListener(fn: (this: unknown, ev: VnEvent) => unknown, self: unknown, ev: VnEvent, passive = false): void {
  // A passive listener's `preventDefault()` does nothing (`passiveOf`). An own
  // no-op shadows the prototype's for the length of the call.
  const own = passive ? Object.getOwnPropertyDescriptor(ev, "preventDefault") : undefined;
  if (passive) Object.defineProperty(ev, "preventDefault", { value: () => {}, configurable: true, writable: true });
  let traceToken = 0;
  try {
    traceToken = traceCallbackBegin?.(fn) ?? 0;
    fn.call(self, ev);
  } catch (e) {
    (globalThis.queueMicrotask ?? queueMicrotask)(() => {
      throw e;
    });
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
  ev: VnEvent,
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

function propagate(target: MirrorNode, name: string, ev: VnEvent): boolean {
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

/** An event targeted at the document (`selectionchange`): window capture, the document's own, then window bubble if it bubbles. */
function propagateAtDocument(name: string, ev: VnEvent): boolean {
  const document = globalThis.document as object | undefined;
  return document ? propagateAtGlobal(document, name, ev) : true;
}

function propagateAtGlobal(target: object, name: string, ev: VnEvent): boolean {
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

const CODE_TO_NAME: Record<number, string | undefined> = Object.fromEntries(
  Object.entries(EV_CODES).map(([k, v]) => [v, k]),
);

/**
 * Is anything on this path going to be CALLED for `name`? Several names share one host code (a `pointermove` listener
 * arms `mousemove`), and building a redjs `new MouseEvent(...)` is most of dispatch cost, so skip it when nothing
 * listens. Ignores `bubbles`: an extra event is cheaper than a dropped handler.
 */
function hasListener(target: MirrorNode, name: string): boolean {
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

const ACTIVATING_EVENTS = new Set(["keydown", "mousedown", "pointerdown", "mouseup", "pointerup", "click", "auxclick"]);

function activatableOnPath(target: MirrorNode): boolean {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (isLabel(n) || isFileInput(n) || isColorInput(n) || isLink(n) || isSubmitButton(n) || isResetButton(n))
      return true;
  }
  return false;
}

// HTML drag and drop: gpui has none, so the runtime runs the gesture off the host's mouse events: a left press on a
// `draggable="true"` element then a move past 4 px starts a drag; each move fires `drag`, `dragenter` / `dragleave`
// and `dragover` (cancelled: a drop target); release fires `drop` then `dragend` and swallows its `click`; Escape
// cancels. Divergences: DataTransfer stays read/write for the whole drag; no drag image; a drag ends at the window's
// edge.

const DRAG_THRESHOLD = 4;

interface DndGesture {
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
): VnEvent {
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
  let ev: VnEvent;
  try {
    const Drag = theGlobal("DragEvent") as (new (type: string, init: Record<string, unknown>) => VnEvent) | undefined;
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
  dnd = { phase: "pressed", source, x, y, dt: null, over: null, op: "none" };
  dndRearm();
}

function dndFinish(swallowClick: boolean): void {
  dnd = null;
  dndSwallowClick = swallowClick;
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
  nodeId: VnNodeId,
  x: number,
  y: number,
  d: number,
  key: string | undefined,
): boolean | undefined {
  if (dndSwallowClick && (name === "click" || name === "mousedown" || name === "keydown")) {
    dndSwallowClick = false;
    if (name === "click") return false;
  }
  if (name === "selectstart") return draggableAt(nodeById(nodeId)) ? false : undefined;
  const g = dnd;
  if (!g) return undefined;
  const held = !!(pointerButtons & 1);
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
      dndCancel(g, pointerPosition.x, pointerPosition.y, pointerPosition.modifiers, true);
      return false;
  }
  return undefined;
}

__vn_set_dispatcher((nodeId, evCode, a, b, c, d, key, data, x, y) => {
  const name = CODE_TO_NAME[evCode];
  if (!name) return true;
  if (name === "mousedown" || name === "mousemove" || name === "mouseup" || name === "pointercancel") {
    pointerPosition = { x: a, y: b, button: c, modifiers: d };
    pointerButtons = buttonsFor(name, c);
    // A lost native release (capture stolen by another window, for example)
    // must terminate the old gesture before an unpressed move is delivered.
    if (name === "mousemove" && !pointerButtons) {
      const captured = pointerCaptures.get(1);
      if (captured) {
        propagate(captured, "pointercancel", synthesize("pointercancel", a, b, -1, d, key));
        releaseCapturedPointer(1);
      }
    }
  }
  if (dnd || dndSwallowClick || (name === "selectstart" && dndSeen)) {
    const answer = dndDispatch(name, nodeId, a, b, d, key);
    if (answer !== undefined) return answer;
  }
  const retarget = name === "mousemove" || name === "mouseup" || name === "pointercancel";
  const target = (retarget && pointerCaptures.get(1)) || nodeById(nodeId);
  if (!target) return true;
  if (name === "keydown" && key === "Escape" && globalThis.__VN_WINDOW__?.escapeFullscreen?.()) return false;
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
    focusChange = ++focusChanges;
    arriving = null;
    focusRelated = (a && nodeById(a)) || null;
    if (name === "focus") focusedNode = target as MirrorElement;
    else if (focusedNode === target) focusedNode = null;
  }
  // Sticky activation: the windowing prelude arms it from the engine's own EventTarget, which gpui events never
  // reach, so the host's events call its hook here; without it `beforeunload` could never veto a close. The host
  // forwards only events something listens for, so a host `click`, `mouseup` or `pointerup` stands in for the
  // press. `el.click()` and `dispatchEvent` never come through here.
  if (ACTIVATING_EVENTS.has(name)) {
    globalThis.__vnStickyActivation?.();
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
        "an uncancelled `contextmenu` — a browser would open its own context menu here; vue-native has " +
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
        const overtaken = focusChanges !== focusChange;
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
        "vue-native's text field has already focused itself and placed the caret",
    );
  }
  if (!proceed && name === "wheel" && (target as MirrorElement).tag === "textarea") {
    warnCompat(
      "wheel-prevent:textarea",
      "`wheel.preventDefault()` on a <textarea> — a browser does not scroll it; vue-native's textarea has " +
        "already scrolled its own content (the boxes around it stay put)",
    );
  }
  if (name === "mouseup" || name === "pointercancel") releaseCapturedPointer(1);
  if (name === "mousedown" && dndSeen && !dnd) dndPress(target, a, b, c, proceed);
  return proceed;
});

const renderer = createRenderer({ ...nodeOps, patchProp });

function teardownDocumentRoot(root: MirrorElement, unmount?: () => void): void {
  if (!root.__vnId) return;
  try {
    unmount?.();
  } catch (e) {
    __vn_log("teardown: " + e);
  }
  pointerButtons = 0;
  for (const pointerId of Array.from(pointerCaptures.keys())) releaseCapturedPointer(pointerId);
  dnd = null;
  dndSeen = false;
  dndSwallowClick = false;
  __vn_remove(root.__vnId);
  // <KeepAlive> creates its cache container with createElement and never inserts or removes it; what is left in the
  // mirror after app.unmount() is such an orphan, freed here or the host arena grows by one node per KeepAlive on each
  // hot restart.
  for (const id of Array.from(nodesById.keys())) __vn_remove(id);
  // …and every loose subtree, held by script or not: the next app's nodes reuse these ids.
  for (const [id, r] of looseRoots) __vn_remove(r.box || id);
  for (const n of nodesById.values()) n.__vnId = 0;
  for (const ref of looseById.values()) {
    const n = ref.deref();
    if (n) n.__vnId = 0;
  }
  root.__vnId = 0;
  nodesById.clear();
  looseById.clear();
  looseRoots.clear();
  globalThis.__VN_CSS__ = [];
  globalThis.__VN_TEARDOWN__ = undefined;
  appRoot = null;
  observers.clear();
  for (const map of globalHandlers.values()) {
    for (const buckets of Object.values(map)) {
      if (buckets) for (const entry of [...buckets.extraC, ...buckets.extraB]) disposeGlobalListener(entry);
    }
  }
  globalHandlers.clear();
}

function ensureDocumentRoot(): MirrorElement {
  if (appRoot) return appRoot;
  // Named `html`, not `ROOT`: this root answers to `document.documentElement` and `document.body`, and DOM-walking
  // libraries stop at `html`: under any other name @floating-ui's `getOverflowAncestors` loops forever. The Rust tag
  // stays "root".
  const root = mk(__vn_create_root(), 1, "html");
  appRoot = root;
  globalThis.__VN_TEARDOWN__ = () => teardownDocumentRoot(root);
  // Listeners added to `document`/`window` at module scope had no root to carry their bit; sync them now.
  for (const m of globalHandlers.values()) for (const name of Object.keys(m)) syncListenerFlag(root, name);
  dndRearm();
  // Module-scope writes to `document.documentElement` (the `dark` class, `--radius`) land before first render.
  if (flushDocumentElement) flushDocumentElement(root);
  return root;
}

/**
 * What `createApp` hands back: Vue's `App`, except `mount()` without a container keeps the native window-root shorthand;
 * a selector or Element mounts there. It extends `App` rather than `Omit`-ing `mount`, since a mapped type resolves
 * `this` to the source type and breaks `createApp(App).use(router).mount()`.
 */
export interface VnApp extends App<MirrorElement> {
  mount(container?: unknown): ComponentPublicInstance;
}

export function createApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): VnApp {
  const app = renderer.createApp(rootComponent as never, rootProps);
  const mount = app.mount.bind(app);
  app.mount = (container?: unknown) => {
    if (globalThis.__VN_CSS__?.length) {
      installStylesheet(globalThis.__VN_CSS__.join("\n"));
    }
    const root = ensureDocumentRoot();
    let target: MirrorElement = root;
    let explicitContainer = container != null;
    if (typeof container === "string") {
      // Root-only native entries use '#app' without an index.html container: prefer a real element, else keep that alias.
      const resolved = (
        globalThis.document?.querySelector as ((selector: string) => MirrorElement | null) | undefined
      )?.(container);
      const legacyRoot = !resolved && container === "#app";
      if (legacyRoot) explicitContainer = false;
      const found = resolved ?? (legacyRoot ? root : null);
      if (!found) {
        warnCompat(
          "mount:container",
          `app.mount(${JSON.stringify(container)}) found no element; the app was not mounted`,
        );
        return undefined as never;
      }
      target = found as MirrorElement;
    } else if (container != null) {
      if (!isMirrorNode(container) || container.kind !== 1) {
        throw new TypeError("app.mount() expects an Element or a CSS selector");
      }
      target = container as MirrorElement;
    }
    if (explicitContainer) target.textContent = "";
    const vm = mount(target);
    if (explicitContainer) {
      target.removeAttribute("v-cloak");
      target.setAttribute("data-v-app", "");
    }
    globalThis.__VN_TEARDOWN__ = () => teardownDocumentRoot(root, () => app.unmount());
    __vn_log(`Vue ${version} app mounted (vue-native runtime)`);
    return vm;
  };
  return app as VnApp;
}

// runtime-dom's other exports: a missing name breaks the build of every app using it (`useCssVars` is what `v-bind()`
// in `<style>` compiles to). Each exists: the real thing where possible, an advisory where not.

export const render = renderer.render;

export { nodeOps, patchProp };

/** Writes the `--hash-name` custom properties a `v-bind()` in `<style>` reads onto the component's root element(s); teleported children are found by `data-v-owner`. */
export function useCssVars(getter: (ctx: unknown) => Record<string, unknown>): void {
  const instance = getCurrentInstance() as unknown as CssVarsInstance | null;
  if (!instance) return;
  const doc = (globalThis as { document?: { querySelectorAll?(s: string): ArrayLike<MirrorElement> } }).document;
  const updateTeleports = (instance.ut = (vars: Record<string, unknown> = getter(instance.proxy)) => {
    const owned = doc?.querySelectorAll?.(`[data-v-owner="${instance.uid}"]`) ?? [];
    Array.from(owned).forEach((node) => setVarsOnNode(node, vars));
  });
  const setVars = () => {
    const vars = getter(instance.proxy);
    if (instance.ce) setVarsOnNode(instance.ce as MirrorElement, vars);
    else setVarsOnVNode(instance.subTree, vars);
    updateTeleports(vars);
  };
  onBeforeUpdate(() => {
    queuePostFlushCb(setVars);
  });
  onMounted(() => {
    watch(setVars, () => {}, { flush: "post" });
    const MO = (
      globalThis as {
        MutationObserver?: new (cb: () => void) => { observe(n: unknown, o: unknown): void; disconnect(): void };
      }
    ).MutationObserver;
    const parent = (instance.subTree?.el as MirrorNode | null)?.parentNode;
    if (MO && parent) {
      const ob = new MO(setVars);
      ob.observe(parent, { childList: true });
      onUnmounted(() => ob.disconnect());
    }
  });
}
interface CssVarsInstance {
  uid: number;
  proxy: unknown;
  ce?: unknown;
  subTree: VNode;
  ut?: (vars?: Record<string, unknown>) => void;
}
function setVarsOnVNode(vnode: VNode, vars: Record<string, unknown>): void {
  if (vnode.shapeFlag & 128 /* SUSPENSE */) {
    const suspense = vnode.suspense as unknown as {
      activeBranch: VNode;
      pendingBranch: VNode | null;
      isHydrating: boolean;
      effects: (() => void)[];
    };
    vnode = suspense.activeBranch;
    if (suspense.pendingBranch && !suspense.isHydrating) {
      suspense.effects.push(() => setVarsOnVNode(suspense.activeBranch, vars));
    }
  }
  while (vnode.component) vnode = vnode.component.subTree;
  if (vnode.shapeFlag & 1 /* ELEMENT */ && vnode.el) {
    setVarsOnNode(vnode.el as MirrorElement, vars);
  } else if (vnode.type === Fragment) {
    (vnode.children as VNode[]).forEach((c) => setVarsOnVNode(c, vars));
  } else if (vnode.type === Static) {
    let el = vnode.el as MirrorNode | null;
    const anchor = vnode.anchor as MirrorNode | null;
    while (el) {
      setVarsOnNode(el as MirrorElement, vars);
      if (el === anchor) break;
      el = nodeOps.nextSibling(el);
    }
  }
}
function setVarsOnNode(el: MirrorElement, vars: Record<string, unknown>): void {
  if (el.nodeType !== 1) return;
  // runtime-dom's `CSS_VAR_TEXT`: `patchStyle` appends this to a string
  // `:style`, so a component's `v-bind()` variables survive it.
  let cssText = "";
  for (const key in vars) {
    const value = normalizeCssVarValue(vars[key]);
    el.style.setProperty(`--${key}`, value);
    cssText += `--${key}: ${value};`;
  }
  inlineOf(el).cssVarText = cssText;
}
function normalizeCssVarValue(value: unknown): string {
  if (value == null) return "initial";
  if (typeof value === "string") return value === "" ? " " : value;
  return String(value);
}

export function useCssModule(name = "$style"): Record<string, string> {
  const instance = getCurrentInstance();
  if (!instance) return {};
  const modules = (instance.type as { __cssModules?: Record<string, Record<string, string>> }).__cssModules;
  const mod = modules?.[name];
  if (!mod) {
    warnCompat(
      `cssModule:${name}`,
      `useCssModule(${JSON.stringify(name)}) — no CSS module of that name: vue-native's builders do not ` +
        `compile <style module> yet, so this answers {} and every class lookup is undefined`,
    );
    return {};
  }
  return mod;
}

function notSupported(msg: string): Error {
  return typeof DOMException === "function"
    ? new DOMException(msg, "NotSupportedError")
    : Object.assign(new Error(msg), { name: "NotSupportedError" });
}
const NO_CUSTOM_ELEMENTS =
  "vue-native has no custom elements (no customElements registry, no shadow DOM) — " +
  "use the component directly instead of defining an element for it";

export class VueElement {
  constructor() {
    warnCompat("customElement", `VueElement — ${NO_CUSTOM_ELEMENTS}`);
    throw notSupported(`VueElement: ${NO_CUSTOM_ELEMENTS}`);
  }
}
export function defineCustomElement(options: unknown, extraOptions?: unknown): typeof VueElement {
  warnCompat("defineCustomElement", `defineCustomElement() — ${NO_CUSTOM_ELEMENTS}`);
  const def = defineComponent(options as never, extraOptions as never);
  return class VueCustomElement extends VueElement {
    static def = def;
  };
}
export const defineSSRCustomElement = (options: unknown, extraOptions?: unknown): typeof VueElement =>
  defineCustomElement(options, extraOptions);
export function useHost(): null {
  warnCompat("useHost", `useHost() — ${NO_CUSTOM_ELEMENTS}; it answers null`);
  return null;
}
export function useShadowRoot(): null {
  warnCompat("useShadowRoot", `useShadowRoot() — ${NO_CUSTOM_ELEMENTS}; it answers null`);
  return null;
}
export function createSSRApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): VnApp {
  warnCompat(
    "createSSRApp",
    "createSSRApp() — a vue-native window has no server-rendered markup to hydrate; it behaves as createApp()",
  );
  return createApp(rootComponent, rootProps);
}
export const hydrate: typeof render = (vnode, container) => {
  warnCompat(
    "hydrate",
    "hydrate() — nothing to hydrate in a vue-native window; it renders from scratch, as render() does",
  );
  render(vnode, container);
};
export const initDirectivesForSSR = (): void => {};

// Missing globals are ReferenceErrors, worse than a wrong answer: reka-ui's FocusScope constructs a
// MutationObserver in a setup-time watchEffect, so every modal would die before rendering. Installed
// unconditionally when the engine lacks them.

// A previous runtime's install counts as vacant (see `REALM`): a same-realm reload replaces it.
const defineGlobal = (name: string, value: unknown) => {
  if (vacant(globalThis, name)) {
    Object.defineProperty(globalThis, name, { value: mark(value), configurable: true, writable: true });
  }
};

// The `CSS` namespace. reka-ui's `Presence` calls `CSS.escape` inside an event handler; without it the exit
// animation never completes and the element never unmounts. `escape` is CSSOM's algorithm, not an
// approximation: it feeds a string comparison, and an under-escaped identifier silently stops matching.
defineGlobal("CSS", {
  escape(value: unknown) {
    const s = String(value);
    let out = "";
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      const ch = s[i];
      if (c === 0) {
        out += "�";
      } else if (
        (c >= 0x1 && c <= 0x1f) ||
        c === 0x7f ||
        (i === 0 && c >= 0x30 && c <= 0x39) ||
        (i === 1 && c >= 0x30 && c <= 0x39 && s.charCodeAt(0) === 0x2d)
      ) {
        out += "\\" + c.toString(16) + " ";
      } else if (i === 0 && c === 0x2d && s.length === 1) {
        out += "\\" + ch;
      } else if (
        c >= 0x80 ||
        c === 0x2d ||
        c === 0x5f ||
        (c >= 0x30 && c <= 0x39) ||
        (c >= 0x41 && c <= 0x5a) ||
        (c >= 0x61 && c <= 0x7a)
      ) {
        out += ch;
      } else {
        out += "\\" + ch;
      }
    }
    return out;
  },
  // A feature probe: answers what vue-native can express (the properties the cascade maps), since `@supports`
  // fallbacks are chosen with it.
  supports(a: unknown, b?: unknown) {
    const body = b === undefined ? String(a) : `${String(a)}:${String(b)}`;
    const colon = body.indexOf(":");
    const name = (colon < 0 ? body : body.slice(0, colon)).trim().toLowerCase();
    if (!name) return false;
    if (name.startsWith("--")) return true;
    if (!CSS_SUPPORTED.has(name)) return false;
    return colon < 0 || !CSS_UNSUPPORTED_VALUE.test(body.slice(colon + 1));
  },
  registerProperty() {
    warnCompat(
      "CSS.registerProperty",
      "CSS.registerProperty() is a no-op — vue-native's custom properties are untyped strings, " +
        "so a registered syntax/inherits/initialValue has nothing to enforce",
    );
  },
});

/// Properties `CSS.supports` answers `true` for. Conservative: a probe that
/// says yes and then does nothing is worse than one that says no.
const CSS_SUPPORTED = new Set<string>([
  "animation",
  "animation-delay",
  "animation-direction",
  "animation-duration",
  "animation-fill-mode",
  "animation-iteration-count",
  "animation-name",
  "animation-play-state",
  "animation-timing-function",
  "transition",
  "transition-delay",
  "transition-duration",
  "transition-property",
  "transition-timing-function",
  "align-items",
  "align-self",
  "background",
  "background-color",
  "border",
  "border-color",
  "border-radius",
  "border-width",
  "bottom",
  "box-shadow",
  "color",
  "column-gap",
  "display",
  "flex",
  "flex-basis",
  "flex-direction",
  "flex-grow",
  "flex-shrink",
  "flex-wrap",
  "font-size",
  "font-style",
  "font-weight",
  "gap",
  "height",
  "inset",
  "justify-content",
  "left",
  "line-height",
  "margin",
  "margin-bottom",
  "margin-left",
  "margin-right",
  "margin-top",
  "max-height",
  "max-width",
  "min-height",
  "min-width",
  "opacity",
  "overflow",
  "overflow-x",
  "overflow-y",
  "padding",
  "padding-bottom",
  "padding-left",
  "padding-right",
  "padding-top",
  "pointer-events",
  "position",
  "right",
  "row-gap",
  "text-align",
  "top",
  "transform",
  "visibility",
  "white-space",
  "width",
  "z-index",
]);

// Values `CSS.supports` rejects for a known property: `linear(…)` easing, which the parser lacks (stylesheets drop it,
// `el.animate()` refuses it), so apps probe and fall back to `cubic-bezier`.
const CSS_UNSUPPORTED_VALUE = /(?:^|[^\w-])linear\(/i;

interface MutationRecordLike {
  type: "childList" | "attributes";
  target: MirrorNode;
  addedNodes: MirrorCollection<MirrorNode>;
  removedNodes: MirrorCollection<MirrorNode>;
  attributeName: string | null;
  attributeNamespace: null;
  oldValue: string | null;
  previousSibling: null;
  nextSibling: null;
}
interface MutationTargetEntry {
  node: MirrorElement;
  subtree: boolean;
  options: {
    childList: boolean;
    attributes: boolean;
    attributeOldValue: boolean;
    attributeFilter: string[] | null;
  };
}
interface MutationObserverInitLike {
  subtree?: boolean;
  childList?: boolean;
  attributes?: boolean;
  attributeOldValue?: boolean;
  attributeFilter?: string[];
  characterData?: boolean;
}
interface MutationObserverLike {
  _callback: (records: MutationRecordLike[], ob: MutationObserverLike) => void;
  _targets: MutationTargetEntry[];
  _records: MutationRecordLike[];
  takeRecords(): MutationRecordLike[];
}

const observers = new Set<MutationObserverLike>();
let mutationFlushQueued = false;

function notifyMutation(
  target: MirrorNode,
  type: "childList" | "attributes",
  extra?: {
    addedNodes?: MirrorNode[];
    removedNodes?: MirrorNode[];
    attributeName?: string;
    oldValue?: string | null;
  },
): void {
  domEpoch++;
  if (type === "attributes" && !noAttrNodes)
    attrChanged(target as MirrorElement, extra!.attributeName!, extra!.oldValue ?? null);
  if (observers.size === 0) return;
  for (const ob of observers) {
    for (const entry of ob._targets) {
      const inScope = entry.node === target || (entry.subtree && isAncestor(entry.node, target));
      if (!inScope) continue;
      if (type === "childList" && !entry.options.childList) continue;
      if (type === "attributes") {
        if (!entry.options.attributes) continue;
        if (entry.options.attributeFilter && !entry.options.attributeFilter.includes(extra!.attributeName!)) {
          continue;
        }
      }
      ob._records.push(
        branded("MutationRecord", {
          type,
          target,
          addedNodes: staticList(NodeListProto, extra?.addedNodes ?? NO_NODES),
          removedNodes: staticList(NodeListProto, extra?.removedNodes ?? NO_NODES),
          attributeName: extra?.attributeName ?? null,
          attributeNamespace: null,
          oldValue: entry.options.attributeOldValue ? (extra?.oldValue ?? null) : null,
          previousSibling: null,
          nextSibling: null,
        }),
      );
      break; // one record per observer per mutation, as in the DOM
    }
  }
  if (!mutationFlushQueued) {
    mutationFlushQueued = true;
    Promise.resolve().then(flushMutations);
  }
}

function isAncestor(ancestor: MirrorNode, node: MirrorNode | null | undefined): boolean {
  for (let n: MirrorElement | null | undefined = node?.parentNode; n; n = n.parentNode) if (n === ancestor) return true;
  return false;
}

function flushMutations() {
  mutationFlushQueued = false;
  for (const ob of [...observers]) {
    if (!ob._records.length) continue;
    const records = ob.takeRecords();
    try {
      ob._callback(records, ob);
    } catch (e) {
      console.error(`[vue-native] MutationObserver callback threw: ${e}`);
    }
  }
}

defineGlobal(
  "MutationObserver",
  class MutationObserver implements MutationObserverLike {
    // `declare` so the fields are ERASED rather than emitted as a constructor
    // prologue that would run before the assignments below.
    declare _callback: MutationObserverLike["_callback"];
    declare _targets: MutationTargetEntry[];
    declare _records: MutationRecordLike[];
    constructor(callback: MutationObserverLike["_callback"]) {
      this._callback = callback;
      this._targets = [];
      this._records = [];
    }
    observe(node: unknown, options: MutationObserverInitLike = {}) {
      // `observe(document, {subtree: true})` is how libraries (reka, floating-ui)
      // say "watch everything". A Document is not an element here, so watch the
      // root. Duck-typed: the same code runs in the test harness's private world.
      if (node && (node as MirrorNode).kind !== 1 && (node as VnDocument).documentElement) {
        node =
          ((node as VnDocument).documentElement as MirrorNode).kind === 1
            ? (node as VnDocument).documentElement
            : (appRoot ?? node);
      }
      if (!node || (node as MirrorNode).kind !== 1) {
        warnCompat(
          "MutationObserver:target",
          "MutationObserver.observe() was given something that is not a rendered element — " +
            "only elements (and `document`, which is mapped to the root element) have a " +
            "subtree in vue-native, so nothing under it is observed",
        );
        return;
      }
      if (options.characterData) {
        warnCompat(
          "MutationObserver:characterData",
          "MutationObserver characterData records are not delivered — the retained tree carries " +
            "text as node content, not as a mutation; childList and attributes are",
        );
      }
      this._targets.push({
        node: node as MirrorElement,
        subtree: !!options.subtree,
        options: {
          childList: !!options.childList,
          attributes: !!options.attributes || !!options.attributeFilter,
          attributeOldValue: !!options.attributeOldValue,
          attributeFilter: options.attributeFilter ?? null,
        },
      });
      observers.add(this);
    }
    disconnect() {
      this._targets.length = 0;
      this._records.length = 0;
      observers.delete(this);
    }
    takeRecords() {
      return this._records.splice(0, this._records.length);
    }
  },
);

// ResizeObserver / IntersectionObserver run after layout and before paint (HTML "update the rendering"): a callback
// sees the size the frame is about to paint, and what it changes paints in that frame. The host runs one step per
// frame through `__vn_set_layout_hook`: it calls `layoutStep` once per op, drains microtasks after each call, and lays
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
interface ResizeObserverLike {
  _callback: (this: ResizeObserverLike, entries: ResizeEntryLike[], ob: ResizeObserverLike) => void;
  _targets: Map<MirrorElement, ResizeSize | null>;
  _active: [MirrorElement, ResizeSize, number][];
  _skipped: boolean;
}

const resizeObservers: ResizeObserverLike[] = [];
let resizeDue: ResizeObserverLike[] = [];
let resizeDepth = Infinity;

/** Nodes from `node` to its tree's root, itself included: a root or detached node is 1, so the first gather (depth 0) takes every target. */
function nodeDepth(node: MirrorNode): number {
  let d = 1;
  for (let n = node.parentNode; n; n = n.parentNode) d++;
  return d;
}

function gatherResizes(depth: number): number {
  resizeDue = [];
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

function broadcastNextResize(): number {
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
function deliverResizeLoopError(): void {
  const g = globalThis as unknown as {
    onerror?: unknown;
    dispatchEvent?: (e: VnEvent) => boolean;
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
const ErrorEventClass: (new (type: string, init?: ErrorEventInit) => VnEvent) | null = (() => {
  const g = globalThis as { ErrorEvent?: unknown };
  if (typeof g.ErrorEvent === "function" && !vacant(g, "ErrorEvent"))
    return g.ErrorEvent as new (type: string, init?: ErrorEventInit) => VnEvent;
  if (typeof Event !== "function") return null;
  const fields = new WeakMap<
    object,
    Required<Pick<ErrorEventInit, "message" | "filename" | "lineno" | "colno">> & { error: unknown }
  >();
  const field = <K extends "message" | "filename" | "lineno" | "colno" | "error">(ev: object, k: K, empty: unknown) =>
    fields.has(ev) ? fields.get(ev)![k] : empty;
  class ErrorEvent extends Event {
    constructor(type: string, init: ErrorEventInit = {}) {
      super(type, init);
      fields.set(this, {
        message: String(init.message ?? ""),
        filename: String(init.filename ?? ""),
        lineno: Number(init.lineno ?? 0) >>> 0,
        colno: Number(init.colno ?? 0) >>> 0,
        error: init.error === undefined ? null : init.error,
      });
    }
    get message(): string {
      return field(this, "message", "") as string;
    }
    get filename(): string {
      return field(this, "filename", "") as string;
    }
    get lineno(): number {
      return field(this, "lineno", 0) as number;
    }
    get colno(): number {
      return field(this, "colno", 0) as number;
    }
    get error(): unknown {
      return field(this, "error", null);
    }
  }
  try {
    Object.defineProperty(globalThis, "ErrorEvent", { value: mark(ErrorEvent), configurable: true, writable: true });
  } catch {
    /* a frozen global: the class still serves this module */
  }
  return ErrorEvent as unknown as new (type: string, init?: ErrorEventInit) => VnEvent;
})();

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
const hostSteps = typeof __vn_set_layout_hook === "function" && typeof __vn_layout_watch === "function";
if (hostSteps) __vn_set_layout_hook(layoutStep);
const QUIET_STEPS = 2;
let fallbackQueued = false;
let fallbackQuiet = 0;
let fallbackTimer: unknown = null;
const layoutTimers = globalThis as {
  setTimeout?: (cb: () => void, ms: number) => unknown;
  clearTimeout?: (id: unknown) => void;
};

function layoutObserved(): void {
  if (hostSteps) {
    __vn_layout_watch(true);
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
      if (!node || typeof node.__vnId !== "number") return;
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

// IntersectionObserver, from the host's painted bounds. redjs's is inert, which is not harmless: @vueuse/motion's
// `visible` variants wait for `isIntersecting`, so elements that start at `opacity: 0` stay invisible forever and lazy
// loaders never load.
// Runs the W3C update steps in the frame step after layout (FINISH op): queues an entry when the threshold index or
// `isIntersecting` moves (so the first update always reports), calls back from a task after paint. Rectangles are
// where boxes were DRAWN, as getBoundingClientRect.
// Divergence: clipped by the ROOT only, not by intermediate scroll ancestors (a host call per ancestor per frame), so
// a target scrolled out of an inner scroller but inside the window reads as intersecting.
interface IntersectionTargetState {
  prevIndex: number;
  prevIntersecting: boolean;
}
interface IntersectionEntryInit {
  time: number;
  target: MirrorElement;
  rootBounds: MirrorRect | null;
  boundingClientRect: MirrorRect;
  intersectionRect: MirrorRect;
  isIntersecting: boolean;
  intersectionRatio: number;
}
const intersectionObservers = new Set<VnIntersectionObserver>();
let intersectionTaskQueued = false;
let intersectionsQueued = false;

function updateIntersections(): void {
  const now = (globalThis as { performance?: { now(): number } }).performance?.now() ?? Date.now();
  let queued = false;
  for (const ob of [...intersectionObservers]) {
    if (!ob._targets.size) {
      intersectionObservers.delete(ob);
      continue;
    }
    if (ob._update(now)) queued = true;
  }
  intersectionsQueued = queued;
  if (!queued || intersectionTaskQueued) return;
  // "Queue an intersection observer task": the next pump's timers, after this
  // frame has painted.
  const later =
    (globalThis as { setTimeout?: (cb: () => void, ms: number) => unknown }).setTimeout ??
    (typeof setTimeout === "function" ? setTimeout : undefined);
  if (typeof later !== "function") return;
  intersectionTaskQueued = true;
  later(notifyIntersectionObservers, 0);
}

function notifyIntersectionObservers(): void {
  intersectionTaskQueued = false;
  for (const ob of [...intersectionObservers]) {
    const records = ob.takeRecords();
    if (!records.length) continue;
    try {
      ob._callback.call(ob, records, ob);
    } catch (e) {
      (globalThis.queueMicrotask ?? queueMicrotask)(() => {
        throw e;
      });
    }
  }
}

const ioRect = (x: number, y: number, width: number, height: number): MirrorRect => ({
  x,
  y,
  width,
  height,
  top: y,
  left: x,
  right: x + width,
  bottom: y + height,
  toJSON() {
    return this;
  },
});

function parseIntersectionMargin(text: unknown, what: string): { v: number; pct: boolean }[] {
  const parts = String(text ?? "0px")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  const out = parts.map((p) => {
    const m = /^(-?\d*\.?\d+)(px|%)?$/.exec(p);
    if (!m || (!m[2] && Number(m[1]) !== 0)) {
      throw new DOMException(
        `Failed to construct 'IntersectionObserver': ${what} must be specified in pixels or percent.`,
        "SyntaxError",
      );
    }
    return { v: Number(m[1]), pct: m[2] === "%" };
  });
  if (out.length < 1 || out.length > 4) {
    throw new DOMException(
      `Failed to construct 'IntersectionObserver': ${what} must have 1 to 4 values.`,
      "SyntaxError",
    );
  }
  const [t, r = t, b = t, l = r] = out as [{ v: number; pct: boolean }, ...{ v: number; pct: boolean }[]];
  return [t, r!, b!, l!];
}

class VnIntersectionObserver {
  declare _callback: (entries: unknown[], ob: VnIntersectionObserver) => void;
  declare _targets: Map<MirrorElement, IntersectionTargetState>;
  declare _records: unknown[];
  declare _margin: { v: number; pct: boolean }[];
  declare root: MirrorElement | null;
  declare rootMargin: string;
  declare scrollMargin: string;
  declare thresholds: readonly number[];
  declare delay: number;
  declare trackVisibility: boolean;
  constructor(
    callback: (entries: unknown[], ob: VnIntersectionObserver) => void,
    options: Record<string, unknown> = {},
  ) {
    if (typeof callback !== "function") {
      throw new TypeError("Failed to construct 'IntersectionObserver': parameter 1 is not of type 'Function'.");
    }
    const opts = options ?? {};
    const root = opts.root ?? null;
    const isDocument =
      !!root &&
      (root === (globalThis as { document?: unknown }).document || (root as { nodeType?: number }).nodeType === 9);
    if (root !== null && !isDocument && typeof (root as { nodeType?: number }).nodeType !== "number") {
      throw new TypeError("Failed to construct 'IntersectionObserver': root must be an Element, a Document or null.");
    }
    const raw = opts.threshold === undefined ? [0] : Array.isArray(opts.threshold) ? opts.threshold : [opts.threshold];
    const thresholds = (raw as unknown[]).map(Number);
    for (const t of thresholds) {
      if (!(t >= 0 && t <= 1)) {
        throw new RangeError(
          "Failed to construct 'IntersectionObserver': Threshold values must be numbers between 0 and 1",
        );
      }
    }
    thresholds.sort((a, b) => a - b);
    this._callback = callback;
    this._targets = new Map();
    this._records = [];
    this._margin = parseIntersectionMargin(opts.rootMargin ?? "0px", "rootMargin");
    this.root = isDocument ? null : (root as MirrorElement | null);
    this.rootMargin = this._margin.map((m) => `${m.v}${m.pct ? "%" : "px"}`).join(" ");
    this.scrollMargin = parseIntersectionMargin(opts.scrollMargin ?? "0px", "scrollMargin")
      .map((m) => `${m.v}${m.pct ? "%" : "px"}`)
      .join(" ");
    this.thresholds = Object.freeze(thresholds.length ? thresholds : [0]);
    this.delay = Number(opts.delay ?? 0) || 0;
    this.trackVisibility = !!opts.trackVisibility;
  }
  observe(target: MirrorElement) {
    if (!target || typeof target !== "object") {
      throw new TypeError(
        "Failed to execute 'observe' on 'IntersectionObserver': parameter 1 is not of type 'Element'.",
      );
    }
    if (this._targets.has(target)) return;
    this._targets.set(target, { prevIndex: -1, prevIntersecting: false });
    intersectionObservers.add(this);
    layoutObserved();
  }
  unobserve(target: MirrorElement) {
    this._targets.delete(target);
  }
  disconnect() {
    this._targets.clear();
    this._records.length = 0;
    intersectionObservers.delete(this);
  }
  takeRecords(): unknown[] {
    return this._records.splice(0, this._records.length);
  }
  _rootBounds(): MirrorRect | null {
    let r: MirrorRect;
    if (this.root) {
      if (!boundsOf(this.root)) return null;
      r = paintedRectOf(this.root);
    } else {
      if (!appRoot) return null;
      r = viewportRect();
    }
    const [t, rt, b, l] = this._margin as [
      { v: number; pct: boolean },
      { v: number; pct: boolean },
      { v: number; pct: boolean },
      { v: number; pct: boolean },
    ];
    const px = (m: { v: number; pct: boolean }, of: number) => (m.pct ? (m.v / 100) * of : m.v);
    const top = r.top - px(t, r.height);
    const left = r.left - px(l, r.width);
    const right = r.right + px(rt, r.width);
    const bottom = r.bottom + px(b, r.height);
    return ioRect(left, top, Math.max(0, right - left), Math.max(0, bottom - top));
  }
  _update(time: number): boolean {
    const rootBounds = this._rootBounds();
    let queued = false;
    for (const [target, state] of this._targets) {
      const painted = !!boundsOf(target);
      let isIntersecting = false;
      let thresholdIndex = 0;
      let ratio = 0;
      let targetRect = ioRect(0, 0, 0, 0);
      let intersectionRect = ioRect(0, 0, 0, 0);
      const descendant = (() => {
        if (!this.root) return true;
        for (let n: MirrorNode | null | undefined = target.parentNode; n; n = n.parentNode)
          if (n === this.root) return true;
        return false;
      })();
      if (painted && rootBounds && descendant) {
        targetRect = paintedRectOf(target);
        const l = Math.max(targetRect.left, rootBounds.left);
        const t = Math.max(targetRect.top, rootBounds.top);
        const r = Math.min(targetRect.right, rootBounds.right);
        const b = Math.min(targetRect.bottom, rootBounds.bottom);
        isIntersecting = r >= l && b >= t;
        if (isIntersecting) intersectionRect = ioRect(l, t, Math.max(0, r - l), Math.max(0, b - t));
        const targetArea = targetRect.width * targetRect.height;
        const area = intersectionRect.width * intersectionRect.height;
        ratio = targetArea > 0 ? area / targetArea : isIntersecting ? 1 : 0;
        const at = this.thresholds.findIndex((th) => th > ratio);
        thresholdIndex = at < 0 ? this.thresholds.length : at;
      }
      if (thresholdIndex === state.prevIndex && isIntersecting === state.prevIntersecting) continue;
      state.prevIndex = thresholdIndex;
      state.prevIntersecting = isIntersecting;
      this._records.push(
        new VnIntersectionObserverEntry({
          time,
          target,
          rootBounds,
          boundingClientRect: targetRect,
          intersectionRect,
          isIntersecting,
          intersectionRatio: ratio,
        }),
      );
      queued = true;
    }
    return queued;
  }
}

class VnIntersectionObserverEntry {
  declare time: number;
  declare target: MirrorElement;
  declare rootBounds: MirrorRect | null;
  declare boundingClientRect: MirrorRect;
  declare intersectionRect: MirrorRect;
  declare isIntersecting: boolean;
  declare intersectionRatio: number;
  declare isVisible: boolean;
  constructor(init: IntersectionEntryInit) {
    this.time = init.time;
    this.target = init.target;
    this.rootBounds = init.rootBounds;
    this.boundingClientRect = init.boundingClientRect;
    this.intersectionRect = init.intersectionRect;
    this.isIntersecting = init.isIntersecting;
    this.intersectionRatio = init.intersectionRatio;
    this.isVisible = false;
  }
}

// OVER redjs's inert pair, not beside it — `defineGlobal` only fills a hole.
for (const [name, value] of [
  ["IntersectionObserver", VnIntersectionObserver],
  ["IntersectionObserverEntry", VnIntersectionObserverEntry],
] as const) {
  try {
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  } catch {
    try {
      (globalThis as Record<string, unknown>)[name] = value;
    } catch {
      warnCompat(`global:${name}`, `${name} could not be installed over the engine's own; it stays inert`);
    }
  }
}

// DocumentFragment: Vue's Teleport accepts one as a target and reka-ui's Select constructs one even while closed.
// Ours is a detached mirror element (loose, `looseCreated`); inserting it inserts its children instead (`insertAll`).
{
  class DocumentFragment {
    constructor() {
      const node = mk(__vn_create_element(I("fragment")), 1, "fragment");
      Object.setPrototypeOf(node, DocumentFragment.prototype);
      fragments.add(node);
      // `new DocumentFragment()` deliberately returns a different object: a detached mirror element.
      return looseCreated(node) as unknown as DocumentFragment;
    }
  }
  Object.setPrototypeOf(DocumentFragment.prototype, ElementProto);
  for (const [k, v] of [
    ["nodeType", 11],
    ["nodeName", "#document-fragment"],
  ] as const) {
    Object.defineProperty(DocumentFragment.prototype, k, { get: () => v, configurable: true });
  }
  defineGlobal("DocumentFragment", DocumentFragment);
}

// -- NodeFilter. reka's FocusScope calls `createTreeWalker(root, NodeFilter.SHOW_ELEMENT, …)`.
defineGlobal("NodeFilter", {
  SHOW_ALL: 0xffffffff,
  SHOW_ELEMENT: 1,
  SHOW_TEXT: 4,
  SHOW_COMMENT: 128,
  FILTER_ACCEPT: 1,
  FILTER_REJECT: 2,
  FILTER_SKIP: 3,
});

// Tag-specific element classes: redjs ships only Element/HTMLElement/SVGElement and reka-ui brands nodes with
// `instanceof HTMLInputElement`. A missing class is a ReferenceError, worse than a false `instanceof`, so each exists
// and answers for its tag(s) by `tagName`. HTMLMediaElement precedes the two that extend it (`TAG_PARENTS`).
const TAG_CLASSES: Record<string, string | readonly string[]> = {
  HTMLInputElement: "INPUT",
  HTMLTextAreaElement: "TEXTAREA",
  HTMLSelectElement: "SELECT",
  HTMLButtonElement: "BUTTON",
  HTMLAnchorElement: "A",
  HTMLFormElement: "FORM",
  HTMLLabelElement: "LABEL",
  HTMLDivElement: "DIV",
  HTMLSpanElement: "SPAN",
  HTMLImageElement: "IMG",
  HTMLIFrameElement: "IFRAME",
  HTMLCanvasElement: "CANVAS",
  HTMLMediaElement: ["VIDEO", "AUDIO"],
  HTMLVideoElement: "VIDEO",
  HTMLAudioElement: "AUDIO",
  HTMLSourceElement: "SOURCE",
  HTMLTrackElement: "TRACK",
  HTMLPictureElement: "PICTURE",
  HTMLStyleElement: "STYLE",
  HTMLLinkElement: "LINK",
  HTMLScriptElement: "SCRIPT",
  HTMLMetaElement: "META",
  HTMLTitleElement: "TITLE",
  HTMLBaseElement: "BASE",
  HTMLHeadElement: "HEAD",
  HTMLBodyElement: "BODY",
  HTMLHtmlElement: "HTML",
  HTMLParagraphElement: "P",
  HTMLHeadingElement: ["H1", "H2", "H3", "H4", "H5", "H6"],
  HTMLPreElement: ["PRE", "LISTING", "XMP"],
  HTMLQuoteElement: ["Q", "BLOCKQUOTE"],
  HTMLHRElement: "HR",
  HTMLBRElement: "BR",
  HTMLUListElement: "UL",
  HTMLOListElement: "OL",
  HTMLLIElement: "LI",
  HTMLDListElement: "DL",
  HTMLMenuElement: "MENU",
  HTMLTableElement: "TABLE",
  HTMLTableCaptionElement: "CAPTION",
  HTMLTableSectionElement: ["THEAD", "TBODY", "TFOOT"],
  HTMLTableRowElement: "TR",
  HTMLTableCellElement: ["TD", "TH"],
  HTMLTableColElement: ["COL", "COLGROUP"],
  HTMLOptionElement: "OPTION",
  HTMLOptGroupElement: "OPTGROUP",
  HTMLDataListElement: "DATALIST",
  HTMLFieldSetElement: "FIELDSET",
  HTMLLegendElement: "LEGEND",
  HTMLOutputElement: "OUTPUT",
  HTMLProgressElement: "PROGRESS",
  HTMLMeterElement: "METER",
  HTMLDetailsElement: "DETAILS",
  HTMLDialogElement: "DIALOG",
  HTMLTemplateElement: "TEMPLATE",
  HTMLSlotElement: "SLOT",
  HTMLEmbedElement: "EMBED",
  HTMLObjectElement: "OBJECT",
  HTMLMapElement: "MAP",
  HTMLAreaElement: "AREA",
  HTMLTimeElement: "TIME",
  HTMLDataElement: "DATA",
  HTMLModElement: ["INS", "DEL"],
};
const TAG_PARENTS: Record<string, string> = {
  HTMLVideoElement: "HTMLMediaElement",
  HTMLAudioElement: "HTMLMediaElement",
};
const MEDIA_CONSTANTS: readonly (readonly [string, number])[] = [
  ["NETWORK_EMPTY", 0],
  ["NETWORK_IDLE", 1],
  ["NETWORK_LOADING", 2],
  ["NETWORK_NO_SOURCE", 3],
  ["HAVE_NOTHING", 0],
  ["HAVE_METADATA", 1],
  ["HAVE_CURRENT_DATA", 2],
  ["HAVE_FUTURE_DATA", 3],
  ["HAVE_ENOUGH_DATA", 4],
];
// Form-control `value` / `checked` are real accessors: reka-ui's hidden inputs call
// `Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, v)` and dispatch `input`/`change`
// themselves. A mirror element's own `value` reaches the host, and the tag classes carry OWN accessors delegating to
// it (`TAG_ACCESSORS`), refusing a wrong receiver with "Illegal invocation".

function isMirrorEl(x: unknown): x is MirrorElement {
  return !!x && typeof x === "object" && Object.prototype.hasOwnProperty.call(x, "_value");
}

function isRangeInput(el: MirrorElement): boolean {
  return el.kind === 1 && el.tag === "input" && String(el.attrs?.type ?? "").toLowerCase() === "range";
}
function rangeNumber(value: unknown): number | undefined {
  const s = String(value ?? "");
  if (!/^-?(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$/.test(s)) return undefined;
  const n = Number(s);
  return Number.isFinite(n) ? n : undefined;
}
function rangeSpec(el: MirrorElement): { min: number; max: number; step: number | null; base: number } {
  const min = rangeNumber(el.attrs.min);
  const step = rangeNumber(el.attrs.step);
  return {
    min: min ?? 0,
    max: rangeNumber(el.attrs.max) ?? 100,
    step: String(el.attrs.step ?? "").toLowerCase() === "any" ? null : step != null && step > 0 ? step : 1,
    base: min ?? rangeNumber(el.attrs.value) ?? 0,
  };
}
function rangeValue(el: MirrorElement, raw: unknown): string {
  const { min, max, step, base } = rangeSpec(el);
  if (max < min) return String(min);
  let value = Math.min(max, Math.max(min, rangeNumber(raw) ?? min / 2 + max / 2));
  if (step !== null) {
    const lo = Math.ceil((min - base) / step - 1e-10),
      hi = Math.floor((max - base) / step + 1e-10);
    if (lo <= hi) {
      const index = Math.min(hi, Math.max(lo, Math.floor((value - base) / step + 0.5 + 1e-10)));
      value = Math.min(max, Math.max(min, Number((base + index * step).toPrecision(15))));
    }
  }
  return String(value);
}
function syncRangeValue(el: MirrorElement): void {
  const base = rangeSpec(el).base;
  __vn_set_prop(el.__vnId, I(":range-base"), String(base));
  writeControlValue(el, el._value, false);
}

const VALUE_TAGS = new Set(["input", "textarea", "select"]);

/** `el.value = v` on a form control: updates mirror and host on EVERY assignment (the host's write counter re-forces a field the user typed over); fires no `input` / `change`. */
function writeControlValue(el: MirrorElement, v: unknown, dirty = true): void {
  if (dirty && isRangeInput(el)) el._rangeDirty = true;
  const s = isRangeInput(el)
    ? rangeValue(el, v)
    : isColorInput(el)
      ? colorValue(v)
      : v == null && el.tag !== "select"
        ? ""
        : String(v);
  el._value = s;
  __vn_set_prop(el.__vnId, I("value"), s);
}

function ownExpando(self: object, key: string, v: unknown): void {
  Object.defineProperty(self, key, { value: v, writable: true, enumerable: true, configurable: true });
}

defineDomGet(
  "value",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    if (isSelectEl(this)) return selectValue(this);
    if (isOptionEl(this)) return optionValue(this);
    if (isRangeInput(this)) return rangeValue(this, this._value);
    if (isColorInput(this)) return colorValue(this._value);
    return this._value;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "value", v);
    if (isSelectEl(this)) setSelectValue(this, String(v));
    else if (isOptionEl(this)) patchAttribute(this, "value", String(v));
    else if (VALUE_TAGS.has(this.tag)) writeControlValue(this, v);
    else this._value = v as string;
  } as (value: never) => void,
);

defineDomGet(
  "valueAsNumber",
  function (this: MirrorElement) {
    if (!isMirrorEl(this) || this.tag !== "input") return NaN;
    return isRangeInput(this)
      ? Number(rangeValue(this, this._value))
      : String(this.attrs.type ?? "").toLowerCase() === "number"
        ? (rangeNumber(this._value) ?? NaN)
        : NaN;
  },
  function (this: MirrorElement, raw: unknown) {
    if (!isMirrorEl(this) || this.tag !== "input") return;
    const value = Number(raw);
    if (!Number.isFinite(value) && !Number.isNaN(value)) throw new TypeError("The provided value is non-finite.");
    if (isRangeInput(this)) writeControlValue(this, Number.isNaN(value) ? "" : String(value));
    else if (String(this.attrs.type ?? "").toLowerCase() === "number")
      writeControlValue(this, Number.isNaN(value) ? "" : String(value));
    else throw new DOMException("This input element does not support Number values.", "InvalidStateError");
  } as (value: never) => void,
);
function stepRange(el: MirrorElement, count: unknown, direction: number): void {
  if (!isRangeInput(el)) throw new DOMException("This input element does not support stepping.", "InvalidStateError");
  const { step } = rangeSpec(el);
  if (step === null) throw new DOMException("This input element does not support stepping.", "InvalidStateError");
  writeControlValue(el, Number(rangeValue(el, el._value)) + direction * toLong(count === undefined ? 1 : count) * step);
}
defineDom("stepUp", function (this: MirrorElement, n?: unknown) {
  stepRange(this, n, 1);
});
defineDom("stepDown", function (this: MirrorElement, n?: unknown) {
  stepRange(this, n, -1);
});

function attrChecked(el: MirrorElement): boolean {
  const a = el.attrs?.checked;
  return a != null && a !== "false";
}

defineDomGet(
  "checked",
  function (this: MirrorElement) {
    if (!isMirrorEl(this) || this.tag !== "input") return isMirrorEl(this) ? this._checked : undefined;
    return this._checked ?? attrChecked(this);
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "checked", v);
    if (this.tag !== "input") {
      this._checked = v as boolean;
      return;
    }
    this._checked = !!v;
    __vn_set_prop(this.__vnId, I("checked"), String(!!v));
  } as (value: never) => void,
);

defineDomGet(
  "defaultValue",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    if (this.tag === "input") return this.attrs.value ?? "";
    if (this.tag === "textarea") return (this as unknown as { textContent: string }).textContent ?? "";
    return undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this) || (this.tag !== "input" && this.tag !== "textarea"))
      return ownExpando(this, "defaultValue", v);
    if (this.tag === "input") this.setAttribute("value", v == null ? "" : String(v));
    else (this as unknown as { textContent: string }).textContent = v == null ? "" : String(v);
  } as (value: never) => void,
);

defineDomGet(
  "defaultChecked",
  function (this: MirrorElement) {
    return isMirrorEl(this) && this.tag === "input" ? attrChecked(this) : undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this) || this.tag !== "input") return ownExpando(this, "defaultChecked", v);
    if (v) this.setAttribute("checked", "");
    else this.removeAttribute("checked");
  } as (value: never) => void,
);

// <select> and <option>: each option's selectedness and dirtiness live in the mirror, and the selectedness setting
// algorithm reruns when a select's options, `multiple` or `size` change. The host draws the select from the reserved
// `:selected` prop and reports a pick as the select's `input` host event with the option's index (`pickOption`); it
// draws a drop-down whatever `multiple` and `size` say (`adviseListBox`).

// Plain predicates, not type guards: `this` in an accessor is already a
// MirrorElement, and a guard's false branch would narrow it to `never`.
function isSelectEl(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "select" && !e._ns;
}

function isOptionEl(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "option" && !e._ns;
}

function isOptGroup(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "optgroup";
}

const attrOn = (el: MirrorElement, k: string): boolean => el.attrs[k] != null && el.attrs[k] !== "false";

function ownerSelect(n: MirrorNode): MirrorElement | null {
  const p = n.parentNode;
  if (isSelectEl(p)) return p;
  if (p && isOptGroup(p) && isSelectEl(p.parentNode)) return p.parentNode;
  return null;
}

function selectOptions(sel: MirrorElement): MirrorElement[] {
  const out: MirrorElement[] = [];
  for (const c of sel.childNodes) {
    if (isOptionEl(c)) out.push(c as MirrorElement);
    else if (isOptGroup(c)) for (const g of c.childNodes) if (isOptionEl(g)) out.push(g as MirrorElement);
  }
  return out;
}

const isMultipleSelect = (sel: MirrorElement): boolean => attrOn(sel, "multiple");

function displaySize(sel: MirrorElement): number {
  const n = parseInt(sel.attrs.size ?? "", 10);
  return n > 0 ? n : isMultipleSelect(sel) ? 4 : 1;
}

function optionDisabled(o: MirrorElement): boolean {
  return attrOn(o, "disabled") || (!!o.parentNode && isOptGroup(o.parentNode) && attrOn(o.parentNode, "disabled"));
}

const stripCollapse = (s: string): string =>
  s
    .split(/[ \t\n\f\r]+/)
    .filter(Boolean)
    .join(" ");

function optionText(o: MirrorElement): string {
  return stripCollapse(String((o as unknown as { textContent: unknown }).textContent ?? ""));
}

function optionValue(o: MirrorElement): string {
  return o.attrs.value ?? optionText(o);
}

function modelValueOf(o: MirrorElement): unknown {
  return Object.prototype.hasOwnProperty.call(o, "_rawValue") ? o._rawValue : optionValue(o);
}

function selectedIndexOf(sel: MirrorElement): number {
  const opts = selectOptions(sel);
  for (let i = 0; i < opts.length; i++) if (opts[i]!._selected) return i;
  return -1;
}

function selectValue(sel: MirrorElement): string {
  const o = selectOptions(sel).find((x) => x._selected);
  return o ? optionValue(o) : "";
}

function showSelected(sel: MirrorElement): void {
  const ix = selectedIndexOf(sel);
  if (sel._shown === ix) return;
  sel._shown = ix;
  __vn_set_prop(sel.__vnId, I(":selected"), String(ix));
}

/** HTML's "selectedness setting algorithm": a single drop-down with nothing selected selects its first enabled option, one with several selected keeps the last; then the host is told. */
function resetSelect(sel: MirrorElement): void {
  if (!isMultipleSelect(sel)) {
    const opts = selectOptions(sel);
    const on = opts.filter((o) => o._selected);
    if (on.length === 0 && displaySize(sel) === 1) {
      const first = opts.find((o) => !optionDisabled(o));
      if (first) first._selected = true;
    } else if (on.length > 1) {
      for (const o of on.slice(0, -1)) o._selected = false;
    }
  }
  showSelected(sel);
}

function optionsChangedIn(n: MirrorNode | null): void {
  if (!n || n.kind !== 1) return;
  const tag = (n as MirrorElement).tag;
  if (tag === "select") {
    if (isSelectEl(n)) resetSelect(n);
  } else if (tag === "optgroup" && isSelectEl(n.parentNode)) {
    resetSelect(n.parentNode as MirrorElement);
  }
}

function selectednessChanged(o: MirrorElement): void {
  const sel = ownerSelect(o);
  if (!sel) return;
  if (o._selected && !isMultipleSelect(sel)) for (const x of selectOptions(sel)) if (x !== o) x._selected = false;
  resetSelect(sel);
}

function setOptionSelected(o: MirrorElement, on: boolean): void {
  o._dirty = true;
  o._selected = on;
  selectednessChanged(o);
}

function setSelectedIndex(sel: MirrorElement, ix: number): void {
  const opts = selectOptions(sel);
  for (const o of opts) o._selected = false;
  const o = ix >= 0 ? opts[ix] : undefined;
  if (o) {
    o._selected = true;
    o._dirty = true;
  }
  showSelected(sel);
}

function setSelectValue(sel: MirrorElement, v: string): void {
  let found = false;
  for (const o of selectOptions(sel)) {
    o._selected = !found && optionValue(o) === v;
    if (o._selected) {
      o._dirty = true;
      found = true;
    }
  }
  showSelected(sel);
}

/** A select's or an option's `value` prop (runtime-dom assigns the PROPERTY). An option keeps the bound value in `_rawValue` for `v-model`, so `<option :value="1">` gives the model a number. */
function patchSelectValue(el: MirrorElement, next: unknown): void {
  if (isOptionEl(el)) {
    el._rawValue = next;
    patchAttribute(el, "value", next == null ? null : String(next));
    return;
  }
  const v = next == null ? "" : String(next);
  if (selectValue(el) !== v) setSelectValue(el, v);
}

function pickOption(sel: MirrorElement, ix: number): boolean {
  const opts = selectOptions(sel);
  const o = opts[ix];
  if (!o || optionDisabled(o) || attrOn(sel, "disabled")) return true;
  if (o._selected && opts.every((x) => x === o || !x._selected)) return true;
  for (const x of opts) x._selected = x === o;
  o._dirty = true;
  showSelected(sel);
  propagate(sel, "input", fileEvent("input"));
  propagate(sel, "change", fileEvent("change"));
  return true;
}

function adviseListBox(sel: MirrorElement): void {
  if (!isMultipleSelect(sel) && displaySize(sel) === 1) return;
  warnCompat(
    "select:list-box",
    "<select multiple> / <select size>: drawn as a drop-down here (the first selected option, and a popup that " +
      "picks one option at a time), not as Chromium's list box; the selection model, selectedOptions and " +
      "v-model with an array work",
  );
}

function defineSelectMembers(sel: MirrorElement): void {
  const options = liveCollection<MirrorElement>(HTMLCollectionProto, true, () => selectOptions(sel));
  const acc = (get: () => unknown, set?: (v: unknown) => void): PropertyDescriptor => ({
    get,
    ...(set ? { set } : {}),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperties(sel, {
    options: acc(() => options),
    selectedOptions: acc(() =>
      staticList(
        HTMLCollectionProto,
        selectOptions(sel).filter((o) => o._selected),
      ),
    ),
    selectedIndex: acc(
      () => selectedIndexOf(sel),
      (v) => setSelectedIndex(sel, toLong(v)),
    ),
    length: acc(() => selectOptions(sel).length),
    item: { value: (i: unknown) => selectOptions(sel)[toLong(i)] ?? null, configurable: true, writable: true },
  });
}

function defineOptionMembers(o: MirrorElement): void {
  o._selected = false;
  o._dirty = false;
  const acc = (get: () => unknown, set?: (v: unknown) => void): PropertyDescriptor => ({
    get,
    ...(set ? { set } : {}),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperties(o, {
    selected: acc(
      () => !!o._selected,
      (v) => setOptionSelected(o, !!v),
    ),
    index: acc(() => {
      const sel = ownerSelect(o);
      return sel ? selectOptions(sel).indexOf(o) : 0;
    }),
    text: acc(
      () => optionText(o),
      (v) => {
        (o as unknown as { textContent: unknown }).textContent = String(v);
      },
    ),
    label: acc(
      () => o.attrs.label ?? optionText(o),
      (v) => patchAttribute(o, "label", String(v)),
    ),
  });
}

/** Members each tag class's PROTOTYPE carries as own accessors; each delegates to the element's own path, so behaviour is the same however it is reached. */
const TAG_ACCESSORS: Record<string, readonly string[]> = {
  INPUT: ["value", "checked", "defaultValue", "defaultChecked"],
  TEXTAREA: ["value", "defaultValue"],
  SELECT: ["value", "selectedIndex"],
  OPTION: ["value", "selected"],
  IMG: ["src", "currentSrc", "complete", "naturalWidth", "naturalHeight"],
};

for (const [name, spec] of Object.entries(TAG_CLASSES)) {
  const tags: readonly string[] = typeof spec === "string" ? [spec] : spec;
  const tag = tags[0]!;
  const existing = (globalThis as Record<string, unknown>)[name];
  const ours = vacant(globalThis, name);
  const parentName = TAG_PARENTS[name];
  const parentProto =
    (parentName &&
      (globalThis as unknown as Record<string, { prototype?: object } | undefined>)[parentName]?.prototype) ||
    ElementProto;
  if (typeof existing === "function" && !ours) {
    // The ENGINE's class (redjs's HTMLImageElement, for `new Image()`) knows
    // nothing of mirror elements; extend its `instanceof` to cover them.
    try {
      const own = Function.prototype[Symbol.hasInstance];
      Object.defineProperty(existing, Symbol.hasInstance, {
        value(this: unknown, x: unknown) {
          return (isMirrorEl(x) && tags.includes(x.tagName)) || own.call(this, x);
        },
        configurable: true,
      });
      // redjs's bootstrap HTMLVideoElement is a bare class on Object.prototype:
      // it goes under HTMLMediaElement, as a browser's does.
      const p = (existing as { prototype?: object }).prototype;
      if (parentName && p && Object.getPrototypeOf(p) === Object.prototype) Object.setPrototypeOf(p, parentProto);
    } catch {
      /* a class that refuses the property keeps its own answer */
    }
    continue;
  }
  if (!ours) continue;
  // Mirror nodes are all plain HTMLElement, so `instanceof` is answered by tag
  // name; the prototype stays on HTMLElement's chain, as in a browser.
  const klass = class {};
  Object.defineProperty(klass, "name", { value: name });
  Object.defineProperty(klass, Symbol.hasInstance, {
    value: (x: unknown) => !!x && typeof x === "object" && tags.includes((x as MirrorElement).tagName),
  });
  Object.setPrototypeOf(klass.prototype, parentProto);
  if (name === "HTMLMediaElement") {
    for (const [k, v] of MEDIA_CONSTANTS) {
      Object.defineProperty(klass, k, { value: v, enumerable: true });
      Object.defineProperty(klass.prototype, k, { value: v, enumerable: true });
    }
  }
  for (const prop of TAG_ACCESSORS[tag] ?? []) {
    // The wrong receiver throws, as in a browser. So does an object that
    // inherits from this prototype: delegating would land right back here.
    const check = (self: unknown, verb: string): Record<string, unknown> => {
      if (
        !self ||
        typeof self !== "object" ||
        (self as MirrorElement).tagName !== tag ||
        klass.prototype.isPrototypeOf(self)
      ) {
        throw new TypeError(`Failed to ${verb} the '${prop}' property on '${name}': Illegal invocation`);
      }
      return self as Record<string, unknown>;
    };
    Object.defineProperty(klass.prototype, prop, {
      get(this: unknown) {
        return check(this, "read")[prop];
      },
      set(this: unknown, v: unknown) {
        check(this, "set")[prop] = v;
      },
      enumerable: true,
      configurable: true,
    });
  }
  defineGlobal(name, klass);
}

// WebIDL interface objects the realm lacks: code that merely names one (tsyringe's `design:paramtypes` with `Window`) must not throw a ReferenceError.

type Iface = { prototype: object };

function defineInterface(
  name: string,
  is: (x: unknown) => boolean,
  parent: Iface | null,
  construct?: (...args: unknown[]) => object,
): Iface | undefined {
  if (!vacant(globalThis, name)) return (globalThis as Record<string, unknown>)[name] as Iface;
  const iface = function (this: unknown, ...args: unknown[]): object {
    if (construct && new.target) return construct(...args);
    throw new TypeError(
      construct
        ? `Failed to construct '${name}': Please use the 'new' operator, this DOM object constructor cannot be called as a function.`
        : "Illegal constructor",
    );
  };
  Object.defineProperty(iface, "name", { value: name });
  const proto = Object.create(parent?.prototype ?? Object.prototype, {
    constructor: { value: iface, writable: true, configurable: true },
  });
  Object.defineProperty(iface, "prototype", { value: proto });
  Object.defineProperty(iface, Symbol.hasInstance, {
    value: (x: unknown) => is(x) || (!!x && typeof x === "object" && proto.isPrototypeOf(x)),
  });
  defineGlobal(name, iface);
  return iface as unknown as Iface;
}

const theGlobal = (k: string) => (globalThis as Record<string, unknown>)[k];
const ET: Iface | null = typeof EventTarget === "function" ? (EventTarget as unknown as Iface) : null;

function adoptInterface(
  name: string,
  proto: object,
  parent: Iface | null,
  also?: (x: object) => boolean,
): Iface | undefined {
  const engine = theGlobal(name);
  const iface = vacant(globalThis, name)
    ? function (): never {
        throw new TypeError("Illegal constructor");
      }
    : null;
  if (!iface) {
    const engineProto = typeof engine === "function" ? (engine as unknown as Iface).prototype : undefined;
    if (engineProto && engineProto !== proto) {
      try {
        Object.setPrototypeOf(proto, engineProto);
      } catch {
        /* keeps its own chain */
      }
    }
  } else {
    Object.defineProperty(iface, "name", { value: name });
    const parentProto = parent?.prototype ?? Object.prototype;
    if (Object.getPrototypeOf(proto) !== parentProto) Object.setPrototypeOf(proto, parentProto);
    Object.defineProperty(proto, "constructor", { value: iface, writable: true, configurable: true });
    Object.defineProperty(iface, "prototype", { value: proto, writable: false });
  }
  const target = iface ?? engine;
  if (also && typeof target === "function") {
    const own = Function.prototype[Symbol.hasInstance];
    try {
      Object.defineProperty(target, Symbol.hasInstance, {
        value(this: unknown, x: unknown) {
          return (!!x && typeof x === "object" && also(x)) || own.call(this, x);
        },
        configurable: true,
      });
    } catch {
      /* an interface that refuses the property keeps its own answer */
    }
  }
  if (iface) defineGlobal(name, iface);
  return (target ?? undefined) as Iface | undefined;
}
adoptInterface("NodeList", NodeListProto, null, isChildList);
adoptInterface("HTMLCollection", HTMLCollectionProto, null);
adoptInterface("DOMRectList", DOMRectListProto, null);

// redjs aliases `Node` to `Element`; this gives Node its own interface (constants, text nodes `instanceof`) and re-parents Element, text mirrors and Attr under it.
const NODE_CONSTANTS: readonly (readonly [string, number])[] = [
  ["ELEMENT_NODE", 1],
  ["ATTRIBUTE_NODE", 2],
  ["TEXT_NODE", 3],
  ["CDATA_SECTION_NODE", 4],
  ["ENTITY_REFERENCE_NODE", 5],
  ["ENTITY_NODE", 6],
  ["PROCESSING_INSTRUCTION_NODE", 7],
  ["COMMENT_NODE", 8],
  ["DOCUMENT_NODE", 9],
  ["DOCUMENT_TYPE_NODE", 10],
  ["DOCUMENT_FRAGMENT_NODE", 11],
  ["NOTATION_NODE", 12],
  ["DOCUMENT_POSITION_DISCONNECTED", 1],
  ["DOCUMENT_POSITION_PRECEDING", 2],
  ["DOCUMENT_POSITION_FOLLOWING", 4],
  ["DOCUMENT_POSITION_CONTAINS", 8],
  ["DOCUMENT_POSITION_CONTAINED_BY", 16],
  ["DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32],
];
const isDocument = (x: unknown) => x != null && x === theGlobal("document");
const NodeIface: Iface | null = (() => {
  const engineNode = theGlobal("Node");
  const engineElement = theGlobal("Element");
  if (typeof engineNode === "function" && engineNode !== engineElement && !vacant(globalThis, "Node")) {
    return engineNode as unknown as Iface;
  }
  const iface = function (): never {
    throw new TypeError("Illegal constructor");
  };
  Object.defineProperty(iface, "name", { value: "Node" });
  const proto = Object.create(ET?.prototype ?? Object.prototype, {
    constructor: { value: iface, writable: true, configurable: true },
  }) as object;
  Object.defineProperty(iface, "prototype", { value: proto, writable: false });
  for (const [k, v] of NODE_CONSTANTS) {
    Object.defineProperty(iface, k, { value: v, enumerable: true });
    Object.defineProperty(proto, k, { value: v, enumerable: true });
  }
  Object.defineProperty(iface, Symbol.hasInstance, {
    value: (x: unknown) =>
      !!x &&
      typeof x === "object" &&
      (proto.isPrototypeOf(x) || isMirrorNode(x) || isDocument(x) || x instanceof AttrImpl),
  });
  try {
    Object.defineProperty(globalThis, "Node", { value: mark(iface), configurable: true, writable: true });
  } catch {
    warnCompat(
      "global:Node",
      "Node could not be installed over the engine's own: Node.TEXT_NODE and the other constants are missing",
    );
    return null;
  }
  if (typeof engineElement === "function" && !vacant(globalThis, "Element")) {
    try {
      const elementProto = (engineElement as unknown as Iface).prototype;
      if (Object.getPrototypeOf(elementProto) !== proto) Object.setPrototypeOf(elementProto, proto);
    } catch {
      /* `instanceof Node` still holds for mirrors, by kind */
    }
  }
  return iface as unknown as Iface;
})();
const ElementIface: Iface | null = (() => {
  if (!vacant(globalThis, "Element")) return (theGlobal("Element") as Iface | undefined) ?? null;
  const iface = defineInterface("Element", () => false, NodeIface) ?? null;
  if (iface) adoptInterface("HTMLElement", ElementProto, iface);
  return iface;
})();

function wrapElementHasInstance(iface: unknown, mirrorIs: (el: MirrorElement) => boolean): void {
  if (typeof iface !== "function") return;
  const own = Function.prototype[Symbol.hasInstance];
  try {
    Object.defineProperty(iface, Symbol.hasInstance, {
      value(this: unknown, x: unknown) {
        return isMirrorEl(x) ? mirrorIs(x) : own.call(this, x);
      },
      configurable: true,
    });
  } catch {
    /* an interface that refuses the property keeps its own answer */
  }
}
wrapElementHasInstance(theGlobal("HTMLElement"), (el) => !el._ns && !fragments.has(el));

const WindowIface = defineInterface("Window", (x) => x != null && (x === globalThis || x === theGlobal("window")), ET);
if (WindowIface) {
  try {
    // `window.constructor === Window`, which is how some code spots a window.
    if (
      !Object.prototype.hasOwnProperty.call(globalThis, "constructor") ||
      theGlobal("constructor") === Object ||
      vacant(globalThis, "constructor")
    )
      Object.defineProperty(globalThis, "constructor", { value: WindowIface, writable: true, configurable: true });
  } catch {
    /* a global that refuses the property keeps Object's */
  }
}
const DocumentIface = defineInterface("Document", isDocument, NodeIface ?? ET);
defineInterface("HTMLDocument", isDocument, DocumentIface ?? ET);
defineInterface("Location", (x) => x != null && x === theGlobal("location"), null);
defineInterface("History", (x) => x != null && x === theGlobal("history"), null);
defineInterface("Screen", (x) => x != null && x === theGlobal("screen"), ET);

const mirrorKind = (x: unknown) =>
  !!x && typeof x === "object" && Object.getPrototypeOf(x) === TextProto ? (x as MirrorNode).kind : 0;
const CharacterDataIface = defineInterface(
  "CharacterData",
  (x) => mirrorKind(x) === 2 || mirrorKind(x) === 3,
  NodeIface,
);
if (CharacterDataIface && Object.getPrototypeOf(TextProto) !== CharacterDataIface.prototype) {
  try {
    Object.setPrototypeOf(TextProto, CharacterDataIface.prototype);
  } catch {
    /* `instanceof` still holds, by kind */
  }
}
defineInterface(
  "Text",
  (x) => mirrorKind(x) === 2,
  CharacterDataIface ?? null,
  (data) => looseCreated(nodeOps.createText(data === undefined ? "" : String(data))),
);
defineInterface(
  "Comment",
  (x) => mirrorKind(x) === 3,
  CharacterDataIface ?? null,
  (data) => {
    const c = looseCreated(nodeOps.createComment(data === undefined ? "" : String(data)));
    c.text = data === undefined ? "" : String(data);
    return c;
  },
);
defineInterface("ShadowRoot", () => false, theGlobal("DocumentFragment") as Iface | null);
defineInterface("Attr", (x) => x instanceof AttrImpl, NodeIface);
if (NodeIface && Object.getPrototypeOf(AttrImpl.prototype) !== NodeIface.prototype) {
  Object.setPrototypeOf(AttrImpl.prototype, NodeIface.prototype);
}
defineInterface("NamedNodeMap", (x) => x instanceof NamedNodeMapImpl, null);
const styleDecls = new WeakSet<object>();
defineInterface("CSSStyleDeclaration", (x) => !!x && typeof x === "object" && styleDecls.has(x), null);

// -- SVG and MathML elements: `instanceof SVGElement` / `MathMLElement` / the tag's interface, never HTMLElement (Vue's Teleport and motion's scroll offsets test these).
const SVG_INTERFACES: readonly (readonly [name: string, parent: string, tags: readonly string[]])[] = [
  ["SVGGraphicsElement", "SVGElement", []],
  ["SVGGeometryElement", "SVGGraphicsElement", []],
  ["SVGSVGElement", "SVGGraphicsElement", ["svg"]],
  ["SVGGElement", "SVGGraphicsElement", ["g"]],
  ["SVGDefsElement", "SVGGraphicsElement", ["defs"]],
  ["SVGUseElement", "SVGGraphicsElement", ["use"]],
  ["SVGImageElement", "SVGGraphicsElement", ["image"]],
  ["SVGForeignObjectElement", "SVGGraphicsElement", ["foreignObject"]],
  ["SVGAElement", "SVGGraphicsElement", ["a"]],
  ["SVGSwitchElement", "SVGGraphicsElement", ["switch"]],
  ["SVGPathElement", "SVGGeometryElement", ["path"]],
  ["SVGRectElement", "SVGGeometryElement", ["rect"]],
  ["SVGCircleElement", "SVGGeometryElement", ["circle"]],
  ["SVGEllipseElement", "SVGGeometryElement", ["ellipse"]],
  ["SVGLineElement", "SVGGeometryElement", ["line"]],
  ["SVGPolylineElement", "SVGGeometryElement", ["polyline"]],
  ["SVGPolygonElement", "SVGGeometryElement", ["polygon"]],
  ["SVGTextContentElement", "SVGGraphicsElement", []],
  ["SVGTextPositioningElement", "SVGTextContentElement", []],
  ["SVGTextElement", "SVGTextPositioningElement", ["text"]],
  ["SVGTSpanElement", "SVGTextPositioningElement", ["tspan"]],
  ["SVGTextPathElement", "SVGTextContentElement", ["textPath"]],
  ["SVGGradientElement", "SVGElement", []],
  ["SVGLinearGradientElement", "SVGGradientElement", ["linearGradient"]],
  ["SVGRadialGradientElement", "SVGGradientElement", ["radialGradient"]],
  ["SVGStopElement", "SVGElement", ["stop"]],
  ["SVGClipPathElement", "SVGElement", ["clipPath"]],
  ["SVGMaskElement", "SVGElement", ["mask"]],
  ["SVGPatternElement", "SVGElement", ["pattern"]],
  ["SVGMarkerElement", "SVGElement", ["marker"]],
  ["SVGSymbolElement", "SVGElement", ["symbol"]],
  ["SVGFilterElement", "SVGElement", ["filter"]],
  ["SVGTitleElement", "SVGElement", ["title"]],
  ["SVGDescElement", "SVGElement", ["desc"]],
  ["SVGMetadataElement", "SVGElement", ["metadata"]],
  ["SVGStyleElement", "SVGElement", ["style"]],
  ["SVGScriptElement", "SVGElement", ["script"]],
  ["SVGViewElement", "SVGElement", ["view"]],
];
{
  const isSvg = (x: unknown): x is MirrorElement => isMirrorEl(x) && x._ns === "svg";
  const SVGElementIface = defineInterface("SVGElement", isSvg, ElementIface);
  wrapElementHasInstance(SVGElementIface, (el) => el._ns === "svg");
  defineInterface("MathMLElement", (x) => isMirrorEl(x) && x._ns === "mathml", ElementIface);
  // Each interface's tags plus those under it; the table lists parents first, so walking backwards completes a child before its parent.
  const tagsOf = new Map<string, Set<string>>(SVG_INTERFACES.map(([name, , tags]) => [name, new Set(tags)]));
  for (let i = SVG_INTERFACES.length - 1; i >= 0; i--) {
    const [name, parent] = SVG_INTERFACES[i]!;
    const into = tagsOf.get(parent);
    if (into) for (const t of tagsOf.get(name)!) into.add(t);
  }
  for (const [name, parent] of SVG_INTERFACES) {
    const tags = tagsOf.get(name)!;
    defineInterface(
      name,
      (x) => isSvg(x) && tags.has(x.tag),
      (theGlobal(parent) as Iface | undefined) ?? SVGElementIface ?? null,
    );
  }
}

for (const [name, set] of Object.entries(BRANDS)) {
  defineInterface(name, (x) => !!x && typeof x === "object" && set.has(x), null);
}
defineInterface(
  "MediaQueryList",
  (x) =>
    !!ET &&
    x instanceof (ET as unknown as typeof EventTarget) &&
    typeof (x as { media?: unknown }).media === "string" &&
    "matches" in x,
  ET,
);
defineInterface("Performance", (x) => x != null && x === theGlobal("performance"), ET);
defineInterface("Crypto", (x) => x != null && x === theGlobal("crypto"), null);

const EVENT_INTERFACES: readonly (readonly [
  name: string,
  parent: "Event" | "UIEvent",
  members: Readonly<Record<string, unknown>>,
])[] = [
  ["TransitionEvent", "Event", { propertyName: "", elapsedTime: 0, pseudoElement: "" }],
  ["AnimationEvent", "Event", { animationName: "", elapsedTime: 0, pseudoElement: "" }],
  ["CompositionEvent", "UIEvent", { data: "" }],
  ["HashChangeEvent", "Event", { oldURL: "", newURL: "" }],
  ["PopStateEvent", "Event", { state: null }],
  ["PageTransitionEvent", "Event", { persisted: false }],
  ["PromiseRejectionEvent", "Event", { promise: undefined, reason: undefined }],
  ["ToggleEvent", "Event", { oldState: "", newState: "" }],
  ["MediaQueryListEvent", "Event", { media: "", matches: false }],
  ["FormDataEvent", "Event", { formData: undefined }],
];
{
  const EventBase = typeof Event === "function" ? Event : null;
  const ui = theGlobal("UIEvent");
  const UIEventBase = typeof ui === "function" ? (ui as typeof Event) : EventBase;
  const inits = new WeakMap<object, Record<string, unknown>>();
  for (const [name, parent, members] of EVENT_INTERFACES) {
    const Base = parent === "UIEvent" ? UIEventBase : EventBase;
    if (!Base || !vacant(globalThis, name)) continue;
    const klass = class extends Base {
      constructor(type: string, init: Record<string, unknown> = {}) {
        super(type, init);
        const own: Record<string, unknown> = {};
        for (const k in members) {
          const d = members[k];
          const v = init?.[k];
          own[k] =
            v === undefined
              ? d
              : typeof d === "number"
                ? Number(v)
                : typeof d === "string"
                  ? String(v)
                  : typeof d === "boolean"
                    ? !!v
                    : v;
        }
        inits.set(this, own);
      }
    };
    Object.defineProperty(klass, "name", { value: name });
    for (const k of Object.keys(members)) {
      Object.defineProperty(klass.prototype, k, {
        get(this: object) {
          const own = inits.get(this);
          if (!own) throw new TypeError("Illegal invocation");
          return own[k];
        },
        enumerable: true,
        configurable: true,
      });
    }
    defineGlobal(name, klass);
  }
}

// InputEvent's `dataTransfer` and `getTargetRanges()`: only for contenteditable; in a text field's `beforeinput` /
// `input` Chromium has no DataTransfer (the text is `data`) and no target ranges.
{
  const IE = theGlobal("InputEvent");
  const proto = typeof IE === "function" ? (IE as { prototype: object }).prototype : null;
  if (proto && vacant(proto, "dataTransfer"))
    Object.defineProperty(proto, "dataTransfer", {
      get: mark(function dataTransfer() {
        return null;
      }),
      enumerable: true,
      configurable: true,
    });
  if (proto && vacant(proto, "getTargetRanges"))
    Object.defineProperty(proto, "getTargetRanges", {
      value: mark(function getTargetRanges(): unknown[] {
        return [];
      }),
      writable: true,
      enumerable: true,
      configurable: true,
    });
}

// -- Touch: the interfaces exist and construct, but as on desktop Chrome without a touch screen `'ontouchstart' in window` is false,
//    `maxTouchPoints` is 0 and no touch event is delivered, so libraries that feature-test touch take their non-touch path.
const TouchListProto = collectionProto("TouchList");
adoptInterface("TouchList", TouchListProto, null);
{
  const TOUCH_MEMBERS: Readonly<Record<string, number | string>> = {
    clientX: 0,
    clientY: 0,
    screenX: 0,
    screenY: 0,
    pageX: 0,
    pageY: 0,
    radiusX: 0,
    radiusY: 0,
    rotationAngle: 0,
    force: 0,
    altitudeAngle: 0,
    azimuthAngle: 0,
    touchType: "direct",
  };
  const touches = new WeakMap<object, Record<string, unknown>>();
  const touchSlot = (self: object) => {
    const own = touches.get(self);
    if (!own) throw new TypeError("Illegal invocation");
    return own;
  };
  class Touch {
    constructor(init?: Record<string, unknown>) {
      if (init == null || typeof init !== "object") {
        throw new TypeError("Failed to construct 'Touch': 1 argument required, but only 0 present.");
      }
      for (const req of ["identifier", "target"]) {
        if (init[req] === undefined) {
          throw new TypeError(
            `Failed to construct 'Touch': Failed to read the '${req}' property from 'TouchInit': Required member is undefined.`,
          );
        }
      }
      const own: Record<string, unknown> = { identifier: Number(init.identifier) | 0, target: init.target };
      for (const [k, d] of Object.entries(TOUCH_MEMBERS)) {
        const v = init[k];
        own[k] = v === undefined ? d : typeof d === "number" ? Number(v) : String(v);
      }
      touches.set(this, own);
    }
  }
  for (const k of ["identifier", "target", ...Object.keys(TOUCH_MEMBERS)]) {
    Object.defineProperty(Touch.prototype, k, {
      get(this: object) {
        return touchSlot(this)[k];
      },
      enumerable: true,
      configurable: true,
    });
  }
  if (vacant(globalThis, "Touch")) defineGlobal("Touch", Touch);
  const TouchCtor = theGlobal("Touch") as { prototype: object } | undefined;

  const ui = theGlobal("UIEvent");
  const Base = typeof ui === "function" ? (ui as typeof Event) : typeof Event === "function" ? Event : null;
  if (Base && vacant(globalThis, "TouchEvent")) {
    const lists = new WeakMap<object, Record<string, unknown>>();
    const touchList = (v: unknown, member: string): MirrorCollection<unknown> => {
      if (v == null) return staticList(TouchListProto, []);
      if (typeof (v as Iterable<unknown>)[Symbol.iterator] !== "function") {
        throw new TypeError(
          `Failed to construct 'TouchEvent': Failed to read the '${member}' property from 'TouchEventInit': The provided value cannot be converted to a sequence.`,
        );
      }
      const items = [...(v as Iterable<unknown>)];
      for (const t of items) {
        if (!(TouchCtor && t instanceof (TouchCtor as unknown as new () => object))) {
          throw new TypeError(
            `Failed to construct 'TouchEvent': Failed to read the '${member}' property from 'TouchEventInit': Failed to convert value to 'Touch'.`,
          );
        }
      }
      return staticList(TouchListProto, items);
    };
    class TouchEvent extends Base {
      constructor(type: string, init: Record<string, unknown> = {}) {
        super(type, init);
        lists.set(this, {
          touches: touchList(init?.touches, "touches"),
          targetTouches: touchList(init?.targetTouches, "targetTouches"),
          changedTouches: touchList(init?.changedTouches, "changedTouches"),
          altKey: !!init?.altKey,
          metaKey: !!init?.metaKey,
          ctrlKey: !!init?.ctrlKey,
          shiftKey: !!init?.shiftKey,
        });
      }
    }
    for (const k of ["touches", "targetTouches", "changedTouches", "altKey", "metaKey", "ctrlKey", "shiftKey"]) {
      Object.defineProperty(TouchEvent.prototype, k, {
        get(this: object) {
          const own = lists.get(this);
          if (!own) throw new TypeError("Illegal invocation");
          return own[k];
        },
        enumerable: true,
        configurable: true,
      });
    }
    defineGlobal("TouchEvent", TouchEvent);
  }
}
{
  const nav = typeof navigator !== "undefined" && navigator ? (navigator as object) : null;
  if (nav && vacant(nav, "maxTouchPoints")) {
    try {
      Object.defineProperty(nav, "maxTouchPoints", { get: mark(() => 0), enumerable: true, configurable: true });
    } catch {
      /* a frozen navigator keeps its own shape */
    }
  }
}

class DOMRectReadOnlyImpl {
  declare x: number;
  declare y: number;
  declare width: number;
  declare height: number;
  declare top: number;
  declare left: number;
  declare right: number;
  declare bottom: number;
  constructor(x: unknown = 0, y: unknown = 0, width: unknown = 0, height: unknown = 0) {
    const [nx = NaN, ny = NaN, nw = NaN, nh = NaN] = [x, y, width, height].map(Number);
    Object.assign(this, {
      x: nx,
      y: ny,
      width: nw,
      height: nh,
      top: Math.min(ny, ny + nh),
      left: Math.min(nx, nx + nw),
      right: Math.max(nx, nx + nw),
      bottom: Math.max(ny, ny + nh),
    });
  }
  static fromRect(r: { x?: number; y?: number; width?: number; height?: number } = {}) {
    return new this(r.x ?? 0, r.y ?? 0, r.width ?? 0, r.height ?? 0);
  }
  toJSON() {
    const { x, y, width, height, top, right, bottom, left } = this;
    return { x, y, width, height, top, right, bottom, left };
  }
}
Object.defineProperty(DOMRectReadOnlyImpl, "name", { value: "DOMRectReadOnly" });
class DOMRectImpl extends DOMRectReadOnlyImpl {}
Object.defineProperty(DOMRectImpl, "name", { value: "DOMRect" });
defineGlobal("DOMRectReadOnly", DOMRectReadOnlyImpl);
defineGlobal("DOMRect", DOMRectImpl);
function asDomRect<T extends object>(r: T): T {
  const C = theGlobal("DOMRect");
  if (C === DOMRectImpl) Object.setPrototypeOf(r, DOMRectImpl.prototype);
  return r;
}

// -- getComputedStyle: values come from the real cascade (the properties components read, used width/height, every custom property).
//    A property nothing declared reads `""`, as an unsupported one would in a browser, with one advisory.
const computedCache = new WeakMap<object, unknown>();
function computedStyleFor(node: unknown) {
  const id = typeof (node as MirrorNode)?.__vnId === "number" ? (node as MirrorNode).__vnId : appRoot?.__vnId;
  const raw = id ? __vn_computed_style(id) : "";
  /** One property scanned out of the answer, not a parsed map (parsing cost three quarters of a read). The host call stays uncached (it flushes pending style). The `\n` before the key stops suffix matches (`width` vs `max-width`). */
  const lookup = (css: string): string | undefined => {
    const head = css + "=";
    const at = raw.startsWith(head) ? 0 : raw.indexOf("\n" + head);
    if (at < 0) return undefined;
    const from = at + head.length + (at === 0 ? 0 : 1);
    const end = raw.indexOf("\n", from);
    return end < 0 ? raw.slice(from) : raw.slice(from, end);
  };
  const read = (k: string | symbol): string | undefined => {
    const key = String(k);
    if (!/^[a-zA-Z-]/.test(key) || key.startsWith("__") || key === "then") return undefined;
    const css = key.startsWith("--") ? key : hyphenate(key);
    const value = lookup(css);
    if (value !== undefined) return value;
    // No answer: the node is not in the tree or there is no root yet. CSSOM gives a disconnected element an empty declaration, so "" is right.
    if (!raw) return "";
    if (css.startsWith("--")) return "";
    warnCompat(
      `getComputedStyle:${css}`,
      `getComputedStyle().${key} reads "" — vue-native's computed style publishes the properties ` +
        `a reused component actually reads (transition-*, animation-*, transform, display, position, ` +
        `overflow-*, width/height, direction, font-size, color) plus every custom property, ` +
        `not all 340 longhands`,
    );
    return "";
  };
  return new Proxy(
    {
      getPropertyValue: read,
      getPropertyPriority: () => "",
      get length() {
        return 0;
      },
    },
    {
      get: (t, k) => (k in t ? t[k as keyof typeof t] : typeof k === "string" ? read(k) : undefined),
      has: () => true,
    },
  );
}
defineGlobal("getComputedStyle", (node: unknown) => {
  let cached = computedCache.get(node as object);
  if (!cached) {
    cached = new Proxy(
      {},
      {
        get: (_t, k) => (computedStyleFor(node) as Record<string | symbol, unknown>)[k],
        has: () => true,
      },
    );
    if (node && typeof node === "object") computedCache.set(node, cached);
    styleDecls.add(cached as object);
  }
  return cached;
});

const doc = globalThis.document;
if (doc) {
  const defDoc = (name: string, value: unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
  };
  const defDocGet = (name: string, get: () => unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { get: mark(get), configurable: true });
  };
  // Force-installed: redjs's own `querySelector`/`getElementById` search a registry of elements it created, which is empty here,
  // so leaving them finds nothing, silently.
  const forceDoc = (name: string, value: unknown) =>
    Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  const rootId = () => (appRoot ? appRoot.__vnId : 0);
  forceDoc("querySelector", (sel: string) => queryOne(sel, rootId(), 3));
  forceDoc("querySelectorAll", (sel: string) => staticList(NodeListProto, hostQuery(sel, rootId(), 3)));
  defDoc("getElementsByClassName", (classNames: unknown) => {
    const test = classTest(classNames);
    return liveCollection(HTMLCollectionProto, true, () => elementsUnder(appRoot, true, test));
  });
  defDoc("getElementsByName", (name: unknown) => {
    const want = String(name);
    return liveCollection(NodeListProto, false, () =>
      elementsUnder(appRoot, true, (el) => !el._ns && el.attrs.name === want),
    );
  });
  forceDoc("getElementById", (id: unknown) => queryOne(`[id=${JSON.stringify(String(id))}]`, rootId()));
  // A mirror element, loose until inserted. `ENGINE_TAGS` stay the engine's detached elements: `<a>`/`<input>` carry download and pick
  // idioms (see `wrapDetached`), and head-only tags are not boxes.
  // The engine's, not a previous runtime's wrapper (`original`): a reload would stack wrappers.
  const engineCreate = original(doc, "createElement", (doc as { createElement?: unknown }).createElement);
  const nativeCreate =
    typeof engineCreate === "function" ? (engineCreate as (tag: unknown, opts?: unknown) => unknown).bind(doc) : null;
  forceDoc("createElement", (tag: unknown, opts?: unknown) => {
    const name = String(tag);
    if (!VALID_ELEMENT_NAME.test(name)) {
      throw domError(
        "createElement",
        "Document",
        "InvalidCharacterError",
        `The tag name provided ('${name}') is not a valid name.`,
      );
    }
    const t = name.toLowerCase();
    if (nativeCreate && ENGINE_TAGS.has(t)) {
      const el = nativeCreate(tag, opts);
      if ((t === "a" || t === "input") && el && typeof el === "object") wrapDetached(el as Detached, t);
      if (t === "style" && el && typeof el === "object") prepareStylesheetElement(el as Record<string, unknown>);
      return el;
    }
    return looseCreated(nodeOps.createElement(t));
  });
  // The SVG namespace keeps the name's case (`linearGradient`), as the renderer's SVG elements do.
  defDoc("createElementNS", (ns: unknown, qualifiedName: unknown) => {
    const name = String(qualifiedName);
    const local = name.slice(name.indexOf(":") + 1);
    if (!VALID_ELEMENT_NAME.test(local)) {
      throw domError(
        "createElementNS",
        "Document",
        "InvalidCharacterError",
        `The qualified name provided ('${name}') contains the invalid name-start character '${local[0] ?? ""}'.`,
      );
    }
    if (ns === HTML_NS) return (doc.createElement as (t: string) => unknown)(local);
    const space = ns === SVG_NS ? "svg" : ns === MATHML_NS ? "mathml" : undefined;
    return looseCreated(nodeOps.createElement(local, space));
  });
  defDoc("createTextNode", (data: unknown) => looseCreated(nodeOps.createText(String(data))));
  defDoc("createComment", (data: unknown) => {
    const c = looseCreated(nodeOps.createComment(String(data)));
    c.text = String(data);
    return c;
  });
  defDoc("createAttribute", (name: unknown) => {
    const n = String(name);
    if (!VALID_ELEMENT_NAME.test(n)) {
      throw domError(
        "createAttribute",
        "Document",
        "InvalidCharacterError",
        `The localName provided ('${n}') contains an invalid character.`,
      );
    }
    return new AttrImpl(n, "");
  });
  defDoc(
    "createDocumentFragment",
    () => new (globalThis as unknown as { DocumentFragment: new () => MirrorElement }).DocumentFragment(),
  );
  defDoc("createTreeWalker", (root: MirrorNode | null, whatToShow = 0xffffffff, filter: TreeWalkerFilter = null) => {
    const flat: MirrorNode[] = [];
    const walk = (n: MirrorNode) => {
      for (const c of n.childNodes || []) {
        const show = c.kind === 1 ? 1 : c.kind === 2 ? 4 : 128;
        if (whatToShow & show) flat.push(c);
        walk(c);
      }
    };
    if (root) walk(root);
    let i = -1;
    const accepts = (n: MirrorNode) => {
      if (!filter) return true;
      const fn = typeof filter === "function" ? filter : filter.acceptNode;
      return !fn || fn.call(filter, n) === 1;
    };
    return branded("TreeWalker", {
      root,
      currentNode: root,
      nextNode() {
        while (++i < flat.length) {
          if (accepts(flat[i]!)) return (this.currentNode = flat[i]!);
        }
        return null;
      },
      previousNode() {
        while (--i >= 0) {
          if (accepts(flat[i]!)) return (this.currentNode = flat[i]!);
        }
        return null;
      },
    });
  });
  defDocGet("defaultView", () => globalThis);
  try {
    Object.defineProperty(doc, "activeElement", { get: activeElement, configurable: true });
  } catch (e) {
    warnCompat("activeElement", `document.activeElement could not be installed on this engine and reads null (${e})`);
  }
  interceptGlobalTarget(doc);
  interceptGlobalTarget(globalThis);
  // Viewport metrics: redjs reports 0; the window IS the app root, so use its painted box (floating positioning reads them).
  // Except with the windowing host present: then inner* (client area) and outer* (GetWindowRect) are real and deliberately
  // differ, so they are left alone.
  if (!globalThis.__VN_WINDOW__) {
    for (const [name, axis] of [
      ["innerWidth", "width"],
      ["innerHeight", "height"],
      ["outerWidth", "width"],
      ["outerHeight", "height"],
    ] as [string, "width" | "height"][]) {
      try {
        Object.defineProperty(globalThis, name, {
          get: () => viewportRect()[axis],
          configurable: true,
        });
      } catch {
        /* a non-configurable engine property — leave it alone */
      }
    }
  }
  for (const name of ["scrollX", "scrollY", "pageXOffset", "pageYOffset"]) {
    try {
      if (typeof (globalThis as Record<string, unknown>)[name] !== "number") {
        Object.defineProperty(globalThis, name, { value: 0, configurable: true, writable: true });
      }
    } catch {
      /* ditto */
    }
  }
  // `documentElement` and `body` are both the app root (the one node `:root`, `html` and `body` match), so theme code such as `classList.add("dark")` or `style.setProperty("--radius", ...)` reaches the cascade root.
  // Before `mount()` the root does not exist but module-scope theme code already writes to it; the pre-root object buffers those writes.
  {
    const pending: {
      classes: string[];
      style: Record<string, string>;
      attrs: Record<string, string>;
    } = { classes: [], style: Object.create(null), attrs: Object.create(null) };
    const preRoot = {
      nodeType: 1,
      tagName: "HTML",
      get className() {
        return pending.classes.join(" ");
      },
      set className(v: unknown) {
        pending.classes = String(v ?? "")
          .split(/\s+/)
          .filter(Boolean);
      },
      classList: {
        add(...names: string[]) {
          for (const n of names) if (n && !pending.classes.includes(n)) pending.classes.push(n);
        },
        remove(...names: string[]) {
          pending.classes = pending.classes.filter((c) => !names.includes(c));
        },
        toggle(name: string, force?: boolean) {
          const on = force === undefined ? !pending.classes.includes(name) : !!force;
          if (on) preRoot.classList.add(name);
          else preRoot.classList.remove(name);
          return on;
        },
        contains: (name: string) => pending.classes.includes(name),
        get length() {
          return pending.classes.length;
        },
        item: (i: number) => pending.classes[i] ?? null,
        toString: () => pending.classes.join(" "),
      },
      style: new Proxy(
        {
          setProperty(k: unknown, v: unknown) {
            pending.style[cssomName(String(k))] = v == null ? "" : String(v);
          },
          removeProperty(k: unknown) {
            const prev = pending.style[cssomName(String(k))] ?? "";
            delete pending.style[cssomName(String(k))];
            return prev;
          },
          getPropertyValue: (k: unknown) => pending.style[cssomName(String(k))] ?? "",
          getPropertyPriority: () => "",
        },
        {
          set(t, k, v) {
            if (typeof k === "symbol" || k in t) return Reflect.set(t, k, v);
            pending.style[cssPropName(k)] = v == null ? "" : String(v);
            return true;
          },
          get: (t, k) =>
            typeof k === "symbol" || k in t ? t[k as keyof typeof t] : (pending.style[cssPropName(k)] ?? ""),
        },
      ),
      getAttribute: (k: unknown) => pending.attrs[String(k)] ?? null,
      setAttribute(k: unknown, v: unknown) {
        pending.attrs[String(k)] = String(v);
      },
      hasAttribute: (k: unknown) => String(k) in pending.attrs,
      removeAttribute(k: unknown) {
        delete pending.attrs[String(k)];
      },
      querySelector: () => null,
      querySelectorAll: () => [],
      contains: () => false,
      appendChild: (n: unknown) => preInsert(ensureDocumentRoot(), n, null, "appendChild"),
      insertBefore: (n: unknown, anchor: unknown) => preInsert(ensureDocumentRoot(), n, anchor, "insertBefore"),
      removeChild(n: unknown) {
        const root = ensureDocumentRoot();
        const child = nodeArg(n, "removeChild", "Node", 1);
        if (!child) {
          foreignNode(n as object, false, "removeChild");
          return n;
        }
        if (child.parentNode !== root)
          throw domError("removeChild", "Node", "NotFoundError", "The node to be removed is not a child of this node.");
        detachNode(child);
        return child;
      },
      insertAdjacentElement: (where: unknown, n: unknown) =>
        ensureDocumentRoot().insertAdjacentElement(String(where), n as MirrorElement),
      addEventListener: () => {},
      removeEventListener: () => {},
      getBoundingClientRect: () => ({
        ...viewportRect(),
        toJSON() {
          return this;
        },
      }),
      children: [],
      ownerDocument: doc,
      parentNode: null,
      scrollLeft: 0,
      scrollTop: 0,
      scrollX: 0,
      scrollY: 0,
      clientTop: 0,
      clientLeft: 0,
    };
    for (const [name, axis] of [
      ["clientWidth", "width"],
      ["clientHeight", "height"],
      ["offsetWidth", "width"],
      ["offsetHeight", "height"],
      ["scrollWidth", "width"],
      ["scrollHeight", "height"],
    ] as [string, "width" | "height"][]) {
      Object.defineProperty(preRoot, name, { get: () => viewportRect()[axis], configurable: true });
    }
    flushDocumentElement = (root) => {
      for (const c of pending.classes) root.classList.add(c);
      for (const k of Object.keys(pending.style)) root.style.setProperty(k, pending.style[k]);
      for (const k of Object.keys(pending.attrs)) root.setAttribute(k, pending.attrs[k]);
    };
    for (const name of ["body", "documentElement"]) {
      try {
        Object.defineProperty(doc, name, {
          get: () => appRoot ?? preRoot,
          configurable: true,
        });
      } catch {
        /* a non-configurable engine property — leave it alone */
      }
    }
    // `<head>` is not a box (a `<style>` appended must not become a child of the root): it stays redjs's property bag, common members filled in.
    const head = doc.head as unknown as Record<string, unknown> | null | undefined;
    if (head && typeof head === "object") {
      if (!head.style || typeof head.style !== "object") head.style = {};
      const hs = head.style as Record<string, unknown>;
      if (typeof hs.setProperty !== "function") {
        hs.setProperty = (k: unknown, v: unknown) => {
          hs[String(k)] = v == null ? "" : String(v);
        };
      }
      if (typeof hs.removeProperty !== "function") {
        hs.removeProperty = (k: unknown) => {
          const prev = hs[String(k)] ?? "";
          delete hs[String(k)];
          return prev;
        };
      }
      if (typeof hs.getPropertyValue !== "function") hs.getPropertyValue = (k: unknown) => hs[String(k)] ?? "";
      if (!head.querySelector) head.querySelector = () => null;
      if (!head.querySelectorAll) head.querySelectorAll = () => [];
      if (!head.contains) head.contains = () => false;
      if (!head.getAttribute) head.getAttribute = () => null;
      if (!head.setAttribute) head.setAttribute = () => {};
      if (!head.appendChild) head.appendChild = (n: unknown) => n;
      if (!head.removeChild) head.removeChild = (n: unknown) => n;
      if (!("ownerDocument" in head)) {
        Object.defineProperty(head, "ownerDocument", { get: () => doc, configurable: true });
      }
    }
  }
}

// -- web fonts: `FontFace`, `FontFaceSet`, `document.fonts`. Inert without the host's loader; a reload takes over the host's listener.
installFonts(globalThis as unknown as Record<string, unknown>, doc);

// -- Web Animations: `el.animate()` on HTMLElement/SVGElement prototypes only, not Element.prototype, so motion / framer-motion keep
//    their JS animations. Inert without the host's native WAAPI core.
if (typeof __vn_waapi_core === "object" && __vn_waapi_core !== null) {
  const g = globalThis as unknown as Record<string, unknown>;
  const svg = (g.SVGElement as { prototype?: object } | undefined)?.prototype;
  installWaapi({
    host: __vn_waapi_core,
    core: __vn_waapi_core,
    protos: svg && svg !== ElementProto ? [ElementProto, svg] : [ElementProto],
    nodeOf: (t) => (isMirrorEl(t) ? t.__vnId : 0),
    setStyle: (t, name, value) => (t as MirrorElement).style.setProperty(name, value),
    contains: (root, node) =>
      isMirrorNode(root) && isMirrorNode(node) && (root as MirrorElement).contains(node as MirrorNode),
    warn: warnCompat,
    now: () => (g.performance as { now(): number } | undefined)?.now() ?? Date.now(),
    frame: (cb) => {
      if (typeof requestAnimationFrame === "function") requestAnimationFrame(cb);
      else setTimeout(() => cb((g.performance as { now(): number } | undefined)?.now() ?? Date.now()), 16);
    },
    onFrame: (stamp) => void frameStampers.push(stamp),
    setTimer: (cb, ms) => setTimeout(cb, ms),
    clearTimer: (h) => clearTimeout(h as number),
    defineGlobal,
    doc: doc as unknown as Record<string, unknown> | null,
    EventTarget: g.EventTarget as (new () => object) | undefined,
    Event: g.Event as (new (type: string, init?: object) => object) | undefined,
    DOMException: g.DOMException as (new (message: string, name: string) => Error) | undefined,
  });
}

// -- text selection: `getSelection()`, `Selection`, `Range`, `document.createRange()` over the host's selection. Inert without it.
installSelection({
  host:
    typeof __vn_selection_get === "function"
      ? { get: __vn_selection_get, set: __vn_selection_set, text: __vn_selection_text }
      : null,
  nodeById: (id) => nodeById(id),
  isNode: (x): x is MirrorNode => isMirrorNode(x),
  root: () => appRoot,
  materialize: (el) => {
    materializeOwnText(el as MirrorElement);
    return el.childNodes[0] ?? null;
  },
  rect: (el) => (el as MirrorElement).getBoundingClientRect(),
  makeRect: (x, y, w, h) => new DOMRectImpl(x, y, w, h),
  createText: (data) => looseCreated(nodeOps.createText(data)),
  createFragment: () => new (globalThis as unknown as { DocumentFragment: new () => MirrorElement }).DocumentFragment(),
  warn: warnCompat,
  error: domError,
  defineGlobal,
  defineDoc: (name, value) => {
    if (doc && vacant(doc, name))
      Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
  },
});
for (const type of ["selectionchange", "selectstart"]) {
  const prop = `on${type}`;
  if (!doc || !vacant(doc, prop)) continue;
  let slot: { fn: unknown; listener: (ev: VnEvent) => void } | null = null;
  Object.defineProperty(doc, prop, {
    get: mark(() => slot?.fn ?? null),
    set: mark((v: unknown) => {
      const fn = v !== null && (typeof v === "function" || typeof v === "object") ? v : null;
      if (slot && !fn) {
        doc.removeEventListener?.(type, slot.listener as EventHandler);
        slot = null;
      } else if (slot) slot.fn = fn;
      else if (fn) {
        const entry = {
          fn,
          listener: (ev: VnEvent) => {
            if (typeof entry.fn !== "function") return;
            const r = (entry.fn as (this: unknown, ev: VnEvent) => unknown).call(doc, ev);
            if (r === false) ev.preventDefault?.();
          },
        };
        slot = entry;
        doc.addEventListener?.(type, entry.listener as EventHandler);
      }
    }),
    configurable: true,
  });
}

// document.elementFromPoint / elementsFromPoint: the host's hit test (skips `pointer-events: none` and `visibility:
// hidden`), viewport coordinates, null / [] outside the viewport; a non-finite coordinate throws (WebIDL `double`).
if (doc && typeof __vn_hit_at === "function") {
  const hitAt = (name: string, x: unknown, y: unknown, all: boolean): MirrorElement[] => {
    const px = Number(x);
    const py = Number(y);
    if (!Number.isFinite(px) || !Number.isFinite(py)) {
      throw new TypeError(`Failed to execute '${name}' on 'Document': The provided double value is non-finite.`);
    }
    const ids = __vn_hit_at(px, py, all ? 1 : 0);
    if (!ids) return [];
    const out: MirrorElement[] = [];
    for (const part of ids.split(",")) {
      const n = nodeById(Number(part) as VnNodeId);
      if (n && n.kind === 1) out.push(n as MirrorElement);
    }
    return out;
  };
  for (const [name, fn] of [
    ["elementFromPoint", (x: unknown, y: unknown) => hitAt("elementFromPoint", x, y, false)[0] ?? null],
    ["elementsFromPoint", (x: unknown, y: unknown) => hitAt("elementsFromPoint", x, y, true)],
  ] as const) {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(fn), configurable: true, writable: true });
  }
}

// `document.head` is not a box, so a head `<style>` is a stylesheet, not a tree node: its text joins the app's sheet
// after the bundle's CSS, in insertion order, and follows later edits. Before mount it rides the first
// `__vn_set_stylesheet`; after, each change re-sets the sheet once per microtask. A `<link rel=stylesheet>` in head is
// not fetched (advisory).
let installStylesheet: (css: string) => void = (css) => __vn_set_stylesheet(css);
{
  const d = globalThis.document as unknown as Record<string, unknown> | undefined;
  const head = d?.head as Record<string, unknown> | undefined;
  // The HOST's function, even after a reload (the global of that name is then the previous runtime's `install`; see `original`).
  const hostSetSheet =
    typeof __vn_set_stylesheet === "function" ? original(globalThis, "__vn_set_stylesheet", __vn_set_stylesheet) : null;
  if (d && head && typeof head === "object" && hostSetSheet) {
    const injected: { el: Record<string, unknown>; text: string }[] = [];
    let base: string | null = null;
    let queued = false;
    // Each injected sheet goes to the host as its own text: the host keeps one parse per distinct text, so concatenating
    // would re-parse the whole bundle sheet on every change.
    const install = (css: string) => {
      base = String(css);
      hostSetSheet(base, injected.map((s) => s.text).filter(Boolean));
    };
    // Every installer goes through this (the mount, loader.ts's flushCss via the global), so injected sheets survive a hot reload's re-aggregation.
    installStylesheet = install;
    try {
      Object.defineProperty(globalThis, "__vn_set_stylesheet", { value: install, configurable: true, writable: true });
    } catch {
      warnCompat(
        "head-style:install",
        "the dev loader's stylesheet updates will drop <style> elements added to document.head",
      );
    }
    const refresh = () => {
      domEpoch++; // `getElementsByTagName("style")` lists them
      if (base === null || queued) return; // before the first sheet: the mount's install carries them
      queued = true;
      queueMicrotask(() => {
        queued = false;
        if (base !== null) install(base);
      });
    };
    const textOf = stylesheetText;
    const isStyle = (n: unknown) =>
      !!n && typeof n === "object" && String((n as { tagName?: unknown }).tagName ?? "").toUpperCase() === "STYLE";
    const isLink = (n: unknown) =>
      !!n && typeof n === "object" && String((n as { tagName?: unknown }).tagName ?? "").toUpperCase() === "LINK";
    const track = (el: Record<string, unknown>) => {
      const entry = { el, text: textOf(el) };
      injected.push(entry);
      const set = (v: unknown) => {
        entry.text = v == null ? "" : String(v);
        refresh();
      };
      try {
        for (const prop of ["textContent", "innerHTML", "innerText", "text"]) {
          Object.defineProperty(el, prop, { get: () => entry.text, set, configurable: true });
        }
        const append = (child: unknown) => {
          entry.text += textOf(child);
          refresh();
          return child;
        };
        Object.defineProperty(el, "appendChild", { value: append, configurable: true, writable: true });
        Object.defineProperty(el, "append", {
          value: (...kids: unknown[]) => kids.forEach(append),
          configurable: true,
          writable: true,
        });
        Object.defineProperty(el, "remove", { value: () => untrack(el), configurable: true, writable: true });
        Object.defineProperty(el, "parentNode", {
          get: () => (injected.includes(entry) ? head : null),
          configurable: true,
        });
      } catch {
        warnCompat("head-style:track", "a <style> in document.head will not follow later edits to its text");
      }
      refresh();
    };
    const untrack = (el: unknown) => {
      const i = injected.findIndex((s) => s.el === el);
      if (i >= 0) {
        injected.splice(i, 1);
        refresh();
      }
    };
    const adopt = (n: unknown) => {
      if (isStyle(n)) {
        if (!injected.some((s) => s.el === n)) track(n as Record<string, unknown>);
      } else if (isLink(n)) {
        const rel = String(
          (n as { rel?: unknown }).rel ?? (n as { getAttribute?(k: string): unknown }).getAttribute?.("rel") ?? "",
        );
        if (/stylesheet/i.test(rel)) {
          warnCompat(
            "head-link",
            "a <link rel=stylesheet> added to document.head is not loaded — vue-native has no network stylesheets; import the CSS instead",
          );
        }
      }
      return n;
    };
    const def = (name: string, value: unknown) =>
      Object.defineProperty(head, name, { value, configurable: true, writable: true });
    def("appendChild", adopt);
    def("insertBefore", (n: unknown) => adopt(n));
    def("append", (...kids: unknown[]) => void kids.forEach(adopt));
    def("prepend", (...kids: unknown[]) => void kids.forEach(adopt));
    def("insertAdjacentElement", (_where: unknown, n: unknown) => adopt(n));
    def("removeChild", (n: unknown) => {
      untrack(n);
      return n;
    });
    if (!("tagName" in head)) def("tagName", "HEAD");
    // A previous runtime's getter (same-realm reload, see `REALM`) read ITS injected list, so it is replaced.
    if (vacant(head, "childNodes")) {
      Object.defineProperty(head, "childNodes", {
        get: mark(() =>
          staticList(
            NodeListProto,
            injected.map((s) => s.el),
          ),
        ),
        configurable: true,
      });
    }

    foreignStyle = (n, inserted) => {
      if (!isStyle(n) && !isLink(n)) return false;
      if (inserted) adopt(n);
      else untrack(n);
      return true;
    };
    if (vacant(d, "getElementsByTagName")) {
      Object.defineProperty(d, "getElementsByTagName", {
        value: mark((tag: unknown) => {
          const t = String(tag).toLowerCase();
          const test = tagTest(tag);
          return liveCollection<unknown>(HTMLCollectionProto, true, () => {
            if (t === "head") return [head];
            if (t === "html" || t === "body") return [d.documentElement];
            const tree = elementsUnder(appRoot, false, test);
            return t === "style" ? [...injected.map((s) => s.el), ...tree] : tree;
          });
        }),
        configurable: true,
        writable: true,
      });
    }
  }
}

const assignKey = Symbol("vnModelAssign");

interface DirectiveBindingLike {
  value?: unknown;
  oldValue?: unknown;
  modifiers?: Partial<Record<string, boolean>>;
}
interface VNodeLike {
  props?: Record<string, unknown> | null;
}

function setElValue(el: MirrorElement, value: unknown): void {
  const v = value == null ? "" : String(value);
  // Compared with what the field holds now — typing keeps `_value` current —
  // so a model reset to the string the user has since typed over still lands.
  if (el._value === v) return;
  writeControlValue(el, v);
}

export const vModelText = {
  created(el: MirrorElement, { modifiers = {} }: DirectiveBindingLike, vnode: VNodeLike) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    // `.lazy` listens for `change`, which the host never sources; fall back to `input` and say so.
    if (modifiers.lazy) {
      warnCompat(
        "v-model:lazy",
        "v-model.lazy updates on every keystroke here, not on `change` — the native host does " +
          "not source a `change` event yet, so binding one would never fire at all",
      );
    }
    el.addEventListener("input", () => {
      if (el.composing) return;
      // `.trim` runs before `.number`, so `v` is still a string at both casts.
      let v: string | number = el.value;
      if (modifiers.trim) v = (v as string).trim();
      if (modifiers.number) {
        const n = parseFloat(v as string);
        if (!Number.isNaN(n)) v = n;
      }
      if (el[assignKey]) (el[assignKey] as (value: unknown) => void)(v);
    });
    // The composition ends re-dispatch `input`, which assigns the IME's text. Runtime-dom binds them only without `.lazy`;
    // here `.lazy` listens for `input`, so it needs them too.
    el.addEventListener("compositionstart", () => {
      el.composing = true;
    });
    el.addEventListener("compositionend", () => {
      if (!el.composing) return;
      el.composing = false;
      el.dispatchEvent(new Event("input"));
    });
  },
  mounted(el: MirrorElement, { value }: DirectiveBindingLike) {
    setElValue(el, value);
  },
  beforeUpdate(el: MirrorElement, { value }: DirectiveBindingLike, vnode: VNodeLike) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    // Leave a composing field alone: assigning would end the IME's composition (vuejs/core#2302).
    if (el.composing) return;
    setElValue(el, value);
  },
};

// Every input renders as a text field: dynamic/checkbox/radio fall back to text behavior with an advisory.
function modelFallback(kind: string) {
  return {
    created(el: MirrorElement, binding: DirectiveBindingLike, vnode: VNodeLike) {
      warnCompat(
        `v-model:${kind}`,
        `v-model on ${kind} inputs is not implemented natively yet — falling back to text ` +
          `semantics; use modelValue/onUpdate:modelValue on a component meanwhile`,
      );
      vModelText.created(el, binding, vnode);
    },
    mounted: vModelText.mounted,
    beforeUpdate: vModelText.beforeUpdate,
  };
}
export const vModelDynamic = modelFallback("dynamic");
export const vModelCheckbox = modelFallback("checkbox");
export const vModelRadio = modelFallback("radio");

function looseEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  const date = (x: unknown): x is Date => x instanceof Date;
  if (date(a) || date(b)) return date(a) && date(b) ? a.getTime() === b.getTime() : false;
  if (typeof a === "symbol" || typeof b === "symbol") return a === b;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!looseEqual(a[i], b[i])) return false;
    return true;
  }
  const obj = (x: unknown): x is Record<string, unknown> => x !== null && typeof x === "object";
  if (obj(a) || obj(b)) {
    if (!obj(a) || !obj(b)) return false;
    if (Object.keys(a).length !== Object.keys(b).length) return false;
    for (const key in a) {
      const has = Object.prototype.hasOwnProperty;
      if (has.call(a, key) !== has.call(b, key) || !looseEqual(a[key], b[key])) return false;
    }
  }
  return String(a) === String(b);
}

const looseIndexOf = (arr: readonly unknown[], v: unknown): number => arr.findIndex((x) => looseEqual(x, v));

function looseToNumber(v: unknown): unknown {
  const n = parseFloat(v as string);
  return Number.isNaN(n) ? v : n;
}

function modelAssigner(vnode: VNodeLike): ((value: unknown) => void) | null {
  const fn = vnode.props?.["onUpdate:modelValue"];
  if (Array.isArray(fn)) return (value) => fn.forEach((f) => (f as (v: unknown) => void)(value));
  return typeof fn === "function" ? (fn as (value: unknown) => void) : null;
}

function setModelSelected(el: MirrorElement, value: unknown): void {
  const multiple = isMultipleSelect(el);
  const isArray = Array.isArray(value);
  if (multiple && !isArray && !(value instanceof Set)) {
    console.warn(
      `[Vue warn]: <select multiple v-model> expects an Array or Set value for its binding, but got ` +
        `${Object.prototype.toString.call(value).slice(8, -1)}.`,
    );
    return;
  }
  const opts = selectOptions(el);
  for (let i = 0; i < opts.length; i++) {
    const option = opts[i]!;
    const optionValue = modelValueOf(option);
    if (multiple) {
      let on: boolean;
      if (isArray) {
        const type = typeof optionValue;
        on =
          type === "string" || type === "number"
            ? (value as unknown[]).some((v) => String(v) === String(optionValue))
            : looseIndexOf(value as unknown[], optionValue) > -1;
      } else {
        on = (value as Set<unknown>).has(optionValue);
      }
      setOptionSelected(option, on);
    } else if (looseEqual(optionValue, value)) {
      if (selectedIndexOf(el) !== i) setSelectedIndex(el, i);
      return;
    }
  }
  if (!multiple && selectedIndexOf(el) !== -1) setSelectedIndex(el, -1);
}

export const vModelSelect = {
  deep: true,
  created(el: MirrorElement, { value, modifiers = {} }: DirectiveBindingLike, vnode: VNodeLike) {
    el._modelValue = value;
    el.addEventListener("change", () => {
      const picked = selectOptions(el)
        .filter((o) => o._selected)
        .map((o) => (modifiers.number ? looseToNumber(modelValueOf(o)) : modelValueOf(o)));
      const assign = el[assignKey] as ((value: unknown) => void) | null | undefined;
      if (assign) {
        assign(isMultipleSelect(el) ? (el._modelValue instanceof Set ? new Set(picked) : picked) : picked[0]);
      }
      el._assigning = true;
      void nextTick(() => {
        el._assigning = false;
      });
    });
    el[assignKey] = modelAssigner(vnode);
  },
  // A select's value depends on its options, which mount and update before it.
  mounted(el: MirrorElement, { value }: DirectiveBindingLike) {
    setModelSelected(el, value);
  },
  beforeUpdate(el: MirrorElement, { value }: DirectiveBindingLike, vnode: VNodeLike) {
    el._modelValue = value;
    el[assignKey] = modelAssigner(vnode);
  },
  updated(el: MirrorElement, { value }: DirectiveBindingLike) {
    if (!el._assigning) setModelSelected(el, value);
  },
};

function setVShowDisplay(el: MirrorElement, value: unknown): void {
  const d = value ? String(el[vShowOriginalDisplay] ?? "") : "none";
  if (d === "") removeInlineDecl(el, "display");
  else setInlineDecl(el, "display", d, false);
  el[vShowHidden] = !value;
}
export const vShow = {
  beforeMount(el: MirrorElement, { value }: DirectiveBindingLike, vnode?: { transition?: unknown }) {
    if (vnode?.transition) {
      warnCompat(
        "v-show:transition",
        "v-show inside <Transition> flips display without the enter/leave phases here; " +
          "use v-if for an animated show/hide",
      );
    }
    const d = el._inline?.decls.get("display")?.value ?? "";
    el[vShowOriginalDisplay] = d === "none" ? "" : d;
    setVShowDisplay(el, value);
  },
  updated(el: MirrorElement, { value, oldValue }: DirectiveBindingLike) {
    if (!value === !oldValue) return;
    setVShowDisplay(el, value);
  },
  beforeUnmount(el: MirrorElement, { value }: DirectiveBindingLike) {
    setVShowDisplay(el, value);
  },
};

const modifierGuards: Record<string, ((e: VnEvent, m: string[]) => unknown) | undefined> = {
  stop: (e) => e.stopPropagation(),
  prevent: (e) => e.preventDefault(),
  self: (e) => e.target !== e.currentTarget,
  ctrl: (e) => !(e as VnMouseEvent).ctrlKey,
  shift: (e) => !(e as VnMouseEvent).shiftKey,
  alt: (e) => !(e as VnMouseEvent).altKey,
  meta: (e) => !(e as VnMouseEvent).metaKey,
  left: (e) => "button" in e && (e as VnMouseEvent).button !== 0,
  middle: (e) => "button" in e && (e as VnMouseEvent).button !== 1,
  right: (e) => "button" in e && (e as VnMouseEvent).button !== 2,
  exact: (e, m) =>
    ["ctrl", "shift", "alt", "meta"].some(
      (x) => (e as unknown as Record<string, unknown>)[x + "Key"] && !m.includes(x),
    ),
};

export function withModifiers(fn: (event: VnEvent, ...args: unknown[]) => unknown, modifiers: string[]) {
  return (event: VnEvent, ...args: unknown[]) => {
    for (const m of modifiers) {
      const guard = modifierGuards[m];
      if (guard && guard(event, modifiers)) return;
    }
    return fn(event, ...args);
  };
}

const keyAliases: Record<string, string | undefined> = {
  esc: "escape",
  space: " ",
  up: "arrow-up",
  left: "arrow-left",
  right: "arrow-right",
  down: "arrow-down",
  delete: "backspace",
};

export function withKeys(fn: (event: VnEvent) => unknown, keys: string[]) {
  return (event: VnEvent) => {
    if (!("key" in event)) return;
    const eventKey = hyphenate(String((event as VnKeyboardEvent).key));
    if (keys.some((k) => k === eventKey || keyAliases[k] === eventKey)) return fn(event);
    return undefined;
  };
}

// `appWindow` is built in the host's JS prelude, which must evaluate before the app bundle; this
// module is only its namespace. `getCurrentWindow()` exists so ported Tauri code runs unmodified.
const windowHost = globalThis.__VN_WINDOW__;
// Lazy: a host without the window prelude (e.g. the unit test harness) must not be warned about an
// API it never touches.
const missing = () =>
  warnCompat(
    "appWindow",
    "the windowing host is missing (the window prelude did not evaluate) — " +
      "`appWindow` is inert; run the app in the vue-native host.",
  );
const inertWindow = new Proxy(
  {},
  {
    get(_t, key) {
      missing();
      // Verbs stay callable and `is*` answers false, so inert code paths do not throw.
      return typeof key === "string" && key.startsWith("is") ? () => false : () => undefined;
    },
    set() {
      missing();
      return true;
    },
  },
);

export const appWindow: VnAppWindow = windowHost ? windowHost.appWindow : (inertWindow as unknown as VnAppWindow);
export const defineWindow: (options: Record<string, unknown>) => void = windowHost ? windowHost.defineWindow : missing;
export const getCurrentWindow = (): VnAppWindow => appWindow;

// `appPermissions` extends `navigator.permissions` with what the web lacks: requesting outside a
// capture call, and the fourth state "unsupported" (which the standard API must report as "denied").
interface PermissionsHost {
  query: typeof __vn_permissions_query;
  list: typeof __vn_permissions_list;
  request: typeof __vn_permissions_request;
  platform: typeof __vn_permissions_platform;
  setDispatcher: typeof __vn_permissions_set_dispatcher;
}

interface PermissionRow {
  name: string;
  state: VnPermissionState;
  canRequest: boolean;
  deniedBySystem?: boolean;
}

interface AppPermissions extends VnEventTarget {
  readonly platform: string;
  readonly names: string[];
  query(name: string): VnPermissionState;
  list(): PermissionRow[];
  request(name: string): Promise<VnPermissionState>;
}

const permHost: PermissionsHost | null =
  typeof __vn_permissions_query === "function"
    ? {
        query: __vn_permissions_query,
        list: __vn_permissions_list,
        request: __vn_permissions_request,
        platform: __vn_permissions_platform,
        setDispatcher: __vn_permissions_set_dispatcher,
      }
    : null;

const PERMISSION_NAMES: VnPermissionName[] = [
  "microphone",
  "camera",
  "display-capture",
  "notifications",
  "clipboard-read",
  "clipboard-write",
  "geolocation",
];

function createPermissions(host: PermissionsHost): AppPermissions {
  const target = new EventTarget();
  const pending = new Map<number, (state: VnPermissionState) => void>();
  host.setDispatcher((token, state) => {
    const resolve = pending.get(token);
    if (!resolve) return;
    pending.delete(token);
    resolve(state);
  });

  // One `change` stream re-broadcast from the standard objects: a second native subscription would
  // race for the same queue. Best-effort; without `navigator.permissions` query/request still work.
  const statuses = new Map<string, VnPermissionStatus>();
  if (typeof navigator === "object" && navigator && navigator.permissions) {
    for (const name of PERMISSION_NAMES) {
      navigator.permissions.query({ name } as never).then(
        (status) => {
          statuses.set(name, status);
          status.addEventListener("change", () => {
            target.dispatchEvent(Object.assign(new Event("change"), { name, state: host.query(name) }));
          });
        },
        () => {},
      );
    }
  }

  const advised = new Set<string>();
  const adviseUnsupported = (name: string) => {
    if (advised.has(name)) return;
    advised.add(name);
    warnCompat(
      `appPermissions:${name}`,
      `the '${name}' permission has no equivalent on ${host.platform()} — ` +
        `appPermissions.query('${name}') is 'unsupported' and request() is a no-op.`,
    );
  };

  return Object.assign(target, {
    get platform() {
      return host.platform();
    },
    get names() {
      return PERMISSION_NAMES.slice();
    },
    query(name: string) {
      const state = host.query(name);
      if (!state) throw new TypeError(`'${name}' is not a permission name`);
      if (state === "unsupported") adviseUnsupported(name);
      return state;
    },
    list(): PermissionRow[] {
      const rows = JSON.parse(host.list()) as PermissionRow[];
      for (const r of rows) if (r.state === "unsupported") adviseUnsupported(r.name);
      return rows;
    },
    /** Asks for a permission and resolves with a fresh OS read, never optimistically (Windows: opens Settings, resolves when the toggle moves or on timeout). */
    request(name: string): Promise<VnPermissionState> {
      const out = JSON.parse(host.request(name)) as {
        error?: string;
        state?: VnPermissionState;
        token?: number;
      };
      if (out.error) return Promise.reject(new TypeError(out.error));
      if (out.state !== undefined) {
        if (out.state === "unsupported") adviseUnsupported(name);
        return Promise.resolve(out.state);
      }
      return new Promise((resolve) => pending.set(out.token!, resolve));
    },
  });
}

const inertPermissions = new Proxy(
  {},
  {
    get(_t, key) {
      warnCompat(
        "appPermissions",
        "the permissions host is missing (vn_permissions::install did not run) — " +
          "`appPermissions` is inert and every capability reads as 'unsupported'.",
      );
      if (key === "platform") return "this platform";
      if (key === "names") return PERMISSION_NAMES.slice();
      if (key === "query") return () => "unsupported";
      if (key === "list")
        return () => PERMISSION_NAMES.map((name) => ({ name, state: "unsupported", canRequest: false }));
      if (key === "request") return () => Promise.resolve("unsupported");
      return () => undefined;
    },
  },
);

export const appPermissions: AppPermissions = permHost
  ? createPermissions(permHost)
  : (inertPermissions as unknown as AppPermissions);

// `appStorage` extends `localStorage` with where the bytes go and whether they are encrypted at
// rest. Synchronous: boot-time calls with no real wait. Not named `app`, which would collide with
// the `const app = createApp(...)` of a typical entry module.
interface StorageHost {
  userFolder: typeof __vn_storage_user_folder;
  setFolder: typeof __vn_storage_set_folder;
  info: typeof __vn_storage_info;
  flush: typeof __vn_storage_flush;
}

/** IndexedDB's half of `appStorage.info()`: databases live in `<folder>\indexeddb`. `mode` is null until the first `indexedDB` use, which latches folder and key for the run; `null` as a whole without on-disk IndexedDB. */
interface IndexedDbInfo {
  folder: string;
  started: boolean;
  mode: "plaintext" | "encrypted" | "decrypting" | "refused" | null;
  encrypted: boolean | null;
  key: boolean;
  error: string | null;
}

/** The cookie jar's half of `appStorage.info()`: always sealed; session cookies are never written. `file` is null with no jar backend. */
interface CookieJarInfo {
  file: string | null;
  sealed: true;
  bytes: number;
  persisted: number;
  pending: boolean;
  error: string | null;
}

interface StorageInfo {
  folder: string;
  file: string;
  encrypted: boolean;
  bytes: number;
  loaded: boolean;
  pending: boolean;
  provider: string;
  kinds: string[];
  error: string | null;
  indexedDB: IndexedDbInfo | null;
  cookies?: CookieJarInfo;
}

interface AppStorage {
  readonly kinds: string[];
  readonly folder: string;
  readonly encrypted: boolean;
  getUserFolder(kind: string, subfolder?: string): string;
  setStorageFolder(path: string, options?: { encrypted?: boolean }): string;
  info(): StorageInfo;
  flush(): boolean;
}

const storageHost: StorageHost | null =
  typeof __vn_storage_info === "function"
    ? {
        userFolder: __vn_storage_user_folder,
        setFolder: __vn_storage_set_folder,
        info: __vn_storage_info,
        flush: __vn_storage_flush,
      }
    : null;

/** Every host fn here answers `{...}` or `{"error":"..."}`. One unwrapper. */
function storageResult<T extends object>(json: VnJson, what: string): T {
  const out = JSON.parse(json) as T & { error?: string };
  if (out.error) throw new TypeError(`appStorage.${what}: ${out.error}`);
  return out;
}

function createStorage(host: StorageHost): AppStorage {
  return {
    get kinds() {
      return (JSON.parse(host.info()) as StorageInfo).kinds;
    },
    get folder() {
      return (JSON.parse(host.info()) as StorageInfo).folder;
    },
    get encrypted() {
      return (JSON.parse(host.info()) as StorageInfo).encrypted;
    },
    /** Absolute path for a folder kind; creates nothing. Throws TypeError for an unknown kind or a `subfolder` that climbs out. */
    getUserFolder(kind: string, subfolder = "") {
      return storageResult<{ path: string }>(host.userFolder(String(kind), String(subfolder)), "getUserFolder").path;
    },
    /**
     * Points `localStorage` at `path`, creating it; returns the canonical path.
     * `{ encrypted: true }` encrypts at rest (Windows: DPAPI) and migrates an existing plaintext area;
     * omitting it keeps the current format, and a platform with no provider throws.
     * IndexedDB follows the same folder and encryption but latches both at its first use, so call
     * this before the first `indexedDB` call. The HTTP cache follows `encrypted` but not `path`.
     */
    setStorageFolder(path: string, options: { encrypted?: boolean } = {}) {
      // No `encrypted` means "keep the format", never "plaintext": a folder change must not silently decrypt.
      const said = options ? options.encrypted : undefined;
      const encrypted = said === undefined ? "keep" : said ? "true" : "false";
      return storageResult<{ path: string }>(host.setFolder(String(path), encrypted), "setStorageFolder").path;
    },
    info(): StorageInfo {
      return JSON.parse(host.info()) as StorageInfo;
    },
    flush() {
      return storageResult<{ wrote: boolean }>(host.flush(), "flush").wrote;
    },
  };
}

const inertStorage = new Proxy(
  {},
  {
    get(_t, key) {
      warnCompat(
        "appStorage",
        "the storage host is missing (vn_storage::install did not run, or the host was built " +
          "without the `storages` feature) — `appStorage` is inert and localStorage does not " +
          "persist.",
      );
      if (key === "kinds") return [];
      if (key === "folder") return "";
      if (key === "encrypted") return false;
      if (key === "info")
        return () => ({
          folder: "",
          file: "",
          encrypted: false,
          bytes: 0,
          loaded: false,
          pending: false,
          provider: "",
          kinds: [],
          error: "no storage host",
        });
      if (key === "flush") return () => false;
      // A wrong path is worse than none: the app would write somewhere real.
      return () => {
        throw new TypeError(`appStorage.${String(key)} is unavailable: no storage host`);
      };
    },
  },
);

export const appStorage: AppStorage = storageHost
  ? createStorage(storageHost)
  : (inertStorage as unknown as AppStorage);

// Async desktop calls answer `{token}` and settle through one dispatcher in the frame pump. Token 0
// is an unsolicited host event; a settle may carry a host-made value (a `File`, a `FileList`).

interface DesktopHost {
  setDispatcher: typeof __vn_desktop_set_dispatcher;
  screenSources: typeof __vn_desktop_get_screen_sources | null;
  openExternal: typeof __vn_shell_open_external;
  openPath: typeof __vn_shell_open_path;
  showItem: typeof __vn_shell_show_item;
  trash: typeof __vn_shell_trash;
  fs: DesktopFsHost | null;
}

const desktopHost: DesktopHost | null =
  typeof __vn_desktop_set_dispatcher === "function"
    ? {
        setDispatcher: __vn_desktop_set_dispatcher,
        screenSources: typeof __vn_desktop_get_screen_sources === "function" ? __vn_desktop_get_screen_sources : null,
        openExternal: __vn_shell_open_external,
        openPath: __vn_shell_open_path,
        showItem: __vn_shell_show_item,
        trash: __vn_shell_trash,
        fs:
          typeof __vn_dialog_open === "function"
            ? {
                setHandleFactory: __vn_fs_set_handle_factory,
                release: __vn_fs_release,
                getFile: __vn_fs_get_file,
                child: __vn_fs_child,
                remove: __vn_fs_remove,
                same: __vn_fs_same,
                resolve: __vn_fs_resolve,
                dirOpen: __vn_fs_dir_open,
                dirNext: __vn_fs_dir_next,
                dirClose: __vn_fs_dir_close,
                writableOpen: __vn_fs_writable_open,
                writableWrite: __vn_fs_writable_write,
                writableSeek: __vn_fs_writable_seek,
                writableTruncate: __vn_fs_writable_truncate,
                writableClose: __vn_fs_writable_close,
                writableAbort: __vn_fs_writable_abort,
                refuseClone: __vn_fs_refuse_clone,
                setSerializer: __vn_fs_set_serializer,
                permission: __vn_fs_permission,
                dialogOpen: __vn_dialog_open,
                download: __vn_download,
              }
            : null,
      }
    : null;

interface DesktopFailure {
  name: string;
  message?: string;
  error?: string;
  code?: string;
  osError?: number;
}

/** A tier-2 desktop rejection: `name` is DOMException-style, `code` Node-style, `osError` the raw Win32 code. */
export interface AppDesktopError extends Error {
  code?: string;
  osError?: number;
}

function desktopError(f: DesktopFailure): AppDesktopError {
  const message = f.message ?? f.error ?? f.name;
  const e: AppDesktopError = f.name === "TypeError" ? new TypeError(message) : new Error(message);
  if (f.name !== "TypeError") e.name = f.name;
  if (f.code) e.code = f.code;
  if (typeof f.osError === "number") e.osError = f.osError;
  return e;
}

/**
 * A pending call. `raw` ones (desktop.ts's) resolve `{payload, value}` and reject with the host's
 * failure object; the rest resolve the payload and reject with `desktopError`.
 */
const desktopPending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void; raw: boolean }>();
const desktopEvents = new Map<string, (payload: Record<string, unknown>) => void>();

if (desktopHost) {
  desktopHost.setDispatcher((token, ok, json, value) => {
    let payload: Record<string, unknown> = {};
    try {
      payload = JSON.parse(json) as Record<string, unknown>;
    } catch {
      /* a malformed payload settles with {} rather than leaving the promise hanging */
    }
    if (token === 0) {
      desktopEvents.get(String(payload.kind))?.(payload);
      return;
    }
    const p = desktopPending.get(token);
    if (!p) return;
    desktopPending.delete(token);
    if (ok) p.resolve(p.raw ? ({ payload, value } satisfies DesktopSettle) : payload);
    else p.reject(p.raw ? payload : desktopError(payload as unknown as DesktopFailure));
  });
}

function desktopCall(json: VnJson): Promise<Record<string, unknown>> {
  return desktopAwait(json, false) as Promise<Record<string, unknown>>;
}

function desktopCallRaw(json: VnJson): Promise<DesktopSettle> {
  return desktopAwait(json, true) as Promise<DesktopSettle>;
}

function desktopAwait(json: VnJson, raw: boolean): Promise<unknown> {
  let out: { token?: number } & Partial<DesktopFailure>;
  try {
    out = JSON.parse(json) as typeof out;
  } catch {
    return Promise.reject(new Error(`the desktop host answered something that is not JSON: ${json}`));
  }
  if (out.error !== undefined || out.token === undefined) {
    const f = { name: out.name ?? "Error", ...out };
    return Promise.reject(raw ? f : desktopError(f));
  }
  const token = out.token;
  return new Promise((resolve, reject) => desktopPending.set(token, { resolve, reject, raw }));
}

// Transient activation: a keydown or mousedown the host delivered in the last five seconds
// (Chromium's lifespan). `el.click()` and `dispatchEvent` never activate; neither do Escape or a
// lone modifier.
const ACTIVATION_MS = 5000;
let activationAt = -Infinity;
let hasBeenActive = false;
const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "Fn", "OS"]);
const nowMs = () => Date.now();

function noteActivation(name: string, key: string | undefined): void {
  if (name === "keydown" && (key === "Escape" || (key !== undefined && MODIFIER_KEYS.has(key)))) return;
  activationAt = nowMs();
  hasBeenActive = true;
}

const activation = {
  isActive: () => nowMs() - activationAt <= ACTIVATION_MS,
  consume: () => void (activationAt = -Infinity),
};

if (typeof navigator !== "undefined" && navigator && vacant(navigator, "userActivation")) {
  try {
    Object.defineProperty(navigator, "userActivation", {
      value: mark(
        Object.freeze(
          Object.create(Object.prototype, {
            isActive: { get: activation.isActive, enumerable: true },
            hasBeenActive: { get: () => hasBeenActive, enumerable: true },
          }),
        ),
      ),
      configurable: true,
    });
  } catch {
    /* a frozen navigator keeps its own shape */
  }
}

const desktopApi: DesktopApi = installDesktop(globalThis as unknown as Record<string, unknown>, {
  host: desktopHost?.fs ?? null,
  engine: globalThis as unknown as DesktopEngine,
  call: desktopCallRaw,
  warnCompat,
  activation,
  realm: { vacant, mark },
});

if (typeof navigator !== "undefined" && navigator) {
  if (typeof __vn_badge_set === "function" && vacant(navigator, "setAppBadge")) {
    installBadge(navigator, { set: __vn_badge_set });
  }
  if (typeof __vn_media_session_update === "function" && vacant(navigator, "mediaSession")) {
    installMediaSession(navigator, {
      host: { update: __vn_media_session_update },
      resolveUrl: (src) => new URL(src, (globalThis as { location?: { href: string } }).location?.href).href,
      fetchBytes: (url) =>
        (globalThis.fetch as unknown as (u: string) => Promise<{ arrayBuffer(): Promise<ArrayBuffer> }>)(url).then(
          (r) => r.arrayBuffer(),
        ),
      onAction: (fn) => void desktopEvents.set("mediasession", fn),
      reportError: (e) => (globalThis as { reportError?: (e: unknown) => void }).reportError?.(e),
    });
  }
  if (typeof __vn_share === "function" && vacant(navigator, "share")) {
    const FileCtor = (globalThis as { File?: abstract new (...args: never[]) => object }).File;
    installShare(navigator, {
      host: { share: __vn_share },
      call: desktopCallRaw,
      resolveUrl: (u) => new URL(u, (globalThis as { location?: { href: string } }).location?.href).href,
      isFile: (v): v is { name: string; arrayBuffer(): Promise<ArrayBuffer> } =>
        typeof FileCtor === "function" && v instanceof FileCtor,
      activation,
      documentTitle: () => document?.title ?? "",
      DOMException: (globalThis as unknown as DesktopEngine).DOMException,
    });
  }
  if (typeof __vn_webauthn_create === "function" && vacant(navigator, "credentials")) {
    installCredentials(navigator, {
      host: {
        create: __vn_webauthn_create,
        get: __vn_webauthn_get,
        cancel: __vn_webauthn_cancel,
        available: __vn_webauthn_available,
      },
      call: desktopCall,
      pageUrl: () => (globalThis as { location?: { href: string } }).location?.href ?? "",
      DOMException: (globalThis as unknown as DesktopEngine).DOMException,
    });
  }
  if (typeof __vn_wake_lock_hold === "function" && vacant(navigator, "wakeLock")) {
    let hidden = false;
    const hiddenListeners: (() => void)[] = [];
    const hide = (): void => {
      hidden = true;
      for (const fn of hiddenListeners) fn();
    };
    (globalThis as unknown as Partial<VnEventTarget>).addEventListener?.("pagehide", hide);
    const appWindow = globalThis.__VN_WINDOW__?.appWindow;
    appWindow?.addEventListener("minimize", hide);
    appWindow?.addEventListener("restore", () => void (hidden = false));
    installWakeLock(navigator, {
      host: { hold: __vn_wake_lock_hold },
      engine: globalThis as unknown as WakeLockDeps["engine"],
      isHidden: () => hidden || appWindow?.isMinimized() === true,
      onHidden: (fn) => void hiddenListeners.push(fn),
    });
  }
}

/**
 * Hands a content URL to the system. The caller's allowlist is applied again by the host, so
 * neither side alone is the security line. Never throws; a failure is a console warning.
 */
function openContentUrl(url: string, via: string): void {
  if (!desktopHost) {
    warnCompat(
      "shell:absent",
      `${via}: the desktop host is missing (vn_desktop::install did not run) — the link was not ` + `opened.`,
    );
    return;
  }
  let href = url;
  try {
    if (typeof URL === "function") href = new URL(url).href;
  } catch {
    /* not a parseable URL: the host validates and refuses */
  }
  desktopCall(desktopHost.openExternal(href, true)).catch((e: AppDesktopError) => {
    if (e && e.code === "ESCHEME") return void adviseScheme(SCHEME_RE.exec(href)?.[1] ?? "?", via);
    console.warn(`[vue-native] ${via}: could not open ${href} — ${e && e.message}`);
  });
}

// `window.open(url)` for an external URL opens the system browser and returns `null` (noopener
// semantics). Anything else falls through to the prelude's `open`, which evaluates first.
{
  type OpenFn = (url?: unknown, target?: unknown, features?: unknown) => unknown;
  const g = globalThis as unknown as { open?: OpenFn };
  const prior = typeof g.open === "function" ? g.open : null;
  g.open = function open(url?: unknown, target?: unknown, features?: unknown) {
    const s = url == null ? "" : String(url).trim();
    const m = SCHEME_RE.exec(s);
    // A one-letter "scheme" is a drive letter (`C:\x`), not a URL.
    if (m && m[1]!.length > 1) {
      const scheme = m[1]!.toLowerCase();
      if (scheme !== "about" && scheme !== "javascript") {
        if (CONTENT_SCHEMES.has(scheme)) openContentUrl(s, "window.open()");
        else adviseScheme(scheme, "window.open()");
        return null;
      }
    }
    return prior ? prior(url, target, features) : null;
  };
}

// `appShell`: what a browser cannot do. Like Electron's `shell`, with two safer differences:
// `openPath` refuses programs and scripts unless `{ allowExecutable: true }`, and `openExternal`
// refuses `file:`, so a URL is never a way around that guard. Calls reject on failure
// (`ENOENT`, `ENOASSOC`) instead of resolving an error string.

interface AppShell {
  openExternal(url: string): Promise<void>;
  openPath(path: string, options?: { allowExecutable?: boolean }): Promise<void>;
  showItemInFolder(path: string): Promise<void>;
  trashItem(path: string): Promise<void>;
}

function createShell(host: DesktopHost): AppShell {
  const done = (p: Promise<unknown>): Promise<void> => p.then(() => undefined);
  return {
    /** Opens `url` with the system handler for its scheme (any but `file:`); resolves once the shell accepts it. */
    openExternal(url: string) {
      return done(desktopCall(host.openExternal(String(url), false)));
    },
    /** Opens a file or folder with its default app; `path` must be absolute. Programs and scripts are refused (`EEXECUTABLE`) unless `allowExecutable`. */
    openPath(path: string, options: { allowExecutable?: boolean } = {}) {
      return done(desktopCall(host.openPath(String(path), !!(options && options.allowExecutable))));
    },
    showItemInFolder(path: string) {
      return done(desktopCall(host.showItem(String(path))));
    },
    /** Moves `path` to the Recycle Bin, never a permanent delete: rejects (`ENOTRASH`) when there is none. */
    trashItem(path: string) {
      return done(desktopCall(host.trash(String(path))));
    },
  };
}

const inertShell = new Proxy(
  {},
  {
    get(_t, key) {
      warnCompat(
        "appShell",
        "the desktop host is missing (vn_desktop::install did not run) — `appShell` is inert and " +
          "every call rejects.",
      );
      return () =>
        Promise.reject(
          desktopError({
            name: "NotSupportedError",
            message: `appShell.${String(key)}: no desktop host`,
            code: "ENOSYS",
          }),
        );
    },
  },
);

export const appShell: AppShell = desktopHost ? createShell(desktopHost) : (inertShell as unknown as AppShell);

export interface ScreenCaptureSource {
  id: string;
  name: string;
  thumbnailDataUrl: string;
  appIconDataUrl: string | null;
  displayId: string;
  [key: string]: unknown;
}
export type CaptureSource = "microphone" | "camera" | "screen_share";
export interface CapturePermission {
  state: VnPermissionState;
  canRequest: boolean;
  deniedBySystem: boolean;
}
const CAPTURE_PERMISSIONS: Record<CaptureSource, VnPermissionName> = {
  microphone: "microphone",
  camera: "camera",
  screen_share: "display-capture",
};
function capturePermissionName(source: unknown): VnPermissionName {
  const name = Object.prototype.hasOwnProperty.call(CAPTURE_PERMISSIONS, source as string)
    ? CAPTURE_PERMISSIONS[source as CaptureSource]
    : undefined;
  if (!name) throw new TypeError("unknown capture source");
  return name;
}
export const appCapture = {
  permission(source: CaptureSource): CapturePermission {
    const name = capturePermissionName(source);
    if (typeof __vn_permissions_list !== "function")
      return { state: "unsupported", canRequest: false, deniedBySystem: false };
    const row = (JSON.parse(__vn_permissions_list()) as PermissionRow[]).find((r) => r.name === name);
    return {
      state: row ? row.state : "unsupported",
      canRequest: !!row?.canRequest,
      deniedBySystem: !!row?.deniedBySystem,
    };
  },
  /** Asks for the source: the app's own prompt, or the platform's settings page after a platform denial. */
  async requestPermission(source: CaptureSource): Promise<VnPermissionState> {
    return appPermissions.request(capturePermissionName(source));
  },
  permissionHint(source: CaptureSource): string {
    const name = capturePermissionName(source);
    if (typeof __vn_permissions_hint !== "function") return "";
    return __vn_permissions_hint(name);
  },
  async getSources(
    types: string[] = ["screen", "window"],
    options: Record<string, unknown> = {},
  ): Promise<ScreenCaptureSource[]> {
    if (!Array.isArray(types) || types.some((t) => t !== "screen" && t !== "window")) {
      throw new TypeError("capture source types must be screen or window");
    }
    if (!desktopHost?.screenSources) {
      throw desktopError({
        name: "NotSupportedError",
        message: "this host has no desktop capture backend",
        code: "ENOSYS",
      });
    }
    const result: unknown = await desktopCall(
      desktopHost.screenSources(JSON.stringify(types), JSON.stringify(options)),
    );
    if (!Array.isArray(result)) throw new TypeError("the desktop capture host returned an invalid source list");
    return result as ScreenCaptureSource[];
  },
  setPendingSource(sourceId: string, includeAudio = false): boolean {
    if (typeof sourceId !== "string" || !/^(screen|window):[0-9]+$/.test(sourceId)) {
      throw new TypeError("capture source must be a screen or window ID from getSources()");
    }
    (globalThis as { __VN_SCREEN_SOURCE__?: { sourceId: string; includeAudio: boolean } }).__VN_SCREEN_SOURCE__ = {
      sourceId,
      includeAudio: !!includeAudio,
    };
    return true;
  },
};
(globalThis as { __VN_CAPTURE__?: typeof appCapture }).__VN_CAPTURE__ = mark(appCapture);

// Tier 1 is the Launch Handler API (`launchQueue.setConsumer`): this process's own launch is
// buffered until a consumer is set, and every launch a second process forwards (`singleInstance`)
// is another. Tier 2, `appLaunch`, adds what a browser cannot say: raw argv, cwd, existing-path
// arguments, the single-instance lock, the `launch` event. Deep links and argv are untrusted input.

interface LaunchInfo {
  argv: string[];
  cwd: string;
  url: string | null;
  paths: string[];
  handles?: HandleWire[];
  singleInstance: boolean;
  id?: string;
  name?: string;
}

export interface VnLaunchEvent extends VnEvent {
  readonly argv: readonly string[];
  readonly cwd: string;
  readonly url: string | null;
  readonly paths: readonly string[];
  readonly source: "second-instance" | "open-url" | "reopen";
}

interface AppLaunch extends VnEventTarget {
  readonly argv: readonly string[];
  readonly cwd: string;
  readonly url: string | null;
  readonly paths: readonly string[];
  readonly singleInstance: boolean;
  registerProtocol(scheme: string): void;
  unregisterProtocol(scheme: string): boolean;
  isProtocolRegistered(scheme: string): boolean;
}

const launchInfo: LaunchInfo = (() => {
  const empty: LaunchInfo = { argv: [], cwd: "", url: null, paths: [], singleInstance: false };
  if (typeof __vn_launch_info !== "function") return empty;
  try {
    return { ...empty, ...(JSON.parse(__vn_launch_info()) as Partial<LaunchInfo>) };
  } catch {
    return empty;
  }
})();

/** `LaunchParams`: `targetURL` is the raw deep link; `files` holds a handle per existing path argument. */
class LaunchParams {
  readonly targetURL: string | null;
  readonly files: readonly unknown[];
  constructor(targetURL: string | null, files: readonly unknown[] = []) {
    this.targetURL = targetURL;
    this.files = Object.freeze(files.slice());
  }
}

type LaunchConsumer = (params: LaunchParams) => unknown;

class LaunchQueue {
  #consumer: LaunchConsumer | null = null;
  #pending: LaunchParams[] = [];

  setConsumer(consumer: LaunchConsumer): void {
    if (typeof consumer !== "function") {
      throw new TypeError("Failed to execute 'setConsumer' on 'LaunchQueue': parameter 1 is not a function.");
    }
    this.#consumer = consumer;
    const queued = this.#pending.splice(0);
    for (const p of queued) this.#deliver(p);
  }

  _enqueue(params: LaunchParams): void {
    if (this.#consumer) this.#deliver(params);
    else this.#pending.push(params);
  }

  #deliver(params: LaunchParams): void {
    try {
      void this.#consumer!(params);
    } catch (e) {
      // A consumer that throws must not take the next launch with it.
      console.error(`[vue-native] launchQueue consumer threw: ${e instanceof Error ? (e.stack ?? e.message) : e}`);
    }
  }
}

function launchParamsFor(url: string | null, handles: unknown): LaunchParams {
  const files = Array.isArray(handles)
    ? (handles as HandleWire[]).filter((w) => w && typeof w.h === "number").map((w) => desktopApi.handleFrom(w))
    : [];
  return new LaunchParams(url, files);
}

const launchQueue = new LaunchQueue();
{
  const g = globalThis as unknown as { launchQueue?: unknown; LaunchParams?: unknown; LaunchQueue?: unknown };
  if (vacant(g, "launchQueue")) {
    g.launchQueue = mark(launchQueue);
    g.LaunchParams = mark(LaunchParams);
    g.LaunchQueue = mark(LaunchQueue);
  }
}
// This process's own launch, once per process: a reload is not a launch.
if (typeof __vn_launch_info === "function" && REALM.runs === 1) {
  launchQueue._enqueue(launchParamsFor(launchInfo.url, launchInfo.handles));
}

function createLaunch(): AppLaunch {
  const target = new EventTarget();
  const frozen = <T>(v: T[]): readonly T[] => Object.freeze(v.slice());
  const argv = frozen(launchInfo.argv);
  const paths = frozen(launchInfo.paths);
  desktopEvents.set("launch", (p) => {
    const evArgv = frozen(Array.isArray(p.argv) ? (p.argv as string[]) : []);
    const evPaths = frozen(Array.isArray(p.paths) ? (p.paths as string[]) : []);
    const url = typeof p.url === "string" ? p.url : null;
    const ev = new Event("launch");
    for (const [k, v] of Object.entries({
      argv: evArgv,
      cwd: typeof p.cwd === "string" ? p.cwd : "",
      url,
      paths: evPaths,
      source: typeof p.source === "string" ? p.source : "second-instance",
    })) {
      Object.defineProperty(ev, k, { value: v, enumerable: true, configurable: true });
    }
    try {
      target.dispatchEvent(ev);
    } finally {
      launchQueue._enqueue(launchParamsFor(url, p.handles));
    }
  });
  return Object.defineProperties(target, {
    argv: { get: () => argv, enumerable: true },
    cwd: { get: () => launchInfo.cwd, enumerable: true },
    /** The deep link this process was started for (`myapp://…`), or null. UNTRUSTED. */
    url: { get: () => launchInfo.url, enumerable: true },
    paths: { get: () => paths, enumerable: true },
    /** Does THIS process hold the single-instance lock (`"singleInstance": true` and it was first)? */
    singleInstance: { get: () => launchInfo.singleInstance, enumerable: true },
    /**
     * Registers `scheme://` for this exe, per user (HKCU). Throws TypeError for an invalid or
     * reserved scheme or one another app owns. In a dev host it points the scheme at the dev slot.
     */
    registerProtocol: {
      value(scheme: string): void {
        if (typeof __vn_launch_register_protocol !== "function") {
          throw new TypeError("appLaunch.registerProtocol is unavailable: no desktop host");
        }
        const out = JSON.parse(__vn_launch_register_protocol(String(scheme))) as Partial<DesktopFailure>;
        if (out.error !== undefined) throw desktopError({ name: out.name ?? "TypeError", ...out });
      },
    },
    unregisterProtocol: {
      value(scheme: string): boolean {
        return typeof __vn_launch_unregister_protocol === "function" && __vn_launch_unregister_protocol(String(scheme));
      },
    },
    isProtocolRegistered: {
      value(scheme: string): boolean {
        return (
          typeof __vn_launch_is_protocol_registered === "function" && __vn_launch_is_protocol_registered(String(scheme))
        );
      },
    },
  }) as AppLaunch;
}

export const appLaunch: AppLaunch = createLaunch();

// `appDevtools` lets an app turn the devtools off at runtime (`disable()`, or `"devtools": false` in vn.config.json):
// the listener closes, inspectors detach, the token is blanked and the handshake file deleted; `enable()` binds a
// fresh port and token. `VN_DEVTOOLS=1` forces them on and refuses `disable()`.

interface DevtoolsHost {
  state: typeof __vn_devtools_state;
  setEnabled: typeof __vn_devtools_set_enabled;
  open: typeof __vn_devtools_open;
}

export interface DevtoolsInfo {
  enabled: boolean;
  locked: boolean;
  port: number | null;
  pid: number;
  key: boolean;
  inspectors: number;
  /** The dist slot whose `devtools.json` this host publishes into. */
  slot: string;
}

interface AppDevtools {
  /** False only for `--features no-devtools`; unlike `isEnabled()`, "not in the binary" cannot be reversed. */
  readonly available: boolean;
  isEnabled(): boolean;
  enable(): boolean;
  disable(): boolean;
  open(): boolean;
  info(): DevtoolsInfo;
  onChange(fn: (enabled: boolean) => void): () => void;
}

const devtoolsHost: DevtoolsHost | null =
  typeof __vn_devtools_state === "function"
    ? {
        state: __vn_devtools_state,
        setEnabled: __vn_devtools_set_enabled,
        open: __vn_devtools_open,
      }
    : null;

const devtoolsWatchers = new Set<(enabled: boolean) => void>();

function notifyDevtools(enabled: boolean) {
  for (const fn of [...devtoolsWatchers]) {
    try {
      fn(enabled);
    } catch (e) {
      // One listener's throw must not stop the others or fail a successful `disable()`.
      warnCompat("appDevtools.onChange", `a change listener threw and was ignored: ${String(e)}`);
    }
  }
}

function createDevtools(host: DevtoolsHost): AppDevtools {
  const read = (): DevtoolsInfo => JSON.parse(host.state()) as DevtoolsInfo;
  const set = (on: boolean): boolean => {
    const was = read().enabled;
    const out = JSON.parse(host.setEnabled(on)) as DevtoolsInfo & { error?: string };
    if (out.error) {
      // Refused (only by `VN_DEVTOOLS=1`), not failed: warn rather than throw so a shipped app's boot survives.
      warnCompat("appDevtools", `${on ? "enable" : "disable"}() was refused: ${out.error}`);
      return was;
    }
    if (out.enabled !== was) notifyDevtools(out.enabled);
    return out.enabled;
  };
  return {
    available: true,
    isEnabled: () => read().enabled,
    /** Binds a fresh port and token and republishes `<dist>/devtools.json`; idempotent, so an attached inspector keeps its port. */
    enable: () => set(true),
    /** Closes the listener and inspector sockets, blanks the token, deletes the handshake. Returns the state afterwards: `true` means `VN_DEVTOOLS=1` overruled it. */
    disable: () => set(false),
    open: () => {
      const out = JSON.parse(host.open()) as { opened?: boolean; error?: string };
      if (out.error) {
        warnCompat("appDevtools", `open() did not open an inspector: ${out.error}`);
        return false;
      }
      return !!out.opened;
    },
    info: () => read(),
    onChange(fn: (enabled: boolean) => void) {
      devtoolsWatchers.add(fn);
      return () => {
        devtoolsWatchers.delete(fn);
      };
    },
  };
}

/** `--features no-devtools`: nothing to switch; answers instead of throwing so `disable()` in an entry module is harmless. */
const inertDevtools: AppDevtools = {
  available: false,
  isEnabled: () => false,
  enable: () => {
    warnCompat(
      "appDevtools",
      "this host was built with `--features no-devtools`, so there are no devtools to enable " +
        "— `appDevtools.enable()` did nothing.",
    );
    return false;
  },
  disable: () => false,
  open: () => {
    warnCompat(
      "appDevtools",
      "this host was built with `--features no-devtools` — `appDevtools.open()` has no " + "inspector to open.",
    );
    return false;
  },
  info: () => ({
    enabled: false,
    locked: false,
    port: null,
    pid: 0,
    key: false,
    inspectors: 0,
    slot: "",
  }),
  onChange(fn: (enabled: boolean) => void) {
    devtoolsWatchers.add(fn);
    return () => {
      devtoolsWatchers.delete(fn);
    };
  },
};

export const appDevtools: AppDevtools = devtoolsHost ? createDevtools(devtoolsHost) : inertDevtools;

// `<Transition>`/`<TransitionGroup>` live in runtime-dom, but libraries import them from "vue" at module scope, so
// their absence breaks builds. Ported onto our transition engine.

const TRANSITION_PROPS = {
  name: String,
  type: String,
  css: { type: Boolean, default: true },
  duration: [String, Number, Object],
  enterFromClass: String,
  enterActiveClass: String,
  enterToClass: String,
  appearFromClass: String,
  appearActiveClass: String,
  appearToClass: String,
  leaveFromClass: String,
  leaveActiveClass: String,
  leaveToClass: String,
};

// runtime-dom records the phase class in `_vtc` and `classList`; here `writeClass` joins two
// sources, so record it once or it is emitted twice.
function addTransitionClass(el: MirrorElement, cls: string): void {
  (el._vtc || (el._vtc = new Set())).add(cls);
  writeClass(el);
}
function removeTransitionClass(el: MirrorElement, cls: string): void {
  if (el._vtc) {
    el._vtc.delete(cls);
    if (!el._vtc.size) el._vtc = undefined;
  }
  const base = (el._class || "").split(/\s+/);
  if (base.includes(cls)) el._class = base.filter((c) => c && c !== cls).join(" ");
  writeClass(el);
}

/// Two frames: one to commit the *-from class, the next to swap in *-to so the engine sees a real change.
function nextFrame(cb: () => void): void {
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf === "function") raf(() => raf(cb));
  else setTimeout(() => setTimeout(cb, 0), 0);
}

const toMs = (s: string): number => (s === "auto" || !s ? 0 : Number(String(s).slice(0, -1).replace(",", ".")) * 1000);
function listTimeout(delays: string[], durations: string[]): number {
  while (delays.length < durations.length) delays = delays.concat(delays);
  return Math.max(0, ...durations.map((d: string, i: number) => toMs(d) + toMs(delays[i]!)));
}

function transitionTimeout(el: MirrorElement): number {
  // `getComputedStyle` is installed by this file, so it is off the engine's declared surface; hence the cast.
  const styles = (
    globalThis as unknown as { getComputedStyle: (el: MirrorElement) => Record<string, string> }
  ).getComputedStyle(el);
  const split = (k: string) => String(styles[k] || "").split(", ");
  return listTimeout(split("transitionDelay"), split("transitionDuration"));
}

let endId = 0;
/// Ends a phase on the real `transitionend`, with the computed timeout as a safety net (the timer alone reports the
/// declared duration even when the engine cut the transition short).
function whenTransitionEnds(el: MirrorElement, explicitTimeout: number | null | undefined, resolve: () => void): void {
  const id = (el._endId = ++endId);
  let done = false;
  const resolveIfNotStale = () => {
    if (done || id !== el._endId) return;
    done = true;
    el.removeEventListener?.("transitionend", onEnd);
    resolve();
  };
  const onEnd = (e: VnEvent) => {
    // Only this element's own transitions end the phase; a descendant's bubbles through.
    if (e.target === el) resolveIfNotStale();
  };
  if (explicitTimeout != null) {
    setTimeout(resolveIfNotStale, explicitTimeout);
    return;
  }
  const timeout = transitionTimeout(el);
  if (!timeout) {
    resolve();
    return;
  }
  el.addEventListener?.("transitionend", onEnd);
  setTimeout(resolveIfNotStale, timeout + 1);
}

const callHook = (hook: unknown, args: unknown[] = []) => {
  if (Array.isArray(hook)) for (const h2 of hook) (h2 as (...a: unknown[]) => void)(...args);
  else if (hook) (hook as (...a: unknown[]) => void)(...args);
};
const hasExplicitCallback = (hook: unknown): boolean =>
  hook
    ? Array.isArray(hook)
      ? hook.some((h2) => (h2 as { length: number }).length > 1)
      : (hook as { length: number }).length > 1
    : false;

type DurationPair = { enter: unknown; leave: unknown };

function normalizeDuration(duration: unknown): [number, number] | null {
  if (duration == null) return null;
  if (typeof duration === "object")
    return [Number((duration as DurationPair).enter), Number((duration as DurationPair).leave)];
  const n = Number(duration);
  return [n, n];
}

interface TransitionRawProps {
  name?: string;
  type?: string;
  css?: boolean;
  duration?: number | string | { enter: number; leave: number };
  enterFromClass?: string;
  enterActiveClass?: string;
  enterToClass?: string;
  appearFromClass?: string;
  appearActiveClass?: string;
  appearToClass?: string;
  leaveFromClass?: string;
  leaveActiveClass?: string;
  leaveToClass?: string;
  [key: string]: unknown;
}

export function resolveTransitionProps(rawProps: TransitionRawProps) {
  const baseProps: Record<string, unknown> = {};
  for (const key in rawProps) if (!(key in TRANSITION_PROPS)) baseProps[key] = rawProps[key];
  if (rawProps.css === false) return baseProps;
  const {
    name = "v",
    duration,
    enterFromClass = `${name}-enter-from`,
    enterActiveClass = `${name}-enter-active`,
    enterToClass = `${name}-enter-to`,
    appearFromClass = enterFromClass,
    appearActiveClass = enterActiveClass,
    appearToClass = enterToClass,
    leaveFromClass = `${name}-leave-from`,
    leaveActiveClass = `${name}-leave-active`,
    leaveToClass = `${name}-leave-to`,
  } = rawProps;
  const durations = normalizeDuration(duration);
  const enterDuration = durations && durations[0];
  const leaveDuration = durations && durations[1];
  const {
    onBeforeEnter,
    onEnter,
    onEnterCancelled,
    onLeave,
    onLeaveCancelled,
    onBeforeAppear = onBeforeEnter,
    onAppear = onEnter,
    onAppearCancelled = onEnterCancelled,
  } = baseProps;

  const finishEnter = (el: MirrorElement, isAppear: boolean, done: (() => void) | undefined) => {
    removeTransitionClass(el, isAppear ? appearToClass : enterToClass);
    removeTransitionClass(el, isAppear ? appearActiveClass : enterActiveClass);
    done && done();
  };
  const finishLeave = (el: MirrorElement, done?: () => void) => {
    el._isLeaving = false;
    removeTransitionClass(el, leaveFromClass);
    removeTransitionClass(el, leaveToClass);
    removeTransitionClass(el, leaveActiveClass);
    done && done();
  };
  const makeEnterHook = (isAppear: boolean) => (el: MirrorElement, done: () => void) => {
    const hook = isAppear ? onAppear : onEnter;
    const resolve = () => finishEnter(el, isAppear, done);
    callHook(hook, [el, resolve]);
    nextFrame(() => {
      removeTransitionClass(el, isAppear ? appearFromClass : enterFromClass);
      addTransitionClass(el, isAppear ? appearToClass : enterToClass);
      if (!hasExplicitCallback(hook)) whenTransitionEnds(el, enterDuration, resolve);
    });
  };

  return Object.assign(baseProps, {
    onBeforeEnter(el: MirrorElement) {
      callHook(onBeforeEnter, [el]);
      addTransitionClass(el, enterFromClass);
      addTransitionClass(el, enterActiveClass);
    },
    onBeforeAppear(el: MirrorElement) {
      callHook(onBeforeAppear, [el]);
      addTransitionClass(el, appearFromClass);
      addTransitionClass(el, appearActiveClass);
    },
    onEnter: makeEnterHook(false),
    onAppear: makeEnterHook(true),
    onLeave(el: MirrorElement, done: () => void) {
      el._isLeaving = true;
      const resolve = () => finishLeave(el, done);
      addTransitionClass(el, leaveFromClass);
      addTransitionClass(el, leaveActiveClass);
      nextFrame(() => {
        if (!el._isLeaving) return;
        removeTransitionClass(el, leaveFromClass);
        addTransitionClass(el, leaveToClass);
        if (!hasExplicitCallback(onLeave)) whenTransitionEnds(el, leaveDuration, resolve);
      });
      callHook(onLeave, [el, resolve]);
    },
    onEnterCancelled(el: MirrorElement) {
      finishEnter(el, false, undefined);
      callHook(onEnterCancelled, [el]);
    },
    onAppearCancelled(el: MirrorElement) {
      finishEnter(el, true, undefined);
      callHook(onAppearCancelled, [el]);
    },
    onLeaveCancelled(el: MirrorElement) {
      finishLeave(el);
      callHook(onLeaveCancelled, [el]);
    },
  });
}

export const Transition = /*@__PURE__*/ defineComponent({
  name: "Transition",
  displayName: "Transition",
  inheritAttrs: false,
  props: /*@__PURE__*/ Object.assign({}, BaseTransitionPropsValidators, TRANSITION_PROPS),
  setup(props, { slots }) {
    return () => h(BaseTransition, resolveTransitionProps(props as TransitionRawProps), slots);
  },
});

export const TransitionGroup = /*@__PURE__*/ defineComponent({
  name: "TransitionGroup",
  props: /*@__PURE__*/ Object.assign({}, BaseTransitionPropsValidators, TRANSITION_PROPS, {
    tag: String,
    moveClass: String,
  }),
  setup(props, { slots }) {
    const instance = getCurrentInstance();
    const state = useTransitionState();
    let children: unknown[] | undefined;
    return () => {
      const rawProps = toRaw(props) as TransitionRawProps;
      const cssTransitionProps = resolveTransitionProps(rawProps);
      const tag = rawProps.tag || Fragment;
      if (rawProps.moveClass || (children && children.length)) {
        warnCompat(
          "TransitionGroup:move",
          "<TransitionGroup> animates enter and leave, but NOT the move of the items that stay " +
            "(the FLIP technique needs a per-element transform, and gpui applies no matrix to a " +
            "laid out box) — `moveClass` is ignored and reordered items jump to their new place",
        );
      }
      children = slots.default ? getTransitionRawChildren(slots.default()) : [];
      for (const child of children as VNode[]) {
        if (child.key != null) {
          setTransitionHooks(child, resolveTransitionHooks(child, cssTransitionProps, state, instance!));
        }
      }
      return createVNode(tag as never, null, children as never);
    };
  },
});

// `<VnWindow>`: a second OS window painting part of this tree, addressed by `mirrorNode.__vnId`.
export { VnWindow, VN_WINDOW_CREATION_TIME_PROPS } from "./vnwindow";
export type { VnWindowBounds, VnWindowMetrics, VnWindowTarget } from "./vnwindow";

// `appHost`: functions and events of a program embedding vue-native as a library; inert in the exe.
export { appHost } from "./apphost";
export { appTray } from "./tray";
export type { AppTray, TrayOptions, TrayPixels, TrayMenuItem, TrayMenuEntry } from "./tray";
export { appHotkeys } from "./hotkeys";
export type { AppHotkeys, HotkeyBinding, HotkeyButton, HotkeyChord, HotkeyEvent, HotkeyStatus } from "./hotkeys";
export { appLocale } from "./locale";
export type { AppLocale, MessageArgs, Messages } from "./locale";
export type { AppHost, AppHostError } from "./apphost";

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "vue-native" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
