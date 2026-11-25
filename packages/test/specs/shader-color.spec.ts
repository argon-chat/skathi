import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { caseEffect, cases, decode, effectHeight, misses } from "../../cli/test/sk-color-table";
import { launch, readPng, SCREENSHOTS, type SkApp } from "../src/index";

const W = 300;
const TILES = 6;
const TILE_HEIGHT = 40;
const T = 60_000;
const all = cases();
const per = Math.ceil(all.length / TILES);
const chunks = Array.from({ length: TILES }, (_, i) => all.slice(i * per, (i + 1) * per));
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

const scratch = mkdtempSync(join(tmpdir(), "sk-shader-color-"));
const effectsDir = join(scratch, "effects");
let app: SkApp | null = null;
if (compiler && SCREENSHOTS) {
  mkdirSync(join(scratch, "shaders"));
  chunks.forEach((chunk, i) =>
    writeFileSync(join(scratch, "shaders", `color${i}.slang`), caseEffect(`color${i}`, chunk)),
  );
  writeEffectFiles(effectsDir, (await compileAppEffects(scratch, compiler)).files);
  app = await launch({
    app: "packages/test/fixtures/shader-color/src/main.ts",
    window: [W, TILES * TILE_HEIGHT],
    label: "shader-color",
    env: { SK_EFFECTS_DIR: effectsDir, SK_WINDOW_FOCUS: "never" },
  });
}
afterAll(() => {
  app?.close();
  rmSync(scratch, { recursive: true, force: true });
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function results(): Promise<number[][][]> {
  const shot = readPng(await app!.screenshot(undefined, "render_to_image"));
  const k = shot.width / W;
  return chunks.map((chunk, i) => {
    const top = Math.round(i * TILE_HEIGHT * k);
    if (effectHeight(chunk.length) > TILE_HEIGHT * k) throw new Error(`tile ${i} is too short for its cases`);
    return decode(chunk.length, (x, y) => shot.at(x, top + y));
  });
}

test.skipIf(!compiler || !SCREENSHOTS)(
  "sk.color matches the float64 reference on the GPU, case by case",
  async () => {
    let tiles: number[][][] | null = null;
    let last: unknown = null;
    const deadline = Date.now() + 20_000;
    while (!tiles && Date.now() < deadline) {
      try {
        tiles = await results();
      } catch (e) {
        last = e;
        await sleep(200);
      }
    }
    if (!tiles) throw last;
    const failures = chunks.flatMap((chunk, i) =>
      chunk.flatMap((c, j) => {
        const bad = misses(c, tiles![i]![j]!);
        return bad.length ? [`${c.name}: ${bad.join(", ")}`] : [];
      }),
    );
    expect(failures).toEqual([]);
  },
  T,
);
