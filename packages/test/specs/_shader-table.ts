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
  casesPerEffect?: number;
}

export async function shaderTable(table: Table): Promise<void> {
  const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;
  const perFrame = table.casesPerEffect ? TILES * table.casesPerEffect : table.cases.length;
  for (let start = 0; start < table.cases.length; start += perFrame) {
    const cases = table.cases.slice(start, start + perFrame);
    const range = cases.length < table.cases.length ? ` ${start + 1}–${start + cases.length}` : "";
    await shaderFrame(table, cases, range, compiler);
  }
}

async function shaderFrame(
  { label, imports, declarations = "" }: Table,
  cases: Case[],
  range: string,
  compiler: string | null,
): Promise<void> {
  const per = Math.ceil(cases.length / TILES);
  const chunks = Array.from({ length: TILES }, (_, i) => cases.slice(i * per, (i + 1) * per));
  const scratch = mkdtempSync(join(tmpdir(), `sk-${label}-`));
  const effectsDir = join(scratch, "effects");
  if (compiler && SCREENSHOTS) {
    mkdirSync(join(scratch, "shaders"));
    chunks.forEach((chunk, i) =>
      writeFileSync(
        join(scratch, "shaders", `table${i}.slang`),
        tableEffect(`table${i}`, chunk, imports, declarations),
      ),
    );
    writeEffectFiles(effectsDir, (await compileAppEffects(scratch, compiler)).files);
  }
  afterAll(() => rmSync(scratch, { recursive: true, force: true }));

  async function results(app: SkApp, scale: number): Promise<number[][][]> {
    const shot = readPng(await app.screenshot(undefined, "render_to_image"));
    return chunks.map((chunk, i) => {
      const top = Math.round(i * TILE_HEIGHT * scale);
      if (effectHeight(chunk.length) > TILE_HEIGHT * scale) throw new Error(`tile ${i} is too short for its cases`);
      try {
        return decode(chunk.length, (x, y) => shot.at(x, top + y));
      } catch (e) {
        throw new Error(`tile ${i}: ${e instanceof Error ? e.message : String(e)}`);
      }
    });
  }

  // Each attempt (bun test --retry) launches its own host, so a retry never inherits a broken one, and
  // decodes only a frame that two screenshots in a row agree on.
  test.skipIf(!compiler || !SCREENSHOTS)(
    `${imports[imports.length - 1]}${range} matches the float64 reference on the GPU, case by case`,
    async () => {
      const app = await launch({
        app: "packages/test/fixtures/shader-table/src/main.ts",
        window: [W, TILES * TILE_HEIGHT],
        label,
        env: { SK_EFFECTS_DIR: effectsDir, SK_WINDOW_FOCUS: "never" },
      });
      try {
        const scale = Number(await app.eval("devicePixelRatio"));
        let tiles: number[][][] | null = null;
        let last: unknown = null;
        const deadline = Date.now() + 30_000;
        while (!tiles && Date.now() < deadline) {
          try {
            const first = await results(app, scale);
            const second = await results(app, scale);
            const unstable = first.flatMap((t, i) => (Bun.deepEquals(t, second[i]) ? [] : [i]));
            if (unstable.length)
              throw new Error(`tiles ${unstable.join(", ")} differ between two screenshots in a row`);
            tiles = first;
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
      } finally {
        await app.close();
      }
    },
    T,
  );
}
