import { afterAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { arch, homedir, hostname, platform } from "node:os";
import { killTree, launch, type SkApp } from "../src/index";

const T = 60_000;
const apps: SkApp[] = [];
const started: number[] = [];
afterAll(async () => {
  for (const pid of started) killTree(pid);
  for (const a of apps) await a.close();
});

async function start() {
  const app = await launch({ app: "packages/test/fixtures/std/src/main.ts", window: [420, 300], label: "std" });
  apps.push(app);
  await app.locator("#ready").waitFor();
  return app;
}

async function ended(app: SkApp): Promise<number | null> {
  const deadline = Date.now() + 10_000;
  while (app.child.exitCode === null && Date.now() < deadline) await Bun.sleep(20);
  return app.child.exitCode;
}

describe("skathi:* modules", () => {
  test(
    "skathi:os, skathi:runtime and skathi:app answer for this machine and this app",
    async () => {
      const app = await start();
      const p = JSON.parse(String(await app.eval("__SK_TEST_STD__.probe()")));
      expect(p.version).toBe("4.5.6");
      expect(p.sameShell).toBe(true);
      expect(p.os.platform).toBe(platform());
      expect(p.os.arch).toBe(arch());
      expect(["x64", "arm64"]).toContain(p.os.machineArch);
      expect(p.os.release).toMatch(/^\d+\.\d+/);
      expect(p.os.hostname.toLowerCase()).toBe(hostname().toLowerCase());
      expect(p.os.homedir).toBe(homedir());
      expect(p.os.tmpdir).not.toMatch(/[\\/]$/);
      expect(p.os.isOS).toBe(true);
      expect(p.os.frozenDistro).toBe(true);
      if (platform() !== "linux") expect(p.os.distro).toBeNull();
      expect(p.runtime.version).toMatch(/^\d+\.\d+\.\d+/);
      expect(Array.isArray(p.runtime.features)).toBe(true);
      expect(p.runtime.isAot).toBe(false);
      expect(p.runtime.isSingleFile).toBe(false);
      expect(p.badExitCode).toStartWith("TypeError");
      expect(p.badArgs).toStartWith("TypeError");
      expect(app.child.exitCode).toBeNull();
    },
    T,
  );

  test(
    "app.quit({ exitCode }) fires pagehide and ends the process with that code",
    async () => {
      const app = await start();
      await app.eval(`addEventListener("pagehide", () => console.log("[std-spec] pagehide")); "ok"`);
      void app.eval("__SK_TEST_STD__.app.quit({ exitCode: 7 }); 1").catch(() => {});
      expect(await ended(app)).toBe(7);
      const log = readFileSync(app.hostLog, "utf8");
      expect(log.split("[std-spec] pagehide").length - 1).toBe(1);
    },
    T,
  );

  test(
    "app.restart({ args }) starts the program again, then ends this process",
    async () => {
      const app = await start();
      // A host given this argument prints what it is and exits, so nothing is left running.
      void app.eval(`__SK_TEST_STD__.app.restart({ args: ["--sk-host-info"] }); 1`).catch(() => {});
      expect(await ended(app)).toBe(0);
      const pid = /app\.restart: started process (\d+)/.exec(readFileSync(app.hostLog, "utf8"))?.[1];
      expect(pid).toBeDefined();
      started.push(Number(pid));
    },
    T,
  );
});
