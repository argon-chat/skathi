// vue-native module registry + HMR client.
//
// The JS engine (JSC via redjs) runs CLASSIC SCRIPTS ONLY — no ESM, no
// dynamic import(), no import.meta, no WebSocket. So the dev loop is:
//
//   tier 1  dist/vendor.js      IIFE: @vue/runtime-core + our renderer + THIS
//                               file. Installs globalThis.__VN__.
//   tier 2  dist/modules/*.js   one classic script per app source file:
//                                 __VN__.setCss(id, css)          (optional)
//                                 __VN__.define(id, factory)
//   transport  dist/manifest.json + dist/changes.json, polled by the Rust host,
//              which re-evals changed module scripts and then calls
//              __VN__.hmrApply(changes).
//
// `define` only registers a factory; nothing runs until `require` pulls it.
// Redefining a module invalidates ONLY its own cache entry — importers keep
// their stale reference on purpose, because Vue's HMR record swaps the
// component definition *in place* (`updateComponentDef`), so the stale
// reference stays live and correct.

import * as runtimeNs from "./runtime.js";

const factories = new Map(); // id -> function (require, module, exports)
const cache = new Map(); // id -> { id, exports }
const cssText = new Map(); // id -> css
const cssOrder = []; // stable aggregation order
let cssDirty = false;
let requireDepth = 0;
let entryId = null;
let mountCount = 0;

const log = (msg) => {
  try {
    console.log("[vn-hmr] " + msg);
  } catch (_) {}
};

// ---- id resolution --------------------------------------------------------
// ids are workspace-relative POSIX paths: "apps/hello/src/components/Foo.vue".
// Bare specifiers ("vue", "vue-native") are registry keys, used verbatim.

const CANDIDATE_EXTS = ["", ".js", ".ts", ".mjs", ".vue", "/index.js", "/index.ts", "/index.vue"];

function dirOf(id) {
  const i = id.lastIndexOf("/");
  return i < 0 ? "" : id.slice(0, i);
}

function normalize(path) {
  const out = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      out.pop();
      continue;
    }
    out.push(seg);
  }
  return out.join("/");
}

function resolveId(spec, fromId) {
  if (spec.charCodeAt(0) !== 46 /* . */) return spec; // bare
  const base = normalize((fromId ? dirOf(fromId) + "/" : "") + spec);
  for (const ext of CANDIDATE_EXTS) {
    if (factories.has(base + ext) || cache.has(base + ext)) return base + ext;
  }
  return base;
}

// ---- registry -------------------------------------------------------------

function define(id, factory) {
  factories.set(id, factory);
  cache.delete(id); // redefinition invalidates exactly one module
}

function makeRequire(fromId) {
  return (spec) => requireId(resolveId(spec, fromId), fromId);
}

function requireId(id, fromId) {
  const hit = cache.get(id);
  if (hit) return hit.exports;
  const factory = factories.get(id);
  if (!factory) {
    throw new Error(
      "[vn] module not found: " + id + (fromId ? " (required from " + fromId + ")" : ""),
    );
  }
  const mod = { id, exports: {} };
  cache.set(id, mod);
  requireDepth++;
  try {
    factory(makeRequire(id), mod, mod.exports);
  } catch (e) {
    cache.delete(id);
    throw e;
  } finally {
    requireDepth--;
    if (requireDepth === 0) flushCss();
  }
  return mod.exports;
}

// ---- per-module CSS -------------------------------------------------------
// setCss runs at module-SCRIPT eval time (outside the factory), so a
// style-only edit needs no module re-execution at all: the host evals the new
// script, hmrApply re-aggregates, done.

function setCss(id, css) {
  if (!cssText.has(id)) cssOrder.push(id);
  if (cssText.get(id) === css) return;
  cssText.set(id, css);
  cssDirty = true;
}

function flushCss(force) {
  if (!cssDirty && !force) return;
  cssDirty = false;
  const parts = [];
  for (const id of cssOrder) {
    const css = cssText.get(id);
    if (css) parts.push(css);
  }
  if (typeof __vn_set_stylesheet === "function") __vn_set_stylesheet(parts.join("\n"));
}

// ---- pre-seeded "vue" / "vue-native" --------------------------------------
// Compiled SFC render functions import their helpers from "vue"; app code
// imports createApp from "vue-native". Both are the bundle-scope runtime
// module, wrapped so we can (a) count mounts for HMR observability and
// (b) guarantee `appContext.reload` exists.
//
// Vue 3.5's own `app.mount()` installs a dev-only `context.reload` that clones
// the root vnode and re-renders it into the same container; combined with
// `hmrDirtyComponents` making `isSameVNodeType` return false, that unmounts and
// re-mounts the root component IN PLACE — no teardown, no new root node, no
// re-run of the entry module. So we only install our own as a fallback, and
// only after mount, when Vue left the slot empty (e.g. a non-dev build).

const vueExports = Object.assign({}, runtimeNs);
const origCreateApp = runtimeNs.createApp;

vueExports.createApp = function (...args) {
  const app = origCreateApp(...args);
  const mount = app.mount.bind(app);
  app.mount = function (...mountArgs) {
    const vm = mount(...mountArgs);
    mountCount++;
    try {
      if (!app._context.reload) app._context.reload = rootReload;
    } catch (e) {
      log("could not install appContext.reload: " + e);
    }
    log("app mounted (mount #" + mountCount + ")");
    return vm;
  };
  return app;
};

for (const alias of ["vue", "vue-native"]) {
  factories.set(alias, () => {});
  cache.set(alias, { id: alias, exports: vueExports });
}

// ---- boot / restart -------------------------------------------------------

function boot(entry) {
  entryId = entry;
  flushCss(true); // module scripts already registered their CSS
  requireId(entry, null);
  log("booted from " + entry + " (" + factories.size + " modules registered)");
}

function teardown() {
  const t = globalThis.__VN_TEARDOWN__;
  if (typeof t !== "function") return false;
  try {
    t();
  } catch (e) {
    log("teardown threw: " + e);
  }
  return true;
}

/// Fallback `appContext.reload` (only installed when Vue's own dev one is
/// absent): unmount + fresh mount from the already-invalidated entry module.
function rootReload() {
  log("root reload");
  teardown();
  cache.delete(entryId);
  requireId(entryId, null);
  flushCss(true);
}

/// Fallback for changes we cannot map onto Vue's HMR record (a plain .js
/// module, the entry itself, a module with no component export): drop the
/// whole require cache and re-run the app. State is lost — same as the old
/// hot-restart loop.
function hardRestart(reason) {
  log("hard restart (" + reason + ")");
  teardown();
  for (const id of Array.from(cache.keys())) {
    if (id !== "vue" && id !== "vue-native") cache.delete(id);
  }
  requireId(entryId, null);
  flushCss(true);
}

// ---- HMR ------------------------------------------------------------------
// The host has already re-evaled every changed module SCRIPT (so `factories`
// holds the new code and the cache entry for each changed id is gone, and
// setCss has run). All that is left is deciding what to tell Vue.

function componentOf(exports) {
  const c = exports && exports.default;
  return c && (typeof c === "object" || typeof c === "function") && c.__hmrId ? c : null;
}

function hmrApply(changes) {
  const list = (changes && changes.changed) || [];
  if (!list.length) {
    flushCss();
    return;
  }
  const hmr = globalThis.__VUE_HMR_RUNTIME__;
  if (!hmr) {
    hardRestart("__VUE_HMR_RUNTIME__ missing (is __DEV__ true?)");
    return;
  }
  for (const change of list) {
    const id = change && change.id;
    const kind = (change && change.kind) || "reload";
    if (!id) continue;
    try {
      if (kind === "css-only") {
        log("css-only " + id);
        continue; // setCss already ran; flushCss below
      }
      if (id === entryId) {
        hardRestart("entry module changed: " + id);
        return;
      }
      const exports = requireId(id, null); // re-executes: cache was invalidated
      const comp = componentOf(exports);
      if (!comp) {
        hardRestart("no HMR-able component export in " + id);
        return;
      }
      if (kind === "rerender") {
        log("rerender " + id);
        hmr.rerender(comp.__hmrId, comp.render);
      } else {
        log("reload " + id);
        hmr.reload(comp.__hmrId, comp);
      }
    } catch (e) {
      log("apply failed for " + id + ": " + (e && e.stack ? e.stack : e));
      hardRestart("exception while applying " + id);
      return;
    }
  }
  flushCss();
}

// ---- public surface -------------------------------------------------------

globalThis.__VN__ = {
  define,
  require: (id) => requireId(id, null),
  resolve: resolveId,
  setCss,
  flushCss,
  boot,
  hmrApply,
  rootReload,
  hardRestart,
  has: (id) => factories.has(id),
  ids: () => Array.from(factories.keys()),
  mountCount: () => mountCount,
};

log("loader ready (vendor.js evaluated)");
