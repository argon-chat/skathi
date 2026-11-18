// Runs src/runtime.ts in an isolated fake host, without gpui, JavaScriptCore or a window. As in
// loader-harness.ts the source is transpiled (./ts-source.ts), import/export lines are stripped and every
// ambient name (`__sk_*`, `console`) is a parameter, so each createRuntime() gets its own world.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BaseTransition,
  BaseTransitionPropsValidators,
  callWithAsyncErrorHandling,
  createRenderer,
  createVNode,
  defineComponent,
  Fragment,
  getCurrentInstance,
  getTransitionRawChildren,
  h,
  nextTick,
  onBeforeUpdate,
  onUpdated,
  onMounted,
  onUnmounted,
  queuePostFlushCb,
  resolveTransitionHooks,
  setTransitionHooks,
  shallowRef,
  Static,
  Teleport,
  isRef,
  setDevtoolsHook,
  toRaw,
  useTransitionState,
  version,
  watch,
} from "@vue/runtime-core";
import type { WaapiHost } from "../src/waapi";
import { bundleRuntime, readTsSource } from "./ts-source";
import { createReferenceWaapiCore, referenceEffect, referenceValues } from "./fixtures/waapi-core-reference";
import { ENGINE_CLASSES } from "./fixtures/engine-classes";
import { styleHost } from "./fixtures/engine-style";

const HERE = dirname(fileURLToPath(import.meta.url));
export const RUNTIME_PATH = join(HERE, "../src/runtime.ts");
const RUNTIME_JS = await bundleRuntime(RUNTIME_PATH);

export const CANVAS_PATH = join(HERE, "../src/canvas.ts");
let adoptCanvasFn: unknown = null;
function loadAdoptCanvas(): unknown {
  if (adoptCanvasFn) return adoptCanvasFn;
  const body = readTsSource(CANVAS_PATH)
    .replace(/^\s*import\s[^;]*;[^\S\n]*$/gm, "")
    .replace(/^export\s+(function|const|let|class)\s/gm, "$1 ");
  adoptCanvasFn = new Function(`${body}\nreturn adoptCanvas;`)();
  return adoptCanvasFn;
}

export const WAAPI_PATH = join(HERE, "../src/waapi.ts");
let installWaapiFn: unknown = null;
function loadInstallWaapi(): unknown {
  if (installWaapiFn) return installWaapiFn;
  const body = readTsSource(WAAPI_PATH)
    .replace(/^\s*import\s[^;]*;[^\S\n]*$/gm, "")
    .replace(/^export\s+(function|const|let|class|interface)\s/gm, "$1 ");
  installWaapiFn = new Function(`${body}\nreturn installWaapi;`)();
  return installWaapiFn;
}

export const SELECTION_PATH = join(HERE, "../src/selection.ts");
let installSelectionFn: unknown = null;
function loadInstallSelection(): unknown {
  if (installSelectionFn) return installSelectionFn;
  const body = readTsSource(SELECTION_PATH)
    .replace(/^\s*import\s[^;]*;[^\S\n]*$/gm, "")
    .replace(/^export\s+(function|const|let|class|interface)\s/gm, "$1 ");
  installSelectionFn = new Function(`${body}\nreturn installSelection;`)();
  return installSelectionFn;
}

const IMPORT_LINE = /^\s*import\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
const STAR_REEXPORT = /^\s*export\s+\*\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
/** `export { A, B } from "./mod";`: dropped, since every name handed back is a parameter or in the harness's own `return`. */
const NAMED_REEXPORT = /^\s*export\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
const LOCAL_EXPORT = /^\s*export\s+\{[^}]*\};?[^\S\n]*$/gm;

const SKWINDOW_STANDINS = (): unknown[] => [Symbol("sk-window-root"), new Set<object>()];

export class FakeFileList {
  [i: number]: unknown;
  length: number;
  constructor(files: unknown[] = []) {
    this.length = files.length;
    files.forEach((f, i) => (this[i] = f));
  }
  item(i: number): unknown {
    return this[i] ?? null;
  }
  *[Symbol.iterator](): Iterator<unknown> {
    for (let i = 0; i < this.length; i++) yield this[i];
  }
}

class FakeDataTransfer {
  files = new FakeFileList();
}

type RuntimeModule = typeof import("../src/runtime");
export type MirrorElement = ReturnType<RuntimeModule["nodeOps"]["createElement"]>;
export type MirrorNode = Parameters<RuntimeModule["nodeOps"]["remove"]>[0];
type EventName =
  | "click"
  | "mousedown"
  | "mouseup"
  | "wheel"
  | "keydown"
  | "keyup"
  | "focus"
  | "blur"
  | "input"
  | "scroll"
  | "complete"
  | "loopComplete"
  | "enterFrame"
  | "mousemove"
  | "mouseenter"
  | "mouseleave"
  | "mouseover"
  | "mouseout"
  | "loadstart"
  | "loadedmetadata"
  | "loadeddata"
  | "canplay"
  | "canplaythrough"
  | "play"
  | "playing"
  | "pause"
  | "seeking"
  | "seeked"
  | "timeupdate"
  | "durationchange"
  | "ratechange"
  | "volumechange"
  | "waiting"
  | "ended"
  | "error"
  | "emptied"
  | "transitionend"
  | "animationstart"
  | "animationend"
  | "animationcancel"
  | "paste"
  | "copy"
  | "cut"
  | "contextmenu"
  | "load"
  | "pointercancel"
  | "selectstart"
  | "selectionchange"
  | "auxclick"
  | "beforeinput"
  | "compositionstart"
  | "compositionupdate"
  | "compositionend"
  | "message"
  | "pointerlockchange"
  | "pointerlockerror";

export interface RealmEvent {
  type: string;
  target: MirrorNode | null;
  defaultPrevented: boolean;
  preventDefault(): void;
  stopPropagation(): void;
  [key: string]: unknown;
}
export interface RealmObserverEntry {
  target: MirrorElement;
  [key: string]: unknown;
}
export interface RealmObserver {
  observe(target: MirrorNode, options?: unknown): void;
  unobserve(target: MirrorNode): void;
  disconnect(): void;
  takeRecords(): RealmObserverEntry[];
}
export type RealmObserverCtor = new (
  callback: (entries: RealmObserverEntry[], observer: RealmObserver) => unknown,
  options?: unknown,
) => RealmObserver;
export interface RealmDocument {
  body: MirrorElement;
  documentElement: MirrorElement;
  head: MirrorElement;
  activeElement: MirrorElement | null;
  createElement(tag: string): MirrorElement;
  createTextNode(data: string): MirrorNode;
  elementFromPoint(x: number, y: number): MirrorElement | null;
  elementsFromPoint(x: number, y: number): MirrorElement[];
  querySelector(selector: string): MirrorElement | null;
  querySelectorAll(selector: string): ArrayLike<MirrorElement> & Iterable<MirrorElement>;
  addEventListener(type: string, listener: (event: RealmEvent) => unknown, options?: unknown): void;
  removeEventListener(type: string, listener: (event: RealmEvent) => unknown, options?: unknown): void;
  [key: string]: unknown;
}
export interface RealmGlobal {
  document: RealmDocument;
  ResizeObserver: RealmObserverCtor;
  IntersectionObserver: RealmObserverCtor;
  MutationObserver: RealmObserverCtor;
  [name: string]: unknown;
}

interface RuntimeExports extends Pick<
  RuntimeModule,
  | "patchProp"
  | "nodeOps"
  | "createApp"
  | "Transition"
  | "TransitionGroup"
  | "resolveTransitionProps"
  | "appWindow"
  | "defineWindow"
  | "getCurrentWindow"
  | "vModelText"
  | "vModelSelect"
  | "vShow"
  | "render"
  | "useCssVars"
  | "useCssModule"
  | "VueElement"
  | "defineCustomElement"
  | "createSSRApp"
  | "hydrate"
  | "useHost"
  | "useShadowRoot"
  | "initDirectivesForSSR"
> {
  EV_CODES: Record<EventName, number>;
  DERIVED: Record<string, { from: string; pre?: boolean }>;
  canonicalEvent(name: string): string;
  hyphenate(name: string): string;
  handlers: WeakMap<MirrorNode, Record<string, unknown>>;
  nodesById: Map<number, MirrorNode>;
  nodeById(id: number): MirrorNode | undefined;
  looseById: Map<number, WeakRef<MirrorNode>>;
  looseRoots: Map<number, { ref: WeakRef<MirrorNode>; box: number }>;
  sweepLoose(): void;
  writeClass(el: MirrorElement, remove?: boolean): void;
  activation: { isActive(): boolean; consume(): void };
  noteActivation(name: string, key: string | undefined): void;
}

type Dispatcher = (
  id: number,
  code: number,
  a: number,
  b: number,
  c: number,
  d: number,
  key?: string | null,
  data?: unknown,
  x?: number,
  y?: number,
  pen?: number[],
) => unknown;
type Json = Record<string, unknown>;

interface FsCallRecord {
  op: string;
  args: unknown[];
  token: number | null;
}
interface TreeModel {
  parent: Map<number, number>;
  children: Map<number, number[]>;
  tag: Map<number, string>;
  text: Map<number, string>;
  live: Set<number>;
  freed: number[];
  errors: string[];
  create(tag: string): number;
  check(op: string, ...ids: number[]): boolean;
  setText(id: number, text: string): void;
  detach(id: number): void;
  free(id: number): void;
  setElementText(id: number, text: string): void;
  insert(child: number, parent: number, anchor: number): void;
  remove(id: number): void;
  shape(id: number): string[];
}

interface RuntimeHost {
  names: string[];
  listeners: { id: number; code: unknown; on: unknown }[];
  props: { id: number; key: string; value: unknown }[];
  styles: { id: number; key: string; value: unknown }[];
  /** Calls into each `__sk_style_*` host function, by name. */
  styleCalls: Record<string, number>;
  classes: { id: number; value: unknown }[];
  sheets: string[];
  sheetParts: string[][];
  logs: string[];
  dispatcher: Dispatcher;
  queryAnswers: Map<string, string>;
  queryCalls: { selector: unknown; root: unknown; mode: unknown }[];
  boundsById: Map<number, string>;
  layout: {
    hook: ((op: number) => number) | null;
    observing: boolean;
    pending: boolean;
    watches: number;
    steps: number;
    loopErrors: number;
    step(relayout?: () => void): number;
  };
  computedById: Map<number, string>;
  selection: { sel: number[] | null; sets: number[][]; rebases: number[][]; text: string };
  pointerCaptureCalls: { id: number; pointerId: number; capture: boolean }[];
  hit: { answer: (x: number, y: number, all: number) => string; calls: number[][] };
  anim: { calls: unknown[][]; notes: string; values: string | null };
  styleRejects: Set<string>;
  acceptCalls: string[];
  inputSelectCalls: number[];
  inputSelectAnswer: boolean;
  /** `__sk_color_picker`: the ids asked for, and whether the in-window picker took them (false: the OS dialog). */
  colorPickerCalls: number[];
  colorPickerAnswer: boolean;
  focus: { calls: { id: number; op: number }[]; answer: (id: number, op: number) => string };
  reloads: number;
  mediaById: Map<number, string>;
  mediaStateReads: number;
  mediaPropReads: number[][];
  mediaCalls: { id: number; verb: unknown; arg: unknown }[];
  canPlayByType: Map<string, string>;
  desktop: {
    calls: { op: string; args: unknown[] }[];
    nextToken: number;
    /** Answers a `__SK_FS__` picker: `{ cancelled: true }`, `{ color }`, or `{ files }` with the FileList as `value`. */
    settle(token: number, ok?: boolean, payload?: Json, value?: unknown): void;
    fs: {
      calls: FsCallRecord[];
      /** The next picker finds one already open: it resolves `undefined` without asking. */
      busy: boolean;
    };
  };
  tree: TreeModel;
  /** The dev-build performance advisories, when the world was made with `perf`. */
  perf: {
    advisories: { code: string; key: string; message: string }[];
    frame: number;
    visible: (id: number) => string;
    calls: number;
  };
}

/** `MouseEvent` hands the world an event class (bun has none). */
export function createRuntime(
  options: {
    MouseEvent?: unknown;
    globals?: Record<string, unknown>;
    beforeRuntime?: (global: Record<string, unknown>) => void;
    styleAccepts?: boolean;
    layoutWatch?: boolean;
    selection?: boolean;
    anim?: boolean;
    realm?: Record<string, unknown>;
    navigator?: Record<string, unknown>;
    /** The `__DEV__` define; true unless a test plays a production bundle. */
    dev?: boolean;
    /** Registers the `__sk_perf_*` host functions. */
    perf?: boolean;
  } = {},
) {
  const raw = RUNTIME_JS;
  if (!IMPORT_LINE.test(raw)) {
    throw new Error("runtime.ts no longer imports @vue/runtime-core — update this harness");
  }
  IMPORT_LINE.lastIndex = 0;
  const body = raw
    .replace(IMPORT_LINE, "")
    .replace(/^\s*import\s*["'][^"']+["'];?[^\S\n]*$/gm, "")
    .replace(STAR_REEXPORT, "")
    .replace(NAMED_REEXPORT, "")
    .replace(LOCAL_EXPORT, "")
    .replace(/^export\s+(function|const|let|class)\s/gm, "$1 ");

  let nextId = 1;
  const tree: TreeModel = {
    parent: new Map(),
    children: new Map(),
    tag: new Map(),
    text: new Map(),
    live: new Set(),
    freed: [],
    errors: [],
    create(tag: string): number {
      const id = nextId++;
      tree.live.add(id);
      tree.tag.set(id, tag);
      tree.children.set(id, []);
      tree.parent.set(id, 0);
      return id;
    },
    check(op: string, ...ids: number[]): boolean {
      for (const id of ids) {
        if (!tree.live.has(id)) {
          tree.errors.push(`${op}(${ids.join(",")}): ${id} is not a live node`);
          return false;
        }
      }
      return true;
    },
    setText(id: number, text: string) {
      if (tree.check("set_text", id)) tree.text.set(id, String(text));
    },
    detach(id: number) {
      const p = tree.parent.get(id) ?? 0;
      if (p)
        tree.children.set(
          p,
          tree.children.get(p)!.filter((c) => c !== id),
        );
      tree.parent.set(id, 0);
    },
    free(id: number) {
      for (const c of tree.children.get(id) ?? []) tree.free(c);
      tree.live.delete(id);
      tree.children.delete(id);
      tree.parent.delete(id);
      tree.freed.push(id);
    },
    setElementText(id: number, text: string) {
      if (!tree.check("set_element_text", id)) return;
      for (const c of tree.children.get(id)!) tree.free(c);
      tree.children.set(id, []);
      tree.text.set(id, String(text));
    },
    insert(child: number, parent: number, anchor: number) {
      if (!tree.check("insert", child, parent)) return;
      if (child === anchor) return;
      for (let a = parent; a; a = tree.parent.get(a) ?? 0) {
        if (a === child) return void tree.errors.push(`insert(${child},${parent}): a cycle`);
      }
      tree.detach(child);
      const kids = tree.children.get(parent)!;
      const i = anchor ? kids.indexOf(anchor) : -1;
      kids.splice(i < 0 ? kids.length : i, 0, child);
      tree.parent.set(child, parent);
      tree.text.delete(parent);
    },
    remove(id: number) {
      if (!tree.check("remove", id)) return;
      tree.detach(id);
      tree.free(id);
    },
    shape(id: number): string[] {
      return (tree.children.get(id) ?? []).map((c) =>
        tree.tag.get(c) === "#text" ? `#text:${tree.text.get(c) ?? ""}` : tree.tag.get(c)!,
      );
    },
  };
  const host: RuntimeHost = {
    tree,
    perf: { advisories: [], frame: 1, visible: () => "", calls: 0 },
    names: [],
    listeners: [],
    props: [],
    styles: [],
    styleCalls: {},
    classes: [],
    sheets: [],
    sheetParts: [],
    logs: [],
    dispatcher: () => {
      throw new Error("the runtime has not installed its dispatcher");
    },
    queryAnswers: new Map(),
    queryCalls: [],
    boundsById: new Map(),
    layout: {
      hook: null,
      observing: false,
      pending: false,
      watches: 0,
      steps: 0,
      loopErrors: 0,
      step(relayout?: () => void): number {
        if (!this.hook || !this.observing) return -1;
        this.pending = false;
        this.steps++;
        let rounds = 0;
        let due = this.hook(0);
        while (due > 0 && rounds < 1000) {
          rounds++;
          while (this.hook(1) > 0);
          relayout?.();
          due = this.hook(2);
        }
        const state = this.hook(3);
        this.observing = (state & 2) !== 0;
        if (state & 1) {
          this.loopErrors++;
          this.pending = true;
        }
        return rounds;
      },
    },
    computedById: new Map(),
    selection: {
      sel: null,
      sets: [],
      rebases: [],
      text: "",
    },
    pointerCaptureCalls: [],
    hit: {
      answer: (_x: number, _y: number, _all: number) => "",
      calls: [],
    },
    anim: {
      calls: [],
      notes: "",
      values: null,
    },
    styleRejects: new Set(),
    acceptCalls: [],
    inputSelectCalls: [],
    inputSelectAnswer: true,
    colorPickerCalls: [],
    colorPickerAnswer: true,
    focus: {
      calls: [],
      answer: (_id, op) => (op === 1 ? "" : "refused"),
    },
    reloads: 0,
    mediaById: new Map(),
    mediaStateReads: 0,
    mediaPropReads: [],
    mediaCalls: [],
    canPlayByType: new Map(),
    desktop: {
      calls: [],
      nextToken: 1,
      settle: (token, ok = true, payload = {}, value = undefined) => {
        const resolve = pickers.get(token);
        pickers.delete(token);
        if (!resolve) return;
        if (!ok) resolve(undefined);
        else if (payload.cancelled) resolve(null);
        else resolve(typeof payload.color === "string" ? payload.color : value);
      },
      fs: { calls: [], busy: false },
    },
  };
  // sk-desktop's `__SK_FS__`, as the inputs and links use it.
  const pickers = new Map<number, (v: unknown) => void>();
  const pick = (op: string, args: unknown[]): Promise<never> => {
    if (host.desktop.fs.busy) {
      host.desktop.fs.calls.push({ op, args, token: null });
      return Promise.resolve(undefined as never);
    }
    const token = host.desktop.nextToken++;
    host.desktop.fs.calls.push({ op, args, token });
    return new Promise((resolve) => pickers.set(token, resolve as (v: unknown) => void));
  };
  const fsHost = {
    pickForInput: (accept: string, multiple: boolean, directory: boolean, current: unknown) =>
      pick("pickForInput", [{ accept, multiple, directory }, current]),
    pickColor: (value: string) => pick("pickColor", [value]),
    download: (url: string, attr: string) =>
      void host.desktop.fs.calls.push({ op: "download", args: [url, attr], token: null }),
  };
  // sk-desktop's `__SK_SHELL__`, as far as links and `window.open` use it.
  const shellHost = {
    openContent(url: string): Promise<void> {
      host.desktop.calls.push({ op: "openExternal", args: [url, true] });
      return Promise.resolve();
    },
  };
  // compat.ts, per world: each world has its own console.
  const warned = new Set<string>();
  const hasWarned = (key: string): boolean => warned.has(key);
  const warnCompat = (key: string, msg: string): void => {
    if (warned.has(key)) return;
    warned.add(key);
    host.logs.push(`[skathi compat] ${msg}`);
  };
  const intern = (name: string): number => {
    let i = host.names.indexOf(name);
    if (i < 0) i = host.names.push(name) - 1;
    return i;
  };
  const nameOf = (id: number): string => host.names[id]!;
  let rootId = 0;
  const style = styleHost({
    op: (id, name, value) => host.styles.push({ id, key: name, value }),
    computed: (id) => host.computedById.get(id) ?? "",
    root: () => rootId,
    accepts: (name, value) => {
      host.acceptCalls.push(`${name}: ${value}`);
      return !host.styleRejects.has(`${name}: ${value}`);
    },
    warnCompat,
  });

  // A private `globalThis` per world: runtime.js installs missing DOM globals on it, and each test needs its own set.
  // redjs's `document.createElement`: a "dumb" element on HTMLElement.prototype with only a `tagName`, outside the
  // tree; the prototype is found off the first mirror the test creates.
  let elementProto: object | null = null;
  let worldExports: { nodeOps: { createElement(tag: string): object } } | null = null;
  const fakeGlobal = options.realm ?? {
    document: {
      body: {},
      documentElement: {},
      head: {},
      createElement(tag: string) {
        elementProto ??= worldExports
          ? Object.getPrototypeOf(worldExports.nodeOps.createElement("div"))
          : Object.prototype;
        return Object.assign(Object.create(elementProto), { tagName: String(tag).toUpperCase() });
      },
    },
    FileList: FakeFileList,
    DataTransfer: FakeDataTransfer,
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 0),
    // The windowing host (crates/sk-app/src/window.prelude.js) publishes itself on the real global before the
    // bundle evaluates, and runtime-window's tests fake it the same way; carry it into the private world.
    __SK_WINDOW__: (globalThis as { __SK_WINDOW__?: unknown }).__SK_WINDOW__,
    ...ENGINE_CLASSES,
    CSSStyleDeclaration: style.CSSStyleDeclaration,
    Blob: globalThis.Blob,
    DOMException: globalThis.DOMException,
    // Whatever a test says the prelude put on the global before the bundle ran.
    ...(options.globals ?? {}),
  };

  const fakeNavigator = {};

  const params = [
    "createRenderer",
    "createVNode",
    "defineComponent",
    "Fragment",
    "getCurrentInstance",
    "getTransitionRawChildren",
    "h",
    "resolveTransitionHooks",
    "setTransitionHooks",
    "toRaw",
    "useTransitionState",
    "BaseTransition",
    "BaseTransitionPropsValidators",
    "callWithAsyncErrorHandling",
    "version",
    "onBeforeUpdate",
    "onUpdated",
    "onMounted",
    "onUnmounted",
    "queuePostFlushCb",
    "Static",
    "watch",
    "nextTick",
    "shallowRef",
    "globalThis",
    "console",
    "__sk_intern",
    "__sk_create_element",
    "__sk_create_text",
    "__sk_create_anchor",
    "__sk_set_text",
    "__sk_set_element_text",
    "__sk_insert",
    "__sk_remove",
    "__sk_set_style",
    "__sk_set_class",
    "__sk_set_prop",
    "__sk_add_scope",
    "__sk_copy_scopes",
    "__sk_set_stylesheet",
    "__sk_set_listener",
    "__sk_create_root",
    "__sk_set_dispatcher",
    "__sk_log",
    "__sk_query",
    "__sk_bounds",
    "__sk_focus",
    "__sk_input_select",
    "__sk_reload",
    "__sk_media_state",
    "__sk_media_call",
    "__sk_media_can_play",
    "__sk_media_events",
    "__sk_media_prop",
    "__SK_FS__",
    "MouseEvent",
    "URL",
    "navigator",
    "__sk_style_accepts",
    "adoptCanvas",
    "__sk_set_layout_hook",
    "__sk_layout_watch",
    "installSelection",
    "__sk_selection_get",
    "__sk_selection_set",
    "__sk_selection_text",
    "__sk_selection_rebase",
    "__sk_hit_at",
    "__sk_pointer_capture",
    "__sk_set_node_lookup",
    "installWaapi",
    "__sk_waapi_core",
    "Teleport",
    "WINDOW_ROOT",
    "windowRoots",
    "shellHost",
    "hasWarned",
    "warnCompat",
    "isRef",
    "setDevtoolsHook",
    "__DEV__",
    "__sk_perf_advise",
    "__sk_perf_frame",
    "__sk_perf_visible",
    "__sk_toggle_begin",
    "__sk_toggle_end",
    "__sk_color_picker",
    "__sk_color_value",
    ...Object.keys(style.fns),
  ];
  const args = [
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
    BaseTransition,
    BaseTransitionPropsValidators,
    callWithAsyncErrorHandling,
    version,
    onBeforeUpdate,
    onUpdated,
    onMounted,
    onUnmounted,
    queuePostFlushCb,
    Static,
    watch,
    nextTick,
    shallowRef,
    fakeGlobal,
    {
      log: (m: unknown) => host.logs.push(String(m)),
      warn: (m: unknown) => host.logs.push(String(m)),
      error: (m: unknown) => host.logs.push(String(m)),
    },
    intern,
    (tag: number) => tree.create(nameOf(tag)),
    () => tree.create("#text"),
    () => tree.create("#comment"),
    (id: number, text: string) => tree.setText(id, text),
    (id: number, text: string) => tree.setElementText(id, text),
    (child: number, parent: number, anchor: number) => tree.insert(child, parent, anchor),
    (id: number) => tree.remove(id),
    (id: number, key: number, value: unknown) => host.styles.push({ id, key: nameOf(key), value }),
    (id: number, value: unknown) => host.classes.push({ id, value }),
    (id: number, key: number, value: unknown) => host.props.push({ id, key: nameOf(key), value }),
    () => {},
    () => {},
    (css: unknown, extra: unknown) => {
      const parts = [String(css), ...((extra as string[] | undefined) ?? []).map(String)];
      host.sheetParts.push(parts);
      host.sheets.push(parts.join("\n"));
    },
    (id: number, code: unknown, on: unknown) => host.listeners.push({ id, code, on }),
    () => (rootId = tree.create("root")),
    (fn: Dispatcher) => {
      host.dispatcher = fn;
    },
    (m: unknown) => host.logs.push(String(m)),
    (selector: string, root: unknown, mode: unknown) => {
      host.queryCalls.push({ selector, root, mode });
      return host.queryAnswers.get(`${selector}|${mode}`) ?? host.queryAnswers.get(selector) ?? "";
    },
    (id: number) => host.boundsById.get(id) ?? "",
    (id: number, op: number) => {
      host.focus.calls.push({ id, op });
      return host.focus.answer(id, op);
    },
    (id: number) => {
      host.inputSelectCalls.push(id);
      return host.inputSelectAnswer;
    },
    () => {
      host.reloads++;
    },
    (id: number) => {
      host.mediaStateReads++;
      return host.mediaById.get(id) ?? "";
    },
    (id: number, verb: unknown, arg: unknown) => {
      host.mediaCalls.push({ id, verb, arg });
      return true;
    },
    (_id: number, type: string) => host.canPlayByType.get(type) ?? "",
    () => "",
    (id: number, prop: number) => {
      host.mediaPropReads.push([id, prop]);
      const names = [
        "currentTime",
        "duration",
        "paused",
        "ended",
        "seeking",
        "readyState",
        "networkState",
        "volume",
        "muted",
        "playbackRate",
        "loop",
        "videoWidth",
        "videoHeight",
      ];
      const raw = host.mediaById.get(id);
      const line = raw?.split("\n").find((l) => l.startsWith(`${names[prop]}=`));
      if (line === undefined) return undefined;
      const v = line.slice(line.indexOf("=") + 1);
      return v === "true" ? 1 : v === "false" ? 0 : Number(v);
    },
    fsHost,
    options.MouseEvent ?? globalThis.MouseEvent,
    globalThis.URL,
    options.navigator ?? fakeNavigator,
    options.styleAccepts === false
      ? undefined
      : (key: number, value: string) => {
          host.acceptCalls.push(`${nameOf(key)}: ${value}`);
          return !host.styleRejects.has(`${nameOf(key)}: ${value}`);
        },
    loadAdoptCanvas(),
    options.layoutWatch === false
      ? undefined
      : (fn: (op: number) => number) => {
          host.layout.hook = fn;
        },
    options.layoutWatch === false
      ? undefined
      : (observing: boolean) => {
          host.layout.observing = observing;
          host.layout.pending = observing;
          host.layout.watches++;
        },
    loadInstallSelection(),
    () => (host.selection.sel ? host.selection.sel.join(",") : ""),
    (a: number, ao: number, f: number, fo: number) => {
      host.selection.sets.push([a, ao, f, fo]);
      const ok = a !== 0 && tree.live.has(a) && tree.live.has(f);
      host.selection.sel = ok ? [a, ao, 0, f, fo, 0] : null;
      return ok || a === 0;
    },
    () => host.selection.text,
    (el: number, text: number) => {
      host.selection.rebases.push([el, text]);
      const s = host.selection.sel;
      if (!s) return;
      for (const at of [0, 3]) if (s[at + 2] && s[at] === el) [s[at], s[at + 2]] = [text, 0];
    },
    (x: number, y: number, all: number) => {
      host.hit.calls.push([x, y, all]);
      return host.hit.answer(x, y, all);
    },
    (id: number, pointerId: number, capture: boolean) => {
      host.pointerCaptureCalls.push({ id, pointerId, capture });
      return true;
    },
    () => {},
    loadInstallWaapi(),
    options.anim === false
      ? undefined
      : {
          ...createReferenceWaapiCore(),
          setEffect(
            id: number,
            node: number,
            frames: Parameters<WaapiHost["setEffect"]>[2],
            timing: Parameters<WaapiHost["setEffect"]>[3],
          ) {
            const projected = referenceEffect(frames, host.anim.notes);
            host.anim.calls.push(["set", id, node, projected.frames, { ...timing }, timing.easing]);
            return projected.notes;
          },
          clock: (id: number, start: number, hold: number, rate: number) =>
            host.anim.calls.push(["clock", id, start, hold, rate]),
          drop: (id: number) => host.anim.calls.push(["drop", id]),
          values(id: number) {
            host.anim.calls.push(["values", id]);
            return referenceValues(host.anim.values);
          },
        },
    Teleport,
    ...SKWINDOW_STANDINS(),
    shellHost,
    hasWarned,
    warnCompat,
    isRef,
    setDevtoolsHook,
    options.dev ?? true,
    options.perf
      ? (code: string, key: string, message: string) => {
          host.perf.calls++;
          host.perf.advisories.push({ code, key, message });
          return true;
        }
      : undefined,
    options.perf
      ? () => {
          host.perf.calls++;
          return host.perf.frame;
        }
      : undefined,
    options.perf
      ? (id: number) => {
          host.perf.calls++;
          return host.perf.visible(id);
        }
      : undefined,
    () => -1,
    () => false,
    (id: number) => {
      host.colorPickerCalls.push(id);
      return host.colorPickerAnswer;
    },
    // The host's sanitization (sk-tree `color::sanitize`) for hex input; anything else is black.
    (text: string, alpha: boolean) => {
      const t = text.trim().toLowerCase();
      const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.test(t)
        ? t.length <= 5
          ? "#" + [...t.slice(1)].map((c) => c + c).join("")
          : t
        : "#000000";
      return alpha ? hex : hex.slice(0, 7);
    },
    ...Object.entries(style.fns).map(([name, fn]) => (...a: unknown[]) => {
      host.styleCalls[name] = (host.styleCalls[name] ?? 0) + 1;
      return Reflect.apply(fn, undefined, a);
    }),
  ];
  if (options.selection === false) {
    const first = params.indexOf("__sk_selection_get");
    for (let i = first; i < first + 4; i++) args[i] = undefined;
  }

  options.beforeRuntime?.(fakeGlobal);
  // eslint-disable-next-line no-new-func
  const exports: RuntimeExports = new Function(
    ...params,
    `${body}\nreturn { patchProp, nodeOps, EV_CODES, DERIVED, canonicalEvent, hyphenate,
       createApp, handlers, nodesById, nodeById, looseById, looseRoots, sweepLoose, Transition, TransitionGroup, resolveTransitionProps,
       writeClass, appWindow, defineWindow, getCurrentWindow,
       vModelText, vModelSelect, vShow, activation, noteActivation,
       render, useCssVars, useCssModule, VueElement, defineCustomElement, createSSRApp, hydrate,
       useHost, useShadowRoot, initDirectivesForSSR };`,
  )(...args);
  worldExports = exports;

  return {
    ...exports,
    host,
    global: fakeGlobal as RealmGlobal,
    el: (id = 1) => ({ __skId: id, value: "", attrs: Object.create(null), className: "" }),
    advisories: () => host.logs.filter((l) => l.startsWith("[skathi compat]")),
    flagsFor: (code: number) => host.listeners.filter((l) => l.code === code),
  };
}
