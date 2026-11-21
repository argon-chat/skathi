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
    "skathi:toml parses into JS values and writes them back",
    async () => {
      const app = await start();
      const t = JSON.parse(String(await app.eval("__SK_TEST_STD__.toml()")));
      expect(t.keys).toEqual(["title", "big", "ratio", "when", "day", "tags", "window", "servers"]);
      expect([t.title, t.big, t.bigType, t.ratio]).toEqual(["Skathi", "9007199254740993", "bigint", 0.5]);
      expect(t.when).toEqual(["Date", "1979-05-27T07:32:00.000Z"]);
      expect(t.day).toEqual(["Temporal.PlainDate", "1979-05-27"]);
      expect(t.window).toEqual({ width: 800, resizable: true });
      expect(t.servers).toEqual([{ name: "one" }]);
      expect(t.round).toBe(true);
      expect(t.written).toBe(
        'name = "x"\nn = 4611686018427387904\nat = 1970-01-01T00:00:00.0Z\nlist = [1, 2]\n\n[t]\nok = true\n',
      );
      expect(t.syntax).toStartWith("SyntaxError: TOML.parse: line 2, column 14:");
      expect(t.nil).toBe("TypeError: TOML.stringify: window.title is null, which TOML cannot represent");
      expect(t.hole).toStartWith("TypeError: TOML.stringify: list[1] is undefined");
      expect(t.notTable).toStartWith("TypeError");
    },
    T,
  );

  test(
    "skathi:network reads the connection from the OS",
    async () => {
      const app = await start();
      const n = JSON.parse(String(await app.eval("__SK_TEST_STD__.network()")));
      expect(n.sameOnline).toBe(true);
      expect(n.target).toBe(true);
      expect(["full", "limited", "portal", "none", "unknown"]).toContain(n.internet);
      expect(["ethernet", "wifi", "cellular", "other", "none", "unknown"]).toContain(n.type);
      for (const flag of [n.metered, n.roaming, n.overDataLimit]) expect([true, false, null]).toContain(flag);
      expect(n.wrapped).toBe(1);
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
