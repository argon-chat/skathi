// A mouse-wheel notch (line deltas, as gpui_windows reports WM_MOUSEWHEEL): WheelEvent says what
// Chromium says, and the app's own scrolling moves by that times `wheelScrollScale`. The fixture's
// sk.config.json sets the scale to 2.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, WINDOWS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/wheel-notch/src/main.ts",
  window: [640, 420],
  label: "wheel-notch",
  env: { SK_WINDOW_FOCUS: "never" },
});
afterAll(() => app.close());

const T = 30_000;
const W = "globalThis.__SK_TEST_WHEEL__";
const BOXES = ["#scroller", "#area", "#ed"] as const;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Wheel = { deltaX: number; deltaY: number; deltaMode: number };

const areaTop = async () => Number((await app.send("scroll", { selector: "#area", dy: 0 })).scrollTop);

async function positions(): Promise<Record<string, number>> {
  return {
    "#scroller": Number(await app.eval("document.querySelector('#scroller').scrollTop")),
    "#area": await areaTop(),
    "#ed": Number(await app.eval("document.querySelector('#ed').scrollTop")),
  };
}

/** One notch down, at the system default of 3 lines, over each box from the top; how far each moved, and the `wheel` events. */
async function notchEach(): Promise<{ moved: Record<string, number>; wheels: Wheel[] }> {
  await app.eval(
    `(() => { document.querySelector('#scroller').scrollTop = 0; document.querySelector('#ed').scrollTop = 0; document.querySelector('#area').setSelectionRange(0, 0); ${W}.wheels.length = 0; return 0 })()`,
  );
  await sleep(100);
  const area = await areaTop();
  if (area) await app.send("scroll", { selector: "#area", dy: -area });
  await sleep(100);
  for (const selector of BOXES) await app.send("wheel", { selector, dy: 3, lines: true });
  await sleep(100);
  return { moved: await positions(), wheels: (await app.eval(`${W}.wheels`)) as Wheel[] };
}

const each = (n: number) => Object.fromEntries(BOXES.map((b) => [b, n]));
const notchEvents = Array(BOXES.length).fill({ deltaX: 0, deltaY: 100, deltaMode: 0 });

describe.skipIf(!WINDOWS)("a wheel notch", () => {
  test(
    "moves every kind of box twice as far under the config's scale 2, and the event still says 100",
    async () => {
      expect(await app.eval(`${W}.app.wheelScrollScale`)).toBe(2);
      const { moved, wheels } = await notchEach();
      expect(wheels).toEqual(notchEvents);
      expect(moved).toEqual(each(200));
    },
    T,
  );

  test(
    "at scale 1 set at run time is deltaY 100 and moves every kind of box by 100",
    async () => {
      await app.eval(`(${W}.app.wheelScrollScale = 1, 0)`);
      const { moved, wheels } = await notchEach();
      expect(wheels).toEqual(notchEvents);
      expect(moved).toEqual(each(100));
    },
    T,
  );

  test(
    "the setter refuses what is not a number above 0 and clamps the rest",
    async () => {
      const refused = await app.eval(
        `[0, -1, NaN, Infinity, "2", null].map((v) => { try { ${W}.app.wheelScrollScale = v; return null } catch (e) { return e.name } })`,
      );
      expect(refused).toEqual(Array(6).fill("TypeError"));
      expect(await app.eval(`${W}.app.wheelScrollScale`)).toBe(1);
      expect(await app.eval(`(${W}.app.wheelScrollScale = 1000, ${W}.app.wheelScrollScale)`)).toBe(10);
      await app.eval(`(${W}.app.wheelScrollScale = 1, 0)`);
    },
    T,
  );
});
