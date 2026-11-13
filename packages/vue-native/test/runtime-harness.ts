// Runs src/runtime.ts in an isolated fake host, without gpui, JavaScriptCore or a window. As in
// loader-harness.ts the source is transpiled (./ts-source.ts), import/export lines are stripped and every
// ambient name (`__vn_*`, `console`) is a parameter, so each createRuntime() gets its own world.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
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
import { readTsSource } from "./ts-source";
import { createReferenceWaapiCore, referenceEffect, referenceValues } from "./fixtures/waapi-core-reference";

const HERE = dirname(fileURLToPath(import.meta.url));
export const RUNTIME_PATH = join(HERE, "../src/runtime.ts");
export const DESKTOP_PATH = join(HERE, "../src/desktop.ts");

let installDesktopFn: unknown = null;
function loadInstallDesktop(): unknown {
  if (installDesktopFn) return installDesktopFn;
  const body = readTsSource(DESKTOP_PATH).replace(/^export\s+(function|const|let|class)\s/gm, "$1 ");
  // eslint-disable-next-line no-new-func
  installDesktopFn = new Function(`${body}\nreturn installDesktop;`)();
  return installDesktopFn;
}

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

export const FONTS_PATH = join(HERE, "../src/fonts.ts");
let installFontsFn: unknown = null;
function loadInstallFonts(): unknown {
  if (installFontsFn) return installFontsFn;
  const body = readTsSource(FONTS_PATH)
    .replace(/^\s*import\s[^;]*;[^\S\n]*$/gm, "")
    .replace(/^export\s+(function|const|let|class|interface)\s/gm, "$1 ");
  installFontsFn = new Function(`${body}\nreturn installFonts;`)();
  return installFontsFn;
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

export const PERMISSION_NAMES = [
  "microphone",
  "camera",
  "display-capture",
  "notifications",
  "clipboard-read",
  "clipboard-write",
  "geolocation",
];

const IMPORT_LINE = /^\s*import\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
const STAR_REEXPORT = /^\s*export\s+\*\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
/** `export { A, B } from "./mod";`: dropped, since every name handed back is a parameter or in the harness's own `return`. */
const NAMED_REEXPORT = /^\s*export\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
const LOCAL_EXPORT = /^\s*export\s+\{[^}]*\};?[^\S\n]*$/gm;

const VNWINDOW_STANDINS = (): unknown[] => [{ name: "VnWindow" }, Symbol("vn-window-root"), new Set<object>()];

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

/** `storage` / `devtools` / `desktop` `false` build a world without those `__vn_*` slots (an older or `--no-default-features` host); `MouseEvent` hands the world an event class (bun has none). */
export function createRuntime(
  options: {
    storage?: boolean;
    devtools?: boolean;
    desktop?: boolean;
    MouseEvent?: unknown;
    globals?: Record<string, unknown>;
    beforeRuntime?: (global: Record<string, unknown>) => void;
    launch?: Record<string, unknown>;
    styleAccepts?: boolean;
    layoutWatch?: boolean;
    selection?: boolean;
    anim?: boolean;
    realm?: Record<string, unknown>;
    navigator?: Record<string, unknown>;
  } = {},
) {
  const withStorage = options.storage !== false;
  const withDevtools = options.devtools !== false;
  const raw = readTsSource(RUNTIME_PATH);
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

  const host = {
    names: [],
    listeners: [], // { id, code, on }
    props: [], // { id, key, value }
    styles: [],
    classes: [],
    sheets: [] as string[],
    sheetParts: [] as string[][],
    logs: [],
    dispatcher: null,
    queryAnswers: new Map(),
    queryCalls: [],
    boundsById: new Map(),
    layout: {
      hook: null as ((op: number) => number) | null,
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
    scrollById: new Map(),
    scrollCalls: [],
    intoViewCalls: [],
    intoViewAnswer: "",
    scrollStale: false,
    selection: {
      sel: null as number[] | null,
      sets: [] as number[][],
      rebases: [] as number[][],
      text: "",
    },
    pointerCaptureCalls: [] as { id: number; pointerId: number; capture: boolean }[],
    hit: {
      answer: (_x: number, _y: number, _all: number) => "",
      calls: [] as number[][],
    },
    anim: {
      calls: [] as unknown[][],
      notes: "",
      values: null as string | null,
    },
    styleRejects: new Set<string>(),
    acceptCalls: [] as string[],
    inputSelectCalls: [] as number[],
    inputSelectAnswer: true,
    focus: {
      calls: [] as { id: number; op: number }[],
      answer: ((_id: number, op: number) => (op === 1 ? "" : "refused")) as (id: number, op: number) => string,
    },
    reloads: 0,
    mediaById: new Map(),
    mediaStateReads: 0,
    mediaPropReads: [] as number[][],
    mediaCalls: [],
    canPlayByType: new Map(),
    permissions: {
      platform: "TestOS",
      states: new Map(),
      requests: [],
      pendingToken: null,
      dispatcher: null,
      hints: new Map(),
      bySystem: new Set(),
      statuses: new Map(),
    },
    storage: {
      folders: new Map([
        ["APPDATA", "C:\\Users\\test\\AppData\\Roaming"],
        ["LOCALAPPDATA", "C:\\Users\\test\\AppData\\Local"],
        ["TEMP", "C:\\Users\\test\\AppData\\Local\\Temp"],
        ["DOCUMENTS", "C:\\Users\\test\\Documents"],
        ["HOME", "C:\\Users\\test"],
      ]),
      folder: "C:\\Users\\test\\AppData\\Local\\hello\\storage",
      encrypted: false,
      provider: "Windows DPAPI (per-user)",
      bytes: 0,
      pending: false,
      flushes: 0,
      error: null,
      refuse: null,
      flushError: null,
      folderCalls: [],
      setCalls: [],
    },
    devtools: {
      enabled: true,
      locked: false,
      key: true,
      port: 49221,
      inspectors: 0,
      setCalls: [],
      opens: 0,
      openError: null,
    },
    desktop: {
      dispatcher: null,
      calls: [],
      nextToken: 1,
      refuse: null,
      protocols: new Map(),
      launch: {
        argv: ["C:\\apps\\Test.exe"],
        cwd: "C:\\work",
        url: null,
        paths: [],
        singleInstance: false,
        id: "test",
        name: "test",
        ...(options.launch ?? {}),
      },
    },
  };
  host.desktop.forward = (launch: Record<string, unknown>) =>
    host.desktop.dispatcher?.(0, true, JSON.stringify({ kind: "launch", source: "second-instance", ...launch }));
  host.desktop.settle = (token, ok = true, payload = {}, value = undefined) =>
    host.desktop.dispatcher?.(token, ok, JSON.stringify(payload), value);
  host.desktop.fs = {
    calls: [],
    answers: new Map(),
    released: [],
    refused: [],
    factory: null,
    serializer: null,
    prompt: new Set(),
    permissionCalls: [],
  };
  const fsCall = (op, ...args) => {
    const answer = host.desktop.fs.answers.get(op);
    const out = answer ? answer(...args) : null;
    if (out && out.sync) {
      host.desktop.fs.calls.push({ op, args, token: null });
      return JSON.stringify(out.sync);
    }
    const token = host.desktop.nextToken++;
    host.desktop.fs.calls.push({ op, args, token });
    if (out) {
      queueMicrotask(() => host.desktop.settle(token, out.ok !== false, out.payload ?? {}, out.value));
    }
    return JSON.stringify({ token });
  };
  const shellCall = (op, ...rest) => {
    host.desktop.calls.push({ op, args: rest });
    if (host.desktop.refuse) {
      const r = host.desktop.refuse;
      host.desktop.refuse = null;
      return JSON.stringify(r);
    }
    return JSON.stringify({ token: host.desktop.nextToken++ });
  };
  host.permissions.settle = (token, state) => host.permissions.dispatcher?.(token, state);
  host.permissions.move = (name, state) => {
    host.permissions.states.set(name, state);
    host.permissions.statuses.get(name)?.dispatchEvent(new Event("change"));
  };
  const intern = (name) => {
    let i = host.names.indexOf(name);
    if (i < 0) i = host.names.push(name) - 1;
    return i;
  };
  const nameOf = (id) => host.names[id];

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
        elementProto ??= worldExports ? Object.getPrototypeOf(worldExports.nodeOps.createElement("div")) : Object.prototype;
        return Object.assign(Object.create(elementProto), { tagName: String(tag).toUpperCase() });
      },
    },
    FileList: FakeFileList,
    DataTransfer: FakeDataTransfer,
    requestAnimationFrame: (cb) => setTimeout(cb, 0),
    // The windowing host (crates/vn-app/src/window.prelude.js) publishes itself on the real global before the
    // bundle evaluates, and runtime-window's tests fake it the same way; carry it into the private world.
    __VN_WINDOW__: globalThis.__VN_WINDOW__,
    // The engine classes desktop.ts builds on (`DesktopEngine`), handed over off the runtime's own global.
    Blob: globalThis.Blob,
    DOMException: globalThis.DOMException,
    TextDecoder: globalThis.TextDecoder,
    WritableStream: globalThis.WritableStream,
    btoa: globalThis.btoa,
    // Whatever a test says the prelude put on the global before the bundle ran.
    ...(options.globals ?? {}),
  };

  const permState = (name) => host.permissions.states.get(name) ?? "unsupported";
  // The standard tier, faked: `PermissionStatus` is an EventTarget whose `state` is the three-word web
  // projection of the four-state answer (redjs's `PermissionState::web_state`).
  const fakeNavigator = {
    permissions: {
      query({ name }) {
        if (!PERMISSION_NAMES.includes(name)) {
          return Promise.reject(new TypeError(`'${name}' is not a valid enum value`));
        }
        let status = host.permissions.statuses.get(name);
        if (!status) {
          status = Object.assign(new EventTarget(), {
            name,
            get state() {
              const s = permState(name);
              return s === "unsupported" ? "denied" : s;
            },
          });
          host.permissions.statuses.set(name, status);
        }
        return Promise.resolve(status);
      },
    },
  };

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
    "version",
    "onBeforeUpdate",
    "onMounted",
    "onUnmounted",
    "queuePostFlushCb",
    "Static",
    "watch",
    "nextTick",
    "globalThis",
    "console",
    "__vn_intern",
    "__vn_create_element",
    "__vn_create_text",
    "__vn_create_anchor",
    "__vn_set_text",
    "__vn_set_element_text",
    "__vn_insert",
    "__vn_remove",
    "__vn_set_style",
    "__vn_set_class",
    "__vn_set_prop",
    "__vn_add_scope",
    "__vn_set_stylesheet",
    "__vn_set_listener",
    "__vn_create_root",
    "__vn_set_dispatcher",
    "__vn_log",
    "__vn_query",
    "__vn_bounds",
    "__vn_computed_style",
    "__vn_scroll",
    "__vn_scroll_into_view",
    "__vn_focus",
    "__vn_input_select",
    "__vn_reload",
    "__vn_media_state",
    "__vn_media_call",
    "__vn_media_can_play",
    "__vn_media_events",
    "__vn_media_prop",
    "__vn_permissions_platform",
    "__vn_permissions_query",
    "__vn_permissions_list",
    "__vn_permissions_request",
    "__vn_permissions_hint",
    "__vn_permissions_set_dispatcher",
    "__vn_storage_user_folder",
    "__vn_storage_set_folder",
    "__vn_storage_info",
    "__vn_storage_flush",
    "__vn_devtools_state",
    "__vn_devtools_set_enabled",
    "__vn_devtools_open",
    "__vn_desktop_set_dispatcher",
    "__vn_desktop_get_screen_sources",
    "__vn_shell_open_external",
    "__vn_shell_open_path",
    "__vn_shell_show_item",
    "__vn_shell_trash",
    "__vn_launch_info",
    "__vn_launch_register_protocol",
    "__vn_launch_unregister_protocol",
    "__vn_launch_is_protocol_registered",
    "__vn_fs_set_handle_factory",
    "__vn_fs_release",
    "__vn_fs_get_file",
    "__vn_fs_child",
    "__vn_fs_remove",
    "__vn_fs_same",
    "__vn_fs_resolve",
    "__vn_fs_dir_open",
    "__vn_fs_dir_next",
    "__vn_fs_dir_close",
    "__vn_fs_writable_open",
    "__vn_fs_writable_write",
    "__vn_fs_writable_seek",
    "__vn_fs_writable_truncate",
    "__vn_fs_writable_close",
    "__vn_fs_writable_abort",
    "__vn_fs_refuse_clone",
    "__vn_fs_set_serializer",
    "__vn_fs_permission",
    "__vn_dialog_open",
    "__vn_download",
    "installDesktop",
    "MouseEvent",
    "URL",
    "navigator",
    "__vn_style_accepts",
    "adoptCanvas",
    "installFonts",
    "__vn_set_layout_hook",
    "__vn_layout_watch",
    "installSelection",
    "__vn_selection_get",
    "__vn_selection_set",
    "__vn_selection_text",
    "__vn_selection_rebase",
    "__vn_hit_at",
    "__vn_pointer_capture",
    "installWaapi",
    "__vn_waapi_core",
    "Teleport",
    "VnWindow",
    "WINDOW_ROOT",
    "windowRoots",
  ];
  let nextId = 1;
  const tree = {
    parent: new Map<number, number>(),
    children: new Map<number, number[]>(),
    tag: new Map<number, string>(),
    text: new Map<number, string>(),
    live: new Set<number>(),
    freed: [] as number[],
    errors: [] as string[],
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
      if (p) tree.children.set(p, tree.children.get(p)!.filter((c) => c !== id));
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
  (host as Record<string, unknown>).tree = tree;
  const devtoolsState = () => ({
    enabled: host.devtools.enabled,
    locked: host.devtools.locked,
    port: host.devtools.enabled ? host.devtools.port : null,
    pid: 4242,
    key: host.devtools.key,
    inspectors: host.devtools.inspectors,
    slot: "C:\\vn\\dist",
  });
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
    version,
    onBeforeUpdate,
    onMounted,
    onUnmounted,
    queuePostFlushCb,
    Static,
    watch,
    nextTick,
    fakeGlobal,
    {
      log: (m) => host.logs.push(String(m)),
      warn: (m) => host.logs.push(String(m)),
      error: (m) => host.logs.push(String(m)),
    },
    intern,
    (tag) => tree.create(nameOf(tag)),
    () => tree.create("#text"),
    () => tree.create("#comment"),
    (id, text) => tree.setText(id, text),
    (id, text) => tree.setElementText(id, text),
    (child, parent, anchor) => tree.insert(child, parent, anchor),
    (id) => tree.remove(id),
    (id, key, value) => host.styles.push({ id, key: nameOf(key), value }),
    (id, value) => host.classes.push({ id, value }),
    (id, key, value) => host.props.push({ id, key: nameOf(key), value }),
    () => {},
    (css, extra) => {
      const parts = [String(css), ...((extra as string[] | undefined) ?? []).map(String)];
      host.sheetParts.push(parts);
      host.sheets.push(parts.join("\n"));
    },
    (id, code, on) => host.listeners.push({ id, code, on }),
    () => tree.create("root"),
    (fn) => {
      host.dispatcher = fn;
    },
    (m) => host.logs.push(String(m)),
    (selector, root, mode) => {
      host.queryCalls.push({ selector, root, mode });
      return host.queryAnswers.get(`${selector}|${mode}`) ?? host.queryAnswers.get(selector) ?? "";
    },
    (id) => host.boundsById.get(id) ?? "",
    (id) => host.computedById.get(id) ?? "",
    (id, op, x, y) => {
      host.scrollCalls.push({ id, op, x, y });
      const s = host.scrollById.get(id);
      if (!s) return "";
      const clamp = (v, max) => Math.max(0, Math.min(v, max));
      if (op === 1) {
        if (Number.isFinite(x)) s.left = clamp(x, s.maxLeft);
        if (Number.isFinite(y)) s.top = clamp(y, s.maxTop);
      } else if (op === 2) {
        if (Number.isFinite(x)) s.left = clamp(s.left + x, s.maxLeft);
        if (Number.isFinite(y)) s.top = clamp(s.top + y, s.maxTop);
      }
      return `${s.left},${s.top},${s.width + s.maxLeft},${s.height + s.maxTop},${host.scrollStale ? 1 : 0}`;
    },
    (id, block, inline) => {
      host.intoViewCalls.push({ id, block, inline });
      return host.intoViewAnswer;
    },
    (id, op) => {
      host.focus.calls.push({ id, op });
      return host.focus.answer(id, op);
    },
    (id) => { host.inputSelectCalls.push(id); return host.inputSelectAnswer; },
    () => {
      host.reloads++;
    },
    (id) => {
      host.mediaStateReads++;
      return host.mediaById.get(id) ?? "";
    },
    (id, verb, arg) => {
      host.mediaCalls.push({ id, verb, arg });
      return true;
    },
    (_id, type) => host.canPlayByType.get(type) ?? "",
    () => "",
    (id, prop) => {
      host.mediaPropReads.push([id, prop]);
      const names = ["currentTime", "duration", "paused", "ended", "seeking", "readyState", "networkState",
        "volume", "muted", "playbackRate", "loop", "videoWidth", "videoHeight"];
      const raw = host.mediaById.get(id);
      const line = raw?.split("\n").find((l) => l.startsWith(`${names[prop]}=`));
      if (line === undefined) return undefined;
      const v = line.slice(line.indexOf("=") + 1);
      return v === "true" ? 1 : v === "false" ? 0 : Number(v);
    },
    () => host.permissions.platform,
    (name) => (PERMISSION_NAMES.includes(name) ? permState(name) : ""),
    () =>
      JSON.stringify(
        PERMISSION_NAMES.map((name) => ({
          name,
          state: permState(name),
          canRequest: permState(name) === "denied" || permState(name) === "prompt",
          deniedBySystem: permState(name) === "denied" && host.permissions.bySystem.has(name),
        })),
      ),
    (name) => {
      host.permissions.requests.push(name);
      if (!PERMISSION_NAMES.includes(name)) {
        return JSON.stringify({ error: `${name} is not a permission name` });
      }
      if (host.permissions.pendingToken !== null) {
        return JSON.stringify({ token: host.permissions.pendingToken });
      }
      return JSON.stringify({ state: permState(name) });
    },
    (name) => host.permissions.hints.get(name) ?? "",
    (fn) => {
      host.permissions.dispatcher = fn;
    },
    (kind, sub) => {
      host.storage.folderCalls.push({ kind, sub });
      const base = host.storage.folders.get(kind);
      if (!base) return JSON.stringify({ error: `"${kind}" is not a user folder kind` });
      if (sub.includes("..")) return JSON.stringify({ error: `subfolder "${sub}" must not …` });
      return JSON.stringify({ path: sub ? `${base}\\${sub.replace(/\//g, "\\")}` : base });
    },
    (path, encrypted) => {
      host.storage.setCalls.push({ path, encrypted });
      if (host.storage.refuse) return JSON.stringify({ error: host.storage.refuse });
      host.storage.folder = path;
      if (encrypted !== "keep") host.storage.encrypted = encrypted === "true";
      return JSON.stringify({ path });
    },
    () =>
      JSON.stringify({
        folder: host.storage.folder,
        file: `${host.storage.folder}\\localStorage.${host.storage.encrypted ? "sealed" : "json"}`,
        encrypted: host.storage.encrypted,
        bytes: host.storage.bytes,
        loaded: true,
        pending: host.storage.pending,
        provider: host.storage.provider,
        kinds: [...host.storage.folders.keys()],
        error: host.storage.error,
      }),
    () => {
      if (host.storage.flushError) return JSON.stringify({ error: host.storage.flushError });
      const wrote = host.storage.pending;
      host.storage.pending = false;
      host.storage.flushes++;
      return JSON.stringify({ wrote });
    },
    () => JSON.stringify(devtoolsState()),
    (on) => {
      host.devtools.setCalls.push(on);
      if (!on && host.devtools.locked) {
        return JSON.stringify({
          error: "VN_DEVTOOLS=1 is set in this process's environment, which holds the devtools on",
        });
      }
      host.devtools.enabled = !!on;
      host.devtools.port = on ? 49221 : null;
      return JSON.stringify(devtoolsState());
    },
    () => {
      host.devtools.opens++;
      if (host.devtools.openError) return JSON.stringify({ error: host.devtools.openError });
      host.devtools.inspectors++;
      return JSON.stringify({ opened: true, window: 4294967297 });
    },
    (fn) => {
      host.desktop.dispatcher = fn;
    },
    (types, options) => shellCall("screenSources", JSON.parse(types), JSON.parse(options)),
    (url, content) => {
      if (content && !/^(https?|mailto):/i.test(url)) {
        return JSON.stringify({ error: "content links only open http, https and mailto", name: "NotAllowedError", code: "ESCHEME" });
      }
      return shellCall("openExternal", url, content);
    },
    (path, allow) => shellCall("openPath", path, allow),
    (path) => shellCall("showItem", path),
    (path) => shellCall("trash", path),
    () => JSON.stringify(host.desktop.launch),
    (scheme) => {
      const s = String(scheme).toLowerCase();
      if (!/^[a-z][a-z0-9+.-]{0,62}$/.test(s) || ["http", "https", "file", "mailto"].includes(s) || s.startsWith("ms-")) {
        return JSON.stringify({ error: `registerProtocol: "${scheme}" cannot be registered`, name: "TypeError" });
      }
      const owner = host.desktop.protocols.get(s);
      if (owner !== undefined && owner !== "test") {
        return JSON.stringify({ error: `registerProtocol: the "${s}" scheme is registered to ${owner}`, name: "TypeError" });
      }
      host.desktop.protocols.set(s, "test");
      return JSON.stringify({ ok: true, wrote: owner === undefined, scheme: s });
    },
    (scheme) => host.desktop.protocols.get(String(scheme).toLowerCase()) === "test" &&
      host.desktop.protocols.delete(String(scheme).toLowerCase()),
    (scheme) => host.desktop.protocols.get(String(scheme).toLowerCase()) === "test",
    (fn) => {
      host.desktop.fs.factory = fn;
    },
    (h) => host.desktop.fs.released.push(h),
    (h) => fsCall("getFile", h),
    (dir, name, isDir, create) => fsCall("child", dir, name, isDir, create),
    (dir, name, recursive) => fsCall("remove", dir, name, recursive),
    (a, b) => fsCall("same", a, b),
    (root, other) => fsCall("resolve", root, other),
    (dir) => fsCall("dirOpen", dir),
    (cursor) => fsCall("dirNext", cursor),
    (cursor) => host.desktop.fs.calls.push({ op: "dirClose", args: [cursor], token: null }),
    (h, keep) => fsCall("writableOpen", h, keep),
    (w, position, data) => fsCall("writableWrite", w, position, data),
    (w, position) => fsCall("writableSeek", w, position),
    (w, size) => fsCall("writableTruncate", w, size),
    (w) => fsCall("writableClose", w),
    (w) => fsCall("writableAbort", w),
    (proto, name) => host.desktop.fs.refused.push({ proto, name }),
    (proto, idOf) => {
      host.desktop.fs.serializer = { proto, idOf };
    },
    (h, request) => {
      host.desktop.fs.permissionCalls.push({ h, request });
      if (request) host.desktop.fs.prompt.delete(h);
      return host.desktop.fs.prompt.has(h) ? "prompt" : "granted";
    },
    (json, current) => fsCall("dialogOpen", JSON.parse(json), current),
    (url, attr) => fsCall("download", url, attr),
    loadInstallDesktop(),
    options.MouseEvent ?? globalThis.MouseEvent,
    globalThis.URL,
    options.navigator ?? fakeNavigator,
    options.styleAccepts === false
      ? undefined
      : (key, value) => {
          host.acceptCalls.push(`${nameOf(key)}: ${value}`);
          return !host.styleRejects.has(`${nameOf(key)}: ${value}`);
        },
    loadAdoptCanvas(),
    loadInstallFonts(),
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
    loadInstallWaapi(),
    options.anim === false ? undefined : {
      ...createReferenceWaapiCore(),
      setEffect(id: number, node: number, frames: any[], timing: any) {
        const projected = referenceEffect(frames, host.anim.notes);
        host.anim.calls.push(["set", id, node, projected.frames, { ...timing }, timing.easing]);
        return projected.notes;
      },
      clock: (id: number, start: number, hold: number, rate: number) => host.anim.calls.push(["clock", id, start, hold, rate]),
      drop: (id: number) => host.anim.calls.push(["drop", id]),
      values(id: number) { host.anim.calls.push(["values", id]); return referenceValues(host.anim.values); },
    },
    Teleport,
    ...VNWINDOW_STANDINS(),
  ];
  if (options.selection === false) {
    const first = params.indexOf("__vn_selection_get");
    for (let i = first; i < first + 4; i++) args[i] = undefined;
  }

  if (!withStorage) {
    const first = params.indexOf("__vn_storage_user_folder");
    for (let i = first; i < first + 4; i++) args[i] = undefined;
  }
  if (!withDevtools) {
    const first = params.indexOf("__vn_devtools_state");
    for (let i = first; i < first + 3; i++) args[i] = undefined;
  }
  if (options.desktop === false) {
    const first = params.indexOf("__vn_desktop_set_dispatcher");
    const last = params.indexOf("__vn_download");
    for (let i = first; i <= last; i++) args[i] = undefined;
  }

  options.beforeRuntime?.(fakeGlobal);
  // eslint-disable-next-line no-new-func
  const exports = new Function(
    ...params,
    `${body}\nreturn { patchProp, nodeOps, EV_CODES, DERIVED, canonicalEvent, hyphenate,
       createApp, handlers, nodesById, nodeById, looseById, looseRoots, sweepLoose, Transition, TransitionGroup, resolveTransitionProps,
       writeClass, appWindow, defineWindow, getCurrentWindow, appPermissions, appStorage,
       appDevtools, appShell, appCapture, appLaunch, vModelText, vModelSelect, vShow, desktopApi, activation, noteActivation,
       render, useCssVars, useCssModule, VueElement, defineCustomElement, createSSRApp, hydrate,
       useHost, useShadowRoot, initDirectivesForSSR };`,
  )(...args);
  worldExports = exports;

  return {
    ...exports,
    host,
    global: fakeGlobal,
    el: (id = 1) => ({ __vnId: id, value: "", attrs: Object.create(null), className: "" }),
    advisories: () => host.logs.filter((l) => l.startsWith("[vue-native compat]")),
    flagsFor: (code) => host.listeners.filter((l) => l.code === code),
  };
}
