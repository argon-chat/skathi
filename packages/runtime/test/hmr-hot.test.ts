// `import.meta.hot` (the dev builder makes it `module.hot`): a module that accepts itself updates in place and is a
// boundary for changes below it; the `restart` change kind and `./x.js` naming x.ts.

import { describe, expect, test } from "bun:test";
import { applyChanges, createLoader, definePlain, type LoaderWorld, type Require } from "./loader-harness";

const ENTRY = "app/src/main.ts";
const VIEW = "app/src/view.ts";
const STORE = "app/src/store.ts";
const UTIL = "app/src/util.ts";

interface ViewExports {
  version: number;
  store: { version: number };
  accepted: { version: number }[];
  data: Record<string, unknown>;
}

/** A component module as Svelte compiles it: it accepts itself and swaps its implementation in its callback. */
function defineView(w: LoaderWorld, version: number, opts: { invalidate?: boolean } = {}) {
  const requires: Require[] = [{ spec: "./store", id: STORE }];
  w.SK.setDeps(
    VIEW,
    requires.map((r) => r.id),
  );
  w.SK.define(
    VIEW,
    function (require, module, exports) {
      w.exec.set(VIEW, (w.exec.get(VIEW) ?? 0) + 1);
      const store = require("./store") as { version: number };
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
  definePlain(w, ENTRY, [{ spec: "./view", id: VIEW }], 1);
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
    definePlain(w, ENTRY, [{ spec: "./view", id: VIEW }], 1);
    w.SK.boot(ENTRY);
    defineView(w, 2);
    applyChanges(w, [{ id: VIEW, kind: "reload" }]);
    expect(w.hardRestarted()).toBe(true);
  });
});

describe("change kinds and ids", () => {
  test("restart re-runs the app from the entry", () => {
    const w = boot();
    applyChanges(w, [{ id: "some-package/sub", kind: "restart" }]);
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
  });

  test("an import of ./x.js resolves to x.ts", () => {
    const w = boot();
    expect(w.SK.resolve("./util.js", ENTRY)).toBe(UTIL);
  });
});
