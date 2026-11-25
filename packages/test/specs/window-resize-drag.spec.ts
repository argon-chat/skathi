// `appWindow.startResizeDragging()` on Windows, from strips just inside the edges of a hidden-title-bar window. The
// system's size loop follows the real cursor, which a spec may not move: `SK_WINDOW_MOVE=fake` stands in a one-second
// loop that moves the asked edge out by 40 px once, through `WM_SIZING` and `SetWindowPos`, and pumps messages.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { readFileSync } from "node:fs";
import { launch, WINDOWS, type SkApp } from "../src/index";

const DRAG = 40;
const WMSZ = { left: 1, right: 2, top: 3, bottom: 6, bottomRight: 8 } as const;

const opts = (app: string, label: string) => ({
  app,
  window: [360, 220] as [number, number],
  scale: 1,
  label,
  env: { SK_WINDOW_MOVE: "fake" },
});
const app = WINDOWS
  ? await launch(opts("packages/test/fixtures/window-resize-drag/src/main.ts", "window-resize"))
  : null;
const fixed = WINDOWS
  ? await launch(opts("packages/test/fixtures/window-resize-drag-fixed/src/main.ts", "window-resize-fixed"))
  : null;
const user32 = WINDOWS
  ? dlopen("user32.dll", {
      PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
    })
  : null;
afterAll(async () => {
  user32?.close();
  await app?.close();
  await fixed?.close();
});

const T = 20_000;
const LOOP = /\[window-resize\] fake size loop (\d+) (\d+)\.\.(\d+)/g;
type Loop = { edge: number; from: number; to: number };
const loops = (on: SkApp): Loop[] =>
  [...readFileSync(on.hostLog, "utf8").matchAll(LOOP)].map((m) => ({
    edge: Number(m[1]),
    from: Number(m[2]),
    to: Number(m[3]),
  }));

async function nextLoop(on: SkApp, seen: number, within = 8000): Promise<Loop | null> {
  const end = Date.now() + within;
  while (Date.now() < end) {
    const all = loops(on);
    if (all.length > seen) return all[seen]!;
    await Bun.sleep(50);
  }
  return null;
}

type Rect = { x: number; y: number; width: number; height: number };
const bounds = async (on: SkApp): Promise<Rect> =>
  JSON.parse(await on.eval("__SK_WINDOW__.refresh(); JSON.stringify(__SK_WINDOW__.appWindow.getContentBounds())"));

async function settled(on: SkApp, differs: (r: Rect) => boolean): Promise<Rect> {
  const end = Date.now() + 3000;
  let r = await bounds(on);
  while (!differs(r) && Date.now() < end) {
    await Bun.sleep(50);
    r = await bounds(on);
  }
  return r;
}

async function press(on: SkApp, selector: string): Promise<Loop | null> {
  const seen = loops(on).length;
  await on.send("mouse", { selector, kind: "down" });
  const loop = await nextLoop(on, seen, on === fixed ? 1500 : 8000);
  await on.send("mouse", { selector, kind: "up" });
  return loop;
}

async function maximized(want: boolean) {
  const end = Date.now() + 5000;
  while ((await app!.eval("__SK_WINDOW__.appWindow.isMaximized()")) !== want) {
    if (Date.now() > end) throw new Error(`isMaximized() never became ${want}`);
    await Bun.sleep(50);
  }
  await Bun.sleep(200);
}

async function restore(to: Rect) {
  await app!.eval(`__SK_WINDOW__.appWindow.setContentBounds(${JSON.stringify(to)}); 'ok'`);
  await settled(app!, (r) => r.width === to.width && r.height === to.height && r.x === to.x && r.y === to.y);
}

describe("appWindow.startResizeDragging()", () => {
  for (const [selector, edge, grow] of [
    ["#right", WMSZ.right, { width: DRAG, height: 0, x: 0, y: 0 }],
    ["#left", WMSZ.left, { width: DRAG, height: 0, x: -DRAG, y: 0 }],
    ["#bottom", WMSZ.bottom, { width: 0, height: DRAG, x: 0, y: 0 }],
    ["#top", WMSZ.top, { width: 0, height: DRAG, x: 0, y: -DRAG }],
    ["#corner", WMSZ.bottomRight, { width: DRAG, height: DRAG, x: 0, y: 0 }],
  ] as const) {
    test.skipIf(!WINDOWS)(
      `${selector}: the size loop runs on its edge and the window grows by the drag`,
      async () => {
        const before = await bounds(app!);
        const loop = await press(app!, selector);
        expect(loop?.edge).toBe(edge);
        const after = await settled(app!, (r) => r.width !== before.width || r.height !== before.height);
        expect(after.width - before.width).toBe(grow.width);
        expect(after.height - before.height).toBe(grow.height);
        expect(after.x - before.x).toBe(grow.x);
        expect(after.y - before.y).toBe(grow.y);
        await restore(before);
      },
      T,
    );
  }

  test.skipIf(!WINDOWS)(
    "from a native press: the loop starts after the input dispatch and the page runs during it",
    async () => {
      const before = await bounds(app!);
      const hwnd = await app!.hwnd();
      const at = JSON.parse(
        await app!.eval(`JSON.stringify((() => {
          const r = document.getElementById("right").getBoundingClientRect();
          return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
        })())`),
      ) as [number, number];
      const point = BigInt((at[1] << 16) | at[0]);
      const seen = loops(app!).length;
      expect(user32!.symbols.PostMessageW(hwnd, 0x0200, 0n, point)).toBe(1); // WM_MOUSEMOVE
      expect(user32!.symbols.PostMessageW(hwnd, 0x0201, 1n, point)).toBe(1); // WM_LBUTTONDOWN, MK_LBUTTON
      const loop = await nextLoop(app!, seen);
      user32!.symbols.PostMessageW(hwnd, 0x0202, 0n, point); // WM_LBUTTONUP
      expect(loop?.edge).toBe(WMSZ.right);
      expect(loop!.to - loop!.from).toBeGreaterThanOrEqual(900);
      const seenLive = JSON.parse(
        await app!.eval(`JSON.stringify(__SK_TEST_RESIZE__.between(${loop!.from}, ${loop!.to}))`),
      );
      expect(seenLive.ticks).toBeGreaterThan(20);
      expect(seenLive.frames).toBeGreaterThan(10);
      await restore(before);
    },
    T,
  );

  test.skipIf(!WINDOWS)(
    "a maximized window does not start a resize",
    async () => {
      await app!.eval("__sk_window_verb('maximize'); 'ok'");
      await maximized(true);
      const before = await bounds(app!);
      expect(await press(app!, "#right")).toBeNull();
      expect(await bounds(app!)).toEqual(before);
      await app!.eval("__sk_window_verb('unmaximize'); 'ok'");
      await maximized(false);
    },
    T,
  );

  test.skipIf(!WINDOWS)(
    "resizable: false: no loop and no change, from a strip or a corner",
    async () => {
      const before = await bounds(fixed!);
      expect(await press(fixed!, "#right")).toBeNull();
      expect(await press(fixed!, "#corner")).toBeNull();
      expect(await bounds(fixed!)).toEqual(before);
    },
    T,
  );
});
