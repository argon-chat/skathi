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
  definePlain,
  redefineCssOnly,
} from "./loader-harness";

const changed = (id, kind) => [{ id, kind: kind || "reload" }];

describe("external app module ids", () => {
  test("retains parents outside the SDK while resolving sibling modules", () => {
    const w = createLoader();
    const entry = "../../client/packages/editor/main.ts";
    const helper = "../../client/packages/editor/gpu/draw.ts";
    const shared = "../../client/packages/shared.ts";
    w.VN.define(helper, (_require, module) => { module.exports = { draw: 42 }; });
    w.VN.define(shared, (_require, module) => { module.exports = { shared: 17 }; });
    w.VN.define(entry, (require, module) => {
      module.exports = { value: require("./gpu/draw").draw + require("../shared").shared };
    });
    expect(w.VN.resolve("./gpu/draw", entry)).toBe(helper);
    expect(w.VN.resolve("../shared", entry)).toBe(shared);
    expect(w.VN.require(entry).value).toBe(59);
  });
});

describe("boot", () => {
  test("requires every reachable module exactly once", () => {
    const w = bootStandardFixture();
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(COUNTER)).toBe(1);
    expect(w.runs(UTIL)).toBe(1);
    expect(w.hmr.createRecord).toEqual([COUNTER, APP]);
    expect(w.hardRestarted()).toBe(false);
  });

  test("records both directions of the graph", () => {
    const w = bootStandardFixture();
    expect(w.VN.depsOf(APP)).toEqual([UTIL, COUNTER]);
    expect(w.VN.importersOf(UTIL).sort()).toEqual([APP, COUNTER].sort());
    expect(w.VN.importersOf(APP)).toEqual([ENTRY]);
    expect(w.VN.importersOf(ENTRY)).toEqual([]);
  });
});

describe("shared plain module changed", () => {
  test("reloads exactly the components that import it, entry untouched", () => {
    const w = bootStandardFixture();
    definePlain(w, UTIL, [], 2); // host re-evals the changed script
    applyChanges(w, changed(UTIL));

    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(APP)).toBe(2);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.hmr.reload.slice().sort()).toEqual([APP, COUNTER].sort());
    expect(w.hmr.reload.length).toBe(2);
    expect(w.hmr.rerender).toEqual([]);
    expect(w.runs(ENTRY)).toBe(1);
    expect(w.hardRestarted()).toBe(false);
  });

  test("the reloaded components see the NEW exports", () => {
    const w = bootStandardFixture();
    const oldApp = w.hmr.records.get(APP);
    expect(oldApp.imported["./util.js"].version).toBe(1);
    definePlain(w, UTIL, [], 2);
    applyChanges(w, changed(UTIL));

    const newApp = w.hmr.records.get(APP);
    expect(newApp).not.toBe(oldApp); // a fresh def object from a fresh factory run
    expect(newApp.__version).toBe(1); // App's own code did not change...
    expect(newApp.imported["./util.js"].version).toBe(2); // ...but its import did
    expect(w.hmr.records.get(COUNTER).imported["../util.js"].version).toBe(2);
    expect(newApp.imported["./util.js"]).toBe(
      w.hmr.records.get(COUNTER).imported["../util.js"],
    );
  });

  test("createRecord stays idempotent across the reload", () => {
    const w = bootStandardFixture();
    definePlain(w, UTIL, [], 2);
    applyChanges(w, changed(UTIL));
    expect(w.hmr.createRecord).toEqual([COUNTER, APP, COUNTER, APP]);
  });

  test("a leaf module only one component imports reloads only that component", () => {
    const w = bootStandardFixture();
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
    applyChanges(w, changed(COUNTER));
    expect(w.hmr.reload).toEqual([COUNTER]);

    definePlain(w, FMT, [], 2);
    applyChanges(w, changed(FMT));
    expect(w.hmr.reload).toEqual([COUNTER, COUNTER]);
    expect(w.runs(APP)).toBe(1);
  });
});

describe("walk boundaries", () => {
  test("stops at the nearest component (does not climb past it)", () => {
    const w = createLoader();
    const HELPER = "app/src/helper.js";
    definePlain(w, HELPER, [], 1);
    defineComponent(w, COUNTER, [{ spec: "../helper.js", id: HELPER }], 1);
    defineComponent(w, APP, [{ spec: "./components/Counter.vue", id: COUNTER }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    w.VN.boot(ENTRY);

    definePlain(w, HELPER, [], 2);
    applyChanges(w, changed(HELPER));
    expect(w.hmr.reload).toEqual([COUNTER]);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(ENTRY)).toBe(1);
  });

  test("an intermediate plain module is walked THROUGH and re-run once", () => {
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
    w.VN.boot(ENTRY);

    definePlain(w, BASE, [], 2);
    applyChanges(w, changed(BASE));
    expect(w.runs(BASE)).toBe(2); // once, despite two paths reaching it
    expect(w.runs(MID)).toBe(2);
    expect(w.runs(APP)).toBe(2);
    expect(w.hmr.reload).toEqual([APP]);
    expect(w.hmr.records.get(APP).imported["./mid.js"].imported["./base.js"].version).toBe(2);
  });

  test("a cycle between plain modules terminates", () => {
    const w = createLoader();
    const A = "app/src/a.js";
    const B = "app/src/b.js";
    definePlain(w, A, [{ spec: "./b.js", id: B }], 1);
    definePlain(w, B, [{ spec: "./a.js", id: A }], 1);
    defineComponent(w, APP, [{ spec: "./a.js", id: A }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    w.VN.boot(ENTRY);

    definePlain(w, A, [{ spec: "./b.js", id: B }], 2);
    applyChanges(w, changed(A));
    expect(w.hmr.reload).toEqual([APP]);
    expect(w.runs(A)).toBe(2);
    expect(w.runs(B)).toBe(2);
    expect(w.hardRestarted()).toBe(false);
  });

  test("a self-import edge is dropped rather than looping", () => {
    const w = createLoader();
    const SELF = "app/src/self.js";
    w.VN.setDeps(SELF, [SELF, "app/src/other.js"]);
    expect(w.VN.depsOf(SELF)).toEqual(["app/src/other.js"]);
    expect(w.VN.importersOf(SELF)).toEqual([]);
  });
});

describe("fallbacks", () => {
  test("reaching the entry hard-restarts", () => {
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
    w.VN.boot(ENTRY);

    definePlain(w, CONFIG, [], 2);
    applyChanges(w, changed(CONFIG));
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
    expect(w.runs(CONFIG)).toBe(2);
    expect(w.hmr.reload).toEqual([]);
  });

  test("the entry module itself still hard-restarts", () => {
    const w = bootStandardFixture();
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 2);
    applyChanges(w, changed(ENTRY));
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
  });

  test("a module the running app never required is a no-op", () => {
    const w = bootStandardFixture();
    const ORPHAN = "app/src/orphan.js";
    definePlain(w, ORPHAN, [], 1);
    applyChanges(w, changed(ORPHAN));
    expect(w.runs(ORPHAN)).toBe(0); // not executed: nothing needs it yet
    expect(w.hmr.reload).toEqual([]);
    expect(w.hardRestarted()).toBe(false);
    expect(w.logsMatching("skip " + ORPHAN).length).toBe(1);
  });

  test("a component with no HMR-able export hard-restarts", () => {
    const w = bootStandardFixture();
    w.VN.setDeps(APP, [UTIL, COUNTER]);
    w.VN.define(
      APP,
      function (require, module, exports) {
        require("./util.js");
        exports.default = { notAComponent: true };
      },
      APP + "@broken",
    );
    applyChanges(w, changed(APP));
    expect(w.hardRestarted()).toBe(true);
  });

  test("a factory that throws hard-restarts instead of leaving a half-state", () => {
    const w = bootStandardFixture();
    w.VN.setDeps(UTIL, []);
    w.VN.define(
      UTIL,
      function () {
        throw new Error("boom");
      },
      UTIL + "@throws",
    );
    let threw = null;
    try {
      applyChanges(w, changed(UTIL));
    } catch (e) {
      threw = e; // hardRestart re-runs the entry, which hits the same throw
    }
    expect(threw && String(threw)).toContain("boom");
    expect(w.hardRestarted()).toBe(true);
  });

  test("recovers by hard restart when a previous boot left the app down", () => {
    const w = createLoader();
    const MISSING = "app/src/missing.js";
    defineComponent(w, APP, [{ spec: "./missing.js", id: MISSING }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    expect(() => w.VN.boot(ENTRY)).toThrow(); // missing.js is not defined yet
    expect(w.runs(APP)).toBe(1);

    definePlain(w, MISSING, [], 1); // the file finally shows up
    applyChanges(w, changed(MISSING));
    expect(w.hardRestarted()).toBe(true);
    expect(w.runs(ENTRY)).toBe(2);
    expect(w.runs(APP)).toBe(2);
    expect(w.runs(MISSING)).toBe(1);
  });
});

describe("component changes keep their existing behaviour", () => {
  test("rerender swaps only the render fn", () => {
    const w = bootStandardFixture();
    defineComponent(w, COUNTER, [{ spec: "../util.js", id: UTIL }], 2);
    applyChanges(w, changed(COUNTER, "rerender"));
    expect(w.hmr.rerender).toEqual([COUNTER]);
    expect(w.hmr.reload).toEqual([]);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.runs(APP)).toBe(1); // importers keep their reference on purpose
    expect(w.runs(UTIL)).toBe(1);
  });

  test("reload of a .vue does not disturb its importers", () => {
    const w = bootStandardFixture();
    defineComponent(w, COUNTER, [{ spec: "../util.js", id: UTIL }], 2);
    applyChanges(w, changed(COUNTER));
    expect(w.hmr.reload).toEqual([COUNTER]);
    expect(w.runs(APP)).toBe(1);
    expect(w.runs(ENTRY)).toBe(1);
  });
});

describe("batched changes", () => {
  test("a component changed alongside its dependency runs once, reloads once", () => {
    const w = bootStandardFixture();
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
    applyChanges(w, [
      { id: APP, kind: "reload" }, // listed first on purpose
      { id: UTIL, kind: "reload" },
    ]);
    expect(w.runs(APP)).toBe(2); // not 3: the graph walk and its own change agree
    expect(w.runs(UTIL)).toBe(2);
    expect(w.runs(COUNTER)).toBe(2);
    expect(w.hmr.reload.slice().sort()).toEqual([APP, COUNTER].sort());
    expect(w.hmr.reload.length).toBe(2);
    expect(w.hmr.records.get(APP).__version).toBe(2);
    expect(w.hmr.records.get(APP).imported["./util.js"].version).toBe(2);
  });

  test("a chain changed end to end runs each module once", () => {
    const w = createLoader();
    const BASE = "app/src/base.js";
    const MID = "app/src/mid.js";
    definePlain(w, BASE, [], 1);
    definePlain(w, MID, [{ spec: "./base.js", id: BASE }], 1);
    defineComponent(w, APP, [{ spec: "./mid.js", id: MID }], 1);
    definePlain(w, ENTRY, [{ spec: "./App.vue", id: APP }], 1);
    w.VN.boot(ENTRY);

    // MID listed BEFORE BASE — the walk from MID already refreshes BASE, so
    // BASE's own entry must not start a second round.
    definePlain(w, MID, [{ spec: "./base.js", id: BASE }], 2);
    definePlain(w, BASE, [], 2);
    applyChanges(w, [
      { id: MID, kind: "reload" },
      { id: BASE, kind: "reload" },
    ]);
    expect(w.runs(APP)).toBe(2);
    expect(w.runs(MID)).toBe(2);
    expect(w.runs(BASE)).toBe(2);
    expect(w.hmr.reload).toEqual([APP]);
    expect(w.logsMatching("already refreshed").length).toBe(1);
  });

  test("two independent plain modules both apply", () => {
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
    w.VN.boot(ENTRY);

    definePlain(w, A, [], 2);
    definePlain(w, B, [], 2);
    applyChanges(w, [
      { id: A, kind: "reload" },
      { id: B, kind: "reload" },
    ]);
    expect(w.hmr.reload).toEqual([APP, COUNTER]);
    expect(w.runs(ENTRY)).toBe(1);
  });
});

describe("style-only edits", () => {
  test("do not invalidate the require cache", () => {
    const w = bootStandardFixture();
    const before = w.VN.require(APP).default;
    redefineCssOnly(w, APP, [
      { spec: "./util.js", id: UTIL },
      { spec: "./components/Counter.vue", id: COUNTER },
    ], 1, ".app { color: green }");
    applyChanges(w, changed(APP, "css-only"));

    expect(w.runs(APP)).toBe(1); // never re-executed
    expect(w.VN.require(APP).default).toBe(before); // same live component object
    expect(w.hmr.reload).toEqual([]);
    expect(w.hmr.rerender).toEqual([]);
    expect(w.stylesheets[w.stylesheets.length - 1]).toContain("color: green");
    expect(w.hardRestarted()).toBe(false);
  });

  test("a define() with no hash still invalidates (older builders)", () => {
    const w = bootStandardFixture();
    const before = w.VN.require(APP).default;
    w.VN.setDeps(APP, [UTIL, COUNTER]);
    w.VN.define(APP, function (require, module, exports) {
      require("./util.js");
      require("./components/Counter.vue");
      exports.default = { __hmrId: APP, render: function () {} };
    });
    expect(w.VN.loaded(APP)).toBe(false);
    expect(w.VN.require(APP).default).not.toBe(before);
  });
});

describe("graph maintenance", () => {
  test("removing an import drops the reverse edge", () => {
    const w = bootStandardFixture();
    expect(w.VN.importersOf(UTIL).sort()).toEqual([APP, COUNTER].sort());
    defineComponent(w, COUNTER, [], 2); // Counter no longer imports util
    applyChanges(w, changed(COUNTER));
    expect(w.VN.importersOf(UTIL)).toEqual([APP]);

    definePlain(w, UTIL, [], 2);
    applyChanges(w, changed(UTIL));
    expect(w.hmr.reload).toEqual([COUNTER, APP]); // Counter from its own change only
    expect(w.runs(COUNTER)).toBe(2);
  });

  test("adding an import creates the reverse edge", () => {
    const w = bootStandardFixture();
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
    applyChanges(w, changed(APP));
    expect(w.VN.importersOf(NEW)).toEqual([APP]);
    expect(w.runs(NEW)).toBe(1);

    definePlain(w, NEW, [], 2);
    applyChanges(w, changed(NEW));
    expect(w.hmr.reload).toEqual([APP, APP]);
    expect(w.runs(NEW)).toBe(2);
  });
});
