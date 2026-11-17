// A user close (WM_CLOSE, as the title bar's X and Alt+F4 send) that the app cancels keeps the process;
// `hide()` takes the window off screen and `show()` brings it back.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launch } from "../src/index";

if (process.platform !== "win32") {
  describe("close-requested", () => {
    test.skip("Windows only: the user close is a posted WM_CLOSE", () => {});
  });
} else {
  const T = 60_000;
  const WM_CLOSE = 0x0010;
  const user32 = dlopen("user32.dll", {
    IsWindowVisible: { args: [FFIType.u64], returns: FFIType.i32 },
    PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
  });
  const app = await launch({
    app: "packages/test/fixtures/launch/src/main.ts",
    label: "close-requested",
    env: { SK_APP_NAME: `sk-test-close-requested-${process.pid}` },
  });
  afterAll(async () => {
    await app.close();
    user32.close();
  });

  const alive = (pid: number) => {
    try {
      process.kill(pid, 0);
      return true;
    } catch {
      return false;
    }
  };
  async function until(what: () => boolean | Promise<boolean>, ms: number): Promise<boolean> {
    const t0 = Date.now();
    while (!(await what())) {
      if (Date.now() - t0 > ms) return false;
      await Bun.sleep(25);
    }
    return true;
  }
  const visible = (hwnd: bigint) => user32.symbols.IsWindowVisible(hwnd) !== 0;

  test(
    "a cancelled close-requested hides to the background, show() brings the window back, an uncancelled one quits",
    async () => {
      const shot = await app.send("screenshot", {
        path: join(tmpdir(), `sk-close-requested-hwnd-${process.pid}.png`),
        backend: "render_to_image",
      });
      rmSync(shot.path, { force: true });
      const hwnd = BigInt(shot.hwnd);
      expect(visible(hwnd)).toBe(true);

      await app.eval(`(() => {
        const w = __SK_WINDOW__.appWindow;
        const s = (globalThis.__CLOSE__ = { asked: 0, keep: true, pagehide: 0 });
        addEventListener("pagehide", () => s.pagehide++);
        w.addEventListener("close-requested", (e) => {
          s.asked++;
          if (!s.keep) return;
          e.preventDefault();
          w.hide();
        });
        return "ok";
      })()`);

      user32.symbols.PostMessageW(hwnd, WM_CLOSE, 0n, 0n);
      expect(await until(() => !visible(hwnd), 5_000)).toBe(true);
      expect(await until(async () => (await app.eval("__SK_WINDOW__.appWindow.isVisible()")) === false, 5_000)).toBe(
        true,
      );
      await Bun.sleep(500);
      expect(alive(app.pid)).toBe(true);
      expect(JSON.parse(await app.eval("JSON.stringify(__CLOSE__)"))).toEqual({ asked: 1, keep: true, pagehide: 0 });
      expect(await app.eval("document.hidden")).toBe(true);

      await app.eval("__SK_WINDOW__.appWindow.show(); 'ok'");
      expect(await until(() => visible(hwnd), 5_000)).toBe(true);
      expect(await until(async () => (await app.eval("__SK_WINDOW__.appWindow.isVisible()")) === true, 5_000)).toBe(
        true,
      );
      expect(alive(app.pid)).toBe(true);

      await app.eval("__CLOSE__.keep = false; 'ok'");
      user32.symbols.PostMessageW(hwnd, WM_CLOSE, 0n, 0n);
      expect(await until(() => !alive(app.pid), 10_000)).toBe(true);
    },
    T,
  );
}
