// Runs src/loader.ts in an isolated fake engine so the HMR logic can be tested without gpui, JavaScriptCore or Vue.
// loader.ts is transpiled (./ts-source.ts), its imports stripped, and the ambient names bound as parameters, so each
// createLoader() gets a private world (own __VN__, registry, __VUE_HMR_RUNTIME__).

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { readTsSource } from "./ts-source";

const HERE = dirname(fileURLToPath(import.meta.url));
export const LOADER_PATH = join(HERE, "../src/loader.ts");

const NS_IMPORT = /^\s*import\s+\*\s+as\s+(\w+)\s+from\s+["']([^"']+)["'];?[^\S\n]*$/gm;
const RUNTIME_SPEC = "./runtime";

export interface Require {
  spec: string;
  id: string;
}

type Exports = Record<string, unknown>;

export interface PlainExports {
  version: number;
  imported: Record<string, PlainExports>;
  [key: string]: unknown;
}

export interface HmrComponent {
  __hmrId: string;
  __file: string;
  __version: number;
  imported: Record<string, PlainExports>;
  render: () => unknown;
}

interface ModuleRecord {
  id: string;
  exports: Exports;
}

type Factory = (require: (spec: string) => Exports, module: ModuleRecord, exports: Exports) => void;

export interface HmrChange {
  id: string;
  kind?: "reload" | "rerender" | "css-only";
}

export interface LoaderRegistry {
  define(id: string, factory: Factory, hash?: string): void;
  setDeps(id: string, list: string[] | null | undefined): void;
  require(id: string): Exports;
  resolve(spec: string, fromId: string | null | undefined): string;
  setCss(id: string, css: string): void;
  flushCss(force?: boolean): void;
  boot(entry: string): void;
  hmrApply(changes: { rev?: number; vendorRev?: number; changed?: HmrChange[] } | null | undefined): void;
  rootReload(): void;
  hardRestart(reason: string): void;
  has(id: string): boolean;
  ids(): string[];
  loaded(id: string): boolean;
  depsOf(id: string): string[];
  importersOf(id: string): string[];
  mountCount(): number;
}

interface HmrRuntime {
  createRecord(id: string, def: HmrComponent): boolean;
  rerender(id: string, render: () => unknown): void;
  reload(id: string, def: HmrComponent): void;
}

interface Sandbox {
  __VN__?: LoaderRegistry;
  __VUE_HMR_RUNTIME__?: HmrRuntime;
}

export interface HmrCalls {
  createRecord: string[];
  rerender: string[];
  reload: string[];
  records: Map<string, HmrComponent>;
}

export interface LoaderWorld {
  VN: LoaderRegistry;
  sandbox: Sandbox;
  logs: string[];
  stylesheets: string[];
  exec: Map<string, number>;
  hmr: HmrCalls;
  runs: (id: string) => number;
  hardRestarted: () => boolean;
  logsMatching: (needle: string) => string[];
}

export function createLoader(): LoaderWorld {
  const raw = readTsSource(LOADER_PATH);
  const namespaces: [string, string][] = []; // [localName, specifier]
  const src = raw.replace(NS_IMPORT, (_m, local: string, spec: string) => {
    namespaces.push([local, spec]);
    return "";
  });
  if (!namespaces.some(([local, spec]) => local === "runtimeNs" && spec === RUNTIME_SPEC)) {
    throw new Error("loader.ts no longer imports ./runtime the expected way — update this harness");
  }

  const logs: string[] = [];
  const stylesheets: string[] = [];
  const sandbox: Sandbox = {}; // stands in for globalThis
  const runtimeNs = {
    version: "test",
    createApp: () => ({ _context: {}, mount: () => ({ __vm: true }) }),
  };
  const consoleStub = {
    log: (m: unknown) => logs.push(String(m)),
    warn: (m: unknown) => logs.push(String(m)),
    error: (m: unknown) => logs.push(String(m)),
  };

  const params = ["globalThis", "__vn_set_stylesheet", "console", ...namespaces.map(([n]) => n)];
  const args = [
    sandbox,
    (css: string) => stylesheets.push(css),
    consoleStub,
    ...namespaces.map(([n]) => (n === "runtimeNs" ? runtimeNs : {})),
  ];
  // eslint-disable-next-line no-new-func
  new Function(...params, src)(...args);

  const hmr = installVueHmr(sandbox);
  const world: LoaderWorld = {
    VN: sandbox.__VN__!,
    sandbox,
    logs,
    stylesheets,
    exec: new Map(), // module id -> times its factory has run
    hmr,
    runs: (id: string) => world.exec.get(id) || 0,
    hardRestarted: () => logs.some((l) => l.indexOf("hard restart") >= 0),
    logsMatching: (needle: string) => logs.filter((l) => l.indexOf(needle) >= 0),
  };
  return world;
}

function installVueHmr(sandbox: Sandbox): HmrCalls {
  const records = new Map<string, HmrComponent>();
  const calls: HmrCalls = { createRecord: [], rerender: [], reload: [], records };
  sandbox.__VUE_HMR_RUNTIME__ = {
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
  return calls;
}

function factoryPreamble(world: LoaderWorld, id: string, requires: Require[]) {
  return (require: (spec: string) => Exports) => {
    world.exec.set(id, (world.exec.get(id) || 0) + 1);
    const imported: Record<string, PlainExports> = {};
    for (const r of requires) imported[r.spec] = require(r.spec) as PlainExports;
    return imported;
  };
}

export function definePlain(world: LoaderWorld, id: string, requires: Require[], version: number): void {
  const run = factoryPreamble(world, id, requires);
  world.VN.setDeps(
    id,
    requires.map((r) => r.id),
  );
  world.VN.define(
    id,
    function (require, _module, exports) {
      const imported = run(require);
      exports.version = version;
      exports.imported = imported;
    },
    id + "@" + version,
  );
}

export function defineComponent(
  world: LoaderWorld,
  id: string,
  requires: Require[],
  version: number,
  css?: string,
): void {
  const run = factoryPreamble(world, id, requires);
  if (css !== undefined) world.VN.setCss(id, css);
  world.VN.setDeps(
    id,
    requires.map((r) => r.id),
  );
  world.VN.define(
    id,
    function (require, _module, exports) {
      const imported = run(require);
      const comp: HmrComponent = {
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

export function redefineCssOnly(
  world: LoaderWorld,
  id: string,
  requires: Require[],
  version: number,
  css: string,
): void {
  defineComponent(world, id, requires, version, css); // same version => same bodyHash
}

export const ENTRY = "app/src/main.js";
export const APP = "app/src/App.vue";
export const COUNTER = "app/src/components/Counter.vue";
export const UTIL = "app/src/util.js";

export function bootStandardFixture(v = 1): LoaderWorld {
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

export function applyChanges(world: LoaderWorld, changed: HmrChange[]): void {
  world.VN.hmrApply({ rev: 2, vendorRev: 1, changed });
}
