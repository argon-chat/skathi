// Dev files register ES module sources and dependency edges. JavaScriptCore links and
// evaluates the graph, including cycles and top-level await. HMR assigns a new
// module key to each invalidated source and keeps unaffected module instances.
import * as nativeNs from "./native-modules";
import * as runtimeNs from "./runtime";
import * as stdNs from "./std/index";

declare const __SK_DEV_ENV__: Record<string, unknown>;
type ModuleId = string;

interface ModuleRecord {
  id: ModuleId;
  exports: Record<string, unknown>;
}

type HotData = Record<string, unknown>;
type HotAccept = (exports: Record<string, unknown>) => void;

/** One evaluation's `import.meta.hot`. */
interface HotState {
  data: HotData;
  /** null: the module does not accept itself. */
  accepts: (HotAccept | null)[] | null;
  /** `acceptExports(names)` alone: an update that changes another export also reaches the importers. */
  acceptedExports: Set<string> | null;
  /** `accept(deps, cb)`: dependencies whose updates stop here and are handed to `cb`. */
  depAccepts: { deps: ModuleId[]; single: boolean; cb: ((mods: unknown) => void) | null }[];
  disposers: ((data: HotData) => void)[];
  disposed: boolean;
  invalidated: boolean;
  record: ModuleRecord;
}

interface HotContext {
  readonly data: HotData;
  accept(deps?: unknown, cb?: unknown): void;
  acceptExports(names?: unknown, cb?: unknown): void;
  dispose(cb: (data: HotData) => void): void;
  prune(cb: (data: HotData) => void): void;
  invalidate(message?: string): void;
  decline(): void;
  on(event: string, cb: unknown): void;
  off(event: string, cb: unknown): void;
  send(event: string, data?: unknown): void;
}

interface Reference {
  start: number;
  end: number;
  spec: string;
  names: string[];
  dynamic: boolean;
}
type Namespace = Record<string, unknown>;
interface Definition {
  body: string;
  refs: Reference[];
  hash: string;
}
type VueDevContext = { reload?: () => void };

interface ComponentLike {
  __hmrId: string;
  render?: unknown;
}

interface HmrChange {
  id: ModuleId;
  /** `restart`: an npm module the app already loaded was rebuilt into a new package bundle. */
  kind?: "reload" | "rerender" | "css-only" | "restart";
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
  var __SK__: SkRegistry;
  var __skModuleGeneration: number | undefined;
  var __sk_provide_module: (key: string, source: string) => void;
  var __sk_forget_module: (key: string) => void;
  var __sk_import_module: (key: string) => Promise<Namespace>;
}

interface SkRegistry {
  defineModule(id: ModuleId, body: string, refs: Reference[], hash: string): void;
  begin(id: ModuleId, namespace: Namespace, meta: Record<string, unknown>): void;
  vendor(id: ModuleId): Namespace;
  importFrom(id: ModuleId, spec: string): Promise<Namespace>;
  setDeps(id: ModuleId, list: ModuleId[] | null | undefined): void;
  importModule(id: ModuleId): Promise<Namespace>;
  resolve(spec: string, fromId: ModuleId | null | undefined): ModuleId;
  setCss(id: ModuleId, css: string): void;
  flushCss(force?: boolean): void;
  boot(entry: ModuleId): Promise<void>;
  hmrApply(changes: HmrChanges | null | undefined): Promise<void>;
  rootReload(): Promise<void>;
  hardRestart(reason: string): Promise<void>;
  has(id: ModuleId): boolean;
  ids(): ModuleId[];
  loaded(id: ModuleId): boolean;
  depsOf(id: ModuleId): ModuleId[];
  importersOf(id: ModuleId): ModuleId[];
  mountCount(): number;
  dispose(): void;
}

const definitions = new Map<ModuleId, Definition>();
const keys = new Map<ModuleId, string>();
const cache = new Map<ModuleId, ModuleRecord>();
const nativeNames = new Map<ModuleId, Set<string>>();
const deps = new Map<ModuleId, ModuleId[]>(); // id -> [imported id, ...]        (forward edges)
const importers = new Map<ModuleId, Set<ModuleId>>(); // id -> Set<importing id>    (reverse edges)
const cssText = new Map<ModuleId, string>(); // id -> css
const cssOrder: ModuleId[] = []; // stable aggregation order
let cssDirty = false;
let execLog: Set<ModuleId> | null = null; // Set collecting ids executed during one hmrApply
let entryId: ModuleId | null = null;
let mountCount = 0;
let degraded = false; // last boot / restart failed — the app is not running

const log = (msg: string) => {
  try {
    console.log("[sk-hmr] " + msg);
  } catch (_) {}
};

// Ids are workspace-relative POSIX paths; bare specifiers ("vue", "@skathi-native/runtime") are registry keys.

const CANDIDATE_EXTS = [
  "",
  ".js",
  ".ts",
  ".mjs",
  ".vue",
  ".svelte",
  ".svelte.ts",
  ".svelte.js",
  "/index.js",
  "/index.ts",
  "/index.vue",
  "/index.svelte",
];

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
    if (definitions.has(base + ext) || cache.has(base + ext)) return base + ext;
  }
  // TypeScript's ESM spelling: `./x.js` names `x.ts`.
  if (/\.js$/.test(base)) {
    const ts = base.slice(0, -3) + ".ts";
    if (definitions.has(ts) || cache.has(ts)) return ts;
  }
  return base;
}

const hotStates = new Map<ModuleId, HotState>(); // id -> its latest evaluation's import.meta.hot state

function hotContext(id: ModuleId, record: ModuleRecord): HotContext {
  const prev = hotStates.get(id);
  const state: HotState = {
    data: prev ? prev.data : {},
    accepts: null,
    acceptedExports: null,
    depAccepts: [],
    disposers: [],
    disposed: false,
    invalidated: false,
    record,
  };
  hotStates.set(id, state);
  let acceptsAll = false;
  const callback = (cb: unknown) => (typeof cb === "function" ? cb : null);
  return {
    get data() {
      return state.data;
    },
    accept(deps?: unknown, cb?: unknown) {
      if (deps === undefined || typeof deps === "function") {
        acceptsAll = true;
        state.acceptedExports = null;
        (state.accepts ??= []).push(callback(deps) as HotAccept | null);
      } else if (typeof deps === "string" || Array.isArray(deps)) {
        const specs = (Array.isArray(deps) ? deps : [deps]).filter((d): d is string => typeof d === "string");
        state.depAccepts.push({
          deps: specs.map((spec) => resolveId(spec, id)),
          single: typeof deps === "string",
          cb: callback(cb) as ((mods: unknown) => void) | null,
        });
      }
    },
    acceptExports(names?: unknown, cb?: unknown) {
      (state.accepts ??= []).push(callback(cb) as HotAccept | null);
      if (acceptsAll) return;
      const list = (Array.isArray(names) ? names : [names]).filter((n): n is string => typeof n === "string");
      state.acceptedExports = new Set([...(state.acceptedExports ?? []), ...list]);
    },
    dispose(cb) {
      if (typeof cb === "function") state.disposers.push(cb);
    },
    prune() {},
    invalidate() {
      state.invalidated = true;
    },
    decline() {},
    on() {},
    off() {},
    send() {},
  };
}

/** Runs the dispose handlers of a module's current evaluation, once, before it is replaced. */
function disposeHot(id: ModuleId): void {
  const state = hotStates.get(id);
  if (!state || state.disposed) return;
  state.disposed = true;
  for (const dispose of state.disposers) dispose(state.data);
}

/**
 * Re-runs a module that accepts itself and hands its old accept callbacks the new exports. "importers": an export
 * outside its `acceptExports` list changed, so the importers' copies are stale.
 */
async function hotUpdate(id: ModuleId): Promise<"updated" | "invalidated" | "importers"> {
  const old = hotStates.get(id)!;
  const before = old.record.exports;
  disposeHot(id);
  invalidateModule(id);
  const exports = await importModule(id);
  for (const accept of old.accepts ?? []) accept?.(exports);
  log("hot update " + id);
  if (old.invalidated) return "invalidated";
  const accepted = old.acceptedExports;
  if (accepted && before) {
    for (const name of new Set([...Object.keys(before), ...Object.keys(exports)])) {
      if (!accepted.has(name) && before[name] !== exports[name]) return "importers";
    }
  }
  return "updated";
}

const selfAccepting = (id: ModuleId) => !!hotStates.get(id)?.accepts;
const acceptsDep = (id: ModuleId, dep: ModuleId) => !!hotStates.get(id)?.depAccepts.some((a) => a.deps.includes(dep));

/** Hands `dep`'s new exports to the `accept(deps, cb)` callbacks of `id` that name it. */
function depUpdate(id: ModuleId, dep: ModuleId, exports: Record<string, unknown>): void {
  for (const a of hotStates.get(id)?.depAccepts ?? []) {
    if (!a.deps.includes(dep)) continue;
    a.cb?.(a.single ? exports : a.deps.map((d) => (d === dep ? exports : undefined)));
  }
  log("hot update " + dep + " (accepted by " + id + ")");
}

function invalidateModule(id: ModuleId): void {
  const key = keys.get(id);
  if (key) globalThis.__sk_forget_module(key);
  keys.delete(id);
  cache.delete(id);
}

function defineModule(id: ModuleId, body: string, refs: Reference[], hash: string): void {
  if (definitions.get(id)?.hash === hash) return;
  for (const ref of refs) {
    if (!ref.spec.startsWith("native:")) continue;
    let names = nativeNames.get(ref.spec);
    if (!names) nativeNames.set(ref.spec, (names = new Set()));
    for (const name of ref.names)
      if (!names.has(name)) {
        names.add(name);
        invalidateModule(ref.spec);
      }
  }
  definitions.set(id, { body, refs, hash });
  invalidateModule(id);
}

function setDeps(id: ModuleId, list: ModuleId[] | null | undefined): void {
  const prev = deps.get(id);
  if (prev) {
    for (let i = 0; i < prev.length; i++) {
      const set = importers.get(prev[i]!);
      if (!set) continue;
      set.delete(id);
      if (!set.size) importers.delete(prev[i]!);
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
    let set = importers.get(next[i]!);
    if (!set) importers.set(next[i]!, (set = new Set()));
    set.add(id);
  }
}

function vendor(id: ModuleId): Namespace {
  if (id.startsWith("native:")) return nativeNs.nativeModule(id.slice(7));
  if (id.startsWith("skathi:")) {
    const exports = stdNs.stdModules[id.slice(7)];
    if (exports) return exports;
    throw new Error("[sk] unknown standard module " + id);
  }
  return vueExports;
}

function vendorSource(id: ModuleId): string {
  const exports = vendor(id);
  const names = new Set([...Object.keys(exports), ...(nativeNames.get(id) ?? [])]);
  if (id.startsWith("native:")) names.add("default");
  const lines = ["const ns = globalThis.__SK__.vendor(" + JSON.stringify(id) + ");"];
  for (const name of names) {
    const local = "e" + lines.length;
    const value = name === "default" && id.startsWith("native:") ? "ns" : "ns[" + JSON.stringify(name) + "]";
    lines.push("const " + local + " = " + value + "; export {" + local + " as " + JSON.stringify(name) + "};");
  }
  return lines.join("\n");
}

// Import the registry through a module binding so application declarations can shadow host globals.
let bridgeKey: string | undefined;
function moduleKey(id: string): string {
  const version = (globalThis.__skModuleGeneration = (globalThis.__skModuleGeneration ?? 0) + 1);
  return "sk://dev/" + version + "/" + id;
}
function bridge(): string {
  if (!bridgeKey) {
    bridgeKey = moduleKey("loader");
    globalThis.__sk_provide_module(bridgeKey, "export default globalThis.__SK__;");
  }
  return bridgeKey;
}

function prepare(id: ModuleId, added: ModuleId[]): string {
  const existing = keys.get(id);
  if (existing) return existing;
  const definition = definitions.get(id);
  if (!definition && !id.startsWith("native:") && !id.startsWith("skathi:") && !aliases.includes(id))
    throw new Error("[sk] module not found: " + id);
  const key = moduleKey(id);
  keys.set(id, key); // Reserve before traversing an import cycle.
  added.push(id);
  const body = definition?.body ?? vendorSource(id);
  let local = "__skBinding";
  while (body.includes(local)) local += "_";
  const self = local + "Self";
  const parts: string[] = [];
  let offset = 0;
  for (const ref of definition?.refs ?? []) {
    const replacement = ref.dynamic
      ? "((specifier) => " + local + ".importFrom(" + JSON.stringify(id) + ", specifier))"
      : JSON.stringify(prepare(resolveId(ref.spec, id), added));
    parts.push(body.slice(offset, ref.start), replacement);
    offset = ref.end;
  }
  // A self-import supplies the live namespace without demanding any binding before initialization.
  const header = [
    `import ${local} from ${JSON.stringify(bridge())};`,
    `import * as ${self} from ${JSON.stringify(key)};`,
    `${local}.begin(${JSON.stringify(id)}, ${self}, import.meta);`,
    "",
  ].join("\n");
  parts.push(body.slice(offset));
  globalThis.__sk_provide_module(key, header + parts.join(""));
  return key;
}

function begin(id: ModuleId, namespace: Namespace, meta: Record<string, unknown>): void {
  const record = { id, exports: namespace };
  cache.set(id, record);
  execLog?.add(id);
  meta.hot = hotContext(id, record);
  meta.url = "sk://module/" + id;
  meta.env = __SK_DEV_ENV__;
}

async function importModule(id: ModuleId): Promise<Namespace> {
  const added: ModuleId[] = [];
  let key: string;
  try {
    key = prepare(id, added);
  } catch (error) {
    // A prepared dependency in a cycle may already reference the failed root's key.
    for (const created of added) invalidateModule(created);
    throw error;
  }
  return globalThis.__sk_import_module(key);
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
  if (typeof __sk_set_stylesheet === "function") __sk_set_stylesheet(parts.join("\n"));
}

// Compiled SFCs import helpers from "vue" and apps import createApp from "@skathi-native/runtime"; both are the
// bundle-scope runtime, wrapped to count mounts and guarantee `appContext.reload`. Vue 3.5's own
// dev-only `reload` re-mounts the root in place; ours is a fallback, installed after mount when
// Vue left the slot empty (non-dev build).

// `Record<string, unknown>`: `createApp` is replaced below by a wrapper of a wider shape that a namespace type would refuse.
const vueExports: Record<string, unknown> = Object.assign({}, runtimeNs);
// Absent from an app without Vue, whose vendor bundle has dom.ts in place of runtime.ts.
const origCreateApp = vueExports.createApp as typeof runtimeNs.createApp | undefined;

if (origCreateApp)
  vueExports.createApp = function (...args: Parameters<typeof runtimeNs.createApp>) {
    const app = origCreateApp(...args);
    const mount = app.mount.bind(app);
    app.mount = function (...mountArgs: Parameters<typeof mount>) {
      const vm = mount(...mountArgs);
      mountCount++;
      try {
        if (!(app._context as VueDevContext).reload) (app._context as VueDevContext).reload = rootReload;
      } catch (e) {
        log("could not install appContext.reload: " + e);
      }
      log("app mounted (mount #" + mountCount + ")");
      return vm;
    };
    return app;
  };
const origMountDocument = runtimeNs.mountDocument;
vueExports.mountDocument = function (...args: Parameters<typeof origMountDocument>) {
  const root = origMountDocument(...args);
  mountCount++;
  log("app mounted (mount #" + mountCount + ")");
  return root;
};

const aliases = origCreateApp ? ["vue", "@skathi-native/runtime"] : ["@skathi-native/runtime"];

// Skipped by invalidation: live library instances (route state, active pinia) a hard restart must not duplicate.
// vue-router and pinia are npm modules the dev builder makes when the app imports them.
const VENDOR_IDS = new Set(["vue", "@skathi-native/runtime", "vue-router", "pinia"]);

async function boot(entry: ModuleId): Promise<void> {
  entryId = entry;
  degraded = true; // cleared once the entry actually ran
  flushCss(true); // module scripts already registered their CSS
  await importModule(entry);
  degraded = false;
  log("booted from " + entry + " (" + definitions.size + " modules registered)");
}

function teardown() {
  const t = globalThis.__SK_TEARDOWN__;
  if (typeof t !== "function") return false;
  try {
    t();
  } catch (e) {
    log("teardown threw: " + e);
  }
  return true;
}

async function rootReload(): Promise<void> {
  log("root reload");
  teardown();
  invalidateModule(entryId!);
  await importModule(entryId!);
  flushCss(true);
}

/// Fallback for changes Vue's HMR record cannot take (the entry, importers reaching the entry, a
/// module with no component export): drop the module cache and re-run the app, losing state.
async function hardRestart(reason: string): Promise<void> {
  log("hard restart (" + reason + ")");
  teardown();
  for (const id of Array.from(keys.keys())) {
    if (!VENDOR_IDS.has(id) && !id.startsWith("native:") && !id.startsWith("skathi:")) invalidateModule(id);
  }
  degraded = true; // cleared once the entry actually ran again
  await importModule(entryId!);
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

/// A changed non-component module: reload the components that transitively import it (so their captured
/// copies of its exports refresh), stopping at each .vue module and giving up at the entry. Every module on the walked
/// paths is invalidated before any re-require so each re-executes once. Returns false when it bailed out to hardRestart.
async function reloadDependents(
  id: ModuleId,
  hmr: SkVueHmrRuntime | undefined,
  batch: HmrBatch,
  updated = false,
): Promise<boolean> {
  const visited = new Set<ModuleId>([id]);
  // `updated`: `id` already re-ran (a hot update whose other exports changed); only its importers are stale.
  const invalidate: ModuleId[] = updated ? [] : [id];
  const targets: ModuleId[] = []; // .vue modules to re-require + hand to Vue
  const boundaries: ModuleId[] = []; // modules that accept themselves (import.meta.hot.accept)
  const accepted: [ModuleId, ModuleId][] = []; // [importer, dep] where the importer accepts the dep
  const queue: ModuleId[] = [id];
  while (queue.length) {
    const cur = queue.shift()!;
    const set = importers.get(cur);
    if (!set) continue;
    for (const imp of set) {
      if (cache.has(imp) && acceptsDep(imp, cur)) {
        accepted.push([imp, cur]);
        continue;
      }
      if (visited.has(imp)) continue; // also the cycle guard
      visited.add(imp);
      if (imp === entryId) {
        // Re-running the entry means a fresh createApp(): state is gone anyway.
        await hardRestart("change in " + id + " reached the entry module");
        return false;
      }
      if (batch.fresh.has(imp)) continue;
      // Not live: never required (picks up the new version later), or invalidated by its own define() this batch, whose change entry covers it.
      if (!cache.has(imp)) continue;
      invalidate.push(imp);
      if (isComponentId(imp)) targets.push(imp);
      else if (selfAccepting(imp)) boundaries.push(imp);
      else queue.push(imp);
    }
  }

  // An importer that re-runs anyway (another path reached it) needs no callback.
  const handed = accepted.filter(([imp]) => !invalidate.includes(imp));
  if (!targets.length && !boundaries.length && !handed.length) {
    if (cache.has(id) && !batch.fresh.has(id)) {
      invalidateModule(id);
      await importModule(id); // keep the cache truthful; nobody to notify
      log("reload " + id + " (no live component importers)");
    } else if (batch.fresh.has(id)) {
      log("skip " + id + " (already refreshed by another change in this batch)");
    } else {
      log("skip " + id + " (not required by the running app yet)");
    }
    return true;
  }

  if (targets.length && !hmr) {
    await hardRestart("__VUE_HMR_RUNTIME__ missing (is __DEV__ true?)");
    return false;
  }
  for (let i = 0; i < invalidate.length; i++) {
    disposeHot(invalidate[i]!);
    invalidateModule(invalidate[i]!);
  }
  for (let i = 0; i < targets.length; i++) {
    const target = targets[i]!;
    if (batch.reloaded.has(target)) continue;
    const comp = componentOf(await importModule(target)); // evaluates the updated dependencies
    if (!comp) {
      await hardRestart("no HMR-able component export in " + target);
      return false;
    }
    batch.reloaded.add(target);
    hmr!.reload(comp.__hmrId, comp);
  }
  for (const boundary of boundaries) {
    if (batch.reloaded.has(boundary)) continue;
    batch.reloaded.add(boundary);
    if (!(await selfUpdate(boundary, hmr, batch))) return false;
  }
  for (const [imp, dep] of handed) depUpdate(imp, dep, await importModule(dep));
  const reached = targets.concat(
    boundaries,
    handed.map(([imp]) => imp),
  );
  log("reload " + id + " -> " + reached.length + " module(s): " + reached.join(", "));
  return true;
}

/** A hot update of a module that accepts itself; false when it bailed out to hardRestart. */
async function selfUpdate(id: ModuleId, hmr: SkVueHmrRuntime | undefined, batch: HmrBatch): Promise<boolean> {
  const result = await hotUpdate(id);
  if (result === "invalidated") {
    await hardRestart("import.meta.hot.invalidate() in " + id);
    return false;
  }
  return result === "updated" || (await reloadDependents(id, hmr, batch, true));
}

async function hmrApply(changes: HmrChanges | null | undefined): Promise<void> {
  const list = (changes && changes.changed) || [];
  if (!list.length) {
    flushCss();
    return;
  }
  const hmr = globalThis.__VUE_HMR_RUNTIME__;
  if (degraded) {
    await hardRestart("app is not running; retrying after " + list.length + " change(s)");
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
    if (kind === "restart") {
      await hardRestart(id + " was rebuilt into a new package bundle");
      return;
    }
    if (id === entryId) {
      await hardRestart("entry module changed: " + id);
      return;
    }
    (isComponentId(id) ? components : plain).push({ id: id, kind: kind });
  }
  if (components.length && !hmr) {
    await hardRestart("__VUE_HMR_RUNTIME__ missing (is __DEV__ true?)");
    return;
  }

  const batch: HmrBatch = { fresh: new Set(), reloaded: new Set() };
  execLog = batch.fresh;
  let at: ModuleId | null = null;
  try {
    for (let i = 0; i < plain.length; i++) {
      at = plain[i]!.id;
      if (batch.fresh.has(at)) {
        log("skip " + at + " (already refreshed by another change in this batch)");
        continue;
      }
      if (selfAccepting(at)) {
        if (batch.reloaded.has(at)) continue;
        batch.reloaded.add(at);
        if (await selfUpdate(at, hmr, batch)) continue;
        return;
      }
      if (!(await reloadDependents(at, hmr, batch))) return;
    }
    for (let i = 0; i < components.length; i++) {
      const change = components[i]!;
      at = change.id;
      if (batch.reloaded.has(at)) {
        // A dependency change already evaluated this module and reloaded it; another pass would re-mount again.
        log("skip " + at + " (already reloaded via a dependency)");
        continue;
      }
      const comp = componentOf(await importModule(at)); // re-executes: define() invalidated it
      if (!comp) {
        await hardRestart("no HMR-able component export in " + at);
        return;
      }
      batch.reloaded.add(at);
      if (change.kind === "rerender") {
        log("rerender " + at);
        hmr!.rerender(comp.__hmrId, comp.render);
      } else {
        log("reload " + at);
        hmr!.reload(comp.__hmrId, comp);
      }
    }
  } catch (e) {
    log("apply failed for " + at + ": " + (e && (e as Error).stack ? (e as Error).stack : e));
    await hardRestart("exception while applying " + at);
    return;
  } finally {
    execLog = null; // every return above passes through here
  }
  flushCss();
}

let pending = Promise.resolve();
function enqueue(action: () => Promise<void>): Promise<void> {
  const result = pending.then(action);
  pending = result.catch(() => {});
  return result;
}

globalThis.__SK__?.dispose?.();
globalThis.__SK__ = {
  defineModule,
  begin,
  vendor,
  importFrom: (from, spec) => importModule(resolveId(String(spec), from)),
  setDeps,
  importModule,
  resolve: resolveId,
  setCss,
  flushCss,
  boot: (id) => enqueue(() => boot(id)),
  hmrApply: (changes) => enqueue(() => hmrApply(changes)),
  rootReload,
  hardRestart,
  has: (id: ModuleId) => definitions.has(id) || aliases.includes(id),
  ids: () => [...aliases, ...definitions.keys()],
  loaded: (id: ModuleId) => cache.has(id),
  depsOf: (id: ModuleId) => (deps.get(id) || []).slice(),
  importersOf: (id: ModuleId) => Array.from(importers.get(id) || []),
  mountCount: () => mountCount,
  dispose: () => {
    for (const id of keys.keys()) invalidateModule(id);
    if (bridgeKey) globalThis.__sk_forget_module(bridgeKey);
  },
};

log("loader ready (vendor.js evaluated)");
