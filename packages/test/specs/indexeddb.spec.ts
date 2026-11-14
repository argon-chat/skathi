import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { readFileSync, rmSync } from "node:fs";
import { basename, dirname } from "node:path";
import { expect, launch } from "../src/index";

const appName = `sk-test-idb-${process.pid}-p1`;
const app = await launch({
  app: "packages/test/fixtures/indexeddb/src/main.ts",
  window: [480, 360],
  label: "indexeddb",
  env: { SK_APP_NAME: appName },
});
let idbFolder = "";
afterAll(async () => {
  await app.close();
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

async function scenario(name: string, ms = 15_000): Promise<string> {
  await app.eval(`__SK_TEST_IDB__.run(${JSON.stringify(name)})`);
  const deadline = Date.now() + ms;
  for (;;) {
    const v = (await app.eval(`String(__SK_TEST_IDB__.results[${JSON.stringify(name)}] ?? "")`)) as string;
    if (v !== "") return v;
    if (Date.now() > deadline) throw new Error(`scenario ${name} did not settle in ${ms} ms`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

describe("IndexedDB in a Vue app", () => {
  test(
    "the app opened its database at boot and rendered what is stored",
    async () => {
      await expect(app.locator("#status")).toHaveText("ready");
      await expect(app.locator("#count")).toHaveText("count=0");
      bunExpect(await scenario("surface")).toBe("object|function|[object IDBFactory]|function");
    },
    T,
  );

  test(
    "a click's transaction commits and Vue renders what came back out",
    async () => {
      // One REAL click (gpui's hit test, a host-dispatched handler) and two devtools clicks: the
      // transaction is made inside the handler either way.
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
      await app.locator("#add").click();
      await expect(app.locator(".note").nth(0)).toHaveText("4:note 1");
    },
    T,
  );

  test(
    "500 sequential awaited gets finish in a few frames, not 500",
    async () => {
      const [sum, frames, ms] = (await scenario("chain")).split("|").map(Number);
      bunExpect(sum).toBe(124750);
      // Delivered once per pump this would be ~500 frames; the pump's budget delivers a chain of
      // dependent results inside one frame.
      console.log(`[indexeddb] 500 sequential awaited gets: ${frames} frames, ${ms} ms`);
      bunExpect(frames).toBeLessThanOrEqual(40);
    },
    T,
  );

  test(
    "versionchange closes the old connection and the upgrade runs, unblocked",
    async () => {
      bunExpect(await scenario("versionchange")).toBe("vc 1->2,upgrade 1->2|v2|added");
    },
    T,
  );

  test(
    "a uniqueness violation fails the request and aborts the transaction",
    async () => {
      bunExpect(await scenario("constraint")).toBe("ConstraintError|aborted ConstraintError|0");
    },
    T,
  );

  test(
    "values go through the structured clone and come back as copies",
    async () => {
      bunExpect(await scenario("structured")).toBe('0|1|2|1,2,3|hi|{"list":[1,"two",null]}|copy');
    },
    T,
  );

  test(
    "databases() and navigator.storage see the app's data",
    async () => {
      const [names, used, quota, persisted] = (await scenario("storage")).split("|");
      bunExpect([names, used, persisted]).toEqual(["true", "true", "true"]);
      console.log(`[indexeddb] navigator.storage.estimate().quota = ${quota}`);
      bunExpect(Number(quota)).toBeGreaterThan(0);
    },
    T,
  );

  test(
    "the databases are on disk under the storage folder, and nothing says 'memory only'",
    async () => {
      const info = JSON.parse((await app.eval("JSON.stringify(__SK_STORAGE__.info())")) as string) as {
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
    },
    T,
  );
});
