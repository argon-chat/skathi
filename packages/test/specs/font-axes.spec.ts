import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng, SCREENSHOTS } from "../src/index";

const WIDTH = 360;
const app = await launch({
  app: "packages/test/fixtures/font-axes/src/main.ts",
  window: [WIDTH, 520],
  label: "font-axes",
});
afterAll(() => app.close());

const T = 30_000;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const js = <V>(src: string) => app.eval(src) as Promise<V>;

type Box = { x: number; y: number; w: number; h: number };
const box = async (id: string): Promise<Box> => (await app.query(`#${id}`)).nodes[0]!.bounds!;
const width = async (id: string) => (await app.query(`#${id}`)).nodes[0]?.bounds?.w ?? -1;
const computed = (id: string, property: string) =>
  js<string>(`getComputedStyle(document.getElementById(${JSON.stringify(id)})).getPropertyValue("${property}")`);

async function settled() {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const loaded = Number(await js("__SK_TEST_AXES__.loaded()"));
    const [base, condensed] = [await width("base"), await width("condensed")];
    if (loaded >= 2 && base > 0 && condensed > 0 && condensed < base - 5) break;
    await sleep(50);
  }
  await sleep(300);
}

type Shot = ReturnType<typeof readPng>;

/** Dark pixels of `id`'s box, and how far right the ink of its top third sits from its bottom third's. */
function ink(shot: Shot, b: Box) {
  const k = shot.width / WIDTH;
  const [x0, y0, cols, rows] = [b.x, b.y, b.w, b.h].map((v) => Math.round(v * k)) as [number, number, number, number];
  let count = 0;
  const centroid = (from: number, to: number) => {
    let [sum, total] = [0, 0];
    for (let y = from; y < to; y++) {
      for (let x = 0; x < cols; x++) {
        const p = shot.at(x0 + x, y0 + y);
        const v = 255 - (p[0] + p[1] + p[2]) / 3;
        sum += v * x;
        total += v;
      }
    }
    return sum / Math.max(total, 1);
  };
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      const p = shot.at(x0 + x, y0 + y);
      if ((p[0] + p[1] + p[2]) / 3 < 160) count++;
    }
  }
  const third = Math.floor(rows / 3);
  return { count, lean: (centroid(0, third) - centroid(rows - third, rows)) / k };
}

async function samePixels(a: string, b: string) {
  const shot = readPng(await app.screenshot());
  const k = shot.width / WIDTH;
  const [ba, bb] = [await box(a), await box(b)];
  const cols = Math.floor(Math.min(ba.w, bb.w) * k);
  const rows = Math.floor(Math.min(ba.h, bb.h) * k);
  let [dark, diff] = [0, 0];
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      const p = shot.at(Math.round(ba.x * k) + x, Math.round(ba.y * k) + y);
      const q = shot.at(Math.round(bb.x * k) + x, Math.round(bb.y * k) + y);
      if ((p[0] + p[1] + p[2]) / 3 < 160) dark++;
      if (Math.abs(p[0] - q[0]) + Math.abs(p[1] - q[1]) + Math.abs(p[2] - q[2]) > 60) diff++;
    }
  }
  expect(dark).toBeGreaterThan(100);
  expect(diff).toBeLessThanOrEqual(Math.max(4, Math.round(dark * 0.02)));
}

describe("variable font axes from CSS", () => {
  test(
    "getComputedStyle reports the font axes properties",
    async () => {
      await settled();
      expect(await computed("base", "font-stretch")).toBe("100%");
      expect(await computed("condensed", "font-stretch")).toBe("75%");
      expect(await computed("width", "font-width")).toBe("75%");
      expect(await computed("oblique", "font-style")).toBe("oblique 8deg");
      expect(await computed("italic", "font-style")).toBe("italic");
      expect(await computed("base", "font-variation-settings")).toBe("normal");
      expect(await computed("settings", "font-variation-settings")).toBe('"wdth" 75');
      expect(await computed("inherited", "font-variation-settings")).toBe('"wdth" 75');
      expect(await computed("override", "font-weight")).toBe("700");
    },
    T,
  );

  test(
    "each axis changes the measured width as asked",
    async () => {
      await settled();
      const base = await width("base");
      const condensed = await width("condensed");
      expect(condensed).toBeLessThan(base - 5);
      expect(await width("expanded")).toBeGreaterThan(base + 5);
      expect(await width("width")).toBeCloseTo(condensed, 1);
      expect(await width("settings")).toBeCloseTo(condensed, 1);
      expect(await width("inherited")).toBeCloseTo(condensed, 1);
      // `"wght" 400` wins over `font-weight: 700`.
      expect(await width("override")).toBeCloseTo(base, 1);
      expect(await width("oblique")).toBeCloseTo(base, 1);
      // The face is declared `font-stretch: 75% 100%`, so 125% draws at 100%.
      expect(await width("narrow")).toBeCloseTo(base, 1);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "the drawn glyphs follow the axes",
    async () => {
      await settled();
      await samePixels("settings", "condensed");
      await samePixels("width", "condensed");
      await samePixels("override", "base");
      await samePixels("narrow", "narrow-ref");
      const shot = readPng(await app.screenshot());
      const [base, oblique, italic, grad] = await Promise.all(
        ["base", "oblique", "italic", "grad"].map(async (id) => ink(shot, await box(id))),
      );
      expect(oblique!.lean).toBeGreaterThan(base!.lean + 0.5);
      expect(italic!.lean).toBeGreaterThan(oblique!.lean);
      expect(grad!.count).toBeGreaterThan(base!.count);
    },
    T,
  );

  test(
    "the app logged nothing broken",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
