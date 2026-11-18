// <input type=color>'s in-window picker. It is no element, so it is found in screenshots: its light
// surface on a dark blue page, 4 px under the input.

import { afterAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { type Bounds, launch, MACOS, readPng, SCREENSHOTS } from "../src/index";

const W = 460;
const app = await launch({
  app: "packages/test/fixtures/color-picker/src/main.ts",
  window: [W, 520],
  label: "color-picker",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

async function js<V>(source: string): Promise<V> {
  return JSON.parse(String(await app.eval(`JSON.stringify((() => { ${source} })())`))) as V;
}

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const box = async (selector: string): Promise<Bounds> => {
  const b = (await app.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

const shoot = async (path?: string) => {
  const shot = readPng(await app.screenshot(path));
  const k = shot.width / W;
  return { at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)) };
};
const near = (c: Rgb, [r, g, b]: Rgb) => Math.abs(c[0] - r) < 6 && Math.abs(c[1] - g) < 6 && Math.abs(c[2] - b) < 6;

/** The popover's top-left: under the input with a 4 px gap. Its content starts 13 px in (border, padding). */
const origin = (b: Bounds) => ({ x: b.x, y: b.y + b.h + 4 });
const SQUARE = { w: 216, h: 144 };
const shown = async (b: Bounds, surface: Rgb = [249, 249, 249]) => {
  const o = origin(b);
  return near((await shoot()).at(o.x + 5, o.y + 40), surface);
};
const opened = (b: Bounds, open: boolean) =>
  until(
    () => shown(b),
    (v) => v === open,
  );

const mouse = (x: number, y: number, kind: "down" | "move" | "up") => app.send("mouse", { x, y, kind });
const log = () => js<string[]>("return globalThis.log.slice()");
const value = (sel: string) => js<string>(`return document.querySelector(${JSON.stringify(sel)}).value`);
const since = async (n: number) => (await log()).slice(n);

describe.skipIf(!SCREENSHOTS)("the in-window picker", () => {
  test(
    "a click fires showpicker and opens it under the input; dragging in the square fires input, Escape restores",
    async () => {
      const b = await box("#plain");
      const before = (await log()).length;
      await app.locator("#plain").click({ real: true });
      expect(await opened(b, true)).toBe(true);
      expect(await since(before)).toEqual(["showpicker:plain:#ff0000"]);
      const o = origin(b);
      const sq = { x: o.x + 13, y: o.y + 13 };
      await mouse(sq.x + SQUARE.w / 2, sq.y + SQUARE.h / 2, "down");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0000",
        ),
      ).toBe("#804040");
      // Past the square's top-left corner: no saturation, full value.
      await mouse(sq.x - 40, sq.y - 40, "move");
      await mouse(sq.x - 40, sq.y - 40, "up");
      expect(
        await until(
          () => value("#plain"),
          (v) => v === "#ffffff",
        ),
      ).toBe("#ffffff");
      expect(await shown(b)).toBe(true);
      expect(await since(before + 1)).toEqual(["input:plain:#804040", "input:plain:#ffffff"]);
      await app.press("Escape");
      expect(await opened(b, false)).toBe(false);
      expect(await value("#plain")).toBe("#ff0000");
      expect(await since(before + 3)).toEqual(["input:plain:#ff0000"]);
    },
    T,
  );

  test(
    "the arrows move in the square, Tab moves to the hue slider, Enter commits with change",
    async () => {
      const b = await box("#plain");
      await app.locator("#plain").click({ real: true });
      expect(await opened(b, true)).toBe(true);
      const before = (await log()).length;
      await app.press("ArrowLeft");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0000",
        ),
      ).toBe("#ff0303");
      await app.press("Tab");
      await app.press("Shift+ArrowRight");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0303",
        ),
      ).toBe("#ff2d03");
      await app.press("Enter");
      expect(await opened(b, false)).toBe(false);
      expect(await since(before)).toEqual(["input:plain:#ff0303", "input:plain:#ff2d03", "change:plain:#ff2d03"]);
    },
    T,
  );

  // macOS CI: synthetic presses outside do not reach the window's hit test.
  test.skipIf(MACOS)(
    "showPicker() from a button opens it; a press outside commits and goes nowhere",
    async () => {
      await js("document.getElementById('plain').value = '#0000ff'; return 0");
      const b = await box("#plain");
      await app.locator("#pick").click({ real: true });
      expect(await opened(b, true)).toBe(true);
      const before = (await log()).length;
      const sq = { x: origin(b).x + 13, y: origin(b).y + 13 };
      await mouse(sq.x + SQUARE.w - 1, sq.y + 1, "down");
      await mouse(sq.x + SQUARE.w - 1, sq.y + 1, "up");
      await mouse(W - 10, 500, "down");
      await mouse(W - 10, 500, "up");
      expect(await opened(b, false)).toBe(false);
      const after = await since(before);
      expect(after.at(-1)).toMatch(/^change:plain:#/);
      expect(after.filter((e) => e.startsWith("change"))).toHaveLength(1);
    },
    T,
  );

  test(
    "a cancelled showpicker keeps it closed",
    async () => {
      const b = await box("#custom");
      const before = (await log()).length;
      await app.locator("#custom").click({ real: true });
      expect(await until(log, (l) => l.length > before)).toContain("showpicker:custom:#00ff00");
      await sleep(300);
      expect(await shown(b)).toBe(false);
    },
    T,
  );

  test(
    "with alpha it shows the alpha slider, and screenshots in light and dark",
    async () => {
      const b = await box("#alpha");
      expect(await value("#alpha")).toBe("color(srgb 0.2 0.4 0.8 / 0.501961)");
      const dir = process.env.SK_PICKER_SHOTS;
      for (const dark of [false, true]) {
        await js(`globalThis.setDark(${dark}); return 0`);
        await sleep(150);
        await app.locator("#alpha").click({ real: true });
        const surface: Rgb = dark ? [44, 44, 44] : [249, 249, 249];
        expect(
          await until(
            () => shown(b, surface),
            (v) => v,
          ),
        ).toBe(true);
        await app.press("Tab");
        await app.press("Tab");
        await sleep(150);
        if (dir) await app.screenshot(join(dir, `color-picker-${dark ? "dark" : "light"}.png`));
        await app.press("Escape");
        expect(
          await until(
            () => shown(b, surface),
            (v) => !v,
          ),
        ).toBe(false);
      }
      await js("globalThis.setDark(false); return 0");
    },
    T,
  );
});
