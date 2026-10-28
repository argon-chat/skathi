// vue-native JS runtime: @vue/runtime-core custom renderer over the __vn_*
// host protocol (crates/vn-tree). Apps import `createApp` from "vue-native";
// compiled SFC render functions import their helpers from "vue" — both are
// aliased to this module by the bundler (packages/vue-native/build.ts).

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
  resolveTransitionHooks,
  setTransitionHooks,
  toRaw,
  useTransitionState,
  version,
} from "@vue/runtime-core";
// Type-only, so it is erased before the bundler ever sees it. `RendererOptions`
// is what makes `nodeOps` below self-typing: every hook's parameters come from
// Vue's own declaration, so a signature that drifts from what the renderer
// actually calls is a build error rather than an `undefined` at runtime.
import type { App, ComponentPublicInstance, RendererOptions, VNode } from "@vue/runtime-core";

// ---- the shapes this file is built out of ---------------------------------
// The host protocol itself is typed in ./protocol.d.ts; what follows is the JS
// side — the mirror tree, the listener registry, and the DOM surface installed
// on the two prototypes below. These interfaces are written as an INVENTORY of
// what answers truthfully: a member that is not here does not exist on a mirror
// node, and a reused browser component reaching for it gets the same TypeError
// it would get in an engine that never had one.

/** 1 element, 2 text, 3 comment. The mirror's own tag — not `nodeType`. */
type MirrorKind = 1 | 2 | 3;

/** What a listener registered through the renderer or `addEventListener` is. */
type EventHandler = (ev: VnEvent) => void;
/** …and what Vue passes for `@click="[a, b]"` — several, in order. */
type EventHandlerOrList = EventHandler | EventHandler[];
/** The third argument to add/removeEventListener, in both spellings. */
type ListenerOptions = boolean | { capture?: boolean; once?: boolean } | undefined;

/** One `addEventListener` registration; `once` is consumed on dispatch. */
interface ExtraListener {
  fn: EventHandler;
  once: boolean;
}

/**
 * Every listener for ONE event name on ONE target, split the way the DOM splits
 * them. `bubble`/`capture` are the RENDERER's slots — one each, because Vue
 * re-patches a prop rather than accumulating; `extraB`/`extraC` are the
 * `addEventListener` ones, which do accumulate.
 */
interface Buckets {
  bubble: EventHandlerOrList | null;
  capture: EventHandlerOrList | null;
  onceB: boolean;
  onceC: boolean;
  extraB: ExtraListener[];
  extraC: ExtraListener[];
}

/** event name -> its buckets, for one node id (or one global target). */
type HandlerMap = Record<string, Buckets | undefined>;

/** `getBoundingClientRect()`'s answer. `toJSON` because the DOM's has one. */
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

/** `el.classList`, over the class Vue assigned (transition classes ride `_vtc`). */
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

/**
 * `el.style`. A Proxy: the named members are real methods, anything else is a
 * property write that goes straight through `__vn_set_style` — CUSTOM
 * PROPERTIES INCLUDED, which is how reka-ui's `--reka-popper-*` sizing
 * variables reach the cascade.
 */
interface MirrorStyle {
  setProperty(key: string, value: string | number | null | undefined): void;
  removeProperty(key: string): string;
  getPropertyValue(key: string): string;
  cssText: string;
  [property: string]: unknown;
}

/** The members every mirror node has — elements and text/comment alike. */
interface MirrorNodeBase {
  id: VnNodeId;
  kind: MirrorKind;
  tag: string;
  /** A parent is always an element; the root's is null. */
  parentNode: MirrorElement | null;
  childNodes: MirrorNode[];
  /** A text/comment node's content. Elements carry theirs in `ownText`. */
  text?: string;
  // -- installed on BOTH prototypes (`NODE_MEMBERS`)
  contains(other: MirrorNode | null | undefined): boolean;
  readonly ownerDocument: VnDocument | undefined;
  readonly nodeType: 1 | 3 | 8 | 11;
  readonly nodeName: string;
  readonly isConnected: boolean;
  dispatchEvent(ev: VnEvent): boolean;
  /**
   * The event codes whose `__vn_set_listener` bit is currently ON in the host,
   * as far as this side knows. Absent = none. See `syncListenerFlag`.
   *
   * It lives on the NODE rather than in a side map keyed by id, and that is
   * load-bearing: `__vn_remove` recycles ids, so a map would answer for the
   * previous occupant of a slot. A fresh node is a fresh object with no set.
   */
  _listenerOn?: Set<VnEventCode>;
  /**
   * Vue writes symbol-keyed bookkeeping straight onto host nodes (the vModel
   * assign fn, the transition hooks). In a browser those land on a real
   * element; here they land here — and `__v_skip` on the prototype is what
   * stops `reactive()` wrapping the node in a Proxy before they do.
   */
  [key: symbol]: unknown;
}

/** A text or comment mirror. NOT an element: no attributes, no layout. */
interface MirrorText extends MirrorNodeBase {
  kind: 2 | 3;
  data: string;
  nodeValue: string;
  textContent: string;
}

/**
 * An element mirror, and with it the whole DOM surface `defineDom` installs.
 *
 * Everything below either answers TRUTHFULLY (from the mirror tree, or from the
 * host's real matcher / real layout bounds / real cascade), or answers a
 * degraded constant AND leaves exactly one advisory. Nothing here is a stub
 * that silently lies.
 */
interface MirrorElement extends MirrorNodeBase {
  kind: 1;
  tagName: string;
  /** `<input>`'s value, mirrored so `e.target.value` reads like the browser. */
  value: string;
  /** Browser-visible attribute state; `patchProp` is the only writer. */
  attrs: Record<string, string>;
  /** The class VUE assigned, kept apart from `<Transition>`'s `_vtc`. */
  className: string;
  /** The element's OWN text — exclusive with children, as in the host tree. */
  ownText?: string;
  addEventListener(name: string, fn: EventHandler, opts?: ListenerOptions): void;
  removeEventListener(name: string, fn: EventHandler, opts?: ListenerOptions): void;

  // -- the tree, truthfully (the mirror IS the tree)
  readonly children: MirrorElement[];
  readonly firstElementChild: MirrorElement | null;
  readonly lastElementChild: MirrorElement | null;
  readonly childElementCount: number;
  readonly nextElementSibling: MirrorElement | null;
  readonly previousElementSibling: MirrorElement | null;
  textContent: string;

  // -- selector matching, through the real cascade matcher
  matches(selector: string): boolean;
  closest(selector: string): MirrorElement | null;
  querySelector(selector: string): MirrorElement | null;
  querySelectorAll(selector: string): MirrorElement[];

  // -- layout, off the bounds gpui recorded for the last painted frame
  getBoundingClientRect(): MirrorRect;
  getClientRects(): MirrorRect[];
  readonly offsetWidth: number;
  readonly offsetHeight: number;
  readonly clientWidth: number;
  readonly clientHeight: number;
  readonly offsetTop: number;
  readonly offsetLeft: number;
  readonly offsetParent: null;
  readonly clientTop: number;
  readonly clientLeft: number;

  // -- scrolling, over the host's real `ScrollHandle` (see `hostScroll`)
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

  // -- the attribute channel
  getAttribute(name: string): string | null;
  hasAttribute(name: string): boolean;
  getAttributeNames(): string[];
  setAttribute(name: string, value: unknown): void;
  removeAttribute(name: string): void;
  toggleAttribute(name: string, force?: boolean): boolean;
  readonly dataset: Record<string, string | undefined>;
  readonly classList: MirrorClassList;
  readonly style: MirrorStyle;

  // -- focus and pointer capture
  focus(): void;
  blur(): void;
  setPointerCapture(pointerId: number): void;
  hasPointerCapture(pointerId: number): boolean;
  releasePointerCapture(pointerId: number): void;

  // -- the retained tree has no copy, and no insert from outside the renderer
  insertAdjacentElement(where: string, el: MirrorElement): MirrorElement;
  append(...nodes: unknown[]): void;
  remove(): void;
  cloneNode(): null;

  // -- HTMLMediaElement, read live off `__vn_media_state` (vn-video CONTRACT §6)
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
  readonly buffered: MirrorTimeRanges;
  readonly seekable: MirrorTimeRanges;
  readonly error: { code: number; message: string } | null;
  play(): Promise<void>;
  pause(): void;
  load(): void;
  canPlayType(type: string): string;

  // -- private bookkeeping, every field of it also present in runtime-dom
  /** The transition classes layered on top of `className`. */
  _vtc?: Set<string>;
  _isLeaving?: boolean;
  _endId?: number;
  _pointerCaptures?: Set<number>;
}

type MirrorNode = MirrorElement | MirrorText;

/** A mirror node mid-construction: both shapes' fields, before `kind` is fixed. */
type MirrorNodeInit = Omit<MirrorElement, "kind"> &
  Omit<MirrorText, "kind"> & { kind: MirrorKind };

/** The `TimeRanges` shape everything in the wild indexes, plus `ranges`. */
interface MirrorTimeRanges {
  length: number;
  start(i: number): number;
  end(i: number): number;
  ranges: { start: number; end: number }[];
}

/** Anything the propagation walk visits: a mirror node, `document`, `window`. */
type PropagationTarget = MirrorNode | object;

// ---- interning: names cross the boundary once, ids ever after -------------
const interned = new Map<string, VnNameId>();
const I = (s: string): VnNameId => {
  let v = interned.get(s);
  if (v === undefined) {
    v = __vn_intern(s);
    interned.set(s, v);
  }
  return v;
};

// Lockstep with crates/vn-tree (tree.rs EV_*).
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
  // <lottie> playback (crates/vn-lottie/CONTRACT.md §2). The names are
  // lottie-web's, camelCase and all, so `e.type` reads the same as it does on a
  // browser <lottie-player>.
  complete: 11,
  loopComplete: 12,
  enterFrame: 13,
  // The pointer family. gpui sources exactly three things — "the mouse moved
  // while over me" and the two edges of "the mouse is over me" — so only those
  // three have host codes; everything else in the W3C pointer set is DERIVED
  // from them (and from mousedown/mouseup) below.
  mousemove: 14,
  mouseenter: 15,
  mouseleave: 16,
  // <video> — the HTML media event set (crates/vn-video/CONTRACT.md §4). Every
  // one of these is also an MF_MEDIA_ENGINE_EVENT below 1000, the range
  // Microsoft documents as "the events defined in HTML 5 for media elements",
  // so the names are the spec's on both sides of the boundary and a component
  // ports from a browser unchanged.
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
  // Re-cadenced by the host to the spec's "no more often than every 250 ms" —
  // MF raises it at the decoder's rate, which is the display's.
  timeupdate: 27,
  durationchange: 28,
  ratechange: 29,
  volumechange: 30,
  waiting: 31,
  ended: 32,
  // `a` is MediaError.code (1-4), `key` the advisory naming the missing codec.
  error: 33,
  emptied: 34,
  // CSS animation/transition events. Unlike everything above, the SOURCE is
  // vn-css's own animator rather than gpui: it queues one per finished
  // transition track and per animation start/end/cancel, and the host drains
  // the queue once a frame (crates/vn-tree/src/render.rs::dispatch_anim_events).
  // The payload carries `elapsedTime` in `a` and the property / @keyframes
  // name in `key`.
  //
  // 35-38, not 17-20: the `<video>` track took 17-34 for the media block below
  // and the two landed in the same milestone. `Node::listeners` is a u64.
  transitionend: 35,
  animationstart: 36,
  animationend: 37,
  animationcancel: 38,
} as const;

/** Every event name the HOST delivers under a code of its own. */
type HostEventName = keyof typeof EV_CODES;
// Events the host does not deliver under their own code, synthesized in JS from
// one that it does. `pre: true` means the derived event fires BEFORE its source,
// which is the W3C order for the pointer family (`pointerdown` precedes
// `mousedown`); `focusin`/`focusout` are the bubbling siblings of `focus`/`blur`
// and fire after them.
//
// This is a real implementation, not a stub: a `pointerdown` listener registers
// the host's `mousedown` bit, and the payload it receives is the same one the
// mouse event carries — which is what makes reka-ui's Select (`onPointerDown`)
// and Tooltip (`onPointerEnter`) work rather than merely not throw.
const DERIVED = {
  pointerdown: { from: "mousedown", pre: true },
  pointerup: { from: "mouseup", pre: true },
  pointermove: { from: "mousemove", pre: true },
  pointerenter: { from: "mouseenter", pre: true },
  pointerleave: { from: "mouseleave", pre: true },
  focusin: { from: "focus", pre: false },
  focusout: { from: "blur", pre: false },
} as const satisfies Record<string, { from: HostEventName; pre: boolean }>;

/** Every event name SYNTHESIZED in JS from one the host does deliver. */
type DerivedEventName = keyof typeof DERIVED;

/** What `__vn_set_listener` has to be told about for this event name. */
const hostCode = (name: string): VnEventCode | undefined =>
  (EV_CODES as Record<string, VnEventCode | undefined>)[name] ??
  (EV_CODES as Record<string, VnEventCode | undefined>)[
    (DERIVED as Record<string, { from: HostEventName } | undefined>)[name]?.from as string
  ];
/** Every name that shares one host code — the flag is the OR over all of them. */
const NAMES_BY_CODE: Record<number, string[]> = (() => {
  const out: Record<number, string[]> = {};
  const add = (code: VnEventCode, name: string) => (out[code] ??= []).push(name);
  for (const name in EV_CODES) add(EV_CODES[name as HostEventName], name);
  for (const name in DERIVED) add(EV_CODES[DERIVED[name as DerivedEventName].from], name);
  return out;
})();
/** The derived events one host event produces, in dispatch order. */
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
]);
// Vue hyphenates an `@camelCase` listener prop (`onLoopComplete` becomes
// "loop-complete"), which is the well-known reason `@loopComplete` silently
// misses a web component's camelCase event in a browser. We accept BOTH
// spellings and fold them onto the canonical camelCase name, so `@loopComplete`
// works from a template and `addEventListener("loopComplete")` works from JS.
const EVENT_ALIASES: Record<string, string | undefined> = {
  "loop-complete": "loopComplete",
  loopcomplete: "loopComplete",
  "enter-frame": "enterFrame",
  enterframe: "enterFrame",
};
const canonicalEvent = (name: string): string => EVENT_ALIASES[name] ?? name;
// Per the UI Events spec these fire at the target only. (`scroll` bubbles from
// the *document*, never from an element.) The Lottie events are CustomEvents
// dispatched on the element by <lottie-player>, i.e. `bubbles: false`.
const NO_BUBBLE = new Set<string>([
  "focus",
  "blur",
  "scroll",
  "complete",
  "loopComplete",
  "enterFrame",
  // The enter/leave pair is the one part of the pointer family that does NOT
  // bubble — getting that wrong is what makes a menu close as soon as the
  // pointer crosses one of its own items.
  "mouseenter",
  "mouseleave",
  "pointerenter",
  "pointerleave",
  // Every media event fires AT THE TARGET only — HTML says so for all of them,
  // and it matters: a `@error` on a page wrapper would otherwise catch a
  // `<video>`'s failed load as if the wrapper had failed.
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
/** The EV_CODES names that come off a media element, in code order. */
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

// ---- JS mirror nodes ------------------------------------------------------
// Vue writes symbol props onto host nodes and needs parentNode/nextSibling
// synchronously — both served from this mirror, zero host calls.
// One-shot compat advisories: never silently drop behavior a browser has —
// tell the developer what happened and what to use instead.
const warned = new Set<string>();
function warnCompat(key: string, msg: string) {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[vue-native compat] ${msg}`);
}

// Host nodes sit on HTMLElement.prototype (redjs provides the real class) so
// `el instanceof HTMLElement` / `Node` checks in reused browser components
// pass, and carry the DOM-ish surface cheap to keep (tagName, value,
// add/removeEventListener).
// (…and when the engine has no HTMLElement at all — the bun test harness — a
// private prototype stands in, so the DOM surface below never lands on
// Object.prototype.)
const ElementProto: object =
  typeof HTMLElement === "function" ? HTMLElement.prototype : Object.create(Object.prototype);
// Text and comment mirrors are NOT elements — no selector matching, no layout,
// no attributes — but they are Nodes, and reused components branch on
// `nodeType === 3` all the time. They get their own prototype carrying only the
// node-level surface (see `NODE_MEMBERS` below).
const TextProto: object = Object.create(Object.prototype);

// A mirror node must never become reactive state.
//
// In a browser this is automatic: `Object.prototype.toString.call(divElement)`
// is `[object HTMLDivElement]`, which Vue's `getTargetType` classifies as
// INVALID, so `reactive(node)` hands the node straight back. Our mirrors are
// `Object.create(HTMLElement.prototype)` and stringify as `[object Object]`, so
// Vue would wrap each one in a Proxy — and then `set.has(node)`,
// `array.indexOf(node)` and `a === b` all start answering NO for the same node
// reached two different ways. reka-ui's DismissableLayer stack, its Collection
// ordering and its focus guards are ALL identity comparisons; every one of them
// silently breaks.
//
// `__v_skip` (Vue's `markRaw` flag) on the prototypes restores the browser's
// behaviour for every node at once, at zero per-node cost.
for (const proto of [ElementProto, TextProto]) {
  Object.defineProperty(proto, "__v_skip", { value: true, configurable: true });
}

function mk(id: VnNodeId, kind: 1, tag: string): MirrorElement;
function mk(id: VnNodeId, kind: 2 | 3, tag: string): MirrorText;
function mk(id: VnNodeId, kind: MirrorKind, tag: string): MirrorNode;
function mk(id: VnNodeId, kind: MirrorKind, tag: string): MirrorNode {
  // `MirrorNodeInit` is the one place the two shapes are held open at once: the
  // node is neither an element nor a text node until `kind` is written.
  const n = Object.create(kind === 1 ? ElementProto : TextProto) as MirrorNodeInit;
  n.id = id;
  n.kind = kind;
  n.tag = tag;
  n.parentNode = null;
  n.childNodes = [];
  if (kind === 1) {
    n.tagName = String(tag).toUpperCase();
    n.value = "";
    // Browser-visible attribute state. `patchProp` is the only writer that Vue
    // drives; reka-ui's roving focus and its Collection read `data-*`/`aria-*`
    // back off candidate nodes, so the mirror has to remember what was written
    // rather than answering "absent" for everything.
    n.attrs = Object.create(null);
    // The `class` Vue assigned, kept separately from the transition classes
    // `<Transition>` layers on top (runtime-dom keeps the same split, for the
    // same reason: a re-render must not wipe an in-flight `v-enter-active`).
    n.className = "";
    // `scrollTop` / `scrollLeft` are NOT initialised here any more: they are
    // accessors on the prototype now, reading and writing the host's real
    // `ScrollHandle` (see `defineDomGet("scrollTop", …)`). An own data property
    // would shadow them, which is exactly what used to make a write a no-op.
    //
    // Own-property methods shadow any prototype ones: these route into the
    // same registry the renderer's props use, so propagation sees both.
    n.addEventListener = (name, fn, opts) => addExtraListener(n as MirrorElement, name, fn, opts);
    n.removeEventListener = (name, fn, opts) =>
      removeExtraListener(n as MirrorElement, name, fn, opts);
  }
  nodesById.set(id, n as MirrorNode);
  return n as MirrorNode;
}

function forEachDeep(n: MirrorNode, f: (n: MirrorNode) => void) {
  f(n);
  for (const c of n.childNodes) forEachDeep(c, f);
}

const handlers = new Map<VnNodeId, HandlerMap>(); // node id -> { click: fn }

// Typed by Vue's own `RendererOptions`, minus the `patchProp` that is defined
// separately below: every hook's parameter types come from the renderer that
// calls them, so nothing here can drift out of step with what Vue passes.
const nodeOps: Omit<RendererOptions<MirrorNode, MirrorElement>, "patchProp"> = {
  createElement: (type) => mk(__vn_create_element(I(type)), 1, type),
  createText: (text) => {
    const n = mk(__vn_create_text(), 2, "#text");
    // The mirror keeps the string too — `textContent` and `Node.data` read it
    // back, and a host round trip for that would be absurd.
    n.text = String(text);
    __vn_set_text(n.id, n.text);
    return n;
  },
  createComment: () => mk(__vn_create_anchor(), 3, "#comment"),
  setText: (node, text) => {
    node.text = String(text);
    __vn_set_text(node.id, node.text);
  },
  setElementText: (node, text) => {
    for (const c of node.childNodes) {
      c.parentNode = null;
      forEachDeep(c, (n) => {
        handlers.delete(n.id);
        nodesById.delete(n.id);
      });
    }
    node.childNodes.length = 0;
    // The element's OWN text (exclusive with children, exactly as in the host
    // tree) — `textContent` reads it back.
    node.ownText = String(text);
    __vn_set_element_text(node.id, String(text));
    notifyMutation(node, "childList");
  },
  insert: (child, parent, anchor) => {
    // insert is also move: detach from the old mirror parent first.
    if (child.parentNode) {
      const cs = child.parentNode.childNodes;
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
    __vn_insert(child.id, parent.id, anchor ? anchor.id : 0);
    notifyMutation(parent, "childList", { addedNodes: [child], removedNodes: [] });
  },
  remove: (child) => {
    const p = child.parentNode;
    if (p) {
      const i = p.childNodes.indexOf(child);
      if (i >= 0) p.childNodes.splice(i, 1);
      child.parentNode = null;
      notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
    }
    forEachDeep(child, (n) => {
      handlers.delete(n.id);
      nodesById.delete(n.id);
    });
    __vn_remove(child.id);
  },
  parentNode: (node) => node.parentNode,
  nextSibling: (node) => {
    const p = node.parentNode;
    if (!p) return null;
    const i = p.childNodes.indexOf(node);
    return i >= 0 ? (p.childNodes[i + 1] ?? null) : null;
  },
  // <style scoped>: Vue stamps every element a scoped component renders (and
  // child component roots) with its data-v id — the Rust matcher buckets on it.
  setScopeId: (el, id) => __vn_add_scope(el.id, id),
  // Vue's `<Teleport>` resolves a STRING `to` through this hook. Without it
  // `<Teleport to="body">` — reka-ui's default for EVERY portal — warns
  // "Current renderer does not support string target" and renders nothing.
  //
  // The document roots a browser component reaches for all mean "the app root"
  // here; anything else goes through the real matcher.
  querySelector: (sel) => {
    if (sel === "body" || sel === "html" || sel === ":root" || sel === "#app") return appRoot;
    return queryOne(sel, appRoot ? appRoot.id : 0);
  },
};

// @vue/shared's hyphenate, verbatim: `\B` so a leading capital is only
// lowercased, never prefixed. Style keys are camelCase either way, but
// `withKeys` runs it over `KeyboardEvent.key` — "Enter" must become "enter",
// not "-enter", or every `@keydown.enter` silently stops matching.
const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

/// A `:style` key as CSS spells it. Custom properties are already CSS-cased and
/// are CASE-SENSITIVE, so hyphenating `--rekaPopperWidth` would rename it.
const styleKey = (k: string): string => (k.startsWith("--") ? k : hyphenate(k));

/**
 * The renderer's single prop hook, and also what `setAttribute` /
 * `removeAttribute` route through — so `prev`/`next` really are `unknown`: a
 * template can bind any value at all, and the five branches below are exactly
 * the five things that can be done with one.
 */
function patchProp(el: MirrorElement, key: string, prev: unknown, next: unknown): void {
  if (key === "style") {
    const p = (prev || {}) as Record<string, unknown>;
    const n = (next || {}) as Record<string, unknown>;
    for (const k in p) if (!(k in n)) __vn_set_style(el.id, I(styleKey(k)), "");
    for (const k in n) {
      if (p[k] === n[k]) continue;
      // runtime-dom's `setStyle` rule: a null/undefined value CLEARS the
      // property. Stringifying it instead writes the literal text "undefined",
      // which makes the whole declaration block unparseable — and
      // `:style="{ transform: maybeUndefined }"` is an extremely common shape
      // (it is what @floating-ui/vue's `floatingStyles` produces before its
      // first measurement).
      __vn_set_style(el.id, I(styleKey(k)), n[k] == null ? "" : String(n[k]));
    }
  } else if (key === "class") {
    if (prev !== next) {
      el.className = next == null ? "" : String(next);
      writeClass(el);
    }
  } else if (key.startsWith("on") && key.length > 2) {
    // runtime-dom's `parseName`, same order: strip the option suffixes, then
    // read the name.
    //   onClick / onClickCapture / onClickOnce / onClickCaptureOnce
    //   on:loopComplete — the case-PRESERVING spelling compiler-dom emits for
    //                     a custom element's camelCase event (`@loopComplete`
    //                     on <lottie>). Everything else is hyphenated.
    let rest = key.slice(2);
    let capture = false;
    let once = false;
    for (;;) {
      if (rest.endsWith("Once")) {
        once = true;
        rest = rest.slice(0, -4);
      } else if (rest.endsWith("Capture")) {
        capture = true;
        rest = rest.slice(0, -7);
      } else break;
    }
    const namespaced = rest.startsWith(":");
    // Hyphenate, don't lowercase — `\B` keeps a leading capital bare, so
    // "Click" -> "click" and "LoopComplete" -> "loop-complete", exactly as in a
    // browser. `canonicalEvent` folds that back onto the camelCase name the
    // host actually emits.
    const name = namespaced ? rest.slice(1) : canonicalEvent(hyphenate(rest));
    const code = hostCode(name);
    if (!code) {
      // Compat-first: a listener the host will never deliver must not vanish in
      // silence. `onUpdate:modelValue` & friends are excluded — a `:` inside
      // (rather than in front of) the name marks a component-emit prop that
      // leaked onto an element, which a browser also binds to an event that
      // never fires, also silently.
      if (namespaced || !rest.includes(":")) {
        warnCompat(
          `event:${name}`,
          `@${name} — this event type is not delivered by the native host yet; ` +
            `the handler will never fire`,
        );
      }
      return;
    }
    let m = handlers.get(el.id);
    if (!m) {
      m = {};
      handlers.set(el.id, m);
    }
    let buckets = ensureBuckets(m, name);
    if (capture) {
      buckets.capture = (next as EventHandlerOrList) || null;
      buckets.onceC = once;
    } else {
      buckets.bubble = (next as EventHandlerOrList) || null;
      buckets.onceB = once;
    }
    syncListenerFlag(el.id, name);
  } else if (key === "key" || key === "ref") {
    // stripped by the vnode layer; never reaches here — but be safe
  } else {
    // DOM-compat attribute channel: standard-ish props flow to the host as
    // strings (native widgets read what they understand); null clears.
    if (next == null) __vn_set_prop(el.id, I(key), undefined);
    else if (typeof next === "string" || typeof next === "number" || typeof next === "boolean")
      __vn_set_prop(el.id, I(key), String(next));
    else
      warnCompat(
        `prop:${key}`,
        `prop "${key}" with a non-primitive value is not supported by the native host yet; ` +
          `it was dropped — pass a string/number/boolean or handle it in the component`,
      );
    // …and stays readable from JS, which is what `getAttribute`/`dataset`/the
    // Collection ordering in reka-ui need.
    if (el.attrs) {
      const before = el.attrs[key];
      if (next == null) delete el.attrs[key];
      else if (typeof next !== "object" && typeof next !== "function") el.attrs[key] = String(next);
      if (before !== el.attrs[key]) {
        notifyMutation(el, "attributes", { attributeName: key, oldValue: before ?? null });
      }
    }
    if (key === "value") el.value = next == null ? "" : String(next);
    // An `<a href>` has an ACTIVATION BEHAVIOUR whether or not anything
    // listens: following the link is what a click on it means. The host only
    // delivers a click to a node whose click bit is on, so the bit follows the
    // href (`syncListenerFlag` ORs it in). docs/SHELL.md §1.
    if (key === "href" && el.tag === "a") syncListenerFlag(el.id, "click");
  }
}

/// The element's class attribute = what Vue assigned + whatever `<Transition>`
/// has layered on top. runtime-dom does exactly this (its `patchClass` merges
/// `el._vtc`); without it a re-render during a transition drops the phase class
/// and the animation stops halfway.
function writeClass(el: MirrorElement): void {
  const base = el.className || "";
  const extra = el._vtc;
  const value = extra && extra.size ? [base, ...extra].filter(Boolean).join(" ") : base;
  __vn_set_class(el.id, value);
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

/// Several event NAMES can share one host code (`pointerdown` rides
/// `mousedown`), so the flag is the OR over every name that maps to it —
/// switching a `pointerdown` listener off must not silence a `mousedown` one.
/// A custom event name has no host code at all and needs no flag: it is
/// dispatched from JS.
function syncListenerFlag(id: VnNodeId, name: string): void {
  const code = hostCode(name);
  if (!code) return;
  const m = handlers.get(id);
  let on = NAMES_BY_CODE[code].some((n) => m?.[n] && bucketsAlive(m[n]!));
  // Document- and window-level listeners ride the APP ROOT's bit: the host
  // delivers events to nodes, and the root is the node that stands in for the
  // document. See `globalHandlers` below.
  if (!on && appRoot && id === appRoot.id) {
    on = NAMES_BY_CODE[code].some((n) =>
      [...globalHandlers.values()].some((g) => g[n] && bucketsAlive(g[n]!)),
    );
  }
  // …and a link listens for its own click (its activation behaviour), so
  // removing the last `@click` from an `<a href>` must not switch it off.
  if (!on && code === EV_CODES.click) {
    const n = nodesById.get(id);
    if (n && isLink(n)) on = true;
  }
  // …and only tell the host when the ANSWER CHANGED.
  //
  // `set_listener` is idempotent — it sets or clears one bit of a u64 — so a
  // write of the value already there is a boundary crossing that buys nothing.
  // It was the single most repeated call in the whole protocol: every re-render
  // that re-patches an `on*` prop (a fresh inline arrow, or any full-props
  // diff) called it once per listening element, so a frame that changed ONE
  // TEXT NODE in a hundred-row list crossed the boundary a hundred times to
  // re-announce a `click` bit that was already set. See
  // `scripts/host-calls.ts`, which is how that was found.
  const node = nodesById.get(id);
  if (node) {
    const known = node._listenerOn;
    if (on) {
      if (known?.has(code)) return;
      (node._listenerOn ??= new Set()).add(code);
    } else {
      // Nothing recorded means the bit was never set, and the host starts every
      // node at zero — so there is nothing to clear.
      if (!known?.has(code)) return;
      known.delete(code);
    }
  }
  __vn_set_listener(id, code, on);
}

// ---- document / window level listeners ------------------------------------
// reka-ui's DismissableLayer (outside click, Escape), its focus guards and
// every "click outside to close" implementation in the ecosystem bind on
// `document` or `window`, neither of which is a node in the mirror tree. They
// get their own registry, joined onto the SAME propagation walk one level above
// the app root — which is exactly where a browser's event path puts them.
//
// Without this, a Dialog opens and never closes, and a menu stays open forever.
const globalHandlers = new Map<object, HandlerMap>(); // target object -> { [event name]: buckets }
/**
 * Is this one of OUR nodes? The propagation walk carries mirror nodes,
 * `document` and `window` in the same array, and this is what tells them apart
 * — so it is a type guard rather than a boolean, and the two registries that
 * follow it are looked up without a cast.
 */
const isMirrorNode = (n: unknown): n is MirrorNode =>
  !!n &&
  typeof (n as MirrorNode).id === "number" &&
  nodesById.get((n as MirrorNode).id) === (n as MirrorNode);

/** The two option spellings, as an object. `true` means `{ capture: true }`. */
type ListenerOptionsObject = { capture?: boolean; once?: boolean };

function addGlobalListener(
  target: object,
  rawName: string,
  fn: EventHandler | null | undefined,
  opts: ListenerOptions,
): boolean {
  if (!fn) return false;
  const name = canonicalEvent(rawName);
  if (!hostCode(name)) return false;
  let m = globalHandlers.get(target);
  if (!m) globalHandlers.set(target, (m = {}));
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  (capture ? buckets.extraC : buckets.extraB).push({ fn, once: !!(opts && (opts as ListenerOptionsObject).once) });
  if (appRoot) syncListenerFlag(appRoot.id, name);
  return true;
}

function removeGlobalListener(
  target: object,
  rawName: string,
  fn: EventHandler,
  opts: ListenerOptions,
): boolean {
  const name = canonicalEvent(rawName);
  const buckets = globalHandlers.get(target)?.[name];
  if (!buckets) return false;
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  if (appRoot) syncListenerFlag(appRoot.id, name);
  return true;
}

/// Installs the interception on one EventTarget-ish global. Events the host CAN
/// deliver are taken over; everything else still reaches the engine's own
/// EventTarget, so redjs' web APIs keep working.
/** What the interception needs off `document` / `globalThis`: two methods, maybe. */
type GlobalTargetLike = {
  addEventListener?: (name: string, fn: EventHandler, opts?: ListenerOptions) => void;
  removeEventListener?: (name: string, fn: EventHandler, opts?: ListenerOptions) => void;
};

function interceptGlobalTarget(target: object | undefined): void {
  if (!target) return;
  const nativeAdd = typeof (target as GlobalTargetLike).addEventListener === "function" ? (target as GlobalTargetLike).addEventListener!.bind(target) : null;
  const nativeRemove =
    typeof (target as GlobalTargetLike).removeEventListener === "function" ? (target as GlobalTargetLike).removeEventListener!.bind(target) : null;
  try {
    Object.defineProperty(target, "addEventListener", {
      value: (name: string, fn: EventHandler, opts: ListenerOptions) => {
        if (!addGlobalListener(target, name, fn, opts)) nativeAdd?.(name, fn, opts);
      },
      configurable: true,
      writable: true,
    });
    Object.defineProperty(target, "removeEventListener", {
      value: (name: string, fn: EventHandler, opts: ListenerOptions) => {
        if (!removeGlobalListener(target, name, fn, opts)) nativeRemove?.(name, fn, opts);
      },
      configurable: true,
      writable: true,
    });
  } catch (e) {
    warnCompat(
      "global-listeners",
      "document/window event listeners could not be intercepted on this engine — a component " +
        `that closes itself on an outside click or Escape will stay open (${e})`,
    );
  }
}

// EventTarget-shaped registration for reused browser components/directives
// (vModelText uses this path). Merged into the same propagation walk.
/// Does this name look like an event the PLATFORM sources? Those are the ones
/// worth an advisory when the host cannot deliver them — a listener for one will
/// simply never fire. A name with a dot or a capital in it is a component's own
/// custom event (`"select.select"`, `"focusScope.autoFocusOnMount"` — reka-ui
/// dispatches a dozen of them), and those work fine: they are dispatched from JS
/// through [`dispatchEvent`] and never touch the host at all.
const looksNative = (name: string): boolean => /^[a-z]+$/.test(name);

/// Events a BROWSER only ever fires on `window` (or `document`), never on an
/// element. `el.addEventListener("resize", …)` is dead code in Chrome too, so
/// warning about it here would report a divergence that does not exist — and a
/// component that hedges by attaching to both targets would cost a log line on
/// every app. On `window` these do work: `interceptGlobalTarget` hands anything
/// the host does not source to redjs's own EventTarget, which is where the
/// trusted `resize` from `redjs_dom_resize` is delivered.
const WINDOW_ONLY_EVENTS = new Set<string>([
  "resize", "load", "unload", "beforeunload", "pageshow", "pagehide",
  "hashchange", "popstate", "online", "offline", "storage", "message",
  "languagechange", "rejectionhandled", "unhandledrejection",
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
  if (!code && looksNative(name)) {
    warnCompat(
      `event:${name}`,
      `addEventListener("${rawName}") — this event type is not delivered by the native host yet; ` +
        `the listener will never fire`,
    );
    return;
  }
  let m = handlers.get(node.id);
  if (!m) {
    m = {};
    handlers.set(node.id, m);
  }
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const entry = { fn, once: !!(opts && (opts as ListenerOptionsObject).once) };
  (capture ? buckets.extraC : buckets.extraB).push(entry);
  syncListenerFlag(node.id, name);
}

function removeExtraListener(
  node: MirrorElement,
  rawName: string,
  fn: EventHandler,
  opts: ListenerOptions,
): void {
  const name = canonicalEvent(rawName);
  const buckets = handlers.get(node.id)?.[name];
  if (!buckets) return;
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  syncListenerFlag(node.id, name);
}

// ---- the DOM surface on mirror nodes (compat roadmap G2) ------------------
// Reused browser components call element methods unconditionally, and redjs'
// HTMLElement.prototype carries none — so the absence is a TypeError rather
// than a wrong answer. Everything below either answers TRUTHFULLY (from the
// mirror tree, or from the host's real matcher / real layout bounds), or
// answers a degraded constant AND leaves exactly one advisory.
//
// The two questions that cannot be answered in JS go to the host:
//   __vn_query          selector matching, through the CASCADE'S OWN matcher —
//                       a second JS-side matcher would be a silent divergence
//   __vn_bounds         the rectangle gpui laid the node out at last frame
//   __vn_computed_style the folded computed values + custom properties

let appRoot: MirrorElement | null = null; // set by createApp().mount()
// Replays the pre-mount `documentElement` writes onto the real root. Installed by
// the document surface below; called by mount().
let flushDocumentElement: ((root: MirrorElement) => void) | null = null;

/**
 * Any function installed on a mirror prototype. The receiver is annotated at
 * each call site (`function (this: MirrorElement) …`), which is what makes
 * `noImplicitThis` catch a member installed on the wrong prototype — the whole
 * reason `NODE_MEMBERS` and the element-only default are separate lists.
 */
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
  for (const proto of protos) Object.defineProperty(proto, name, { get, set, configurable: true });
}
/** `createTreeWalker`'s third argument, in both DOM spellings. */
type TreeWalkerFilter =
  | ((node: MirrorNode) => number)
  | { acceptNode?: (node: MirrorNode) => number }
  | null;

/// The members every Node has, elements and text/comment mirrors alike.
const NODE_MEMBERS: object[] = [ElementProto, TextProto];

/// `__vn_query`, decoded. The host answers `"!<advisory>"` for a selector the
/// engine cannot run (`:has()`, `::before`, `:visited`) — the same text a rule
/// using it would print — so an unanswerable query explains itself instead of
/// quietly reporting "no match".
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
    const node = nodesById.get(Number(part));
    // The cascade's matcher only ever answers with ELEMENTS — text and comment
    // nodes are not selectable — so the narrowing is the host's guarantee, not
    // an assumption of ours.
    if (node) out.push(node as MirrorElement);
  }
  return out;
}
const queryOne = (
  selector: string,
  rootId: VnNodeId,
  mode: VnQueryMode = 0,
): MirrorElement | null => hostQuery(selector, rootId, mode)[0] ?? null;

const ZERO_RECT = {
  x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0,
};

/// The viewport, as `document.documentElement` reports it.
///
/// The coordinate space every rect here lives in is the WINDOW'S CLIENT AREA
/// (that is what gpui hands each element during prepaint), and the app root is
/// inset within it by whatever chrome the host draws. So the viewport is the
/// rectangle from the client-area origin to the far corner of the app root —
/// NOT the root's own box. Reporting the root's size instead would leave
/// @floating-ui clipping against `(0,0,rootW,rootH)` while measuring elements in
/// a space offset by the chrome, and every popover would be pushed against the
/// left edge by the `shift` middleware.
function viewportRect(): MirrorRect {
  if (!appRoot) return { ...ZERO_RECT };
  const r = rectOf(appRoot);
  const width = Math.round(r.x + r.width);
  const height = Math.round(r.y + r.height);
  return { x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height };
}
function rectOf(node: MirrorNode): MirrorRect {
  const raw = node.id ? __vn_bounds(node.id) : "";
  // A node that was not painted (detached, `display: none`, parked in a
  // <KeepAlive> cache) reports nothing — and an all-zero rect is exactly what a
  // browser reports for those, so no advisory is warranted.
  if (!raw) return { ...ZERO_RECT, toJSON() { return this; } };
  const [x, y, w, h] = raw.split(",").map(Number);
  return {
    x, y, width: w, height: h,
    top: y, left: x, right: x + w, bottom: y + h,
    toJSON() { return this; },
  };
}

// -- truthful: the mirror IS the tree
defineDom(
  "contains",
  function (this: MirrorNode, other: MirrorNode | null | undefined) {
    for (let n: MirrorNode | null | undefined = other; n; n = n.parentNode)
      if (n === this) return true;
    return false;
  },
  NODE_MEMBERS,
);
defineDomGet("ownerDocument", () => globalThis.document, undefined, NODE_MEMBERS);
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
    // `tagName` is an ELEMENT field; on a text/comment mirror it is genuinely
    // absent, which is what the `??` is reading. The cast says so out loud
    // rather than pretending the union has it.
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
// A text/comment mirror's content, under both DOM names.
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
// `childNodes` is the renderer's own list: every child node, in order.
// `children` is the DOM's ELEMENT-only view of it, and the distinction is
// load-bearing — `aria-hidden` (which reka's Dialog uses to hide the rest of
// the page) walks `parent.children` and calls `getAttribute` on each, so
// handing it text and comment mirrors made it log one failure per node.
const elementChildren = (n: MirrorNode): MirrorElement[] =>
  (n.childNodes || []).filter((c): c is MirrorElement => c.kind === 1);
defineDomGet("children", function (this: MirrorElement) {
  return elementChildren(this);
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
  if (!p) return null;
  for (let i = p.childNodes.indexOf(node) + step; i >= 0 && i < p.childNodes.length; i += step) {
    if (p.childNodes[i].kind === 1) return p.childNodes[i] as MirrorElement;
  }
  return null;
}
defineDomGet("nextElementSibling", function (this: MirrorElement) {
  return elementSibling(this, 1);
});
defineDomGet("previousElementSibling", function (this: MirrorElement) {
  return elementSibling(this, -1);
});
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
    nodeOps.setElementText(this, value == null ? "" : String(value));
  },
);

// -- selector matching, through the real cascade matcher
defineDom("matches", function (this: MirrorElement, selector: string) {
  return hostQuery(selector, this.id, 1).length > 0;
});
defineDom("closest", function (this: MirrorElement, selector: string) {
  return queryOne(selector, this.id, 2);
});
defineDom("querySelector", function (this: MirrorElement, selector: string) {
  return queryOne(selector, this.id, 0);
});
defineDom("querySelectorAll", function (this: MirrorElement, selector: string) {
  return hostQuery(selector, this.id, 0);
});

// -- layout, off the bounds gpui recorded for the last painted frame
defineDom("getBoundingClientRect", function (this: MirrorElement) {
  return rectOf(this);
});
defineDom("getClientRects", function (this: MirrorElement) {
  const r = rectOf(this);
  return r.width || r.height ? [r] : [];
});
for (const [name, pick] of [
  ["offsetWidth", (r: MirrorRect) => r.width],
  ["offsetHeight", (r: MirrorRect) => r.height],
  ["clientWidth", (r: MirrorRect) => r.width],
  ["clientHeight", (r: MirrorRect) => r.height],
] as [string, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    return Math.round(pick(rectOf(this)));
  });
}
// `scrollWidth` / `scrollHeight` are the CONTENT box, not the painted one.
// For a scroll container the two differ by exactly the thing that makes the
// box a scroll container, and `el.scrollTop = el.scrollHeight` — the
// scroll-to-bottom idiom — is wrong by the whole overflow if this answers the
// viewport. For everything else a browser reports the border box, which is
// what the rectangle already is.
for (const [name, pick, fallback] of [
  ["scrollWidth", (s: ScrollPos) => s.scrollWidth, (r: MirrorRect) => r.width],
  ["scrollHeight", (s: ScrollPos) => s.scrollHeight, (r: MirrorRect) => r.height],
] as [string, (s: ScrollPos) => number, (r: MirrorRect) => number][]) {
  defineDomGet(name, function (this: MirrorElement) {
    const s = hostScroll(this, 0, NaN, NaN, true);
    return Math.round(s ? pick(s) : fallback(rectOf(this)));
  });
}
// `offset*` is measured from the offsetParent in the DOM; we have no positioned
// -ancestor chain to measure from, so these are window-relative.
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
    return Math.round(pick(rectOf(this)));
  });
}
defineDomGet("offsetParent", () => null);
// The border widths a browser reports here. gpui's painted rectangle is the
// BORDER box and the cascade's border widths are not published, so 0 is the
// honest constant — @floating-ui adds these to every clipping rect, and
// `undefined` there turns the whole computation into NaN.
defineDomGet("clientTop", () => 0);
defineDomGet("clientLeft", () => 0);

// ---- programmatic scrolling ----------------------------------------------
//
// `scrollTop` / `scrollLeft` / `scrollTo` / `scrollBy` / `scrollIntoView` all
// drive the host's real `ScrollHandle` — the same object the wheel, the
// scrollbar thumb and the devtools `scroll` command move
// (`crates/vn-tree/src/scroll.rs`). Three properties carry over from a
// browser and are worth stating, because each one is load-bearing somewhere:
//
//  * the write is CLAMPED to what the box can cover, and the getter answers
//    the clamped value SYNCHRONOUSLY — `el.scrollTop = 1e9` then reading it
//    back gives the bottom, with no frame in between;
//  * a write on an element that does not scroll does nothing and says
//    nothing, exactly as in a browser;
//  * the `scroll` EVENT is asynchronous. CSSOM-View fires it from the
//    update-the-rendering loop, not from the assignment, and so does the host.

/** What `__vn_scroll` answers, decoded. */
interface ScrollPos {
  left: number;
  top: number;
  scrollWidth: number;
  scrollHeight: number;
  /** The content size and the clamp are a frame old — see `noteStale`. */
  stale: boolean;
}

/**
 * One host call for every scroll accessor. `null` means "this element is not a
 * scroll container", which is the answer a browser gives by reporting zeros.
 *
 * `sizeMatters` asks for the advisory in `noteStale` when the answer's CONTENT
 * half is a frame old. The offset half never is, so a plain `scrollTop` read
 * passes `false` and stays silent.
 */
function hostScroll(
  node: MirrorElement,
  op: 0 | 1 | 2,
  x = NaN,
  y = NaN,
  sizeMatters = false,
): ScrollPos | null {
  if (!node.id) return null;
  const raw = __vn_scroll(node.id, op, x, y);
  if (!raw) return null;
  const [left, top, scrollWidth, scrollHeight, stale] = raw.split(",").map(Number);
  if (stale && sizeMatters) noteStale();
  return { left, top, scrollWidth, scrollHeight, stale: !!stale };
}

/**
 * The one honest gap in this surface, named once per session.
 *
 * A browser lays out on read, so `list.append(row); list.scrollTop =
 * list.scrollHeight` sees the row. Here layout is a frame, and the host can
 * only force one where a second `Window::draw` is legal — during boot, module
 * evaluation and `onMounted`, and NOWHERE inside an event handler or a timer,
 * because those already run inside the window's own update. So a content-size
 * read (or the clamp that uses it) in the same JS wave as a DOM change answers
 * the last painted frame, and "scroll to bottom" lands one item short.
 *
 * The offset itself is never stale — `el.scrollTop` is exact at all times, and
 * so is a scroll to a position the app computed itself. This is only about
 * `scrollWidth` / `scrollHeight` and the clamp derived from them.
 */
function noteStale(): void {
  warnCompat(
    "scroll-layout",
    "scrollWidth/scrollHeight (and the clamp on a scroll) answered from the LAST PAINTED frame: " +
      "the DOM changed in this same wave and a layout cannot be forced from inside an event " +
      "handler here (it can during boot/onMounted). `el.scrollTop = el.scrollHeight` right after " +
      "adding content therefore stops one item short — do it inside requestAnimationFrame(), " +
      "which runs after the frame that laid the new content out",
  );
}

/**
 * `behavior: "smooth"` — performed instantly, once per session.
 *
 * The final position is identical, which is the half app logic reads back; the
 * interpolation is the half that is missing. It is not faked with the CSS
 * animator on purpose: a scroll offset is not a style property, a running
 * animation switches the retained-render cache off document-wide (so a smooth
 * scroll would cost exactly the frames caching a long list saves), and a
 * smooth scroll that cannot be cancelled by the wheel and never fires
 * `scrollend` is a worse divergence than an instant jump that says so.
 * See research/reports/m23/scroll.md.
 */
function noteBehavior(behavior: unknown): void {
  if (behavior !== "smooth") return;
  warnCompat(
    "scroll-behavior",
    'scroll behavior: "smooth" is performed INSTANTLY — the final position is exact and readable ' +
      "on the next line, but there is no interpolation and no `scrollend` event; " +
      'use behavior: "auto" (or "instant") to say so explicitly',
  );
}

/** `scrollTo(x, y)` and `scrollTo({ left, top, behavior })`, both spellings. */
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
      // A browser rounds these to integers for a non-fractional layout; ours
      // are gpui pixels, which are already the units the box was laid out in.
      const s = hostScroll(this, 0);
      return s ? s[axis] : 0;
    },
    function (this: MirrorElement, value: never) {
      const v = Number(value);
      // A browser ignores a non-finite assignment rather than throwing.
      if (!Number.isFinite(v)) return;
      // `sizeMatters`: the write is CLAMPED against the content size, so a
      // stale content size makes a stale landing.
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

/** `start | center | end | nearest` → the host's alignment code. */
const SCROLL_ALIGN: Record<string, VnScrollAlign> = {
  start: 0,
  center: 1,
  end: 2,
  nearest: 3,
};

defineDom("scrollIntoView", function (this: MirrorElement, arg: unknown) {
  if (!this.id) return;
  // The three spellings a browser takes: nothing / `true` is
  // `{block: "start", inline: "nearest"}`, `false` is `{block: "end"}`, and an
  // object names either axis.
  let block: VnScrollAlign = 0;
  let inline: VnScrollAlign = 3;
  if (arg === false) block = 2;
  else if (arg !== null && typeof arg === "object") {
    const o = arg as { block?: unknown; inline?: unknown; behavior?: unknown };
    noteBehavior(o.behavior);
    if (o.block !== undefined) block = SCROLL_ALIGN[String(o.block)] ?? 0;
    if (o.inline !== undefined) inline = SCROLL_ALIGN[String(o.inline)] ?? 3;
  }
  // `block` / `inline` are writing-mode relative in CSS; we have one writing
  // mode (horizontal-tb, ltr), so block is vertical and inline horizontal.
  // That is stated rather than assumed because `direction: rtl` would flip the
  // inline axis and nothing here would notice.
  const out = __vn_scroll_into_view(this.id, block, inline);
  if (out.charCodeAt(0) === 33 /* ! */) warnCompat("scrollIntoView", out.slice(1));
  else if (out === "stale") noteStale();
});

// -- the attribute channel (`patchProp` is the writer; see `mk`)
defineDom("getAttribute", function (this: MirrorElement, name: unknown) {
  const v = this.attrs?.[String(name)];
  return v === undefined ? null : v;
});
defineDom("hasAttribute", function (this: MirrorElement, name: unknown) {
  return this.attrs ? String(name) in this.attrs : false;
});
defineDom("getAttributeNames", function (this: MirrorElement) {
  return this.attrs ? Object.keys(this.attrs) : [];
});
defineDom("setAttribute", function (this: MirrorElement, name: unknown, value: unknown) {
  const key = String(name);
  // `style` is a declaration BODY as an attribute, not the object `patchProp`'s
  // `:style` path expects.
  if (key === "style") {
    this.style.cssText = value == null ? "" : String(value);
    return;
  }
  patchProp(this, key, undefined, value == null ? "" : String(value));
});
defineDom("removeAttribute", function (this: MirrorElement, name: unknown) {
  patchProp(this, String(name), undefined, null);
});
defineDom("toggleAttribute", function (this: MirrorElement, name: string, force?: boolean) {
  const on = force === undefined ? !this.hasAttribute(name) : !!force;
  if (on) this.setAttribute(name, "");
  else this.removeAttribute(name);
  return on;
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
  Object.defineProperty(this, "dataset", { value: proxy, configurable: true });
  return proxy;
});

// -- classList, over the class Vue assigned (transition classes ride `_vtc`)
defineDomGet("classList", function (this: MirrorElement) {
  const node = this;
  const parts = () => (node.className ? node.className.split(/\s+/).filter(Boolean) : []);
  const commit = (list: string[]) => {
    node.className = list.join(" ");
    writeClass(node);
  };
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
  Object.defineProperty(this, "classList", { value: list, configurable: true });
  return list;
});

// -- inline style: writes go through the same host call `:style` makes, so this
//    is genuinely functional — CUSTOM PROPERTIES INCLUDED, which is how
//    reka-ui's `--reka-popper-*` sizing variables reach the cascade.
defineDomGet("style", function (this: MirrorElement) {
  const node = this;
  const own: Record<string, string> = Object.create(null);
  const write = (k: string | symbol, v: unknown) => {
    const key = String(k);
    const css = key.startsWith("--") ? key : hyphenate(key);
    own[key] = v == null ? "" : String(v);
    __vn_set_style(node.id, I(css), v == null ? "" : String(v));
  };
  // The target carries the REAL members; the handler routes everything else
  // into `write`. `as MirrorStyle` is the one place the two halves are joined:
  // a Proxy's type is its target's, and the target deliberately does not carry
  // the open-ended `[property: string]` half that the handler serves.
  const style = new Proxy(
    {
      setProperty: (k: string, v: unknown) => write(k, v),
      removeProperty: (k: string) => {
        const prev = own[String(k)] ?? "";
        write(k, "");
        return prev;
      },
      getPropertyValue: (k: string) => own[String(k)] ?? "",
      get cssText() {
        return Object.keys(own)
          .filter((k) => own[k] !== "")
          .map((k) => `${k.startsWith("--") ? k : hyphenate(k)}: ${own[k]}`)
          .join("; ");
      },
      set cssText(v: string) {
        for (const decl of String(v).split(";")) {
          const i = decl.indexOf(":");
          if (i > 0) write(decl.slice(0, i).trim(), decl.slice(i + 1).trim());
        }
      },
    } as MirrorStyle,
    {
      get: (t, k) => (k in t ? t[k as string] : (own[k as string] ?? "")),
      set: (t, k, v) => {
        if (!(k in t)) write(k, v);
        else t[k as string] = v;
        return true;
      },
    },
  );
  Object.defineProperty(this, "style", { value: style, configurable: true });
  return style;
});

// -- focus: gpui owns it and exposes no programmatic call to JS yet
defineDom("focus", function () {
  warnCompat(
    "focus",
    "element.focus() is a no-op — gpui owns focus and the host exposes no programmatic focus " +
      "call to JS yet, so focus traps and roving (arrow-key) focus are inert; clicking and Tab " +
      "still move focus for real",
  );
});
defineDom("blur", function () {});

// -- dispatchEvent: a component's own events, through the SAME capture/bubble
//    walk the host's events take. reka-ui builds its whole selection protocol
//    out of this (`select.select`, `menu.itemSelect`,
//    `dismissableLayer.pointerDownOutside`, `focusScope.autoFocusOnMount`): it
//    adds a one-shot listener for a custom name and immediately dispatches it,
//    so without this an option click reaches a handler that is never called.
defineDom(
  "dispatchEvent",
  function (this: MirrorNode, ev: VnEvent) {
    if (!ev || typeof ev.type !== "string") return true;
    const name = canonicalEvent(ev.type);
    const proceed = propagate(this, name, ev);
    // The DOM's activation step: a dispatched `MouseEvent("click")` that
    // nobody cancelled follows a link exactly as a real click does — which is
    // the FileSaver.js / "build an <a>, dispatch a click" idiom, and what
    // `el.click()` below is built on.
    if (proceed && name === "click" && isActivationEvent(ev)) activate(this, !!ev.bubbles);
    return proceed;
  },
  NODE_MEMBERS,
);

// -- links: activation behaviour (docs/SHELL.md §1; desktop-apis-design §5.1)
//
//    There is no document to navigate, so a link's activation hands its URL
//    to the system — the system browser is the only honest "browsing context"
//    a desktop app has. And because a link is reachable by CONTENT (a chat
//    message, a rendered Markdown file), the scheme allowlist is the security
//    line: http, https and mailto only. `file:`, custom protocols and
//    `ms-settings:` are refused with an advisory naming
//    `appShell.openExternal`, which is the call for a URL the app's own code
//    trusts. That closes `openExternal(untrusted)` → `file:///…/calc.exe` by
//    default.

/** What content may hand to the OS. Mirrors `shell::CONTENT_SCHEMES`. */
const CONTENT_SCHEMES = new Set(["http", "https", "mailto"]);
/** RFC 3986 `scheme ":"`, or null for a relative reference. */
const SCHEME_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*):/;

/** An `<a>` (or `<area>`) with an `href` — an element whose click is a navigation. */
function isLink(n: MirrorNode): n is MirrorElement {
  return n.kind === 1 && (n.tag === "a" || n.tag === "area") && n.attrs != null && n.attrs.href != null;
}

/** The elements HTML gives an activation behaviour. The FIRST one on the path wins. */
const ACTIVATABLE = new Set(["button", "input", "label", "summary"]);
const hasActivationBehaviour = (n: MirrorNode): n is MirrorElement =>
  n.kind === 1 && (ACTIVATABLE.has(n.tag) || isLink(n));

/**
 * DOM "dispatch" step 5.5: is this event an activation event? A `MouseEvent`
 * whose type is `click`. A `CustomEvent("click")` is not — and an engine
 * with no MouseEvent class at all cannot make that distinction, so there any
 * click counts (that is only ever the unit-test world).
 */
function isActivationEvent(ev: VnEvent): boolean {
  return typeof MouseEvent !== "function" || ev instanceof MouseEvent;
}

/**
 * The activation target: the event's target if it has an activation
 * behaviour, else — for a bubbling event — the nearest ancestor that does.
 * A `<button>` inside an `<a>` is therefore the button's click, not the
 * link's, exactly as the DOM spec words it.
 */
function activate(target: MirrorNode, bubbles: boolean): void {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (hasActivationBehaviour(n)) {
      if (isLink(n)) followLink(n);
      return;
    }
    if (!bubbles) return;
  }
}

function followLink(a: MirrorElement): void {
  const raw = String(a.attrs.href ?? "").trim();
  // `href=""` and `href="#…"` are the in-page idioms (a JS-handled link); a
  // browser would scroll, there is nothing to do here.
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
  const scheme = m[1].toLowerCase();
  if (scheme === "javascript") {
    // `javascript:void(0)` is the old "a link that only has a click handler"
    // idiom and means nothing either way; anything else asked for code to run.
    if (!/^javascript:\s*(void\s*\(?\s*0\s*\)?\s*;?\s*|;?)$/i.test(raw)) {
      warnCompat(
        "link:javascript",
        `<a href="javascript:…"> — javascript: URLs are not executed (CSP-style); put the code ` +
          `in a @click handler`,
      );
    }
    return;
  }
  if (a.attrs.download != null && (scheme === "blob" || scheme === "data")) {
    warnCompat(
      "link:download",
      `<a download href="${scheme}:…"> — saving a download is not supported yet: it needs the ` +
        `Save As dialog and the blob store (desktop-apis tracks B/C). Nothing was saved`,
    );
    return;
  }
  if (!CONTENT_SCHEMES.has(scheme)) {
    adviseScheme(scheme, "<a href>");
    return;
  }
  openContentUrl(raw, "<a href>");
}

/** A-F1: a content link with a scheme the allowlist does not hold. */
function adviseScheme(scheme: string, via: string): void {
  warnCompat(
    `link-scheme:${scheme}`,
    `${via} with a ${scheme}: URL was refused: links and window.open() only hand http, https ` +
      `and mailto to the system, because content can reach them. If the app trusts this URL, ` +
      `open it from your own code with appShell.openExternal(url) (docs/SHELL.md)`,
  );
}

// -- el.click(): HTML's synthetic click. A real `MouseEvent`, through the same
//    walk a host click takes, then the activation behaviour if nobody
//    cancelled it. `click()` on a disabled form control does nothing at all,
//    and a click() from inside its own click handler is ignored (HTML's
//    "click in progress" flag).
const DISABLEABLE = new Set(["button", "input", "select", "textarea", "fieldset", "optgroup", "option"]);
const clicking = new Set<MirrorElement>();
defineDom("click", function (this: MirrorElement) {
  if (DISABLEABLE.has(this.tag) && this.attrs.disabled != null && this.attrs.disabled !== "false") return;
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

// -- pointer capture. gpui captures the mouse implicitly for the duration of a
//    press, which is what this API is FOR — so the capture itself is already
//    the platform's behaviour and only the browser BOOKKEEPING has to exist,
//    consistently. reka-ui's Select opens on `pointerdown` and immediately
//    releases the capture it would otherwise hold; a missing
//    `hasPointerCapture` is a TypeError that kills the open outright.
defineDom("setPointerCapture", function (this: MirrorElement, pointerId: number) {
  (this._pointerCaptures || (this._pointerCaptures = new Set())).add(pointerId);
});
defineDom("hasPointerCapture", function (this: MirrorElement, pointerId: number) {
  return !!this._pointerCaptures?.has(pointerId);
});
defineDom("releasePointerCapture", function (this: MirrorElement, pointerId: number) {
  this._pointerCaptures?.delete(pointerId);
});

defineDom("insertAdjacentElement", function (_where: string, el: MirrorElement) {
  warnCompat(
    "insertAdjacentElement",
    "insertAdjacentElement() is a no-op — nodes injected outside the Vue renderer are not part of " +
      "the retained tree and are never painted",
  );
  return el;
});
defineDom("append", function () {});
defineDom("remove", function () {
  warnCompat(
    "remove",
    "element.remove() is a no-op — the retained tree is owned by the Vue renderer; use v-if/v-for",
  );
});
defineDom("cloneNode", function () {
  warnCompat(
    "cloneNode",
    "element.cloneNode() is not implemented — the retained tree has no copy operation; render a " +
      "second element instead",
  );
  return null;
});

// -- HTMLMediaElement (crates/vn-video/CONTRACT.md §6).
//
//    `<video>` is the one element whose interesting state lives in the HOST and
//    not in the mirror: `currentTime` moves inside the OS media pipeline sixty
//    times a second and no prop write puts it there. So the surface splits the
//    same way `getBoundingClientRect` and `getComputedStyle` already do — the
//    accessors below read `__vn_media_state` live, per call, uncached, exactly
//    as `rectOf` reads `__vn_bounds`. A browser's `el.currentTime` is a live
//    property too; caching it would be the divergence.
//
//    The verbs go the other way, through `__vn_media_call`. They are the same
//    seven the element layer implements (`vn_tree::video::media_call`), and
//    nothing in Rust changed to make this work: the host has dispatched all
//    eighteen events on the ordinary EV_* channel since the day `<video>`
//    landed, and this file simply learned their names.
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
/** A media element is an ELEMENT with a media tag — hence the type guard. */
const isMediaEl = (n: MirrorNode | null | undefined): n is MirrorElement =>
  !!n && MEDIA_TAGS.has((n as MirrorElement).tagName);
/** `name=value` lines -> object; `{}` for a node the host knows nothing about. */
function mediaState(node: MirrorNode | null | undefined): Record<string, string> | null {
  if (!isMediaEl(node) || !node.id) return null;
  const raw = __vn_media_state(node.id);
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
  isMediaEl(node) && node.id ? __vn_media_call(node.id, verb, arg) : false;
/** `"1.5:3,8:9"` -> the TimeRanges shape everything in the wild indexes. */
function timeRanges(raw: string | undefined): MirrorTimeRanges {
  const list = raw
    ? raw.split(",").filter(Boolean).map((pair) => {
        const [start, end] = pair.split(":").map(Number);
        return { start, end };
      })
    : [];
  return {
    length: list.length,
    start: (i: number) => list[i]?.start ?? NaN,
    end: (i: number) => list[i]?.end ?? NaN,
    // Not in the DOM, and deliberately: a TimeRanges is painful to read in a
    // template and every transport component ends up writing this loop itself.
    ranges: list,
  };
}
/** A read-only media property, off the one state string. */
function defineMediaGet(
  name: string,
  read: (s: Record<string, string>) => unknown,
  write?: (value: never) => void,
): void {
  defineDomGet(
    name,
    function (this: MirrorElement) {
      const s = mediaState(this);
      // A non-media element has no such property in a browser either.
      return s ? read(s) : isMediaEl(this) ? read({}) : undefined;
    },
    write,
  );
}
const num = (s: Record<string, string>, k: string, fallback: number): number =>
  s[k] === undefined ? fallback : Number(s[k]);
const bool = (s: Record<string, string>, k: string, fallback: boolean): boolean =>
  s[k] === undefined ? fallback : s[k] === "true";

defineMediaGet("currentTime", (s) => num(s, "currentTime", 0), function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.currentTime, Number(v));
});
defineMediaGet("duration", (s) => num(s, "duration", NaN));
defineMediaGet("paused", (s) => bool(s, "paused", true));
defineMediaGet("ended", (s) => bool(s, "ended", false));
defineMediaGet("seeking", (s) => bool(s, "seeking", false));
defineMediaGet("readyState", (s) => num(s, "readyState", 0));
defineMediaGet("networkState", (s) => num(s, "networkState", 0));
defineMediaGet("volume", (s) => num(s, "volume", 1), function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.volume, Number(v));
});
defineMediaGet("muted", (s) => bool(s, "muted", false), function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.muted, v ? 1 : 0);
});
defineMediaGet("playbackRate", (s) => num(s, "playbackRate", 1), function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.playbackRate, Number(v));
});
defineMediaGet("loop", (s) => bool(s, "loop", false), function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.loop, v ? 1 : 0);
});
defineMediaGet("videoWidth", (s) => num(s, "videoWidth", 0));
defineMediaGet("videoHeight", (s) => num(s, "videoHeight", 0));
defineMediaGet("buffered", (s) => timeRanges(s.buffered));
defineMediaGet("seekable", (s) => timeRanges(s.seekable));
// `MediaError`: `null` when there is none, which is the check every handler
// makes (`if (el.error)`). `message` carries the advisory naming this machine's
// missing decoder and the ffmpeg one-liner that fixes the file.
defineMediaGet("error", (s) => {
  const code = num(s, "error", 0);
  return code ? { code, message: s.errorMessage ?? "" } : null;
});

defineDom("play", function (this: MirrorElement) {
  mediaCall(this, MEDIA_CALL.play);
  // HTML returns a promise that resolves when playback actually begins. The
  // host's `play` is synchronous and the `playing` event is what says it
  // started, so resolving immediately is the honest approximation — and it is
  // what makes `el.play().catch(…)` (the autoplay-rejection idiom) not throw.
  return Promise.resolve();
});
defineDom("pause", function (this: MirrorElement) {
  mediaCall(this, MEDIA_CALL.pause);
});
defineDom("load", function (this: MirrorElement) {
  // The host runs the load algorithm off the `src` prop, edge-triggered, so a
  // re-load is `src = ""` then `src = url`. There is no separate verb for it.
  if (isMediaEl(this)) {
    warnCompat(
      "media-load",
      "video.load() is a no-op — the host loads from the `src` prop, edge-triggered; " +
        "to force a reload, clear `src` and set it again",
    );
  }
});
defineDom("canPlayType", function (this: MirrorElement, type: unknown) {
  if (!isMediaEl(this) || !this.id) return "";
  // A live probe of THIS machine's decoder registry, not a table — a Store
  // codec package installed after we shipped is picked up on the next launch.
  return __vn_media_can_play(this.id, String(type));
});

// ---- W3C event synthesis + propagation ------------------------------------
// The host sends ONE compact payload per input event targeted at the deepest
// hit node; the mirror tree provides the ancestor path. Events are real redjs
// native classes when available (KeyboardEvent & co), with a CustomEvent
// fallback so nothing hard-depends on the engine revision.
const mods = (d: number) => ({
  ctrlKey: !!(d & 1),
  shiftKey: !!(d & 2),
  altKey: !!(d & 4),
  metaKey: !!(d & 8),
});

// The one synthetic pointer: gpui reports a single OS cursor, so there is
// exactly one primary mouse pointer. (A pen or a touch point would have to be
// reported by the platform layer, which does not.)
const POINTER_INIT = {
  pointerId: 1,
  pointerType: "mouse",
  isPrimary: true,
  width: 1,
  height: 1,
};

function synthesize(
  name: string,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
): VnEvent {
  const common = { bubbles: !NO_BUBBLE.has(name), cancelable: !NO_BUBBLE.has(name) };
  const M = mods(d);
  try {
    if (POINTER_EVENTS.has(name)) {
      const init = {
        ...common,
        ...M,
        ...POINTER_INIT,
        clientX: a,
        clientY: b,
        button: c,
        buttons: c >= 0 ? 1 << c : 0,
        pressure: c >= 0 ? 0.5 : 0,
      };
      if (typeof PointerEvent === "function") return new PointerEvent(name, init);
      // No PointerEvent class in this engine: a MouseEvent carrying the pointer
      // fields reads identically to everything that consumes one.
      return withProps(new MouseEvent(name, init), { ...POINTER_INIT, pressure: init.pressure });
    }
    switch (name) {
      case "click":
      case "mousedown":
      case "mouseup":
      case "mousemove":
      case "mouseenter":
      case "mouseleave":
        return new MouseEvent(name, { ...common, ...M, clientX: a, clientY: b, button: c });
      case "focusin":
      case "focusout":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "wheel":
        if (typeof WheelEvent === "function")
          return new WheelEvent(name, { ...common, ...M, deltaX: a, deltaY: b });
        break;
      case "keydown":
      case "keyup":
        if (typeof KeyboardEvent === "function")
          return new KeyboardEvent(name, { ...common, ...M, key: key ?? "" });
        break;
      case "focus":
      case "blur":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "input":
        if (typeof InputEvent === "function")
          return new InputEvent(name, { ...common, data: key ?? null });
        break;
      case "scroll":
        // A plain Event in every browser; the position is read off e.target.
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
        // <lottie-player> dispatches CustomEvents; lottie-web hands its own
        // listeners a plain object with these fields. Both shapes are served:
        // the numbers sit on `detail` AND directly on the event.
        return withProps(new CustomEvent(name, { ...common, detail: lottieDetail(name, a, b, c, d) }),
                         lottieDetail(name, a, b, c, d));
    }
    if (MEDIA_EVENTS.has(name)) {
      // In a browser these are plain `Event`s and everything interesting is
      // read off `e.target` — which works here too, because the accessors
      // below answer from the host. The numbers ride along anyway (on the
      // event AND on `detail`) so a handler can read `e.currentTime` without a
      // second host call per frame, which is what makes a 4 Hz `@timeupdate`
      // readout cost nothing.
      const detail = mediaDetail(a, b, c, d);
      return withProps(new CustomEvent(name, { ...common, detail }), detail);
    }
  } catch (e) {
    /* fall through to the generic event */
  }
  const ev = new CustomEvent(name, common);
  const extra =
    name === "wheel"
      ? { deltaX: a, deltaY: b, ...M }
      : name === "keydown" || name === "keyup"
        ? { key: key ?? "", ...M }
        : name === "input"
          ? { data: key ?? null }
          : name === "transitionend"
            ? { propertyName: key ?? "", elapsedTime: a, pseudoElement: "" }
          : name === "animationstart" || name === "animationend" || name === "animationcancel"
            ? { animationName: key ?? "", elapsedTime: a, pseudoElement: "" }
          : name === "focus" || name === "blur" || name === "focusin" || name === "focusout" || name === "scroll"
            ? {}
            : name === "complete" || name === "loopComplete" || name === "enterFrame"
              ? lottieDetail(name, a, b, c, d)
              : MEDIA_EVENTS.has(name)
              ? mediaDetail(a, b, c, d)
              : POINTER_EVENTS.has(name)
                ? { clientX: a, clientY: b, button: c, ...POINTER_INIT, ...M }
                : { clientX: a, clientY: b, button: c, ...M };
  for (const k in extra) Object.defineProperty(ev, k, { value: (extra as ExtraProps)[k], configurable: true });
  return ev;
}

/** The per-event-family extra fields `synthesize` stamps onto a fallback event. */
type ExtraProps = Record<string, unknown>;

function withProps(ev: VnEvent, props: Record<string, unknown>): VnEvent {
  for (const k in props) Object.defineProperty(ev, k, { value: props[k], configurable: true });
  return ev;
}

// The payload crates/vn-video/src/state.rs sends with every media event
// (CONTRACT.md §4): a = currentTime, b = duration, c = readyState,
// d = playbackRate. For `error`, a is MediaError.code and b is currentTime —
// `e.target.error` is the shape a browser handler reads, and that one is served
// live off `__vn_media_state` by the accessor below rather than frozen here.
function mediaDetail(a: number, b: number, c: number, d: number) {
  return { currentTime: a, duration: b, readyState: c, playbackRate: d };
}

// The payload crates/vn-tree/src/lottie.rs sends, named the way lottie-web
// names it (`currentTime` / `totalTime` are frame counts there too).
function lottieDetail(name: string, a: number, b: number, c: number, d: number) {
  if (name === "enterFrame") return { currentTime: a, totalTime: b, elapsed: c, progress: d };
  if (name === "loopComplete") return { currentTime: a, totalTime: b, currentLoop: c };
  return { currentTime: a, totalTime: b, loops: c };
}

/** Which half of the DOM's two-phase dispatch a listener was registered for. */
type Phase = "capture" | "bubble";
/** `stopPropagation` / `stopImmediatePropagation`, as the walk sees them. */
interface PropagationState {
  stopped: boolean;
  immediate: boolean;
}

/** DOM `Event.eventPhase` (NONE, CAPTURING_PHASE, AT_TARGET, BUBBLING_PHASE). */
const EVENT_PHASE = { none: 0, capturing: 1, atTarget: 2, bubbling: 3 } as const;

/**
 * One listener call. A listener that throws does not end the dispatch: DOM
 * §2.10 "inner invoke" REPORTS the exception and goes on to the next listener,
 * and `dispatchEvent` itself never throws a listener's error. Until m25 the
 * throw escaped the walk, so every listener after it — on that node and on
 * every ancestor — silently never ran.
 *
 * Reported from a microtask: it reaches the same uncaught-error sink a throw
 * out of a host callback always did (`[js uncaught]` in the log and the
 * devtools console), after the walk instead of in place of the rest of it.
 */
function callListener(fn: (this: unknown, ev: VnEvent) => unknown, self: unknown, ev: VnEvent): void {
  try {
    fn.call(self, ev);
  } catch (e) {
    (globalThis.queueMicrotask ?? queueMicrotask)(() => {
      throw e;
    });
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
  // The walk is ours, not the engine's, so the engine's own `currentTarget` /
  // `eventPhase` getters would answer "no dispatch" — own properties say what
  // a browser's would at this step.
  Object.defineProperty(ev, "currentTarget", { value: node, configurable: true });
  Object.defineProperty(ev, "eventPhase", { value: eventPhase, configurable: true });
  const fn = phase === "capture" ? buckets.capture : buckets.bubble;
  if (fn) {
    if (buckets[phase === "capture" ? "onceC" : "onceB"]) {
      if (phase === "capture") buckets.capture = null;
      else buckets.bubble = null;
    }
    if (Array.isArray(fn)) {
      for (const f of fn) {
        if (state.immediate) break;
        callListener(f, undefined, ev);
      }
    } else callListener(fn, undefined, ev);
  }
  const extra = phase === "capture" ? buckets.extraC : buckets.extraB;
  // snapshot: spec semantics — listeners added during dispatch don't fire now
  for (const entry of [...extra]) {
    if (state.immediate) break;
    if (entry.once) {
      const i = extra.indexOf(entry);
      if (i >= 0) extra.splice(i, 1);
    }
    callListener(entry.fn, node, ev);
  }
  if (isMirrorNode(node)) syncListenerFlag(node.id, name);
  else if (appRoot) syncListenerFlag(appRoot.id, name);
}

function propagate(target: MirrorNode, name: string, ev: VnEvent): boolean {
  const path: PropagationTarget[] = [];
  for (let n: MirrorNode | null = target; n; n = n.parentNode) path.push(n);
  // …and above the root, the document and the window — where a browser's event
  // path puts them, and where every dismissable-layer implementation listens.
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
    const buckets = isMirrorNode(node)
      ? handlers.get(node.id)?.[name]
      : globalHandlers.get(node)?.[name];
    const eventPhase =
      node === target ? EVENT_PHASE.atTarget : phase === "capture" ? EVENT_PHASE.capturing : EVENT_PHASE.bubbling;
    if (buckets) invoke(buckets, phase, eventPhase, ev, node, name, state);
  };
  // `composedPath()` is the path for as long as the dispatch runs — target
  // first, window last — and empty once it is over (DOM §2.9).
  const composed = path.slice();
  Object.defineProperty(ev, "composedPath", { value: () => composed.slice(), configurable: true });
  try {
    // capture: root -> target (exclusive)
    for (let i = path.length - 1; i > 0 && !state.stopped; i--) at(path[i], "capture");
    // at target: capture listeners, then bubble listeners
    if (!state.stopped) at(target, "capture");
    if (!state.stopped) at(target, "bubble");
    // bubble: target.parent -> root
    if (ev.bubbles) for (let i = 1; i < path.length && !state.stopped; i++) at(path[i], "bubble");
  } finally {
    // The dispatch is over: no current target, no phase, no path — what a
    // handler that kept the event and reads it later sees in a browser.
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
 * Is there anything on this event's path that would be CALLED for `name`?
 *
 * The host arms ONE BIT PER CODE, and several event names share a code — a
 * `pointermove` listener arms `mousemove`, and a `mousedown` listener arms the
 * bit `pointerdown` rides on. So the host delivering an event does not mean
 * anyone is listening for the particular NAME about to be synthesized: the
 * dispatcher below used to build, and walk, one full W3C event per derived name
 * on every delivery, whether or not a single handler existed for it.
 *
 * That is worth avoiding rather than tidying: `scripts/boundary-cost.ts`
 * measures redjs's `new MouseEvent(...)` at roughly two thirds of the entire
 * cost of dispatching an event — far more than the propagation walk it feeds —
 * and it is engine code, so no amount of work on our side makes one cheaper.
 * The only saving available is not building it.
 *
 * The walk is the same ancestor chain `propagate` would take, minus the
 * allocation: a handful of Map lookups against a 16 µs constructor. It is
 * deliberately CONSERVATIVE — it ignores `bubbles`, so a listener above a
 * non-bubbling event's target still counts as a reason to synthesize. Being
 * wrong in that direction costs an event object; being wrong in the other
 * direction would drop a handler.
 */
function hasListener(target: MirrorNode, name: string): boolean {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    const buckets = handlers.get(n.id)?.[name];
    if (buckets && bucketsAlive(buckets)) return true;
  }
  // …and the document/window registry, which sits one level above the root.
  for (const m of globalHandlers.values()) {
    const buckets = m[name];
    if (buckets && bucketsAlive(buckets)) return true;
  }
  return false;
}

/** Is there an `<a href>` on this node's ancestor chain (itself included)? */
function linkOnPath(target: MirrorNode): boolean {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) if (isLink(n)) return true;
  return false;
}

__vn_set_dispatcher((nodeId, evCode, a, b, c, d, key) => {
  const name = CODE_TO_NAME[evCode];
  const target = nodesById.get(nodeId);
  if (!name || !target) return true;
  // Sticky activation (docs/WINDOWING.md §1.9). The windowing prelude arms it
  // from `keydown`/`mousedown` on the engine's own EventTarget, which gpui
  // events never reach — the host delivers them HERE. The prelude publishes
  // this hook for exactly that reason; without the call, `beforeunload` could
  // never veto a close because the window would never count as interacted with.
  if (name === "keydown" || name === "mousedown") globalThis.__vnStickyActivation?.();
  // input: the payload string is the new value — mirror it onto the node
  // BEFORE dispatch so `e.target.value` reads like the browser.
  // (`value` is an element field; the host only ever targets an element with
  // this event, which is what the cast records — a text node reaching here
  // would be a host bug, not a JS one.)
  //
  // `change` is listed with it deliberately, and is currently UNREACHABLE:
  // `CODE_TO_NAME` is built from `EV_CODES`, which mirrors `tree.rs`, and
  // `tree.rs` has no `EV_CHANGE`. It stays because the day the host sources one
  // this is the line that has to already be right — and `test/protocol-codes
  // .test.ts` is what will notice the code arriving. See `vModelText`, which is
  // the directive that wanted it.
  if ((name === "input" || name === "change") && key != null)
    (target as MirrorElement).value = key;
  // `scroll` used to write the position onto the node here, because that was
  // the only way `e.target.scrollTop` could answer anything. It is now an
  // accessor over the host's live `ScrollHandle`, so writing it back would
  // push the position the event describes into the box the event came from —
  // a round trip with one possible outcome and one possible bug.
  // The derived family (`pointerdown` from `mousedown`, `focusin` from `focus`,
  // …): each one is a full W3C event of its own with its own propagation, in
  // the order a browser fires them. `preventDefault()` on a derived pointer
  // event cancels the source, exactly as it does in a browser.
  const derived = DERIVED_OF[name];
  let proceed = true;
  if (derived) {
    for (const dn of derived) {
      if (!DERIVED[dn as DerivedEventName].pre) continue;
      if (!hasListener(target, dn)) continue;
      if (!propagate(target, dn, synthesize(dn, a, b, c, d, key))) proceed = false;
    }
  }
  // Skipping the source event when nothing listens for it is exactly as safe:
  // with no handler to run, `propagate` can only return `!ev.defaultPrevented`,
  // and the sole thing that could have set that flag is the `preventDefault()`
  // two lines below — whose whole job is to make `proceed` false, which it
  // already is.
  // A click whose path holds a link must be BUILT even with no handler on it:
  // a `preventDefault()` anywhere on the path is what cancels the navigation,
  // and the walk is the only way to learn that.
  const link = name === "click" && linkOnPath(target);
  if (link || hasListener(target, name)) {
    const ev = synthesize(name, a, b, c, d, key);
    if (!proceed && ev.preventDefault) ev.preventDefault();
    proceed = propagate(target, name, ev) && proceed;
    if (link && proceed) activate(target, true);
  }
  if (derived) {
    for (const dn of derived) {
      if (DERIVED[dn as DerivedEventName].pre) continue;
      if (!hasListener(target, dn)) continue;
      propagate(target, dn, synthesize(dn, a, b, c, d, key));
    }
  }
  return proceed;
});

const renderer = createRenderer({ ...nodeOps, patchProp });

/**
 * What `createApp` hands back: Vue's own `App`, with ONE signature different.
 *
 * `mount()` takes NO container here — the window is the root — and Vue's
 * `App["mount"]` requires one, so `createApp(App).mount()` did not type-check
 * against the upstream type and every app in this repository had to cast around
 * it. A container argument is still ACCEPTED, for browser parity, and ignored
 * with an advisory; the optional parameter is what records that.
 *
 * It EXTENDS `App` rather than `Omit`-ing `mount` out of it, and that is not a
 * style choice: `use`, `component`, `directive` and `provide` all return
 * `this`, and a mapped type resolves `this` to the type it was mapped FROM — so
 * an `Omit` version type-checks `createApp(App).mount()` and then fails on
 * `createApp(App).use(router).mount()`, which is what every real entry module
 * writes. Extending keeps `this` polymorphic, so the whole chain stays one
 * expression.
 */
export interface VnApp extends App<MirrorElement> {
  mount(container?: unknown): ComponentPublicInstance;
}

/// App entry point. No container argument — the window IS the root.
export function createApp(
  rootComponent: unknown,
  rootProps: Record<string, unknown> | null = null,
): VnApp {
  const app = renderer.createApp(rootComponent as never, rootProps);
  const mount = app.mount.bind(app);
  app.mount = (container?: unknown) => {
    if (container != null)
      warnCompat(
        "mount:container",
        `app.mount(${JSON.stringify(container)}) — container arguments are accepted for ` +
          `browser parity but ignored: the window is the root in vue-native`,
      );
    // SFC styles collected at module-eval time (see build.ts) become the
    // stylesheet, installed before first mount so the first paint is styled.
    if (globalThis.__VN_CSS__?.length) {
      __vn_set_stylesheet(__VN_CSS__!.join("\n"));
    }
    // `HTML`, not `ROOT`. The retained tree has one root box and it answers to
    // `document.documentElement` AND `document.body` (see the block that
    // installs them), so whatever `nodeName` it reports is what every
    // DOM-walking library sees at the top of the chain — and the walk has to
    // TERMINATE there.
    //
    // @floating-ui/dom, which every reka-ui popper rides on, walks with
    //
    //     getParentNode(n) = n.assignedSlot || n.parentNode || getDocumentElement(n)
    //     isLastTraversableNode(n) = ['html','body','#document'].includes(nodeName)
    //
    // and `getNearestOverflowAncestor` recurses on `getParentNode` until
    // `isLastTraversableNode`. Our root has no `parentNode`, so `getParentNode`
    // falls through to `getDocumentElement`, which IS the root — a self-loop.
    // Called "ROOT" the loop never terminates, and `getOverflowAncestors(floating)`
    // (the FIRST thing `autoUpdate` does, before it ever calls `update()`)
    // blew the JS stack. `computePosition` then never ran: reka's PopperContent
    // was left at its unplaced `translate(0, -200%)` with an inline
    // `animation: none`, so every tooltip/popover/dropdown painted two of its
    // own heights above the window origin, never animated, and — because the
    // throw also ate the queued `useGraceArea` effect that installs the
    // pointer-exit watcher — never closed. Naming the root `html` is the whole
    // fix: `isLastTraversableNode` is true one step earlier, and floating-ui's
    // own `if (getNodeName(node) === 'html') return node` guard exists for
    // exactly this shape. See research/reports/m19/hover-anim-tooltips.md.
    //
    // The Rust tag stays "root" — `vn_css::element::has_local_name` already
    // answers `html` and `body` off `is_root()`, so the cascade is unaffected
    // and devtools still names the box for what it is.
    const root = mk(__vn_create_root(), 1, "html");
    // The document roots a reused browser component reaches for (`<Teleport
    // to="body">`, `document.querySelectorAll`, `document.body.contains`) all
    // mean this node.
    appRoot = root;
    // Anything module-scope theme code wrote to `document.documentElement`
    // before there was a root — the `dark` class, the `--radius` overrides —
    // lands now, BEFORE the first render, so the first paint is themed.
    if (flushDocumentElement) flushDocumentElement(root);
    const vm = mount(root);
    // Dev hot-restart: the host calls this before re-evaling a new bundle.
    globalThis.__VN_TEARDOWN__ = () => {
      try {
        app.unmount();
      } catch (e) {
        __vn_log("teardown: " + e);
      }
      __vn_remove(root.id);
      // Detached-but-live nodes: <KeepAlive> creates its cache container with
      // createElement and NEVER inserts or removes it (Vue moves subtrees in
      // and out of it), so nothing above ever tells us to free it. After
      // app.unmount() every node Vue owns is gone from the mirror, so whatever
      // is left is exactly that kind of orphan — free it or the host arena
      // grows by one node per KeepAlive on every hot restart.
      for (const id of Array.from(nodesById.keys())) __vn_remove(id);
      nodesById.clear();
      handlers.clear();
      globalThis.__VN_CSS__ = [];
      globalThis.__VN_TEARDOWN__ = undefined;
      appRoot = null;
      observers.clear();
      globalHandlers.clear();
    };
    __vn_log(`Vue ${version} app mounted (vue-native runtime)`);
    return vm;
  };
  return app as VnApp;
}

// ---- missing globals (compat roadmap G5) ----------------------------------
// Each of these is a ReferenceError today, i.e. strictly worse than a wrong
// answer: reka-ui's FocusScope constructs a MutationObserver inside a
// SETUP-TIME watchEffect, so every modal (Dialog, Popover, Menu, Select) dies
// before it renders. They are installed on the global unconditionally when the
// engine does not already have them.

const defineGlobal = (name: string, value: unknown) => {
  if (typeof (globalThis as Record<string, unknown>)[name] === "undefined") {
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  }
};

// -- the `CSS` namespace object.
//
// reka-ui's `Presence` matches the `animationend` it just received against the
// element's `animationName` with `CSS.escape(...)`. `CSS is not defined` there
// is a ReferenceError *inside an event handler*: the exit animation never
// completes and the element never unmounts. It only became reachable when
// `animationend` started firing at all (m12 item 3).
//
// `escape` is CSSOM §4.2's algorithm implemented rather than approximated — it
// feeds a string comparison, and an under-escaped identifier silently stops
// matching.
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
  // A feature PROBE. Answering `false` for everything would be a lie in the
  // other direction (it is how `@supports`-guarded fallbacks get chosen), so
  // this says what vue-native can express: the properties the cascade maps.
  supports(a: unknown, b?: unknown) {
    const body = b === undefined ? String(a) : `${String(a)}:${String(b)}`;
    const name = (body.split(":")[0] || "").trim().toLowerCase();
    if (!name) return false;
    if (name.startsWith("--")) return true;
    return CSS_SUPPORTED.has(name);
  },
  registerProperty() {
    warnCompat(
      "CSS.registerProperty",
      "CSS.registerProperty() is a no-op — vue-native's custom properties are untyped strings, " +
        "so a registered syntax/inherits/initialValue has nothing to enforce",
    );
  },
});

/// The property names `CSS.supports` answers `true` for: the ones vn-css maps
/// onto a real computed value. Deliberately conservative — a probe that says
/// yes and then does nothing is worse than one that says no.
const CSS_SUPPORTED = new Set<string>([
  "animation", "animation-delay", "animation-direction", "animation-duration",
  "animation-fill-mode", "animation-iteration-count", "animation-name",
  "animation-play-state", "animation-timing-function",
  "transition", "transition-delay", "transition-duration", "transition-property",
  "transition-timing-function",
  "align-items", "align-self", "background", "background-color", "border", "border-color",
  "border-radius", "border-width", "bottom", "box-shadow", "color", "column-gap", "display",
  "flex", "flex-basis", "flex-direction", "flex-grow", "flex-shrink", "flex-wrap", "font-size",
  "font-style", "font-weight", "gap", "height", "inset", "justify-content", "left",
  "line-height", "margin", "margin-bottom", "margin-left", "margin-right", "margin-top",
  "max-height", "max-width", "min-height", "min-width", "opacity", "overflow", "overflow-x",
  "overflow-y", "padding", "padding-bottom", "padding-left", "padding-right", "padding-top",
  "pointer-events", "position", "right", "row-gap", "text-align", "top", "transform",
  "visibility", "white-space", "width", "z-index",
]);

// -- MutationObserver, wired to the REAL mirror-tree mutations.
// Records are queued at the nodeOps/patchProp call sites and delivered on a
// microtask, which is exactly a browser's timing (the observer callback runs at
// the end of the current task, before rendering).
/** One `MutationRecord`, with exactly the fields this runtime can fill in. */
interface MutationRecordLike {
  type: "childList" | "attributes";
  target: MirrorNode;
  addedNodes: MirrorNode[];
  removedNodes: MirrorNode[];
  attributeName: string | null;
  attributeNamespace: null;
  oldValue: string | null;
  previousSibling: null;
  nextSibling: null;
}
/** What `observe()` recorded for one target. */
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
/** `MutationObserver`'s `observe()` options, as far as they are honoured. */
interface MutationObserverInitLike {
  subtree?: boolean;
  childList?: boolean;
  attributes?: boolean;
  attributeOldValue?: boolean;
  attributeFilter?: string[];
  characterData?: boolean;
}
/** The instance shape `notifyMutation` and `flushMutations` drive. */
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
      ob._records.push({
        type,
        target,
        addedNodes: extra?.addedNodes ?? [],
        removedNodes: extra?.removedNodes ?? [],
        attributeName: extra?.attributeName ?? null,
        attributeNamespace: null,
        oldValue: entry.options.attributeOldValue ? (extra?.oldValue ?? null) : null,
        previousSibling: null,
        nextSibling: null,
      });
      break; // one record per observer per mutation, as in the DOM
    }
  }
  if (!mutationFlushQueued) {
    mutationFlushQueued = true;
    Promise.resolve().then(flushMutations);
  }
}

function isAncestor(ancestor: MirrorNode, node: MirrorNode | null | undefined): boolean {
  for (let n: MirrorElement | null | undefined = node?.parentNode; n; n = n.parentNode)
    if (n === ancestor) return true;
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
      // `observe(document, {subtree: true})` is how a library says "watch
      // everything" — reka's focus scope and floating-ui's auto-update both do
      // it. A Document is not an element here, but its element subtree is
      // exactly the root's, so watching the root answers the same question.
      // `document.documentElement` / `document.body` ARE real elements since
      // m19, and arrive as themselves.
      // Duck-typed rather than compared against the `document` global, because
      // the same code runs in the test harness's private world.
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

// -- ResizeObserver, sampled from the host's real layout.
//
// gpui recomputes layout every frame and publishes no per-element resize
// SIGNAL, but it does publish the resulting rectangle (`__vn_bounds`) — so the
// observer polls its targets once per animation frame and reports the frames on
// which a size actually changed. That is a frame of latency against a browser's
// notification, and it costs one host call per observed element per frame while
// anything is observed (nothing is, until something floats).
//
// It has to be real: @floating-ui's `autoUpdate` positions an element ONCE and
// then waits for a ResizeObserver to tell it the element has been laid out. An
// inert observer means every popover, tooltip, dropdown and select keeps the
// position computed from its pre-layout 0×0 box — which is to say none of them
// is ever placed at all.
/** One `ResizeObserverEntry`, with the box fields every consumer reads. */
interface ResizeEntryLike {
  target: MirrorElement;
  contentRect: MirrorRect;
  borderBoxSize: { inlineSize: number; blockSize: number }[];
  contentBoxSize: { inlineSize: number; blockSize: number }[];
  devicePixelContentBoxSize: { inlineSize: number; blockSize: number }[];
}
/** The instance shape the poll below drives. */
interface ResizeObserverLike {
  _callback: (entries: ResizeEntryLike[], ob: ResizeObserverLike) => void;
  /** node -> the size last REPORTED for it; `null` = nothing reported yet. */
  _targets: Map<MirrorElement, { w: number; h: number } | null>;
}

const resizeObservers = new Set<ResizeObserverLike>();
let resizePollQueued = false;

function scheduleResizePoll() {
  if (resizePollQueued) return;
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf !== "function") return;
  resizePollQueued = true;
  raf(pollResizeObservers);
}

function pollResizeObservers() {
  resizePollQueued = false;
  let observing = false;
  for (const ob of [...resizeObservers]) {
    if (!ob._targets.size) continue;
    observing = true;
    const entries: ResizeEntryLike[] = [];
    for (const [node, last] of ob._targets) {
      const r = rectOf(node);
      if (last && last.w === r.width && last.h === r.height) continue;
      ob._targets.set(node, { w: r.width, h: r.height });
      const box = [{ inlineSize: r.width, blockSize: r.height }];
      entries.push({ target: node, contentRect: r, borderBoxSize: box, contentBoxSize: box, devicePixelContentBoxSize: box });
    }
    if (entries.length) {
      try {
        ob._callback(entries, ob);
      } catch (e) {
        console.error(`[vue-native] ResizeObserver callback threw: ${e}`);
      }
    }
  }
  if (observing) scheduleResizePoll();
}

defineGlobal(
  "ResizeObserver",
  class ResizeObserver implements ResizeObserverLike {
    declare _callback: ResizeObserverLike["_callback"];
    declare _targets: ResizeObserverLike["_targets"];
    constructor(callback: ResizeObserverLike["_callback"]) {
      this._callback = callback;
      this._targets = new Map(); // node -> last reported {w, h}
    }
    observe(node: MirrorElement) {
      if (!node || typeof node.id !== "number") return;
      // `null` = "no size reported yet", so the first poll always fires —
      // which is what a browser does on `observe()` too.
      this._targets.set(node, null);
      resizeObservers.add(this);
      scheduleResizePoll();
    }
    unobserve(node: MirrorElement) {
      this._targets.delete(node);
    }
    disconnect() {
      this._targets.clear();
      resizeObservers.delete(this);
    }
  },
);

// -- DocumentFragment. Vue's Teleport accepts one as a target, and reka-ui's
//    Select CONSTRUCTS one even while closed — a missing class kills it at
//    setup. Ours is a detached mirror element, so a Teleport into it renders
//    into the retained tree without being painted, which is what a browser's
//    off-document fragment does too.
defineGlobal(
  "DocumentFragment",
  class DocumentFragment {
    constructor() {
      const node = mk(__vn_create_element(I("fragment")), 1, "fragment");
      Object.defineProperty(node, "nodeType", { value: 11, configurable: true });
      // A constructor that hands back a DIFFERENT object — `new
      // DocumentFragment()` really is a detached mirror element, which is the
      // whole point. The double cast is what says so.
      return node as unknown as DocumentFragment;
    }
  },
);

// -- NodeFilter. `document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, …)`
//    is what reka's FocusScope walks with; the missing GLOBAL is what throws
//    first, before the walker is even built.
defineGlobal("NodeFilter", {
  SHOW_ALL: 0xffffffff,
  SHOW_ELEMENT: 1,
  SHOW_TEXT: 4,
  SHOW_COMMENT: 128,
  FILTER_ACCEPT: 1,
  FILTER_REJECT: 2,
  FILTER_SKIP: 3,
});

// -- Tag-specific element classes. redjs ships Element/HTMLElement/SVGElement
//    only, and reka-ui brands nodes with `instanceof HTMLInputElement`
//    (FocusScope's isSelectableInput). A MISSING class is a ReferenceError,
//    which is strictly worse than a false `instanceof` — so each one exists and
//    answers for the tag it names.
const TAG_CLASSES = {
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
};
for (const [name, tag] of Object.entries(TAG_CLASSES)) {
  if (typeof (globalThis as Record<string, unknown>)[name] !== "undefined") continue;
  // Mirror nodes are all plain HTMLElement, so `instanceof` is answered by the
  // tag name instead of by the prototype chain — which is the ONLY way to make
  // `el instanceof HTMLInputElement` true for a real `<input>` here.
  const klass = class {};
  Object.defineProperty(klass, "name", { value: name });
  Object.defineProperty(klass, Symbol.hasInstance, {
    value: (x: unknown) => !!x && typeof x === "object" && (x as MirrorElement).tagName === tag,
  });
  defineGlobal(name, klass);
}

// -- getComputedStyle. The values come from the REAL cascade (see
//    `crates/vn-tree/src/dom.rs`): the properties reused components actually
//    read at runtime, the USED width/height off the painted rectangle, and
//    every custom property. A property nothing declared reads `""`, exactly as
//    an unsupported one would in a browser — with one advisory naming it.
const computedCache = new WeakMap<object, unknown>();
function computedStyleFor(node: unknown) {
  // `document.body` / `documentElement` are not nodes in the retained tree;
  // they stand for the app root, so that is whose style they report.
  const id = typeof (node as MirrorNode)?.id === "number" ? (node as MirrorNode).id : appRoot?.id;
  const raw = id ? __vn_computed_style(id) : "";
  /**
   * One property, scanned out of the answer — NOT a parsed map.
   *
   * This function runs once per PROPERTY READ. The host call above has to: it
   * is `dom::computed_style`, which flushes pending style before answering, so
   * a cached answer would be the pre-mutation one — the exact bug reka-ui's
   * `<Presence>` hit, and the reason that flush exists. Splitting the ~20
   * `name=value` lines into an object to answer ONE of them does not have to:
   * `scripts/boundary-cost.ts` measured that parse at roughly three quarters of
   * what a property read cost, repeated for every property a component looked
   * at. A `<Transition>` reads two, @floating-ui reads six.
   *
   * The `\n` in front of the key is what stops a SUFFIX from matching — a
   * lookup of `width` must not find the `max-width=` line.
   */
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
    // Framework probes, not CSS: Vue's reactivity asks every object it touches
    // for `__v_isReadonly`/`__v_raw`, promise-detection asks for `then`. None
    // of those are properties a stylesheet can declare, so they answer
    // `undefined` (not `""`) and never warn.
    if (!/^[a-zA-Z-]/.test(key) || key.startsWith("__") || key === "then") return undefined;
    const css = key.startsWith("--") ? key : hyphenate(key);
    const value = lookup(css);
    if (value !== undefined) return value;
    warnCompat(
      `getComputedStyle:${css}`,
      `getComputedStyle().${key} reads "" — vue-native's computed style publishes the properties ` +
        `a reused component actually reads (transition-*, transform, display, position, ` +
        `overflow-*, width/height, direction) plus every custom property, not all 340 longhands`,
    );
    return "";
  };
  return new Proxy(
    { getPropertyValue: read, getPropertyPriority: () => "", get length() { return 0; } },
    {
      get: (t, k) => (k in t ? t[k as keyof typeof t] : typeof k === "string" ? read(k) : undefined),
      has: () => true,
    },
  );
}
defineGlobal("getComputedStyle", (node: unknown) => {
  let cached = computedCache.get(node as object);
  // One live object per element, as in a browser — but the VALUES are read at
  // access time, so it tracks the cascade instead of freezing it.
  if (!cached) {
    cached = new Proxy(
      {},
      {
        get: (_t, k) => (computedStyleFor(node) as Record<string | symbol, unknown>)[k],
        has: () => true,
      },
    );
    if (node && typeof node === "object") computedCache.set(node, cached);
  }
  return cached;
});

// -- the document surface. redjs ships a `document` object with no tree behind
//    it; these are the entry points a reused component reaches through.
const doc = globalThis.document;
if (doc) {
  const defDoc = (name: string, value: unknown) => {
    if (!(name in doc)) Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  };
  const defDocGet = (name: string, get: () => unknown) => {
    if (!(name in doc)) Object.defineProperty(doc, name, { get, configurable: true });
  };
  // These three are FORCE-installed, not filled in: redjs ships its own
  // `querySelector`/`getElementById` backed by a registry of the elements IT
  // created, which is empty here — the retained tree is ours. Leaving the
  // engine's in place is the difference between "finds the node" and "silently
  // finds nothing", which is what made every `DialogContent` log a false
  // "requires a DialogTitle" accessibility warning.
  const forceDoc = (name: string, value: unknown) =>
    Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  const rootId = () => (appRoot ? appRoot.id : 0);
  forceDoc("querySelector", (sel: string) => queryOne(sel, rootId()));
  forceDoc("querySelectorAll", (sel: string) => hostQuery(sel, rootId(), 0));
  forceDoc("getElementById", (id: unknown) =>
    queryOne(`[id=${JSON.stringify(String(id))}]`, rootId()),
  );
  // `DocumentFragment` is installed on the global by `defineGlobal` above, so
  // it is not part of the engine's declared surface — hence the cast.
  defDoc("createDocumentFragment", () =>
    new (globalThis as unknown as { DocumentFragment: new () => MirrorElement })
      .DocumentFragment(),
  );
  defDoc("createTreeWalker", (root: MirrorNode | null, whatToShow = 0xffffffff, filter: TreeWalkerFilter = null) => {
    // A real pre-order walker over the mirror tree — focus traps use it to
    // collect tabbable candidates, and an empty one silently traps nothing.
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
    return {
      root,
      currentNode: root,
      nextNode() {
        while (++i < flat.length) {
          if (accepts(flat[i])) return (this.currentNode = flat[i]);
        }
        return null;
      },
      previousNode() {
        while (--i >= 0) {
          if (accepts(flat[i])) return (this.currentNode = flat[i]);
        }
        return null;
      },
    };
  });
  defDocGet("defaultView", () => globalThis);
  defDocGet("activeElement", () => null);
  // `document.addEventListener("keydown"|"pointerdown"|…)` is how every
  // dismissable layer in the ecosystem closes itself.
  interceptGlobalTarget(doc);
  interceptGlobalTarget(globalThis);
  // The window's own viewport metrics. redjs reports 0 for these; the window IS
  // the app root here, so they are the root's painted box — and floating
  // positioning reads them.
  //
  // …EXCEPT when the windowing host is present (crates/vn-app/src/
  // window.prelude.js). Then these four are real: `inner*` is the client area
  // the host pushes through `redjs_dom_resize` (and re-pushes with every
  // trusted `resize` event), and `outer*` is `GetWindowRect` — a window fact
  // the app root cannot know, and one that is deliberately NOT equal to
  // `inner*` (docs/WINDOWING.md §1.2). Clobbering them with the root's box
  // would make `outerWidth === innerWidth` and silently break `screenX`'s
  // neighbours. The root box stays the source for `document.documentElement`
  // and `getBoundingClientRect` below, which is what floating positioning
  // measures against.
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
  // `document.documentElement` and `document.body` are REAL STYLE HOSTS: both
  // ARE the app root element, the one node `:root`, `html` and `body` all match
  // (crates/vn-tree/src/tree.rs). So `documentElement.classList.add("dark")`
  // reaches the cascade root and every `hsl(var(--foreground))` under it
  // recomputes, and `documentElement.style.setProperty("--radius", "1rem")`
  // declares a root-level custom property the whole tree inherits — which is
  // how every shadcn/Tailwind theme in existence is switched.
  //
  // They are the SAME object, and that is the honest shape rather than a
  // shortcut: the retained tree has one root, there is no separate `<html>` and
  // `<body>` box to distinguish, and handing out two objects would only make
  // `el.parentNode === document.body` lie. `document.head` stays a property bag
  // — it is not a box at all.
  //
  // Before `mount()` the root does not exist, and module-scope theme code
  // (which runs first, by construction) writes to these. So the pre-mount
  // object BUFFERS, and `mount()` replays it onto the real root.
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
        pending.classes = String(v ?? "").split(/\s+/).filter(Boolean);
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
            pending.style[String(k)] = v == null ? "" : String(v);
          },
          removeProperty(k: unknown) {
            const prev = pending.style[String(k)] ?? "";
            delete pending.style[String(k)];
            return prev;
          },
          getPropertyValue: (k: unknown) => pending.style[String(k)] ?? "",
          getPropertyPriority: () => "",
        },
        {
          set(t, k, v) {
            if (k in t) return Reflect.set(t, k, v);
            pending.style[String(k)] = v == null ? "" : String(v);
            return true;
          },
          get: (t, k) => (k in t ? t[k as keyof typeof t] : pending.style[String(k)] ?? ""),
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
      appendChild: (n: unknown) => n,
      removeChild: (n: unknown) => n,
      insertAdjacentElement: (_w: unknown, n: unknown) => n,
      addEventListener: () => {},
      removeEventListener: () => {},
      getBoundingClientRect: () => ({ ...viewportRect(), toJSON() { return this; } }),
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
    // Called by mount(), once the root node exists.
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
    // `<head>` is not a box: a `<style>` or `<link>` appended to it must not
    // become a child of the root. It stays the property bag redjs ships, with
    // the members a library reaches for filled in.
    const head = doc.head;
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
      if (typeof hs.getPropertyValue !== "function")
        hs.getPropertyValue = (k: unknown) => hs[String(k)] ?? "";
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

// ---- runtime-dom-shaped directives ----------------------------------------
// Standard compiled output (`<input v-model="x">` etc.) must run UNMODIFIED —
// we implement the directives against our host instead of forking the
// compiler (compat-first policy).
const assignKey = Symbol("vnModelAssign");

/** The binding object Vue hands a directive hook, as these four hooks read it. */
interface DirectiveBindingLike {
  value?: unknown;
  oldValue?: unknown;
  modifiers?: Record<string, boolean>;
}
/** The vnode fields the model directives read off their own element. */
interface VNodeLike {
  props?: Record<string, unknown> | null;
}

function setElValue(el: MirrorElement, value: unknown): void {
  const v = value == null ? "" : String(value);
  if (el.value === v) return;
  el.value = v;
  __vn_set_prop(el.id, I("value"), v);
}

export const vModelText = {
  created(el: MirrorElement, { modifiers = {} }: DirectiveBindingLike, vnode: VNodeLike) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    // `.lazy` asks to update on `change` instead of `input`. There is no
    // `change` in `tree.rs` — the host sources no such event — so binding one
    // registered a listener that could never fire, and `v-model.lazy` simply
    // never updated the model. The only sign was one advisory about an unknown
    // event NAME, which reads like a typo rather than like a broken directive.
    //
    // Falling back to `input` makes the binding work and says exactly how it
    // differs. That is the compat-first order of preference: a named
    // divergence beats a silent nothing.
    if (modifiers.lazy) {
      warnCompat(
        "v-model:lazy",
        "v-model.lazy updates on every keystroke here, not on `change` — the native host does " +
          "not source a `change` event yet, so binding one would never fire at all",
      );
    }
    el.addEventListener("input", () => {
      // `.trim` runs before `.number`, so `v` is still a string at both string
      // uses below — which is exactly what the two casts assert.
      let v: string | number = el.value;
      if (modifiers.trim) v = (v as string).trim();
      if (modifiers.number) {
        const n = parseFloat(v as string);
        if (!Number.isNaN(n)) v = n;
      }
      if (el[assignKey]) (el[assignKey] as (value: unknown) => void)(v);
    });
  },
  mounted(el: MirrorElement, { value }: DirectiveBindingLike) {
    setElValue(el, value);
  },
  beforeUpdate(el: MirrorElement, { value }: DirectiveBindingLike, vnode: VNodeLike) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    setElValue(el, value);
  },
};

// v1: every input renders as a text field, so the dynamic/checkbox/radio/
// select variants fall back to the text behavior with an advisory.
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
export const vModelSelect = modelFallback("select");

export const vShow = {
  mounted(el: MirrorElement, { value }: DirectiveBindingLike) {
    __vn_set_style(el.id, I("display"), value ? "" : "none");
  },
  updated(el: MirrorElement, { value, oldValue }: DirectiveBindingLike) {
    if (!value === !oldValue) return;
    __vn_set_style(el.id, I("display"), value ? "" : "none");
  },
};

// ---- template helpers (compiler-dom emits these for @event modifiers) -----
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

export function withModifiers(
  fn: (event: VnEvent, ...args: unknown[]) => unknown,
  modifiers: string[],
) {
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
  };
}

// ---- windowing (docs/WINDOWING.md tier 3) ---------------------------------
//
// `appWindow` itself is built in the host's JS prelude (crates/vn-app/src/
// window.prelude.js) because the host observer, the state snapshot and the
// standard tier-1 globals all have to exist before the app bundle evaluates.
// This module is only its NAMESPACE: apps say
//
//     import { appWindow } from "vue-native";
//
// rather than reaching for a global, which is what keeps the extension
// namespaced (tier 3) instead of pretending to be a web API.
//
// `getCurrentWindow()` exists only so ported Tauri code runs unmodified. When
// multi-window lands it keeps meaning "this window" and `appWindow` keeps being
// its binding — neither gets renamed.
const windowHost = globalThis.__VN_WINDOW__;
// The advisory is LAZY, not eager: a host without the window prelude (the unit
// test harness, a future embedder) must not be scolded for an API it never
// touches — an advisory that fires on code nobody wrote is noise.
const missing = () =>
  warnCompat(
    "appWindow",
    "the windowing host is missing (crates/vn-app/src/window.prelude.js did not evaluate) — " +
      "`appWindow` is inert. See docs/WINDOWING.md.",
  );
// The inert stand-in is a Proxy over `{}`, so only a cast can give it the
// shape: it answers for members that do not exist, which is the point.
const inertWindow = new Proxy(
  {},
  {
    get(_t, key) {
      missing();
      // `isMaximized()` and friends must still answer a boolean, and every
      // verb must still be callable, so inert code paths do not throw.
      return typeof key === "string" && key.startsWith("is") ? () => false : () => undefined;
    },
    set() {
      missing();
      return true;
    },
  },
);

export const appWindow: VnAppWindow = windowHost
  ? windowHost.appWindow
  : (inertWindow as unknown as VnAppWindow);
export const defineWindow: (options: Record<string, unknown>) => void = windowHost
  ? windowHost.defineWindow
  : missing;
export const getCurrentWindow = (): VnAppWindow => appWindow;

// ---- permissions (docs/PERMISSIONS.md tier 2) ------------------------------
//
// The STANDARD surface is `navigator.permissions.query({ name })`, installed by
// redjs, and an app that only needs to read a state should use it — it is the
// same code a browser build runs. `appPermissions` is the deliberately
// namespaced extension for the two things the web has no words for:
//
//   1. REQUESTING outside a capture call. A browser asks for the microphone as
//      a side effect of `getUserMedia()` and for notifications through
//      `Notification.requestPermission()`; there is no standalone request, and a
//      desktop settings screen with a "Grant access" button needs one.
//   2. The FOURTH state, `"unsupported"` — "this platform has no such concept",
//      which `navigator.permissions` has to report as `"denied"` because the web
//      only has three words.
//
// Named after `appWindow` for the same reason: an extension must not be able to
// pass for a web API.
/** The five host fns, gathered so the inert branch is a single `null` check. */
interface PermissionsHost {
  query: typeof __vn_permissions_query;
  list: typeof __vn_permissions_list;
  request: typeof __vn_permissions_request;
  platform: typeof __vn_permissions_platform;
  setDispatcher: typeof __vn_permissions_set_dispatcher;
}

/** One row of `appPermissions.list()`. */
interface PermissionRow {
  name: string;
  state: VnPermissionState;
  canRequest: boolean;
}

/** The tier-2 surface, in both its live and its inert spelling. */
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
  // reqId -> resolve. Settled by the host's per-frame pump, exactly as the
  // LiveKit shim settles `__vn_voice_call` (packages/vue-native/src/livekit.ts).
  const pending = new Map<number, (state: VnPermissionState) => void>();
  host.setDispatcher((token, state) => {
    const resolve = pending.get(token);
    if (!resolve) return;
    pending.delete(token);
    resolve(state);
  });

  // One `change` stream for the whole surface, re-broadcast from the STANDARD
  // objects rather than from a second native subscription: `navigator.
  // permissions` is the source of truth for change delivery, and two drains of
  // one queue would race. Subscribing is best-effort — a runtime without
  // `navigator.permissions` still gets working `query`/`request`.
  const statuses = new Map<string, VnPermissionStatus>();
  if (typeof navigator === "object" && navigator && navigator.permissions) {
    for (const name of PERMISSION_NAMES) {
      navigator.permissions.query({ name }).then(
        (status) => {
          statuses.set(name, status);
          status.addEventListener("change", () => {
            target.dispatchEvent(
              Object.assign(new Event("change"), { name, state: host.query(name) }),
            );
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
        `appPermissions.query('${name}') is 'unsupported' and request() is a no-op. ` +
        `See docs/PERMISSIONS.md.`,
    );
  };

  return Object.assign(target, {
    /** The platform name an advisory would use: "Windows", "macOS". */
    get platform() {
      return host.platform();
    },
    /** Every known permission name. */
    get names() {
      return PERMISSION_NAMES.slice();
    },
    /**
     * SYNCHRONOUS four-state read: "granted" | "denied" | "prompt" |
     * "unsupported". Deliberately not a promise — the standard, promise-shaped
     * read is `navigator.permissions.query()`, and having the extension differ
     * in shape as well as in name makes it obvious which one a call site used.
     */
    query(name: string) {
      const state = host.query(name);
      if (!state) throw new TypeError(`'${name}' is not a permission name`);
      if (state === "unsupported") adviseUnsupported(name);
      return state;
    },
    /** All of them at once, as `{ name, state, canRequest }[]`. */
    list(): PermissionRow[] {
      const rows = JSON.parse(host.list()) as PermissionRow[];
      for (const r of rows) if (r.state === "unsupported") adviseUnsupported(r.name);
      return rows;
    },
    /**
     * Ask for a permission. Resolves with the resulting four-state string.
     *
     * THE DIVERGENCE, stated plainly: no browser has this call. What it does is
     * platform-shaped — on Windows it opens the relevant Settings page and
     * resolves when the toggle actually moves (or after a timeout, with the
     * unchanged state, so an `await` can never hang); on macOS it will raise the
     * real system dialog. It never resolves optimistically: the value is always
     * a fresh read of what the OS now says.
     */
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
          "`appPermissions` is inert and every capability reads as 'unsupported'. " +
          "See docs/PERMISSIONS.md.",
      );
      if (key === "platform") return "this platform";
      if (key === "names") return PERMISSION_NAMES.slice();
      if (key === "query") return () => "unsupported";
      if (key === "list")
        return () =>
          PERMISSION_NAMES.map((name) => ({ name, state: "unsupported", canRequest: false }));
      if (key === "request") return () => Promise.resolve("unsupported");
      return () => undefined;
    },
  },
);

export const appPermissions: AppPermissions = permHost
  ? createPermissions(permHost)
  : (inertPermissions as unknown as AppPermissions);

// ---- storage (docs/STORAGE.md tier 2) --------------------------------------
//
// The STANDARD surface is `localStorage` / `sessionStorage`, installed by redjs,
// and an app that just wants to remember a preference should use it and nothing
// else — it is the same code a browser build runs, down to the
// `QuotaExceededError`. `appStorage` is the deliberately namespaced extension
// for the two things a browser has no words for, because a browser decides them
// for you:
//
//   1. WHERE the bytes go. A browser has one profile directory and no API to
//      ask about it; a desktop app has %APPDATA% vs %LOCALAPPDATA% vs a
//      portable folder next to the exe, and the choice is the app's to make.
//   2. WHETHER they are encrypted at rest. There is no browser spelling for
//      "and please use DPAPI".
//
// Named after `appWindow`/`appPermissions` for the same reason: an extension
// must not be able to pass for a web API. It is NOT called `app` — which is
// what the original sketch said — because `const app = createApp(...)` is the
// first line of nearly every Vue entry module, and `import { app } from
// "vue-native"` next to it is a redeclaration error in the same scope. The
// verbs are exactly as sketched.
//
// Synchronous, like `appPermissions.query` and for the same reason: these are
// boot-time calls that touch a path and a directory, and a promise would imply
// a wait that does not exist.
/** The four host fns, gathered so the inert branch is a single `null` check. */
interface StorageHost {
  userFolder: typeof __vn_storage_user_folder;
  setFolder: typeof __vn_storage_set_folder;
  info: typeof __vn_storage_info;
  flush: typeof __vn_storage_flush;
}

/**
 * IndexedDB's half of `appStorage.info()` (crates/vn-storage/src/idb.rs): the
 * databases live in `<folder>\indexeddb`. `mode` is what IndexedDB is running
 * in — `null` until the app's first `indexedDB` use, when the folder and the
 * key are latched for the run; a `setStorageFolder` after that applies at the
 * next start. `key`: a sealed key exists in that folder. `null` as a whole in
 * a host built without on-disk IndexedDB.
 */
interface IndexedDbInfo {
  folder: string;
  started: boolean;
  mode: "plaintext" | "encrypted" | "decrypting" | "refused" | null;
  encrypted: boolean | null;
  key: boolean;
  error: string | null;
}

/** Everything `appStorage.info()` reports. `error` is the LAST failure, or null. */
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
}

/** The tier-2 surface, in both its live and its inert spelling. */
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
    /** The folder kinds `getUserFolder` accepts. */
    get kinds() {
      return (JSON.parse(host.info()) as StorageInfo).kinds;
    },
    /** Where `localStorage` is being kept right now. */
    get folder() {
      return (JSON.parse(host.info()) as StorageInfo).folder;
    },
    /** Is it encrypted at rest? */
    get encrypted() {
      return (JSON.parse(host.info()) as StorageInfo).encrypted;
    },
    /**
     * `getUserFolder("APPDATA", "myAppFolder")` → an absolute path.
     *
     * Creates nothing — a path is not a directory, and an app that only wants
     * to SHOW a user where its data would go should not leave a folder behind.
     * Throws a TypeError for an unknown kind or a `subfolder` that tries to
     * climb out of it, rather than returning a path nobody checked.
     */
    getUserFolder(kind: string, subfolder = "") {
      return storageResult<{ path: string }>(
        host.userFolder(String(kind), String(subfolder)),
        "getUserFolder",
      ).path;
    },
    /**
     * Points `localStorage` at `path`, creating it. Returns the canonical path.
     *
     * `{ encrypted: true }` encrypts the file at rest (Windows: DPAPI, tied to
     * the Windows user — see docs/STORAGE.md for what that does and does not
     * protect). Turning it on for a folder that already holds a plaintext area
     * MIGRATES that area and removes the plaintext copy; turning it off does
     * the reverse. On a platform with no encryption provider this THROWS rather
     * than quietly writing plaintext.
     *
     * Best called before the first `localStorage` touch. Called later it still
     * works: the area this process is holding is written to the new place.
     *
     * IndexedDB follows the same folder (`<path>\indexeddb`) and the same
     * encryption, with the same sticky rule — but it latches both at its first
     * use and never moves open databases, so call this BEFORE the first
     * `indexedDB` call. Later, the change applies at the next start (said once
     * on stderr; the encryption half is remembered even if that start calls
     * late again). docs/STORAGE.md "IndexedDB".
     */
    setStorageFolder(path: string, options: { encrypted?: boolean } = {}) {
      const encrypted = !!(options && options.encrypted);
      return storageResult<{ path: string }>(
        host.setFolder(String(path), encrypted),
        "setStorageFolder",
      ).path;
    },
    /**
     * Everything at once: `{ folder, file, encrypted, bytes, loaded, pending,
     * provider, kinds, error }`. `error` is the last thing that went wrong
     * (a folder that could not be written, a file that could not be decrypted)
     * or null.
     */
    info(): StorageInfo {
      return JSON.parse(host.info()) as StorageInfo;
    },
    /**
     * Writes any pending change to disk NOW and reports whether there was one.
     *
     * Writes are normally coalesced to one per frame, so a hard kill can lose
     * the last frame's. This is the escape hatch for the moment where that
     * matters — and what the persistence test calls before restarting the host.
     */
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
          "persist. See docs/STORAGE.md.",
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
      // `getUserFolder` and `setStorageFolder` cannot answer with a plausible
      // path — a wrong path is worse than no path, because the app would then
      // write somewhere real.
      return () => {
        throw new TypeError(`appStorage.${String(key)} is unavailable: no storage host`);
      };
    },
  },
);

export const appStorage: AppStorage = storageHost
  ? createStorage(storageHost)
  : (inertStorage as unknown as AppStorage);

// ---- the desktop host: settle channel (crates/vn-desktop) -------------------
//
// Every async desktop call answers `{token}` (or a synchronous refusal) and is
// settled later through ONE dispatcher by `vn_desktop::pump()` in the frame
// pump — the permissions shape. Token 0 is an UNSOLICITED host event (a second
// instance's `launch`) and is handed to `desktopEvents`.

/** The host fns, gathered so the inert branch is a single `null` check. */
interface DesktopHost {
  setDispatcher: typeof __vn_desktop_set_dispatcher;
  openExternal: typeof __vn_shell_open_external;
  openPath: typeof __vn_shell_open_path;
  showItem: typeof __vn_shell_show_item;
  trash: typeof __vn_shell_trash;
}

const desktopHost: DesktopHost | null =
  typeof __vn_desktop_set_dispatcher === "function"
    ? {
        setDispatcher: __vn_desktop_set_dispatcher,
        openExternal: __vn_shell_open_external,
        openPath: __vn_shell_open_path,
        showItem: __vn_shell_show_item,
        trash: __vn_shell_trash,
      }
    : null;

/** A rejection as the host serialises it (`Fail::to_json` / `to_sync_json`). */
interface DesktopFailure {
  name: string;
  message?: string;
  error?: string;
  code?: string;
  osError?: number;
}

/**
 * The error a tier-2 desktop call rejects with: an `Error` whose `name` is the
 * DOMException-style name (`NotFoundError`, `NotAllowedError`, …), plus the
 * Node-style `code` an app can switch on and the raw Win32 `osError`
 * (desktop-apis-design §3.3). A refused argument is a plain `TypeError`.
 */
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

const desktopPending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
/** Consumers of token-0 events, by `payload.kind`. */
const desktopEvents = new Map<string, (payload: Record<string, unknown>) => void>();

if (desktopHost) {
  desktopHost.setDispatcher((token, ok, json) => {
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
    if (ok) p.resolve(payload);
    else p.reject(desktopError(payload as unknown as DesktopFailure));
  });
}

/** One host answer → a promise: `{token}` waits for the pump, a refusal rejects now. */
function desktopCall(json: VnJson): Promise<Record<string, unknown>> {
  let out: { token?: number } & Partial<DesktopFailure>;
  try {
    out = JSON.parse(json) as typeof out;
  } catch {
    return Promise.reject(new Error(`the desktop host answered something that is not JSON: ${json}`));
  }
  if (out.error !== undefined || out.token === undefined) {
    return Promise.reject(desktopError({ name: out.name ?? "Error", ...out }));
  }
  const token = out.token;
  return new Promise((resolve, reject) =>
    desktopPending.set(token, { resolve: resolve as (v: unknown) => void, reject }),
  );
}

// ---- links and window.open: the tier-1 path to the shell --------------------

/**
 * Hands a CONTENT URL (an `<a href>`, a `window.open`) to the system. The
 * allowlist has already been applied by the caller and is applied AGAIN by the
 * host (`content = true`), so neither side alone is the security line. Never
 * throws and returns nothing, as a browser's navigation does not; a failure
 * (no mail client for `mailto:`) is a console warning.
 */
function openContentUrl(url: string, via: string): void {
  if (!desktopHost) {
    warnCompat(
      "shell:absent",
      `${via}: the desktop host is missing (vn_desktop::install did not run) — the link was not ` +
        `opened. See docs/SHELL.md`,
    );
    return;
  }
  // The serialised form a browser's `a.href` reports: spaces and non-ASCII
  // percent-encoded, which is also what the shell wants.
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

// `window.open(url)` for an EXTERNAL url opens the system browser and returns
// `null` — `noopener` semantics, and what an Electron app whose
// `setWindowOpenHandler` answers `{action: "deny"}` returns (D-F7). Everything
// else — no url, a relative one, `about:blank` — is still the prelude's A-22
// ("a second document" stays reserved, docs/WINDOWING.md §8). Wrapped HERE
// rather than in window.prelude.ts because this is where the link path and the
// desktop host live; the prelude evaluates first, so its `open` is the
// fallback.
{
  type OpenFn = (url?: unknown, target?: unknown, features?: unknown) => unknown;
  const g = globalThis as unknown as { open?: OpenFn };
  const prior = typeof g.open === "function" ? g.open : null;
  g.open = function open(url?: unknown, target?: unknown, features?: unknown) {
    const s = url == null ? "" : String(url).trim();
    const m = SCHEME_RE.exec(s);
    // A one-letter "scheme" is a drive letter (`C:\x`), not a URL.
    if (m && m[1].length > 1) {
      const scheme = m[1].toLowerCase();
      if (scheme !== "about" && scheme !== "javascript") {
        if (CONTENT_SCHEMES.has(scheme)) openContentUrl(s, "window.open()");
        else adviseScheme(scheme, "window.open()");
        return null;
      }
    }
    return prior ? prior(url, target, features) : null;
  };
}

// ---- shell (docs/SHELL.md tier 2) -------------------------------------------
//
// What a browser cannot do, so there is no tier-1 spelling to prefer: open a
// FILE with its default app, reveal one in Explorer, move one to the Recycle
// Bin, and open a URL of a scheme content may not reach. Electron's
// `shell.openExternal` / `openPath` / `showItemInFolder` / `trashItem`, with
// two deliberate differences, both of them in the safe direction:
//
//   * `openPath` REFUSES programs and scripts (`.exe`, `.bat`, `.lnk`, `.ps1`,
//     `.js`, every PATHEXT entry, …) unless `{ allowExecutable: true }` — a
//     Save-dialog-then-open flow must not launch an attacker-named
//     `report.pdf.exe`;
//   * `openExternal` refuses `file:` (a TypeError naming `openPath`), so a URL
//     is never a way around the executable guard.
//
// Every call is a promise that REJECTS on failure — `NotFoundError`/`ENOENT`
// for a missing file, `NotFoundError`/`ENOASSOC` for "nothing is associated
// with .xyz" — instead of Electron's resolve-with-an-error-string.

/** The tier-2 surface, in both its live and its inert spelling. */
interface AppShell {
  openExternal(url: string): Promise<void>;
  openPath(path: string, options?: { allowExecutable?: boolean }): Promise<void>;
  showItemInFolder(path: string): Promise<void>;
  trashItem(path: string): Promise<void>;
}

function createShell(host: DesktopHost): AppShell {
  const done = (p: Promise<unknown>): Promise<void> => p.then(() => undefined);
  return {
    /**
     * Opens `url` with whatever the system has registered for its scheme —
     * the default browser for https, the mail client for mailto, an app's own
     * protocol, `ms-settings:`. Any scheme but `file:`. Resolves once the shell
     * has accepted it (not when the browser has loaded the page).
     */
    openExternal(url: string) {
      return done(desktopCall(host.openExternal(String(url), false)));
    },
    /**
     * Opens a FILE (or folder) with its default application, as a double-click
     * in Explorer does. `path` must be absolute. Programs and scripts are
     * refused (`NotAllowedError`, code `EEXECUTABLE`) unless
     * `{ allowExecutable: true }`.
     */
    openPath(path: string, options: { allowExecutable?: boolean } = {}) {
      return done(desktopCall(host.openPath(String(path), !!(options && options.allowExecutable))));
    },
    /** Opens the containing folder in Explorer with `path` selected. */
    showItemInFolder(path: string) {
      return done(desktopCall(host.showItem(String(path))));
    },
    /**
     * Moves `path` to the Recycle Bin. NEVER a permanent delete: on a drive
     * with no Recycle Bin (or an item too large for it) this rejects
     * (`InvalidStateError`, code `ENOTRASH`) and nothing is deleted.
     */
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
          "every call rejects. See docs/SHELL.md.",
      );
      return () =>
        Promise.reject(
          desktopError({ name: "NotSupportedError", message: `appShell.${String(key)}: no desktop host`, code: "ENOSYS" }),
        );
    },
  },
);

export const appShell: AppShell = desktopHost ? createShell(desktopHost) : (inertShell as unknown as AppShell);

// ---- launch (docs/LAUNCH.md) ------------------------------------------------
//
// TIER 1 is the Launch Handler API: `launchQueue.setConsumer(params => …)`,
// with `params.targetURL` the deep link and `params.files` the file handles.
// This process's own launch is one `LaunchParams`, buffered until a consumer
// is set (per spec); every launch a SECOND process forwards here (with
// `singleInstance`) is another one — the `"focus-existing"` client mode.
//
// TIER 2, `appLaunch`, is what a browser cannot say: the raw argv (Electron's
// `process.argv` shape), the launch directory, which arguments are existing
// paths, whether this process holds the single-instance lock, and the
// `launch` event with its `source`. Deep links and argv are UNTRUSTED INPUT —
// a web page can start `myapp://anything` — so they are strings, never code.

/** What `__vn_launch_info` reports. */
interface LaunchInfo {
  argv: string[];
  cwd: string;
  url: string | null;
  paths: string[];
  singleInstance: boolean;
  id?: string;
  name?: string;
}

/** The `launch` event (`appLaunch.addEventListener("launch", …)`). */
export interface VnLaunchEvent extends VnEvent {
  readonly argv: readonly string[];
  readonly cwd: string;
  readonly url: string | null;
  readonly paths: readonly string[];
  readonly source: "second-instance" | "open-url" | "reopen";
}

/** The tier-2 surface, in both its live and its host-less spelling. */
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

/**
 * `LaunchParams` (WICG Launch Handler). `targetURL` is the RAW deep link —
 * there is no app URL scope to template it into — and `files` is empty until
 * the File System Access handles exist (desktop-apis tracks B/C); the paths
 * are on `appLaunch.paths` / the `launch` event meanwhile. Both divergences
 * are D-F8, and the second one advises once.
 */
class LaunchParams {
  readonly targetURL: string | null;
  readonly files: readonly unknown[];
  constructor(targetURL: string | null, files: readonly unknown[] = []) {
    this.targetURL = targetURL;
    this.files = Object.freeze(files.slice());
  }
}

type LaunchConsumer = (params: LaunchParams) => unknown;

/** `window.launchQueue`. */
class LaunchQueue {
  #consumer: LaunchConsumer | null = null;
  #pending: LaunchParams[] = [];

  /**
   * Sets the consumer and hands it every launch that arrived before it —
   * synchronously, in order, as Chromium's `LaunchQueue::setConsumer` does.
   */
  setConsumer(consumer: LaunchConsumer): void {
    if (typeof consumer !== "function") {
      throw new TypeError("Failed to execute 'setConsumer' on 'LaunchQueue': parameter 1 is not a function.");
    }
    this.#consumer = consumer;
    const queued = this.#pending.splice(0);
    for (const p of queued) this.#deliver(p);
  }

  /** Host side: a launch happened. */
  _enqueue(params: LaunchParams): void {
    if (this.#consumer) this.#deliver(params);
    else this.#pending.push(params);
  }

  #deliver(params: LaunchParams): void {
    try {
      void this.#consumer!(params);
    } catch (e) {
      // A consumer that throws must not take the next launch with it.
      console.error(`[vue-native] launchQueue consumer threw: ${e instanceof Error ? e.stack ?? e.message : e}`);
    }
  }
}

function launchParamsFor(url: string | null, paths: readonly string[]): LaunchParams {
  if (paths.length) {
    warnCompat(
      "launchQueue:files",
      "launchQueue: LaunchParams.files is empty — FileSystemFileHandle is not implemented yet (desktop-apis " +
        "track C); the launch's existing paths are on appLaunch.paths and on the `launch` event",
    );
  }
  return new LaunchParams(url, []);
}

const launchQueue = new LaunchQueue();
{
  const g = globalThis as unknown as { launchQueue?: unknown; LaunchParams?: unknown; LaunchQueue?: unknown };
  // Installed only where the engine has none (redjs does not), never over one.
  if (g.launchQueue === undefined) {
    g.launchQueue = launchQueue;
    g.LaunchParams = LaunchParams;
    g.LaunchQueue = LaunchQueue;
  }
}
// This process's own launch. A host without vn-desktop has no launch to report.
if (typeof __vn_launch_info === "function") {
  launchQueue._enqueue(launchParamsFor(launchInfo.url, launchInfo.paths));
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
    // The tier-2 event first, then the tier-1 queue — the order docs/LAUNCH.md
    // §4 documents. Either may be the one an app listens to; neither waits on
    // the other.
    try {
      target.dispatchEvent(ev);
    } finally {
      launchQueue._enqueue(launchParamsFor(url, evPaths));
    }
  });
  return Object.defineProperties(target, {
    /** Electron's `process.argv` shape: the exe first, then the arguments (no `--vn-*`). */
    argv: { get: () => argv, enumerable: true },
    /** The directory the app was launched from. */
    cwd: { get: () => launchInfo.cwd, enumerable: true },
    /** The deep link this process was started for (`myapp://…`), or null. UNTRUSTED. */
    url: { get: () => launchInfo.url, enumerable: true },
    /** The arguments that name an existing file or folder, absolute. */
    paths: { get: () => paths, enumerable: true },
    /** Does THIS process hold the single-instance lock (`"singleInstance": true` and it was first)? */
    singleInstance: { get: () => launchInfo.singleInstance, enumerable: true },
    /**
     * Registers `scheme://` for THIS exe, per user (HKCU — never machine-wide).
     * Synchronous: three registry writes. Throws a TypeError for an invalid
     * or reserved scheme (http, file, ms-*, …) and for one that another app
     * already owns — vue-native never takes over a scheme. A packaged app
     * registers its declared `protocols` itself at startup; in a DEV host this
     * points the scheme at the dev slot (advisory A-F10).
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
    /** Removes a registration of OURS that points at THIS exe. Returns whether it did. */
    unregisterProtocol: {
      value(scheme: string): boolean {
        return typeof __vn_launch_unregister_protocol === "function" && __vn_launch_unregister_protocol(String(scheme));
      },
    },
    /** Is `scheme://` registered to this app AND to this exe? */
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

// ---- devtools (docs/DEVTOOLS.md, tier 3) -----------------------------------
//
// Since m22 the devtools — the protocol listener, the embedded inspector and
// F12 — are compiled into EVERY build, `--features prod` included. That is the
// owner's decision (docs/ARCHITECTURE.md §11 decision 12) and it is what a
// browser and what Electron do; the thing an app gets in exchange is the
// ability to turn them off HERE, at runtime, in its own JS:
//
//     import { appDevtools } from "vue-native";
//     appDevtools.disable();     // …or `"devtools": false` in vn.config.json,
//                                //   which is the same decision made earlier
//
// Named after `appWindow`/`appPermissions`/`appStorage` for the same reason:
// an extension must not be able to pass for a web API. There is no browser
// spelling of any of this — a page cannot disable F12 and has no idea whether
// a debugger is attached — so the whole surface is tier 3 by construction, and
// nothing here shadows or weakens a standard API.
//
// WHAT "DISABLED" MEANS, and why it is the stronger of the two possible
// answers: the devtools port carries `eval`. A shipped app whose author turned
// the devtools off must not still be holding a localhost socket that runs
// arbitrary JS inside it — "the key is refused but the port is open" would be
// a security posture nobody asked for. So `disable()` closes the listener,
// drops every attached inspector socket, blanks the token and deletes the
// handshake file; `enable()` binds a FRESH port with a FRESH token, so a client
// that was shown the door cannot walk back in on the old one.
//
// THE ONE OVERRIDE: `VN_DEVTOOLS=1` in the environment forces them on and
// refuses `disable()`. It exists so a developer can debug a binary somebody
// shipped with them off. It hands out no authority its user did not already
// have — the port is localhost-only and token-guarded, so reaching it already
// needs local code execution as the same user, and somebody who can set this
// process's environment could equally replace its executable. What it buys back
// is the property this whole project runs on: an app must not be able to make
// itself undebuggable.

/** The three host fns, gathered so the inert branch is a single `null` check. */
interface DevtoolsHost {
  state: typeof __vn_devtools_state;
  setEnabled: typeof __vn_devtools_set_enabled;
  open: typeof __vn_devtools_open;
}

/** Everything `appDevtools.info()` reports. */
export interface DevtoolsInfo {
  /** Are they on? Off means there is no listener at all. */
  enabled: boolean;
  /** Held on by `VN_DEVTOOLS=1`, so `disable()` is refused. */
  locked: boolean;
  /** The port the listener is on, or `null` when disabled. */
  port: number | null;
  /** This process's pid — what a client matches the handshake against. */
  pid: number;
  /** May F12 open the inspector? The separate `VN_DEVTOOLS_KEY` gate. */
  key: boolean;
  /** How many inspectors this process currently has open on itself. */
  inspectors: number;
  /** The dist slot whose `devtools.json` this host publishes into. */
  slot: string;
}

/** The tier-3 surface, in both its live and its inert spelling. */
interface AppDevtools {
  /**
   * Does this BUILD have the devtools at all?
   *
   * `false` only for `--features no-devtools`, and it is a different fact from
   * `isEnabled()`: "turned off" is a decision this process can reverse, "not in
   * the binary" is not. An app that draws a devtools toggle should hide it
   * rather than draw a switch that cannot move.
   */
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

/**
 * The `onChange` registry.
 *
 * JS-side, and that is honest rather than a shortcut: every transition goes
 * through `enable()`/`disable()` in this module (the boot state is settled
 * before the app bundle evaluates, and nothing else in the process moves the
 * switch), so a listener here sees exactly the set of changes that exist. If
 * the host ever grows a second mover — a menu item in the shell, say — this
 * becomes a host callback and the signature does not change.
 */
const devtoolsWatchers = new Set<(enabled: boolean) => void>();

function notifyDevtools(enabled: boolean) {
  for (const fn of [...devtoolsWatchers]) {
    try {
      fn(enabled);
    } catch (e) {
      // One listener's throw must not stop the others, and must not turn a
      // successful `disable()` into a failed one.
      warnCompat(
        "appDevtools.onChange",
        `a change listener threw and was ignored: ${String(e)}`,
      );
    }
  }
}

function createDevtools(host: DevtoolsHost): AppDevtools {
  const read = (): DevtoolsInfo => JSON.parse(host.state()) as DevtoolsInfo;
  const set = (on: boolean): boolean => {
    const was = read().enabled;
    const out = JSON.parse(host.setEnabled(on)) as DevtoolsInfo & { error?: string };
    if (out.error) {
      // Refused, not failed — and refusal has exactly one cause. An advisory
      // rather than a throw: the app asked for something reasonable and the
      // machine's owner overruled it, which must not take a shipped app's boot
      // code down.
      warnCompat("appDevtools", `${on ? "enable" : "disable"}() was refused: ${out.error}`);
      return was;
    }
    if (out.enabled !== was) notifyDevtools(out.enabled);
    return out.enabled;
  };
  return {
    available: true,
    /** Are the devtools on in this process right now? */
    isEnabled: () => read().enabled,
    /**
     * Turns them on: binds a fresh localhost port with a fresh token and
     * republishes `<dist>/devtools.json`. Idempotent — enabling a host that is
     * already listening keeps the port it has, so calling this on every route
     * change does not hand an attached inspector a new port each time.
     */
    enable: () => set(true),
    /**
     * Turns them off: closes the listener, drops every attached inspector
     * socket, blanks the token and deletes the handshake. F12 becomes a no-op
     * that says why, once, in the log.
     *
     * Returns the state AFTERWARDS, so `false` means it worked and `true` means
     * `VN_DEVTOOLS=1` overruled it (with an advisory naming that).
     */
    disable: () => set(false),
    /**
     * Programmatic F12 — for an app that wants its own "Open devtools" menu
     * item. The same entry point the key uses, so it shares the debounce and
     * the "raise the one that is already open" behaviour, and it does not take
     * focus. `false` when the devtools are disabled.
     */
    open: () => {
      const out = JSON.parse(host.open()) as { opened?: boolean; error?: string };
      if (out.error) {
        warnCompat("appDevtools", `open() did not open an inspector: ${out.error}`);
        return false;
      }
      return !!out.opened;
    },
    /** Everything at once: `{ enabled, locked, port, pid, key, inspectors, slot }`. */
    info: () => read(),
    /** Calls `fn(enabled)` on every settled change. Returns the unsubscriber. */
    onChange(fn: (enabled: boolean) => void) {
      devtoolsWatchers.add(fn);
      return () => {
        devtoolsWatchers.delete(fn);
      };
    },
  };
}

/**
 * `--features no-devtools`: there is nothing to switch.
 *
 * It answers rather than throws — `appDevtools.disable()` in an app's entry
 * module is the most likely single caller, and a build that already has no
 * devtools must not crash on being asked to turn them off. One advisory, on
 * the first touch, so the absence is stated rather than mimed.
 */
const inertDevtools: AppDevtools = {
  available: false,
  isEnabled: () => false,
  enable: () => {
    warnCompat(
      "appDevtools",
      "this host was built with `--features no-devtools`, so there are no devtools to enable " +
        "— `appDevtools.enable()` did nothing. See docs/DEVTOOLS.md.",
    );
    return false;
  },
  disable: () => false,
  open: () => {
    warnCompat(
      "appDevtools",
      "this host was built with `--features no-devtools` — `appDevtools.open()` has no " +
        "inspector to open. See docs/DEVTOOLS.md.",
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
  // Still a real registry: a listener registered here is simply never called,
  // which is the truth, and the unsubscriber still works so no caller special-
  // cases the inert build.
  onChange(fn: (enabled: boolean) => void) {
    devtoolsWatchers.add(fn);
    return () => {
      devtoolsWatchers.delete(fn);
    };
  },
};

export const appDevtools: AppDevtools = devtoolsHost
  ? createDevtools(devtoolsHost)
  : inertDevtools;

// ---- <Transition> / <TransitionGroup> (compat roadmap G9) ------------------
// runtime-core exports BaseTransition (the state machine) but NOT these two —
// they live in runtime-dom, because the CSS half is DOM-specific. Libraries
// import them from "vue" unconditionally (@vueuse/core does, at MODULE SCOPE),
// so their absence is `undefined` in dev and a hard build failure in prod.
//
// This is runtime-dom's implementation, ported onto our host: the same class
// phases, the same hook shapes, the same `mode`/`appear` handling (which is
// BaseTransition's, so it comes for free), riding vn-css's REAL transition
// engine. One divergence, stated once at runtime: there is no `transitionend`
// event, so the end of a transition is detected purely by the timer runtime-dom
// itself already uses as its fallback — read off the element's computed
// `transition-duration`/`transition-delay`.

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

// runtime-dom adds the phase class to `el.classList` AND records it in `_vtc`,
// because in a browser those are two views of the same attribute. Here they are
// two SOURCES for one attribute (`writeClass` joins them), so recording it once
// is the whole job — adding it to `className` as well would emit it twice.
function addTransitionClass(el: MirrorElement, cls: string): void {
  (el._vtc || (el._vtc = new Set())).add(cls);
  writeClass(el);
}
function removeTransitionClass(el: MirrorElement, cls: string): void {
  if (el._vtc) {
    el._vtc.delete(cls);
    if (!el._vtc.size) el._vtc = undefined;
  }
  writeClass(el);
}

/// Two frames, as runtime-dom does it: one to let the *-from class be committed
/// and cascaded, the next to swap in *-to so the engine sees a real change.
function nextFrame(cb: () => void): void {
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf === "function") raf(() => raf(cb));
  else setTimeout(() => setTimeout(cb, 0), 0);
}

const toMs = (s: string): number =>
  s === "auto" || !s ? 0 : Number(String(s).slice(0, -1).replace(",", ".")) * 1000;
function listTimeout(delays: string[], durations: string[]): number {
  while (delays.length < durations.length) delays = delays.concat(delays);
  return Math.max(0, ...durations.map((d: string, i: number) => toMs(d) + toMs(delays[i])));
}

/// How long the element's CSS says its transition takes. Same computation
/// runtime-dom does, off the same two properties.
function transitionTimeout(el: MirrorElement): number {
  // `getComputedStyle` is one of the globals THIS FILE installs (see
  // `defineGlobal` above), so it is not on the engine's declared surface and
  // the cast is what says where it came from.
  const styles = (
    globalThis as unknown as { getComputedStyle: (el: MirrorElement) => Record<string, string> }
  ).getComputedStyle(el);
  const split = (k: string) => String(styles[k] || "").split(", ");
  return listTimeout(split("transitionDelay"), split("transitionDuration"));
}

let endId = 0;
/// End a `<Transition>` phase on the REAL `transitionend`, with the computed
/// timeout as the safety net — which is exactly runtime-dom's own strategy, and
/// is now possible because vn-css's animator sources the event (m12 item 6).
///
/// The event matters rather than merely being tidier: the timer reports the
/// declared duration whatever actually happened, so a transition the engine cut
/// short (reduced motion, a competing write to the same property, the element
/// being restyled mid-flight) left the phase class on for its full nominal
/// length. The event fires when the track actually settles.
///
/// The net is still armed, and for the same two reasons runtime-dom arms it: an
/// element whose transition never starts (the property did not actually change)
/// fires nothing at all, and a property that is not animatable here settles
/// without an event.
function whenTransitionEnds(
  el: MirrorElement,
  explicitTimeout: number | null | undefined,
  resolve: () => void,
): void {
  const id = (el._endId = ++endId);
  let done = false;
  const resolveIfNotStale = () => {
    if (done || id !== el._endId) return;
    done = true;
    el.removeEventListener?.("transitionend", onEnd);
    resolve();
  };
  const onEnd = (e: VnEvent) => {
    // Only this element's own transitions end this phase — a descendant's
    // bubbles through, and runtime-dom filters it the same way.
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

// A `<Transition>` hook prop is whatever the app passed: one function, an array
// of them, or nothing. Vue's own runtime-dom types it the same open way, so the
// casts below are at the call, where the arity is known.
const callHook = (hook: unknown, args: unknown[] = []) => {
  if (Array.isArray(hook)) for (const h2 of hook) (h2 as (...a: unknown[]) => void)(...args);
  else if (hook) (hook as (...a: unknown[]) => void)(...args);
};
/** Did the app's hook declare a `done` parameter? Then IT ends the phase. */
const hasExplicitCallback = (hook: unknown): boolean =>
  hook
    ? Array.isArray(hook)
      ? hook.some((h2) => (h2 as { length: number }).length > 1)
      : (hook as { length: number }).length > 1
    : false;

/** `:duration="{ enter, leave }"` — the object form of the prop. */
type DurationPair = { enter: unknown; leave: unknown };

function normalizeDuration(duration: unknown): [number, number] | null {
  if (duration == null) return null;
  if (typeof duration === "object")
    return [Number((duration as DurationPair).enter), Number((duration as DurationPair).leave)];
  const n = Number(duration);
  return [n, n];
}

/**
 * `<Transition>`'s own props, plus whatever else the app put on the component —
 * everything not in `TRANSITION_PROPS` is forwarded to `BaseTransition`, which
 * is what the index signature stands for.
 */
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
          setTransitionHooks(
            child,
            resolveTransitionHooks(child, cssTransitionProps, state, instance!),
          );
        }
      }
      return createVNode(tag as never, null, children as never);
    };
  },
});

// `<VnWindow>` — a second OS window painting part of this tree
// (docs/MULTIWINDOW.md). Tier 3, so it is a NAMESPACED import an app opts into
// by name: `import { VnWindow } from "vue-native"`. Nothing in the renderer
// above knows about it — a pop-out is addressed by `mirrorNode.id`, which every
// template ref already hands you, so the component is a leaf of this module
// rather than a change to it.
export { VnWindow, VN_WINDOW_CREATION_TIME_PROPS } from "./vnwindow";
export type { VnWindowBounds, VnWindowMetrics, VnWindowTarget } from "./vnwindow";

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "vue-native" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
