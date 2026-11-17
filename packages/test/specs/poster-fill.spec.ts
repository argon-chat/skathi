import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng, SCREENSHOTS } from "../src/index";

const W = 720;
const app = await launch({
  app: "packages/test/fixtures/poster-fill/src/main.ts",
  window: [W, 520],
  label: "poster-fill",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];
type Rect = [number, number, number, number];

const rect = (id: string) =>
  app.eval(
    `(() => { const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map((v) => Math.round(v)); })()`,
  ) as Promise<Rect>;

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 8_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(100);
    v = await read();
  }
  return v;
}

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / W;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};
const grey = ([r, g, b]: Rgb) => Math.abs(r - 128) < 12 && Math.abs(g - 128) < 12 && Math.abs(b - 128) < 12;
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;

describe("a picture at width/height 100% fills its box", () => {
  const covers: [string, string][] = [
    ["c-wide", "p-wide"],
    ["c-png-wide", "p-c-png-wide"],
    ["c-canvas", "p-canvas"],
    ["c-thumb", "t-c-thumb"],
    ["c-inflow-block", "p-inflow-block"],
    ["c-inflow-flex", "p-inflow-flex"],
    ["c-tall", "p-tall"],
    ["c-video", "p-video"],
  ];

  test(
    "its box is the cell's, not its own shape",
    async () => {
      for (const [cell, pic] of covers) {
        const c = await rect(cell);
        const want: Rect =
          pic === "t-c-thumb"
            ? ([c[0] - c[2] * 0.05, c[1] - c[3] * 0.05, c[2] * 1.1, c[3] * 1.1].map((v) => Math.round(v)) as Rect)
            : c;
        expect(
          await until(
            () => rect(pic),
            (r) => r.join() === want.join(),
          ),
        ).toEqual(want);
      }
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`cover` crops: red at the centre, no grey at the edges",
    async () => {
      for (const [cell] of covers.filter(([c]) => c !== "c-video")) {
        const [x, y, w, h] = await rect(cell);
        const at = await until(shoot, (at) => !grey(at(x + w / 2, y + h - 3)));
        expect({ cell, mid: red(at(x + w / 2, y + h / 2 + 8)) }).toEqual({ cell, mid: true });
        for (const [px, py] of [
          [x + w / 2, y + 3],
          [x + w / 2, y + h - 3],
          [x + 3, y + h - 3],
          [x + w - 3, y + h - 3],
        ]) {
          expect({ cell, px, py, grey: grey(at(px!, py!)) }).toEqual({ cell, px, py, grey: false });
        }
      }
      const [x, y, w, h] = await rect("c-video");
      const at = await shoot();
      expect(grey(at(x + w / 2, y + h - 3))).toBe(false);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`contain` letterboxes, centred, with the grey above and below",
    async () => {
      const [x, y, w, h] = await rect("c-contain");
      expect(await rect("p-contain")).toEqual([x, y, w, h]);
      // The box is laid out before the source decodes: wait for the picture.
      const at = await until(shoot, (at) => red(at(x + w / 2, y + h / 2 + 8)));
      expect(grey(at(x + w / 2, y + 1))).toBe(true);
      expect(red(at(x + w / 2, y + h / 2 + 8))).toBe(true);
      expect(grey(at(x + w / 2, y + h - 2))).toBe(true);
    },
    T,
  );
});
