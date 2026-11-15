import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/text-shadow/src/main.ts",
  window: [720, 520],
  label: "text-shadow",
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

/** Reads in device pixels; `k` converts a CSS length. */
const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  const at = (x: number, y: number): Rgb => shot.at(Math.round(x), Math.round(y));
  return { k, at };
};
type Shot = Awaited<ReturnType<typeof shoot>>;

const luma = ([r, g, b]: Rgb) => (r + g + b) / 3;
const red = ([r, g, b]: Rgb) => r > 200 && g < 70 && b < 70;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 70 && g < 70;
const green = ([r, g, b]: Rgb) => g > 120 && r < 60 && b < 60;
const magenta = ([r, g, b]: Rgb) => r > 200 && b > 200 && g < 70;
const dark = (c: Rgb) => luma(c) < 128;

/** Every device pixel of a CSS rect, grown by `pad` CSS px. */
function* pixels(s: Shot, b: Bounds, pad = 0): Generator<[number, number]> {
  const x0 = Math.floor((b.x - pad) * s.k);
  const y0 = Math.floor((b.y - pad) * s.k);
  const x1 = Math.ceil((b.x + b.w + pad) * s.k);
  const y1 = Math.ceil((b.y + b.h + pad) * s.k);
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) yield [x, y];
}

const count = (s: Shot, b: Bounds, hit: (c: Rgb) => boolean, pad = 0) => {
  let n = 0;
  for (const [x, y] of pixels(s, b, pad)) if (hit(s.at(x, y))) n++;
  return n;
};

/** Darkness (255 - luma) summed, and its centroid, over a rect. */
const ink = (s: Shot, b: Bounds, pad: number) => {
  let mass = 0;
  let cx = 0;
  let cy = 0;
  let peak = 0;
  let spread = 0;
  for (const [x, y] of pixels(s, b, pad)) {
    const d = 255 - luma(s.at(x, y));
    if (d <= 2) continue;
    mass += d;
    cx += d * x;
    cy += d * y;
    peak = Math.max(peak, d);
    spread++;
  }
  return { mass, cx: cx / mass, cy: cy / mass, peak, spread };
};

const shift = (b: Bounds, dx: number, dy: number): Bounds => ({ ...b, x: b.x + dx, y: b.y + dy });

const advisories = async (pattern: RegExp) =>
  (await app.logs(0)).entries
    .filter((e) => (e.kind === "advisory" || e.kind === "css") && pattern.test(e.text))
    .map((e) => e.text);

const computed = async (id: string): Promise<string> =>
  String(await app.eval(`getComputedStyle(document.getElementById(${JSON.stringify(id)})).textShadow`));

describe("text-shadow", () => {
  test(
    "a sharp shadow is the text drawn again, offset exactly",
    async () => {
      await sleep(500);
      const b = await box("#hard");
      const s = await shoot();
      const dx = Math.round(60 * s.k);
      let inked = 0;
      const off: string[] = [];
      for (const [x, y] of pixels(s, b, 2)) {
        const text = s.at(x, y);
        if (dark(text)) inked++;
        const shadow = s.at(x + dx, y);
        if (text.some((v, i) => Math.abs(v - shadow[i]!) > 1)) off.push(`(${x}, ${y}) ${text} vs ${shadow}`);
      }
      expect(inked).toBeGreaterThan(200);
      expect(off).toEqual([]);
    },
    T,
  );

  test(
    "a blurred shadow spreads its ink and stays centred on its offset",
    async () => {
      const [ref, soft] = [await box("#soft-ref"), await box("#soft")];
      const s = await shoot();
      const sharp = ink(s, shift(ref, 20, 10), 14);
      const blurred = ink(s, shift(soft, 20, 10), 14);
      const ax = blurred.cx - ((soft.x - ref.x) * s.k + sharp.cx);
      const ay = blurred.cy - sharp.cy;
      expect(Math.abs(ax)).toBeLessThan(0.5 * s.k);
      expect(Math.abs(ay)).toBeLessThan(0.5 * s.k);
      expect(blurred.mass / sharp.mass).toBeGreaterThan(0.9);
      expect(blurred.mass / sharp.mass).toBeLessThan(1.1);
      expect(blurred.spread).toBeGreaterThan(sharp.spread * 1.4);
      expect(blurred.peak).toBeLessThan(sharp.peak * 0.9);
    },
    T,
  );

  test(
    "shadows paint in list order, the first on top, all under the text",
    async () => {
      const [hard, multi] = [await box("#hard"), await box("#multi")];
      const s = await shoot();
      const dy = Math.round((multi.y - hard.y) * s.k);
      const dx = Math.round(12 * s.k);
      // The glyph's own pixels, read off the black text of `#hard` (same font, same x).
      const glyph = (x: number, y: number) => luma(s.at(x, y - dy)) < 40;
      const bare = (x: number, y: number) => luma(s.at(x, y - dy)) > 250;
      let onTop = 0;
      let under = 0;
      const wrong: string[] = [];
      for (const [x, y] of pixels(s, { ...multi, w: 30 })) {
        const c = s.at(x, y);
        if (glyph(x, y)) {
          onTop++;
          if (!red(c)) wrong.push(`(${x}, ${y}) ${c} should be the first shadow's red`);
        } else if (bare(x, y) && glyph(x - dx, y)) {
          under++;
          if (!blue(c)) wrong.push(`(${x}, ${y}) ${c} should be the second shadow's blue`);
        }
      }
      expect(onTop).toBeGreaterThan(100);
      expect(under).toBeGreaterThan(50);
      expect(wrong).toEqual([]);
    },
    T,
  );

  test(
    "it inherits, `none` stops it, and currentColor is each element's own color",
    async () => {
      const [inherit, none, current] = [await box("#inherit"), await box("#inherit-none"), await box("#current")];
      const s = await shoot();
      const glyph = { ...inherit, w: 30 };
      expect(count(s, shift(glyph, 60, 0), green)).toBeGreaterThan(100);
      expect(count(s, shift({ ...none, w: 30 }, 60, 0), (c) => luma(c) < 250, 2)).toBe(0);
      const currentGlyph = { ...current, w: 30 };
      expect(count(s, shift(currentGlyph, 60, 0), magenta)).toBeGreaterThan(100);
      expect(count(s, shift(currentGlyph, 60, 0), dark)).toBe(0);

      expect(await computed("hard")).toBe("rgb(0, 0, 0) 60px 0px 0px");
      expect(await computed("inherit-none")).toBe("none");
      expect(await computed("current-child")).toBe("rgb(255, 0, 255) 60px 0px 0px");
      expect(await computed("multi")).toBe("rgb(255, 0, 0) 0px 0px 0px, rgb(0, 0, 255) 12px 0px 0px");
      expect(await computed("soft")).toBe("rgb(0, 0, 0) 20px 10px 6px");
    },
    T,
  );

  test(
    "it shadows the ellipsis of truncated text, and gradient-filled text",
    async () => {
      const [ellipsis, bgclip] = [await box("#ellipsis"), await box("#bgclip")];
      const s = await shoot();
      const tail = { x: ellipsis.x + ellipsis.w - 14, y: ellipsis.y, w: 14, h: ellipsis.h };
      expect(count(s, tail, dark)).toBeGreaterThan(4);
      expect(count(s, tail, red)).toBeGreaterThan(4);
      expect(count(s, bgclip, red)).toBeGreaterThan(20);
      expect(count(s, bgclip, (c) => c[2] > 150 && c[0] < 120 && c[1] < 80)).toBeGreaterThan(20);
      expect(count(s, shift(bgclip, 6, 6), (c) => luma(c) < 30)).toBeGreaterThan(100);
    },
    T,
  );

  test(
    "a transition interpolates the shadow's colour",
    async () => {
      const b = await box("#trans");
      // The shadow's colour where the glyph is solid: the median skips its antialiased edge.
      const stem = async () => {
        const s = await shoot();
        const reds: number[] = [];
        const greens: number[] = [];
        for (const [x, y] of pixels(s, { ...b, w: 30 })) {
          if (luma(s.at(x, y)) > 20) continue;
          const c = s.at(x + Math.round(60 * s.k), y);
          reds.push(c[0]);
          greens.push(c[1]);
        }
        const median = (v: number[]) => v.sort((p, q) => p - q)[v.length >> 1] ?? NaN;
        return { r: median(reds), g: median(greens) };
      };
      const before = await stem();
      expect(before.g).toBeGreaterThan(250);
      await app.eval(`(document.getElementById("trans").classList.add("on"), 0)`);
      await sleep(1500);
      const mid = await stem();
      expect(mid.r).toBeGreaterThan(250);
      expect(mid.g).toBeGreaterThan(40);
      expect(mid.g).toBeLessThan(215);
      await sleep(2000);
      const after = await stem();
      expect(after.g).toBe(0);
    },
    T,
  );

  test(
    "valid values advise nothing; a spread or a negative blur is dropped with a reason",
    async () => {
      expect(await advisories(/text-shadow/)).toEqual([]);
      for (const value of ["1px 1px 2px 3px red", "1px 1px -2px red"]) {
        await app.eval(`(document.getElementById("hard").style.textShadow = ${JSON.stringify(value)}, 0)`);
      }
      await sleep(300);
      const said = await advisories(/text-shadow/);
      expect(said.some((t) => /spread/.test(t) && /Remove the fourth length/.test(t))).toBe(true);
      expect(said.some((t) => /negative/.test(t) && /Use 0/.test(t))).toBe(true);
      expect(await computed("hard")).toBe("rgb(0, 0, 0) 60px 0px 0px");
    },
    T,
  );
});
