// An outer `box-shadow` is cut out of the ROUNDED border box, so a box with no background shows the
// shadow in its corners outside the radius; an inset one is clipped to that rounded box.

import { afterAll, describe, expect, test } from "bun:test";
import { type Bounds, launch, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/box-shadow/src/main.ts",
  window: [640, 520],
  label: "box-shadow",
});
afterAll(() => app.close());

const T = 30_000;
type Rgb = [number, number, number];

const box = async (id: string): Promise<Bounds> => {
  const b = (await app.query(`#${id}`)).nodes[0]?.bounds;
  if (!b) throw new Error(`#${id} has no box`);
  return b;
};

const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const white = ([r, g, b]: Rgb) => r > 235 && g > 235 && b > 235;
const tinted = ([r, g, b]: Rgb) => r > 200 && g < 235 && b < 235;

/** `[dx, dy, test]` offsets from the box's top left (negative: outside it). */
const cases: [string, [number, number, (c: Rgb) => boolean][]][] = [
  [
    "ring",
    [
      [4, 4, red],
      [60, 40, white],
      [-1, -1, red],
      [-12, -12, white],
      [-8, 40, red],
    ],
  ],
  [
    "blur",
    [
      [4, 4, tinted],
      [60, 40, white],
    ],
  ],
  [
    "mixed",
    [
      [2, 2, white],
      [-8, -8, red],
      [112, 10, red],
      [4, 76, red],
      [118, 78, red],
      [60, 40, white],
    ],
  ],
  [
    "ellipse",
    [
      [4, 4, red],
      [60, 40, white],
    ],
  ],
  [
    "square",
    [
      [-8, -8, red],
      [2, 2, white],
    ],
  ],
  [
    "inset",
    [
      [4, 4, white],
      [2, 40, red],
      [60, 6, red],
      [60, 40, white],
      [-4, 40, white],
    ],
  ],
  [
    "clipped",
    [
      [4, 4, red],
      [60, 40, white],
    ],
  ],
];

describe("box-shadow around a rounded box with no background", () => {
  test.skipIf(!SCREENSHOTS)(
    "fills the corners outside the radius, follows each corner's radius, and clips inset to the rounded box",
    async () => {
      await Bun.sleep(500);
      const shot = readPng(await app.screenshot());
      const k = shot.width / 640;
      const wrong: string[] = [];
      for (const [id, points] of cases) {
        const b = await box(id);
        for (const [dx, dy, ok] of points) {
          const c = shot.at(Math.floor((b.x + dx + 0.5) * k), Math.floor((b.y + dy + 0.5) * k));
          if (!ok(c)) wrong.push(`#${id} (${dx}, ${dy}): ${c} is not ${ok.name}`);
        }
      }
      expect(wrong).toEqual([]);
    },
    T,
  );
});

// Abramowitz and Stegun 7.1.26, |error| < 1.5e-7.
const erfc = (x: number): number => {
  const t = 1 / (1 + 0.3275911 * Math.abs(x));
  const y =
    t *
    (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429)))) *
    Math.exp(-x * x);
  return x >= 0 ? y : 2 - y;
};

describe("box-shadow blur", () => {
  test.skipIf(!SCREENSHOTS)(
    "falls off as a Gaussian whose sigma is half the blur radius",
    async () => {
      await Bun.sleep(500);
      const shot = readPng(await app.screenshot());
      const k = shot.width / 640;
      const b = await box("falloff");
      const sigma = 16 / 2;
      const y = Math.floor((b.y + b.h / 2) * k);
      const wrong: string[] = [];
      for (const d of [1, 4, 8, 12, 16, 24]) {
        const x = Math.floor((b.x + b.w + d) * k);
        const at = (x + 0.5) / k - (b.x + b.w);
        const want = 255 * (1 - 0.5 * erfc(at / (sigma * Math.SQRT2)));
        const [r, g, bl] = shot.at(x, y);
        const luma = (r + g + bl) / 3;
        if (Math.abs(luma - want) > 12) wrong.push(`${d}px out: luma ${luma.toFixed(0)}, want ${want.toFixed(0)}`);
      }
      expect(wrong).toEqual([]);
      expect(await app.eval(`getComputedStyle(document.getElementById("falloff")).boxShadow`)).toBe(
        "rgb(0, 0, 0) 0px 0px 16px 0px",
      );
    },
    T,
  );
});
