import { afterAll, describe, expect, test } from "bun:test";
import { type Bounds, launch, readPng, SCREENSHOTS } from "../src/index";

const W = 560;
const app = await launch({
  app: "packages/test/fixtures/mask/src/main.ts",
  window: [W, 400],
  label: "mask",
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
  const k = shot.width / W;
  return {
    k,
    at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)),
    /** The device pixels on the row through CSS `y` from CSS `x0` to `x1`. */
    row: (y: number, x0: number, x1: number): Rgb[] => {
      const out: Rgb[] = [];
      for (let x = Math.floor(x0 * k); x < Math.ceil(x1 * k); x++) out.push(shot.at(x, Math.floor((y + 0.5) * k)));
      return out;
    },
  };
};
type Shot = Awaited<ReturnType<typeof shoot>>;

async function until(ok: (s: Shot) => boolean, ms = 5_000): Promise<Shot> {
  const deadline = Date.now() + ms;
  let s = await shoot();
  while (!ok(s) && Date.now() < deadline) {
    await sleep(80);
    s = await shoot();
  }
  return s;
}

const near =
  (want: Rgb, tol = 40) =>
  (got: Rgb) =>
    got.every((v, i) => Math.abs(v - want[i]!) <= tol);
const white = near([255, 255, 255], 12);
const blue = near([0, 0, 255]);
const red = near([255, 0, 0]);
const green = near([0, 160, 0]);
const said = (s: Shot, x: number, y: number) => `rgb(${s.at(x, y).join(", ")}) at (${x}, ${y})`;

/** Pixels on a row that are neither of the two colours an edge separates: its antialiasing ramp. */
const ramp = (pixels: Rgb[], a: (c: Rgb) => boolean, b: (c: Rgb) => boolean) =>
  pixels.filter((c) => !a(c) && !b(c)).length;

describe("an SVG icon as a mask over `currentColor`", () => {
  test.skipIf(!SCREENSHOTS)(
    "paints the icon's shape in the text colour, edges one device pixel wide",
    async () => {
      const b = await box("#icon");
      expect(b).toMatchObject({ w: 48, h: 48 });
      // A 12px icon drawn at 48px: the square is 12..36. Rasterised at its natural size and scaled
      // up, its edges would ramp over several device pixels.
      const s = await until((s) => blue(s.at(b.x + 24, b.y + 24)));
      expect(blue(s.at(b.x + 24, b.y + 24)), said(s, b.x + 24, b.y + 24)).toBe(true);
      for (const [x, y] of [
        [4, 4],
        [8, 24],
        [40, 24],
        [24, 44],
      ] as const)
        expect(white(s.at(b.x + x, b.y + y)), said(s, b.x + x, b.y + y)).toBe(true);
      const edge = s.row(b.y + 24, b.x + 6, b.x + 18);
      expect(ramp(edge, white, blue), `k=${s.k}: ${JSON.stringify(edge)}`).toBeLessThanOrEqual(1);
      expect(edge.filter(blue).length).toBe(Math.round(6 * s.k));
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "under `transform: scale(2)` it is as sharp",
    async () => {
      // `.zoom` is at (100, 20) and doubles a 24px icon to 48px.
      const [x, y] = [100, 20];
      const s = await until((s) => blue(s.at(x + 24, y + 24)));
      expect(blue(s.at(x + 24, y + 24)), said(s, x + 24, y + 24)).toBe(true);
      expect(white(s.at(x + 6, y + 24)), said(s, x + 6, y + 24)).toBe(true);
      const edge = s.row(y + 24, x + 6, x + 18);
      expect(ramp(edge, white, blue), `k=${s.k}: ${JSON.stringify(edge)}`).toBeLessThanOrEqual(1);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "an icon on a `::before` box, the usual icon-font replacement",
    async () => {
      const b = await box("#pseudo");
      const s = await until((s) => red(s.at(b.x + 24, b.y + 24)));
      expect(red(s.at(b.x + 24, b.y + 24)), said(s, b.x + 24, b.y + 24)).toBe(true);
      expect(white(s.at(b.x + 6, b.y + 24)), said(s, b.x + 6, b.y + 24)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`mask: url() center / 32px 32px no-repeat` from a data: URL",
    async () => {
      const b = await box("#dot");
      const s = await until((s) => red(s.at(b.x + 24, b.y + 24)));
      expect(red(s.at(b.x + 24, b.y + 24)), said(s, b.x + 24, b.y + 24)).toBe(true);
      // The 32px circle sits 8px in: its rim, the box's corners and the 8px margin show nothing.
      expect(red(s.at(b.x + 10, b.y + 24)), said(s, b.x + 10, b.y + 24)).toBe(true);
      for (const [x, y] of [
        [5, 24],
        [42, 24],
        [10, 10],
        [2, 2],
      ] as const)
        expect(white(s.at(b.x + x, b.y + y)), said(s, b.x + x, b.y + y)).toBe(true);
    },
    T,
  );
});

describe("gradient masks and tiling", () => {
  test.skipIf(!SCREENSHOTS)(
    "`linear-gradient(to bottom, #000 50%, transparent)` fades the bottom half out",
    async () => {
      const b = await box("#fade");
      const s = await until((s) => blue(s.at(b.x + 100, b.y + 20)));
      expect(blue(s.at(b.x + 100, b.y + 20)), said(s, b.x + 100, b.y + 20)).toBe(true);
      const [r, , bl] = s.at(b.x + 100, b.y + 75);
      expect(Math.abs(r - 128), said(s, b.x + 100, b.y + 75)).toBeLessThan(30);
      expect(bl, said(s, b.x + 100, b.y + 75)).toBe(255);
      expect(white(s.at(b.x + 100, b.y + 99)), said(s, b.x + 100, b.y + 99)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`mask-size: 20px 20px; mask-repeat: repeat-x; mask-position: 10px 0` tiles one row",
    async () => {
      const b = await box("#stripes");
      const s = await until((s) => green(s.at(b.x + 15, b.y + 10)));
      for (const x of [15, 35, 55, 115]) expect(green(s.at(b.x + x, b.y + 10)), said(s, b.x + x, b.y + 10)).toBe(true);
      for (const x of [5, 25, 45, 105]) expect(white(s.at(b.x + x, b.y + 10)), said(s, b.x + x, b.y + 10)).toBe(true);
      expect(white(s.at(b.x + 15, b.y + 30)), said(s, b.x + 15, b.y + 30)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "the mask cuts the element's children too",
    async () => {
      const b = await box("#subtree");
      const s = await until((s) => red(s.at(b.x + 20, b.y + 30)));
      expect(red(s.at(b.x + 20, b.y + 30)), said(s, b.x + 20, b.y + 30)).toBe(true);
      expect(white(s.at(b.x + 100, b.y + 30)), said(s, b.x + 100, b.y + 30)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "`luminance` masks by the brightness: white keeps, black hides",
    async () => {
      const b = await box("#luminance");
      const s = await until((s) => red(s.at(b.x + 20, b.y + 20)));
      expect(red(s.at(b.x + 20, b.y + 20)), said(s, b.x + 20, b.y + 20)).toBe(true);
      expect(white(s.at(b.x + 100, b.y + 20)), said(s, b.x + 100, b.y + 20)).toBe(true);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "a stop at `calc(100% - 100px)` is a hard edge 100px from the end",
    async () => {
      const b = await box("#edge");
      const s = await until((s) => blue(s.at(b.x + 290, b.y + 20)));
      for (const x of [20, 200, 290]) expect(blue(s.at(b.x + x, b.y + 20)), said(s, b.x + x, b.y + 20)).toBe(true);
      for (const x of [310, 390]) expect(white(s.at(b.x + x, b.y + 20)), said(s, b.x + x, b.y + 20)).toBe(true);
    },
    T,
  );
});

describe("advisories", () => {
  test.skipIf(!SCREENSHOTS)(
    "`-webkit-mask-image` is ignored with an advisory; the supported masks raise none",
    async () => {
      const b = await box("#webkit");
      const s = await until((s) => green(s.at(b.x + 2, b.y + 2)));
      expect(green(s.at(b.x + 2, b.y + 2)), said(s, b.x + 2, b.y + 2)).toBe(true);
      const logs = JSON.stringify((await app.logs()).entries);
      expect(logs).toContain("`-webkit-mask-image` is not supported");
      expect(logs).not.toContain("is not applied (v1), and the element paints unmasked");
      expect(logs).not.toContain("could not be loaded");
      expect(logs).not.toContain("property not mapped (v1): mask");
    },
    T,
  );
});
