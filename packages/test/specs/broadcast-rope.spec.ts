import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng, SCREENSHOTS } from "../src/index";

const W = 360;
const app = await launch({
  app: "packages/test/fixtures/broadcast-rope/src/main.ts",
  window: [W, 420],
  label: "broadcast-rope",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];
type Rope = { measured: number; paths: string[]; height: number; settled: boolean };

const rope = () => app.eval(`globalThis.__SK_TEST_ROPE__`) as Promise<Rope>;
const rect = (sel: string) =>
  app.eval(
    `(() => { const r = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })()`,
  ) as Promise<[number, number, number, number]>;

async function settled(): Promise<Rope> {
  const deadline = Date.now() + 8_000;
  let r = await rope();
  while (!r.settled && Date.now() < deadline) {
    await sleep(100);
    r = await rope();
  }
  return r;
}

const dark = ([r, g, b]: Rgb) => r < 110 && g < 110 && b < 110;

describe("broadcast connector", () => {
  test.skipIf(!SCREENSHOTS)(
    "the path is drawn where its coordinates say, in the svg box's pixels",
    async () => {
      const r = await settled();
      expect(r.settled).toBe(true);
      expect(r.paths.length).toBe(1);
      const nums = r.paths[0]!.match(/-?\d+(\.\d+)?/g)!.map(Number);
      const [x, top, bottom] = [nums[0]!, nums[1]!, nums[2]!];
      const [sx, sy] = await rect("#scroll");
      const [, svgTop, svgW, svgH] = await rect("#rope");
      expect(Math.round(svgW)).toBe(240);
      expect(Math.round(svgH)).toBe(r.height);
      expect(Math.round(svgTop)).toBe(Math.round(sy));

      const shot = readPng(await app.screenshot());
      const k = shot.width / W;
      const at = (px: number, py: number): Rgb => shot.at(Math.floor(px * k), Math.floor(py * k));
      const lane = sx + x;
      const samples = [0.25, 0.5, 0.75].map((f) => at(lane, sy + top + (bottom - top) * f));
      for (const s of samples) expect(dark(s), `rope pixel ${s}`).toBe(true);
      // And the gutter beside it is not: a 100x100 document scaled over the box would smear the
      // line, or clip it away entirely.
      expect(dark(at(lane + 6, sy + (top + bottom) / 2))).toBe(false);
    },
    T,
  );
});
