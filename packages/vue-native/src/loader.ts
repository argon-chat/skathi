// vue-native module registry + HMR client. The JS engine (JSC via redjs) runs classic scripts only
// (no ESM, import(), import.meta or WebSocket), so:
//
//   tier 1  dist/vendor.js      IIFE: @vue/runtime-core + renderer + this file; installs globalThis.__VN__.
//   tier 2  dist/modules/*.js   one script per app source file: setCss(id, css),
//                               setDeps(id, [ids]), define(id, factory, bodyHash).
//   transport  dist/manifest.json + dist/changes.json, polled by the Rust host, which re-evals
//              changed scripts and then calls __VN__.hmrApply(changes).
//
// `define` only registers a factory; `require` runs it. Redefining a .vue module invalidates only
// its own cache entry: Vue's HMR record swaps the component def in place, so importers' stale
// references stay valid.
//
// A plain .js module has no record and importers captured its exports by value, so
// `reloadDependents` walks the importer graph (setDeps) up to the nearest .vue modules and lets
// Vue reload them. Reaching the entry, or no component, falls back to a state-losing hard restart.

import * as runtimeNs from "./runtime";
// Vendored libraries ride the vendor bundle (no node_modules resolution at runtime) and are
// pre-seeded into the registry below; their `import ... from "vue"` is aliased to runtime.js by
// the vendor build.
import * as vueRouterNs from "vue-router";
import * as piniaNs from "pinia";

type ModuleId = string;

interface ModuleRecord {
  id: ModuleId;
  exports: Record<string, unknown>;
}

type ModuleFactory = (
  require: (spec: string) => unknown,
  module: ModuleRecord,
  exports: Record<string, unknown>,
) => void;

type VueDevContext = { reload?: () => void };

interface ComponentLike {
  __hmrId: string;
  render?: unknown;
}

interface HmrChange {
  id: ModuleId;
  kind?: "reload" | "rerender" | "css-only";
}
interface HmrChanges {
  rev?: number;
  vendorRev?: number;
  changed?: HmrChange[];
}
interface HmrBatch {
  fresh: Set<ModuleId>;
  reloaded: Set<ModuleId>;
}

declare global {
  var __VN__: VnRegistry;
}

interface VnRegistry {
  define(id: ModuleId, factory: ModuleFactory, hash?: string): void;
  setDeps(id: ModuleId, list: ModuleId[] | null | undefined): void;
  require(id: ModuleId): Record<string, unknown>;
  resolve(spec: string, fromId: ModuleId | null | undefined): ModuleId;
  setCss(id: ModuleId, css: string): void;
  flushCss(force?: boolean): void;
  boot(entry: ModuleId): void;
  hmrApply(changes: HmrChanges | null | undefined): void;
  rootReload(): void;
  hardRestart(reason: string): void;
  has(id: ModuleId): boolean;
  ids(): ModuleId[];
  loaded(id: ModuleId): boolean;
  depsOf(id: ModuleId): ModuleId[];
  importersOf(id: ModuleId): ModuleId[];
  mountCount(): number;
}

const factories = new Map<ModuleId, ModuleFactory>(); // id -> function (require, module, exports)
const cache = new Map<ModuleId, ModuleRecord>(); // id -> { id, exports }
const bodyHash = new Map<ModuleId, string | undefined>(); // id -> hash of the factory body (3rd arg of define)
const deps = new Map<ModuleId, ModuleId[]>(); // id -> [imported id, ...]        (forward edges)
const importers = new Map<ModuleId, Set<ModuleId>>(); // id -> Set<importing id>    (reverse edges)
const cssText = new Map<ModuleId, string>(); // id -> css
const cssOrder: ModuleId[] = []; // stable aggregation order
let cssDirty = false;
let requireDepth = 0;
let execLog: Set<ModuleId> | null = null; // Set collecting ids executed during one hmrApply
let entryId: ModuleId | null = null;
let mountCount = 0;
let degraded = false; // last boot / restart failed — the app is not running

const log = (msg: string) => {
  try {
    console.log("[vn-hmr] " + msg);
  } catch (_) {}
};

// Ids are workspace-relative POSIX paths; bare specifiers ("vue", "vue-native") are registry keys.

const CANDIDATE_EXTS = ["", ".js", ".ts", ".mjs", ".vue", "/index.js", "/index.ts", "/index.vue"];

function dirOf(id: ModuleId): string {
  const i = id.lastIndexOf("/");
  return i < 0 ? "" : id.slice(0, i);
}

function normalize(path: string): string {
  const out: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      // App sources outside the SDK workspace keep unresolved parent segments to match the builder's ids.
      if (out.length && out[out.length - 1] !== "..") out.pop();
      else out.push("..");
      continue;
    }
    out.push(seg);
  }
  return out.join("/");
}

function resolveId(spec: string, fromId: ModuleId | null | undefined): ModuleId {
  if (spec.charCodeAt(0) !== 46 /* . */) return spec; // bare
  const base = normalize((fromId ? dirOf(fromId) + "/" : "") + spec);
  for (const ext of CANDIDATE_EXTS) {
    if (factories.has(base + ext) || cache.has(base + ext)) return base + ext;
  }
  return base;
}

function define(id: ModuleId, factory: ModuleFactory, hash?: string): void {
  const known = factories.has(id);
  const prevHash = bodyHash.get(id);
  factories.set(id, factory);
  bodyHash.set(id, hash);
  // Byte-identical factory body (a <style>-only edit re-evals the script): keep the live exports,
  // or the next require builds a second component def whose createRecord is a no-op.
  if (known && hash !== undefined && hash === prevHash) return;
  cache.delete(id); // redefinition invalidates exactly one module
}

function setDeps(id: ModuleId, list: ModuleId[] | null | undefined): void {
  const prev = deps.get(id);
  if (prev) {
    for (let i = 0; i < prev.length; i++) {
      const set = importers.get(prev[i]);
      if (!set) continue;
      set.delete(id);
      if (!set.size) importers.delete(prev[i]);
    }
  }
  const next: ModuleId[] = [];
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

function makeRequire(fromId: ModuleId) {
  return (spec: string) => requireId(resolveId(spec, fromId), fromId);
}

function requireId(id: ModuleId, fromId: ModuleId | null): Record<string, unknown> {
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

function setCss(id: ModuleId, css: string): void {
  if (!cssText.has(id)) cssOrder.push(id);
  if (cssText.get(id) === css) return;
  cssText.set(id, css);
  cssDirty = true;
}

function flushCss(force?: boolean): void {
  if (!cssDirty && !force) return;
  cssDirty = false;
  const parts: string[] = [];
  for (const id of cssOrder) {
    const css = cssText.get(id);
    if (css) parts.push(css);
  }
  if (typeof __vn_set_stylesheet === "function") __vn_set_stylesheet(parts.join("\n"));
}

// Compiled SFCs import helpers from "vue" and apps import createApp from "vue-native"; both are the
// bundle-scope runtime, wrapped to count mounts and guarantee `appContext.reload`. Vue 3.5's own
// dev-only `reload` re-mounts the root in place; ours is a fallback, installed after mount when
// Vue left the slot empty (non-dev build).

// `Record<string, unknown>`: `createApp` is replaced below by a wrapper of a wider shape that a namespace type would refuse.
const vueExports: Record<string, unknown> = Object.assign({}, runtimeNs);
const origCreateApp = runtimeNs.createApp;

vueExports.createApp = function (...args: Parameters<typeof origCreateApp>) {
  const app = origCreateApp(...args);
  const mount = app.mount.bind(app);
  app.mount = function (...mountArgs: Parameters<typeof mount>) {
    const vm = mount(...mountArgs);
    mountCount++;
    try {
      if (!(app._context as VueDevContext).reload)
        (app._context as VueDevContext).reload = rootReload;
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

// Vendored npm packages: one registry entry each. `VENDOR_IDS` is skipped by invalidation: these
// are live library instances (route state, active pinia), and a hard restart must not duplicate them.
const VENDOR_IDS = new Set(["vue", "vue-native"]);
for (const [name, ns] of [
  ["vue-router", vueRouterNs],
  ["pinia", piniaNs],
] as [ModuleId, Record<string, unknown>][]) {
  factories.set(name, () => {});
  cache.set(name, { id: name, exports: ns });
  VENDOR_IDS.add(name);
}

function boot(entry: ModuleId): void {
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

function rootReload(): void {
  log("root reload");
  teardown();
  cache.delete(entryId!);
  requireId(entryId!, null);
  flushCss(true);
}

/// Fallback for changes Vue's HMR record cannot take (the entry, importers reaching the entry, a
/// module with no component export): drop the require cache and re-run the app, losing state.
function hardRestart(reason: string): void {
  log("hard restart (" + reason + ")");
  teardown();
  for (const id of Array.from(cache.keys())) {
    if (!VENDOR_IDS.has(id)) cache.delete(id);
  }
  degraded = true; // cleared once the entry actually ran again
  requireId(entryId!, null);
  degraded = false;
  flushCss(true);
}

function componentOf(exports: Record<string, unknown> | undefined): ComponentLike | null {
  const c = exports && exports.default;
  return c && (typeof c === "object" || typeof c === "function") && (c as ComponentLike).__hmrId
    ? (c as ComponentLike)
    : null;
}

function isComponentId(id: ModuleId): boolean {
  return id.length > 4 && id.slice(-4) === ".vue";
}

/// A changed non-component module: reload the components that transitively import it (the only way their captured
/// copies of its exports refresh), stopping at each .vue module and giving up at the entry. Every module on the walked
/// paths is invalidated before any re-require so each re-executes once. Returns false when it bailed out to hardRestart.
function reloadDependents(id: ModuleId, hmr: VnVueHmrRuntime, batch: HmrBatch): boolean {
  const visited = new Set<ModuleId>([id]);
  const invalidate: ModuleId[] = [id];
  const targets: ModuleId[] = []; // .vue modules to re-require + hand to Vue
  const queue: ModuleId[] = [id];
  while (queue.length) {
    const cur = queue.shift()!;
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
      if (batch.fresh.has(imp)) continue;
      // Not live: never required (picks up the new factory later), or invalidated by its own define() this batch, whose change entry covers it.
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

function hmrApply(changes: HmrChanges | null | undefined): void {
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
    hardRestart("app is not running; retrying after " + list.length + " change(s)");
    return;
  }

  // Order matters: plain modules first, since their walk re-requires importing components, so a
  // component changed in the same batch does not run twice.
  const plain: HmrChange[] = [];
  const components: HmrChange[] = [];
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

  const batch: HmrBatch = { fresh: new Set(), reloaded: new Set() };
  execLog = batch.fresh;
  let at: ModuleId | null = null;
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
        // A dependency change already re-ran this factory and reloaded it; another pass would re-mount again.
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
    log("apply failed for " + at + ": " + (e && (e as Error).stack ? (e as Error).stack : e));
    hardRestart("exception while applying " + at);
    return;
  } finally {
    execLog = null; // every return above passes through here
  }
  flushCss();
}

globalThis.__VN__ = {
  define,
  setDeps,
  require: (id: ModuleId) => requireId(id, null),
  resolve: resolveId,
  setCss,
  flushCss,
  boot,
  hmrApply,
  rootReload,
  hardRestart,
  has: (id: ModuleId) => factories.has(id),
  ids: () => Array.from(factories.keys()),
  loaded: (id: ModuleId) => cache.has(id),
  depsOf: (id: ModuleId) => (deps.get(id) || []).slice(),
  importersOf: (id: ModuleId) => Array.from(importers.get(id) || []),
  mountCount: () => mountCount,
};

log("loader ready (vendor.js evaluated)");
