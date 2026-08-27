// vue-native module registry + HMR client.
//
// The JS engine (JSC via redjs) runs CLASSIC SCRIPTS ONLY — no ESM, no
// dynamic import(), no import.meta, no WebSocket. So the dev loop is:
//
//   tier 1  dist/vendor.js      IIFE: @vue/runtime-core + our renderer + THIS
//                               file. Installs globalThis.__VN__.
//   tier 2  dist/modules/*.js   one classic script per app source file:
//                                 __VN__.setCss(id, css)          (optional)
//                                 __VN__.setDeps(id, [ids])
//                                 __VN__.define(id, factory, bodyHash)
//   transport  dist/manifest.json + dist/changes.json, polled by the Rust host,
//              which re-evals changed module scripts and then calls
//              __VN__.hmrApply(changes).
//
// `define` only registers a factory; nothing runs until `require` pulls it.
// Redefining a .vue module invalidates ONLY its own cache entry — importers
// keep their stale reference on purpose, because Vue's HMR record swaps the
// component definition *in place* (`updateComponentDef`), so the stale
// reference stays live and correct.
//
// A plain .js module has no such record: importers captured its exports by
// value, so re-running it alone changes nothing. That is what the IMPORTER
// GRAPH (setDeps) is for — see `reloadDependents`: we walk importers up to the
// nearest .vue modules, invalidate that whole path, and let Vue reload those
// components, which re-runs their factory chain against the new exports. Only
// when the walk reaches the entry (or finds no component to hand to Vue) do we
// fall back to a state-losing hard restart.

import * as runtimeNs from "./runtime.js";
// Vendored libraries ride the vendor bundle so real library-based apps work
// unchanged: there is no node_modules resolution at runtime (classic scripts
// only), so anything an app may `import` from npm has to be baked in here and
// pre-seeded into the registry below. Their own `import ... from "vue"` is
// aliased to runtime.js by the vendor build (see build.ts).
//   vue-router  memory history — no browser location/history needed.
//   pinia       store library; `createPinia()` is an ordinary app plugin.
import * as vueRouterNs from "vue-router";
import * as piniaNs from "pinia";
//   livekit-client  NOT the npm package — the name. The real SDK needs
//                   WebSocket / RTCPeerConnection / MediaStream / getUserMedia,
//                   none of which redjs has; ours drives the native LiveKit
//                   engine in crates/vn-voice instead (docs/VOICE.md).
import * as livekitNs from "./livekit.js";

const factories = new Map(); // id -> function (require, module, exports)
const cache = new Map(); // id -> { id, exports }
const bodyHash = new Map(); // id -> hash of the factory body (3rd arg of define)
const deps = new Map(); // id -> [imported id, ...]        (forward edges)
const importers = new Map(); // id -> Set<importing id>    (reverse edges)
const cssText = new Map(); // id -> css
const cssOrder = []; // stable aggregation order
let cssDirty = false;
let requireDepth = 0;
let execLog = null; // Set collecting ids executed during one hmrApply
let entryId = null;
let mountCount = 0;
let degraded = false; // last boot / restart failed — the app is not running

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

function define(id, factory, hash) {
  const known = factories.has(id);
  const prevHash = bodyHash.get(id);
  factories.set(id, factory);
  bodyHash.set(id, hash);
  // Byte-identical factory body (a <style>-only edit re-evals the whole module
  // script, setCss included): keep the live exports. Dropping them would strand
  // the component def that Vue's HMR record owns — the next require would build
  // a *second* def object whose createRecord call is a no-op.
  // No hash (an older/other builder) -> assume it changed, as before.
  if (known && hash !== undefined && hash === prevHash) return;
  cache.delete(id); // redefinition invalidates exactly one module
}

/// Replaces `id`'s outgoing edges and keeps the reverse index in sync.
/// Emitted by the builder next to define(), so the graph tracks the code.
function setDeps(id, list) {
  const prev = deps.get(id);
  if (prev) {
    for (let i = 0; i < prev.length; i++) {
      const set = importers.get(prev[i]);
      if (!set) continue;
      set.delete(id);
      if (!set.size) importers.delete(prev[i]);
    }
  }
  const next = [];
  if (list) {
    for (let i = 0; i < list.length; i++) {
      const dep = list[i];
      if (typeof dep !== "string" || dep === id || next.indexOf(dep) >= 0) continue;
      next.push(dep);
    }
  }
  deps.set(id, next);
  for (let i = 0; i < next.length; i++) {
    let set = importers.get(next[i]);
    if (!set) importers.set(next[i], (set = new Set()));
    set.add(id);
  }
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
    if (execLog) execLog.add(id); // hmrApply watches what actually re-ran
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

// Vendored npm packages: one registry entry each, exports handed over
// verbatim. `VENDOR_IDS` is what the invalidation paths below skip — these
// module records are the *live library instances* (vue-router's route state,
// pinia's active-pinia pointer), so dropping them on a hard restart would hand
// the fresh app a second copy of every library.
const VENDOR_IDS = new Set(["vue", "vue-native"]);
// `livekit-client` is here for a stronger reason than the other two: a Room
// holds a live native engine thread and a published track, so a second module
// copy during a hot restart would strand the first one's actor and its
// publication. VENDOR_IDS is what keeps the restart paths from dropping it.
for (const [name, ns] of [
  ["vue-router", vueRouterNs],
  ["pinia", piniaNs],
  ["livekit-client", livekitNs],
]) {
  factories.set(name, () => {});
  cache.set(name, { id: name, exports: ns });
  VENDOR_IDS.add(name);
}

// ---- boot / restart -------------------------------------------------------

function boot(entry) {
  entryId = entry;
  degraded = true; // cleared once the entry actually ran
  flushCss(true); // module scripts already registered their CSS
  requireId(entry, null);
  degraded = false;
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

/// Fallback for changes we cannot map onto Vue's HMR record (the entry itself,
/// a change whose importers reach the entry, a module with no component
/// export): drop the whole require cache and re-run the app. State is lost —
/// same as the old hot-restart loop.
function hardRestart(reason) {
  log("hard restart (" + reason + ")");
  teardown();
  for (const id of Array.from(cache.keys())) {
    if (!VENDOR_IDS.has(id)) cache.delete(id);
  }
  degraded = true; // cleared once the entry actually ran again
  requireId(entryId, null);
  degraded = false;
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

/// Only .vue modules get a `createRecord` epilogue from the builder, so only
/// they can be swapped in place by Vue. Everything else needs the graph walk.
function isComponentId(id) {
  return id.length > 4 && id.slice(-4) === ".vue";
}

/// A changed non-component module: reload the components that (transitively)
/// import it, which is the only way their captured copies of its exports get
/// refreshed.
///
/// Walk the reverse graph from `id` over LIVE (already required) modules only —
/// a module nothing has required yet needs nothing done, it will pick up the
/// new factory the first time it is pulled. The walk stops at each .vue module
/// (Vue's record takes over from there) and gives up if it reaches the entry.
/// Every module on the walked paths is invalidated BEFORE any re-require, so
/// each one re-executes exactly once no matter how many paths reach it.
///
/// `batch` carries the per-hmrApply bookkeeping: `fresh` = ids already
/// re-executed by an earlier change in this batch, `reloaded` = components
/// already handed to Vue. Both exist to keep one module from running twice when
/// several changes in one batch reach it.
///
/// Returns false when it bailed out to hardRestart.
function reloadDependents(id, hmr, batch) {
  const visited = new Set([id]);
  const invalidate = [id];
  const targets = []; // .vue modules to re-require + hand to Vue
  const queue = [id];
  while (queue.length) {
    const cur = queue.shift();
    const set = importers.get(cur);
    if (!set) continue;
    for (const imp of set) {
      if (visited.has(imp)) continue; // also the cycle guard
      visited.add(imp);
      if (imp === entryId) {
        // Re-running the entry means a fresh createApp(): state is gone anyway.
        hardRestart("change in " + id + " reached the entry module");
        return false;
      }
      // Already re-executed by an earlier change in this batch, which means the
      // components above it were reloaded too — walking through it again would
      // only run everything a second time.
      if (batch.fresh.has(imp)) continue;
      // Not live: either never required (it will pick up the new factory when
      // it first is), or invalidated by its own define() earlier in this batch
      // — in which case its own change entry covers it and everything above it.
      if (!cache.has(imp)) continue;
      invalidate.push(imp);
      if (isComponentId(imp)) targets.push(imp);
      else queue.push(imp);
    }
  }

  if (!targets.length) {
    if (cache.has(id) && !batch.fresh.has(id)) {
      cache.delete(id);
      requireId(id, null); // keep the cache truthful; nobody to notify
      log("reload " + id + " (no live component importers)");
    } else if (batch.fresh.has(id)) {
      log("skip " + id + " (already refreshed by another change in this batch)");
    } else {
      log("skip " + id + " (not required by the running app yet)");
    }
    return true;
  }

  for (let i = 0; i < invalidate.length; i++) cache.delete(invalidate[i]);
  for (let i = 0; i < targets.length; i++) {
    const target = targets[i];
    // Already re-required by an earlier change in this batch: that run saw the
    // new factories of EVERY module in the batch (the host redefines them all
    // before calling us), so there is nothing left to pick up.
    if (batch.reloaded.has(target)) continue;
    const comp = componentOf(requireId(target, null)); // re-runs the factory chain
    if (!comp) {
      hardRestart("no HMR-able component export in " + target);
      return false;
    }
    batch.reloaded.add(target);
    hmr.reload(comp.__hmrId, comp);
  }
  log("reload " + id + " -> " + targets.length + " component(s): " + targets.join(", "));
  return true;
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
  if (degraded) {
    // Nothing is mounted (a previous boot or restart threw), so there are no
    // instances for Vue to update — whatever just changed might be the fix.
    hardRestart("app is not running; retrying after " + list.length + " change(s)");
    return;
  }

  // Split, because order matters within one batch: a plain module's walk
  // re-requires the components that import it, so doing those FIRST keeps a
  // component that changed in the same batch from executing twice.
  const plain = [];
  const components = [];
  for (const change of list) {
    const id = change && change.id;
    if (!id) continue;
    const kind = (change && change.kind) || "reload";
    if (kind === "css-only") {
      log("css-only " + id); // setCss already ran; flushCss below
      continue;
    }
    if (id === entryId) {
      hardRestart("entry module changed: " + id);
      return;
    }
    (isComponentId(id) ? components : plain).push({ id: id, kind: kind });
  }

  // `fresh` is filled by requireId itself, so it is exactly the set of modules
  // that re-executed during this batch — including ones pulled in sideways by
  // someone else's factory chain.
  const batch = { fresh: new Set(), reloaded: new Set() };
  execLog = batch.fresh;
  let at = null;
  try {
    for (let i = 0; i < plain.length; i++) {
      at = plain[i].id;
      if (batch.fresh.has(at)) {
        log("skip " + at + " (already refreshed by another change in this batch)");
        continue;
      }
      if (!reloadDependents(at, hmr, batch)) return;
    }
    for (let i = 0; i < components.length; i++) {
      const change = components[i];
      at = change.id;
      if (batch.reloaded.has(at)) {
        // A dependency change already re-ran this module's factory and reloaded
        // it with the new code — a second pass would only re-mount it again.
        log("skip " + at + " (already reloaded via a dependency)");
        continue;
      }
      const comp = componentOf(requireId(at, null)); // re-executes: define() invalidated it
      if (!comp) {
        hardRestart("no HMR-able component export in " + at);
        return;
      }
      batch.reloaded.add(at);
      if (change.kind === "rerender") {
        log("rerender " + at);
        hmr.rerender(comp.__hmrId, comp.render);
      } else {
        log("reload " + at);
        hmr.reload(comp.__hmrId, comp);
      }
    }
  } catch (e) {
    log("apply failed for " + at + ": " + (e && e.stack ? e.stack : e));
    hardRestart("exception while applying " + at);
    return;
  } finally {
    execLog = null; // every return above passes through here
  }
  flushCss();
}

// ---- public surface -------------------------------------------------------

globalThis.__VN__ = {
  define,
  setDeps,
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
  loaded: (id) => cache.has(id),
  depsOf: (id) => (deps.get(id) || []).slice(),
  importersOf: (id) => Array.from(importers.get(id) || []),
  mountCount: () => mountCount,
};

log("loader ready (vendor.js evaluated)");
