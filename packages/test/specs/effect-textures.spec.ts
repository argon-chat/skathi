import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { launch, readPng, SCREENSHOTS, type SkApp } from "../src/index";

const W = 360;
const T = 30_000;
const fixture = resolve(import.meta.dir, "../fixtures/effect-textures");
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

let app: SkApp | null = null;
const effectsDir = mkdtempSync(join(tmpdir(), "sk-effect-textures-"));
if (compiler) {
  writeEffectFiles(effectsDir, (await compileAppEffects(fixture, compiler)).files);
  app = await launch({
    app: "packages/test/fixtures/effect-textures/src/main.ts",
    window: [W, 120],
    label: "effect-textures",
    env: { SK_EFFECTS_DIR: effectsDir },
  });
}
afterAll(() => {
  app?.close();
  rmSync(effectsDir, { recursive: true, force: true });
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

async function at(selector: string, fx: number): Promise<Rgb> {
  const b = (await app!.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  const shot = readPng(await app!.screenshot());
  const k = shot.width / W;
  return shot.at(Math.round((b.x + b.w * fx) * k), Math.round((b.y + b.h / 2) * k));
}

const near = (got: Rgb, want: Rgb) => got.every((c, i) => Math.abs(c - want[i]!) <= 4);

async function settle(selector: string, fx: number, want: Rgb): Promise<Rgb> {
  const deadline = Date.now() + 5_000;
  let got = await at(selector, fx);
  while (!near(got, want) && Date.now() < deadline) {
    await sleep(100);
    got = await at(selector, fx);
  }
  return got;
}

test.skipIf(!compiler || !SCREENSHOTS)(
  "a colour image samples premultiplied, an [SkLinear] one as stored, a missing one as its default",
  async () => {
    expect(await settle("#colour", 0.1, [128, 0, 0])).toSatisfy((c) => near(c, [128, 0, 0]));
    expect(await at("#colour", 0.9)).toSatisfy((c) => near(c, [0, 0, 255]));
    expect(await settle("#linear", 0.1, [255, 0, 32])).toSatisfy((c) => near(c, [255, 0, 32]));
    expect(await settle("#missing", 0.5, [0, 255, 0])).toSatisfy((c) => near(c, [0, 255, 0]));
    expect(await settle("#other", 0.5, [255, 255, 0])).toSatisfy((c) => near(c, [255, 255, 0]));
  },
  T,
);
