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
// Type-only, so it is erased before the bundler ever sees it. `RendererOptions`
// is what makes `nodeOps` below self-typing: every hook's parameters come from
// Vue's own declaration, so a signature that drifts from what the renderer
// actually calls is a build error rather than an `undefined` at runtime.
import type { App, ComponentPublicInstance, RendererOptions, VNode } from "@vue/runtime-core";
// File System Access, FileReader and the file pickers (docs/DESKTOP.md). A
// module of its own with its dependencies injected (see its header); it has no
// tier-2 surface to re-export, so it is installed here rather than re-exported.
import { installDesktop } from "./desktop";
import type { DesktopApi, DesktopEngine, DesktopFsHost, DesktopSettle, HandleWire } from "./desktop";
// `<canvas>`'s bitmap and its 2D context, keyed by the host node id (the
// host+media stream's module). `nodeOps.createElement` adopts every canvas,
// the renderer's and `document.createElement`'s alike.
import { adoptCanvas } from "./canvas";
// `FontFace` and `document.fonts` over the host's `@font-face` loader (the
// host+media stream's module; crates/vn-tree/src/fonts.rs).
import { installFonts } from "./fonts";
// `getSelection()`, `Selection` and `Range` over the host's document text
// selection (crates/vn-tree/src/selection.rs).
import { installSelection } from "./selection";
// `Element.animate()`, `Animation`, `KeyframeEffect` and `document.timeline`
// over the host's script-animation layer (crates/vn-css/src/animator/script.rs).
import { installWaapi } from "./waapi";
// The painted roots of the `<VnWindow>`s: a node under one is in that window's
// document (`windowRootOf`).
import { VnWindow, WINDOW_ROOT, windowRoots } from "./vnwindow";

// ---- the realm a reload keeps (m26 client sweep #3) ------------------------
//
// `location.reload()` evaluates this module again in the SAME JavaScript realm
// (crates/vn-app/src/reload.rs), and so does the dev host's cold restart. A
// browser would start over in a fresh realm. Here the outgoing runtime and app
// leave their globals behind, and two records keep that from leaking into the
// next run:
//
// - `REALM.installed` holds every function and object a runtime installed on
//   an engine-owned object: the global, `document`, `document.head`. Where this
//   file fills a hole ("only when the engine has none"), `vacant()` counts a
//   predecessor's install as a hole, so the runtime evaluated next installs its
//   OWN class over it. A kept class runs against the old runtime's state. The
//   old `ResizeObserver` waited forever on an animation frame the unload had
//   cancelled, and every observer after a reload stayed silent.
// - `REALM.base` is the global's own keys when the FIRST runtime started,
//   before the app. `__VN_UNLOAD__` deletes every configurable global added
//   since (`resetGlobals`), so the next app sees what a fresh realm would; for
//   example, Mediabunny's "loaded twice" flag is gone. `__vn*` and `__VN*`
//   names stay. They are the host's and this runtime's process-wide state, this
//   record among them.
interface RealmRecord {
  base: Set<PropertyKey>;
  installed: WeakSet<object>;
  /** How many runtimes this realm has evaluated, this one included. */
  runs: number;
  /** What `original()` recorded: engine and host functions before any runtime replaced them. */
  originals?: WeakMap<object, Map<PropertyKey, unknown>>;
}
const REALM: RealmRecord = ((globalThis as { __VN_REALM__?: RealmRecord }).__VN_REALM__ ??= {
  base: new Set(Reflect.ownKeys(globalThis)),
  installed: new WeakSet(),
  runs: 0,
});
REALM.runs++;

/** Records `v` as a vue-native runtime's install, and hands it back. */
function mark<T>(v: T): T {
  if ((typeof v === "object" && v !== null) || typeof v === "function") REALM.installed.add(v as object);
  return v;
}

/**
 * `obj[name]` as the FIRST runtime in this realm found it, given what it is
 * now. For an engine or host function a runtime wraps or replaces
 * (`document.createElement`, `__vn_set_stylesheet`): the runtime evaluated
 * after a reload must wrap the original, not its predecessor's wrapper, whose
 * state belongs to the old app.
 */
function original<T>(obj: object, name: PropertyKey, now: T): T {
  const table = (REALM.originals ??= new WeakMap());
  let byName = table.get(obj);
  if (!byName) table.set(obj, (byName = new Map()));
  if (!byName.has(name)) byName.set(name, now);
  return byName.get(name) as T;
}

/**
 * Is `obj[name]` missing, or a previous runtime's install? Either way, the
 * runtime evaluated now installs its own. The lookup walks the prototype
 * chain by descriptor, so no getter runs.
 */
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

/**
 * Deletes the configurable globals added since the first runtime started,
 * except `__vn*` / `__VN*`. Called by `__VN_UNLOAD__`, after the teardown, so
 * the runtime and app evaluated next start from the realm the first one
 * found. Returns how many could not be deleted (non-configurable ones).
 */
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
type ListenerOptions = boolean | { capture?: boolean; once?: boolean; passive?: boolean } | undefined;

/** One `addEventListener` registration; `once` is consumed on dispatch. */
interface ExtraListener {
  fn: EventHandler;
  once: boolean;
  /** `{ passive: true }`, or the passive default (`passiveOf`). */
  passive?: boolean;
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
  /** `@wheel.passive`: Vue's `onWheelPassive` prop (`passiveOf`). */
  passiveB?: boolean;
  passiveC?: boolean;
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

/**
 * A NodeList, HTMLCollection or DOMRectList as script sees one: indexed, with
 * `length`, `item()` and iteration (the collection kit, after `TextProto`).
 */
interface MirrorCollection<T> extends ArrayLike<T>, Iterable<T> {
  item(index: number): T | null;
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
  setProperty(key: string, value: string | number | null | undefined, priority?: string): void;
  removeProperty(key: string): string;
  getPropertyValue(key: string): string;
  getPropertyPriority(key: string): string;
  item(i: number): string;
  readonly length: number;
  cssText: string;
  [property: string]: unknown;
}

/** One declaration of an element's inline style, as CSSOM keeps it. */
interface InlineDecl {
  value: string;
  important: boolean;
}

/**
 * The CSS declaration block behind `el.style` AND the `style` attribute. It is
 * one object, so `:style`, `el.style.x = …`, `style.cssText`,
 * `setAttribute("style", …)` and `getAttribute("style")` all read and write
 * the same declarations, and every change reaches the host through
 * `__vn_set_style`, the call `:style` has always made.
 */
interface InlineStyleState {
  /** CSS-spelled name → declaration, in declaration order. A Map keeps an updated key in place, as CSSOM does. */
  decls: Map<string, InlineDecl>;
  /** The `style` attribute exists. It can exist and be empty, as in a browser. */
  present: boolean;
  /** The text `setAttribute("style", …)` wrote, verbatim, until a CSSOM change re-serializes it. */
  raw: string | null;
  /** runtime-dom's `CSS_VAR_TEXT`: what `useCssVars` set, kept across a string `:style`. */
  cssVarText: string;
}

/** The members every mirror node has — elements and text/comment alike. */
interface MirrorNodeBase {
  /**
   * The host node id (`VnNodeId`), under a name of its own: `el.id` is the
   * `id` ATTRIBUTE, as in a browser. Fixed for the life of the host node, and
   * 0 once the node is freed (`forgetDeep`): host ids are recycled, so a
   * reference that outlives its node must not name the next one.
   */
  __vnId: VnNodeId;
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
  /** The parent when it is an Element, else null (DOM §4.4). */
  readonly parentElement: MirrorElement | null;
  readonly firstChild: MirrorNode | null;
  readonly lastChild: MirrorNode | null;
  readonly nextSibling: MirrorNode | null;
  readonly previousSibling: MirrorNode | null;
  /** NonDocumentTypeChildNode: Element and CharacterData both have these. */
  readonly nextElementSibling: MirrorElement | null;
  readonly previousElementSibling: MirrorElement | null;
  hasChildNodes(): boolean;
  getRootNode(): unknown;
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
  /**
   * `<input>`'s value, mirrored so `e.target.value` reads like the browser. An
   * accessor on the prototype (see `writeControlValue`): assigning it on an
   * `<input>` / `<textarea>` / `<select>` reaches the host, as `el.value = x`
   * reaches the widget in a browser.
   */
  value: string;
  /** The `value` accessor's storage. Internal writers (patchProp, the `input` dispatch) use it directly. */
  _value: string;
  /** `checked` once something assigned it; until then it follows the `checked` attribute. */
  _checked?: boolean;
  /** `:true-value` / `:false-value`, kept as given for v-model's checkbox, as runtime-dom keeps them. */
  _trueValue?: unknown;
  _falseValue?: unknown;
  /** v-model's expando while an IME composes, as runtime-dom sets it (`vModelText`). */
  composing?: boolean;
  /** `<option>`: its selectedness and dirtiness (HTML §4.10.10). */
  _selected?: boolean;
  _dirty?: boolean;
  /** `<option>`: the `value` prop as Vue bound it, which `v-model` assigns (runtime-dom's `_value`). Own only once bound. */
  _rawValue?: unknown;
  /** `<select>`: the selected index the host was last told (`showSelected`). */
  _shown?: number;
  /** `<select v-model>`: the model value, and "a change is being assigned" (runtime-dom's `vModelSelect`). */
  _modelValue?: unknown;
  _assigning?: boolean;
  /** `<img>`: the host's answer for `src` (`load` / `error`), which `complete` and `naturalWidth` read. */
  _img?: { src: string; ok: boolean; w: number; h: number };
  /** `on<event>` IDL handlers: the function, and the listener that calls it (`defineHandlerAttr`). */
  _on?: Record<string, { fn: unknown; listener: (ev: VnEvent) => void }>;
  /** Browser-visible attribute state; `patchProp` is the only writer. */
  attrs: Record<string, string>;
  /** The `class` attribute: an accessor over `_class` and `_vtc` (`writeClass`). */
  className: string;
  /** The class Vue or a script assigned, kept apart from `<Transition>`'s `_vtc`. */
  _class: string;
  /** The element's OWN text — exclusive with children, as in the host tree. */
  ownText?: string;
  addEventListener(name: string, fn: EventHandler, opts?: ListenerOptions): void;
  removeEventListener(name: string, fn: EventHandler, opts?: ListenerOptions): void;

  /**
   * The element's namespace when it is not HTML: `<svg>` and everything Vue
   * renders inside one, `createElementNS(svgNS, …)`, `<math>`. Absent = HTML.
   */
  _ns?: "svg" | "mathml";

  // -- the tree, truthfully (the mirror IS the tree)
  /** A live HTMLCollection (`liveCollection`). */
  readonly children: MirrorCollection<MirrorElement>;
  readonly firstElementChild: MirrorElement | null;
  readonly lastElementChild: MirrorElement | null;
  readonly childElementCount: number;
  textContent: string;

  // -- selector matching, through the real cascade matcher
  matches(selector: string): boolean;
  closest(selector: string): MirrorElement | null;
  querySelector(selector: string): MirrorElement | null;
  /** A static NodeList. */
  querySelectorAll(selector: string): MirrorCollection<MirrorElement>;

  // -- layout, off the bounds gpui recorded for the last painted frame
  getBoundingClientRect(): MirrorRect;
  /** A DOMRectList. */
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

  // -- activation (a click, and a file input's picker without one)
  click(): void;
  showPicker(): void;

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
  /** The inline style's declarations (runtime-dom keeps them in the real `el.style`). */
  _inline?: InlineStyleState;
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
  // The ClipboardEvents (crates/vn-tree/src/clipboard.rs): copy/cut/paste in a
  // text field, or the same keys anywhere else, at the focused node or the
  // root. No numbers: `clipboardData` is the dispatcher's eighth argument.
  paste: 39,
  copy: 40,
  cut: 41,
  // m26 fixes. `contextmenu`: the right button's release (after `mouseup`,
  // Chromium on Windows), or the Menu key / Shift+F10 at the focused element.
  // `load`: an `<img>` has its picture (a/b = natural size, `key` = the src;
  // a failure is `error` with the same `key`). `pointercancel`: the host gave
  // up on a press — the window lost activation in the middle of it.
  contextmenu: 42,
  load: 43,
  pointercancel: 44,
  // The document text selection (crates/vn-tree/src/selection.rs, and
  // src/selection.ts for `getSelection()`). `selectstart`: a press that is
  // about to start one, at the press target, cancelable. `selectionchange`:
  // the selection moved; the host delivers it at the ROOT and the dispatcher
  // retargets it to `document`, where a browser fires it.
  selectstart: 45,
  selectionchange: 46,
  // The middle or right button's click (UI Events §3.5): after the release's
  // `mouseup` and, for the right button, before its `contextmenu` (Chromium
  // on Windows). `button` 1 or 2, `detail` the click count.
  auxclick: 47,
  // A text field's edits (crates/vn-elements TextInputState). `beforeinput`
  // comes before each edit and cancels it (the IME's excepted); `a` is the
  // edit's `EDIT_TYPES` code, `b` 1 while an IME composes, the eighth
  // argument the `data` string. `input` (9) carries the same, with the new
  // value in `key`. The composition events carry their `data` the same way.
  beforeinput: 48,
  compositionstart: 49,
  compositionupdate: 50,
  compositionend: 51,
} as const;

/**
 * `InputEvent.inputType` by the host's code: crates/vn-elements
 * `text_input::InputType`, whose discriminants index this table (in lockstep:
 * test/protocol-codes.test.ts reads the Rust enum). 0 is no edit the host
 * named.
 */
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
  // After the click whose `detail` is 2, with the primary button (UI Events
  // §3.5; Chromium fires it from the click's own dispatch). A `@dblclick`
  // therefore arms the click bit, and only the second click of a pair makes
  // one — a triple click does not make a second.
  dblclick: { from: "click", pre: false },
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
  "pointercancel",
]);
/** Bubbles, but cannot be cancelled (Pointer Events §5: pointercancel). */
const NOT_CANCELABLE = new Set<string>(["pointercancel"]);
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
  // Fired at `document`, neither bubbling nor cancelable (Selection API §6).
  "selectionchange",
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
  // …and an image's `load` (its `error` is the one above).
  "load",
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

// ---- loose nodes: the ones only script holds (m26 client sweep #5a) --------
//
// A node the renderer made is held by the renderer (`vnode.el`) or by the tree
// it is in, and `nodesById` holds it strongly until Vue's `remove` frees it.
// A node SCRIPT made (`document.createElement`, `createTextNode`, `new
// DocumentFragment()`) or took out of the tree (`removeChild`, `remove()`,
// `textContent =`) is a different thing: in a browser it lives exactly as long
// as script can reach it, and a strong map would keep it, and its host node,
// forever. So a LOOSE subtree is held weakly here, and once the collector has
// taken its root (a loose subtree dies whole: `parentNode` and `childNodes`
// tie it together) `sweepLoose` frees its host nodes with the renderer's own
// `__vn_remove`.
//
// The host has no "detach" op, and none is needed: a removed subtree is
// `__vn_insert`ed into a container of its own that is never attached (its
// "box"), the way <KeepAlive> parks a deactivated subtree in its storage
// container. It is not painted, and the box goes when the subtree is inserted
// again or freed.
//
// Invariant: a node is loose exactly when its topmost ancestor is in
// `looseRoots`; `nodeOps.insert` moves a subtree between the two registries
// when it changes sides.
const looseById = new Map<VnNodeId, WeakRef<MirrorNode>>();
/** The root of each loose subtree, and the box a removed one is parked in (0: none, it never had a parent). */
const looseRoots = new Map<VnNodeId, { ref: WeakRef<MirrorNode>; box: VnNodeId }>();
/** A mirror node by id: the renderer's, or a loose one script still holds. */
function nodeById(id: VnNodeId): MirrorNode | undefined {
  return nodesById.get(id) ?? (looseById.size ? looseById.get(id)?.deref() : undefined);
}
const isLoose = (n: MirrorNode): boolean => looseById.size !== 0 && looseById.get(n.__vnId)?.deref() === n;
/** `n`'s subtree is now held only by script. */
function loosen(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    if (nodesById.get(d.__vnId) === d) nodesById.delete(d.__vnId);
    looseById.set(d.__vnId, new WeakRef(d));
  });
}
/** `n`'s subtree joined one the renderer or the document holds. */
function tighten(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    looseById.delete(d.__vnId);
    nodesById.set(d.__vnId, d);
  });
}
let sweepQueued = false;
/** Asks for a sweep once the collector has taken a loose root. */
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
/** `n` stops being a loose root (inserted, or freed). Answers the box it was parked in, 0 for none. */
function dropLooseRoot(n: MirrorNode): VnNodeId {
  if (looseRoots.size === 0) return 0;
  const r = looseRoots.get(n.__vnId);
  if (!r || r.ref.deref() !== n) return 0;
  looseRoots.delete(n.__vnId);
  looseFinalizer?.unregister(n);
  return r.box;
}
/** Frees the host nodes of every loose subtree the collector has taken. */
function sweepLoose(): void {
  sweepQueued = false;
  for (const [id, r] of looseRoots) {
    if (r.ref.deref()) continue;
    looseRoots.delete(id);
    __vn_remove(r.box || id);
  }
  for (const [id, ref] of looseById) if (!ref.deref()) looseById.delete(id);
}
/** A node script made: a loose subtree of its own, with no box (it never had a parent). */
function looseCreated<T extends MirrorNode>(n: T): T {
  loosen(n);
  addLooseRoot(n, 0);
  return n;
}
/**
 * Takes `child` out of its parent WITHOUT freeing it, as a browser's removal
 * does: a script may insert it again. Its subtree becomes loose, parked in a box.
 */
function detachNode(child: MirrorNode, notify = true): void {
  const p = child.parentNode;
  if (!p) return;
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
/**
 * Forgets a subtree whose host nodes are being freed: every registry, every
 * handler, and its ids, which become 0. Host ids are recycled, so a reference
 * that outlives its node must not go on naming whatever gets the id next.
 */
function forgetDeep(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    handlers.delete(d);
    if (nodesById.get(d.__vnId) === d) nodesById.delete(d.__vnId);
    else if (looseById.size) looseById.delete(d.__vnId);
    d.__vnId = 0;
  });
}

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

// ---- NodeList, HTMLCollection, DOMRectList ----------------------------------
//
// In a browser `childNodes` and `querySelectorAll()` are NodeLists, `children`
// and `getElementsBy*()` are HTMLCollections, and code tests for them by name:
// motion's `animate()` resolves its subject with `subject instanceof NodeList`
// (framer-motion `resolveSubjects`), a ReferenceError while the global did not
// exist. The prototypes are made here, before the first mirror (`mk` gives
// every node its `childNodes`). The interface objects that own them are made
// with the other interfaces, below (`adoptInterface`).
//
// There are three kinds of list, by what backs them:
// - `childNodes` IS the renderer's own child array (every insert and removal
//   splices it), so it is live by construction. It stays a plain Array on
//   Array.prototype, because JavaScriptCore's fast `indexOf` and element
//   access need the original array prototype chain: on an Array re-parented
//   onto NodeList.prototype, `indexOf` measured 30 times slower, and the
//   renderer calls it for every insert, move and `nextSibling`. Each list
//   carries NodeList's `item` and toString tag as own properties
//   (`childList`), and `instanceof NodeList` accepts it. `forEach`,
//   `entries`, `keys`, `values` and iteration are Array's, which are
//   NodeList's own functions in a browser too. Divergence, said in
//   docs/RUNTIME.md §5.24: `Array.isArray(el.childNodes)` is true, its
//   prototype is Array.prototype, and the other array methods (`map`,
//   `filter`, …) are there too.
// - A STATIC list (`querySelectorAll()`, a MutationRecord's `addedNodes` and
//   `removedNodes`, `getClientRects()`) is a snapshot. It is a plain object on
//   the interface's prototype, with read-only index properties, as a browser's is.
// - A LIVE collection (`children`, `getElementsByTagName()`, …) is a Proxy. It
//   recomputes its members when the tree has changed since it last looked
//   (`domEpoch`), and it has index properties and, for an HTMLCollection,
//   named ones (`form.elements.email`), as WebIDL's legacy platform objects do.

/**
 * Bumped by every change a live collection could see: an insert, a removal,
 * an attribute (`id`, `name` and `class` select members).
 */
let domEpoch = 0;

/** A static list's members. */
const staticItems = new WeakMap<object, readonly unknown[]>();

/** A live collection's state, under both its Proxy and its target. */
interface LiveState {
  compute: () => readonly unknown[];
  items: readonly unknown[];
  /** The `domEpoch` that `items` was computed at. */
  epoch: number;
  /** An HTMLCollection: its elements are also properties, by `id` and `name`. */
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

/** The members of a list, or WebIDL's "Illegal invocation" for any other receiver. */
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

/** An element's id, or (HTML elements only) its `name`, as HTMLCollection's named properties match. */
function namedMember(items: ArrayLike<unknown>, key: string): unknown {
  if (key === "") return null;
  for (let i = 0; i < items.length; i++) {
    const e = items[i] as Partial<MirrorElement> | undefined;
    const a = e?.attrs;
    if (a && (a.id === key || (a.name === key && !e._ns))) return e;
  }
  return null;
}
/** HTMLCollection's supported property names: ids and names, in tree order, each once. */
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

/** `k` as an array index (WebIDL: a canonical numeric string below 2³² − 1), else -1. */
function indexOfKey(k: string): number {
  const c = k.charCodeAt(0);
  if (!(c >= 48 && c <= 57)) return -1;
  const n = Number(k);
  return n <= 4294967294 && String(n) === k ? n : -1;
}

/**
 * WebIDL's legacy platform object, over a live member list: index properties
 * are read-only and enumerable, named ones (an HTMLCollection's) read-only
 * and not enumerable, and a name the prototype chain already has (`item`,
 * `length`) is the prototype's.
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
        return i < items.length ? { value: items[i], writable: false, enumerable: true, configurable: true } : undefined;
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

/** A live collection on `proto`, whose members are `compute()`'s answer for the current tree. */
function liveCollection<T>(proto: object, named: boolean, compute: () => readonly T[]): MirrorCollection<T> {
  const target = Object.create(proto) as object;
  const s: LiveState = { compute, items: [], epoch: -1, named };
  const list = new Proxy(target, LIVE_HANDLER);
  liveStates.set(target, s);
  liveStates.set(list, s);
  return list as MirrorCollection<T>;
}

/** A static list on `proto`: a snapshot of `items`, which must not change afterwards. */
function staticList<T>(proto: object, items: readonly T[]): MirrorCollection<T> {
  const list = Object.create(proto) as Record<number, unknown>;
  for (let i = 0; i < items.length; i++) {
    Object.defineProperty(list, i, { value: items[i], enumerable: true, configurable: true });
  }
  staticItems.set(list, items);
  return list as unknown as MirrorCollection<T>;
}

/**
 * A collection interface's prototype, with WebIDL's members on it (enumerable,
 * as WebIDL's are). The iteration methods are the Array ones themselves, as
 * in a browser (`NodeList.prototype.forEach === Array.prototype.forEach`).
 */
function collectionProto(name: string, opts: { named?: boolean; iterable?: boolean } = {}): object {
  const proto: object = Object.create(Object.prototype);
  const required = (member: string, n: number) => {
    if (n < 1) throw new TypeError(`Failed to execute '${member}' on '${name}': 1 argument required, but only 0 present.`);
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
    Object.defineProperty(proto, k, { ...d, enumerable: true, configurable: true, ...("value" in d ? { writable: true } : {}) });
  }
  Object.defineProperty(proto, Symbol.iterator, { value: Array.prototype.values, writable: true, configurable: true });
  Object.defineProperty(proto, Symbol.toStringTag, { value: name, configurable: true });
  return proto;
}
const NodeListProto = collectionProto("NodeList", { iterable: true });
const HTMLCollectionProto = collectionProto("HTMLCollection", { named: true });
const DOMRectListProto = collectionProto("DOMRectList");

// What each `childNodes` array carries of NodeList's own (see above): its
// `item`, the very function on NodeList.prototype, and its toString tag, so
// `Object.prototype.toString` says "[object NodeList]" (and Vue's `reactive()`
// leaves the list alone, as it does a browser's).
const CHILD_ITEM: PropertyDescriptor = {
  value: (NodeListProto as { item: unknown }).item,
  writable: true,
  configurable: true,
};
const CHILD_TAG: PropertyDescriptor = { value: "NodeList", configurable: true };
/** A new, empty `childNodes`. */
function childList(): MirrorNode[] {
  const list: MirrorNode[] = [];
  Object.defineProperty(list, "item", CHILD_ITEM);
  Object.defineProperty(list, Symbol.toStringTag, CHILD_TAG);
  return list;
}
/** Is `x` a node's `childNodes` (`childList`)? */
function isChildList(x: object): x is MirrorNode[] {
  return Array.isArray(x) && (x as { item?: unknown }).item === CHILD_ITEM.value;
}

/** The empty answer, shared: a static list can never change. */
const NO_NODES: readonly MirrorNode[] = Object.freeze([]);

/**
 * Objects made here for interfaces this runtime has no class for (a
 * `classList`, a MutationRecord, …), by interface. Each interface's
 * `instanceof` answers from its set (the interface objects are made below,
 * with the others).
 */
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

/** The element namespaces (`namespaceURI`). */
const HTML_NS = "http://www.w3.org/1999/xhtml";
const SVG_NS = "http://www.w3.org/2000/svg";
const MATHML_NS = "http://www.w3.org/1998/Math/MathML";

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
  n.__vnId = id;
  n.kind = kind;
  n.tag = tag;
  n.parentNode = null;
  // A live NodeList, because it is the list the renderer splices (`childList`).
  n.childNodes = childList();
  if (kind === 1) {
    n.tagName = String(tag).toUpperCase();
    // The storage under the prototype's `value` accessor; its being OWN is
    // also what tells a mirror element from a detached engine one.
    n._value = "";
    // Browser-visible attribute state. `patchProp` is the only writer that Vue
    // drives; reka-ui's roving focus and its Collection read `data-*`/`aria-*`
    // back off candidate nodes, so the mirror has to remember what was written
    // rather than answering "absent" for everything.
    n.attrs = Object.create(null);
    // The `class` Vue (or a script) assigned, kept separately from the
    // transition classes `<Transition>` layers on top (runtime-dom keeps the
    // same split, for the same reason: a re-render must not wipe an in-flight
    // `v-enter-active`). `className` is the prototype's accessor over both.
    n._class = "";
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

// node -> { click: fn }. Keyed by the NODE, weakly: a loose node script dropped
// must not be kept alive by its own listeners (`addEventListener("load", () =>
// use(img))` closes over the node it is registered on).
const handlers = new WeakMap<MirrorNode, HandlerMap>();

// Typed by Vue's own `RendererOptions`, minus the `patchProp` that is defined
// separately below: every hook's parameter types come from the renderer that
// calls them, so nothing here can drift out of step with what Vue passes.
const nodeOps: Omit<RendererOptions<MirrorNode, MirrorElement>, "patchProp"> = {
  createElement: (type, namespace, _is, props) => {
    const el = mk(__vn_create_element(I(type)), 1, type);
    // `<VnWindow>`'s root: registered before its content mounts (vnwindow.ts).
    if (props && (props as Record<symbol, unknown>)[WINDOW_ROOT]) windowRoots.add(el);
    // `<svg>` and what Vue renders inside it, and `<math>`: not HTML elements.
    // Their `tagName` keeps the name's case (`svg`, `linearGradient`), as a
    // browser's does, and `instanceof SVGElement` / `MathMLElement` hold
    // (`namespaceURI`, `wrapElementHasInstance`).
    if (namespace === "svg" || namespace === "mathml") {
      el._ns = namespace;
      el.tagName = type;
    }
    // A `<label>` clicks its control whether or not anything listens, so the
    // host must deliver its clicks (desktop-apis-design §4.2). So does a
    // `<button>` (submit is its default type: it submits its form), and an
    // `<input>` hears Enter for implicit submission (text is its default).
    if (type === "label" || type === "button") syncListenerFlag(el, "click");
    if (type === "input") syncListenerFlag(el, "keydown");
    // A text control hears its own `input`: that event is what carries the
    // typed text into the mirror's `value`, so `el.value` reads what the user
    // typed with no `@input` anywhere.
    if (type === "input" || type === "textarea") syncListenerFlag(el, "input");
    if (type === "canvas") adoptCanvas(el);
    // `<select>` / `<option>`: the selection model's members (`selectedIndex`,
    // `options`, `option.selected`…).
    if (!namespace && type === "select") defineSelectMembers(el);
    else if (!namespace && type === "option") defineOptionMembers(el);
    return el;
  },
  createText: (text) => {
    const n = mk(__vn_create_text(), 2, "#text");
    // The mirror keeps the string too — `textContent` and `Node.data` read it
    // back, and a host round trip for that would be absurd.
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
    // The children are DETACHED, not freed, as `el.textContent = text` does in
    // a browser. Normally there are none (Vue unmounts them first), but a
    // leaving <Transition> child is still here until its leave ends, and Vue
    // removes it then; one a script appended is still the script's. Freeing
    // either here left a mirror over a host id that was recycled under it.
    const kids = node.childNodes.length ? node.childNodes.slice() : null;
    if (kids) for (const c of kids) detachNode(c, false);
    // The element's OWN text (exclusive with children, exactly as in the host
    // tree) — `textContent` reads it back.
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
    // Already freed (its id is 0 now): nothing to free.
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
  // <style scoped>: Vue stamps every element a scoped component renders (and
  // child component roots) with its data-v id — the Rust matcher buckets on it.
  setScopeId: (el, id) => __vn_add_scope(el.__vnId, id),
  // Vue's `<Teleport>` resolves a STRING `to` through this hook. Without it
  // `<Teleport to="body">` — reka-ui's default for EVERY portal — warns
  // "Current renderer does not support string target" and renders nothing.
  //
  // The document roots a browser component reaches for all mean "the app root"
  // here; anything else goes through the real matcher. A Teleport inside a
  // `<VnWindow>` asks through `windowTeleport` instead, where they mean that
  // window's root.
  querySelector: (sel) => {
    if (DOCUMENT_ROOTS.has(sel)) return appRoot;
    return queryOne(sel, appRoot ? appRoot.__vnId : 0);
  },
};

/** The selectors that name the document itself, as a Teleport target. */
const DOCUMENT_ROOTS = new Set(["body", "html", ":root", "#app"]);

/**
 * The `<VnWindow>` root `node` paints under, or `null` for the main window.
 *
 * A pop-out's content stays in this ONE tree (MULTIWINDOW.md §1), so nothing
 * in the tree's shape says which window a node is in: its nearest registered
 * window root does (vnwindow.ts `windowRoots`). Free without a pop-out.
 */
function windowRootOf(node: MirrorNode | null | undefined): MirrorElement | null {
  if (windowRoots.size === 0) return null;
  for (let n = node; n; n = n.parentNode) if (windowRoots.has(n)) return n as MirrorElement;
  return null;
}

/** What `mountingWindowRoot` reads of a component instance. */
interface MountingInstance {
  type: unknown;
  parent: MountingInstance | null;
  subTree?: { el?: unknown } | null;
}

/**
 * The `<VnWindow>` root that `container`, NOT in the document yet, will paint
 * under — or `null`.
 *
 * Vue mounts an element's children before it inserts the element, so content
 * that mounts with its window (or with any new element in it) hands a
 * `<Teleport>` a container whose parents stop short of the window root, and
 * `windowRootOf` cannot see the window. The components can: the nearest
 * `<VnWindow>` above the Teleport's, whose root element exists before its
 * content mounts. A container that IS in the document and under no window root
 * is in the main window, whatever its components are.
 */
function mountingWindowRoot(container: MirrorNode | null | undefined, parent: MountingInstance | null | undefined): MirrorElement | null {
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
 * `<Teleport to="body">` inside a `<VnWindow>` lands in THAT window.
 *
 * In a browser, content in a popup is in the popup's document, and a portal
 * that teleports to "body" from there (`document.body` of the component's own
 * document) stays in the popup. Here a pop-out's subtree is part of the main
 * tree, so the renderer's `querySelector("body")` alone would carry every
 * reka-ui menu, tooltip and dialog opened in a pop-out back to the MAIN
 * window. Teleport resolves a string `to` through the renderer options it is
 * handed (`internals.o.querySelector`), at mount, on a `to` change and in a
 * deferred mount; this hands it options whose document roots are the window
 * root its own placeholder sits under (or will, once the content it mounts
 * with is inserted: `mountingWindowRoot`). A `to` that is a selector for
 * anything else, or an element, is untouched.
 */
const TELEPORT_ORIGINAL = Symbol.for("vn.teleport.original");
type TeleportProcess = ((this: unknown, ...args: unknown[]) => unknown) & { [TELEPORT_ORIGINAL]?: TeleportProcess };
interface TeleportInternals {
  o: { querySelector?: (sel: string) => unknown };
}
{
  const impl = Teleport as unknown as { process: TeleportProcess };
  // `location.reload()` evaluates this module again (see the top of this
  // file), maybe over the same Teleport: the wrapper installed now wraps Vue's
  // own `process`, never a predecessor's wrapper, whose window registry is the
  // old app's.
  let process = impl.process;
  while (process[TELEPORT_ORIGINAL]) process = process[TELEPORT_ORIGINAL];
  const wrapped: TeleportProcess = function (this: unknown, ...args: unknown[]) {
    // process(n1, n2, container, anchor, parentComponent, parentSuspense,
    //         namespace, slotScopeIds, optimized, internals)
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
 * The document a node in a `<VnWindow>` belongs to: its `ownerDocument`.
 *
 * A browser popup's nodes are in the POPUP's document, whose root element is
 * the popup's viewport — so `el.ownerDocument.documentElement.clientWidth` is
 * the popup's width, which is how floating-ui (and with it every reka-ui
 * popper: a context menu, a tooltip) measures the viewport it keeps a menu
 * inside. One realm and one `window` stay one (D-M01): this document is the
 * real one with its root elements (`documentElement`, `body`,
 * `scrollingElement`) answered by the window's root, and everything else —
 * listeners, `activeElement`, `createElement`, queries — the real document's,
 * so a listener added through it hears the same events. Like a popup's
 * document, it is not `=== document`.
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
      // A method answers for the real document (its `this` is a native
      // receiver), and the same one on every read.
      let b = bound.get(v);
      if (!b) bound.set(v, (b = (v as (...a: unknown[]) => unknown).bind(target)));
      return b;
    },
  });
  WINDOW_DOCUMENTS.set(root, doc);
  return doc;
}

/**
 * The renderer's insert, which is also a move, and the DOM methods' too.
 * `notifyAdd: false` leaves the addition record to the caller (one record for
 * a whole fragment); the old parent always hears the removal, as DOM "insert"
 * removes the node from its parent first.
 */
function insertNode(child: MirrorNode, parent: MirrorElement, anchor: MirrorNode | null, notifyAdd: boolean): void {
  const old = child.parentNode;
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
  // The host drops an element's own text when it gains a child; so does the mirror.
  if (parent.ownText) parent.ownText = "";
  __vn_insert(child.__vnId, parent.__vnId, anchor ? anchor.__vnId : 0);
  if (looseById.size) {
    // A loose root that was parked: it left its box, which goes now.
    const box = dropLooseRoot(child);
    if (box) __vn_remove(box);
    const now = isLoose(parent);
    if (now !== isLoose(child)) (now ? loosen : tighten)(child);
  }
  // A select's list of options changed: its selection is settled again.
  if (old && old !== parent) optionsChangedIn(old);
  optionsChangedIn(parent);
  if (old) notifyMutation(old, "childList", { addedNodes: [], removedNodes: [child] });
  if (notifyAdd) notifyMutation(parent, "childList", { addedNodes: [child], removedNodes: [] });
}

// @vue/shared's hyphenate, verbatim: `\B` so a leading capital is only
// lowercased, never prefixed. Style keys are camelCase either way, but
// `withKeys` runs it over `KeyboardEvent.key` — "Enter" must become "enter",
// not "-enter", or every `@keydown.enter` silently stops matching.
const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

// ---- the inline style: one CSSOM declaration block per element -------------
//
// `el.style`, `:style`, `style.cssText` and the `style` attribute are four views
// of ONE thing in a browser, and here they are too (`InlineStyleState`). The
// host keeps the same list (`Tree::set_style`: an updated key stays in place, a
// new one is appended), so the mirror's declaration order is the order the
// engine cascades them in.

/**
 * A `:style` key or a `CSSStyleDeclaration` attribute name, as CSS spells the
 * property: `backgroundColor` and `background-color` are `background-color`,
 * `webkitTransform` / `WebkitTransform` are `-webkit-transform`, `cssFloat` is
 * `float`. Custom properties are CASE-SENSITIVE and stay as written, so
 * `--rekaPopperWidth` is not renamed.
 */
function cssPropName(k: string): string {
  if (k.startsWith("--")) return k;
  if (k === "cssFloat") return "float";
  if (k.includes("-")) return k.toLowerCase();
  const h = hyphenate(k);
  return /^(webkit|moz|ms)-/.test(h) ? "-" + h : h;
}
/** `setProperty`/`getPropertyValue`'s argument: already a CSS name, ASCII-lowercased unless custom. */
const cssomName = (k: string): string => (k.startsWith("--") ? k : k.toLowerCase());
const IMPORTANT_RE = /\s*!\s*important\s*$/i;
/** A CSS property name, or a custom property (`--*`). "0" is not one. */
const PROP_NAME_RE = /^(?:--[^\s:;]+|-?[A-Za-z_\u0080-￿][\w\u0080-￿-]*)$/;

/**
 * A declaration's value as stored: trimmed, as CSS Syntax trims it. An EMPTY
 * custom property is valid CSS (`--x: ;`, and `normalizeCssVarValue` turns ""
 * into " " to keep one), but "" is "remove" on the host channel, so an empty
 * custom property is kept as one space.
 */
const inlineValue = (v: string, custom: boolean): string => {
  const t = v.trim();
  return t === "" && custom ? " " : t;
};

function inlineOf(el: MirrorElement): InlineStyleState {
  return (el._inline ??= { decls: new Map(), present: false, raw: null, cssVarText: "" });
}
/** What the host stores for one declaration: the value, with its priority. */
const hostDecl = (d: InlineDecl): string => (d.important ? `${d.value} !important` : d.value);
/** CSSOM "serialize a CSS declaration block": `prop: value;`, joined by one space. */
function serializeInline(st: InlineStyleState | undefined): string {
  let out = "";
  if (st) {
    for (const [name, d] of st.decls) {
      out += `${out ? " " : ""}${name}: ${d.value}${d.important ? " !important" : ""};`;
    }
  }
  return out;
}
/**
 * The serialization is from the declarations as written. A browser re-spells
 * each value (`#f00` → `rgb(255, 0, 0)`, `0` → `0px`), folds longhands into
 * shorthands and drops a value it rejects; this has no parser of its own on
 * the JS side, so it does none of that. Said once, at the first read.
 */
function noteInlineSerialization(): void {
  warnCompat(
    "inline-style-serialize",
    "el.style.cssText / getAttribute('style') serialize the inline declarations as they were written: " +
      "values are not re-spelled (`#f00` stays `#f00`), a shorthand is not expanded into its longhands " +
      "(after `margin: 0`, `el.style.marginTop` reads \"\"), and a value the engine rejects is still listed",
  );
}
/** `getAttribute("style")`: the attribute's text, or null when there is no attribute. */
function styleAttribute(el: MirrorElement): string | null {
  const st = el._inline;
  if (!st || !st.present) return null;
  if (st.raw !== null) return st.raw;
  const text = serializeInline(st);
  if (text) noteInlineSerialization();
  return text;
}
/** The attribute's old value, for a MutationObserver that asked for one; nothing is serialized otherwise. */
const styleBefore = (el: MirrorElement): string | null => (observers.size ? styleAttribute(el) : null);
/** "Update style attribute": the attribute now exists, and its text is the serialization again. */
function styleTouched(el: MirrorElement, st: InlineStyleState, before: string | null): void {
  st.present = true;
  st.raw = null;
  if (el.attrs) el.attrs.style = "";
  notifyMutation(el, "attributes", { attributeName: "style", oldValue: before });
}

/**
 * CSSOM "set a CSS declaration", then "update style attribute" when it changed.
 * An empty value removes the declaration (`setProperty`'s own rule). Answers
 * whether anything changed.
 */
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
/** `removeProperty`: the old value, or null when there was no such declaration. */
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

/**
 * CSS's "parse a list of declarations" for an inline body. A `;` ends a
 * declaration only outside a string, a comment and `()`/`[]`/`{}`, so
 * `background: url("data:image/png;base64,…")` is one declaration. A
 * declaration without a property name (`0: d`) or without a value is dropped,
 * as a browser drops it. A repeated property keeps its LAST occurrence, at that
 * position, except that a normal one does not override an earlier
 * `!important` one.
 */
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
      // CSS Syntax "consume a declaration" trims the value at both ends.
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
 * `style.cssText = text` (`asAttribute` false) or `setAttribute("style", text)`
 * (true, which also keeps `text` verbatim as the attribute's value): the block
 * is emptied and refilled from `text`. A declaration that stays with the same
 * value costs no host call, as long as the host's order (the surviving keys in
 * their old order, then the new ones) is the new block's order; otherwise the
 * block is re-sent from scratch, because the order decides a shorthand against
 * its longhand.
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

// ---- `:style`: runtime-dom's `patchStyle`, over the declaration block -------

/** runtime-dom's `vShowOriginalDisplay` / `vShowHidden`: what `v-show` restores, and whether it is hiding. */
const vShowOriginalDisplay = Symbol("_vod");
const vShowHidden = Symbol("_vsh");
/** runtime-dom's `displayRE` and `importantRE`, verbatim. */
const DISPLAY_RE = /(^|;)\s*display\s*:/;
const RD_IMPORTANT_RE = /\s*!important$/;

/**
 * `:style`, as runtime-dom's `patchStyle` does it, line for line:
 *
 * - a STRING is the whole block (`style.cssText = next`), with the variables
 *   `useCssVars` set appended so a string style does not wipe them;
 * - a RECORD is diffed per property. A key the new record lacks (or holds
 *   `null`/`undefined` in) is removed, and that includes a key a previous
 *   STRING declared, which is how a record replacing a string clears it;
 * - `null`/`undefined` after a style removes the attribute and every
 *   declaration with it;
 * - under `v-show`, a `display` the style controls becomes the one it shows,
 *   and a hidden element stays `display: none`.
 */
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

/**
 * runtime-dom's `setStyle`: one `:style` entry. A custom property goes
 * through `setProperty`; a value ending in `!important` is set with that
 * priority; anything else is the attribute setter. `null`/`undefined` (and
 * `""`) remove the property. An ARRAY is the vendor-fallback form.
 */
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

/**
 * `{ display: ["-webkit-box", "-ms-flexbox", "flex"] }`. runtime-dom sets each
 * entry in turn: the browser keeps a value it accepts, ignores one it does
 * not, and an empty entry removes the property. So the outcome is the LAST
 * entry that is empty or accepted, and with none of those the declaration is
 * left as it was. "Accepted" is the engine's answer (`__vn_style_accepts`),
 * because only the engine knows what it applies.
 */
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

/** Does the engine apply `prop: value`? Custom properties take any value. */
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

/** `removeAttribute("style")`: the declarations go with the attribute. */
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

// -- Reflected IDL attributes (m26 client sweep #10, #32)
//
// A browser element carries most of its content attributes twice: as
// attributes, and as typed DOM properties that reflect them (`el.hidden`,
// `input.placeholder`, `el.tabIndex`). This table is the subset vue-native
// reflects: which property, on which tags (`null`: every HTML element), as
// which attribute, and of which IDL type. It drives two things:
// `attributeOf` (what runtime-dom's `patchProp` leaves in the attribute when it
// assigns the property) and the accessors on the element prototype
// (`defineReflected`), which used to be missing: `input.placeholder` read
// `undefined` while `getAttribute("placeholder")` worked.
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
    const [prop, attr = prop.toLowerCase()] = n.split("=");
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
reflect("string", "accept min max step pattern", "input");
reflect("string", "htmlFor=for", "label output");
reflect("long", "tabIndex", null);
reflect("long", "maxLength minLength", "input textarea");

/** The reflected property `key` names on this element, if it has one. */
function reflectedOf(el: MirrorElement, key: string): Reflected | null {
  const r = REFLECTED[key];
  return r && (!r.tags || r.tags.has(el.tag)) ? r : null;
}

/** WebIDL `long`: ToNumber, then truncated into 32 bits; NaN and the infinities are 0. */
function toLong(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) ? Math.trunc(n) | 0 : 0;
}

/**
 * A listener key, by `@vue/shared`'s `isOn`: `on` and then anything but a
 * lowercase letter (`onClick`, `on:loopComplete`). `once`, `online` and
 * `only` are attributes, as in runtime-dom.
 */
function isOn(key: string): boolean {
  const c = key.charCodeAt(2);
  return key.charCodeAt(0) === 111 && key.charCodeAt(1) === 110 && (c > 122 || c < 97);
}

/**
 * The renderer's single prop hook — so `prev`/`next` really are `unknown`: a
 * template can bind any value at all. `namespace` is what Vue passes for an
 * element inside `<svg>` / `<math>`, where every key is an attribute.
 * (`setAttribute` / `removeAttribute` write the attribute itself: see
 * `writeAttribute`.)
 */
function patchProp(el: MirrorElement, key: string, prev: unknown, next: unknown, namespace?: unknown): void {
  if (key === "style") {
    // A string is `cssText`, a record is diffed, `null` clears: runtime-dom's
    // `patchStyle`. A null/undefined VALUE in a record clears its property
    // (`:style="{ transform: maybeUndefined }"` is what @floating-ui/vue's
    // `floatingStyles` produces before its first measurement).
    patchStyle(el, prev, next);
  } else if (key === "class") {
    // runtime-dom's `patchClass`: Vue's class, with the transition classes
    // merged on top; `null` removes the attribute.
    if (prev !== next) {
      el._class = next == null ? "" : String(next);
      writeClass(el, next == null);
    }
  } else if (isOn(key)) {
    // runtime-dom's `parseName`, same order: strip the option suffixes, then
    // read the name.
    //   onClick / onClickCapture / onClickOnce / onClickCaptureOnce
    //   on:loopComplete — the case-PRESERVING spelling compiler-dom emits for
    //                     a custom element's camelCase event (`@loopComplete`
    //                     on <lottie>). Everything else is hyphenated.
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
        // `@wheel.passive`: before this, "wheel-passive", an event nothing
        // fires, so the handler never ran.
        passive = true;
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
    if (!code && RUNTIME_FIRED_EVENTS.has(name)) {
      // `submit` / `reset`: fired by the runtime's form submission.
    } else if (!code && JS_SOURCED_EVENTS.has(name)) {
      // `change` / `cancel`: the runtime fires them itself on a file input
      // (and they bubble to whatever wraps one), so the handler is kept.
      if (next) adviseJsSourced(el, name, `@${name}`);
    } else if (!code) {
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
    let m = handlers.get(el);
    if (!m) {
      m = {};
      handlers.set(el, m);
    }
    let buckets = ensureBuckets(m, name);
    // runtime-dom hands `{ passive: true }` to `addEventListener`, so a prop
    // with no `.passive` gets the browser's passive default as well.
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
    // stripped by the vnode layer; never reaches here — but be safe
  } else if (key === "textContent" && !foreignNamespace(namespace)) {
    // `v-text` compiles to this DOM property; runtime-dom assigns it, and
    // ignores a null.
    if (next != null) (el as { textContent: unknown }).textContent = next;
  } else if (key === "value" && (isSelectEl(el) || isOptionEl(el)) && !foreignNamespace(namespace)) {
    patchSelectValue(el, next);
  } else if (key === "selected" && isOptionEl(el) && !foreignNamespace(namespace)) {
    // runtime-dom assigns the PROPERTY: the option's selectedness, and its
    // dirtiness (the attribute is written as well, as it always was here).
    patchAttribute(el, ...attributeOf(el, key, next, namespace));
    setOptionSelected(el, next === "" || !!next);
  } else {
    // runtime-dom keeps these two as properties for v-model's checkbox.
    if (key === "true-value") el._trueValue = next;
    else if (key === "false-value") el._falseValue = next;
    // `:checked` assigns the PROPERTY (and, below, the boolean attribute).
    else if (key === "checked" && el.tag === "input" && !foreignNamespace(namespace)) el._checked = next === "" || !!next;
    patchAttribute(el, ...attributeOf(el, key, next, namespace));
  }
}

/** `<svg>` / `<math>` content: runtime-dom's `shouldSetAsProp` is false for (nearly) every key there. */
const foreignNamespace = (ns: unknown): boolean => ns === "svg" || ns === "mathml";

/**
 * runtime-dom's `patchProp` for a key that is not style, class or a listener,
 * reduced to the one thing that reaches this host: the attribute it leaves.
 * Where the element has a DOM property of that name (`shouldSetAsProp`), a
 * browser assigns the PROPERTY, and the attribute is what the property
 * reflects, which is not always the value written:
 *
 * - a boolean property (`hidden`, `disabled`, `open`…) is present as `""` or
 *   absent, so `:hidden="false"` REMOVES the attribute. Writing the string
 *   "false" made it present: Tailwind's `[hidden]` rule then hid every reka
 *   `TabsContent`, and `[disabled]` matched enabled controls (m26 #10);
 * - so is `checked` / `selected`, which runtime-dom also writes as a boolean
 *   attribute, and the attribute-only booleans of `isSpecialBooleanAttr`
 *   (`readonly`, `novalidate`…), on any element;
 * - a camelCase property names a different attribute (`tabIndex` → tabindex,
 *   `htmlFor` → for), and a `long` one is written as an integer.
 *
 * Everything else is the attribute verbatim, which is also what a string
 * property reflects. Returns `[attribute, value]`; `null` removes.
 */
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

/** runtime-dom's `isSpecialBooleanAttr`: boolean attributes no DOM property of the same name reflects. */
const SPECIAL_BOOLEAN_ATTRS = new Set(["itemscope", "allowfullscreen", "formnovalidate", "ismap", "nomodule", "novalidate", "readonly"]);

/**
 * Writes one attribute, verbatim: the host's string channel (its native
 * widgets read what they understand; `null` clears), the mirror's `attrs`,
 * and the bookkeeping some attributes carry. `setAttribute` lands here, so
 * `setAttribute("hidden", "false")` keeps the attribute, as it does in a
 * browser.
 */
function patchAttribute(el: MirrorElement, key: string, next: unknown): void {
  // DOM-compat attribute channel: standard-ish props flow to the host as
  // strings (native widgets read what they understand); null clears.
  if (next == null) __vn_set_prop(el.__vnId, I(key), undefined);
  else if (typeof next === "string" || typeof next === "number" || typeof next === "boolean")
    __vn_set_prop(el.__vnId, I(key), String(next));
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
  // The host has the value already (the generic channel, above); only the
  // mirror's storage follows, so the accessor does not push it twice.
  if (key === "value") el._value = next == null ? "" : String(next);
  // An option's `selected` attribute is its default selectedness: it selects
  // (or deselects) an option nothing has selected by hand yet (HTML §4.10.10).
  // A select's `multiple` and `size` change what its selection may be.
  if (key === "selected" && isOptionEl(el) && !el._dirty) {
    el._selected = next != null;
    selectednessChanged(el);
  }
  if ((key === "multiple" || key === "size") && isSelectEl(el)) {
    resetSelect(el);
    adviseListBox(el);
  }
  // (`checked` is the other way round: the attribute is `defaultChecked`, and
  // the `checked` getter follows it until something assigns the property.)
  // An `<a href>` has an ACTIVATION BEHAVIOUR whether or not anything
  // listens: following the link is what a click on it means. The host only
  // delivers a click to a node whose click bit is on, so the bit follows the
  // href (`syncListenerFlag` ORs it in). docs/SHELL.md §1.
  if (key === "href" && el.tag === "a") syncListenerFlag(el, "click");
  // …and so does `<input type=file>`'s, which also takes Enter and Space.
  if (key === "type" && el.tag === "input") syncFileInput(el);
  // …and a submit button's and a text field's (forms, below): the type
  // decides whether a click submits and whether Enter does.
  if (key === "type" && (el.tag === "input" || el.tag === "button")) {
    syncListenerFlag(el, "click");
    if (el.tag === "input") syncListenerFlag(el, "keydown");
  }
  // F-4 (docs/RUNTIME.md §5.2): the runtime honours a disabled fieldset;
  // the host's native widgets read only their own `disabled`.
  if (key === "disabled" && el.tag === "fieldset" && next != null && next !== false && next !== "false") {
    warnCompat(
      "fieldset:disabled",
      "<fieldset disabled>: clicks, focus() and form submission treat the controls inside as disabled, " +
        "as a browser does, but the native <input>s inside still accept typing; disable each one as well",
    );
  }
}

/** `setAttribute` / `removeAttribute`: `class` and `on*` keep their own paths, everything else is verbatim. */
function writeAttribute(el: MirrorElement, key: string, value: string | null): void {
  if (key === "class") {
    if (value != null) return setClassText(el, value);
    el._class = "";
    el._vtc = undefined;
    writeClass(el, true);
  } else if (key.startsWith("on") && key.length > 2) patchProp(el, key, undefined, value);
  else patchAttribute(el, key, value);
}

/// The element's class attribute = what Vue assigned + whatever `<Transition>`
/// has layered on top. runtime-dom does exactly this (its `patchClass` merges
/// `el._vtc`); without it a re-render during a transition drops the phase class
/// and the animation stops halfway.
///
/// The merged value is also the ATTRIBUTE (m26 client sweep #19): it is what
/// `getAttribute("class")`, `className` and `classList` read, and the host
/// keeps it for `[class*=…]` selectors. A change is an `attributes` mutation.
/// `remove`: the attribute goes away when nothing is left (`:class="null"`,
/// `removeAttribute("class")`); otherwise an empty class stays present.
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
 * The class attribute becomes `text`, as `el.className = text`,
 * `setAttribute("class", text)` and `classList` make it in a browser. A
 * transition class `text` still names stays a transition class (so a
 * read-modify-write does not bake `v-enter-active` in for good); one it drops
 * is dropped, as removing it from a browser's attribute drops it.
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

/// Several event NAMES can share one host code (`pointerdown` rides
/// `mousedown`), so the flag is the OR over every name that maps to it —
/// switching a `pointerdown` listener off must not silence a `mousedown` one.
/// A custom event name has no host code at all and needs no flag: it is
/// dispatched from JS.
function syncListenerFlag(self: MirrorNode, name: string): void {
  const code = hostCode(name);
  if (!code) return;
  const id = self.__vnId;
  const m = handlers.get(self);
  let on = NAMES_BY_CODE[code].some((n) => m?.[n] && bucketsAlive(m[n]!));
  // Document- and window-level listeners ride the APP ROOT's bit: the host
  // delivers events to nodes, and the root is the node that stands in for the
  // document. See `globalHandlers` below.
  if (!on && appRoot && id === appRoot.__vnId) {
    on = NAMES_BY_CODE[code].some((n) =>
      [...globalHandlers.values()].some((g) => g[n] && bucketsAlive(g[n]!)),
    );
  }
  // …and a link listens for its own click (its activation behaviour), so
  // removing the last `@click` from an `<a href>` must not switch it off. So
  // do a `<label>` and an `<input type=file>`, which also hears Enter (keydown)
  // and Space (keyup) — the keys that click it.
  // A submit or reset button submits or resets its form, and a text-like
  // `<input>` submits its form on Enter (implicit submission).
  if (!on && code === EV_CODES.click) {
    const n = self;
    if (n && (isLabel(n) || isFileInput(n) || isLink(n) || isSubmitButton(n) || isResetButton(n))) on = true;
  }
  if (!on && (code === EV_CODES.keydown || code === EV_CODES.keyup)) {
    const n = self;
    if (isFileInput(n) || (code === EV_CODES.keydown && blocksImplicitSubmission(n))) on = true;
  }
  // …and a text control hears its own `input`, which is how its `value` keeps
  // up with what the user types (the dispatcher writes it before propagating).
  if (!on && code === EV_CODES.input) {
    const n = self;
    if (n && n.kind === 1 && (n.tag === "input" || n.tag === "textarea")) on = true;
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
  // (Only a live mirror carries the cache; a stand-in the tests pass does not.)
  const node = isMirrorNode(self) ? self : null;
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
  typeof (n as MirrorNode).__vnId === "number" &&
  nodeById((n as MirrorNode).__vnId) === (n as MirrorNode);

/** The two option spellings, as an object. `true` means `{ capture: true }`. */
type ListenerOptionsObject = { capture?: boolean; once?: boolean; passive?: boolean };

/** The events whose listeners on `window`, `document` and `<body>` are passive unless they say otherwise. */
const PASSIVE_BY_DEFAULT = new Set(["wheel", "mousewheel", "touchstart", "touchmove"]);

/**
 * Is this listener passive? A passive listener's `preventDefault()` does
 * nothing (DOM §2.10). An explicit `passive` decides. Without one, a `wheel`,
 * `mousewheel`, `touchstart` or `touchmove` listener on `window`, `document`
 * or `<body>` (the root here) is passive. That is the DOM's "default passive
 * value", which every engine applies, so that a page-level listener cannot
 * hold up scrolling. Before the host honoured a cancelled `wheel`
 * (docs/RUNTIME.md §5.8), nothing could tell the difference.
 */
function passiveOf(opts: ListenerOptions | undefined, target: object, name: string): boolean {
  if (opts && typeof opts === "object" && typeof (opts as ListenerOptionsObject).passive === "boolean")
    return (opts as ListenerOptionsObject).passive!;
  if (!PASSIVE_BY_DEFAULT.has(name)) return false;
  return target === globalThis || target === globalThis.document || (appRoot != null && target === appRoot);
}

/** Host event names a window/document listener must hear from the engine too. */
const ALSO_NATIVE = new Set<string>(["load", "error"]);

function addGlobalListener(
  target: object,
  rawName: string,
  fn: EventHandler | null | undefined,
  opts: ListenerOptions,
): boolean {
  if (!fn) return false;
  const name = canonicalEvent(rawName);
  // A file input's `change`/`cancel` bubble to the document like the host's
  // events do, so a delegated listener takes the same walk.
  if (!hostCode(name) && !JS_SOURCED_EVENTS.has(name) && !RUNTIME_FIRED_EVENTS.has(name)) return false;
  let m = globalHandlers.get(target);
  if (!m) globalHandlers.set(target, (m = {}));
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && (opts as ListenerOptionsObject).capture);
  (capture ? buckets.extraC : buckets.extraB).push({
    fn,
    once: !!(opts && (opts as ListenerOptionsObject).once),
    passive: passiveOf(opts, target, name),
  });
  if (appRoot) syncListenerFlag(appRoot, name);
  // `load` and `error` are ALSO the window's own events (the page's load, an
  // uncaught error), which the engine would fire at its EventTarget: the
  // listener goes to both, and hears an element's (capture phase) and the
  // window's.
  return !ALSO_NATIVE.has(name);
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
  if (appRoot) syncListenerFlag(appRoot, name);
  return !ALSO_NATIVE.has(name);
}

/// Installs the interception on one EventTarget-ish global. Events the host CAN
/// deliver are taken over; everything else still reaches the engine's own
/// EventTarget, so redjs' web APIs keep working.
/** What the interception needs off `document` / `globalThis`: two methods, maybe. */
type GlobalTargetLike = {
  addEventListener?: (name: string, fn: EventHandler, opts?: ListenerOptions) => void;
  removeEventListener?: (name: string, fn: EventHandler, opts?: ListenerOptions) => void;
};

/**
 * What a `window` / `document` listener the HOST does not source was handed
 * to: the target's own `addEventListener` as the FIRST runtime of this process
 * found it. Kept on the global so that a runtime evaluated again by
 * `location.reload()` wraps the same function, not the previous runtime's
 * wrapper (see `realm` below).
 */
function nativeListenFns(target: object): { add: GlobalTargetLike["addEventListener"]; remove: GlobalTargetLike["removeEventListener"] } {
  const g = globalThis as { __VN_NATIVE_LISTEN__?: WeakMap<object, ReturnType<typeof nativeListenFns>> };
  const table = (g.__VN_NATIVE_LISTEN__ ??= new WeakMap());
  let fns = table.get(target);
  if (!fns) {
    const t = target as GlobalTargetLike;
    fns = {
      add: typeof t.addEventListener === "function" ? t.addEventListener.bind(target) : undefined,
      remove: typeof t.removeEventListener === "function" ? t.removeEventListener.bind(target) : undefined,
    };
    table.set(target, fns);
  }
  return fns;
}

/** The engine-side `window`/`document` listeners this runtime added, which `location.reload()` removes. */
const passedThrough: { target: object; name: string; fn: EventHandler; opts: ListenerOptions }[] = [];

function interceptGlobalTarget(target: object | undefined): void {
  if (!target) return;
  const { add: nativeAdd, remove: nativeRemove } = nativeListenFns(target);
  try {
    Object.defineProperty(target, "addEventListener", {
      value: (name: string, fn: EventHandler, opts: ListenerOptions) => {
        if (addGlobalListener(target, name, fn, opts)) return;
        nativeAdd?.(name, fn, opts);
        passedThrough.push({ target, name, fn, opts });
      },
      configurable: true,
      writable: true,
    });
    Object.defineProperty(target, "removeEventListener", {
      value: (name: string, fn: EventHandler, opts: ListenerOptions) => {
        if (removeGlobalListener(target, name, fn, opts)) return;
        nativeRemove?.(name, fn, opts);
        const i = passedThrough.findIndex((p) => p.target === target && p.name === name && p.fn === fn);
        if (i >= 0) passedThrough.splice(i, 1);
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

/**
 * Platform events the RUNTIME fires, which no host code exists for: a file
 * input's `change` and `cancel` (desktop-apis-design §4.1). Their listeners
 * are kept; the "never fires" advisory is left for elements that neither are
 * nor wrap a file input (`adviseJsSourced`).
 */
const JS_SOURCED_EVENTS = new Set<string>(["change", "cancel"]);

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
  if (name === "selectionchange") {
    warnCompat(
      "event:selectionchange:element",
      `addEventListener("${rawName}") on an element — vue-native fires selectionchange at document only ` +
        "(a text field's own selection does not fire it); listen on document",
    );
  }
  if (!code && RUNTIME_FIRED_EVENTS.has(name)) {
    // `submit` / `reset`: kept; the runtime's form submission fires them.
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

function removeExtraListener(
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
    const node = nodeById(Number(part));
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
/**
 * `__vn_bounds`, with `"stale"` folded into `""` (not painted). The host says
 * `"stale"` when the node has no rectangle yet and the layout it is owed could
 * not be forced: a gpui input listener is on the stack, and a draw there would
 * swap the frame whose listeners are running (crates/vn-tree/src/dom.rs). A
 * browser would lay out and answer the real size, so a script-facing read
 * (`kind`) leaves an advisory, once per call site and kind: a component that
 * measures in a click handler would otherwise print one per row.
 */
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
/**
 * The node's LAYOUT box: where it was laid out, before any CSS `transform`.
 * What `offsetWidth`, `clientWidth`, `scrollWidth` and ResizeObserver report in
 * a browser, which all ignore transforms. `getBoundingClientRect()` is
 * {@link paintedRectOf}.
 */
function rectOf(node: MirrorNode, kind?: string): MirrorRect {
  return rectFrom(boundsOf(node, kind), 0);
}
/**
 * Where the node was DRAWN: the bounding box of its border box under its own
 * `transform` and its ancestors'. `getBoundingClientRect()`, `getClientRects()`
 * and IntersectionObserver report this in a browser. A popper is laid out at
 * (0, 0) and moved by `transform: translate()`, and a dialog is centred with
 * `translate(-50%, -50%)`: their contents measured from the layout box put
 * anything anchored to them in the window's corner. The host appends these four
 * numbers to `__vn_bounds` only when a transform moved the box.
 */
function paintedRectOf(node: MirrorNode, kind?: string): MirrorRect {
  return rectFrom(boundsOf(node, kind), 4);
}
function rectFrom(raw: VnRect, at: 0 | 4): MirrorRect {
  // A node that was not painted (detached, `display: none`, parked in a
  // <KeepAlive> cache) reports nothing — and an all-zero rect is exactly what a
  // browser reports for those, so no advisory is warranted.
  if (!raw) return { ...ZERO_RECT, toJSON() { return this; } };
  const n = raw.split(",").map(Number);
  const [x, y, w, h] = n.length >= at + 4 ? n.slice(at, at + 4) : n;
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
// A node painted in a `<VnWindow>` is in that window's document (`windowDocument`).
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
// `parentElement` is `parentNode` when that parent is an Element, else null
// (DOM §4.4). A mirror's parent is always an element; the one parent that is
// not a mirror is `document.head`, the property bag a `<style>` injected there
// reports, and head IS an element in a browser. So the test is "an element
// node, or an object shaped like one" rather than `kind === 1`.
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
// The Node-level walk (`for (let n = el.firstChild; n; n = n.nextSibling)`),
// from the same lists the renderer's own `nodeOps.nextSibling` reads.
function siblingOf(node: MirrorNode, step: 1 | -1): MirrorNode | null {
  const p = node.parentNode;
  if (!p || !p.childNodes) return null;
  // `document.head`'s list is a NodeList with no `indexOf`; a mirror's is an Array.
  const i = Array.prototype.indexOf.call(p.childNodes, node);
  return i >= 0 ? (p.childNodes[i + step] ?? null) : null;
}
defineDomGet("nextSibling", function (this: MirrorNode) { return siblingOf(this, 1); }, undefined, NODE_MEMBERS);
defineDomGet("previousSibling", function (this: MirrorNode) { return siblingOf(this, -1); }, undefined, NODE_MEMBERS);
defineDomGet("firstChild", function (this: MirrorNode) { return this.childNodes?.[0] ?? null; }, undefined, NODE_MEMBERS);
defineDomGet(
  "lastChild",
  function (this: MirrorNode) {
    const kids = this.childNodes;
    return kids && kids.length ? kids[kids.length - 1] : null;
  },
  undefined,
  NODE_MEMBERS,
);
defineDom("hasChildNodes", function (this: MirrorNode) { return !!this.childNodes?.length; }, NODE_MEMBERS);
// `getRootNode()`: the document when connected (the root's parent is the
// document in a browser), otherwise the topmost ancestor. There is no shadow
// DOM here, so `{ composed }` changes nothing.
defineDom(
  "getRootNode",
  function (this: MirrorNode) {
    let n: MirrorNode = this;
    while (n.parentNode) n = n.parentNode;
    return n === appRoot ? (globalThis.document ?? n) : n;
  },
  NODE_MEMBERS,
);
// `childNodes` is the renderer's own list: every child node, in order.
// `children` is the DOM's ELEMENT-only view of it, and the distinction is
// load-bearing — `aria-hidden` (which reka's Dialog uses to hide the rest of
// the page) walks `parent.children` and calls `getAttribute` on each, so
// handing it text and comment mirrors made it log one failure per node.
const elementChildren = (n: MirrorNode): MirrorElement[] =>
  (n.childNodes || []).filter((c): c is MirrorElement => c.kind === 1);
// A live HTMLCollection, the same object on every read, as a browser's is.
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
    if (p.childNodes[i].kind === 1) return p.childNodes[i] as MirrorElement;
  }
  return null;
}
// NonDocumentTypeChildNode: on Element AND CharacterData, so a text node's
// `nextElementSibling` is the element after it.
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

// -- selector matching, through the real cascade matcher
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

// -- getElementsByTagName / getElementsByClassName: live HTMLCollections, from
//    a walk of the mirror tree (not the selector matcher: `foo:bar` is a tag
//    name here and an invalid selector there).
/** The elements under `root` (and `root` itself when `inclusive`) that `test` takes, in tree order. */
function elementsUnder(root: MirrorNode | null, inclusive: boolean, test: (el: MirrorElement) => boolean): MirrorElement[] {
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
/** DOM's "list of elements with qualified name": `*`, else the name (lowercased against HTML elements). */
function tagTest(qualifiedName: unknown): (el: MirrorElement) => boolean {
  const q = String(qualifiedName);
  if (q === "*") return () => true;
  const lower = q.toLowerCase();
  return (el) => (el._ns ? el.tag === q : el.tag.toLowerCase() === lower);
}
/** DOM's "list of elements with class names": every token, case-sensitively; none = nothing. */
function classTest(classNames: unknown): (el: MirrorElement) => boolean {
  const want = String(classNames).split(/[ \t\n\f\r]+/).filter(Boolean);
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

// -- namespaceURI / localName / prefix (DOM §4.9). A mirror's namespace is
//    `_ns`; any other element on this prototype (the engine's detached ones)
//    is HTML. Vue's own `getContainerType`, VueUse's `isSVG` and
//    `instanceof SVGElement` all read it.
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

// -- layout, off the bounds gpui recorded for the last painted frame. The two
// rect methods answer where the box was DRAWN (transforms applied); the size
// and scroll accessors below answer the layout box, as in a browser.
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
    return Math.round(s ? pick(s) : fallback(rectOf(this, name)));
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
    return Math.round(pick(rectOf(this, name)));
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
  if (!node.__vnId) return null;
  const raw = __vn_scroll(node.__vnId, op, x, y);
  if (!raw) return null;
  const [left, top, scrollWidth, scrollHeight, stale] = raw.split(",").map(Number);
  if (stale && sizeMatters) noteStale();
  return { left, top, scrollWidth, scrollHeight, stale: !!stale };
}

/**
 * The one honest gap in this surface, named once per session.
 *
 * A browser lays out on read, so `list.append(row); list.scrollTop =
 * list.scrollHeight` sees the row. Here layout is a frame, and the host forces
 * one wherever a draw is legal: boot, timers, rAF, fetch/XHR/WebSocket
 * callbacks, mousemove, mouseup, a mouse click, contextmenu and a text field's
 * `input` all lay out on read (m26 #8). The exception is a handler that a gpui
 * listener calls directly — keydown/keyup (and the click Enter or Space
 * makes), mousedown, wheel, the clipboard events, and focus/blur, which run
 * inside gpui's own draw — because a draw there would
 * swap or nest the frame gpui is dispatching from (`vn_tree::
 * in_input_dispatch`). Only there does a content-size read (or the clamp that
 * uses it) in the same JS wave as a DOM change answer the last painted frame,
 * so "scroll to bottom" from a keydown lands one item short.
 *
 * The offset itself is never stale — `el.scrollTop` is exact at all times, and
 * so is a scroll to a position the app computed itself. This is only about
 * `scrollWidth` / `scrollHeight` and the clamp derived from them.
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
  if (!this.__vnId) return;
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
  const out = __vn_scroll_into_view(this.__vnId, block, inline);
  if (out.charCodeAt(0) === 33 /* ! */) warnCompat("scrollIntoView", out.slice(1));
  else if (out === "stale") noteStale();
});

// -- the attribute channel (`patchProp` is the writer; see `mk`)
// `style` is the one attribute whose value is not stored as text: it IS the
// inline declaration block (`InlineStyleState`), so `getAttribute("style")`
// reads what `:style` and `el.style` wrote. `attrs.style` is only a marker
// that keeps `hasAttribute` / `getAttributeNames` (and their order) honest.
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
  // `style` is a declaration BODY as an attribute: the block is replaced by
  // its parse, and the attribute keeps the text verbatim, as in a browser.
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
// `el.id` reflects the `id` attribute, as in a browser. (The host node id is
// `__vnId`; `id` named it until m26 client sweep #5a, so a library's
// `el.id = "x"` used to overwrite the node's address.)
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

// The reflected properties of `REFLECTED` (above `patchProp`) as accessors:
// each reads and writes its attribute, so `el.hidden = true`,
// `input.placeholder` and `el.tabIndex` behave as a browser's (m26 #32). An
// element whose tag does not have the property reads `undefined` and keeps an
// assignment as an expando, as a browser's `<div>` keeps `div.placeholder`.
/** HTML: tabIndex defaults to 0 on these, -1 elsewhere (and on a summary that summarizes its details). */
const TAB_INDEX_ZERO = new Set(["a", "area", "button", "frame", "iframe", "input", "object", "select", "textarea"]);
/** HTML's "rules for parsing integers", or `null` for an error. */
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
  // maxLength / minLength: "limited to only non-negative numbers", default -1.
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
    throw new DOMException(`Failed to set the '${prop}' property: The value provided (${n}) is negative.`, "IndexSizeError");
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
// `hidden` is a boolean that may also be "until-found" (HTML).
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
    else writeReflected(this, "hidden", REFLECTED.hidden, v);
  } as (value: never) => void,
);
/** `<input type>`'s keywords; anything else, or none, reads "text". */
const INPUT_TYPES = new Set(
  ("hidden text search tel url email password date month week time datetime-local number range color " +
    "checkbox radio file submit image reset button").split(" "),
);
const TYPE_STRING_TAGS = new Set(["a", "link", "ol", "li", "source", "embed", "object", "script", "style"]);
// `type`: an enumerated attribute on <input> and <button>, read-only and
// derived on <select> / <textarea>, a plain string on the rest.
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
    if (this.tag === "input" || this.tag === "button" || TYPE_STRING_TAGS.has(this.tag)) this.setAttribute("type", String(v));
    else if (!["select", "textarea", "fieldset", "output"].includes(this.tag)) ownExpando(this, "type", v);
    // (read-only on those four: the assignment is ignored, as in sloppy-mode JS)
  } as (value: never) => void,
);
// `dir`: enumerated, "" for anything but ltr / rtl / auto.
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
// DOM §4.9 `toggleAttribute`: `force` true keeps an existing value, false
// removes; it answers whether the attribute is there afterwards.
defineDom("toggleAttribute", function (this: MirrorElement, name: string, force?: boolean) {
  const has = this.hasAttribute(name);
  const on = force === undefined ? !has : !!force;
  if (on && !has) this.setAttribute(name, "");
  else if (!on && has) this.removeAttribute(name);
  return on;
});

// -- `el.attributes`, `Attr` nodes and the namespaced attribute methods
//    (DOM §4.9, §4.9.1 NamedNodeMap, §4.9.2 Attr)
//
// `attributes` is a live `NamedNodeMap` over the element's attribute list
// (`attrs`, in the order the attributes were added, as a browser keeps them):
// `length`, `item(i)`, `[i]`, `[name]`, iteration, and the get / set / remove
// methods. It is the same object on every read. Each attribute is one `Attr`
// for as long as the element has it, so `el.attributes[0] ===
// el.getAttributeNode(name)`. An Attr reads and writes through the element
// (`attr.value = "x"` is `setAttribute`), so `style` and `class` keep their
// own rules. Removing the attribute detaches its Attr (`ownerElement` null),
// which keeps the value it had.
//
// Names are kept as written, as `getAttribute` keeps them here; a browser
// lowercases them on an HTML element. The mirror has no element namespaces,
// so an attribute's namespace comes from its prefix: `xlink:`, `xml:` and
// `xmlns` are the XLink, XML and XMLNS namespaces, the way Vue's renderer
// sets them on SVG; any other name has none.

const XLINK_NS = "http://www.w3.org/1999/xlink";
const XML_NS = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS = "http://www.w3.org/2000/xmlns/";

/** A qualified attribute name's namespace, prefix and local name. */
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

/** The qualified name of `el`'s attribute in namespace `ns` with local name `local`, if it has one. */
function attrNameNS(el: MirrorElement, ns: unknown, local: unknown): string | null {
  const want = ns == null || ns === "" ? null : String(ns);
  const l = String(local);
  for (const name of Object.keys(el.attrs ?? {})) {
    const p = attrParts(name);
    if (p.ns === want && p.local === l) return name;
  }
  return null;
}

/** One attribute as a node. Attached: `_el` is its element. Detached: `_value` is its value. */
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
for (const [k, v] of [["ATTRIBUTE_NODE", 2], ["ELEMENT_NODE", 1], ["TEXT_NODE", 3], ["COMMENT_NODE", 8]] as const) {
  Object.defineProperty(AttrImpl.prototype, k, { value: v, enumerable: true });
}

/** Each element's Attr objects, by name, for the attributes it has now. */
const attrNodes = new WeakMap<MirrorElement, Map<string, AttrImpl>>();
/** No Attr has been made yet: an attribute change then has none to detach. */
let noAttrNodes = true;

/** The Attr for `el`'s attribute `name`, made on first use; null when it has none. */
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

/** An attribute changed (`notifyMutation`): if it is gone, its Attr detaches, keeping the value it had. */
function attrChanged(el: MirrorElement, name: string, oldValue: string | null): void {
  if (noAttrNodes || (el.attrs && name in el.attrs)) return;
  const map = attrNodes.get(el);
  const a = map?.get(name);
  if (!a) return;
  if (oldValue !== null) a._value = oldValue;
  a._el = null;
  map!.delete(name);
}

/** DOM "set an attribute": `attr` becomes `el`'s, replacing (and returning) the one of its name. */
function setAttrNode(el: MirrorElement, attr: unknown, method: string): AttrImpl | null {
  if (!(attr instanceof AttrImpl)) {
    throw new TypeError(`Failed to execute '${method}' on 'Element': parameter 1 is not of type 'Attr'.`);
  }
  if (attr._el && attr._el !== el) {
    throw domError(method, "Element", "InUseAttributeError", "The node provided is an attribute node that is already an attribute of another Element; attribute nodes must be explicitly cloned.");
  }
  if (attr._el === el) return attr;
  const old = attrNode(el, attr._name);
  if (old) old._value = old.value;
  el.setAttribute(attr._name, attr._value);
  // `setAttribute` kept the old Attr attached; the new one takes its place.
  if (old) old._el = null;
  noAttrNodes = false;
  let map = attrNodes.get(el);
  if (!map) attrNodes.set(el, (map = new Map()));
  attr._el = el;
  map.set(attr._name, attr);
  return old;
}

/** DOM "remove an attribute": `attr` must be `el`'s. */
function removeAttrNode(el: MirrorElement, attr: unknown, method: string, iface: string): AttrImpl {
  if (!(attr instanceof AttrImpl) || attr._el !== el) {
    throw domError(method, iface, "NotFoundError", "The node provided is owned by another element.");
  }
  attr._value = attr.value;
  el.removeAttribute(attr._name);
  return attr;
}

/** `el.attributes`: one per element, live. */
const namedNodeMaps = new WeakMap<MirrorElement, object>();

class NamedNodeMapImpl {
  /** The element whose attributes these are. */
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
    if (!a) throw domError("removeNamedItem", "NamedNodeMap", "NotFoundError", `No item with name '${String(name)}' was found.`);
    return removeAttrNode(this._el, a, "removeNamedItem", "NamedNodeMap");
  }
  removeNamedItemNS(ns: unknown, local: unknown): AttrImpl {
    const name = attrNameNS(this._el, ns, local);
    if (name === null) {
      throw domError("removeNamedItemNS", "NamedNodeMap", "NotFoundError", `No item with name '${String(ns)}::${String(local)}' was found.`);
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

/** Index and name lookups (`attributes[0]`, `attributes.id`), live, over the map's methods. */
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
      if (i >= 0) return i < t.length ? { value: t.item(i), enumerable: true, configurable: true, writable: false } : undefined;
      return Reflect.getOwnPropertyDescriptor(t, k);
    },
    // Indexed and named properties are read-only, as on a browser's map.
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
// The qualified name is the key, so `setAttributeNS(XLINK_NS, "xlink:href", v)`
// and `getAttribute("xlink:href")` meet, and `setAttributeNS(null, "x", v)` is
// `setAttribute("x", v)`.
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

// -- className and classList, over the class ATTRIBUTE: what Vue assigned plus
//    the transition classes (`writeClass`), as a browser's are (m26 #19).
//    `className` was a plain field: a script's `el.className = …` changed the
//    mirror only, and `getAttribute("class")` read null.
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

// -- inline style: `el.style` is a CSSStyleDeclaration over the element's
//    `InlineStyleState`, so it reads what `:style` and `setAttribute("style")`
//    wrote, and its writes go through the same host call `:style` makes —
//    CUSTOM PROPERTIES INCLUDED, which is how reka-ui's `--reka-popper-*`
//    sizing variables reach the cascade.

/**
 * `style.backgroundColor = v`, the IDL attribute setter. `null` is the empty
 * string ([LegacyNullToEmptyString]), which removes the declaration. A browser
 * ignores a value it cannot parse, and two are knowable here without a parser:
 * `undefined` (the text "undefined" is no property's value) and a value that
 * carries `!important` (a priority is only `setProperty`'s third argument).
 *
 * `style["--x"] = v` sets the custom property. In a browser a custom property
 * is not an attribute of the declaration and the write is an expando; here it
 * is kept as the cascade write it has always been (docs/RUNTIME.md §5.3).
 */
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
  // The target carries the REAL members; the handler routes everything else
  // to the declarations. `as MirrorStyle` is the one place the two halves are
  // joined: a Proxy's type is its target's, and the target deliberately does
  // not carry the open-ended `[property: string]` half that the handler serves.
  const target = {
    /** CSSOM `setProperty(property, value, priority)`. */
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
      // The indexed getter: a declared name, `undefined` past the end.
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

// -- focus: `el.focus()` / `el.blur()` are defined with the forms, below
//    (`__vn_focus`; crates/vn-tree/src/render.rs, "Programmatic focus").

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
  // redjs's own `document.createElement("a" | "input")` elements are not in
  // the tree; the FileSaver.js and VueUse `useFileDialog` idioms click them.
  if (!isMirrorNode(target)) return activateDetached(target);
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (hasActivationBehaviour(n)) {
      if (isFileInput(n)) showFilePicker(fileInputOps(n), null);
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

/** A link's activation, by its `href` and its `download` attribute (null: none). */
function followHref(href: string, download: string | null): void {
  const raw = href.trim();
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
  if (download != null && (scheme === "blob" || scheme === "data")) {
    // Save As, then the bytes (desktop.ts; D-F6). Resolved now, at the click,
    // so `a.click(); URL.revokeObjectURL(url)` still saves. No activation is
    // needed: Chromium lets a page start a download without one.
    desktopApi.download(raw, download);
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

// -- <input type=file> and <label>: activation behaviour (docs/DESKTOP.md
//    "<input type=file>"; desktop-apis-design §4.1, §4.2)
//
//    A file input's click opens the picker, through desktop.ts and the host's
//    dialog. Its `files`, `value`, `accept`, `multiple` and `webkitdirectory`
//    are own accessors, installed when `type` becomes "file". Its `input`,
//    `change` and `cancel` are fired from here and walk the tree like any
//    host event. A `<label>` clicks its control. The same behaviour serves
//    redjs's detached `document.createElement("input")`, which is how VueUse's
//    `useFileDialog` picks files without an element in the tree.

/** Chromium's English status strings (D-F12): what `:file-label` pushes. */
function fileStatus(files: ArrayLike<{ name?: unknown }> | null): string {
  const n = files?.length ?? 0;
  if (n === 0) return "No file chosen";
  return n === 1 ? String(files![0]?.name ?? "") : `${n} files`;
}

/**
 * Is this a mirror `<input type=file>`? (Any casing of the attribute, as HTML
 * says.) Deliberately not a type guard: an element that is not one is still
 * an element.
 */
function isFileInput(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "input" && String(e.attrs?.type ?? "").trim().toLowerCase() === "file";
}

/**
 * Is this one of the host's text widgets? An `<input>` other than a file input,
 * or a `<textarea>`: the host's own rule (`vn-tree` `build_node`).
 */
function isTextWidget(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && (e.tag === "textarea" || (e.tag === "input" && !isFileInput(e)));
}

/** Is this a mirror `<label>`? */
function isLabel(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "label";
}

/** `instanceof FileList`, where the engine has the class (redjs does). */
function isFileList(v: unknown): boolean {
  const FL = (globalThis as { FileList?: unknown }).FileList;
  if (typeof FL === "function") return v instanceof (FL as new () => object);
  return !!v && typeof v === "object" && typeof (v as { length?: unknown }).length === "number" &&
    typeof (v as { item?: unknown }).item === "function";
}

/** An empty FileList: what `files` is before anything is picked. */
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

/**
 * A file input's selected files (`null`: none yet), mirror or detached alike,
 * and what to do when they change (push the status to the host).
 */
interface FileState {
  files: unknown;
  changed: () => void;
}
const fileStates = new WeakMap<object, FileState>();

const FAKEPATH = "C:\\fakepath\\";
const VALUE_REFUSED =
  "Failed to set the 'value' property on 'HTMLInputElement': This input element accepts a filename, " +
  "which may only be programmatically set to the empty string.";
const FILES_REFUSED = "Failed to set the 'files' property on 'HTMLInputElement': The provided value is not of type 'FileList'.";

/**
 * HTML's `files` and filename-mode `value` over one state, for either kind of
 * element. `files` is a FileList always (an empty one before a pick); setting
 * it takes a FileList (`input.files = dataTransfer.files`) and fires nothing;
 * `null` is ignored. `value` is `C:\fakepath\<name>`. Setting it to "" clears
 * the selection; anything else throws.
 */
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

/**
 * `type` was patched on an `<input>`: become a file input (the accessors, the
 * status prop, the click/Enter/Space bits) or stop being one.
 */
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
        "<input type=file> (D-F12): the button and status are an approximation of Chromium's, with its " +
          "English strings (\"Choose File\", \"No file chosen\"); restyle it with ::file-selector-button",
      );
    }
  } else if (!file && had) {
    fileStates.delete(el);
    for (const k of ["files", "value", "accept", "multiple", "webkitdirectory"]) delete (el as unknown as Record<string, unknown>)[k];
    el._value = "";
    __vn_set_prop(el.__vnId, I(":file-label"), undefined);
  }
  for (const n of ["click", "keydown", "keyup"]) syncListenerFlag(el, n);
}

/** What the picker flow needs from a file input, whichever kind it is. */
interface FileInputOps {
  accept: string;
  multiple: boolean;
  directory: boolean;
  disabled: boolean;
  current: unknown;
  setFiles(list: unknown): void;
  fire(type: "input" | "change" | "cancel"): void;
}

/** HTML: `input` bubbles and is composed; `change` and `cancel` bubble. */
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

/**
 * HTML "show the picker, if applicable", for a click (`via` null) or for
 * `showPicker()`, which has already thrown for what it throws. A click with
 * no transient activation does nothing but say so, in Chromium's words.
 * Picked: `files` changes, then `input` and `change`. Dismissed, or the same
 * selection again: `cancel`. Another dialog already open on the window: the
 * request is dropped, as Chromium drops it.
 */
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

/** HTML's labelable elements. */
const LABELABLE = new Set(["button", "meter", "output", "progress", "select", "textarea"]);
const isLabelable = (n: MirrorNode): n is MirrorElement =>
  n.kind === 1 && (LABELABLE.has(n.tag) || (n.tag === "input" && String(n.attrs.type ?? "").trim().toLowerCase() !== "hidden"));

/** The first element at or under `root`, in tree order, that `pred` accepts. */
function firstInTree(root: MirrorNode, pred: (n: MirrorElement) => boolean): MirrorElement | null {
  if (root.kind === 1 && pred(root)) return root;
  for (const c of root.childNodes ?? []) {
    const hit = firstInTree(c, pred);
    if (hit) return hit;
  }
  return null;
}

/**
 * A label's labeled control: the element its `for` names, if that one is
 * labelable (the first with that id in the label's tree, as
 * `getElementById` finds it), else its first labelable descendant.
 */
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

/** The controls a label click would focus in a browser (and cannot here). */
const FOCUS_ONLY_TYPES = new Set(["button", "submit", "reset", "image", "file", "checkbox", "radio", "range", "color"]);

/**
 * A `<label>`'s activation: a click on its control, unless the click came
 * from inside the control (then it already was one). A text control is also
 * focused, as in a browser (D-F23 until m26 phase 2, when `focus()` became
 * real).
 */
function activateLabel(label: MirrorElement, target: MirrorNode): void {
  const control = labelControl(label);
  if (!control) return;
  for (let n: MirrorNode | null = target; n; n = n.parentNode) if (n === control) return;
  const textual =
    control.tag === "textarea" || control.tag === "select" ||
    (control.tag === "input" && !FOCUS_ONLY_TYPES.has(String(control.attrs.type ?? "text").trim().toLowerCase()));
  if (textual) control.focus();
  control.click();
}

/** A `change` or `cancel` listener on something that neither is nor wraps a file input. */
function adviseJsSourced(el: MirrorElement, name: string, via: string): void {
  const key = `event:${name}`;
  if (warned.has(key)) return;
  queueMicrotask(() => {
    if (warned.has(key) || firstInTree(el, (n) => isFileInput(n) || (name === "change" && isSelectEl(n)))) return;
    warnCompat(
      key,
      `${via} — this event type is not delivered by the native host yet: only <input type=file> ` +
        `(and <select>, for \`change\`) fires it here, so on this element it will never fire`,
    );
  });
}

// -- detached elements: redjs's `document.createElement("a" | "input")`
//
//    They are plain objects on HTMLElement.prototype, outside the tree, and
//    their attribute methods are no-ops. `wrapDetached` gives the two the
//    idioms need a reflected attribute map, and an input the same
//    `files`/`value` accessors a mirror file input has. Their events go
//    through the engine's own EventTarget (`input.onchange = …` included).

const REFLECTED_BOOL = new Set(["multiple", "webkitdirectory", "disabled"]);
const REFLECTED_STR = new Set(["href", "download", "type", "accept", "rel", "target"]);
const nativeDispatch =
  typeof EventTarget === "function" ? (EventTarget.prototype.dispatchEvent as (this: object, ev: VnEvent) => boolean) : null;

type Detached = Record<string, unknown>;

/** Makes a fresh detached `<a>` / `<input>` behave like the element it names. */
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
  const acc = fileAccessors(state, () => String(el.type ?? "").trim().toLowerCase() === "file", other);
  try {
    Object.defineProperties(el, { files: acc.files, value: acc.value });
  } catch {
    /* as above */
  }
}

/** Fires `type` on a detached element through the engine's EventTarget. */
function fireDetached(el: Detached, type: string): void {
  const ev = fileEvent(type);
  try {
    if (!nativeDispatch) throw new TypeError("no EventTarget");
    nativeDispatch.call(el, ev);
  } catch {
    // Not an EventTarget the engine will dispatch on: the handler attribute,
    // with the target a browser would give it (VueUse reads
    // `event.target.files`).
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

/** The activation behaviour of a detached `<a>` or `<input type=file>`. */
function activateDetached(target: object): void {
  const el = target as Detached;
  const tag = String(el.tagName ?? "").toUpperCase();
  if (tag === "A" && el.href != null) {
    followHref(String(el.href), el.download == null ? null : String(el.download));
  } else if (tag === "INPUT" && String(el.type ?? "").trim().toLowerCase() === "file") {
    showFilePicker(detachedFileOps(el), null);
  }
}

// -- showPicker() (HTML): the picker without the click. Chromium's checks and
//    words: an immutable control is InvalidStateError, no user activation is
//    NotAllowedError. Only file pickers exist here (A-F6).
defineDom("showPicker", function (this: MirrorElement) {
  const mirror = isMirrorNode(this);
  const raw = this as unknown as Detached;
  const tag = mirror ? this.tag : String(raw.tagName ?? "").toLowerCase();
  if (tag !== "input") {
    throw new TypeError("Failed to execute 'showPicker': this element has no picker (only <input> has one here).");
  }
  const attr = (k: string) => (mirror ? this.attrs[k] != null && this.attrs[k] !== "false" : !!raw[k]);
  const type = String((mirror ? this.attrs.type : raw.type) ?? "text").trim().toLowerCase();
  const fail = (name: string, msg: string) =>
    new DOMException(`Failed to execute 'showPicker' on 'HTMLInputElement': ${msg}`, name);
  if (attr("disabled") || (type !== "file" && attr("readonly"))) {
    throw fail("InvalidStateError", "HTMLInputElement::showPicker() cannot be used on immutable controls.");
  }
  if (!activation.isActive()) throw fail("NotAllowedError", "HTMLInputElement::showPicker() requires a user gesture.");
  if (type !== "file") {
    warnCompat(`showPicker:${type}`, `showPicker() on <input type=${type}> is a no-op: only file pickers exist here (A-F6)`);
    return;
  }
  showFilePicker(mirror ? fileInputOps(this) : detachedFileOps(raw), "showPicker");
});

// -- el.click(): HTML's synthetic click. A real `MouseEvent`, through the same
//    walk a host click takes, then the activation behaviour if nobody
//    cancelled it. `click()` on a disabled form control does nothing at all,
//    and a click() from inside its own click handler is ignored (HTML's
//    "click in progress" flag).
const DISABLEABLE = new Set(["button", "input", "select", "textarea", "fieldset", "optgroup", "option"]);
const clicking = new Set<MirrorElement>();
defineDom("click", function (this: MirrorElement) {
  // A mirror control is "actually disabled" (its own attribute or a disabled
  // fieldset); a detached one has only its own attribute.
  const mirror: boolean = isMirrorNode(this);
  if (mirror ? isDisabledControl(this) : DISABLEABLE.has(this.tag) && this.attrs.disabled != null && this.attrs.disabled !== "false") return;
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

// -- forms: submission (HTML §4.10.21; m26 phase 2)
//
//    There was none. `@submit` had no host code, so patchProp dropped the
//    handler with an advisory: Enter in a field did nothing, and so did a
//    click on `<button type=submit>`. Login forms are the case that matters
//    (`<form @submit.prevent>` plus a submit button).
//
//    What is here, in the spec's own terms:
//    - a submit button's activation behaviour: a click (host or `el.click()`)
//      submits its form owner, unless it is disabled;
//    - implicit submission: Enter in a field (keydown, not cancelled) clicks
//      the form's default button (the first submit button in tree order;
//      nothing if it is disabled), or, with no submit button, submits the form
//      when at most one field blocks implicit submission;
//    - `form.requestSubmit(submitter?)` with the spec's TypeError and
//      NotFoundError, `form.submit()` (no event, as in a browser), and
//      `form.reset()` / a reset button (fires `reset`);
//    - the `submit` event is a `SubmitEvent` (bubbles, cancelable,
//      `submitter`), walked like any other event; `SubmitEvent` is defined if
//      the engine lacks it.
//
//    Divergences, each an advisory: constraint validation is not run (a
//    form without `novalidate` submits anyway, and no bubble is shown); a
//    submission nobody cancelled has no document to navigate, so it does
//    nothing; a reset does not restore the controls' values.

/** Events the runtime itself fires, for which no host code exists. Their handlers are kept. */
const RUNTIME_FIRED_EVENTS = new Set<string>(["submit", "reset"]);

/** HTML's boolean-attribute rule as patchProp stores it: present, and not the string "false". */
const hasFlag = (n: MirrorElement, name: string): boolean => n.attrs?.[name] != null && n.attrs[name] !== "false";
/** HTML's "actually disabled": its own `disabled`, or a disabled `<fieldset>` around it (outside that fieldset's first `<legend>`). */
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
const typeOf = (n: MirrorElement): string => String(n.attrs?.type ?? "").trim().toLowerCase();

/** `<button>` (submit is its default, and any unknown type), `<input type=submit|image>`. */
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

/** The input types that "block implicit submission" (HTML §4.10.21.2). No type is `text`. */
const IMPLICIT_SUBMIT_TYPES = new Set([
  "text", "search", "url", "tel", "email", "password", "date", "month", "week", "time", "datetime-local", "number",
]);
function blocksImplicitSubmission(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "input" && IMPLICIT_SUBMIT_TYPES.has(typeOf(e) || "text");
}

/** HTML's listed elements: the ones a form owns. */
const LISTED = new Set(["button", "fieldset", "input", "object", "output", "select", "textarea"]);

function topOf(n: MirrorNode): MirrorNode {
  while (n.parentNode) n = n.parentNode;
  return n;
}

/** The form owner: the `form` attribute's form (or none), else the nearest `<form>` ancestor. */
function formOwner(n: MirrorElement): MirrorElement | null {
  const fid = n.attrs?.form;
  if (fid != null && LISTED.has(n.tag)) {
    const f = firstInTree(topOf(n), (x) => x.attrs?.id === fid);
    return f && f.tag === "form" ? f : null;
  }
  for (let p = n.parentNode; p; p = p.parentNode) if (p.kind === 1 && p.tag === "form") return p as MirrorElement;
  return null;
}

/** The form's owned controls, in tree order. */
function formControls(form: MirrorElement): MirrorElement[] {
  const out: MirrorElement[] = [];
  forEachDeep(topOf(form), (n) => {
    if (n.kind === 1 && LISTED.has(n.tag) && formOwner(n as MirrorElement) === form) out.push(n as MirrorElement);
  });
  return out;
}

type SubmitEventInit = { bubbles?: boolean; cancelable?: boolean; composed?: boolean; submitter?: unknown };
/** The engine's `SubmitEvent`, or ours, installed on the global when it lacks one. */
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
/**
 * The engine's `ClipboardEvent`, or ours, installed on the global when it
 * lacks one. `clipboardData` is a prototype getter, as WebIDL puts a readonly
 * attribute, so `"clipboardData" in ClipboardEvent.prototype` holds.
 */
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
    Object.defineProperty(globalThis, "ClipboardEvent", { value: mark(ClipboardEvent), configurable: true, writable: true });
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

/** Constraint validation is not run; said once, and only for a form that has constraints to skip. */
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

/** The form's "firing submission events" flag: a submit from inside a submit handler is ignored. */
const submitting = new WeakSet<MirrorElement>();

/** HTML "submit": `submitter` is the button, or null for the form itself. */
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
    const ev = SubmitEventClass ? new SubmitEventClass("submit", init) : withProps(new CustomEvent("submit", init), { submitter });
    proceed = propagate(form, "submit", ev);
  } finally {
    submitting.delete(form);
  }
  if (proceed) adviseNoNavigation("a submit event nobody cancelled");
}

/** HTML "reset": the `reset` event; the controls are not restored (advisory). */
function resetForm(form: MirrorElement): void {
  const init = { bubbles: true, cancelable: true };
  const ev = typeof Event === "function" ? new Event("reset", init) : new CustomEvent("reset", init);
  if (propagate(form, "reset", ev)) {
    warnCompat(
      "form:reset",
      "form reset (a reset button or form.reset()) fired `reset` but cannot restore the controls: the " +
        "mirror keeps no default values. Reset the model in the handler",
    );
  }
}

/** A submit or reset button's activation behaviour. */
function activateFormButton(n: MirrorElement): void {
  if (isDisabledControl(n)) return;
  const form = formOwner(n);
  if (!form) return;
  if (isSubmitButton(n)) submitForm(form, n, false);
  else if (isResetButton(n)) resetForm(form);
}

/** Enter in `field` (HTML §4.10.21.2). */
function implicitSubmit(field: MirrorElement): void {
  const form = formOwner(field);
  if (!form) return;
  const controls = formControls(form);
  const defaultButton = controls.find(isSubmitButton);
  if (defaultButton) {
    // "If the form has no submit button…" — it has one, so the button is
    // clicked, and a disabled default button means nothing happens at all.
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
      throw new TypeError("Failed to execute 'requestSubmit' on 'HTMLFormElement': The specified element is not a submit button.");
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

// -- focus: `el.focus()` / `el.blur()` / `document.activeElement` (m26 phase 2)
//
//    The host moves gpui's focus (`__vn_focus`, crates/vn-tree/src/lib.rs and
//    render.rs "Programmatic focus"). A node that can take focus NOW gets it
//    synchronously, and the four events fire here, in the browser's order:
//    blur and focusout on the old node, then focus and focusin on the new one,
//    each with its `relatedTarget`. A node that has not been built yet (mounted
//    in this same update) is queued; its events come from the host a frame
//    later, which is one advisory. Not focusable, disabled or detached: a
//    no-op, as in a browser. Divergence, named: `focus()` does not scroll the
//    element into view (`preventScroll` is the only behaviour).
//
//    A handler that moves the focus while another move's events are running
//    abandons that move, as every browser does (Chromium's
//    `Document::SetFocusedElement`, "handler shifted focus"). While the old
//    node hears `blur` and `focusout`, nothing is focused, so
//    `document.activeElement` is `body`. A `focus()` from one of those handlers
//    wins: the node the first move was going to gets no `focus` and no
//    `focusin`. reka's FocusScope relies on this when it traps focus in a
//    dialog: its `focusout` handler pulls the focus back. Without it, a focus
//    handed back to a menu's trigger after the dialog opened still reached the
//    trigger's `focus` listener, and a tooltip opened above the dialog
//    (docs/RUNTIME.md §5.21). A `focus` handler that moves the focus on
//    likewise cancels its own node's `focusin`.

/** The element JS last learned has focus: a move from here, or a host focus/blur event. */
let focusedNode: MirrorElement | null = null;

/** Bumped by every focus change. A move whose count changed while its handlers ran was overtaken. */
let focusChanges = 0;

/**
 * The node a `focus()` is moving to while the old node hears `blur` and
 * `focusout`. The host has already given it the focus, but in a browser
 * nothing is focused yet: `blur()` on it does nothing, and `focus()` on it
 * focuses it now.
 */
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

/**
 * The events of a move the host has already made, in the browser's order:
 * `blur` and `focusout` on `old` with nothing focused, then `focus` and
 * `focusin` on `next`. A handler that moves the focus ends the move there.
 * `focusout` then has no `relatedTarget`, as in Chromium.
 */
function focusMoved(next: MirrorElement, old: MirrorElement | null): void {
  const change = ++focusChanges;
  // Any move whose blur phase is still running is overtaken by this one.
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

/** What `focus()` / `blur()` work on: a connected mirror element. */
const focusable = (el: MirrorElement): boolean =>
  isMirrorNode(el) && el.kind === 1 && nodeById(el.__vnId) === el && !isDisabledControl(el);

defineDom("focus", function (this: MirrorElement) {
  if (!focusable(this)) return;
  if (typeof __vn_focus !== "function") {
    warnCompat("focus", "element.focus() is a no-op: this host has no programmatic focus (__vn_focus)");
    return;
  }
  // From a handler of a move to this very node: the host already gave it the
  // focus, and a browser focuses it now, with no old node.
  if (this === arriving) {
    focusMoved(this, null);
    return;
  }
  const answer = __vn_focus(this.__vnId, 0);
  if (answer.startsWith("moved:")) {
    const old = (nodeById(Number(answer.slice(6))) as MirrorElement | undefined) ?? null;
    // The host's old node is the one a move in progress was going to. It
    // never got the focus, so there is nothing to blur.
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
  // Not focused yet in a browser: a move to it is still in its blur phase.
  if (this === arriving) return;
  if (__vn_focus(this.__vnId, 1) === "blurred") {
    focusChanges++;
    arriving = null;
    if (focusedNode === this) focusedNode = null;
    fireFocusEvent(this, "blur", null);
    fireFocusEvent(this, "focusout", null);
  }
});

/** `document.activeElement`: the focused element, else the root (which is `body` here). */
function activeElement(): MirrorElement | null {
  if (focusedNode && nodeById(focusedNode.__vnId) === focusedNode) return focusedNode;
  return appRoot;
}

// -- location.reload() (m26 phase 2): the host's restart path, on request
//
//    `location.reload()` asks the host (`__vn_reload`, crates/vn-app/src/
//    reload.rs). At the start of the next frame the host calls
//    `__VN_UNLOAD__`, below, and then evaluates the bundle again. A browser
//    would throw the realm away. This one stays, so the outgoing app's own
//    scheduling is undone here, after `pagehide` and the unmount:
//
//    - every timer, interval, animation frame and idle callback scheduled
//      since this runtime loaded;
//    - every WebSocket opened since, with its listeners removed first, so
//      the old code's reconnect logic cannot run;
//    - every `window` / `document` listener added through the runtime for
//      an event the host does not source (`resize`, `message`, `storage`…).
//
//    The host then ends the old page's media as a navigation does
//    (crates/vn-app/src/reload.rs `end_media`): every voice room leaves, with
//    its peer connections, every MediaStreamTrack stops (an unused microphone
//    is released) and every AudioContext closes.
//
//    What stays, and is said once when a reload happens: globals the old app
//    set, fetches in flight (they settle into the old promise chains),
//    IndexedDB connections, and the fact that `DOMContentLoaded`
//    / `load` do not fire again. `beforeunload` is not asked.

type Fn = (...args: unknown[]) => unknown;
type SocketLike = {
  close(): void;
  addEventListener?: Fn;
  removeEventListener?: Fn;
  [handler: string]: unknown;
};

/// Called with each animation frame's timestamp before a rAF callback runs:
/// `document.timeline` holds its time to it (src/waapi.ts, `onFrame`).
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

  /** `set(cb, …)` → id, tracked until it fires (`once`) or is cancelled. */
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
            return (cb as Fn).apply(this, a);
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

  /** Undoes everything above for the outgoing app. */
  const close = () => {
    for (const { target, name, fn, opts } of passedThrough.splice(0)) {
      try {
        nativeListenFns(target).remove?.(name, fn, opts);
      } catch {
        /* already gone */
      }
    }
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
  const g = globalThis as { location?: Record<string, unknown>; __VN_UNLOAD__?: () => void; __VN_TEARDOWN__?: () => void };
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
      warnCompat("location.reload:install", `location.reload() could not be installed on this engine and does nothing (${e})`);
    }
  }
  // The host calls this instead of `__VN_TEARDOWN__` for a reload.
  g.__VN_UNLOAD__ = () => {
    g.__VN_UNLOAD__ = undefined;
    warnCompat(
      "location.reload",
      "location.reload() re-runs the app in the SAME JavaScript realm: its timers, animation frames, " +
        "WebSockets, window/document listeners, voice rooms (with their peer connections), media tracks " +
        "and AudioContexts are closed and the globals it added are deleted, but " +
        "changes it made to built-in objects and to globals that were already there stay, fetches in " +
        "flight settle into the old code, IndexedDB connections stay until the old code " +
        "ends them, beforeunload is not asked, and DOMContentLoaded / load do not fire again",
    );
    try {
      const ev = typeof Event === "function" ? new Event("pagehide") : null;
      if (ev) {
        Object.defineProperty(ev, "persisted", { value: false });
        (globalThis as { dispatchEvent?: (e: VnEvent) => boolean }).dispatchEvent?.(ev);
      }
    } catch (e) {
      __vn_log("reload: pagehide: " + e);
    }
    try {
      g.__VN_TEARDOWN__?.();
    } catch (e) {
      __vn_log("reload: teardown: " + e);
    }
    realm.close();
    // The host keeps calling the observer step it was given until the next
    // runtime installs its own; with nothing observed, the old one does
    // nothing, and the host is told so it stops asking.
    resizeObservers.length = 0;
    resizeDue = [];
    intersectionObservers.clear();
    if (hostSteps) __vn_layout_watch(false);
    // Last: the teardown and the realm hygiene above still call through the
    // globals they are about to lose.
    const kept = resetGlobals();
    if (kept)
      warnCompat(
        "location.reload:globals",
        `location.reload(): ${kept} global(s) the old app declared cannot be deleted (a top-level var or ` +
          "function in a classic script, or a non-configurable property), so the new app finds them set",
      );
  };
}

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

// ---- imperative tree mutation (DOM §4.2.3–§4.2.8) — m26 client sweep #5a
//
// `appendChild`, `insertBefore`, `removeChild`, `replaceChild`, ChildNode's
// `remove`/`before`/`after`/`replaceWith`, ParentNode's `append`/`prepend`/
// `replaceChildren`, and `insertAdjacentElement`/`insertAdjacentText`, with
// the DOM's validity checks, exceptions and return values. They run through
// the renderer's own `nodeOps.insert` (which is also a move), so the host sees
// exactly the ops a Vue patch sends. A removal DETACHES (see `detachNode`): the
// node keeps its listeners and its state and can be inserted again.
//
// One thing a script can hand in is not a tree node at all: the engine's own
// detached `document.createElement("a" | "input" | "style" | …)` objects
// (`ENGINE_TAGS`). A `<style>` / `<link>` one joins the stylesheet as it would
// in `<head>` (`foreignNode`); any other is left out of the tree, with an
// advisory.

/** Tags `document.createElement` leaves to the engine (see there). */
const ENGINE_TAGS = new Set(["a", "input", "style", "link", "script", "meta", "title", "base"]);
/** DOM's "valid element local name" (the 2025 relaxed rule): no whitespace, NUL, `/` or `>`, and a letter, `:`, `_` or non-ASCII first. */
const VALID_ELEMENT_NAME = /^[A-Za-z:_\u0080-￿][^\t\n\f\r \/>\0]*$/;
/** The head-style bookkeeping's hook (set below): an engine `<style>`/`<link>` inserted or removed anywhere. */
let foreignStyle: ((n: object, inserted: boolean) => boolean) | null = null;
/** `new DocumentFragment()` / `createDocumentFragment()` mirrors. Inserting one inserts its children. */
const fragments = new WeakSet<MirrorNode>();

const domError = (method: string, iface: string, name: string, msg: string): Error =>
  new DOMException(`Failed to execute '${method}' on '${iface}': ${msg}`, name);

/**
 * A Node argument: a live mirror, or `null` for something that is a node in a
 * browser but not one of ours (an engine element, or a mirror the renderer has
 * freed), which `foreignNode` deals with. Anything else is WebIDL's TypeError.
 */
function nodeArg(x: unknown, method: string, iface: string, pos: number, type = "Node"): MirrorNode | null {
  if (isMirrorNode(x)) {
    if (type === "Element" && x.kind !== 1) throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter ${pos} is not of type 'Element'.`);
    return x;
  }
  if (x && typeof x === "object" && (typeof (x as { tagName?: unknown }).tagName === "string" || "__vnId" in x)) return null;
  throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter ${pos} is not of type '${type}'.`);
}

/** Inserting or removing a node that is not in the tree: a stylesheet, or an advisory. */
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

/** DOM "ensure pre-insertion validity", for one node into `parent`. */
function preInsertCheck(parent: MirrorNode, node: MirrorNode, method: string): void {
  if (parent.kind !== 1) throw domError(method, "Node", "HierarchyRequestError", "This node type does not support this method.");
  for (let a: MirrorNode | null = parent; a; a = a.parentNode) {
    if (a === node) throw domError(method, "Node", "HierarchyRequestError", "The new child element contains the parent.");
  }
}

/** An element's own text becomes a real Text child before it gains a sibling, so the text is kept, as in a browser. */
function materializeOwnText(el: MirrorElement): void {
  if (!el.ownText || el.childNodes.length) return;
  const t = nodeOps.createText(el.ownText);
  el.ownText = "";
  insertNode(t, el, null, false);
  // A selection boundary in the own text moves into the Text child.
  if (typeof __vn_selection_rebase === "function") __vn_selection_rebase(el.__vnId, t.__vnId);
}

/** Inserts `items` (mirrors, fragments expanded, or engine elements) before `ref` in `parent`, in order: one record. */
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

/** DOM "pre-insert": checks, then the insertion. Answers `node`. */
function preInsert(parent: MirrorNode, node: unknown, child: unknown, method: string): unknown {
  const n = nodeArg(node, method, "Node", 1);
  const c = child == null ? null : nodeArg(child, method, "Node", 2);
  if (!n) {
    foreignNode(node as object, true, method);
    return node;
  }
  preInsertCheck(parent, n, method);
  if (child != null && (!c || c.parentNode !== parent)) {
    throw domError(method, "Node", "NotFoundError", "The node before which the new node is to be inserted is not a child of this node.");
  }
  insertAll(parent as MirrorElement, [n], c === n ? n.nextSibling : c);
  return n;
}

/** DOM "convert nodes into a node": strings (and any non-Node) become Text; every node is checked against `parent` first. */
function convertNodes(parent: MirrorNode, args: unknown[], method: string, iface: string): (MirrorNode | object)[] {
  const items = args.map((a) => {
    if (isMirrorNode(a)) return a;
    if (a && typeof a === "object" && (typeof (a as { tagName?: unknown }).tagName === "string" || "__vnId" in a)) return a;
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

/** The first of `from`, `from.next`, … that is not one of `items` (DOM's "viable" sibling). */
function viable(from: MirrorNode | null, items: unknown[], step: 1 | -1): MirrorNode | null {
  let n = from;
  while (n && items.includes(n)) n = siblingOf(n, step);
  return n;
}

// -- Node
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

// -- ChildNode (Element and CharacterData)
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

// -- ParentNode (Element and DocumentFragment)
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

// -- insertAdjacent* (Element)
const ADJACENT = ["beforebegin", "afterbegin", "beforeend", "afterend"] as const;
/** Where `insertAdjacent*` puts a node: `[parent, ref]`, or null when there is no parent. */
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

/**
 * A read-only (or verb-backed) SCALAR media property: one host call for the
 * one value (`__vn_media_prop`), not the whole state string. The state string
 * costs every engine getter at once (~0.2 ms on Media Foundation), and a
 * canvas loop that checks `video.readyState` per target per animation frame
 * (Argon's video emoji: eight targets) spent most of an idle window's JS on it.
 * A host without the call, or a test world without it, reads the state string.
 */
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
  if (!isMediaEl(this) || !this.__vnId) return "";
  // A live probe of THIS machine's decoder registry, not a table — a Store
  // codec package installed after we shipped is picked up on the next launch.
  return __vn_media_can_play(this.__vnId, String(type));
});

// -- HTMLImageElement: `src`, `complete`, `naturalWidth` / `naturalHeight`,
//    `currentSrc`, `decode()`.
//
//    The host answers every `src` with `load` or `error` (crates/vn-tree/src/
//    img.rs), painted or not — `new Image()` measuring a picked file is the
//    case that matters — and the dispatcher records the answer on the element
//    before the event runs (`_img`). A stale answer (for a `src` the element
//    has since dropped) is discarded there.
const isImgEl = (x: unknown): x is MirrorElement => isMirrorEl(x) && x.tag === "img";
/** The answer for the element's CURRENT `src`, if it has come. */
const imgAnswer = (el: MirrorElement) => (el._img && el._img.src === el.attrs.src ? el._img : undefined);
/** Elements whose `src` IDL attribute reflects the content attribute the host loads from. */
const SRC_TAGS = new Set(["img", "video", "audio", "source", "iframe", "lottie"]);

defineDomGet(
  "src",
  function (this: MirrorElement) {
    // A reflected URL attribute. A browser resolves it against the document
    // URL; the attribute's own text is returned here (docs/RUNTIME.md §5.6).
    if (!isMirrorEl(this) || !SRC_TAGS.has(this.tag)) return undefined;
    return this.attrs.src ?? "";
  },
  function (this: MirrorElement, v: unknown) {
    // Anything else keeps `src` as a plain expando, as a browser's <div> does.
    if (!isMirrorEl(this) || !SRC_TAGS.has(this.tag)) return ownExpando(this, "src", v);
    patchProp(this, "src", undefined, String(v));
  } as (value: never) => void,
);
defineDomGet(
  "currentSrc",
  function (this: MirrorElement) {
    if (!isImgEl(this)) return undefined;
    return imgAnswer(this)?.ok ? this.attrs.src : "";
  },
);
defineDomGet(
  "complete",
  function (this: MirrorElement) {
    if (!isImgEl(this)) return undefined;
    // HTML: true with no src (or an empty one), and once the request is
    // completely available or broken; false while it is on its way.
    const src = this.attrs.src;
    return src == null || src === "" || !!imgAnswer(this);
  },
);
defineDomGet(
  "naturalWidth",
  function (this: MirrorElement) {
    if (!isImgEl(this)) return undefined;
    return imgAnswer(this)?.w ?? 0;
  },
);
defineDomGet(
  "naturalHeight",
  function (this: MirrorElement) {
    if (!isImgEl(this)) return undefined;
    return imgAnswer(this)?.h ?? 0;
  },
);
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
        (settled() ?? Promise.reject(new DOMException("The source image cannot be decoded.", "EncodingError"))).then(resolve, reject);
      };
      el.addEventListener("load", done);
      el.addEventListener("error", done);
    })
  );
});

// -- `on<event>` IDL handler attributes (`img.onload = fn`, `el.onclick = …`).
//
//    One listener per element and type, added the first time a handler is set
//    and kept in its place in the listener order while the handler changes
//    (HTML "event handler attributes"); `null` removes it. A handler returning
//    `false` cancels the event. On something that is not a mirror element the
//    property stays a plain one, as it always was here (a detached engine
//    `<input>`'s `onchange` is read back by `fireDetached`).
const HANDLER_ATTRS = [
  "abort", "animationcancel", "animationend", "animationiteration", "animationstart", "auxclick",
  "beforeinput", "blur", "cancel", "canplay", "canplaythrough", "change", "click", "close",
  "contextmenu", "copy", "cut", "dblclick", "durationchange", "emptied", "ended", "error", "focus",
  "input", "invalid", "keydown", "keypress", "keyup", "load", "loadeddata", "loadedmetadata",
  "loadstart", "mousedown", "mouseenter", "mouseleave", "mousemove", "mouseout", "mouseover",
  "mouseup", "paste", "pause", "play", "playing", "pointercancel", "pointerdown", "pointerenter",
  "pointerleave", "pointermove", "pointerout", "pointerover", "pointerup", "progress", "ratechange",
  "reset", "scroll", "scrollend", "seeked", "seeking", "select", "selectionchange", "selectstart",
  "submit", "timeupdate", "toggle",
  "transitioncancel", "transitionend", "transitionrun", "transitionstart", "volumechange", "waiting",
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
      // [LegacyTreatNonObjectAsNull]: anything that is not an object is null.
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

/** `MouseEvent.buttons` bit for a `button` index: left 1, right 2, middle 4, back 8, forward 16. */
const BUTTON_BITS = [1, 4, 2, 8, 16];
/**
 * `buttons` — which buttons are DOWN as the event fires. Only the host's press
 * and the moves made while one is held have one down: by a `mouseup`, a
 * `click`, a `dblclick`, an `auxclick` or a `contextmenu` (Chromium on
 * Windows opens the menu on the release) the button is already up. Other
 * buttons held at the same time are not tracked.
 */
function buttonsFor(name: string, c: number): number {
  switch (name) {
    case "mousedown":
    case "pointerdown":
    case "mousemove":
    case "pointermove":
    case "mouseenter":
    case "mouseleave":
    case "pointerenter":
    case "pointerleave":
      return c >= 0 && c < BUTTON_BITS.length ? BUTTON_BITS[c] : 0;
    default:
      return 0;
  }
}
/**
 * `button`: the button whose state CHANGED. A move or an enter/leave changes
 * none — 0 for a MouseEvent (UI Events §3.5), -1 for a PointerEvent (Pointer
 * Events §4.1.1, and `pointercancel` too). The host passes the HELD button in
 * `c` for a move, which is what `buttons` reads.
 */
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
/**
 * `detail` — the click count (UI Events §3.5): the host's `mousedown` /
 * `mouseup` / `click` / `auxclick` carry it above the four modifier bits of
 * `d`; `dblclick` is 2 by definition; everything else is 0, `contextmenu`
 * included (Blink counts only the press, release and click family).
 */
function detailFor(name: string, d: number): number {
  if (name === "click" || name === "auxclick" || name === "mousedown" || name === "mouseup") return (d >>> 4) & 0xff;
  return name === "dblclick" ? 2 : 0;
}

/**
 * An `input` or `beforeinput` event's init from the host's payload: `a` the
 * `EDIT_TYPES` code, `b` 1 while an IME composes, `data` the eighth
 * argument. Both bubble and are composed; `input` never cancels, and
 * `beforeinput` does except while an IME composes.
 */
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

/**
 * A wheel's `clientX` / `clientY`: where the pointer was, which the dispatcher
 * hands `synthesize` as the event's `data` (its `a`/`b` are the deltas). A
 * wheel built without one (a test's) is at 0, 0, as `new WheelEvent` is.
 */
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
    // copy / cut / paste: bubbling, cancelable and composed, carrying the
    // host's DataTransfer (read-only for paste, read/write for copy and cut).
    if ((name === "copy" || name === "cut" || name === "paste") && ClipboardEventClass)
      return new ClipboardEventClass(name, { ...common, composed: true, clipboardData: data ?? null });
    // An image's `load` and `error` are plain Events at the element, neither
    // bubbling nor cancelable; the picture's size is read off the element.
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
      // No PointerEvent class in this engine: a MouseEvent carrying the pointer
      // fields reads identically to everything that consumes one.
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
        if (typeof KeyboardEvent === "function")
          return new KeyboardEvent(name, { ...common, ...M, key: key ?? "" });
        break;
      case "focus":
      case "blur":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "input":
      case "beforeinput":
        // Both bubble and are composed; only `beforeinput` cancels, and not
        // while an IME composes (Input Events Level 1, Chromium's). `data` is
        // the inserted text, `null` for a deletion or a line break.
        if (typeof InputEvent === "function") return new InputEvent(name, inputInit(name, a, b, data));
        break;
      case "compositionstart":
      case "compositionupdate":
      case "compositionend": {
        // Chromium makes all three cancelable, and cancelling one does
        // nothing (the IME has already acted).
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
  const ev = new CustomEvent(name, name === "input" || name === "beforeinput" ? inputInit(name, a, b, data) : common);
  const extra =
    name === "wheel"
      ? { deltaX: a, deltaY: b, ...wheelAt(data), ...M }
      : name === "keydown" || name === "keyup"
        ? { key: key ?? "", ...M }
        : name === "input" || name === "beforeinput"
          ? (({ data: d, inputType, isComposing }) => ({ data: d, inputType, isComposing }))(inputInit(name, a, b, data))
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
                ? { clientX: a, clientY: b, button: buttonFor(name, c), buttons: buttonsFor(name, c), ...POINTER_INIT, ...M }
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
function callListener(
  fn: (this: unknown, ev: VnEvent) => unknown,
  self: unknown,
  ev: VnEvent,
  passive = false,
): void {
  // A passive listener's `preventDefault()` does nothing (`passiveOf`). An own
  // no-op shadows the prototype's for the length of the call.
  const own = passive ? Object.getOwnPropertyDescriptor(ev, "preventDefault") : undefined;
  if (passive) Object.defineProperty(ev, "preventDefault", { value: () => {}, configurable: true, writable: true });
  try {
    fn.call(self, ev);
  } catch (e) {
    (globalThis.queueMicrotask ?? queueMicrotask)(() => {
      throw e;
    });
  } finally {
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
  // The walk is ours, not the engine's, so the engine's own `currentTarget` /
  // `eventPhase` getters would answer "no dispatch" — own properties say what
  // a browser's would at this step.
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
  // snapshot: spec semantics — listeners added during dispatch don't fire now
  for (const entry of [...extra]) {
    if (state.immediate) break;
    if (entry.once) {
      const i = extra.indexOf(entry);
      if (i >= 0) extra.splice(i, 1);
    }
    callListener(entry.fn, node, ev, !!entry.passive);
  }
  if (isMirrorNode(node)) syncListenerFlag(node, name);
  else if (appRoot) syncListenerFlag(appRoot, name);
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
      ? handlers.get(node)?.[name]
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

/**
 * An event whose TARGET is the document (`selectionchange`): the window's
 * capture listeners, then the document's own, then — for one that bubbles —
 * the window's bubble listeners. No element is on its path.
 */
function propagateAtDocument(name: string, ev: VnEvent): boolean {
  const doc = globalThis.document as object | undefined;
  if (!doc) return true;
  Object.defineProperty(ev, "target", { value: doc, configurable: true });
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
  const composed = [doc, globalThis];
  Object.defineProperty(ev, "composedPath", { value: () => composed.slice(), configurable: true });
  const win = globalHandlers.get(globalThis)?.[name];
  const at = globalHandlers.get(doc)?.[name];
  try {
    if (win) invoke(win, "capture", EVENT_PHASE.capturing, ev, globalThis, name, state);
    if (at && !state.stopped) invoke(at, "capture", EVENT_PHASE.atTarget, ev, doc, name, state);
    if (at && !state.stopped) invoke(at, "bubble", EVENT_PHASE.atTarget, ev, doc, name, state);
    if (ev.bubbles && win && !state.stopped) invoke(win, "bubble", EVENT_PHASE.bubbling, ev, globalThis, name, state);
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
    const buckets = handlers.get(n)?.[name];
    if (buckets && bucketsAlive(buckets)) return true;
  }
  // …and the document/window registry, which sits one level above the root.
  for (const m of globalHandlers.values()) {
    const buckets = m[name];
    if (buckets && bucketsAlive(buckets)) return true;
  }
  return false;
}

/**
 * Is there an element on this node's ancestor chain (itself included) whose
 * click does something by itself: an `<a href>`, a `<label>`, an
 * `<input type=file>`?
 */
/** Host events that mean a person just pressed something (see the dispatcher). */
const ACTIVATING_EVENTS = new Set(["keydown", "mousedown", "pointerdown", "mouseup", "pointerup", "click", "auxclick"]);

function activatableOnPath(target: MirrorNode): boolean {
  for (let n: MirrorNode | null = target; n; n = n.parentNode) {
    if (isLabel(n) || isFileInput(n) || isLink(n) || isSubmitButton(n) || isResetButton(n)) return true;
  }
  return false;
}

__vn_set_dispatcher((nodeId, evCode, a, b, c, d, key, data, x, y) => {
  const name = CODE_TO_NAME[evCode];
  const target = nodeById(nodeId);
  if (!name || !target) return true;
  // A wheel's pointer position rides beside its deltas (`VnDispatcher`); it
  // reaches `synthesize` as the event's data.
  if (name === "wheel" && x !== undefined) data = { clientX: x, clientY: y ?? 0 };
  // The host delivers `selectionchange` at the root, whose listener bit the
  // document's listeners arm; a browser fires it AT the document.
  if (name === "selectionchange") return propagateAtDocument(name, synthesize(name, a, b, c, d, key));
  // `document.activeElement` follows the host's focus moves (a click, Tab, a
  // queued `el.focus()`): blur arrives before the next focus, with nothing
  // focused in between. `a` is the move's other node (0: none), which is the
  // events' `relatedTarget`. A handler that moves the focus on cancels the
  // rest of the move, as `focusMoved` does: the host skips the `focus`, and
  // here `focusin` is skipped and `focusout` loses its `relatedTarget`.
  let focusChange = 0;
  let focusRelated: MirrorNode | null = null;
  if (name === "focus" || name === "blur") {
    focusChange = ++focusChanges;
    arriving = null;
    focusRelated = (a && nodeById(a)) || null;
    if (name === "focus") focusedNode = target as MirrorElement;
    else if (focusedNode === target) focusedNode = null;
  }
  // Sticky activation (docs/WINDOWING.md §1.9). The windowing prelude arms it
  // from `keydown`/`mousedown` on the engine's own EventTarget, which gpui
  // events never reach — the host delivers them HERE. The prelude publishes
  // this hook for exactly that reason; without the call, `beforeunload` could
  // never veto a close because the window would never count as interacted with.
  //
  // The host forwards only the events some listener (or an armed bit) asks
  // for, so the press itself often never arrives: a `<button @click>` gets
  // its `click` and no `mousedown`. A host `click`, `mouseup` or
  // `pointerup` is the proof of a press that just happened, so each counts as
  // the activation the press would have given. Only the host's: `el.click()`
  // and `dispatchEvent` never come through here.
  if (ACTIVATING_EVENTS.has(name)) {
    globalThis.__vnStickyActivation?.();
    // …and TRANSIENT activation, which a file picker needs (HTML; Chromium).
    noteActivation(name, key ?? undefined);
  }
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
  // A pick in a select's popup, or a key on a closed one: `a` is the option's
  // index. The selection changes, then `input` and `change` fire.
  if (name === "input" && isSelectEl(target)) return pickOption(target as MirrorElement, a);
  if ((name === "input" || name === "change") && key != null)
    // The storage, not the accessor: the host already holds this value, and
    // the accessor would push it straight back.
    (target as MirrorElement)._value = key;
  // An image's answer: the state `complete` / `naturalWidth` read, BEFORE the
  // event, so `onload` sees the size. An answer for a `src` the element no
  // longer has is stale — a browser would have aborted that request.
  const isImg = target.kind === 1 && (target as MirrorElement).tag === "img";
  if (isImg && (name === "load" || name === "error")) {
    const img = target as MirrorElement;
    if (key !== img.attrs.src) return true;
    img._img = { src: key, ok: name === "load", w: name === "load" ? a : 0, h: name === "load" ? b : 0 };
  }
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
  // A click whose path holds a link (or a label, or a file input) must be
  // BUILT even with no handler on it: a `preventDefault()` anywhere on the
  // path is what cancels the activation, and the walk is the only way to
  // learn that.
  const link = name === "click" && activatableOnPath(target);
  if (link || hasListener(target, name)) {
    const ev = synthesize(name, a, b, c, d, key, data, isImg ? "img" : undefined);
    if (focusChange) withRelatedTarget(ev, focusRelated);
    if (!proceed && ev.preventDefault) ev.preventDefault();
    proceed = propagate(target, name, ev) && proceed;
    if (link && proceed) activate(target, true);
    // A browser opens its context menu now. There is none here, by design —
    // say so once, the first time a page leaves the default to happen.
    if (name === "contextmenu" && proceed) {
      warnCompat(
        "contextmenu:default",
        "an uncancelled `contextmenu` — a browser would open its own context menu here; vue-native has " +
          "none (not even in a text field), so nothing opens. Call e.preventDefault() and show your own",
      );
    }
  }
  // A focused file input clicks itself on Enter (keydown; Chromium's
  // keypress) and on Space (keyup), unless the key event was cancelled —
  // KeyboardClickableInputTypeView.
  if (proceed && isFileInput(target) && ((name === "keydown" && key === "Enter") || (name === "keyup" && key === " "))) {
    (target as MirrorElement).click();
  }
  // Enter in a text field submits its form (implicit submission, HTML
  // §4.10.21.2), unless the keydown was cancelled. Chromium does it from
  // keypress, which a cancelled keydown suppresses: the same rule.
  if (proceed && name === "keydown" && key === "Enter" && blocksImplicitSubmission(target)) {
    implicitSubmit(target as MirrorElement);
  }
  if (derived) {
    for (const dn of derived) {
      if (DERIVED[dn as DerivedEventName].pre) continue;
      // `dblclick` follows only the second click of a pair (detail 2), and
      // only the primary button's — the host's click is always that one.
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
  // L-2 (docs/RUNTIME.md §5.8): the host's text widget takes its own press
  // before any listener runs. The host cancels a press's focus, but not on one.
  if (!proceed && name === "mousedown" && isTextWidget(target)) {
    warnCompat(
      "mousedown-prevent:text-field",
      "`mousedown.preventDefault()` on an <input> or <textarea> — a browser leaves the focus where it was; " +
        "vue-native's text field has already focused itself and placed the caret (docs/RUNTIME.md §5.8 L-2)",
    );
  }
  // W-1 (docs/RUNTIME.md §5.9): a textarea scrolls itself inside the widget.
  if (!proceed && name === "wheel" && (target as MirrorElement).tag === "textarea") {
    warnCompat(
      "wheel-prevent:textarea",
      "`wheel.preventDefault()` on a <textarea> — a browser does not scroll it; vue-native's textarea has " +
        "already scrolled its own content (the boxes around it stay put) (docs/RUNTIME.md §5.9 W-1)",
    );
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
      installStylesheet(globalThis.__VN_CSS__.join("\n"));
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
    // `document`/`window` listeners added at module scope, before there was a
    // root to carry their bit (`addGlobalListener` only syncs one that exists):
    // without this a `document.addEventListener("copy", …)` from before
    // `mount()` was never delivered.
    for (const m of globalHandlers.values()) for (const name of Object.keys(m)) syncListenerFlag(root, name);
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
      __vn_remove(root.__vnId);
      // Detached-but-live nodes: <KeepAlive> creates its cache container with
      // createElement and NEVER inserts or removes it (Vue moves subtrees in
      // and out of it), so nothing above ever tells us to free it. After
      // app.unmount() every node Vue owns is gone from the mirror, so whatever
      // is left is exactly that kind of orphan — free it or the host arena
      // grows by one node per KeepAlive on every hot restart.
      for (const id of Array.from(nodesById.keys())) __vn_remove(id);
      // …and every loose subtree, whether or not script still holds it: the
      // old app is going, and the next one's nodes will reuse these ids.
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
      globalHandlers.clear();
    };
    __vn_log(`Vue ${version} app mounted (vue-native runtime)`);
    return vm;
  };
  return app as VnApp;
}

// ---- runtime-dom's other exports ------------------------------------------
// `import { … } from "vue"` resolves to THIS module, and `vue` is runtime-dom
// plus runtime-core. runtime-core arrives by `export *` at the bottom; these are
// the thirteen names runtime-dom adds on top (research/reports/m26/client-gap/
// B-build-ui.md §1.2 #22). A missing one is a BUILD error in every app that
// names it — `useCssVars` is what the SFC compiler emits for `v-bind()` in a
// `<style>`, so any library using that is unbuildable — so each one exists:
// the real thing where vue-native can do it, an advisory where it cannot.

/** `render(vnode, container)` — this renderer's, exactly as runtime-dom exports its own. */
export const render = renderer.render;

/** runtime-dom's renderer options are its exports too; these are ours. */
export { nodeOps, patchProp };

/**
 * The `--hash-name` custom properties a `v-bind()` in `<style>` reads, written
 * onto the component's root element(s) — runtime-dom's `useCssVars`, line for
 * line (runtime-dom.esm-bundler.js `useCssVars` / `setVarsOnVNode`). vn-css
 * inherits custom properties down the tree (crates/vn-css/src/vars.rs), so the
 * rule `color: var(--abc-color)` under that root resolves as in a browser.
 * Teleported children are found by the `data-v-owner` attribute runtime-core's
 * Teleport stamps, as in the browser.
 */
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
    const MO = (globalThis as { MutationObserver?: new (cb: () => void) => { observe(n: unknown, o: unknown): void; disconnect(): void } })
      .MutationObserver;
    const parent = (instance.subTree?.el as MirrorNode | null)?.parentNode;
    if (MO && parent) {
      const ob = new MO(setVars);
      ob.observe(parent, { childList: true });
      onUnmounted(() => ob.disconnect());
    }
  });
}
/** The instance fields `useCssVars` reads — Vue's internal `ComponentInternalInstance`, narrowed. */
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
/** @vue/shared's `normalizeCssVarValue`. */
function normalizeCssVarValue(value: unknown): string {
  if (value == null) return "initial";
  if (typeof value === "string") return value === "" ? " " : value;
  return String(value);
}

/** `<style module>` classes, by name — runtime-dom's `useCssModule`. */
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

/** A NotSupportedError, as close to the DOM's as the engine allows. */
function notSupported(msg: string): Error {
  return typeof DOMException === "function"
    ? new DOMException(msg, "NotSupportedError")
    : Object.assign(new Error(msg), { name: "NotSupportedError" });
}
const NO_CUSTOM_ELEMENTS =
  "vue-native has no custom elements (no customElements registry, no shadow DOM) — " +
  "use the component directly instead of defining an element for it";

/** runtime-dom's custom-element base class. Constructing one is not possible here. */
export class VueElement {
  constructor() {
    warnCompat("customElement", `VueElement — ${NO_CUSTOM_ELEMENTS}`);
    throw notSupported(`VueElement: ${NO_CUSTOM_ELEMENTS}`);
  }
}
/** Returns an element class, as in the browser; constructing it throws a named error. */
export function defineCustomElement(options: unknown, extraOptions?: unknown): typeof VueElement {
  warnCompat("defineCustomElement", `defineCustomElement() — ${NO_CUSTOM_ELEMENTS}`);
  const def = defineComponent(options as never, extraOptions as never);
  return class VueCustomElement extends VueElement {
    static def = def;
  };
}
export const defineSSRCustomElement = (options: unknown, extraOptions?: unknown): typeof VueElement =>
  defineCustomElement(options, extraOptions);
/** Only a custom element has a host; outside one runtime-dom answers null too. */
export function useHost(): null {
  warnCompat("useHost", `useHost() — ${NO_CUSTOM_ELEMENTS}; it answers null`);
  return null;
}
export function useShadowRoot(): null {
  warnCompat("useShadowRoot", `useShadowRoot() — ${NO_CUSTOM_ELEMENTS}; it answers null`);
  return null;
}
/** There is no server-rendered markup in a vue-native window: `createApp`, said out loud. */
export function createSSRApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): VnApp {
  warnCompat(
    "createSSRApp",
    "createSSRApp() — a vue-native window has no server-rendered markup to hydrate; it behaves as createApp()",
  );
  return createApp(rootComponent, rootProps);
}
export const hydrate: typeof render = (vnode, container) => {
  warnCompat("hydrate", "hydrate() — nothing to hydrate in a vue-native window; it renders from scratch, as render() does");
  render(vnode, container);
};
/** SSR's directive hooks: there is no SSR here, so there is nothing to initialize. */
export const initDirectivesForSSR = (): void => {};

// ---- missing globals (compat roadmap G5) ----------------------------------
// Each of these is a ReferenceError today, i.e. strictly worse than a wrong
// answer: reka-ui's FocusScope constructs a MutationObserver inside a
// SETUP-TIME watchEffect, so every modal (Dialog, Popover, Menu, Select) dies
// before it renders. They are installed on the global unconditionally when the
// engine does not already have them.

// A hole includes a PREVIOUS runtime's install (see `REALM`): after a
// same-realm reload this runtime's class replaces the old one, whose state
// (queued polls, the old mirror tree) is dead.
const defineGlobal = (name: string, value: unknown) => {
  if (vacant(globalThis, name)) {
    Object.defineProperty(globalThis, name, { value: mark(value), configurable: true, writable: true });
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

/// The values `CSS.supports` says no to for a property it otherwise knows.
/// `linear(…)` easing: the parser (lightningcss alpha.72) has no such
/// function, so a stylesheet drops the declaration and `el.animate()` refuses
/// it (src/waapi.ts). Apps probe for it and fall back — Argon's counter
/// (Digit.vue) picks a `cubic-bezier` when this answers no. Not
/// `linear-gradient(`.
const CSS_UNSUPPORTED_VALUE = /(?:^|[^\w-])linear\(/i;

// -- MutationObserver, wired to the REAL mirror-tree mutations.
// Records are queued at the nodeOps/patchProp call sites and delivered on a
// microtask, which is exactly a browser's timing (the observer callback runs at
// the end of the current task, before rendering).
/** One `MutationRecord`, with exactly the fields this runtime can fill in. */
interface MutationRecordLike {
  type: "childList" | "attributes";
  target: MirrorNode;
  /** Static NodeLists, as a browser's are. */
  addedNodes: MirrorCollection<MirrorNode>;
  removedNodes: MirrorCollection<MirrorNode>;
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
  // An `id`, `name` or `class` change moves elements in and out of live collections.
  domEpoch++;
  if (type === "attributes" && !noAttrNodes) attrChanged(target as MirrorElement, extra!.attributeName!, extra!.oldValue ?? null);
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

// -- ResizeObserver and IntersectionObserver, after layout and before paint.
//
// HTML "update the rendering" (§8.1.7.3) runs both where the document's
// layout is fresh: after the animation frame callbacks it recalculates styles
// and updates layout, runs ResizeObserver's loop (Resize Observer §3.4.4:
// gather the active observations deeper than `depth`, broadcast them, lay out
// again, gather again at the shallowest depth just broadcast, until nothing
// is active; an observation skipped on the way raises the "loop completed
// with undelivered notifications" error), then the update intersection
// observations steps, and then paints. So a ResizeObserver callback sees the
// size the frame is about to paint, and what it changes is painted in that
// same frame.
//
// The host runs this once per frame as one step (`layoutStep` below, through
// `__vn_set_layout_hook`; crates/vn-tree/src/observe.rs): it draws the frame
// first when the frame is going to draw, calls `layoutStep` once per op, drains
// microtasks after every call (a browser's microtask checkpoint after each
// callback, where Vue's scheduler flushes the patch a callback asked for), and
// lays out again between rounds when the callbacks changed anything. A frame
// that lays nothing out runs no step, unless an `observe()` is waiting for its
// first report.
//
// Until m26 fixes-lag-1 both observers were POLLED from requestAnimationFrame,
// which runs before the frame's layout: a callback saw the previous frame's
// size, its response painted a frame late (the "one-frame jump"), and a target
// mounted in the wave, which had no box yet, forced a layout from inside the
// rAF.
//
// It has to be real: @floating-ui's `autoUpdate` positions an element ONCE and
// then waits for a ResizeObserver to tell it the element has been laid out. An
// inert observer means every popover, tooltip, dropdown and select keeps the
// position computed from its pre-layout 0×0 box — which is to say none of them
// is ever placed at all.
//
// Divergence, said in docs/RUNTIME.md §5.1: `contentRect`, `contentBoxSize`
// and `borderBoxSize` are all the BORDER box, as `clientWidth` is here. The
// cascade's padding and border widths are not published per node, so the
// content box is not known; `contentRect` is at (0, 0), as for a box without
// padding.
/** One `ResizeObserverEntry`, with the box fields every consumer reads. */
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
/** The instance shape the step below drives. */
interface ResizeObserverLike {
  _callback: (this: ResizeObserverLike, entries: ResizeEntryLike[], ob: ResizeObserverLike) => void;
  /**
   * The observation targets, with the size last REPORTED for each; `null` =
   * nothing reported yet, so the next gather reports it, as `observe()` does
   * in a browser, a 0×0 or undisplayed target included.
   */
  _targets: Map<MirrorElement, ResizeSize | null>;
  /** What the last gather found active: the target, its size, its depth. */
  _active: [MirrorElement, ResizeSize, number][];
  /** The last gather skipped an active observation at or above its depth. */
  _skipped: boolean;
}

/** The document's ResizeObservers that have observations, in the order they observed first. */
const resizeObservers: ResizeObserverLike[] = [];
/** The observers the current round calls back, in order. */
let resizeDue: ResizeObserverLike[] = [];
/** The shallowest target depth the current round has broadcast. */
let resizeDepth = Infinity;

/**
 * Resize Observer "calculate depth for node": the nodes on the path from it
 * to the root of its tree, itself included, so a root (or a detached node) is
 * at depth 1 and the first gather, at depth 0, takes every target.
 */
function nodeDepth(node: MirrorNode): number {
  let d = 1;
  for (let n = node.parentNode; n; n = n.parentNode) d++;
  return d;
}

/** Resize Observer "gather active observations at depth". Answers how many observers have entries. */
function gatherResizes(depth: number): number {
  resizeDue = [];
  resizeDepth = Infinity;
  for (const ob of resizeObservers) {
    ob._active = [];
    ob._skipped = false;
    for (const [target, last] of ob._targets) {
      // The layout box (transforms ignored, as the spec's is). The step runs
      // after the frame's layout, so nothing is owed and no read forces one.
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

/** One observer of "broadcast active resize observations". Answers how many are left this round. */
function broadcastNextResize(): number {
  const ob = resizeDue.shift();
  // A callback earlier in the round may have disconnected it.
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
      // "Report the exception", after the callback, as a listener's is.
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
    Object.freeze(branded("ResizeObserverSize", { inlineSize: Math.round(size.w * dpr), blockSize: Math.round(size.h * dpr) })),
  ]);
  const contentRect = asDomRect({
    x: 0, y: 0, width: size.w, height: size.h,
    top: 0, left: 0, right: size.w, bottom: size.h,
    toJSON() { return this; },
  });
  return branded("ResizeObserverEntry", { target, contentRect, borderBoxSize: box, contentBoxSize: box, devicePixelContentBoxSize: device });
}

const RESIZE_LOOP_MESSAGE = "ResizeObserver loop completed with undelivered notifications.";

/**
 * Resize Observer "deliver resize loop error notification": HTML "report an
 * exception" with no Error object, at the global. An `ErrorEvent` named
 * `error`, cancelable, for `addEventListener("error")`, and `window.onerror`
 * called the HTML way, `(message, source, lineno, colno, error)`. The engine
 * calls an `on<type>` attribute with the event alone, which would hand an
 * error reporter an event object for a message (and Sentry's default filter,
 * which drops this message, would not recognise it), so the attribute is held
 * back from the engine's dispatch and called here. Nothing is printed: a
 * browser does not print this one either.
 */
function deliverResizeLoopError(): void {
  const g = globalThis as {
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
/** The engine's `ErrorEvent`, or ours, installed on the global when it lacks one. */
const ErrorEventClass: (new (type: string, init?: ErrorEventInit) => VnEvent) | null = (() => {
  const g = globalThis as { ErrorEvent?: unknown };
  if (typeof g.ErrorEvent === "function" && !vacant(g, "ErrorEvent"))
    return g.ErrorEvent as new (type: string, init?: ErrorEventInit) => VnEvent;
  if (typeof Event !== "function") return null;
  const fields = new WeakMap<object, Required<Pick<ErrorEventInit, "message" | "filename" | "lineno" | "colno">> & { error: unknown }>();
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

// The step's ops: the numbers are the protocol with crates/vn-tree/src/observe.rs.
const LAYOUT_START = 0;
const LAYOUT_NEXT = 1;
const LAYOUT_AGAIN = 2;
const LAYOUT_FINISH = 3;

/** Is anything observed, by either observer? */
function observingLayout(): boolean {
  for (const ob of resizeObservers) if (ob._targets.size) return true;
  for (const ob of intersectionObservers) if (ob._targets.size) return true;
  return false;
}

/**
 * The runtime's half of the step (see the section header): one op per host
 * call. Anything that throws out of here would lose the step's state, so the
 * callbacks' exceptions are reported from a microtask instead.
 */
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
      // Observers that observe nothing leave the list: a browser keeps them
      // (weakly), and nothing can tell the difference.
      for (let i = resizeObservers.length - 1; i >= 0; i--) if (!resizeObservers[i]!._targets.size) resizeObservers.splice(i, 1);
      return raised | (observingLayout() ? 2 : 0);
    }
    default:
      return 0;
  }
}

// The host runs the step; one without the two functions (an older shell, a
// test harness that leaves them out) gets the step from requestAnimationFrame,
// which is BEFORE layout, with no re-layout between rounds, and while nothing
// changes it re-polls on a 250 ms timer instead of every frame. A safety net,
// not the mechanism.
const hostSteps = typeof __vn_set_layout_hook === "function" && typeof __vn_layout_watch === "function";
if (hostSteps) __vn_set_layout_hook(layoutStep);
const QUIET_STEPS = 2;
let fallbackQueued = false;
let fallbackQuiet = 0;
let fallbackTimer: unknown = null;
const layoutTimers = globalThis as { setTimeout?: (cb: () => void, ms: number) => unknown; clearTimeout?: (id: unknown) => void };

/** An `observe()`: the next frame reports the target's first state, whatever it lays out. */
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
      // A second observe() of a target replaces its observation: reported anew.
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

// -- IntersectionObserver, from the host's painted bounds.
//
// redjs ships an INERT one ("nothing intersects in a game host"), and an inert
// IntersectionObserver is not a harmless one in an app: every `visible` /
// `visibleOnce` variant of @vueuse/motion waits for `isIntersecting` before it
// animates in, so an element that starts at `opacity: 0` (the Argon client's
// whole login card, research/reports/m26/client-gap.md §1 #12) stays invisible
// forever; lazy loaders never load; off-screen animations never pause.
//
// This one is the W3C algorithm, run where HTML runs it: "run the update
// intersection observations steps" in the frame's step after layout (the
// FINISH op above), which, for each target, intersects its painted rectangle
// with the root's (the viewport, or the `root` element's box, grown by
// `rootMargin`), maps the ratio onto `thresholds`, and queues an entry when the
// threshold index or `isIntersecting` moves, so the first update always
// reports, as `observe()` does in a browser. The callbacks run from a task
// ("queue an intersection observer task"), after the frame has painted.
//
// The rectangles are where the boxes were DRAWN, transforms applied: the spec
// takes the target's from getBoundingClientRect() ("getting the bounding box"),
// not from its layout box as ResizeObserver does.
//
// Divergence, deliberate: the intersection is clipped by the ROOT only, not by
// every scrolling ancestor between target and root, because answering "which
// ancestors clip" per target per frame would cost a host call per ancestor; a
// target scrolled out of an inner scroller that is still inside the window
// reads as intersecting.
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
/** The last update queued entries. */
let intersectionsQueued = false;

/** HTML "run the update intersection observations steps", for the document. */
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

/** HTML "notify intersection observers". */
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
  x, y, width, height, top: y, left: x, right: x + width, bottom: y + height,
  toJSON() { return this; },
});

/** `rootMargin` / `scrollMargin`: one to four `<n>px` / `<n>%`, CSS margin order. */
function parseIntersectionMargin(text: unknown, what: string): { v: number; pct: boolean }[] {
  const parts = String(text ?? "0px").trim().split(/\s+/).filter(Boolean);
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
    throw new DOMException(`Failed to construct 'IntersectionObserver': ${what} must have 1 to 4 values.`, "SyntaxError");
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
  constructor(callback: (entries: unknown[], ob: VnIntersectionObserver) => void, options: Record<string, unknown> = {}) {
    if (typeof callback !== "function") {
      throw new TypeError("Failed to construct 'IntersectionObserver': parameter 1 is not of type 'Function'.");
    }
    const opts = options ?? {};
    const root = opts.root ?? null;
    // A Document root is the implicit root's document, i.e. the viewport.
    const isDocument = !!root && (root === (globalThis as { document?: unknown }).document || (root as { nodeType?: number }).nodeType === 9);
    if (root !== null && !isDocument && typeof (root as { nodeType?: number }).nodeType !== "number") {
      throw new TypeError("Failed to construct 'IntersectionObserver': root must be an Element, a Document or null.");
    }
    const raw = opts.threshold === undefined ? [0] : Array.isArray(opts.threshold) ? opts.threshold : [opts.threshold];
    const thresholds = (raw as unknown[]).map(Number);
    for (const t of thresholds) {
      if (!(t >= 0 && t <= 1)) {
        throw new RangeError("Failed to construct 'IntersectionObserver': Threshold values must be numbers between 0 and 1");
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
      throw new TypeError("Failed to execute 'observe' on 'IntersectionObserver': parameter 1 is not of type 'Element'.");
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
  /** The root intersection rectangle, `rootMargin` applied; null when the root has no box. */
  _rootBounds(): MirrorRect | null {
    let r: MirrorRect;
    if (this.root) {
      if (!boundsOf(this.root)) return null;
      r = paintedRectOf(this.root);
    } else {
      if (!appRoot) return null;
      r = viewportRect();
    }
    const [t, rt, b, l] = this._margin as [{ v: number; pct: boolean }, { v: number; pct: boolean }, { v: number; pct: boolean }, { v: number; pct: boolean }];
    const px = (m: { v: number; pct: boolean }, of: number) => (m.pct ? (m.v / 100) * of : m.v);
    const top = r.top - px(t, r.height);
    const left = r.left - px(l, r.width);
    const right = r.right + px(rt, r.width);
    const bottom = r.bottom + px(b, r.height);
    return ioRect(left, top, Math.max(0, right - left), Math.max(0, bottom - top));
  }
  /** "Run the update intersection observations steps" for this observer. `true` when it queued entries. */
  _update(time: number): boolean {
    const rootBounds = this._rootBounds();
    let queued = false;
    for (const [target, state] of this._targets) {
      // After the frame's layout: a target with no box is not being rendered.
      const painted = !!boundsOf(target);
      let isIntersecting = false;
      let thresholdIndex = 0;
      let ratio = 0;
      let targetRect = ioRect(0, 0, 0, 0);
      let intersectionRect = ioRect(0, 0, 0, 0);
      const descendant = (() => {
        if (!this.root) return true;
        for (let n: MirrorNode | null | undefined = target.parentNode; n; n = n.parentNode) if (n === this.root) return true;
        return false;
      })();
      if (painted && rootBounds && descendant) {
        targetRect = paintedRectOf(target);
        const l = Math.max(targetRect.left, rootBounds.left);
        const t = Math.max(targetRect.top, rootBounds.top);
        const r = Math.min(targetRect.right, rootBounds.right);
        const b = Math.min(targetRect.bottom, rootBounds.bottom);
        // Edge-adjacent counts: a zero-area overlap still intersects (spec).
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
  /** IntersectionObserver v2: visibility is never tracked here. */
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

// -- DocumentFragment. Vue's Teleport accepts one as a target, and reka-ui's
//    Select CONSTRUCTS one even while closed — a missing class kills it at
//    setup. Ours is a detached mirror element, so a Teleport into it renders
//    into the retained tree without being painted, which is what a browser's
//    off-document fragment does too.
//    Script holds it (it is loose, see `looseCreated`), and inserting it
//    inserts its children instead (`insertAll`), leaving it empty.
{
  class DocumentFragment {
    constructor() {
      const node = mk(__vn_create_element(I("fragment")), 1, "fragment");
      Object.setPrototypeOf(node, DocumentFragment.prototype);
      fragments.add(node);
      // A constructor that hands back a DIFFERENT object — `new
      // DocumentFragment()` really is a detached mirror element, which is the
      // whole point. The double cast is what says so.
      return looseCreated(node) as unknown as DocumentFragment;
    }
  }
  // The element surface (querySelector, append, childNodes…) through the
  // chain, so `instanceof DocumentFragment` holds as well.
  Object.setPrototypeOf(DocumentFragment.prototype, ElementProto);
  for (const [k, v] of [
    ["nodeType", 11],
    ["nodeName", "#document-fragment"],
  ] as const) {
    Object.defineProperty(DocumentFragment.prototype, k, { get: () => v, configurable: true });
  }
  defineGlobal("DocumentFragment", DocumentFragment);
}

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
//
//    Every HTML element interface Chrome has for a tag, by `tagName` (a list
//    where one interface serves several tags). HTMLMediaElement comes before
//    the two that extend it (`TAG_PARENTS`). `<audio>` matters most: the
//    client's AudioManager tests `instanceof HTMLAudioElement` inside a
//    MutationObserver callback, so a missing global threw on every mutation
//    (research/reports/m26/dom-globals-1.md).
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
/** Tag classes that extend another tag class rather than HTMLElement. */
const TAG_PARENTS: Record<string, string> = {
  HTMLVideoElement: "HTMLMediaElement",
  HTMLAudioElement: "HTMLMediaElement",
};
/** HTMLMediaElement's constants (HTML §4.8.11): `readyState` and `networkState` values. */
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
// -- Form-control `value` / `checked` are real accessors.
//
// reka-ui's VisuallyHiddenInputBubble (Checkbox, Switch, RadioGroup inside a
// form) and BubbleSelect do what React taught a generation of libraries:
//
//   Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, v)
//
// and then dispatch `input` and `change` themselves. Against tag classes with
// no `value` member that is `undefined.set`, and the Argon client died right
// after sign-in. Two halves: a mirror element's own `value` is an accessor
// that reaches the host (below), and the tag classes carry OWN accessors that
// delegate to it (`TAG_ACCESSORS`), refusing a receiver of the wrong kind with
// the browser's "Illegal invocation".

/** Is `x` a mirror element (not a detached engine element, not a stranger)? */
function isMirrorEl(x: unknown): x is MirrorElement {
  return !!x && typeof x === "object" && Object.prototype.hasOwnProperty.call(x, "_value");
}

/** Tags whose `value` is the live value of a control the host draws. */
const VALUE_TAGS = new Set(["input", "textarea", "select"]);

/**
 * `el.value = v` on a form control: the mirror and the host, on EVERY
 * assignment. The host ticks a write counter even for the same string, so an
 * assignment re-forces a field the user has typed over since, as in a
 * browser. No `input` / `change` fires (HTML: setting the IDL attribute is not
 * a user edit). `<input>` / `<textarea>` take null as "" ([LegacyNullToEmptyString]).
 */
function writeControlValue(el: MirrorElement, v: unknown): void {
  const s = v == null && el.tag !== "select" ? "" : String(v);
  el._value = s;
  __vn_set_prop(el.__vnId, I("value"), s);
}

/** Not a mirror: the plain own property it would have had with no accessor here. */
function ownExpando(self: object, key: string, v: unknown): void {
  Object.defineProperty(self, key, { value: v, writable: true, enumerable: true, configurable: true });
}

defineDomGet(
  "value",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    // A select's is its first selected option's; an option's is its `value`
    // attribute, else its text (the select model, below).
    if (isSelectEl(this)) return selectValue(this);
    if (isOptionEl(this)) return optionValue(this);
    return this._value;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "value", v);
    // Selects the first option with that value (none: nothing is selected).
    if (isSelectEl(this)) setSelectValue(this, String(v));
    // Sets the `value` attribute, as HTMLOptionElement's setter does.
    else if (isOptionEl(this)) patchAttribute(this, "value", String(v));
    else if (VALUE_TAGS.has(this.tag)) writeControlValue(this, v);
    // Any other element: an expando, kept as given, as on a browser's <div>.
    else this._value = v as string;
  } as (value: never) => void,
);

/** `defaultChecked`: the `checked` attribute's presence ("false" is Vue's spelling of absent). */
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
    // Every input renders as a text field today (no native checkbox), so the
    // host keeps this for whatever reads it; the mirror is what JS sees.
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
    if (!isMirrorEl(this) || (this.tag !== "input" && this.tag !== "textarea")) return ownExpando(this, "defaultValue", v);
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

// ---- <select> and <option>: the selection model -----------------------------
//
// HTML §4.10.7 and §4.10.10, kept in the mirror: each option's selectedness
// and dirtiness, the selectedness setting algorithm whenever a select's list
// of options or its `multiple` / `size` changes, and the members scripts and
// `v-model` use (`value`, `selectedIndex`, `options`, `selectedOptions`,
// `option.selected`, `option.index`, `option.text`, `option.label`).
//
// The host draws the select (crates/vn-tree/src/select.rs) from the reserved
// `:selected` prop, the selected index, and reports a user's pick as the
// select's `input` host event with the option's index (`pickOption`), which
// changes the selection and fires `input` and `change`, as Chromium does.
// The host draws a drop-down whatever `multiple` and `size` say
// (`adviseListBox`).

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

/** A present boolean attribute ("false" is Vue's spelling of absent). */
const attrOn = (el: MirrorElement, k: string): boolean => el.attrs[k] != null && el.attrs[k] !== "false";

/** The select whose list of options `n` (an option) is in: its parent, or its optgroup parent's parent. */
function ownerSelect(n: MirrorNode): MirrorElement | null {
  const p = n.parentNode;
  if (isSelectEl(p)) return p;
  if (p && isOptGroup(p) && isSelectEl(p.parentNode)) return p.parentNode;
  return null;
}

/** HTML's list of options: the option children, and the option children of the optgroup children, in tree order. */
function selectOptions(sel: MirrorElement): MirrorElement[] {
  const out: MirrorElement[] = [];
  for (const c of sel.childNodes) {
    if (isOptionEl(c)) out.push(c as MirrorElement);
    else if (isOptGroup(c)) for (const g of c.childNodes) if (isOptionEl(g)) out.push(g as MirrorElement);
  }
  return out;
}

const isMultipleSelect = (sel: MirrorElement): boolean => attrOn(sel, "multiple");

/** The display size: `size` if positive, else 4 for a multiple select and 1 for the rest. */
function displaySize(sel: MirrorElement): number {
  const n = parseInt(sel.attrs.size ?? "", 10);
  return n > 0 ? n : isMultipleSelect(sel) ? 4 : 1;
}

/** Disabled itself, or in a disabled optgroup. */
function optionDisabled(o: MirrorElement): boolean {
  return attrOn(o, "disabled") || (!!o.parentNode && isOptGroup(o.parentNode) && attrOn(o.parentNode, "disabled"));
}

/** The text with ASCII white space stripped and collapsed. */
const stripCollapse = (s: string): string => s.split(/[ \t\n\f\r]+/).filter(Boolean).join(" ");

function optionText(o: MirrorElement): string {
  return stripCollapse(String((o as unknown as { textContent: unknown }).textContent ?? ""));
}

/** `option.value`: the `value` attribute, else the text. */
function optionValue(o: MirrorElement): string {
  return o.attrs.value ?? optionText(o);
}

/** What `v-model` reads off an option: the value Vue bound, else `option.value` (runtime-dom's `getValue`). */
function modelValueOf(o: MirrorElement): unknown {
  return Object.prototype.hasOwnProperty.call(o, "_rawValue") ? o._rawValue : optionValue(o);
}

function selectedIndexOf(sel: MirrorElement): number {
  const opts = selectOptions(sel);
  for (let i = 0; i < opts.length; i++) if (opts[i]!._selected) return i;
  return -1;
}

/** `select.value`: the first selected option's value, "" with none. */
function selectValue(sel: MirrorElement): string {
  const o = selectOptions(sel).find((x) => x._selected);
  return o ? optionValue(o) : "";
}

/** Tells the host which option to show, when that changed. */
function showSelected(sel: MirrorElement): void {
  const ix = selectedIndexOf(sel);
  if (sel._shown === ix) return;
  sel._shown = ix;
  __vn_set_prop(sel.__vnId, I(":selected"), String(ix));
}

/**
 * HTML's "selectedness setting algorithm": a single drop-down with nothing
 * selected selects its first enabled option, and a single select with
 * several selected keeps the last. Then the host is told.
 */
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

/** `n`'s child list changed: if that is a select's list of options, its selection settles again. */
function optionsChangedIn(n: MirrorNode | null): void {
  if (!n || n.kind !== 1) return;
  const tag = (n as MirrorElement).tag;
  if (tag === "select") {
    if (isSelectEl(n)) resetSelect(n);
  } else if (tag === "optgroup" && isSelectEl(n.parentNode)) {
    resetSelect(n.parentNode as MirrorElement);
  }
}

/** An option's selectedness just changed: a selected one deselects the others of a single select, then the select settles. */
function selectednessChanged(o: MirrorElement): void {
  const sel = ownerSelect(o);
  if (!sel) return;
  if (o._selected && !isMultipleSelect(sel)) for (const x of selectOptions(sel)) if (x !== o) x._selected = false;
  resetSelect(sel);
}

/** `option.selected = on`: the selectedness, dirty (HTML §4.10.10). */
function setOptionSelected(o: MirrorElement, on: boolean): void {
  o._dirty = true;
  o._selected = on;
  selectednessChanged(o);
}

/** `select.selectedIndex = ix`: every option deselected, then the one at `ix`, if any, selected and dirty. */
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

/** `select.value = v`: every option deselected, then the first whose value is `v`, if any, selected and dirty. */
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

/**
 * A select's or an option's `value` prop. runtime-dom assigns the PROPERTY:
 * on a select that selects the first option with the value (and only when
 * the value read differs); on an option it is the `value` attribute, and the
 * value as bound is kept for `v-model` (`_rawValue`, runtime-dom's `_value`),
 * so `<option :value="1">` gives the model a number, and `:value="obj"` the
 * object.
 */
function patchSelectValue(el: MirrorElement, next: unknown): void {
  if (isOptionEl(el)) {
    el._rawValue = next;
    patchAttribute(el, "value", next == null ? null : String(next));
    return;
  }
  const v = next == null ? "" : String(next);
  if (selectValue(el) !== v) setSelectValue(el, v);
}

/**
 * The user picked option `ix` (the host's `input` on the select): it alone is
 * selected, and `input` then `change` fire, as Chromium fires them for a
 * drop-down. Nothing happens for the option already selected, a disabled one,
 * or a disabled select.
 */
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

/** `multiple` or a `size` above 1: Chromium draws a list box, which the host does not. */
function adviseListBox(sel: MirrorElement): void {
  if (!isMultipleSelect(sel) && displaySize(sel) === 1) return;
  warnCompat(
    "select:list-box",
    "<select multiple> / <select size>: drawn as a drop-down here (the first selected option, and a popup that " +
      "picks one option at a time), not as Chromium's list box; the selection model, selectedOptions and " +
      "v-model with an array work",
  );
}

/** A select's own members (per element: `type` and `length` mean other things on other elements). */
function defineSelectMembers(sel: MirrorElement): void {
  const options = liveCollection<MirrorElement>(HTMLCollectionProto, true, () => selectOptions(sel));
  const acc = (get: () => unknown, set?: (v: unknown) => void): PropertyDescriptor => ({
    get,
    set,
    configurable: true,
    enumerable: true,
  });
  Object.defineProperties(sel, {
    options: acc(() => options),
    selectedOptions: acc(() => staticList(HTMLCollectionProto, selectOptions(sel).filter((o) => o._selected))),
    selectedIndex: acc(
      () => selectedIndexOf(sel),
      (v) => setSelectedIndex(sel, toLong(v)),
    ),
    length: acc(() => selectOptions(sel).length),
    item: { value: (i: unknown) => selectOptions(sel)[toLong(i)] ?? null, configurable: true, writable: true },
  });
}

/** An option's own members. */
function defineOptionMembers(o: MirrorElement): void {
  o._selected = false;
  o._dirty = false;
  const acc = (get: () => unknown, set?: (v: unknown) => void): PropertyDescriptor => ({
    get,
    set,
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

/**
 * The members each tag class's PROTOTYPE carries as own accessors, for
 * `Object.getOwnPropertyDescriptor(X.prototype, name).get/.set.call(el)`.
 * Each delegates to the element's own path (the accessors above, or a file
 * input's own `value`), so there is one behaviour however it is reached.
 */
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
  // HTMLElement.prototype (the mirrors' own), or the tag class this one extends.
  const parentName = TAG_PARENTS[name];
  const parentProto =
    (parentName && (globalThis as Record<string, { prototype?: object } | undefined>)[parentName]?.prototype) || ElementProto;
  if (typeof existing === "function" && !ours) {
    // The ENGINE's class — redjs ships HTMLImageElement, the offscreen image
    // `new Image()` and `document.createElement("img")` make (and a canvas
    // draws). It knows nothing of mirror elements, so `<img>` in the tree was
    // not `instanceof HTMLImageElement`; it is now, as well as the engine's own.
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
  // Mirror nodes are all plain HTMLElement, so `instanceof` is answered by the
  // tag name instead of by the prototype chain — which is the ONLY way to make
  // `el instanceof HTMLInputElement` true for a real `<input>` here. The
  // prototype is still on HTMLElement's chain, so `HTMLInputElement.prototype.focus`
  // is HTMLElement's, as in a browser.
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
      if (!self || typeof self !== "object" || (self as MirrorElement).tagName !== tag || klass.prototype.isPrototypeOf(self)) {
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

// -- WebIDL interface objects the realm lacks.
//
// Code names these interfaces without constructing them, and a missing one is
// a ReferenceError at CLASS DEFINITION: tsyringe-style DI emits
// `__metadata("design:paramtypes", [Window])` for a constructor parameter typed
// `Window` (the Xsolla pay SDK's `psdk-*` elements), and the Argon home screen
// died on "Window is not defined". Each one here is what a browser's is: an
// interface object whose constructor throws "Illegal constructor" (unless the
// browser lets you construct it), with a prototype on the right parent, and an
// `instanceof` that is true for the objects this realm already has — which it
// never built, so the answer is by identity or by kind (`Symbol.hasInstance`).

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

/**
 * An interface whose prototype object already exists: NodeList.prototype and
 * its kin, made before the first mirror. `instanceof` is the prototype chain's
 * answer. When the engine has an interface of the same name, ours goes under
 * it, so `instanceof` holds for both.
 */
function adoptInterface(
  name: string,
  proto: object,
  parent: Iface | null,
  /** Objects off the chain that are instances all the same (`childNodes`). */
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

// -- Node: an interface of its own (DOM §4.4), the parent of Element,
//    CharacterData, Attr and Document. redjs aliases `Node` to `Element`, so
//    `Node.TEXT_NODE` was undefined (emojix compares `node.nodeType` with it)
//    and a text node was not `instanceof Node` (the client's EnterText.vue,
//    DOMPurify). The engine's Element.prototype is re-parented onto it, and so
//    are the text mirrors' prototype and Attr's (below).
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
  // A real Node (not the alias, not a previous runtime's) stays.
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
  // WebIDL constants: on the interface object and its prototype, read-only.
  for (const [k, v] of NODE_CONSTANTS) {
    Object.defineProperty(iface, k, { value: v, enumerable: true });
    Object.defineProperty(proto, k, { value: v, enumerable: true });
  }
  // The chain answers for elements and text; `document` (redjs's own object)
  // and a mirror whose chain a page re-parented answer by kind.
  Object.defineProperty(iface, Symbol.hasInstance, {
    value: (x: unknown) =>
      !!x && typeof x === "object" && (proto.isPrototypeOf(x) || isMirrorNode(x) || isDocument(x) || x instanceof AttrImpl),
  });
  try {
    Object.defineProperty(globalThis, "Node", { value: mark(iface), configurable: true, writable: true });
  } catch {
    warnCompat("global:Node", "Node could not be installed over the engine's own: Node.TEXT_NODE and the other constants are missing");
    return null;
  }
  // The engine's Element.prototype, under Node.prototype. A realm with no
  // Element of its own (the test harness) gets one below.
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
// A realm with no Element at all (the bun test harness): Element and
// HTMLElement are made here, with the mirrors' prototype as
// HTMLElement.prototype, as it is redjs's.
const ElementIface: Iface | null = (() => {
  if (!vacant(globalThis, "Element")) return (theGlobal("Element") as Iface | undefined) ?? null;
  const iface = defineInterface("Element", () => false, NodeIface) ?? null;
  if (iface) adoptInterface("HTMLElement", ElementProto, iface);
  return iface;
})();

/**
 * `instanceof` for an element interface whose prototype all mirror elements
 * share (they are all on HTMLElement.prototype): a mirror answers by its
 * namespace, anything else by the prototype chain, as before.
 */
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
// An `<svg>` or `<math>` element is not an HTMLElement, and a fragment is
// not an element at all, although each is on HTMLElement.prototype here.
wrapElementHasInstance(theGlobal("HTMLElement"), (el) => !el._ns && !fragments.has(el));

// `window` is the global (or the global's proxy, which is what `window` names).
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

// Text and comment mirrors share a prototype; `kind` tells them apart.
const mirrorKind = (x: unknown) =>
  !!x && typeof x === "object" && Object.getPrototypeOf(x) === TextProto ? (x as MirrorNode).kind : 0;
const CharacterDataIface = defineInterface("CharacterData", (x) => mirrorKind(x) === 2 || mirrorKind(x) === 3, NodeIface);
// Text and comment mirrors inherit Node's constants and EventTarget's methods,
// as a browser's Text and Comment do.
if (CharacterDataIface && Object.getPrototypeOf(TextProto) !== CharacterDataIface.prototype) {
  try {
    Object.setPrototypeOf(TextProto, CharacterDataIface.prototype);
  } catch {
    /* `instanceof` still holds, by kind */
  }
}
defineInterface("Text", (x) => mirrorKind(x) === 2, CharacterDataIface ?? null, (data) =>
  looseCreated(nodeOps.createText(data === undefined ? "" : String(data))),
);
defineInterface("Comment", (x) => mirrorKind(x) === 3, CharacterDataIface ?? null, (data) => {
  const c = looseCreated(nodeOps.createComment(data === undefined ? "" : String(data)));
  c.text = data === undefined ? "" : String(data);
  return c;
});
// No shadow DOM here, so nothing is a ShadowRoot.
defineInterface("ShadowRoot", () => false, theGlobal("DocumentFragment") as Iface | null);
// `el.attributes` and its Attr nodes (DOM §4.9). `document.createAttribute` makes one.
defineInterface("Attr", (x) => x instanceof AttrImpl, NodeIface);
if (NodeIface && Object.getPrototypeOf(AttrImpl.prototype) !== NodeIface.prototype) {
  Object.setPrototypeOf(AttrImpl.prototype, NodeIface.prototype);
}
defineInterface("NamedNodeMap", (x) => x instanceof NamedNodeMapImpl, null);
// `el.style` and `getComputedStyle()` answers register themselves.
const styleDecls = new WeakSet<object>();
defineInterface("CSSStyleDeclaration", (x) => !!x && typeof x === "object" && styleDecls.has(x), null);

// -- SVG and MathML elements. An element Vue renders inside `<svg>` (or
//    `createElementNS(svgNS, …)` makes) is `instanceof SVGElement` and of its
//    tag's interface, and not `instanceof HTMLElement`; `<math>` likewise for
//    MathMLElement. Vue's Teleport asks `instanceof SVGElement` /
//    `MathMLElement` of its target, and motion's scroll offsets ask
//    `instanceof SVGGraphicsElement`, a ReferenceError while it was missing.
//    Each interface answers by tag; one with no tag of its own answers for
//    the tags of those under it.
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
  // The engine's SVGElement (redjs has one), or ours in a realm without.
  const SVGElementIface = defineInterface("SVGElement", isSvg, ElementIface);
  wrapElementHasInstance(SVGElementIface, (el) => el._ns === "svg");
  defineInterface("MathMLElement", (x) => isMirrorEl(x) && x._ns === "mathml", ElementIface);
  // Each interface's tags and those of every interface under it. The table
  // lists a parent before its children, so walking it backwards completes a
  // child's set before it is added to its parent's.
  const tagsOf = new Map<string, Set<string>>(SVG_INTERFACES.map(([name, , tags]) => [name, new Set(tags)]));
  for (let i = SVG_INTERFACES.length - 1; i >= 0; i--) {
    const [name, parent] = SVG_INTERFACES[i]!;
    const into = tagsOf.get(parent);
    if (into) for (const t of tagsOf.get(name)!) into.add(t);
  }
  for (const [name, parent] of SVG_INTERFACES) {
    const tags = tagsOf.get(name)!;
    defineInterface(name, (x) => isSvg(x) && tags.has(x.tag), (theGlobal(parent) as Iface | undefined) ?? SVGElementIface ?? null);
  }
}

// -- Interfaces over objects that already exist here: a `classList`, a
//    `dataset`, a MutationRecord, a ResizeObserver entry and its sizes, a
//    TreeWalker (each branded where it is made, `BRANDS`), `matchMedia()`'s
//    answer, `performance` and `crypto`.
for (const [name, set] of Object.entries(BRANDS)) {
  defineInterface(name, (x) => !!x && typeof x === "object" && set.has(x), null);
}
// redjs's `matchMedia()` answers an EventTarget with `media` and `matches`.
defineInterface(
  "MediaQueryList",
  (x) => !!ET && x instanceof (ET as unknown as typeof EventTarget) && typeof (x as { media?: unknown }).media === "string" && "matches" in x,
  ET,
);
defineInterface("Performance", (x) => x != null && x === theGlobal("performance"), ET);
defineInterface("Crypto", (x) => x != null && x === theGlobal("crypto"), null);

// -- Event interfaces the engine lacks, each constructible with its init
//    dictionary as a browser's is, its members read-only prototype getters.
//    The runtime makes its `transitionend` / `animation*` events with the
//    first two when they exist (`makeEvent`); the rest exist so `instanceof`
//    and `new` work in code that names them.
const EVENT_INTERFACES: readonly (readonly [name: string, parent: "Event" | "UIEvent", members: Readonly<Record<string, unknown>>])[] = [
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
          // WebIDL's conversions for each member's type, from its default.
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

// -- InputEvent's two members the engine's lacks. Both are for a
//    contenteditable: in a text field's `beforeinput` and `input` Chromium
//    has no DataTransfer (the text is `data`) and no target ranges.
{
  const IE = theGlobal("InputEvent");
  const proto = typeof IE === "function" ? (IE as { prototype: object }).prototype : null;
  if (proto && vacant(proto, "dataTransfer"))
    Object.defineProperty(proto, "dataTransfer", { get: mark(function dataTransfer() { return null; }), enumerable: true, configurable: true });
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

// -- Touch: what desktop Chrome (Electron, where the client runs today) shows
//    on a Windows machine without a touch screen. The interfaces exist and
//    construct (`TouchEvent`, `Touch`; `TouchList` is not constructible),
//    `'ontouchstart' in window` is false (no `ontouch*` handler attribute is
//    defined anywhere), `navigator.maxTouchPoints` is 0, and no touch event
//    is ever delivered. A library that feature-tests any of these takes the
//    path it takes in Electron (naive-ui's `window.TouchEvent && e instanceof
//    TouchEvent`, a "touch device" test on `maxTouchPoints`).
const TouchListProto = collectionProto("TouchList");
adoptInterface("TouchList", TouchListProto, null);
{
  /** TouchInit's members and their defaults (Touch Events §4.1). */
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
          throw new TypeError(`Failed to construct 'Touch': Failed to read the '${req}' property from 'TouchInit': Required member is undefined.`);
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
    /** A `sequence<Touch>` init member as a TouchList: every member must be a Touch. */
    const touchList = (v: unknown, member: string): MirrorCollection<unknown> => {
      if (v == null) return staticList(TouchListProto, []);
      if (typeof (v as Iterable<unknown>)[Symbol.iterator] !== "function") {
        throw new TypeError(`Failed to construct 'TouchEvent': Failed to read the '${member}' property from 'TouchEventInit': The provided value cannot be converted to a sequence.`);
      }
      const items = [...(v as Iterable<unknown>)];
      for (const t of items) {
        if (!(TouchCtor && t instanceof (TouchCtor as unknown as new () => object))) {
          throw new TypeError(`Failed to construct 'TouchEvent': Failed to read the '${member}' property from 'TouchEventInit': Failed to convert value to 'Touch'.`);
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
// `navigator.maxTouchPoints`: 0, a machine with no touch screen.
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

// -- DOMRectReadOnly / DOMRect: constructible, with OWN data members (so a
//    `{ ...rect }` spread keeps them, as the rects built here already do), and
//    `getBoundingClientRect()` answers are DOMRects.
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
    const [nx, ny, nw, nh] = [x, y, width, height].map(Number);
    Object.assign(this, {
      x: nx, y: ny, width: nw, height: nh,
      top: Math.min(ny, ny + nh), left: Math.min(nx, nx + nw),
      right: Math.max(nx, nx + nw), bottom: Math.max(ny, ny + nh),
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
/** The rect a host answer makes, as a DOMRect when the realm's is ours. */
function asDomRect<T extends object>(r: T): T {
  const C = theGlobal("DOMRect");
  if (C === DOMRectImpl) Object.setPrototypeOf(r, DOMRectImpl.prototype);
  return r;
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
  const id = typeof (node as MirrorNode)?.__vnId === "number" ? (node as MirrorNode).__vnId : appRoot?.__vnId;
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
    // No answer at all: the node is not in the tree (removed, or a stale
    // reference after an unmount) or there is no root yet. CSSOM gives an
    // element that is not connected an EMPTY declaration list, so every
    // property reads "" in a browser too, and that is not a gap to advise on.
    // A late floating-ui or <Presence> read on an unmounted node is what made
    // width, position, animationName and the rest look missing (m26 client
    // sweep #30).
    if (!raw) return "";
    // A custom property nothing declared is "" in a browser as well.
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
    styleDecls.add(cached as object);
  }
  return cached;
});

// -- the document surface. redjs ships a `document` object with no tree behind
//    it; these are the entry points a reused component reaches through.
const doc = globalThis.document;
if (doc) {
  // Filled in where the engine has none; a previous runtime's counts as none.
  const defDoc = (name: string, value: unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
  };
  const defDocGet = (name: string, get: () => unknown) => {
    if (vacant(doc, name)) Object.defineProperty(doc, name, { get: mark(get), configurable: true });
  };
  // These three are FORCE-installed, not filled in: redjs ships its own
  // `querySelector`/`getElementById` backed by a registry of the elements IT
  // created, which is empty here — the retained tree is ours. Leaving the
  // engine's in place is the difference between "finds the node" and "silently
  // finds nothing", which is what made every `DialogContent` log a false
  // "requires a DialogTitle" accessibility warning.
  const forceDoc = (name: string, value: unknown) =>
    Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  const rootId = () => (appRoot ? appRoot.__vnId : 0);
  forceDoc("querySelector", (sel: string) => queryOne(sel, rootId()));
  forceDoc("querySelectorAll", (sel: string) => staticList(NodeListProto, hostQuery(sel, rootId(), 0)));
  // Live, over the whole tree (`getElementsByTagName` is with `<head>`, below).
  defDoc("getElementsByClassName", (classNames: unknown) => {
    const test = classTest(classNames);
    return liveCollection(HTMLCollectionProto, true, () => elementsUnder(appRoot, true, test));
  });
  // A NodeList, live: the HTML elements whose `name` attribute is `name`.
  defDoc("getElementsByName", (name: unknown) => {
    const want = String(name);
    return liveCollection(NodeListProto, false, () =>
      elementsUnder(appRoot, true, (el) => !el._ns && el.attrs.name === want),
    );
  });
  forceDoc("getElementById", (id: unknown) =>
    queryOne(`[id=${JSON.stringify(String(id))}]`, rootId()),
  );
  // `document.createElement(tag)` is a mirror element: a host node made by
  // the renderer's own `nodeOps.createElement`, loose until it is inserted, so
  // a script can build a subtree and append it anywhere (m26 client sweep
  // #5a). The `ENGINE_TAGS` stay the engine's detached elements: `<a>` and
  // `<input>` carry the download and pick idioms (FileSaver.js, VueUse
  // `useFileDialog`; see `wrapDetached`), and the head-only ones are not boxes
  // (a `<style>` joins the sheet wherever it is inserted).
  // The engine's, not a previous runtime's wrapper (`original`): a reload would
  // otherwise wrap the old wrapper, and every reload would add a layer.
  const engineCreate = original(doc, "createElement", (doc as { createElement?: unknown }).createElement);
  const nativeCreate =
    typeof engineCreate === "function"
      ? (engineCreate as (tag: unknown, opts?: unknown) => unknown).bind(doc)
      : null;
  forceDoc("createElement", (tag: unknown, opts?: unknown) => {
    const name = String(tag);
    if (!VALID_ELEMENT_NAME.test(name)) {
      throw domError("createElement", "Document", "InvalidCharacterError", `The tag name provided ('${name}') is not a valid name.`);
    }
    const t = name.toLowerCase();
    if (nativeCreate && ENGINE_TAGS.has(t)) {
      const el = nativeCreate(tag, opts);
      if ((t === "a" || t === "input") && el && typeof el === "object") wrapDetached(el as Detached, t);
      return el;
    }
    return looseCreated(nodeOps.createElement(t));
  });
  // The SVG namespace keeps the name's case (`linearGradient`), as the
  // renderer's SVG elements do; the HTML one is `createElement`.
  defDoc("createElementNS", (ns: unknown, qualifiedName: unknown) => {
    const name = String(qualifiedName);
    const local = name.slice(name.indexOf(":") + 1);
    if (!VALID_ELEMENT_NAME.test(local)) {
      throw domError("createElementNS", "Document", "InvalidCharacterError", `The qualified name provided ('${name}') contains the invalid name-start character '${local[0] ?? ""}'.`);
    }
    if (ns === HTML_NS) return (doc.createElement as (t: string) => unknown)(local);
    const space = ns === SVG_NS ? "svg" : ns === MATHML_NS ? "mathml" : undefined;
    return looseCreated(nodeOps.createElement(local, space));
  });
  // A browser's Text node, here a mirror text node: Vue-created and
  // library-created text are the same kind of object, so either can be
  // inserted anywhere in the tree. Replaced after a reload as well: the old
  // one made nodes in the OLD mirror.
  defDoc("createTextNode", (data: unknown) => looseCreated(nodeOps.createText(String(data))));
  defDoc("createComment", (data: unknown) => {
    const c = looseCreated(nodeOps.createComment(String(data)));
    c.text = String(data);
    return c;
  });
  // A detached Attr, for `setAttributeNode` (DOM §4.5).
  defDoc("createAttribute", (name: unknown) => {
    const n = String(name);
    if (!VALID_ELEMENT_NAME.test(n)) {
      throw domError("createAttribute", "Document", "InvalidCharacterError", `The localName provided ('${n}') contains an invalid character.`);
    }
    return new AttrImpl(n, "");
  });
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
    return branded("TreeWalker", {
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
    });
  });
  defDocGet("defaultView", () => globalThis);
  // Forced, not filled in: redjs answers null whatever has focus.
  try {
    Object.defineProperty(doc, "activeElement", { get: activeElement, configurable: true });
  } catch (e) {
    warnCompat("activeElement", `document.activeElement could not be installed on this engine and reads null (${e})`);
  }
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
      // Keyed by the CSS spelling, as the real block is, so the replay below
      // is `setProperty` with names it accepts (`documentElement.style.fontSize`
      // is `font-size`), and `getPropertyValue("font-size")` reads it back.
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

// -- web fonts: `FontFace`, `FontFaceSet` and `document.fonts` (src/fonts.ts).
//    Inert without the host's loader. A reload installs a fresh set over the
//    old one and takes over the host's listener.
installFonts(globalThis as unknown as Record<string, unknown>, doc);

// -- Web Animations: `el.animate()` (src/waapi.ts). On HTMLElement.prototype
//    and SVGElement.prototype only — not Element.prototype, so motion /
//    framer-motion keep their JS animations (docs/RUNTIME.md §5.23). Inert
//    without the host's `__vn_anim_*`.
if (typeof __vn_anim_set === "function") {
  const g = globalThis as unknown as Record<string, unknown>;
  const svg = (g.SVGElement as { prototype?: object } | undefined)?.prototype;
  installWaapi({
    host: { set: __vn_anim_set, clock: __vn_anim_clock, drop: __vn_anim_drop, values: __vn_anim_values },
    protos: svg && svg !== ElementProto ? [ElementProto, svg] : [ElementProto],
    nodeOf: (t) => (isMirrorEl(t) ? t.__vnId : 0),
    setStyle: (t, name, value) => (t as MirrorElement).style.setProperty(name, value),
    contains: (root, node) => isMirrorNode(root) && isMirrorNode(node) && (root as MirrorElement).contains(node as MirrorNode),
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

// -- the text selection: `getSelection()`, `Selection`, `Range`,
//    `document.createRange()` (src/selection.ts) over the host's selection
//    (crates/vn-tree/src/selection.rs). Inert without the host's half.
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
    if (doc && vacant(doc, name)) Object.defineProperty(doc, name, { value: mark(value), configurable: true, writable: true });
  },
});
// `document.onselectionchange` / `onselectstart`: the IDL handler over the
// document's listener registry (an element's are `HANDLER_ATTRS`).
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

// ---- document.elementFromPoint / elementsFromPoint
// The host's own hit test (crates/vn-tree/src/pointer.rs), the one a press
// resolves its target with: the topmost element under the point, skipping
// `pointer-events: none` and `visibility: hidden`, in the coordinates
// `clientX` / `getBoundingClientRect()` use. `elementsFromPoint` is every
// element whose box is under the point, topmost first. A point outside the
// viewport is null / []. Coordinates are a WebIDL `double`, so a non-finite one
// throws.
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

// ---- <style> injected into document.head, createTextNode, getElementsByTagName
// The runtime-CSS idiom of half the npm ecosystem (style-inject, which
// vue3-spinners runs at MODULE SCOPE; css-in-js; toast libraries):
//
//     const s = document.createElement("style");
//     document.getElementsByTagName("head")[0].appendChild(s);   // or insertBefore
//     s.appendChild(document.createTextNode(css));               // AFTER insertion
//
// `document.head` is not a box (see above), so a `<style>` there is not a
// node of the tree. It IS a stylesheet: its text joins the app's sheet, after
// the bundle's CSS, in insertion order, and follows later edits to the
// element (appendChild of text, `textContent =`, removal). Injection before
// mount, the usual case, costs nothing extra because it rides the first
// `__vn_set_stylesheet`. After mount, each change re-sets the whole sheet once
// per microtask. A `<link rel=stylesheet>` appended to head is not fetched;
// that is an advisory.
/** What `mount()` installs the stylesheet through: the host's, or the head-style-aware one below. */
let installStylesheet: (css: string) => void = (css) => __vn_set_stylesheet(css);
{
  const d = globalThis.document as unknown as Record<string, unknown> | undefined;
  const head = d?.head as Record<string, unknown> | undefined;
  // The HOST's function, even after a reload: the global of that name is the
  // previous runtime's `install` by then (see `original`).
  const hostSetSheet =
    typeof __vn_set_stylesheet === "function" ? original(globalThis, "__vn_set_stylesheet", __vn_set_stylesheet) : null;
  if (d && head && typeof head === "object" && hostSetSheet) {
    /** `<style>` elements in head, insertion order, with the text each one holds. */
    const injected: { el: Record<string, unknown>; text: string }[] = [];
    /** The last sheet anyone else installed (the bundle's, or the dev loader's). */
    let base: string | null = null;
    let queued = false;
    // Each injected sheet goes to the host as its OWN text, not concatenated
    // onto the bundle's: the host parses (and keeps) one parse per distinct
    // text, so a concatenation would re-parse the whole bundle sheet — and
    // keep another copy of it — every time an injected one changed
    // (research/reports/m26/perf-mem-1.md §3).
    const install = (css: string) => {
      base = String(css);
      hostSetSheet(base, injected.map((s) => s.text).filter(Boolean));
    };
    // Every installer goes through this — the mount below by name, loader.ts's
    // flushCss through the global — so the injected sheets survive a hot
    // reload's re-aggregation.
    installStylesheet = install;
    try {
      Object.defineProperty(globalThis, "__vn_set_stylesheet", { value: install, configurable: true, writable: true });
    } catch {
      warnCompat("head-style:install", "the dev loader's stylesheet updates will drop <style> elements added to document.head");
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
    const textOf = (n: unknown): string => {
      if (n == null) return "";
      if (typeof n === "string") return n;
      const o = n as { text?: unknown; data?: unknown; textContent?: unknown; nodeValue?: unknown };
      const v = o.text ?? o.data ?? o.textContent ?? o.nodeValue;
      return v == null ? "" : String(v);
    };
    const isStyle = (n: unknown) =>
      !!n && typeof n === "object" && String((n as { tagName?: unknown }).tagName ?? "").toUpperCase() === "STYLE";
    const isLink = (n: unknown) =>
      !!n && typeof n === "object" && String((n as { tagName?: unknown }).tagName ?? "").toUpperCase() === "LINK";
    /** Makes a head `<style>` follow its own text from now on. */
    const track = (el: Record<string, unknown>) => {
      const entry = { el, text: textOf(el) };
      injected.push(entry);
      const set = (v: unknown) => {
        entry.text = v == null ? "" : String(v);
        refresh();
      };
      try {
        for (const prop of ["textContent", "innerHTML", "innerText"]) {
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
        // `style.remove()` and `style.parentNode.removeChild(style)` both take it out.
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
        const rel = String((n as { rel?: unknown }).rel ?? (n as { getAttribute?(k: string): unknown }).getAttribute?.("rel") ?? "");
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
    // style-inject's own spelling ("afterbegin" for `prepend: true`). The
    // position is not honoured: an injected sheet always lands after the
    // bundle's CSS, where an append would put it.
    def("insertAdjacentElement", (_where: unknown, n: unknown) => adopt(n));
    def("removeChild", (n: unknown) => {
      untrack(n);
      return n;
    });
    if (!("tagName" in head)) def("tagName", "HEAD");
    // A previous runtime's getter (a same-realm reload, see `REALM`) read ITS
    // injected list, so it is replaced too.
    if (vacant(head, "childNodes")) {
      Object.defineProperty(head, "childNodes", {
        get: mark(() => staticList(NodeListProto, injected.map((s) => s.el))),
        configurable: true,
      });
    }

    // A `<style>` / `<link>` from the engine inserted anywhere else (a
    // browser applies a `<style>` in `<body>` too), and taken out again.
    foreignStyle = (n, inserted) => {
      if (!isStyle(n) && !isLink(n)) return false;
      if (inserted) adopt(n);
      else untrack(n);
      return true;
    };
    // A live HTMLCollection. `head`, `html` and `body` are the document's own
    // (`html` and `body` are both the root here), `style` includes the sheets
    // injected into `<head>`, and anything else is the tree's elements.
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
  // Compared with what the field holds now — typing keeps `_value` current —
  // so a model reset to the string the user has since typed over still lands.
  if (el._value === v) return;
  writeControlValue(el, v);
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
      // Runtime-dom's rule: no model update in the middle of an IME
      // composition. The composition's end assigns the committed text.
      if (el.composing) return;
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
    // …and the composition's two ends, as runtime-dom binds them: the end
    // re-dispatches `input`, which assigns what the IME committed. (Runtime-dom
    // binds them only without `.lazy`, which listens for `change`; here
    // `.lazy` listens for `input`, so it needs them too.)
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
    // Runtime-dom leaves a composing field alone: assigning it would end the
    // IME's composition (vuejs/core#2302).
    if (el.composing) return;
    setElValue(el, value);
  },
};

// v1: every input renders as a text field, so the dynamic/checkbox/radio
// variants fall back to the text behavior with an advisory. (`<select>` has
// its own, `vModelSelect`, below.)
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

// @vue/shared's `looseEqual`, `looseIndexOf` and `looseToNumber`, which
// `vModelSelect` matches option values with.
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

/** runtime-dom's `getModelAssigner`. */
function modelAssigner(vnode: VNodeLike): ((value: unknown) => void) | null {
  const fn = vnode.props?.["onUpdate:modelValue"];
  if (Array.isArray(fn)) return (value) => fn.forEach((f) => (f as (v: unknown) => void)(value));
  return typeof fn === "function" ? (fn as (value: unknown) => void) : null;
}

/** runtime-dom's `setSelected`: the model value onto the options. */
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

/**
 * runtime-dom's `vModelSelect`, over the select model above: `change` assigns
 * the selected option's value as Vue bound it (`.number` through
 * `looseToNumber`), or an array / Set of them for `multiple`, and the model
 * selects the matching option once the options are mounted and after every
 * update but the one its own assignment caused.
 */
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
  // Set in `mounted` and `updated`: a select's value depends on its options,
  // which are mounted (and updated) before it.
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

// runtime-dom's `vShow`: the element's own inline `display` is remembered
// before the first hide and restored on show, so `<div v-show="x"
// style="display: flex">` shows as flex again. `patchStyle` keeps the two in
// step when `:style` changes `display` while it is hidden. (`<Transition>`
// around `v-show` is not wired: the display flips without the phases.)
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

/**
 * The cookie jar's half of `appStorage.info()` (crates/vn-storage/src/cookies.rs):
 * `<folder at startup>\cookies.sealed`, always sealed whatever `encrypted` says
 * (cookies are credentials). `persisted` counts the persistent cookies last
 * loaded or staged; session cookies are never written. `file` is null when the
 * host has no jar backend.
 */
interface CookieJarInfo {
  file: string | null;
  sealed: true;
  bytes: number;
  persisted: number;
  pending: boolean;
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
  /** Absent from an inert `appStorage` (no host). */
  cookies?: CookieJarInfo;
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
     * than quietly writing plaintext. Leaving `encrypted` out keeps the current
     * format — a folder that is encrypted stays encrypted; only an explicit
     * `{ encrypted: false }` decrypts.
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
     *
     * The HTTP cache (docs/STORAGE.md §12) follows `encrypted` at once: its
     * folder is purged and refills in the new format. It does not follow
     * `path`; it lives beside the default storage folder.
     */
    setStorageFolder(path: string, options: { encrypted?: boolean } = {}) {
      // No `encrypted` is "keep the format", not "plaintext": a folder change
      // must never silently decrypt (the sticky rule, docs/STORAGE.md §5).
      const said = options ? options.encrypted : undefined;
      const encrypted = said === undefined ? "keep" : said ? "true" : "false";
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
// instance's `launch`) and is handed to `desktopEvents`. A settle may carry a
// VALUE the host made on delivery (a picked file's `File`, an input's
// `FileList`) as the dispatcher's fourth argument.

/** The host fns, gathered so the inert branch is a single `null` check. */
interface DesktopHost {
  setDispatcher: typeof __vn_desktop_set_dispatcher;
  openExternal: typeof __vn_shell_open_external;
  openPath: typeof __vn_shell_open_path;
  showItem: typeof __vn_shell_show_item;
  trash: typeof __vn_shell_trash;
  /** The File System Access and dialog half, or null on a host from before it. */
  fs: DesktopFsHost | null;
}

const desktopHost: DesktopHost | null =
  typeof __vn_desktop_set_dispatcher === "function"
    ? {
        setDispatcher: __vn_desktop_set_dispatcher,
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

/**
 * A pending call. `raw` ones (desktop.ts's) resolve `{payload, value}` and
 * reject with the host's failure object, which desktop.ts turns into the
 * DOMException its spec names; the rest resolve the payload and reject with
 * `desktopError`.
 */
const desktopPending = new Map<
  number,
  { resolve: (v: unknown) => void; reject: (e: unknown) => void; raw: boolean }
>();
/** Consumers of token-0 events, by `payload.kind`. */
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

/** One host answer → a promise: `{token}` waits for the pump, a refusal rejects now. */
function desktopCall(json: VnJson): Promise<Record<string, unknown>> {
  return desktopAwait(json, false) as Promise<Record<string, unknown>>;
}

/** {@link desktopCall} for desktop.ts: `{payload, value}`, and raw failures. */
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

// ---- transient activation (HTML "user activation") ---------------------------
//
// What a picker needs and a `<input type=file>` click checks: a keydown or a
// mousedown the HOST delivered in the last five seconds (Chromium's lifespan).
// `el.click()` and `dispatchEvent` never activate — script cannot vouch for a
// user. Escape and a lone modifier do not count (HTML: "the Esc key" is
// excluded; Chromium skips modifier-only presses). The host dispatcher below
// calls `noteActivation`, beside the sticky-activation hook.

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

// `navigator.userActivation` (HTML), where the engine has none: the read side
// of the same state, for an app that wants to know before it asks.
// A previous runtime's (a same-realm reload) read ITS activation, so it goes too.
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

// ---- File System Access, FileReader, the pickers (desktop.ts) ---------------

const desktopApi: DesktopApi = installDesktop(globalThis as unknown as Record<string, unknown>, {
  host: desktopHost?.fs ?? null,
  engine: globalThis as unknown as DesktopEngine,
  call: desktopCallRaw,
  warnCompat,
  activation,
  realm: { vacant, mark },
});

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
  /** One granted handle per existing path, for `LaunchParams.files`. */
  handles?: HandleWire[];
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
 * `LaunchParams` (WICG Launch Handler + File Handling). `targetURL` is the RAW
 * deep link — there is no app URL scope to template it into — and `files`
 * holds a `FileSystemFileHandle` / `FileSystemDirectoryHandle` for EVERY
 * existing path argument, where a PWA gets only those its manifest's
 * `file_handlers` accept: there is no such list. Both are D-F8.
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

function launchParamsFor(url: string | null, handles: unknown): LaunchParams {
  const files = Array.isArray(handles)
    ? (handles as HandleWire[]).filter((w) => w && typeof w.h === "number").map((w) => desktopApi.handleFrom(w))
    : [];
  return new LaunchParams(url, files);
}

const launchQueue = new LaunchQueue();
{
  const g = globalThis as unknown as { launchQueue?: unknown; LaunchParams?: unknown; LaunchQueue?: unknown };
  // Installed only where the engine has none (redjs does not), never over one.
  // A previous runtime's counts as none (`REALM`).
  if (vacant(g, "launchQueue")) {
    g.launchQueue = mark(launchQueue);
    g.LaunchParams = mark(LaunchParams);
    g.LaunchQueue = mark(LaunchQueue);
  }
}
// This process's own launch, once per process. A reload is not a launch, and a
// browser hands a reloaded page no LaunchParams either. A host without
// vn-desktop has no launch to report.
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
    // The tier-2 event first, then the tier-1 queue — the order docs/LAUNCH.md
    // §4 documents. Either may be the one an app listens to; neither waits on
    // the other.
    try {
      target.dispatchEvent(ev);
    } finally {
      launchQueue._enqueue(launchParamsFor(url, p.handles));
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
  // runtime-dom's `classList.remove(cls)` takes it out of the whole attribute,
  // so a copy a script wrote back into the class goes too.
  const base = (el._class || "").split(/\s+/);
  if (base.includes(cls)) el._class = base.filter((c) => c && c !== cls).join(" ");
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
// above knows about it — a pop-out is addressed by `mirrorNode.__vnId`, which every
// template ref already hands you, so the component is a leaf of this module
// rather than a change to it.
export { VnWindow, VN_WINDOW_CREATION_TIME_PROPS } from "./vnwindow";
export type { VnWindowBounds, VnWindowMetrics, VnWindowTarget } from "./vnwindow";

// `appHost` — the functions and events of a program that embeds vue-native as
// a library (docs/EMBEDDING.md). Inert, and saying so, in the exe.
export { appHost } from "./apphost";
export type { AppHost, AppHostError } from "./apphost";

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "vue-native" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
