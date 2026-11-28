// The importer graph: a changed plain .js module must reload the components that import it instead of restarting the app.

import { describe, expect, test } from "bun:test";
import {
  APP,
  COUNTER,
  ENTRY,
  UTIL,
  applyChanges,
  bootStandardFixture,
  createLoader,
  defineComponent,
  type HmrChange,
  definePlain,
  redefineCssOnly,
} from "./loader-harness";

const changed = (id: string, kind?: HmrChange["kind"]): HmrChange[] => [{ id, kind: kind || "reload" }];

describe("external app module ids", () => {
  test("retains parents outside the SDK while resolving sibling modules", async () => {
    const w = createLoader();
    const entry = "../../client/packages/editor/main.ts";
    const helper = "../../client/packages/editor/gpu/draw.ts";
    const shared = "../../client/packages/shared.ts";
    w.SK.define(helper, ["draw"], (_require, module) => {
      module.exports = { draw: 42 };
    });
    w.SK.define(shared, ["shared"], (_require, module) => {
      module.exports = { shared: 17 };
    });
    w.SK.setDeps(entry, [helper, shared]);
    w.SK.define(entry, ["value"], (require, module) => {
      module.exports = { value: (require("./gpu/draw").draw as number) + (require("../shared").shared as number) };
    });
    expect(w.SK.resolve("./gpu/draw", entry)).toBe(helper);
    expect(w.SK.resolve("../shared", entry)).toBe(shared);
    expect((await w.SK.importModule(entry)).value).toBe(59);
  });
});

describe("boot", () => {
  test("requires every reachable module exactly once", async () => {
    const w = await bootStandardFixture();
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(COUNTER)).toBe(1);
    expect(w.runs(UTIL)).toBe(1);
    expect(w.hmr.createRecord).toEqual([COUNTER, APP]);
    expect(w.hardRestarted()).toBe(false);
  });

  test("records both directions of the graph", async () => {
    const w = await bootStandardFixture();
    expect(w.SK.depsOf(APP)).toEqual([UTIL, COUNTER]);
    expect(w.SK.importersOf(UTIL).sort()).toEqual([APP, COUNTER].sort());
    expect(w.SK.importersOf(APP)).toEqual([ENTRY]);
    expect(w.SK.importersOf(ENTRY)).toEqual([]);
  });
});

describe("shared plain module changed", () => {
  test("reloads exactly the components that import it, entry untouched", async () => {
    const w = await bootStandardFixture();
    definePlain(w, UTIL, [], 2); // host re-evals the changed script
    await applyChanges(w, changed(UTIL));

    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(APP)).toBe(2);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.hmr.reload.slice().sort()).toEqual([APP, COUNTER].sort());
    expect(w.hmr.reload.length).toBe(2);
    expect(w.hmr.rerender).toEqual([]);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.hardRestarted()).toBe(false);
  });

  test("the reloaded components see the NEW exports", async () => {
    const w = await bootStandardFixture();
    const oldApp = w.hmr.records.get(APP)!;
    expect(oldApp.imported["./util.js"]!.version).toBe(1);
    definePlain(w, UTIL, [], 2);
    await applyChanges(w, changed(UTIL));

    const newApp = w.hmr.records.get(APP)!;
    expect(newApp).not.toBe(oldApp); // a fresh def object from a fresh factory run
    expect(newApp.__version).toBe(1); // App's own code did not change...
    expect(newApp.imported["./util.js"]!.version).toBe(2); // ...but its import did
    expect(w.hmr.records.get(COUNTER)!.imported["../util.js"]!.version).toBe(2);
    expect(newApp.imported["./util.js"]!).toBe(w.hmr.records.get(COUNTER)!.imported["../util.js"]!);
  });

  test("createRecord stays idempotent across the reload", async () => {
    const w = await bootStandardFixture();
    definePlain(w, UTIL, [], 2);
    await applyChanges(w, changed(UTIL));
    expect(w.hmr.createRecord).toEqual([COUNTER, APP, COUNTER, APP]);
  });

  test("a leaf module only one component imports reloads only that component", async () => {
    const w = await bootStandardFixture();
    const FMT = "app/src/format.js";
    definePlain(w, FMT, [], 1);
    defineComponent(
      w,
      COUNTER,
      [
        { spec: "../util.js", id: UTIL },
        { spec: "../format.js", id: FMT },
      ],
      2,
    );
    await applyChanges(w, changed(COUNTER));
    expect(w.hmr.reload).toEqual([COUNTER]);

    definePlain(w, FMT, [], 2);
    await applyChanges(w, changed(FMT));
    expect(w.hmr.reload).toEqual([COUNTER, COUNTER]);
    expect(w.runs(APP)).toBe(1);
  });
});

describe("walk boundaries", () => {
  test("stops at the nearest component (does not climb past it)", async () => {
    const w = createLoader();
    const HELPER = "app/src/helper.js";
    definePlain(w, HELPER, [], 1);
    defineComponent(w, COUNTER, [{ spec: "../helper.js", id: HELPER }], 1);
    defineComponent(w, APP, [{ spec: "./components/Counter.vue", id: COUNTER }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    await w.SK.boot(ENTRY);

    definePlain(w, HELPER, [], 2);
    await applyChanges(w, changed(HELPER));
    expect(w.hmr.reload).toEqual([COUNTER]);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(ENTRY)).toBe(1);
  });

  test("an intermediate plain module is walked THROUGH and re-run once", async () => {
    const w = createLoader();
    const BASE = "app/src/base.js";
    const MID = "app/src/mid.js";
    definePlain(w, BASE, [], 1);
    definePlain(w, MID, [{ spec: "./base.js", id: BASE }], 1);
    defineComponent(
      w,
      APP,
      [
        { spec: "./mid.js", id: MID },
        { spec: "./base.js", id: BASE },
      ],
      1,
    );
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    await w.SK.boot(ENTRY);

    definePlain(w, BASE, [], 2);
    await applyChanges(w, changed(BASE));
    expect(w.runs(BASE)).toBe(2); // once, despite two paths reaching it
    expect(w.runs(MID)).toBe(2);
    expect(w.runs(APP)).toBe(2);
    expect(w.hmr.reload).toEqual([APP]);
    expect(w.hmr.records.get(APP)!.imported["./mid.js"]!.imported["./base.js"]!.version).toBe(2);
  });

  test("a cycle between plain modules terminates", async () => {
    const w = createLoader();
    const A = "app/src/a.js";
    const B = "app/src/b.js";
    definePlain(w, A, [{ spec: "./b.js", id: B }], 1);
    definePlain(w, B, [{ spec: "./a.js", id: A }], 1);
    defineComponent(w, APP, [{ spec: "./a.js", id: A }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    await w.SK.boot(ENTRY);

    definePlain(w, A, [{ spec: "./b.js", id: B }], 2);
    await applyChanges(w, changed(A));
    expect(w.hmr.reload).toEqual([APP]);
    expect(w.runs(A)).toBe(2);
    expect(w.runs(B)).toBe(2);
    expect(w.hardRestarted()).toBe(false);
  });

  test("a self-import edge is dropped rather than looping", async () => {
    const w = createLoader();
    const SELF = "app/src/self.js";
    w.SK.setDeps(SELF, [SELF, "app/src/other.js"]);
    expect(w.SK.depsOf(SELF)).toEqual(["app/src/other.js"]);
    expect(w.SK.importersOf(SELF)).toEqual([]);
  });
});

describe("fallbacks", () => {
  test("reaching the entry hard-restarts", async () => {
    const w = createLoader();
    const CONFIG = "app/src/config.js";
    definePlain(w, CONFIG, [], 1);
    defineComponent(w, APP, [], 1);
    definePlain(
      w,
      ENTRY,
      [
        { spec: "./App.vue", id: APP },
        { spec: "./config.js", id: CONFIG },
      ],
      1,
    );
    await w.SK.boot(ENTRY);

    definePlain(w, CONFIG, [], 2);
    await applyChanges(w, changed(CONFIG));
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
    expect(w.runs(CONFIG)).toBe(2);
    expect(w.hmr.reload).toEqual([]);
  });

  test("the entry module itself still hard-restarts", async () => {
    const w = await bootStandardFixture();
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 2);
    await applyChanges(w, changed(ENTRY));
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
  });

  test("a module the running app never required is a no-op", async () => {
    const w = await bootStandardFixture();
    const ORPHAN = "app/src/orphan.js";
    definePlain(w, ORPHAN, [], 1);
    await applyChanges(w, changed(ORPHAN));
    expect(w.runs(ORPHAN)).toBe(0); // not executed: nothing needs it yet
    expect(w.hmr.reload).toEqual([]);
    expect(w.hardRestarted()).toBe(false);
  });

  test("a component with no HMR-able export hard-restarts", async () => {
    const w = await bootStandardFixture();
    w.SK.setDeps(APP, [UTIL, COUNTER]);
    w.SK.define(
      APP,
      ["default"],
      function (require, _module, exports) {
        require("./util.js");
        exports.default = { notAComponent: true };
      },
      APP + "@broken",
    );
    await applyChanges(w, changed(APP));
    expect(w.hardRestarted()).toBe(true);
  });

  test("a factory that throws hard-restarts instead of leaving a half-state", async () => {
    const w = await bootStandardFixture();
    w.SK.setDeps(UTIL, []);
    w.SK.define(
      UTIL,
      [],
      function () {
        throw new Error("boom");
      },
      UTIL + "@throws",
    );
    let threw = null;
    try {
      await applyChanges(w, changed(UTIL));
    } catch (e) {
      threw = e; // hardRestart re-runs the entry, which hits the same throw
    }
    expect(threw && String(threw)).toContain("boom");
    expect(w.hardRestarted()).toBe(true);
  });

  test("recovers by hard restart when a previous boot left the app down", async () => {
    const w = createLoader();
    const MISSING = "app/src/missing.js";
    defineComponent(w, APP, [{ spec: "./missing.js", id: MISSING }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    await expect(w.SK.boot(ENTRY)).rejects.toThrow(); // missing.js is not defined yet
    expect(w.runs(APP)).toBe(0); // ESM links every dependency before executing this module.

    definePlain(w, MISSING, [], 1); // the file finally shows up
    await applyChanges(w, changed(MISSING));
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(MISSING)).toBe(1);
  });
});

describe("component changes keep their existing behaviour", () => {
  test("rerender swaps only the render fn", async () => {
    const w = await bootStandardFixture();
    defineComponent(w, COUNTER, [{ spec: "../util.js", id: UTIL }], 2);
    await applyChanges(w, changed(COUNTER, "rerender"));
    expect(w.hmr.rerender).toEqual([COUNTER]);
    expect(w.hmr.reload).toEqual([]);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.runs(APP)).toBe(1); // importers keep their reference on purpose
    expect(w.runs(UTIL)).toBe(1);
  });

  test("reload of a .vue does not disturb its importers", async () => {
    const w = await bootStandardFixture();
    defineComponent(w, COUNTER, [{ spec: "../util.js", id: UTIL }], 2);
    await applyChanges(w, changed(COUNTER));
    expect(w.hmr.reload).toEqual([COUNTER]);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(ENTRY)).toBe(1);
  });
});

describe("batched changes", () => {
  test("a component changed alongside its dependency runs once, reloads once", async () => {
    const w = await bootStandardFixture();
    definePlain(w, UTIL, [], 2);
    defineComponent(
      w,
      APP,
      [
        { spec: "./util.js", id: UTIL },
        { spec: "./components/Counter.vue", id: COUNTER },
      ],
      2,
    );
    await applyChanges(w, [
      { id: APP, kind: "reload" }, // listed first on purpose
      { id: UTIL, kind: "reload" },
    ]);
    expect(w.runs(APP)).toBe(2); // not 3: the graph walk and its own change agree
    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.hmr.reload.slice().sort()).toEqual([APP, COUNTER].sort());
    expect(w.hmr.reload.length).toBe(2);
    expect(w.hmr.records.get(APP)!.__version).toBe(2);
    expect(w.hmr.records.get(APP)!.imported["./util.js"]!.version).toBe(2);
  });

  test("a chain changed end to end runs each module once", async () => {
    const w = createLoader();
    const BASE = "app/src/base.js";
    const MID = "app/src/mid.js";
    definePlain(w, BASE, [], 1);
    definePlain(w, MID, [{ spec: "./base.js", id: BASE }], 1);
    defineComponent(w, APP, [{ spec: "./mid.js", id: MID }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    await w.SK.boot(ENTRY);

    // MID listed BEFORE BASE — the walk from MID already refreshes BASE, so
    // BASE's own entry must not start a second round.
    definePlain(w, MID, [{ spec: "./base.js", id: BASE }], 2);
    definePlain(w, BASE, [], 2);
    await applyChanges(w, [
      { id: MID, kind: "reload" },
      { id: BASE, kind: "reload" },
    ]);
    expect(w.runs(APP)).toBe(2);
    expect(w.runs(MID)).toBe(2);
    expect(w.runs(BASE)).toBe(2);
    expect(w.hmr.reload).toEqual([APP]);
  });

  test("two independent plain modules both apply", async () => {
    const w = createLoader();
    const A = "app/src/a.js";
    const B = "app/src/b.js";
    definePlain(w, A, [], 1);
    definePlain(w, B, [], 1);
    defineComponent(w, APP, [{ spec: "./a.js", id: A }], 1);
    defineComponent(w, COUNTER, [{ spec: "../b.js", id: B }], 1);
    definePlain(
      w,
      ENTRY,
      [
        { spec: "./App.vue", id: APP },
        { spec: "./components/Counter.vue", id: COUNTER },
      ],
      1,
    );
    await w.SK.boot(ENTRY);

    definePlain(w, A, [], 2);
    definePlain(w, B, [], 2);
    await applyChanges(w, [
      { id: A, kind: "reload" },
      { id: B, kind: "reload" },
    ]);
    expect(w.hmr.reload).toEqual([APP, COUNTER]);
    expect(w.runs(ENTRY)).toBe(1);
  });
});

describe("style-only edits", () => {
  test("do not invalidate the require cache", async () => {
    const w = await bootStandardFixture();
    const before = (await w.SK.importModule(APP)).default;
    redefineCssOnly(
      w,
      APP,
      [
        { spec: "./util.js", id: UTIL },
        { spec: "./components/Counter.vue", id: COUNTER },
      ],
      1,
      ".app { color: green }",
    );
    await applyChanges(w, changed(APP, "css-only"));

    expect(w.runs(APP)).toBe(1); // never re-executed
    expect((await w.SK.importModule(APP)).default).toBe(before); // same live component object
    expect(w.hmr.reload).toEqual([]);
    expect(w.hmr.rerender).toEqual([]);
    expect(w.stylesheets[w.stylesheets.length - 1]).toContain("color: green");
    expect(w.hardRestarted()).toBe(false);
  });

  test("a define() with no hash still invalidates (older builders)", async () => {
    const w = await bootStandardFixture();
    const before = (await w.SK.importModule(APP)).default;
    w.SK.setDeps(APP, [UTIL, COUNTER]);
    w.SK.define(APP, ["default"], function (require, _module, exports) {
      require("./util.js");
      require("./components/Counter.vue");
      exports.default = { __hmrId: APP, render: function () {} };
    });
    expect(w.SK.loaded(APP)).toBe(false);
    expect((await w.SK.importModule(APP)).default).not.toBe(before);
  });
});

describe("graph maintenance", () => {
  test("removing an import drops the reverse edge", async () => {
    const w = await bootStandardFixture();
    expect(w.SK.importersOf(UTIL).sort()).toEqual([APP, COUNTER].sort());
    defineComponent(w, COUNTER, [], 2); // Counter no longer imports util
    await applyChanges(w, changed(COUNTER));
    expect(w.SK.importersOf(UTIL)).toEqual([APP]);

    definePlain(w, UTIL, [], 2);
    await applyChanges(w, changed(UTIL));
    expect(w.hmr.reload).toEqual([COUNTER, APP]); // Counter from its own change only
    expect(w.runs(COUNTER)).toBe(2);
  });

  test("adding an import creates the reverse edge", async () => {
    const w = await bootStandardFixture();
    const NEW = "app/src/new.js";
    definePlain(w, NEW, [], 1);
    defineComponent(
      w,
      APP,
      [
        { spec: "./util.js", id: UTIL },
        { spec: "./components/Counter.vue", id: COUNTER },
        { spec: "./new.js", id: NEW },
      ],
      2,
    );
    await applyChanges(w, changed(APP));
    expect(w.SK.importersOf(NEW)).toEqual([APP]);
    expect(w.runs(NEW)).toBe(1);

    definePlain(w, NEW, [], 2);
    await applyChanges(w, changed(NEW));
    expect(w.hmr.reload).toEqual([APP, APP]);
    expect(w.runs(NEW)).toBe(2);
  });
});
