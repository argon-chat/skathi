import { afterAll, describe, expect, test } from "bun:test";
import { launch, LINUX } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/call-timer/src/main.ts",
  window: [320, 200],
  label: "window-geometry",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(what: string, probe: () => Promise<T>, ms = 8000): Promise<NonNullable<T>> {
  const deadline = Date.now() + ms;
  let last: unknown;
  for (;;) {
    const v = await probe();
    if (v) return v;
    last = v;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
    await sleep(50);
  }
}

type Outer = { x: number; y: number; width: number; height: number };
const outer = async (): Promise<Outer> =>
  JSON.parse(await app.eval(`JSON.stringify(__SK_WINDOW__.refresh() && __SK_WINDOW__.appWindow.getBounds())`));

describe("main window geometry", () => {
  test(
    "a window `resize` handler sees the new size through matchMedia, and appWindow hears it too",
    async () => {
      await sleep(300);
      await app.eval(`(() => {
        globalThis.__resizes = [];
        addEventListener("resize", () => {
          const w = innerWidth;
          __resizes.push({
            on: "window",
            width: w,
            matches: matchMedia("(min-width: " + w + "px)").matches && !matchMedia("(min-width: " + (w + 1) + "px)").matches,
          });
        });
        __SK_WINDOW__.appWindow.addEventListener("resize", () => __resizes.push({ on: "appWindow", width: innerWidth }));
        __SK_WINDOW__.appWindow.setContentBounds({ width: 360 });
        return "ok";
      })()`);
      const seen = await until("both resize events", async () => {
        const list: { on: string; width: number; matches?: boolean }[] = JSON.parse(
          await app.eval("JSON.stringify(__resizes)"),
        );
        return list.some((e) => e.on === "window" && e.width === 360) && list.some((e) => e.on === "appWindow")
          ? list
          : null;
      });
      expect(seen.filter((e) => e.on === "window").every((e) => e.matches)).toBe(true);
    },
    T,
  );

  test(
    "resizeTo() sets the OUTER size, as on the web",
    async () => {
      await app.eval(`resizeTo(400, 260); "ok"`);
      const b = await until("the outer size to follow", async () => {
        const r = await outer();
        return Math.round(r.width) === 400 && Math.round(r.height) === 260 ? r : null;
      });
      expect(Math.round(b.width)).toBe(400);
      await app.eval(`resizeBy(0, 0); "ok"`);
      await sleep(300);
      const after = await outer();
      expect(Math.round(after.width)).toBe(400);
      expect(Math.round(after.height)).toBe(260);
    },
    T,
  );

  // Linux CI: the outer rect leaves out openbox's frame, so center() lands 20 px off.
  test.skipIf(LINUX)(
    "center() centres the outer rect in the work area at any scale",
    async () => {
      await app.eval(`__SK_WINDOW__.appWindow.center(); "ok"`);
      await sleep(300);
      const r = await outer();
      const s = JSON.parse(await app.eval("JSON.stringify(__SK_WINDOW__.state().screen)"));
      expect(Math.abs(r.x - (s.availLeft + (s.availWidth - r.width) / 2))).toBeLessThanOrEqual(1);
      expect(Math.abs(r.y - (s.availTop + (s.availHeight - r.height) / 2))).toBeLessThanOrEqual(1);
    },
    T,
  );

  test(
    "nothing in the run was a JS error",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
