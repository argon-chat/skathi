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

function boot(): LoaderWorld {
  const w = createLoader();
  definePlain(w, UTIL, [], 1);
  definePlain(w, STORE, [{ spec: "./util", id: UTIL }], 1);
  defineView(w, 1);
  definePlain(w, ENTRY, [{ spec: "./View.svelte", id: VIEW }], 1);
  w.SK.boot(ENTRY);
  return w;
}

const view = (w: LoaderWorld) => w.SK.require(VIEW) as unknown as ViewExports;

describe("a module that accepts itself", () => {
  test("re-runs alone, hands the new exports to the old callbacks, and keeps hot.data", () => {
    const w = boot();
    const old = view(w);
    defineView(w, 2);
    applyChanges(w, [{ id: VIEW, kind: "reload" }]);

    expect(w.runs(VIEW)).toBe(2);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.runs(STORE)).toBe(1);
    expect(old.accepted.map((m) => m.version)).toEqual([2]);
    expect(view(w).version).toBe(2);
    expect(view(w).data).toBe(old.data);
    expect(view(w).data.disposedAt).toBe(1);
    expect(w.hardRestarted()).toBe(false);
  });

  test("is the boundary for a change below it: the walk stops there instead of reaching the entry", () => {
    const w = boot();
    const old = view(w);
    definePlain(w, UTIL, [], 2);
    applyChanges(w, [{ id: UTIL, kind: "reload" }]);

    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(STORE)).toBe(2);
    expect(w.runs(VIEW)).toBe(2);
    expect(w.runs(ENTRY)).toBe(1);
    expect(old.accepted.map((m) => m.version)).toEqual([1]);
    expect(view(w).store.version).toBe(1);
    expect(w.hardRestarted()).toBe(false);
  });

  test("works without Vue's HMR runtime", () => {
    const w = boot();
    delete w.sandbox.__VUE_HMR_RUNTIME__;
    defineView(w, 2);
    applyChanges(w, [{ id: VIEW, kind: "reload" }]);
    expect(view(w).version).toBe(2);
    expect(w.hardRestarted()).toBe(false);
  });

  test("an update the callback invalidates restarts the app", () => {
    const w = createLoader();
    definePlain(w, UTIL, [], 1);
    definePlain(w, STORE, [{ spec: "./util", id: UTIL }], 1);
    defineView(w, 1, { invalidate: true });
    definePlain(w, ENTRY, [{ spec: "./View.svelte", id: VIEW }], 1);
    w.SK.boot(ENTRY);
    defineView(w, 2);
    applyChanges(w, [{ id: VIEW, kind: "reload" }]);
    expect(w.hardRestarted()).toBe(true);
  });
});

describe("a module that accepts a dependency or some of its exports", () => {
  const HOST = "app/src/host.ts";
  const OTHER = "app/src/other.ts";
  const LIB = "app/src/lib.ts";

  function acceptingDeps(deps: string | string[]) {
    const w = createLoader();
    definePlain(w, UTIL, [], 1);
    definePlain(w, OTHER, [], 1);
    w.SK.setDeps(HOST, [UTIL, OTHER]);
    w.SK.define(HOST, function (require, module, exports) {
      w.exec.set(HOST, (w.exec.get(HOST) ?? 0) + 1);
      exports.util = require("./util");
      exports.got = [];
      module.hot!.accept(deps, (mods) => (exports.got as unknown[]).push(mods));
    });
    definePlain(w, ENTRY, [{ spec: "./host", id: HOST }], 1);
    w.SK.boot(ENTRY);
    definePlain(w, UTIL, [], 2);
    applyChanges(w, [{ id: UTIL, kind: "reload" }]);
    return w;
  }

  test("accept(dep, cb) re-runs the dependency and hands cb its exports; the importer stays", () => {
    const w = acceptingDeps("./util");
    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(HOST)).toBe(1);
    expect(w.runs(ENTRY)).toBe(1);
    const got = w.SK.require(HOST).got as { version: number }[];
    expect(got.map((m) => m.version)).toEqual([2]);
    expect(w.hardRestarted()).toBe(false);
  });

  test("accept([deps], cb) hands cb a list with the updated one in its place", () => {
    const w = acceptingDeps(["./other", "./util"]);
    const got = w.SK.require(HOST).got as [unknown, { version: number }][];
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

  test("acceptExports updates in place while the other exports keep their values, else reaches the importers", () => {
    const w = createLoader();
    w.SK.setDeps(LIB, []);
    w.SK.define(LIB, exporting(1), "1");
    definePlain(w, ENTRY, [{ spec: "./lib", id: LIB }], 1);
    w.SK.boot(ENTRY);
    w.SK.define(LIB, exporting(1), "1b");
    applyChanges(w, [{ id: LIB, kind: "reload" }]);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.hardRestarted()).toBe(false);
    w.SK.define(LIB, exporting(2), "2");
    applyChanges(w, [{ id: LIB, kind: "reload" }]);
    expect(w.hardRestarted()).toBe(true);
  });
});

describe("change kinds and ids", () => {
  test("restart re-runs the app from the entry", () => {
    const w = boot();
    applyChanges(w, [{ id: "svelte/transition", kind: "restart" }]);
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
  });

  test("an import resolves to a rune module, a component and a .ts named as .js", () => {
    const w = boot();
    expect(w.SK.resolve("./store.svelte", ENTRY)).toBe(STORE);
    expect(w.SK.resolve("./store.svelte.js", ENTRY)).toBe(STORE);
    expect(w.SK.resolve("./View", ENTRY)).toBe(VIEW);
    expect(w.SK.resolve("./util.js", ENTRY)).toBe(UTIL);
  });
});
