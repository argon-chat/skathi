import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Bounds, launch, type Png, readPng, type SkApp, WINDOWS } from "../src/index";

// Where text lands at rest and how it moves: on whole device pixels with phases measured in device
// pixels at every scale, and as one rigid body under a transform, a scroll and a view transition.
const T = 60_000;
const FRAMES = 12;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const LOG = !!process.env.SK_TEXT_MOTION_LOG;
const MODE = process.env.SK_TEXT_RENDERING ?? "skathi-mode";

type Rect = { x0: number; y0: number; x1: number; y1: number };

const device = (b: Bounds, scale: number, pad = 0): Rect => ({
  x0: Math.floor((b.x - pad) * scale),
  y0: Math.floor((b.y - pad) * scale),
  x1: Math.ceil((b.x + b.w + pad) * scale),
  y1: Math.ceil((b.y + b.h + pad) * scale),
});

function profile(png: Png, r: Rect, axis: "x" | "y"): number[] {
  const out: number[] = [];
  const [a0, a1, b0, b1] = axis === "x" ? [r.x0, r.x1, r.y0, r.y1] : [r.y0, r.y1, r.x0, r.x1];
  for (let a = a0; a < a1; a++) {
    let ink = 0;
    for (let b = b0; b < b1; b++) ink += 255 - (axis === "x" ? png.luma(a, b) : png.luma(b, a));
    out.push(ink);
  }
  return out;
}

/** The ink centroid of each run of inked columns (or rows), in device pixels. */
function centroids(values: number[], from: number): number[] {
  const max = Math.max(...values);
  const out: number[] = [];
  let mass = 0;
  let moment = 0;
  values.forEach((v, i) => {
    if (v > max * 0.02) {
      mass += v;
      moment += v * (from + i + 0.5);
    } else if (mass > 0) {
      out.push(moment / mass);
      mass = 0;
      moment = 0;
    }
  });
  if (mass > 0) out.push(moment / mass);
  return out;
}

/** How much ink a pixel fully covered by the fixture's text colour (#111 on white) has. */
const FULL = 255 - 0x11;

/**
 * The ink centroid of each vertical stem in `r`, in device pixels, over the rows through the middle
 * of the stems, for a mask whose pixels hold the share of them the ink covers (skathi-mode's). A
 * partly covered pixel at either end of a stem is covered on its inner side, so its ink counts at
 * the middle of that part: at the pixel's centre it would move the centroid by up to a tenth of a
 * pixel for a stem 1.5 px wide as its edges cross pixels.
 */
function coveredCentres(png: Png, r: Rect): number[] {
  const inked = profile(png, r, "y").flatMap((v, i) => (v > 0 ? [r.y0 + i] : []));
  const from = inked[Math.floor(inked.length / 4)]!;
  const to = inked[Math.floor((inked.length * 3) / 4)]!;
  const mass: number[] = [];
  const moment: number[] = [];
  for (let y = from; y <= to; y++) {
    let stem = 0;
    let run: number[] = [];
    for (let x = r.x0; x <= r.x1; x++) {
      const v = x < r.x1 ? Math.min((255 - png.luma(x, y)) / FULL, 1) : 0;
      if (v > 0) {
        run.push(v);
        continue;
      }
      if (run.length === 0) continue;
      const x0 = x - run.length;
      const at = run.map((_, i) => x0 + i + 0.5);
      if (run.length > 1) {
        at[0] = x0 + 1 - run[0]! / 2;
        at[run.length - 1] = x - 1 + run.at(-1)! / 2;
      }
      run.forEach((v, i) => {
        mass[stem] = (mass[stem] ?? 0) + v;
        moment[stem] = (moment[stem] ?? 0) + v * at[i]!;
      });
      stem++;
      run = [];
    }
  }
  return moment.map((m, i) => m / mass[i]!);
}

/**
 * Stem centres in text drawn straight from glyph masks. A resampled image keeps the plain centroid
 * of its pixels exactly, so text under a scale or a layer is measured with that.
 */
const stemCentres = (png: Png, r: Rect) =>
  MODE === "skathi-mode" ? coveredCentres(png, r) : centroids(profile(png, r, "x"), r.x0);
const resampledCentres = (png: Png, r: Rect) => centroids(profile(png, r, "x"), r.x0);

const gaps = (c: number[]) => c.slice(1).map((v, i) => v - c[i]!);
const spread = (v: number[]) => Math.max(...v) - Math.min(...v);

/**
 * How much the spacing of adjacent stems changes from frame to frame once each frame's own scale is
 * divided out: zero for a rigid body.
 */
function wobble(frames: number[][]): number {
  const n = Math.min(...frames.map((f) => f.length));
  const rows = frames.map((f) => gaps(f.slice(0, n)));
  const unit = rows.map((g) => g.reduce((a, b) => a + b, 0) / g.length);
  let worst = 0;
  for (let i = 0; i < n - 1; i++) {
    const ratio = rows.map((g, f) => g[i]! / unit[f]!);
    worst = Math.max(worst, spread(ratio) * Math.min(...unit));
  }
  return worst;
}

for (const scale of WINDOWS ? [1, 1.25, 1.5, 2] : []) {
  describe(`at scale ${scale}`, () => {
    let app: SkApp;
    beforeAll(async () => {
      app = await launch({
        app: "packages/test/fixtures/text-motion/src/main.ts",
        scale,
        label: "text-motion",
        env: { SK_TEXT_RENDERING: MODE },
      });
    }, T);
    afterAll(() => app.close());

    const boxes = async (selector: string) =>
      (await app.query(selector)).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
    let shots = 0;
    const shoot = async () => {
      const file = join(tmpdir(), `sk-text-motion-${process.pid}-${shots++ % 2}.png`);
      await app.send("screenshot", { path: file, backend: "render_to_image" });
      return readPng(file);
    };
    const show = async (page: string) => {
      await app.eval(`__SK_TEST_MOTION__.show(${JSON.stringify(page)}); 'ok'`);
      await sleep(400);
    };

    test(
      "text at rest sits on a whole device pixel, its phases following its position in device pixels",
      async () => {
        await show("rest");
        const rows = await boxes(".at");
        expect(rows.length).toBe(10);
        const png = await shoot();
        // Row k spaces its characters k/100 CSS px wider than row 0, so its stem i (character 2i) is
        // 2ik/100 CSS px further right. Less that, each stem sits at a_k + b i with b the same for
        // every row; what is left is where the stem's phase put it.
        const stems = rows.map((b, k) => {
          const r = device(b, scale);
          return stemCentres(png, r).map((x, i) => x - ((2 * i * k) / 100) * scale);
        });
        expect(stems.every((xs) => xs.length === 10)).toBe(true);
        const mean = (v: number[]) => v.reduce((a, b) => a + b, 0) / v.length;
        let num = 0;
        let den = 0;
        for (const ys of stems) {
          const m = mean(ys);
          ys.forEach((y, i) => {
            num += (i - 4.5) * (y - m);
            den += (i - 4.5) ** 2;
          });
        }
        const pitch = num / den;
        let error = 0;
        for (const ys of stems) {
          const a = mean(ys.map((y, i) => y - pitch * i));
          ys.forEach((y, i) => (error = Math.max(error, Math.abs(y - a - pitch * i))));
        }
        // A line whose baseline is between two device pixels has a bottom row of partial ink under the
        // stems of its Hs; on the pixel grid the row under the baseline is as dark as the rows above it.
        const [block] = await boxes(".baselines");
        const ys = profile(png, device(block!, scale), "y");
        const max = Math.max(...ys);
        const bottoms = ys.flatMap((v, i) => (v > max * 0.02 && !(ys[i + 1]! > max * 0.02) ? [i] : []));
        expect(bottoms.length).toBe(10);
        const crisp = bottoms.map((i) => ys[i]! / ys[i - 2]!);
        if (LOG)
          console.log(
            `scale ${scale}: baseline rows ${crisp.map((c) => c.toFixed(2)).join(" ")}; x error ${error.toFixed(3)}`,
          );
        for (const c of crisp) expect(c).toBeGreaterThan(0.85);
        expect(error).toBeLessThan(0.4);
      },
      T,
    );

    test(
      "text in a scale or translate animation moves as one body: adjacent glyphs keep their spacing",
      async () => {
        await show("motion");
        const lanes = (await boxes(".lane")).map((b) => device(b, scale));
        expect(lanes.length).toBe(4);
        // The slide lane is drawn from glyph masks at each step; the others are resampled.
        const measure = [resampledCentres, stemCentres, resampledCentres, resampledCentres];
        const frames: number[][][] = lanes.map(() => []);
        for (let i = 0; i < FRAMES; i++) {
          const png = await shoot();
          lanes.forEach((r, k) => frames[k]!.push(measure[k]!(png, r)));
          await sleep(37 + ((i * 53) % 41));
        }
        const [pulse, slide, layer, spin] = frames.map(wobble);
        if (LOG)
          console.log(
            `scale ${scale}: spacing wobble pulse ${pulse!.toFixed(3)} slide ${slide!.toFixed(3)} will-change ${layer!.toFixed(3)} translate+scale ${spin!.toFixed(3)}`,
          );
        // A translated run keeps its spacing exactly; a resampled scale adds the filter's own bias on
        // narrow stems, about 0.15 px.
        expect(slide).toBeLessThan(0.15);
        expect(layer).toBeLessThan(0.15);
        expect(pulse).toBeLessThan(0.3);
        expect(spin).toBeLessThan(0.3);
      },
      T,
    );

    test(
      "lines of a text scrolled by fractional steps keep their distances",
      async () => {
        await show("scroll");
        const [box] = await boxes("#scroller");
        const r = device({ ...box!, y: box!.y + 20, h: box!.h - 40 }, scale);
        let reference: number[] = [];
        let worst = 0;
        let off = 0;
        for (let i = 0; i < FRAMES; i++) {
          const top = 30 + i * 0.37;
          await app.eval(`__SK_TEST_MOTION__.scroll(${top}); 'ok'`);
          await sleep(120);
          // The first and last line may be cut by the crop.
          const lines = centroids(profile(await shoot(), r, "y"), r.y0).slice(1, -1);
          if (i === 0) {
            reference = lines;
            continue;
          }
          const shift = (Number(await app.eval("document.getElementById('scroller').scrollTop")) - 30) * scale;
          const residual = lines.flatMap((c) => {
            const near = reference.find((c0) => Math.abs(c + shift - c0) < 17 * scale * 0.4);
            return near === undefined ? [] : [c + shift - near];
          });
          worst = Math.max(worst, spread(residual));
          off = Math.max(off, ...residual.map(Math.abs));
        }
        if (LOG)
          console.log(
            `scale ${scale}: scroll line-gap wobble ${worst.toFixed(3)}, off the scroll position ${off.toFixed(3)}`,
          );
        expect(worst).toBeLessThan(0.15);
      },
      T,
    );

    test(
      "text in a view transition moves as one body",
      async () => {
        await show("vt");
        await app.eval("__SK_TEST_MOTION__.transition().then(() => 'ok')");
        const frames: number[][] = [];
        for (let i = 0; i < FRAMES; i++) {
          const png = await shoot();
          const rows = profile(png, { x0: 0, y0: 0, x1: png.width, y1: png.height }, "y");
          const ys = rows.flatMap((v, y) => (v > 0 ? [y] : []));
          const band = { x0: 0, y0: Math.min(...ys), x1: png.width, y1: Math.max(...ys) + 1 };
          frames.push(centroids(profile(png, band, "x"), 0));
          await sleep(97 + ((i * 53) % 41));
        }
        const vt = wobble(frames);
        if (LOG) console.log(`scale ${scale}: view transition spacing wobble ${vt.toFixed(3)}`);
        expect(vt).toBeLessThan(0.15);
      },
      T,
    );
  });
}
