// `VN_SHUTDOWN_DELAY_MS` adds a quit hook that takes that long: a slow teardown, made on purpose.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { spawn, spawnSync } from "node:child_process";
import { readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stagedExe } from "../src/dist";
import { launch, REPO, type VnApp } from "../src/index";

const T = 60_000;
const FIXTURE = "packages/vn-test/fixtures/launch/src/main.ts";
const user32 = dlopen("user32.dll", {
  IsWindowVisible: { args: [FFIType.u64], returns: FFIType.i32 },
});
const extra: number[] = [];
const apps: VnApp[] = [];
afterAll(async () => {
  for (const pid of extra) spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
  for (const a of apps) await a.close();
  user32.close();
});

let n = 0;
async function start(env: Record<string, string> = {}) {
  const appName = `vn-test-close-${process.pid}-${n++}`;
  const app = await launch({
    app: FIXTURE,
    label: `close-${n}`,
    env: { VN_APP_NAME: appName, ...env },
  });
  apps.push(app);
  const shot = await app.send("screenshot", {
    path: join(tmpdir(), `vn-close-hwnd-${process.pid}-${n}.png`),
    backend: "render_to_image",
  });
  rmSync(shot.path, { force: true });
  await app.eval(`addEventListener("pagehide", () => console.log("[close-spec] pagehide")); "ok"`);
  return { app, appName, hwnd: BigInt(shot.hwnd) };
}

const alive = (pid: number) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};
async function until(what: () => boolean, ms: number): Promise<number> {
  const t0 = Date.now();
  while (!what()) {
    if (Date.now() - t0 > ms) return -1;
    await Bun.sleep(10);
  }
  return Date.now() - t0;
}
const logLines = (app: VnApp) => readFileSync(app.hostLog, "utf8").split(/\r?\n/);

describe("closing the app", () => {
  test(
    "appWindow.close(): the window is gone at once, pagehide fired once, the process ends with one line",
    async () => {
      const { app, hwnd } = await start();
      expect(user32.symbols.IsWindowVisible(hwnd)).not.toBe(0);
      // The reply may never come: the host is gone by then.
      void app.eval("__VN_WINDOW__.appWindow.close(); 1").catch(() => {});
      const hidden = await until(() => user32.symbols.IsWindowVisible(hwnd) === 0, 5_000);
      const ended = await until(() => !alive(app.pid), 10_000);
      expect(hidden).toBeGreaterThanOrEqual(0);
      expect(ended).toBeGreaterThanOrEqual(0);
      const lines = logLines(app);
      expect(lines.filter((l) => l.includes("[close-spec] pagehide")).length).toBe(1);
      const closed = lines.filter((l) => l.startsWith("[shutdown] closed. close(): hid 1 window(s)"));
      expect(closed.length).toBe(1);
      expect(closed[0]).toContain("quit hooks:");
    },
    T,
  );

  test(
    "a slow teardown runs with the window hidden, and the deadline ends it at 4 s",
    async () => {
      const { app, hwnd } = await start({ VN_SHUTDOWN_DELAY_MS: "20000" });
      const t0 = Date.now();
      void app.eval("close(); 1").catch(() => {});
      expect(await until(() => user32.symbols.IsWindowVisible(hwnd) === 0, 5_000)).toBeGreaterThanOrEqual(0);
      await Bun.sleep(1_000);
      expect(alive(app.pid)).toBe(true);
      expect(await until(() => !alive(app.pid), 15_000)).toBeGreaterThanOrEqual(0);
      const took = Date.now() - t0;
      expect(took).toBeGreaterThan(3_500);
      expect(took).toBeLessThan(8_000);
      const line = logLines(app).find((l) => l.includes("passed its 4 s deadline"));
      expect(line).toBeDefined();
      expect(line).toContain("close(): hid 1 window(s)");
      expect(line).toMatch(/VN_SHUTDOWN_DELAY_MS still running after \d+ ms/);
      expect(logLines(app).some((l) => l.startsWith("[shutdown] closed."))).toBe(false);
    },
    T,
  );

  test(
    "a launch while the app is closing waits for it to end and starts the app afresh",
    async () => {
      const { app, appName, hwnd } = await start({ VN_SINGLE_INSTANCE: "1", VN_SHUTDOWN_DELAY_MS: "1500" });
      void app.eval("close(); 1").catch(() => {});
      expect(await until(() => user32.symbols.IsWindowVisible(hwnd) === 0, 5_000)).toBeGreaterThanOrEqual(0);
      const second = spawn(stagedExe(), ["again"], {
        cwd: REPO,
        env: {
          ...(process.env as Record<string, string>),
          VN_DIST: app.slotDir,
          VN_APP_NAME: appName,
          VN_SINGLE_INSTANCE: "1",
          VN_WINDOW_FOCUS: "never",
          VN_MEDIA_CAPTURE: "deny",
          VN_OWNER_PID: String(process.pid),
        },
        detached: true,
        stdio: "ignore",
      });
      second.unref();
      if (second.pid) extra.push(second.pid);
      let exited: number | null = null;
      second.on("exit", (code) => (exited = code ?? -1));
      // It was not handed to the closing instance (that would exit 0 at once)…
      expect(await until(() => !alive(app.pid), 10_000)).toBeGreaterThanOrEqual(0);
      await Bun.sleep(2_000);
      expect(exited).toBe(null);
      expect(logLines(app).filter((l) => l.includes("arrived while this instance is closing")).length).toBe(1);
    },
    T,
  );
});
