import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { appDataDir, expect, launch, packagedExe, REPO, type SkApp } from "../src/index";

const T = 60_000;
const BUILD_T = 20 * 60_000;
const ENTRY = "packages/test/fixtures/idb-persist/src/main.ts";
const run = `${process.pid}`;
const folders = new Set<string>();

afterAll(async () => {
  for (const f of folders) {
    const appDir = dirname(dirname(f));
    if (!/^sk-(test-idb|app-sktest)-/.test(basename(appDir))) continue;
    for (let i = 0; ; i++) {
      try {
        rmSync(appDir, { recursive: true, force: true });
        break;
      } catch (e) {
        if (i >= 50) throw e;
        await new Promise((r) => setTimeout(r, 100));
      }
    }
  }
});

/**
 * taskkill /F — no quit observer runs — and wait until the process is gone (its folder lock and
 * file handles with it).
 */
async function kill(app: SkApp) {
  await app.close();
  const deadline = Date.now() + 10_000;
  while (app.child.exitCode === null && app.child.signalCode === null && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 25));
  }
}

async function scenario(app: SkApp, name: string, ...args: string[]): Promise<string> {
  await app.eval(`__SK_TEST_IDB__.run(${[name, ...args].map((a) => JSON.stringify(a)).join(",")})`);
  const deadline = Date.now() + 15_000;
  for (;;) {
    const v = (await app.eval(`String(__SK_TEST_IDB__.results[${JSON.stringify(name)}] ?? "")`)) as string;
    if (v !== "") return v;
    if (Date.now() > deadline) throw new Error(`scenario ${name} did not settle`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

type IdbInfo = {
  folder: string;
  started: boolean;
  mode: string | null;
  encrypted: boolean | null;
  key: boolean;
  error: string | null;
};
async function info(app: SkApp): Promise<IdbInfo> {
  const i = JSON.parse(await scenario(app, "info")) as IdbInfo;
  folders.add(i.folder);
  return i;
}

function host(name: string, args: string[] = []): Promise<SkApp> {
  return launch({ app: ENTRY, label: `idb-persist-${name}`, env: { SK_APP_NAME: `sk-test-idb-${run}-${name}` }, args });
}

/** A graceful quit — the last window closes and gpui runs its quit observers. */
async function quit(app: SkApp) {
  await app.eval("setTimeout(() => close(), 50), 1");
  const deadline = Date.now() + 15_000;
  while (app.child.exitCode === null && Date.now() < deadline) await new Promise((r) => setTimeout(r, 50));
  bunExpect(app.child.exitCode).not.toBeNull();
  await app.close();
}

function files(dir: string): string[] {
  return readdirSync(dir)
    .filter((n) => !n.startsWith("key."))
    .map((n) => join(dir, n));
}

function spellings(s: string): Buffer[] {
  return [Buffer.from(s, "latin1"), Buffer.from(s, "utf16le"), Buffer.from([...s].map((c) => c.charCodeAt(0) + 1))];
}

function legible(dir: string, ...needles: string[]): string[] {
  const hits: string[] = [];
  for (const f of files(dir)) {
    const bytes = readFileSync(f);
    for (const n of needles) for (const sp of spellings(n)) if (bytes.includes(sp)) hits.push(`${basename(f)}: ${n}`);
  }
  return hits;
}

const logLines = (app: SkApp, needle: string) =>
  readFileSync(app.hostLog, "utf8")
    .split(/\r?\n/)
    .filter((l) => l.includes(needle));

describe("IndexedDB on disk, dev host", () => {
  test(
    "what one host wrote is there for the next — after a kill, and after a graceful quit",
    async () => {
      const value = `persisted-${run}-${Date.now()}`;
      let a = await host("restart");
      await expect(a.locator("#status")).toHaveText("ready");
      const i = await info(a);
      bunExpect(i.mode).toBe("plaintext");
      bunExpect(await scenario(a, "put", "greeting", value)).toBe("ok");
      bunExpect(logLines(a, "IN MEMORY ONLY")).toEqual([]);
      bunExpect(readdirSync(i.folder).some((n) => n.endsWith(".sqlite3"))).toBe(true);
      await kill(a);
      bunExpect(
        readdirSync(i.folder).some((n) => n.endsWith(".sqlite3-wal")),
        "the WAL survives a kill",
      ).toBe(true);

      a = await host("restart");
      await expect(a.locator("#status")).toHaveText("ready");
      bunExpect(await scenario(a, "get", "greeting")).toBe(value);
      bunExpect(await scenario(a, "databases")).toBe("sk-test-persist@1");
      await quit(a);
      bunExpect(
        readdirSync(i.folder).filter((n) => n.endsWith("-wal")),
        "a graceful quit checkpoints the WAL away",
      ).toEqual([]);
      bunExpect(logLines(a, "did not finish closing")).toEqual([]);

      a = await host("restart");
      await expect(a.locator("#status")).toHaveText("ready");
      bunExpect(await scenario(a, "get", "greeting")).toBe(value);
      await kill(a);
    },
    T,
  );

  test(
    "encrypted at rest: no plaintext on disk, sticky across a restart, decrypted only when asked",
    async () => {
      const key = `sk-key-${run}`;
      const value = `sk-secret-${run}-${Date.now()}`;
      let a = await host("sealed", ["encrypt"]);
      await expect(a.locator("#status")).toHaveText("ready");
      let i = await info(a);
      bunExpect(i.mode).toBe("encrypted");
      bunExpect(i.key).toBe(true);
      bunExpect(await scenario(a, "put", key, value)).toBe("ok");
      await kill(a);
      bunExpect(legible(i.folder, key, value), "plaintext in an encrypted folder").toEqual([]);

      a = await host("sealed");
      await expect(a.locator("#status")).toHaveText("ready");
      i = await info(a);
      bunExpect(i.mode).toBe("encrypted");
      bunExpect(await scenario(a, "get", key)).toBe(value);
      bunExpect(await scenario(a, "put", `${key}-2`, `${value}-2`)).toBe("ok");
      await quit(a);
      bunExpect(legible(i.folder, key, value, `${value}-2`)).toEqual([]);

      a = await host("sealed", ["keep"]);
      await expect(a.locator("#status")).toHaveText("ready");
      i = await info(a);
      bunExpect(i.mode).toBe("encrypted");
      bunExpect(await scenario(a, "get", key)).toBe(value);
      const kept = JSON.parse(await scenario(a, "late", "keep")) as IdbInfo;
      bunExpect(kept.key).toBe(true);
      bunExpect(existsSync(join(i.folder, "key.retire")), "no decryption was queued for the next start").toBe(false);
      await quit(a);
      bunExpect(legible(i.folder, key, value, `${value}-2`)).toEqual([]);

      a = await host("sealed", ["decrypt"]);
      await expect(a.locator("#status")).toHaveText("ready");
      i = await info(a);
      bunExpect(i.mode).toBe("plaintext");
      bunExpect(i.key).toBe(false);
      bunExpect(await scenario(a, "get", key)).toBe(value);
      await quit(a);
      bunExpect(legible(i.folder, value).length).toBeGreaterThan(0);
      bunExpect(existsSync(join(i.folder, "key.sealed"))).toBe(false);
    },
    T,
  );

  test(
    "a late { encrypted: true } applies at the next start, said once",
    async () => {
      let a = await host("late");
      await expect(a.locator("#status")).toHaveText("ready");
      bunExpect((await info(a)).mode).toBe("plaintext");
      const late = JSON.parse(await scenario(a, "late", "true")) as IdbInfo;
      bunExpect(late.mode, "this run stays as it started").toBe("plaintext");
      bunExpect(late.key, "but the key for the next start exists").toBe(true);
      bunExpect(logLines(a, "applies at the next start").length).toBe(1);
      bunExpect(await scenario(a, "put", "k", "late-value")).toBe("ok");
      await quit(a);

      a = await host("late");
      await expect(a.locator("#status")).toHaveText("ready");
      const i = await info(a);
      bunExpect(i.mode).toBe("encrypted");
      bunExpect(await scenario(a, "get", "k")).toBe("late-value");
      await quit(a);
      bunExpect(legible(i.folder, "late-value")).toEqual([]);
    },
    T,
  );

  test(
    "a second instance on the same folder gets UnknownError and an advisory, then recovers",
    async () => {
      const first = await host("twice");
      await expect(first.locator("#status")).toHaveText("ready");
      bunExpect(await scenario(first, "put", "owner", "first")).toBe("ok");
      await info(first);

      const second = await host("twice");
      await expect(second.locator("#status")).toHaveText("failed: UnknownError");
      bunExpect(await second.eval("String(__SK_TEST_IDB__.results.boot)")).toContain("another instance");
      bunExpect(await scenario(second, "databases")).toStartWith("ERR UnknownError");
      const advisory = logLines(second, "[redjs] IndexedDB:").filter((l) => l.includes("another instance"));
      bunExpect(advisory.length).toBeGreaterThan(0);
      bunExpect(advisory[0]).toContain(`pid ${first.pid}`);

      await kill(first);
      let opened = "";
      for (let i = 0; i < 50 && opened !== "ok"; i++) {
        opened = await scenario(second, "open");
        if (opened !== "ok") await new Promise((r) => setTimeout(r, 100));
      }
      bunExpect(opened).toBe("ok");
      bunExpect(await scenario(second, "get", "owner")).toBe("first");
      await kill(second);
    },
    T,
  );
});

describe("IndexedDB on disk, packaged exe", () => {
  test(
    "a packaged app's record survives its restart",
    async () => {
      const p = await packagedExe({
        app: join(REPO, "packages", "test", "fixtures", "idb-persist"),
        label: "idb-persist",
      });
      const appDir = appDataDir(p.name);
      rmSync(appDir, { recursive: true, force: true });
      const value = `packaged-${run}-${Date.now()}`;
      let a = await launch({ app: "idb-persist", exe: p.exe, packaged: p.name, label: "idb-persist-packaged" });
      await expect(a.locator("#status")).toHaveText("ready");
      const i = await info(a);
      bunExpect(i.folder).toBe(join(appDir, "storage", "indexeddb"));
      bunExpect(i.mode).toBe("plaintext");
      bunExpect(logLines(a, "IN MEMORY ONLY")).toEqual([]);
      bunExpect(await scenario(a, "put", "greeting", value)).toBe("ok");
      await quit(a);

      a = await launch({ app: "idb-persist", exe: p.exe, packaged: p.name, label: "idb-persist-packaged" });
      await expect(a.locator("#status")).toHaveText("ready");
      bunExpect(await scenario(a, "get", "greeting")).toBe(value);
      await kill(a);
    },
    BUILD_T,
  );
});
