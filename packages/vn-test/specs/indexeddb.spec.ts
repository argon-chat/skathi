// IndexedDB in a real Vue app, on the real host, inside the real frame loop
// (m25 P1/P2 — research/reports/m25/indexeddb-design.md, -p1-impl.md,
// -p2-impl.md).
//
//   bun test packages/vn-test/specs/indexeddb.spec.ts
//
// redjs proves the API itself — the WPT subset (vendor/redjs/redjs/tests/
// wpt_idb.rs) and Dexie / idb / idb-keyval / localForage unmodified
// (tests/idb_libs.rs), on both backends. What only a host can prove is the
// integration: that the build ships it, that a Vue app's state can live in
// it, that a click handler's transaction commits and Vue re-renders what came
// back, that results land inside vn-app's own frame pump within the budget
// vn-runtime sets (`delivery_budget_us`), and that the databases are ON DISK
// under the app's storage folder — no memory-only advisory. Persistence across
// restarts, encryption and the second instance: indexeddb-disk.spec.ts.
//
// `fixtures/indexeddb` is the app; `__VN_TEST_IDB__.run(name)` starts one of its
// scenarios and records how it settled. The host gets its own `VN_APP_NAME`
// (a fresh, empty storage folder), removed afterwards.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { readFileSync, rmSync } from "node:fs";
import { basename, dirname } from "node:path";
import { expect, launch } from "../src/index";

const appName = `vn-test-idb-${process.pid}-p1`;
const app = await launch({
  app: "packages/vn-test/fixtures/indexeddb/src/main.ts",
  window: [480, 360],
  label: "indexeddb",
  env: { VN_APP_NAME: appName },
});
let idbFolder = "";
afterAll(async () => {
  await app.close();
  // `<LOCALAPPDATA>\<appName>\storage\indexeddb` — the app-name directory goes,
  // once the killed host has let go of its files.
  const appDir = dirname(dirname(idbFolder));
  if (!idbFolder || basename(appDir) !== appName) return;
  for (let i = 0; ; i++) {
    try {
      rmSync(appDir, { recursive: true, force: true });
      break;
    } catch (e) {
      if (i >= 50) throw e;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
});

const T = 30_000;

/** Starts a scenario and polls until it has settled. */
async function scenario(name: string, ms = 15_000): Promise<string> {
  await app.eval(`__VN_TEST_IDB__.run(${JSON.stringify(name)})`);
  const deadline = Date.now() + ms;
  for (;;) {
    const v = (await app.eval(`String(__VN_TEST_IDB__.results[${JSON.stringify(name)}] ?? "")`)) as string;
    if (v !== "") return v;
    if (Date.now() > deadline) throw new Error(`scenario ${name} did not settle in ${ms} ms`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

describe("IndexedDB in a Vue app", () => {
  test("the app opened its database at boot and rendered what is stored", async () => {
    await expect(app.locator("#status")).toHaveText("ready");
    await expect(app.locator("#count")).toHaveText("count=0");
    bunExpect(await scenario("surface")).toBe("object|function|[object IDBFactory]|function");
  }, T);

  test("a click's transaction commits and Vue renders what came back out", async () => {
    // One REAL click (gpui's hit test, a host-dispatched handler) and two
    // devtools clicks: the transaction is made inside the handler either way.
    await app.locator("#add").click({ real: true });
    await expect(app.locator("#count")).toHaveText("count=1");
    await app.locator("#add").click();
    await expect(app.locator("#count")).toHaveText("count=2");
    await app.locator("#add").click();
    await expect(app.locator("#count")).toHaveText("count=3");
    await expect(app.locator(".note")).toHaveCount(3);
    await expect(app.locator(".note").nth(0)).toHaveText("1:note 1");
    await expect(app.locator(".note").nth(1)).toHaveText("2:note 2 (done)");
    await expect(app.locator(".note").nth(2)).toHaveText("3:note 3");
    await app.locator("#clear").click();
    await expect(app.locator("#count")).toHaveText("count=0");
    await expect(app.locator(".note")).toHaveCount(0);
    // The key generator is not reset by clear(), exactly as in a browser.
    await app.locator("#add").click();
    await expect(app.locator(".note").nth(0)).toHaveText("4:note 1");
  }, T);

  test("500 sequential awaited gets finish in a few frames, not 500", async () => {
    const [sum, frames, ms] = (await scenario("chain")).split("|").map(Number);
    bunExpect(sum).toBe(124750);
    // Delivered once per pump this would be ~500 frames; the pump's budget
    // delivers a chain of dependent results inside one frame. The guard is
    // loose on purpose (a debug host, a busy machine); the number is printed.
    console.log(`[indexeddb] 500 sequential awaited gets: ${frames} frames, ${ms} ms`);
    bunExpect(frames).toBeLessThanOrEqual(40);
  }, T);

  test("versionchange closes the old connection and the upgrade runs, unblocked", async () => {
    bunExpect(await scenario("versionchange")).toBe("vc 1->2,upgrade 1->2|v2|added");
  }, T);

  test("a uniqueness violation fails the request and aborts the transaction", async () => {
    bunExpect(await scenario("constraint")).toBe("ConstraintError|aborted ConstraintError|0");
  }, T);

  test("values go through the structured clone and come back as copies", async () => {
    bunExpect(await scenario("structured")).toBe('0|1|2|1,2,3|hi|{"list":[1,"two",null]}|copy');
  }, T);

  test("databases() and navigator.storage see the app's data", async () => {
    // The quota is the disk's (vn-storage idb.rs: min(60% of the volume,
    // used + free)), not P1's fixed 1 GiB.
    const [names, used, quota, persisted] = (await scenario("storage")).split("|");
    bunExpect([names, used, persisted]).toEqual(["true", "true", "true"]);
    console.log(`[indexeddb] navigator.storage.estimate().quota = ${quota}`);
    bunExpect(Number(quota)).toBeGreaterThan(0);
  }, T);

  test("the databases are on disk under the storage folder, and nothing says 'memory only'", async () => {
    const info = JSON.parse((await app.eval("__vn_storage_info()")) as string) as {
      folder: string;
      indexedDB: { folder: string; started: boolean; mode: string };
    };
    idbFolder = info.indexedDB.folder;
    bunExpect(info.indexedDB.folder).toBe(`${info.folder}\\indexeddb`);
    bunExpect(info.indexedDB.started).toBe(true);
    bunExpect(info.indexedDB.mode).toBe("plaintext");
    const log = readFileSync(app.hostLog, "utf8");
    const lines = log.split(/\r?\n/).filter((l) => l.includes("[redjs] IndexedDB") && l.includes("IN MEMORY ONLY"));
    bunExpect(lines).toEqual([]);
  }, T);
});
