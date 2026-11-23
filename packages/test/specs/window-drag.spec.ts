// `appWindow.startDragging()` on Windows: the system's move loop is modal, and must start outside the input dispatch or
// frame that asked for it, so timers, frames and devtools keep running while the window is dragged.
// `SK_WINDOW_MOVE=fake` stands in a one-second loop that pumps messages as the system's does, without the real cursor.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { readFileSync } from "node:fs";
import { launch, WINDOWS } from "../src/index";

const app = WINDOWS
  ? await launch({
      app: "packages/test/fixtures/window-drag/src/main.ts",
      window: [360, 220],
      scale: 1,
      label: "window-drag",
      env: { SK_WINDOW_MOVE: "fake" },
    })
  : null;
const user32 = WINDOWS
  ? dlopen("user32.dll", {
      PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
    })
  : null;
afterAll(async () => {
  user32?.close();
  await app?.close();
});

const T = 20_000;
const LOOP = /\[window-move\] fake move loop (\d+)\.\.(\d+)/g;
const loops = () => [...readFileSync(app!.hostLog, "utf8").matchAll(LOOP)].map((m) => [Number(m[1]), Number(m[2])]);

async function nextLoop(seen: number): Promise<[number, number]> {
  const end = Date.now() + 8000;
  while (Date.now() < end) {
    const all = loops();
    if (all.length > seen) return all[seen] as [number, number];
    await Bun.sleep(50);
  }
  throw new Error("no window move loop ran");
}

async function expectLivePage([from, to]: [number, number]) {
  expect(to - from).toBeGreaterThanOrEqual(900);
  const seen = JSON.parse(await app!.eval(`JSON.stringify(__SK_TEST_DRAG__.between(${from}, ${to}))`));
  // A 10 ms interval and rAF over one second: a frozen page has none of either.
  expect(seen.ticks).toBeGreaterThan(20);
  expect(seen.frames).toBeGreaterThan(10);
}

describe("appWindow.startDragging()", () => {
  test.skipIf(!WINDOWS)(
    "from a press the frame pump dispatches: devtools answer and the page runs during the move",
    async () => {
      const seen = loops().length;
      await app!.send("mouse", { selector: "#bar", kind: "down" });
      const asked = Number(await app!.eval("Date.now()"));
      const loop = await nextLoop(seen);
      await app!.send("mouse", { selector: "#bar", kind: "up" });
      expect(asked).toBeGreaterThan(loop[0]);
      expect(asked).toBeLessThan(loop[1]);
      await expectLivePage(loop);
    },
    T,
  );

  test.skipIf(!WINDOWS)(
    "from a native press: the page runs during the move",
    async () => {
      const hwnd = await app!.hwnd();
      const at = JSON.parse(
        await app!.eval(`JSON.stringify((() => {
          const r = document.getElementById("bar").getBoundingClientRect();
          return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
        })())`),
      ) as [number, number];
      const point = BigInt((at[1] << 16) | at[0]);
      const seen = loops().length;
      const presses = Number(await app!.eval("__SK_TEST_DRAG__.presses"));
      expect(user32!.symbols.PostMessageW(hwnd, 0x0200, 0n, point)).toBe(1); // WM_MOUSEMOVE
      expect(user32!.symbols.PostMessageW(hwnd, 0x0201, 1n, point)).toBe(1); // WM_LBUTTONDOWN, MK_LBUTTON
      const loop = await nextLoop(seen).catch(async (e) => {
        throw new Error(
          `${e.message}; presses ${presses} -> ${await app!.eval("__SK_TEST_DRAG__.presses")}, at ${at}, hwnd ${hwnd}`,
        );
      });
      user32!.symbols.PostMessageW(hwnd, 0x0202, 0n, point); // WM_LBUTTONUP
      await expectLivePage(loop);
    },
    T,
  );
});
