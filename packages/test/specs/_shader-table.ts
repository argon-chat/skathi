// Runs a shader-library golden table end to end: one effect per tile of fixtures/shader-table, each
// encoding its cases' results into pixels, read back from a screenshot and compared case by case.

import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { type Case, decode, effectHeight, misses, tableEffect } from "../../cli/test/shader-table";
import { launch, readPng, SCREENSHOTS, type SkApp } from "../src/index";

const W = 300;
const TILES = 8;
const TILE_HEIGHT = 60;
const T = 90_000;

export interface Table {
  label: string;
  imports: string[];
  declarations?: string;
  cases: Case[];
}

export async function shaderTable({ label, imports, declarations = "", cases }: Table): Promise<void> {
  const per = Math.ceil(cases.length / TILES);
  const chunks = Array.from({ length: TILES }, (_, i) => cases.slice(i * per, (i + 1) * per));
  const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;
  const scratch = mkdtempSync(join(tmpdir(), `sk-${label}-`));
  const effectsDir = join(scratch, "effects");
  let app: SkApp | null = null;
  if (compiler && SCREENSHOTS) {
    mkdirSync(join(scratch, "shaders"));
    chunks.forEach((chunk, i) =>
      writeFileSync(
        join(scratch, "shaders", `table${i}.slang`),
        tableEffect(`table${i}`, chunk, imports, declarations),
      ),
    );
    writeEffectFiles(effectsDir, (await compileAppEffects(scratch, compiler)).files);
    app = await launch({
      app: "packages/test/fixtures/shader-table/src/main.ts",
      window: [W, TILES * TILE_HEIGHT],
      label,
      env: { SK_EFFECTS_DIR: effectsDir, SK_WINDOW_FOCUS: "never" },
    });
  }
  afterAll(() => {
    app?.close();
    rmSync(scratch, { recursive: true, force: true });
  });

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
    `${imports[imports.length - 1]} matches the float64 reference on the GPU, case by case`,
    async () => {
      let tiles: number[][][] | null = null;
      let last: unknown = null;
      const deadline = Date.now() + 30_000;
      while (!tiles && Date.now() < deadline) {
        try {
          tiles = await results();
        } catch (e) {
          last = e;
          await new Promise((r) => setTimeout(r, 200));
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
}
