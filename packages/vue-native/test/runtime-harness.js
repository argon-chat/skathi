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
import { createRenderer, version } from "@vue/runtime-core";

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
  };
  const intern = (name) => {
    let i = host.names.indexOf(name);
    if (i < 0) i = host.names.push(name) - 1;
    return i;
  };
  const nameOf = (id) => host.names[id];

  const params = [
    "createRenderer",
    "version",
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
  ];
  let nextId = 1;
  const args = [
    createRenderer,
    version,
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
  ];

  // eslint-disable-next-line no-new-func
  const exports = new Function(
    ...params,
    `${body}\nreturn { patchProp, nodeOps, EV_CODES, canonicalEvent, hyphenate, createApp, handlers,
      appWindow, defineWindow, getCurrentWindow };`,
  )(...args);

  return {
    ...exports,
    host,
    /** A stand-in host element: enough surface for patchProp. */
    el: (id = 1) => ({ id, value: "" }),
    /** Advisories only (warnCompat prefixes every one of them). */
    advisories: () => host.logs.filter((l) => l.startsWith("[vue-native compat]")),
    /** Listener flag writes for one event code. */
    flagsFor: (code) => host.listeners.filter((l) => l.code === code),
  };
}
