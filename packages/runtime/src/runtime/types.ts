// Members not listed here do not exist on a mirror node: reused browser code gets the TypeError an engine without them would give.

/** 1 element, 2 text, 3 comment. The mirror's own tag — not `nodeType`. */
export type MirrorKind = 1 | 2 | 3;

export type EventHandler = (ev: Event) => void;
/** …and what Vue passes for `@click="[a, b]"` — several, in order. */
export type EventHandlerOrList = EventHandler | EventHandler[];
export type ListenerSignal = {
  readonly aborted: boolean;
  addEventListener(type: string, listener: () => void, options?: { once?: boolean }): void;
  removeEventListener(type: string, listener: () => void): void;
};
export type ListenerOptions =
  boolean | { capture?: boolean; once?: boolean; passive?: boolean; signal?: ListenerSignal } | undefined;

export interface ExtraListener {
  fn: EventHandler;
  once: boolean;
  passive?: boolean;
  signalCleanup?: (() => void) | undefined;
  removed?: boolean;
  /** Runs before the renderer's own handler for the event. */
  first?: boolean;
}

/**
 * Listeners for one event name on one target. `bubble`/`capture` are the RENDERER's single slots (Vue
 * re-patches a prop); `extraB`/`extraC` are `addEventListener`'s and accumulate.
 */
export interface Buckets {
  bubble: EventHandlerOrList | null;
  capture: EventHandlerOrList | null;
  onceB: boolean;
  onceC: boolean;
  passiveB?: boolean;
  passiveC?: boolean;
  extraB: ExtraListener[];
  extraC: ExtraListener[];
}

export type HandlerMap = Record<string, Buckets | undefined>;

export interface MirrorRect {
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

export interface MirrorCollection<T> extends ArrayLike<T>, Iterable<T> {
  item(index: number): T | null;
}

export interface MirrorClassList {
  add(...names: string[]): void;
  remove(...names: string[]): void;
  toggle(name: string, force?: boolean): boolean;
  contains(name: string): boolean;
  readonly length: number;
  item(i: number): string | null;
  toString(): string;
  [Symbol.iterator](): IterableIterator<string>;
}

/** `el.style`: a Proxy. Unnamed property writes go straight to `__sk_set_style`, custom properties included (reka-ui's `--reka-popper-*`). */
export interface MirrorStyle {
  setProperty(key: string, value: string | number | null | undefined, priority?: string): void;
  removeProperty(key: string): string;
  getPropertyValue(key: string): string;
  getPropertyPriority(key: string): string;
  item(i: number): string;
  readonly length: number;
  cssText: string;
  [property: string]: unknown;
}

interface MirrorNodeBase {
  /**
   * The host node id (`SkNodeId`); `el.id` is the `id` ATTRIBUTE. 0 once the node is freed (`forgetDeep`):
   * host ids are recycled, so a stale reference must not name the next node.
   */
  __skId: SkNodeId;
  kind: MirrorKind;
  tag: string;
  parentNode: MirrorElement | null;
  childNodes: MirrorNode[];
  text?: string;
  contains(other: MirrorNode | null | undefined): boolean;
  readonly ownerDocument: Document | undefined;
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
  cloneNode(deep?: boolean): MirrorNode;
  getRootNode(): unknown;
  dispatchEvent(ev: Event): boolean;
  /**
   * Event codes whose `__sk_set_listener` bit is ON in the host (see `syncListenerFlag`). Kept on the node,
   * not in a map by id: `__sk_remove` recycles ids, so a map would answer for a slot's previous occupant.
   */
  _listenerOn?: Set<SkEventCode>;
  /** Vue's symbol-keyed bookkeeping lands here; `__v_skip` on the prototype stops `reactive()` proxying the node. */
  [key: symbol]: unknown;
}

export interface MirrorText extends MirrorNodeBase {
  kind: 2 | 3;
  data: string;
  nodeValue: string;
  textContent: string;
}

export interface MirrorElement extends MirrorNodeBase {
  kind: 1;
  tagName: string;
  value: string;
  _value: string;
  /** Native accessors: the host holds a checkbox's and radio's state. */
  checked: boolean;
  indeterminate: boolean;
  /** Option: whether the host's `:checked` property is set. */
  _shownChecked?: boolean;
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
  _on?: Record<string, { fn: unknown; listener: (ev: Event) => void }>;
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
  setSelectionRange(start: number | null, end: number | null, direction?: "forward" | "backward" | "none"): void;
  setPointerCapture(pointerId: number): void;
  hasPointerCapture(pointerId: number): boolean;
  releasePointerCapture(pointerId: number): void;

  insertAdjacentElement(where: string, el: MirrorElement): MirrorElement;
  append(...nodes: unknown[]): void;
  remove(): void;

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

  /** runtime-dom's `CSS_VAR_TEXT`: what `useCssVars` set, kept across a string `:style`. */
  _cssVarText?: string;
  _vtc?: Set<string> | undefined;
  _isLeaving?: boolean;
  _endId?: number;
}

export type MirrorNode = MirrorElement | MirrorText;

export type MirrorNodeInit = Omit<MirrorElement, "kind"> & Omit<MirrorText, "kind"> & { kind: MirrorKind };

export interface MirrorTimeRanges {
  length: number;
  start(i: number): number;
  end(i: number): number;
  ranges: { start: number; end: number }[];
}

export type PropagationTarget = MirrorNode | object;
