// Runs src/loader.ts in an isolated fake "engine" so the HMR logic can be tested without gpui, JavaScriptCore or a
// real Vue.
//
// The engine evaluates loader.ts as a classic script touching four ambient names (`globalThis`, `console`,
// `__vn_set_stylesheet`, plus the runtime namespace it imports). It is transpiled (./ts-source.ts), the imports are
// stripped and the rest evaluated with those names bound as parameters, so each createLoader() gets a private world
// (own __VN__, registry, __VUE_HMR_RUNTIME__).

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { readTsSource } from "./ts-source";

const HERE = dirname(fileURLToPath(import.meta.url));
export const LOADER_PATH = join(HERE, "../src/loader.ts");

const NS_IMPORT = /^\s*import\s+\*\s+as\s+(\w+)\s+from\s+["']([^"']+)["'];?[^\S\n]*$/gm;
const RUNTIME_SPEC = "./runtime";

/** A fresh loader world. */
export function createLoader() {
  const raw = readTsSource(LOADER_PATH);
  const namespaces = []; // [localName, specifier]
  const src = raw.replace(NS_IMPORT, (_m, local, spec) => {
    namespaces.push([local, spec]);
    return "";
  });
  if (!namespaces.some(([local, spec]) => local === "runtimeNs" && spec === RUNTIME_SPEC)) {
    throw new Error("loader.ts no longer imports ./runtime the expected way — update this harness");
  }

  const logs = [];
  const stylesheets = [];
  const sandbox = {}; // stands in for globalThis
  const runtimeNs = {
    version: "test",
    createApp: () => ({ _context: {}, mount: () => ({ __vm: true }) }),
  };
  const consoleStub = {
    log: (m) => logs.push(String(m)),
    warn: (m) => logs.push(String(m)),
    error: (m) => logs.push(String(m)),
  };

  const params = ["globalThis", "__vn_set_stylesheet", "console", ...namespaces.map(([n]) => n)];
  const args = [
    sandbox,
    (css) => stylesheets.push(css),
    consoleStub,
    ...namespaces.map(([n]) => (n === "runtimeNs" ? runtimeNs : {})),
  ];
  // eslint-disable-next-line no-new-func
  new Function(...params, src)(...args);

  const world = {
    VN: sandbox.__VN__,
    sandbox,
    logs,
    stylesheets,
    exec: new Map(), // module id -> times its factory has run
    hmr: null,
    /** How many times `id`'s factory has been executed. */
    runs: (id) => world.exec.get(id) || 0,
    /** Did anything log a hard restart? */
    hardRestarted: () => logs.some((l) => l.indexOf("hard restart") >= 0),
    logsMatching: (needle) => logs.filter((l) => l.indexOf(needle) >= 0),
  };
  world.hmr = installVueHmr(world);
  return world;
}

/** Minimal stand-in for Vue's __VUE_HMR_RUNTIME__, recording every call. */
function installVueHmr(world) {
  const calls = { createRecord: [], rerender: [], reload: [] };
  const records = new Map();
  world.sandbox.__VUE_HMR_RUNTIME__ = {
    createRecord(id, def) {
      calls.createRecord.push(id);
      if (records.has(id)) return false; // idempotent, like Vue's
      records.set(id, def);
      return true;
    },
    rerender(id, render) {
      calls.rerender.push(id);
      const def = records.get(id);
      if (def) def.render = render;
    },
    reload(id, def) {
      calls.reload.push(id);
      records.set(id, def);
    },
  };
  calls.records = records;
  return calls;
}

function factoryPreamble(world, id, requires) {
  return (require) => {
    world.exec.set(id, (world.exec.get(id) || 0) + 1);
    const imported = {};
    for (const r of requires) imported[r.spec] = require(r.spec);
    return imported;
  };
}

/** A plain .js module: `exports.version` is what importers capture by value. */
export function definePlain(world, id, requires, version) {
  const run = factoryPreamble(world, id, requires);
  world.VN.setDeps(id, requires.map((r) => r.id));
  world.VN.define(
    id,
    function (require, module, exports) {
      const imported = run(require);
      exports.version = version;
      exports.imported = imported;
    },
    id + "@" + version,
  );
}

/** A .vue module: default-exports a component and registers an HMR record. */
export function defineComponent(world, id, requires, version, css) {
  const run = factoryPreamble(world, id, requires);
  if (css !== undefined) world.VN.setCss(id, css);
  world.VN.setDeps(id, requires.map((r) => r.id));
  world.VN.define(
    id,
    function (require, module, exports) {
      const imported = run(require);
      const comp = {
        __hmrId: id,
        __file: id,
        __version: version,
        imported,
        render: function () {
          return version;
        },
      };
      const hmr = world.sandbox.__VUE_HMR_RUNTIME__;
      if (hmr) hmr.createRecord(id, comp);
      exports.default = comp;
    },
    id + "@" + version,
  );
}

/** Re-eval of a module script with UNCHANGED js (a <style>-only edit). */
export function redefineCssOnly(world, id, requires, version, css) {
  defineComponent(world, id, requires, version, css); // same version => same bodyHash
}

export const ENTRY = "app/src/main.js";
export const APP = "app/src/App.vue";
export const COUNTER = "app/src/components/Counter.vue";
export const UTIL = "app/src/util.js";

export function bootStandardFixture(v = 1) {
  const world = createLoader();
  definePlain(world, UTIL, [], v);
  defineComponent(world, COUNTER, [{ spec: "../util.js", id: UTIL }], v, ".counter { color: red }");
  defineComponent(
    world,
    APP,
    [
      { spec: "./util.js", id: UTIL },
      { spec: "./components/Counter.vue", id: COUNTER },
    ],
    v,
    ".app { color: blue }",
  );
  definePlain(world, ENTRY, [{ spec: "./App.vue", id: APP }], v);
  world.VN.boot(ENTRY);
  return world;
}

/** What the host does on a change: re-eval the changed scripts, then hmrApply. */
export function applyChanges(world, changed) {
  world.VN.hmrApply({ rev: 2, vendorRev: 1, changed });
}
