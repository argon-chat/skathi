import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Bounds, launch, readPng, type SkApp, WINDOWS } from "../src/index";

// `text-contrast: luminance` gives light text on a dark ground more weight and leaves dark text alone.
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** How far the text in `b` is from its background, summed over its pixels. */
function ink(file: string, b: Bounds, scale: number): number {
  const png = readPng(file);
  const x0 = Math.floor(b.x * scale);
  const y0 = Math.floor(b.y * scale);
  const bg = png.luma(x0, y0);
  let sum = 0;
  for (let y = y0; y < Math.ceil((b.y + b.h) * scale); y++)
    for (let x = x0; x < Math.ceil((b.x + b.w) * scale); x++) sum += Math.abs(png.luma(x, y) - bg);
  return sum;
}

for (const scale of WINDOWS ? [1, 2] : []) {
  describe(`at scale ${scale}`, () => {
    let app: SkApp;
    beforeAll(async () => {
      app = await launch({ app: "packages/test/fixtures/text-contrast/src/main.ts", scale, label: "text-contrast" });
    }, 60_000);
    afterAll(() => app.close());

    for (const smoothing of ["antialiased", "subpixel"]) {
      test(`${smoothing}: light text on dark gains ink, dark text on light keeps it`, async () => {
        await app.eval(`__SK_TEST_CONTRAST__.smooth(${JSON.stringify(smoothing)}); 'ok'`);
        await sleep(400);
        const [darkAuto, darkLum, lightAuto, lightLum] = (await app.query(".line")).nodes.flatMap((n) =>
          n.bounds ? [n.bounds] : [],
        );
        const file = join(tmpdir(), `sk-text-contrast-${process.pid}.png`);
        await app.send("screenshot", { path: file, backend: "render_to_image" });
        const dark = ink(file, darkLum!, scale) / ink(file, darkAuto!, scale);
        const light = ink(file, lightLum!, scale) / ink(file, lightAuto!, scale);
        console.log(
          `scale ${scale} ${smoothing}: light-on-dark ink x${dark.toFixed(3)}, dark-on-light x${light.toFixed(3)}`,
        );
        expect(dark).toBeGreaterThan(1.05);
        expect(Math.abs(light - 1)).toBeLessThan(0.005);
      });
    }
  });
}
