import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { launch, readPng, SCREENSHOTS, type SkApp } from "../src/index";

const W = 960;
const H = 360;
const T = 90_000;
const fixture = resolve(import.meta.dir, "../fixtures/shader-glass");
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

let app: SkApp | null = null;
const effectsDir = mkdtempSync(join(tmpdir(), "sk-shader-glass-"));
if (compiler && SCREENSHOTS) {
  writeEffectFiles(effectsDir, (await compileAppEffects(fixture, compiler)).files);
  app = await launch({
    app: "packages/test/fixtures/shader-glass/src/main.ts",
    window: [W, H],
    label: "shader-glass",
    env: { SK_EFFECTS_DIR: effectsDir, SK_WINDOW_FOCUS: "never" },
  });
}
afterAll(() => {
  app?.close();
  rmSync(effectsDir, { recursive: true, force: true });
});
const scale = app ? Number(await app.eval("devicePixelRatio")) : 1;

type Shot = ReturnType<typeof readPng>;

/**
 * The largest channel difference between two cells' elements over 8 px block means, `inset` logical
 * pixels in from the element's edge. The ramp is dithered per pixel, so single pixels differ by up to 6.
 */
function diff(shot: Shot, a: [number, number], b: [number, number], inset = 0): number {
  const at = (cell: [number, number], x: number, y: number) =>
    shot.at(Math.round((cell[1] * 120 + x) * scale), Math.round((cell[0] * 120 + y) * scale));
  let worst = 0;
  for (let by = 20 + inset; by + 8 <= 100 - inset; by += 8)
    for (let bx = 20 + inset; bx + 8 <= 100 - inset; bx += 8) {
      const sum = [0, 0, 0];
      for (let y = by; y < by + 8; y++)
        for (let x = bx; x < bx + 8; x++) {
          const p = at(a, x, y);
          const q = at(b, x, y);
          for (let i = 0; i < 3; i++) sum[i]! += p[i]! - q[i]!;
        }
      worst = Math.max(worst, ...sum.map((s) => Math.abs(s) / 64));
    }
  return worst;
}

/** The largest channel difference of a cell's element from a colour. */
function from(shot: Shot, cell: [number, number], rgb: [number, number, number]): number {
  let worst = 0;
  for (let y = 20; y < 100; y++)
    for (let x = 20; x < 100; x++) {
      const p = shot.at(Math.round((cell[1] * 120 + x) * scale), Math.round((cell[0] * 120 + y) * scale));
      worst = Math.max(worst, ...p.map((c, i) => Math.abs(c - rgb[i]!)));
    }
  return worst;
}

async function shot(): Promise<Shot> {
  // Effects compile on first draw; wait until the last cell's effect has replaced the plain backdrop.
  const deadline = Date.now() + 60_000;
  let s = readPng(await app!.screenshot(undefined, "render_to_image"));
  while (from(s, [2, 7], [0x3a, 0x7b, 0xd5]) === 0 && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 250));
    s = readPng(await app!.screenshot(undefined, "render_to_image"));
  }
  return s;
}

test.skipIf(!compiler || !SCREENSHOTS)(
  "the showcase glass and lens draw the same with sk.light and sk.sample as by hand",
  async () => {
    const s = await shot();
    expect(diff(s, [0, 1], [0, 2])).toBeLessThanOrEqual(1.5);
    expect(diff(s, [0, 3], [0, 4])).toBeLessThanOrEqual(1.5);
    expect(diff(s, [0, 1], [0, 0], 4)).toBeGreaterThan(4);
  },
  T,
);

test.skipIf(!compiler || !SCREENSHOTS)(
  "kernels over the engine's backdrop keep a ramp inside and a flat colour up to and past their reach",
  async () => {
    const s = await shot();
    const modes = Array.from({ length: 7 }, (_, mode) => mode);
    const ramp = modes.map((mode) => [mode, diff(s, [1, mode + 1], [1, 0], 12)]);
    const flat = modes.map((mode) => [mode, from(s, [2, mode], [0x3a, 0x7b, 0xd5])]);
    expect(ramp.filter(([, d]) => d! > 1.5)).toEqual([]);
    expect(flat.filter(([, d]) => d! > 1)).toEqual([]);
    expect(from(s, [2, 7], [0x3a, 0x7b, 0xd5])).toBeGreaterThan(4);
  },
  T,
);
