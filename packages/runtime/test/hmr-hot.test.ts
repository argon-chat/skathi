// `import.meta.hot` (the dev builder makes it `module.hot`): a module that accepts itself updates in place and is a
// boundary for changes below it; the `restart` change kind and resolution of `.svelte` / `./x.js` ids.

import { describe, expect, test } from "bun:test";
import { applyChanges, createLoader, definePlain, type LoaderWorld, type Require } from "./loader-harness";

const ENTRY = "app/src/main.ts";
const VIEW = "app/src/View.svelte";
const STORE = "app/src/store.svelte.ts";
const UTIL = "app/src/util.ts";

interface ViewExports {
  version: number;
  store: { version: number };
  accepted: { version: number }[];
  data: Record<string, unknown>;
}

/** A component module as Svelte compiles it: it accepts itself and swaps its implementation in its callback. */
function defineView(w: LoaderWorld, version: number, opts: { invalidate?: boolean } = {}) {
  const requires: Require[] = [{ spec: "./store.svelte", id: STORE }];
  w.SK.setDeps(
    VIEW,
    requires.map((r) => r.id),
  );
  w.SK.define(
    VIEW,
    ["version", "store", "accepted", "data"],
    function (require, module, exports) {
      w.exec.set(VIEW, (w.exec.get(VIEW) ?? 0) + 1);
      const store = require("./store.svelte") as { version: number };
      const accepted: { version: number }[] = [];
      exports.version = version;
      exports.store = store;
      exports.accepted = accepted;
      exports.data = module.hot!.data;
      module.hot!.dispose((data) => {
        data.disposedAt = version;
      });
      module.hot!.accept((next) => {
        accepted.push(next as unknown as { version: number });
        if (opts.invalidate) module.hot!.invalidate();
      });
    },
    VIEW + "@" + version,
  );
}

async function boot(): Promise<LoaderWorld> {
  const w = createLoader();
  definePlain(w, UTIL, [], 1);
  definePlain(w, STORE, [{ spec: "./util", id: UTIL }], 1);
  defineView(w, 1);
  definePlain(w, ENTRY, [{ spec: "./View.svelte", id: VIEW }], 1);
  await w.SK.boot(ENTRY);
  return w;
}

const view = async (w: LoaderWorld) => (await w.SK.importModule(VIEW)) as unknown as ViewExports;

describe("a module that accepts itself", () => {
  test("re-runs alone, hands the new exports to the old callbacks, and keeps hot.data", async () => {
    const w = await boot();
    const old = await view(w);
    defineView(w, 2);
    await applyChanges(w, [{ id: VIEW, kind: "reload" }]);

    expect(w.runs(VIEW)).toBe(2);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.runs(STORE)).toBe(1);
    expect(old.accepted.map((m) => m.version)).toEqual([2]);
    expect((await view(w)).version).toBe(2);
    expect((await view(w)).data).toBe(old.data);
    expect((await view(w)).data.disposedAt).toBe(1);
    expect(w.hardRestarted()).toBe(false);
  });

  test("is the boundary for a change below it: the walk stops there instead of reaching the entry", async () => {
    const w = await boot();
    const old = await view(w);
    definePlain(w, UTIL, [], 2);
    await applyChanges(w, [{ id: UTIL, kind: "reload" }]);

    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(STORE)).toBe(2);
    expect(w.runs(VIEW)).toBe(2);
    expect(w.runs(ENTRY)).toBe(1);
    expect(old.accepted.map((m) => m.version)).toEqual([1]);
    expect((await view(w)).store.version).toBe(1);
    expect(w.hardRestarted()).toBe(false);
  });

  test("works without Vue's HMR runtime", async () => {
    const w = await boot();
    delete w.sandbox.__VUE_HMR_RUNTIME__;
    defineView(w, 2);
    await applyChanges(w, [{ id: VIEW, kind: "reload" }]);
    expect((await view(w)).version).toBe(2);
    expect(w.hardRestarted()).toBe(false);
  });

  test("an update the callback invalidates restarts the app", async () => {
    const w = createLoader();
    definePlain(w, UTIL, [], 1);
    definePlain(w, STORE, [{ spec: "./util", id: UTIL }], 1);
    defineView(w, 1, { invalidate: true });
    definePlain(w, ENTRY, [{ spec: "./View.svelte", id: VIEW }], 1);
    await w.SK.boot(ENTRY);
    defineView(w, 2);
    await applyChanges(w, [{ id: VIEW, kind: "reload" }]);
    expect(w.hardRestarted()).toBe(true);
  });
});

describe("a module that accepts a dependency or some of its exports", () => {
  const HOST = "app/src/host.ts";
  const OTHER = "app/src/other.ts";
  const LIB = "app/src/lib.ts";

  async function acceptingDeps(deps: string | string[]) {
    const w = createLoader();
    definePlain(w, UTIL, [], 1);
    definePlain(w, OTHER, [], 1);
    w.SK.setDeps(HOST, [UTIL, OTHER]);
    w.SK.define(HOST, ["util", "got"], function (require, module, exports) {
      w.exec.set(HOST, (w.exec.get(HOST) ?? 0) + 1);
      exports.util = require("./util");
      exports.got = [];
      module.hot!.accept(deps, (mods) => (exports.got as unknown[]).push(mods));
    });
    definePlain(w, ENTRY, [{ spec: "./host", id: HOST }], 1);
    await w.SK.boot(ENTRY);
    definePlain(w, UTIL, [], 2);
    await applyChanges(w, [{ id: UTIL, kind: "reload" }]);
    return w;
  }

  test("accept(dep, cb) re-runs the dependency and hands cb its exports; the importer stays", async () => {
    const w = await acceptingDeps("./util");
    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(HOST)).toBe(1);
    expect(w.runs(ENTRY)).toBe(1);
    const got = (await w.SK.importModule(HOST)).got as { version: number }[];
    expect(got.map((m) => m.version)).toEqual([2]);
    expect(w.hardRestarted()).toBe(false);
  });

  test("accept([deps], cb) hands cb a list with the updated one in its place", async () => {
    const w = await acceptingDeps(["./other", "./util"]);
    const got = (await w.SK.importModule(HOST)).got as [unknown, { version: number }][];
    expect(got.map(([other, util]) => [other, util.version])).toEqual([[undefined, 2]]);
    expect(w.hardRestarted()).toBe(false);
  });

  function exporting(answer: number) {
    return function (
      _require: unknown,
      module: { hot?: { acceptExports(names: string): void } },
      exports: Record<string, unknown>,
    ) {
      exports.default = { answer };
      exports.answer = answer;
      module.hot!.acceptExports("default");
    };
  }

  test("acceptExports updates in place while the other exports keep their values, else reaches the importers", async () => {
    const w = createLoader();
    w.SK.setDeps(LIB, []);
    w.SK.define(LIB, ["answer", "default"], exporting(1), "1");
    definePlain(w, ENTRY, [{ spec: "./lib", id: LIB }], 1);
    await w.SK.boot(ENTRY);
    w.SK.define(LIB, ["answer", "default"], exporting(1), "1b");
    await applyChanges(w, [{ id: LIB, kind: "reload" }]);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.hardRestarted()).toBe(false);
    w.SK.define(LIB, ["answer", "default"], exporting(2), "2");
    await applyChanges(w, [{ id: LIB, kind: "reload" }]);
    expect(w.hardRestarted()).toBe(true);
  });
});

describe("change kinds and ids", () => {
  test("restart re-runs the app from the entry", async () => {
    const w = await boot();
    await applyChanges(w, [{ id: "svelte/transition", kind: "restart" }]);
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
  });

  test("an import resolves to a rune module, a component and a .ts named as .js", async () => {
    const w = await boot();
    expect(w.SK.resolve("./store.svelte", ENTRY)).toBe(STORE);
    expect(w.SK.resolve("./store.svelte.js", ENTRY)).toBe(STORE);
    expect(w.SK.resolve("./View", ENTRY)).toBe(VIEW);
    expect(w.SK.resolve("./util.js", ENTRY)).toBe(UTIL);
  });
});

test("an accepted ES module waits for top-level await before handing over its new namespace", async () => {
  const w = createLoader();
  const started = Promise.withResolvers<void>();
  const resume = Promise.withResolvers<number>();
  let accepted: number | undefined;
  Object.assign(w.sandbox, {
    started: started.resolve,
    gate: resume.promise,
    acceptValue: (v: number) => {
      accepted = v;
    },
  });
  const id = "async.js";
  w.SK.defineModule(id, "export const v = 1; import.meta.hot.accept(ns => globalThis.acceptValue(ns.v));", [], "old");
  definePlain(w, ENTRY, [{ spec: id, id }], 1);
  await w.SK.boot(ENTRY);
  w.SK.defineModule(id, "globalThis.started(); export const v = await globalThis.gate;", [], "new");
  const applied = applyChanges(w, [{ id }]);
  await started.promise;
  expect(accepted).toBeUndefined();
  resume.resolve(42);
  await applied;
  expect(accepted).toBe(42);
  expect(w.runs(ENTRY)).toBe(1);
});
