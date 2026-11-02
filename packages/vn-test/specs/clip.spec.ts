// Rounded clipping: `overflow` under a `border-radius` clips to the rounded
// padding box. gpui's content masks were rectangles, so the Argon client's
// avatars (reka's AvatarRoot, `rounded-full overflow-hidden` around an <img>)
// came out square, and its rounded cards showed their lists' rows in the
// corners. The fork's masks carry corner radii now (docs/ARCHITECTURE.md,
// fork item 33), and a percentage radius resolves against the box at paint
// (fork item 34).
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

  test("an <img> with `border-radius: 50%` is a disc too", async () => {
    const b = await box("#round-img");
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeGreaterThan(3050);
    bunExpect(s.count(b, red)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
  }, T);

  test("`50%` on a box that is not square clips to a pill", async () => {
    // 120x44: CSS draws 60x22 elliptical corners (area pi*60*22 = 4147).
    // gpui's corners are circles, so the smaller radius makes a 22px pill:
    // 76*44 + pi*22^2 = 4865. The square box was 5280. This pins the
    // documented divergence (crates/vn-css/README.md §5 item 13).
    const b = await box("#oval");
    bunExpect(b).toMatchObject({ w: 120, h: 44 });
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeGreaterThan(4750);
    bunExpect(s.count(b, red)).toBeLessThan(5000);
    for (const [x, y] of corners(b, 2)) bunExpect(white(s.at(x, y))).toBe(true);
    bunExpect(red(s.at(b.x + 60, b.y + 1))).toBe(true);
    bunExpect(red(s.at(b.x + 1, b.y + 22))).toBe(true);
    // The advisory goes out on the build after the paint that drew the
    // circle; nudge a frame in case the page has gone idle.
    await app.eval(`document.getElementById("tick").style.right = "3px"`);
    const said = await until(
      async () => JSON.stringify((await app.logs()).entries),
      (text) => text.includes("elliptical corner"),
    );
    bunExpect(said).toContain("elliptical corner");
  }, T);
});

describe("`border-radius: inherit` (the Argon client's avatars and profile card)", () => {
  test("an inline `inherit` on an <img> takes its round wrapper's radius", async () => {
    const b = await box("#inh-img");
    bunExpect(b).toMatchObject({ w: 64, h: 64 });
    const s = await until(shoot, (s) => s.count(b, red) > 1000);
    bunExpect(s.count(b, red)).toBeGreaterThan(3050);
    bunExpect(s.count(b, red)).toBeLessThan(3400);
    for (const [x, y] of corners(b, 3)) bunExpect(white(s.at(x, y))).toBe(true);
  }, T);

  test("a card's layers round themselves from it, two levels down", async () => {
    // radius 16 on an 88x46 card: (2, 2) from a corner is outside the curve.
    const b = await box("#pcard");
    bunExpect(b).toMatchObject({ w: 88, h: 46 });
    const s = await until(shoot, (s) => red(s.at(b.x + 44, b.y + 6)));
    // The red background clips at the inherited curve; the blue body, two
    // `inherit`s down, paints its bottom corners round. Square, either one
    // showed in the corners.
    for (const [x, y] of corners(b, 2)) bunExpect(white(s.at(x, y))).toBe(true);
    const [tl, tr, bl, br] = corners(b, 8);
    for (const [x, y] of [tl, tr]) bunExpect(red(s.at(x, y))).toBe(true);
    for (const [x, y] of [bl, br]) bunExpect(blue(s.at(x, y))).toBe(true);
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

describe("box-shadow outside the border box (CSS Backgrounds 3 §7.1)", () => {
  // An outer shadow is drawn only outside the border box, so a control with a
  // transparent or translucent background and a focus ring (Tailwind's
  // `ring-[3px]`, a spread shadow) shows a ring, not a filled slab of the
  // ring colour. `filter: blur()` on a solid box, which vue-native draws as
  // the box's own shadow, stays filled.
  test("a ring around a transparent box is a ring, and a blurred box stays filled", async () => {
    await app.eval(`(() => {
      const pad = document.createElement("div");
      pad.id = "shadow-pad";
      pad.setAttribute("style", "position: fixed; left: 540px; top: 380px; width: 170px; height: 130px; z-index: 100; background-color: rgb(255, 255, 255)");
      const ring = document.createElement("div");
      ring.id = "ring";
      ring.setAttribute("style", "position: absolute; left: 20px; top: 20px; width: 80px; height: 40px; border-radius: 6px; background-color: transparent; box-shadow: 0 0 0 6px rgb(255, 0, 0)");
      const blur = document.createElement("div");
      blur.id = "blurred";
      blur.setAttribute("style", "position: absolute; left: 115px; top: 20px; width: 40px; height: 40px; background-color: rgb(0, 0, 255); filter: blur(2px)");
      pad.append(ring, blur);
      document.body.appendChild(pad);
      return 0;
    })()`);
    const r = await box("#ring");
    const b = await box("#blurred");
    const s = await until(shoot, (s) => red(s.at(r.x - 3, r.y + 20)));
    bunExpect(red(s.at(r.x - 3, r.y + 20))).toBe(true);
    bunExpect(red(s.at(r.x + 40, r.y + r.h + 3))).toBe(true);
    // The middle is the white pad, not the ring colour.
    bunExpect(white(s.at(r.x + 40, r.y + 20))).toBe(true);
    bunExpect(white(s.at(r.x + 3, r.y + 3))).toBe(true);
    bunExpect(blue(s.at(b.x + 20, b.y + 20))).toBe(true);
    await app.eval(`(document.getElementById("shadow-pad").remove(), 0)`);
  }, T);
});
