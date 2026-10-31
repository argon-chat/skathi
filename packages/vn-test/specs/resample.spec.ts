// Images drawn far below their own size (m26 client sweep #22). gpui's sprite
// atlas keeps one mip level and samples it linearly, so the Argon client's
// 1080 px avatars at 40 px aliased. The host now draws such an image from a
// filtered copy at the size it is shown (crates/vn-tree/src/resample.rs,
// docs/RUNTIME.md §5.16).
//
//   bun test packages/vn-test/specs/resample.spec.ts
//
// `fixtures/resample` shows one-pixel checkerboards at 40 x 40 on white, and a
// 50% grey reference. Filtered, a checkerboard is that grey; unfiltered at
// 27:1 the sampler lands on texel centres and it is black and white noise.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const W = 360;
const app = await launch({
  app: "packages/vn-test/fixtures/resample/src/main.ts",
  window: [W, 100],
  label: "resample",
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

/** Every device pixel of `b`, inset by `inset` CSS px (the edges blend into
 *  the white page), from one screenshot. */
async function pixels(b: Bounds, inset = 4): Promise<Rgb[]> {
  const shot = readPng(await app.screenshot());
  const k = shot.width / W;
  const out: Rgb[] = [];
  const [x0, y0] = [Math.ceil((b.x + inset) * k), Math.ceil((b.y + inset) * k)];
  const [x1, y1] = [Math.floor((b.x + b.w - inset) * k), Math.floor((b.y + b.h - inset) * k)];
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) out.push(shot.at(x, y));
  return out;
}

interface Stats {
  n: number;
  mean: number;
  sd: number;
  /** The largest distance of any channel from the reference grey. */
  worst: number;
}

function stats(px: Rgb[], ref: number): Stats {
  const v = px.flatMap(([r, g, b]) => [r, g, b]);
  const mean = v.reduce((a, b) => a + b, 0) / v.length;
  const sd = Math.sqrt(v.reduce((a, b) => a + (b - mean) ** 2, 0) / v.length);
  const worst = v.reduce((a, b) => Math.max(a, Math.abs(b - ref)), 0);
  return { n: px.length, mean, sd, worst };
}

/** The reference grey, as the screenshot has it. */
async function reference(): Promise<number> {
  const s = stats(await pixels(await box("#ref")), 0);
  bunExpect(s.sd).toBeLessThan(1);
  return s.mean;
}

/** Polls the probe until it is flat grey (or the time is up) and returns the
 *  last reading: images decode, and `#big`'s copy lands, asynchronously. */
async function settle(selector: string, ms = 8_000): Promise<Stats> {
  const ref = await reference();
  const b = await box(selector);
  bunExpect(b).toMatchObject({ w: 40, h: 40 });
  const deadline = Date.now() + ms;
  let s = stats(await pixels(b), ref);
  while ((s.worst > 12 || s.sd > 4) && Date.now() < deadline) {
    await sleep(100);
    s = stats(await pixels(b), ref);
  }
  console.log(selector, JSON.stringify({ ref, ...s }));
  return s;
}

const flat = (s: Stats, ref: number) => {
  bunExpect(s.n).toBeGreaterThan(400);
  bunExpect(Math.abs(s.mean - ref)).toBeLessThan(4);
  bunExpect(s.sd).toBeLessThan(4);
  bunExpect(s.worst).toBeLessThan(12);
};

describe("a 1080 px checkerboard at 40 px is the reference grey", () => {
  test("<img>", async () => {
    flat(await settle("#img"), await reference());
  }, T);

  test("CSS background-image", async () => {
    flat(await settle("#bg"), await reference());
  }, T);

  test("a still <canvas>", async () => {
    flat(await settle("#cv"), await reference());
  }, T);

  test("a 1531 px <img>, whose copy is made off the painting thread", async () => {
    flat(await settle("#big"), await reference());
  }, T);
});
