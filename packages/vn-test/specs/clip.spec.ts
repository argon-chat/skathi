// Rounded clipping: `overflow` under a `border-radius` clips to the rounded
// padding box. gpui's content masks were rectangles, so the Argon client's
// avatars (reka's AvatarRoot, `rounded-full overflow-hidden` around an <img>)
// came out square, and its rounded cards showed their lists' rows in the
// corners. The fork's masks carry corner radii now (docs/ARCHITECTURE.md,
// fork item 33).
//
//   bun test packages/vn-test/specs/clip.spec.ts
//
// `fixtures/clip` lays the shapes out at fixed positions on white.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/clip/src/main.ts",
  window: [720, 520],
  label: "clip",
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

/** Polls `read` until `ok` holds, and returns the last answer either way. */
async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

/** A screenshot, read at CSS pixel `(x, y)`'s centre whatever the scale. */
const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  return {
    at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)),
    /** How many CSS pixels of `b` satisfy `hit`, sampled at their centres. */
    count: (b: Bounds, hit: (rgb: Rgb) => boolean) => {
      let n = 0;
      for (let y = 0; y < b.h; y++) for (let x = 0; x < b.w; x++) if (hit(shot.at(Math.floor((b.x + x + 0.5) * k), Math.floor((b.y + y + 0.5) * k)))) n++;
      return n;
    },
  };
};

const white = ([r, g, b]: Rgb) => r > 235 && g > 235 && b > 235;
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const lime = ([r, g, b]: Rgb) => g > 200 && r < 60 && b < 60;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 60 && g < 60;
const gray = ([r, g, b]: Rgb) => Math.abs(r - 136) < 20 && Math.abs(g - 136) < 20 && Math.abs(b - 136) < 20;

/** The four corner pixels `inset` in from each corner of `b`. */
const corners = (b: Bounds, inset: number): [number, number][] => [
  [b.x + inset, b.y + inset],
  [b.x + b.w - 1 - inset, b.y + inset],
  [b.x + inset, b.y + b.h - 1 - inset],
  [b.x + b.w - 1 - inset, b.y + b.h - 1 - inset],
];

describe("a round avatar (reka's AvatarRoot around an <img>)", () => {
  test("the image is clipped to the circle", async () => {
    const b = await box("#avatar");
    bunExpect(b).toMatchObject({ w: 64, h: 64 });
    const area = (s: Awaited<ReturnType<typeof shoot>>) => s.count(b, red);
    // The image decodes asynchronously; wait for it to paint at all.
    await until(async () => area(await shoot()), (n) => n > 1000);
    const s = await shoot();
    // A 64px disc is pi * 32^2 = 3217 px; the square was 4096.
    bunExpect(area(s)).toBeGreaterThan(3050);
    bunExpect(area(s)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
    bunExpect(red(s.at(b.x + 32, b.y + 32))).toBe(true);
    bunExpect(red(s.at(b.x + 32, b.y + 2))).toBe(true);
  }, T);

  test("an <img> rounded by its own border-radius is a disc too", async () => {
    const b = await box("#round-img");
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
  }, T);
});

describe("a rounded card around a list that scrolls", () => {
  const banded = (rgb: Rgb) => lime(rgb) || blue(rgb);

  test("the rows clip at the card's curve, before and after scrolling", async () => {
    const b = await box("#card-a");
    const check = async () => {
      const s = await shoot();
      // radius 24: (3, 3) is 29px from the corner circle's centre.
      for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
      // Inside the curve, and the straight edges, are the rows.
      for (const [x, y] of corners(b, 9)) bunExpect(banded(s.at(x, y))).toBe(true);
      bunExpect(banded(s.at(b.x + 100, b.y))).toBe(true);
      bunExpect(banded(s.at(b.x, b.y + 80))).toBe(true);
    };
    await until(shoot, (s) => banded(s.at(b.x + 100, b.y + 80)));
    await check();

    const top = await app.eval(`(() => { const l = document.getElementById("list-a"); l.scrollTop = 37; return l.scrollTop; })()`);
    bunExpect(top).toBe(37);
    // Band 0 is lime and 40px tall, so 37px down the top row is lime's last
    // 3px and then blue: the list moved under the mask.
    await until(shoot, (s) => blue(s.at(b.x + 100, b.y + 10)));
    await check();
  }, T);

  test("a bordered scroller clips its rows to the inner radius", async () => {
    // radius 24 less the 4px border: the padding box's corners are 20px
    // circles around the same centres, (24, 24) in from each corner.
    const b = await box("#card-b");
    const check = async () => {
      const s = await shoot();
      for (const [x, y] of corners(b, 2)) bunExpect(white(s.at(x, y))).toBe(true);
      // 21.9px from the centre: inside the border's curve, outside the
      // padding box's. The rows used to cover it.
      for (const [x, y] of corners(b, 8)) bunExpect(gray(s.at(x, y))).toBe(true);
      bunExpect(banded(s.at(b.x + 100, b.y + 80))).toBe(true);
    };
    await until(shoot, (s) => banded(s.at(b.x + 100, b.y + 80)));
    await check();

    const top = await app.eval(`(() => { const l = document.getElementById("card-b"); l.scrollTop = 57; return l.scrollTop; })()`);
    bunExpect(top).toBe(57);
    await sleep(200);
    await check();
  }, T);
});
