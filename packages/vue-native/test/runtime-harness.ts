// Runs src/runtime.ts in an isolated fake host so the renderer's prop/event
// plumbing can be tested without gpui, JavaScriptCore or a window.
//
// Same trick as loader-harness.ts: runtime.ts is authored as an ES module for
// the bundler's benefit, but its body only touches ambient names — the `__vn_*`
// host functions and `console`. The source is transpiled to plain JS (see
// ./ts-source.ts — bun's own emitter, the one that produces dist/vendor.js),
// the import/export lines are stripped, and every ambient name is bound as a
// PARAMETER, so each createRuntime() call gets its own private world (own
// intern table, own handler registry, own `warned` set, which is what makes
// "advisory fires exactly once" testable at all).

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
  onBeforeUpdate,
  onMounted,
  onUnmounted,
  queuePostFlushCb,
  resolveTransitionHooks,
  setTransitionHooks,
  Static,
  toRaw,
  useTransitionState,
  version,
  watch,
} from "@vue/runtime-core";
import { readTsSource } from "./ts-source";

const HERE = dirname(fileURLToPath(import.meta.url));
export const RUNTIME_PATH = join(HERE, "../src/runtime.ts");
export const DESKTOP_PATH = join(HERE, "../src/desktop.ts");

/**
 * src/desktop.ts, evaluated once into its one export. runtime.ts IMPORTS it
 * (`import { installDesktop } from "./desktop"`), and that line goes with the
 * other imports below; the function comes back in as a parameter. desktop.ts
 * reads no ambient `__vn_*` name — its host, engine classes and settle
 * channel are arguments — so one evaluation serves every world.
 */
let installDesktopFn: unknown = null;
function loadInstallDesktop(): unknown {
  if (installDesktopFn) return installDesktopFn;
  const body = readTsSource(DESKTOP_PATH).replace(/^export\s+(function|const|let|class)\s/gm, "$1 ");
  // eslint-disable-next-line no-new-func
  installDesktopFn = new Function(`${body}\nreturn installDesktop;`)();
  return installDesktopFn;
}

/** The W3C names `vn-permissions` knows — kept in step with `Capability::ALL`. */
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
/**
 * `export { A, B } from "./mod";` — a NAMED re-export of another module.
 *
 * `<VnWindow>` gave runtime.ts its first one (the component lives in
 * src/vnwindow.ts and is re-exported so an app imports one module), and this
 * harness evaluates ONE file with `new Function`: the module a re-export names
 * is not being loaded, and `export` is a syntax error inside a function body
 * either way. Dropping the line is right rather than merely expedient — every
 * name the harness hands back is bound as a parameter or listed in its own
 * `return`, so nothing here was ever reached through a re-export, and
 * `<VnWindow>` is a component the runtime's own tests never render
 * (packages/vn-test drives it against a real host instead).
 */
const NAMED_REEXPORT = /^\s*export\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
/**
 * `export { nodeOps, patchProp };` — a LOCAL export list (runtime-dom exports
 * its renderer options, so runtime.ts exports its own). The names are already
 * declarations in the body, so dropping the list loses nothing.
 */
const LOCAL_EXPORT = /^\s*export\s+\{[^}]*\};?[^\S\n]*$/gm;

/**
 * `FileList`, as much of it as an input's `files` needs (redjs has the real
 * class; bun has none): indexed, `length`, `item()`, iterable, and a class for
 * `instanceof`. A test makes one from bun's own `File`s.
 */
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

/** `DataTransfer`, for `new DataTransfer().files` (an empty FileList). */
class FakeDataTransfer {
  files = new FakeFileList();
}

/** A fresh runtime world with a recording host. */
/**
 * @param {{ storage?: boolean, devtools?: boolean }} [options] —
 *   `storage: false` builds a world with no `__vn_storage_*` at all, which is
 *   what a host compiled `--no-default-features` (or an older one) looks like
 *   from JS. It is the only way to reach `appStorage`'s inert branch, and that
 *   branch is the one that must not invent a plausible path.
 *
 *   `devtools: false` does the same for `__vn_devtools_*`, i.e. a host built
 *   `--features no-devtools` (docs/DEVTOOLS.md "Shape"). That branch has to
 *   tell an app the devtools are not merely off but ABSENT, without throwing —
 *   `appDevtools.disable()` in an entry module is its likeliest caller.
 *
 *   `desktop: false` blanks `__vn_desktop_*` / `__vn_shell_*` (no vn-desktop).
 *   `MouseEvent` hands the world an event class (bun has none), which is what
 *   makes the "only a MouseEvent click activates" rule observable.
 */
export function createRuntime(
  options: {
    storage?: boolean;
    devtools?: boolean;
    desktop?: boolean;
    MouseEvent?: unknown;
    globals?: Record<string, unknown>;
    launch?: Record<string, unknown>;
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
    .replace(STAR_REEXPORT, "")
    .replace(NAMED_REEXPORT, "")
    .replace(LOCAL_EXPORT, "")
    // `export function foo` / `export const foo` become plain declarations, so
    // the whole module body is valid inside a function.
    .replace(/^export\s+(function|const|let|class)\s/gm, "$1 ");

  const host = {
    /** interned name -> id, and back */
    names: [],
    listeners: [], // { id, code, on }
    props: [], // { id, key, value }
    styles: [],
    classes: [],
    logs: [],
    dispatcher: null,
    /** `__vn_query` answers: selector -> ids (or a `!advisory` string). */
    queryAnswers: new Map(),
    queryCalls: [],
    /** `__vn_bounds` answers: node id -> "x,y,w,h". */
    boundsById: new Map(),
    /** `__vn_computed_style` answers: node id -> "k=v\n…". */
    computedById: new Map(),
    /**
     * The scroll containers this fake host has, keyed by node id:
     * `{ left, top, maxLeft, maxTop, width, height }` — the viewport size plus
     * how far the content overflows it, which is exactly what a real
     * `ScrollHandle` holds. A node that is not in here is not a scroll
     * container, and `__vn_scroll` answers `""` for it, as the host does.
     */
    scrollById: new Map(),
    /** Every `__vn_scroll` call: `{ id, op, x, y }`. */
    scrollCalls: [],
    /** Every `__vn_scroll_into_view` call: `{ id, block, inline }`. */
    intoViewCalls: [],
    /** What `__vn_scroll_into_view` answers — `""`, `"stale"`, or `"!advisory"`. */
    intoViewAnswer: "",
    /**
     * Is the host reporting that the CONTENT half of a scroll answer is from
     * the last painted frame? `__vn_scroll`'s fifth field — see
     * `crates/vn-tree/src/scroll.rs` and `noteStale` in runtime.ts.
     */
    scrollStale: false,
    /** `__vn_media_state` answers: node id -> "name=value\n…". */
    mediaById: new Map(),
    /** `__vn_media_call` log: { id, verb, arg }. */
    mediaCalls: [],
    /** `__vn_media_can_play` answers: mime type -> "" | "maybe" | "probably". */
    canPlayByType: new Map(),
    /**
     * The permission backend (`vn-permissions`), faked. `states` is what
     * `__vn_permissions_query` answers; set `pendingToken` to make `request()`
     * return a token instead of settling, then call `settle()` to deliver it —
     * that is the vn-app pump, in one line.
     */
    permissions: {
      platform: "TestOS",
      states: new Map(),
      requests: [],
      pendingToken: null,
      dispatcher: null,
      /** The fake `navigator.permissions` PermissionStatus objects, by name. */
      statuses: new Map(),
    },
    /**
     * The storage backend (`vn-storage`), faked. `folders` is the known-folder
     * table `folders.rs` resolves; `refuse`/`flushError` make the two failing
     * calls fail, which is the half of `appStorage` that has to throw rather
     * than return a path nobody checked.
     */
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
    /**
     * The devtools switch (`crates/vn-app/src/devtools/mod.rs`), faked. The
     * Rust half owns a listener and a socket; the JS half's whole job is the
     * JSON round trip, the refusal advisory and the `onChange` fan-out, so the
     * fake is the state machine and a call log.
     */
    devtools: {
      enabled: true,
      /** `VN_DEVTOOLS=1`: `set_enabled(false)` answers `{error}` instead. */
      locked: false,
      key: true,
      port: 49221,
      inspectors: 0,
      /** Every `__vn_devtools_set_enabled` argument, in order. */
      setCalls: [],
      /** How many times `__vn_devtools_open` was called. */
      opens: 0,
      /** When set, `open()` answers `{error}` with this text. */
      openError: null,
    },
    /**
     * The desktop host (`crates/vn-desktop`), faked at the seam: every
     * `__vn_shell_*` call is recorded and answers `{token}` (or the refusal
     * in `refuse`, verbatim); `settle(token, ok, payload)` is the frame pump.
     * The scheme allowlist and the executable guard are the RUST side's — the
     * fake applies only the content allowlist, so a test can tell whether the
     * JS refused before ever reaching the host.
     */
    desktop: {
      dispatcher: null,
      calls: [],
      nextToken: 1,
      /** When set, the next shell call answers this object (a sync refusal). */
      refuse: null,
      /** The fake registry: scheme -> owner id ("test" is this app). */
      protocols: new Map(),
      /** What `__vn_launch_info` answers — this world's own launch. */
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
  /** Deliver a forwarded second launch, as the pipe thread + pump would. */
  host.desktop.forward = (launch: Record<string, unknown>) =>
    host.desktop.dispatcher?.(0, true, JSON.stringify({ kind: "launch", source: "second-instance", ...launch }));
  /** Settle a desktop token as `vn_desktop::pump()` would (`value`: what the host made). */
  host.desktop.settle = (token, ok = true, payload = {}, value = undefined) =>
    host.desktop.dispatcher?.(token, ok, JSON.stringify(payload), value);
  /**
   * The File System Access + dialog host (`vn-desktop` fs/, dialog/), faked
   * at the seam. Every call is recorded in `fs.calls` as `{ op, args, token }`
   * and answers `{token}`. An entry in `fs.answers` (op → fn(...args)) makes
   * the fake settle that call itself, a microtask later — the pump source,
   * in one line — with what the fn returns: `{ ok?, payload?, value? }`, or
   * `{ sync }` for a synchronous refusal. Without one, a test settles the
   * token by hand with `host.desktop.settle`.
   */
  host.desktop.fs = {
    calls: [],
    answers: new Map(),
    released: [],
    refused: [],
    factory: null,
    /** What `__vn_fs_set_serializer` was handed: `{ proto, idOf }`. */
    serializer: null,
    /** Handle ids whose permission is "prompt" (restored, not yet granted). */
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
  /** Deliver a settled request as the host's per-frame pump would. */
  host.permissions.settle = (token, state) => host.permissions.dispatcher?.(token, state);
  /** Move a capability and fire the standard-tier `change`, as the OS would. */
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

  // A private `globalThis` per world: runtime.js installs the missing DOM
  // globals (MutationObserver, DocumentFragment, getComputedStyle, …) on it,
  // and each test must get its own set rather than whichever world happened to
  // run first.
  // redjs's `document.createElement`: a "dumb" element on HTMLElement.prototype
  // with only a `tagName`, outside the tree. The prototype is the world's own
  // element prototype, found off the first mirror the test creates.
  let elementProto: object | null = null;
  let worldExports: { nodeOps: { createElement(tag: string): object } } | null = null;
  const fakeGlobal = {
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
    // The windowing host (crates/vn-app/src/window.prelude.js) publishes itself
    // on the REAL global before the app bundle evaluates, and runtime-window's
    // tests fake it the same way. Carry it into the private world so the two
    // stay compatible: the world is private so that each test gets its own DOM
    // globals, not so that it is sealed off from the host.
    __VN_WINDOW__: globalThis.__VN_WINDOW__,
    // The engine classes desktop.ts builds on (its `DesktopEngine`), which the
    // runtime hands it off its own global. bun has all five; redjs has all
    // five too, which is the point of testing against bun's.
    Blob: globalThis.Blob,
    DOMException: globalThis.DOMException,
    TextDecoder: globalThis.TextDecoder,
    WritableStream: globalThis.WritableStream,
    btoa: globalThis.btoa,
    // Whatever a test says the PRELUDE put on the global before the bundle
    // ran (window.open's A-22, say).
    ...(options.globals ?? {}),
  };

  const permState = (name) => host.permissions.states.get(name) ?? "unsupported";
  // The STANDARD tier, faked: `PermissionStatus` is an EventTarget whose
  // `state` is the three-word web projection of the four-state answer, which is
  // exactly what redjs's `PermissionState::web_state` does.
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
    "__vn_media_state",
    "__vn_media_call",
    "__vn_media_can_play",
    "__vn_media_events",
    "__vn_permissions_platform",
    "__vn_permissions_query",
    "__vn_permissions_list",
    "__vn_permissions_request",
    "__vn_permissions_set_dispatcher",
    "__vn_storage_user_folder",
    "__vn_storage_set_folder",
    "__vn_storage_info",
    "__vn_storage_flush",
    "__vn_devtools_state",
    "__vn_devtools_set_enabled",
    "__vn_devtools_open",
    "__vn_desktop_set_dispatcher",
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
    // desktop.ts's one export (see `loadInstallDesktop`).
    "installDesktop",
    // Bound as parameters so the link path's `new MouseEvent` / `new URL` are
    // the world's own: bun has a `URL` but no `MouseEvent`, and a test that
    // wants the engine's "is this a MouseEvent?" check can hand one in.
    "MouseEvent",
    "URL",
    // Bound as a parameter rather than left ambient so a test can hand
    // `appPermissions` a `navigator.permissions` to re-broadcast `change` from
    // — bun has a `navigator`, but not that.
    "navigator",
  ];
  let nextId = 1;
  /** Exactly the object `devtools::state_json` writes. */
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
    fakeGlobal,
    {
      log: (m) => host.logs.push(String(m)),
      warn: (m) => host.logs.push(String(m)),
      error: (m) => host.logs.push(String(m)),
    },
    intern,
    () => nextId++,
    () => nextId++,
    () => nextId++,
    () => {},
    () => {},
    () => {},
    () => {},
    (id, key, value) => host.styles.push({ id, key: nameOf(key), value }),
    (id, value) => host.classes.push({ id, value }),
    (id, key, value) => host.props.push({ id, key: nameOf(key), value }),
    () => {},
    () => {},
    (id, code, on) => host.listeners.push({ id, code, on }),
    () => nextId++,
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
    // `__vn_scroll` — the same clamp the host applies (`scroll::apply`), so a
    // test that asserts "a write past the end lands at the end" is asserting
    // the contract rather than the fake.
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
    (id) => host.mediaById.get(id) ?? "",
    (id, verb, arg) => {
      host.mediaCalls.push({ id, verb, arg });
      return true;
    },
    (_id, type) => host.canPlayByType.get(type) ?? "",
    () => "",
    // ---- __vn_permissions_* ------------------------------------------------
    () => host.permissions.platform,
    (name) => (PERMISSION_NAMES.includes(name) ? permState(name) : ""),
    () =>
      JSON.stringify(
        PERMISSION_NAMES.map((name) => ({
          name,
          state: permState(name),
          canRequest: permState(name) === "denied" || permState(name) === "prompt",
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
    (fn) => {
      host.permissions.dispatcher = fn;
    },
    // ---- __vn_storage_* ----------------------------------------------------
    // The whole crate in four closures. `folders` is what
    // `crates/vn-storage/src/folders.rs` resolves, `error` is what a rejected
    // call answers with — the JS half's only job is to unwrap that and throw.
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
      // "keep" — the app did not say — leaves the format alone, as lib.rs does.
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
    // ---- __vn_devtools_* ---------------------------------------------------
    // `state` is the reader, `set_enabled` returns the SETTLED state (or
    // `{error}` when the environment holds the switch), `open` is programmatic
    // F12. The Rust side is `devtools::{state_json, set_enabled}` +
    // `inspector::open_for_window`.
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
    // ---- __vn_desktop_* / __vn_shell_* --------------------------------------
    (fn) => {
      host.desktop.dispatcher = fn;
    },
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
    // The registry, faked as scheme -> owner id. `launch/protocol_win.rs` is
    // the real one (and its unit tests run against a scratch HKCU subtree).
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
    // ---- __vn_fs_* / __vn_dialog_open / __vn_download (see `fsCall`) ---------
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
    // The session's grants, faked per handle id: an id in `prompt` is a
    // restored handle nobody granted yet; `request` grants it.
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
    fakeNavigator,
  ];

  if (!withStorage) {
    // Four `undefined`s in the storage slots: `typeof __vn_storage_info` is
    // then "undefined" and runtime.js takes the inert branch.
    const first = params.indexOf("__vn_storage_user_folder");
    for (let i = first; i < first + 4; i++) args[i] = undefined;
  }
  if (!withDevtools) {
    // …and the same three slots blank, which is what `--features no-devtools`
    // looks like from JS: `appDevtools` goes inert and says so.
    const first = params.indexOf("__vn_devtools_state");
    for (let i = first; i < first + 3; i++) args[i] = undefined;
  }
  if (options.desktop === false) {
    // A host without vn-desktop: `appShell` goes inert, links advise, there
    // are no pickers. Every `__vn_desktop_*` … `__vn_download` slot blanks.
    const first = params.indexOf("__vn_desktop_set_dispatcher");
    const last = params.indexOf("__vn_download");
    for (let i = first; i <= last; i++) args[i] = undefined;
  }

  // eslint-disable-next-line no-new-func
  const exports = new Function(
    ...params,
    `${body}\nreturn { patchProp, nodeOps, EV_CODES, DERIVED, canonicalEvent, hyphenate,
       createApp, handlers, nodesById, Transition, TransitionGroup, resolveTransitionProps,
       writeClass, appWindow, defineWindow, getCurrentWindow, appPermissions, appStorage,
       appDevtools, appShell, appLaunch, vModelText, vShow, desktopApi, activation, noteActivation,
       render, useCssVars, useCssModule, VueElement, defineCustomElement, createSSRApp, hydrate,
       useHost, useShadowRoot, initDirectivesForSSR };`,
  )(...args);
  worldExports = exports;

  return {
    ...exports,
    host,
    /** The private global runtime.js installed its DOM shims on. */
    global: fakeGlobal,
    /** A stand-in host element: enough surface for patchProp. */
    el: (id = 1) => ({ id, value: "", attrs: Object.create(null), className: "" }),
    /** Advisories only (warnCompat prefixes every one of them). */
    advisories: () => host.logs.filter((l) => l.startsWith("[vue-native compat]")),
    /** Listener flag writes for one event code. */
    flagsFor: (code) => host.listeners.filter((l) => l.code === code),
  };
}
