import { afterAll, describe, expect, test } from "bun:test";
import { type Bounds, launch, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/corner-shape/src/main.ts",
  window: [840, 300],
  label: "corner-shape",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Rgb = [number, number, number];

const box = async (selector: string): Promise<Bounds> => {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 840;
  /** The device pixel at CSS offset (dx, dy) from a box's top left. */
  const at = (b: Bounds, dx: number, dy: number): Rgb =>
    shot.at(Math.floor((b.x + dx) * k), Math.floor((b.y + dy) * k));
  return { k, at, shot };
};

const white = ([r, g, b]: Rgb) => r > 235 && g > 235 && b > 235;
const blue = ([r, g, b]: Rgb) => b > 220 && r < 60 && g < 110;
const red = ([r, g, b]: Rgb) => r > 220 && g < 40 && b < 40;
const green = ([r, g, b]: Rgb) => g > 170 && r < 40 && b < 40;

/** CSS offsets from the box's top left: one the corner cuts away, one it keeps. */
const corners: [string, [number, number], [number, number]][] = [
  ["#round", [4, 4], [12, 12]],
  ["#squircle", [1, 1], [6, 6]],
  ["#bevel", [8, 8], [18, 18]],
  ["#scoop", [12, 12], [26, 26]],
  ["#notch", [20, 20], [36, 20]],
  ["#superellipse-3", [0, 0], [5, 5]],
  ["#superellipse-neg", [8, 8], [26, 26]],
];

describe("corner-shape", () => {
  test("parses into the four computed longhands", async () => {
    const value = (id: string, side: string) =>
      app.eval(
        `getComputedStyle(document.getElementById(${JSON.stringify(id)})).getPropertyValue("corner-${side}-shape")`,
      );
    expect(await value("squircle", "top-left")).toBe("squircle");
    expect(await value("superellipse-3", "bottom-right")).toBe("superellipse(3)");
    expect(await value("mixed", "top-left")).toBe("bevel");
    expect(await value("mixed", "top-right")).toBe("scoop");
    expect(await value("mixed", "bottom-right")).toBe("notch");
    expect(await value("mixed", "bottom-left")).toBe("round");
  });

  test.skipIf(!SCREENSHOTS)(
    "each keyword cuts its corner and keeps the rest",
    async () => {
      await sleep(300);
      const s = await shoot();
      const wrong: string[] = [];
      for (const [selector, cut, kept] of corners) {
        const b = await box(selector);
        const outside = s.at(b, ...cut);
        const inside = s.at(b, ...kept);
        if (!white(outside)) wrong.push(`${selector} paints its cut corner ${outside}`);
        if (!blue(inside)) wrong.push(`${selector} leaves out ${kept}: ${inside}`);
      }
      const square = await box("#square");
      if (!blue(s.at(square, 0.5, 0.5))) wrong.push("#square cuts its corner");
      const round = await box("#round");
      if (!white(s.at(round, 6, 6))) wrong.push("#round should cut where #squircle fills");
      expect(wrong).toEqual([]);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "halfway from scoop to round is bevel",
    async () => {
      const [anim, bevel] = [await box("#anim"), await box("#bevel")];
      const s = await shoot();
      let off = 0;
      for (let y = 0; y < 80; y++)
        for (let x = 0; x < 80; x++) {
          const a = s.at(anim, x, y);
          const b = s.at(bevel, x, y);
          if (a.some((v, i) => Math.abs(v - b[i]!) > 24)) off++;
        }
      expect(off).toBeLessThan(8);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "overflow clips to the shape inside the border, and a spread shadow follows it",
    async () => {
      const [clip, shadow] = [await box("#clip"), await box("#shadow")];
      const s = await shoot();
      expect(white(s.at(clip, 8, 8))).toBe(true);
      expect(red(s.at(clip, 26, 26))).toBe(true);
      expect(green(s.at(shadow, 13, 13))).toBe(true);
      expect(white(s.at(shadow, 4, 4))).toBe(true);
      expect(blue(s.at(shadow, 20, 20))).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "the corners of one box take their own shapes",
    async () => {
      const b = await box("#mixed");
      const s = await shoot();
      expect(white(s.at(b, 8, 8))).toBe(true);
      expect(white(s.at(b, 80 - 12, 12))).toBe(true);
      expect(white(s.at(b, 80 - 20, 80 - 20))).toBe(true);
      expect(white(s.at(b, 3, 80 - 3))).toBe(true);
      expect(blue(s.at(b, 80 - 26, 26))).toBe(true);
      expect(blue(s.at(b, 12, 80 - 12))).toBe(true);
    },
    T,
  );
});
