// Runs src/runtime.js in an isolated fake host so the renderer's prop/event
// plumbing can be tested without gpui, JavaScriptCore or a window.
//
// Same trick as loader-harness.js: runtime.js is authored as an ES module for
// the bundler's benefit, but its body only touches ambient names — the `__vn_*`
// host functions and `console`. The import/export lines are stripped and every
// ambient name is bound as a PARAMETER, so each createRuntime() call gets its
// own private world (own intern table, own handler registry, own `warned` set,
// which is what makes "advisory fires exactly once" testable at all).

import { readFileSync } from "node:fs";
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
  resolveTransitionHooks,
  setTransitionHooks,
  toRaw,
  useTransitionState,
  version,
} from "@vue/runtime-core";

const HERE = dirname(fileURLToPath(import.meta.url));
export const RUNTIME_PATH = join(HERE, "../src/runtime.js");

const IMPORT_LINE = /^\s*import\s+\{[^}]*\}\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;
const STAR_REEXPORT = /^\s*export\s+\*\s+from\s+["'][^"']+["'];?[^\S\n]*$/gm;

/** A fresh runtime world with a recording host. */
export function createRuntime() {
  const raw = readFileSync(RUNTIME_PATH, "utf8");
  if (!IMPORT_LINE.test(raw)) {
    throw new Error("runtime.js no longer imports @vue/runtime-core — update this harness");
  }
  IMPORT_LINE.lastIndex = 0;
  const body = raw
    .replace(IMPORT_LINE, "")
    .replace(STAR_REEXPORT, "")
    // `export function foo` / `export const foo` become plain declarations, so
    // the whole module body is valid inside a function.
    .replace(/^export\s+(function|const|let)\s/gm, "$1 ");

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
  const fakeGlobal = {
    document: {
      body: {},
      documentElement: {},
      head: {},
    },
    requestAnimationFrame: (cb) => setTimeout(cb, 0),
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
  ];
  let nextId = 1;
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
  ];

  // eslint-disable-next-line no-new-func
  const exports = new Function(
    ...params,
    `${body}\nreturn { patchProp, nodeOps, EV_CODES, DERIVED, canonicalEvent, hyphenate,
       createApp, handlers, nodesById, Transition, TransitionGroup, resolveTransitionProps,
       writeClass };`,
  )(...args);

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
