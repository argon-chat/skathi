// `window.x`/`y`/`center`/`monitor` in sk.config.json: the main window is created in place,
// measured from its monitor's work area and kept inside it.

import { afterAll, describe, expect, test } from "bun:test";
import { dlopen, FFIType, ptr } from "bun:ffi";
import { launch, WINDOWS, type SkApp } from "../src/index";

const T = 60_000;

type Rect = { x: number; y: number; width: number; height: number };
type Placed = { content: Rect; outer: Rect; work: Rect };

const placed = async (app: SkApp): Promise<Placed> =>
  JSON.parse(
    await app.eval(`(() => {
      __SK_WINDOW__.refresh();
      const w = __SK_WINDOW__.appWindow;
      return JSON.stringify({
        content: w.getContentBounds(),
        outer: w.getBounds(),
        work: { x: screen.availLeft, y: screen.availTop, width: screen.availWidth, height: screen.availHeight },
      });
    })()`),
  );

const open: SkApp[] = [];
afterAll(async () => {
  await Promise.all(open.map((a) => a.close()));
});
const start = async (fixture: string): Promise<SkApp> => {
  const app = await launch({
    app: `packages/test/fixtures/${fixture}/src/main.ts`,
    label: fixture,
  });
  open.push(app);
  return app;
};

// Linux CI runs one X screen with no work area of its own; macOS reads a screen-relative origin.
describe.skipIf(!WINDOWS)("main window placement", () => {
  test(
    "x and y place the window from the top-left of the work area",
    async () => {
      const app = await start("window-place");
      const p = await placed(app);
      expect(Math.round(p.content.x - p.work.x)).toBe(40);
      expect(Math.round(p.content.y - p.work.y)).toBe(30);
      expect(Math.round(p.content.width)).toBe(360);
      expect(Math.round(p.content.height)).toBe(240);
    },
    T,
  );

  test(
    "a position past the work area is clamped into it, OS caption included",
    async () => {
      const app = await start("window-place-clamp");
      const p = await placed(app);
      expect(Math.abs(p.content.x + p.content.width - (p.work.x + p.work.width))).toBeLessThanOrEqual(1);
      expect(p.content.y).toBeGreaterThan(p.work.y);
      expect(p.outer.y).toBeGreaterThanOrEqual(p.work.y - 1);
    },
    T,
  );

  test(
    'monitor "cursor" opens on the monitor under the pointer, centred on it',
    async () => {
      const user32 = dlopen("user32.dll", {
        GetCursorPos: { args: [FFIType.ptr], returns: FFIType.i32 },
        MonitorFromPoint: {
          args: [FFIType.u64, FFIType.u32],
          returns: FFIType.u64,
        },
        MonitorFromWindow: {
          args: [FFIType.u64, FFIType.u32],
          returns: FFIType.u64,
        },
      });
      try {
        const point = new Int32Array(2);
        user32.symbols.GetCursorPos(ptr(point));
        const packed = (BigInt(point[1]!) << 32n) | (BigInt(point[0]!) & 0xffffffffn);
        const under = user32.symbols.MonitorFromPoint(packed, 2);
        const app = await start("window-place-cursor");
        expect(user32.symbols.MonitorFromWindow(await app.hwnd(), 2)).toBe(under);
        const p = await placed(app);
        const centre = (a: Rect) => [a.x + a.width / 2, a.y + a.height / 2];
        const [cx, cy] = centre(p.content);
        const [wx, wy] = centre(p.work);
        expect(Math.abs(cx! - wx!)).toBeLessThanOrEqual(1);
        expect(Math.abs(cy! - wy!)).toBeLessThanOrEqual(1);
      } finally {
        user32.close();
      }
    },
    T,
  );
});
