// `setTitleBarOverlay()`'s host-drawn caption buttons over a page whose positioned content fills
// the window: a WebGPU canvas, a 2D canvas and an <iframe>. The buttons are seen on screen and
// answer the OS hit test.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType, ptr } from "bun:ffi";
import { join } from "node:path";
import { launch, ON_SCREEN, readPng, WORK_DIR } from "../src/index";

if (process.platform !== "win32") {
  describe("caption overlay", () => {
    test.skip("Windows only: the host draws caption buttons on Windows; macOS has its traffic lights", () => {});
  });
} else {
  const user32 = dlopen("user32.dll", {
    SetProcessDpiAwarenessContext: { args: [FFIType.i64], returns: FFIType.i32 },
    SendMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i64 },
    ClientToScreen: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
  });
  user32.symbols.SetProcessDpiAwarenessContext(-4n);
  const WM_NCHITTEST = 0x0084;
  const HTMINBUTTON = 8;
  const HTMAXBUTTON = 9;
  const HTCLOSE = 20;

  const app = await launch({ app: "packages/test/fixtures/caption-canvas/src/main.ts", label: "caption-overlay" });
  afterAll(async () => {
    await app.close();
    user32.close();
  });
  const hwnd = await app.hwnd();

  const BUTTON = 46;
  const BAR = 40;
  const T = 30_000;

  async function until(what: string, ok: () => Promise<boolean>) {
    const deadline = Date.now() + 10_000;
    while (!(await ok())) {
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}\n${await app.tailLogs()}`);
      await Bun.sleep(50);
    }
  }

  function hitTest(x: number, y: number, scale: number): number {
    const pt = new Int32Array(2);
    user32.symbols.ClientToScreen(hwnd, ptr(pt));
    const sx = Math.round(pt[0]! + x * scale) & 0xffff;
    const sy = Math.round(pt[1]! + y * scale) & 0xffff;
    return Number(user32.symbols.SendMessageW(hwnd, WM_NCHITTEST, 0n, BigInt(sx | (sy << 16))));
  }

  test(
    "hiding the caption buttons removes their Win32 hitboxes and showing restores their current height",
    async () => {
      const scale = Number(await app.eval("devicePixelRatio"));
      const width = Number(await app.eval("innerWidth"));
      const query = "__SK_WINDOW__.appWindow.getWindowButtonsRect()";
      await app.eval("__SK_WINDOW__.appWindow.setWindowButtonsVisible(false); true");
      try {
        await until(
          "hidden caption buttons",
          async () => (await app.eval(query)) === null && hitTest(width - 20, 20, scale) !== HTCLOSE,
        );
        await app.eval(
          "__SK_WINDOW__.appWindow.setTitleBarOverlay({height:52}); __SK_WINDOW__.appWindow.setWindowButtonsVisible(true); true",
        );
        await until("caption hitbox with new height", async () => {
          const rect = await app.eval(query);
          return (
            rect?.x === width - 3 * BUTTON &&
            rect.width === 3 * BUTTON &&
            rect.height === 52 &&
            hitTest(width - 20, 26, scale) === HTCLOSE
          );
        });
        await app.eval("__SK_WINDOW__.appWindow.setWindowButtonsVisible(false,{fade:180}); true");
        await until(
          "the end of the caption fade",
          async () => (await app.eval(query)) === null && hitTest(width - 20, 26, scale) !== HTCLOSE,
        );
        await app.eval("__SK_WINDOW__.appWindow.setWindowButtonsVisible(true,{fade:180}); true");
        await until("the returning caption buttons", async () => (await app.eval(query))?.width === 3 * BUTTON);
        await app.eval(
          "__SK_WINDOW__.appWindow.setWindowButtonsVisible(false,{fade:180}); __SK_WINDOW__.appWindow.setWindowButtonsVisible(true); true",
        );
        await Bun.sleep(210);
        expect(hitTest(width - 20, 26, scale)).toBe(HTCLOSE);
      } finally {
        await app.eval(
          "__SK_WINDOW__.appWindow.setTitleBarOverlay({height:40}); __SK_WINDOW__.appWindow.setWindowButtonsVisible(true); true",
        );
      }
    },
    T,
  );

  for (const kind of ["webgpu", "2d", "iframe"] as const) {
    test(
      `the buttons show and answer the hit test above a full-window ${kind}`,
      async () => {
        await app.eval(`__caption.use(${JSON.stringify(kind)}); "ok"`);
        await until(`the ${kind} to present`, async () => {
          const s: { frames: number; error: string } = await app.eval("__caption.state");
          if (s.error) throw new Error(s.error);
          return s.frames > 0;
        });
        const width = Number(await app.eval("innerWidth"));

        let scale = 1;
        let at = (_x: number, _y: number): [number, number, number] => [0, 0, 0];
        const filled = () => at(width / 2, 200).join() === [0x20, 0x50, 0xc0].join();
        await until("the composed frame", async () => {
          const shot = await app.send("screenshot", {
            path: join(WORK_DIR, `caption-overlay-${kind}.png`),
            backend: ON_SCREEN,
          });
          const png = readPng(shot.path);
          scale = shot.scale;
          at = (x, y) => png.at(Math.round(x * shot.scale + shot.clientX), Math.round(y * shot.scale + shot.clientY));
          return filled();
        }).catch(() => {});
        expect(at(width / 2, 200)).toEqual([0x20, 0x50, 0xc0]);

        for (const [i, area] of [HTMINBUTTON, HTMAXBUTTON, HTCLOSE].entries()) {
          const left = width - BUTTON * (3 - i);
          let glyph = 0;
          for (let y = 4; y < BAR - 4; y++)
            for (let x = left + 4; x < left + BUTTON - 4; x++) if (at(x, y).every((c) => c > 0xc0)) glyph++;
          expect({ button: i, glyph: glyph > 0 }).toEqual({ button: i, glyph: true });
          expect(hitTest(left + BUTTON / 2, BAR / 2, scale)).toBe(area);
        }
      },
      T,
    );
  }
}
