// `SK_SHUTDOWN_DELAY_MS` adds a quit hook that takes that long: a slow teardown, made on purpose.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { stagedExe } from "../src/dist";
import { killTree, launch, REPO, type SkApp, WINDOWS } from "../src/index";

const T = 60_000;
const FIXTURE = "packages/test/fixtures/launch/src/main.ts";
const user32 =
  process.platform === "win32"
    ? dlopen("user32.dll", { IsWindowVisible: { args: [FFIType.u64], returns: FFIType.i32 } })
    : null;
const extra: ChildProcess[] = [];
const apps: SkApp[] = [];
afterAll(async () => {
  for (const child of extra) killTree(child);
  for (const a of apps) await a.close();
  user32?.close();
});

let n = 0;
async function start(env: Record<string, string> = {}) {
  const appName = `sk-test-close-${process.pid}-${n++}`;
  const app = await launch({
    app: FIXTURE,
    label: `close-${n}`,
    env: { SK_APP_NAME: appName, ...env },
  });
  apps.push(app);
  await app.eval(`addEventListener("pagehide", () => console.log("[close-spec] pagehide")); "ok"`);
  return { app, appName, hwnd: await app.hwnd() };
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
/** How long the window took to hide; 0 where only Win32 can tell. */
const hidden = (hwnd: bigint) => (user32 ? until(() => user32.symbols.IsWindowVisible(hwnd) === 0, 5_000) : 0);
const logLines = (app: SkApp) => readFileSync(app.hostLog, "utf8").split(/\r?\n/);

describe("closing the app", () => {
  test(
    "appWindow.close(): the window is gone at once, pagehide fired once, the process ends with one line",
    async () => {
      const { app, hwnd } = await start();
      if (user32) expect(user32.symbols.IsWindowVisible(hwnd)).not.toBe(0);
      // The reply may never come: the host is gone by then.
      void app.eval("__SK_WINDOW__.appWindow.close(); 1").catch(() => {});
      const hid = await hidden(hwnd);
      const ended = await until(() => !alive(app.pid), 10_000);
      expect(hid).toBeGreaterThanOrEqual(0);
      expect(ended).toBeGreaterThanOrEqual(0);
      const lines = logLines(app);
      expect(lines.filter((l) => l.includes("[close-spec] pagehide")).length).toBe(1);
      // Only the Windows shutdown hides the window itself; elsewhere it reports none hidden.
      const hiddenByShutdown = process.platform === "win32" ? 1 : 0;
      const closed = lines.filter((l) => l.startsWith(`[shutdown] closed. close(): hid ${hiddenByShutdown} window(s)`));
      expect(closed.length).toBe(1);
      expect(closed[0]).toContain("quit hooks:");
    },
    T,
  );

  test(
    "a slow teardown runs with the window hidden, and the deadline ends it at 4 s",
    async () => {
      const { app, hwnd } = await start({ SK_SHUTDOWN_DELAY_MS: "20000" });
      const t0 = Date.now();
      void app.eval("close(); 1").catch(() => {});
      expect(await hidden(hwnd)).toBeGreaterThanOrEqual(0);
      await Bun.sleep(1_000);
      expect(alive(app.pid)).toBe(true);
      expect(await until(() => !alive(app.pid), 15_000)).toBeGreaterThanOrEqual(0);
      const took = Date.now() - t0;
      expect(took).toBeGreaterThan(3_500);
      expect(took).toBeLessThan(8_000);
      const line = logLines(app).find((l) => l.includes("passed its 4 s deadline"));
      expect(line).toBeDefined();
      expect(logLines(app).some((l) => l.startsWith("[shutdown] closed."))).toBe(false);
    },
    T,
  );

  // Off Windows the second launch exits instead of waiting for the closing instance.
  test.skipIf(!WINDOWS)(
    "a launch while the app is closing waits for it to end and starts the app afresh",
    async () => {
      const { app, appName, hwnd } = await start({ SK_SINGLE_INSTANCE: "1", SK_SHUTDOWN_DELAY_MS: "1500" });
      void app.eval("close(); 1").catch(() => {});
      expect(await hidden(hwnd)).toBeGreaterThanOrEqual(0);
      const second = spawn(stagedExe(), ["again"], {
        cwd: REPO,
        env: {
          ...(process.env as Record<string, string>),
          SK_DIST: app.slotDir,
          SK_APP_NAME: appName,
          SK_SINGLE_INSTANCE: "1",
          SK_WINDOW_FOCUS: "never",
          SK_MEDIA_CAPTURE: "deny",
          SK_OWNER_PID: String(process.pid),
        },
        detached: true,
        stdio: "ignore",
      });
      second.unref();
      extra.push(second);
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
