import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { launch, readPng, SCREENSHOTS, type SkApp } from "../src/index";

const W = 360;
const T = 30_000;
const fixture = resolve(import.meta.dir, "../fixtures/backdrop-reach");
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

let app: SkApp | null = null;
const effectsDir = mkdtempSync(join(tmpdir(), "sk-backdrop-reach-"));
if (compiler) {
  writeEffectFiles(effectsDir, (await compileAppEffects(fixture, compiler)).files);
  app = await launch({
    app: "packages/test/fixtures/backdrop-reach/src/main.ts",
    window: [W, 200],
    label: "backdrop-reach",
    env: { SK_EFFECTS_DIR: effectsDir, SK_WINDOW_FOCUS: "never" },
  });
}
afterAll(() => {
  app?.close();
  rmSync(effectsDir, { recursive: true, force: true });
});

type Rgb = [number, number, number];
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const near = (got: Rgb, want: Rgb) => got.every((c, i) => Math.abs(c - want[i]!) <= 4);
const GREEN: Rgb = [0, 255, 0];
const RED: Rgb = [255, 0, 0];
const BLUE: Rgb = [0, 0, 255];
const STEPS = Array.from({ length: 19 }, (_, i) => (i + 0.5) / 19);

/** The probe's middle row at each of `STEPS` across it, from one screenshot. */
async function row(shot: ReturnType<typeof readPng>, selector: string): Promise<Rgb[]> {
  const b = (await app!.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  const k = shot.width / W;
  return STEPS.map((fx) => shot.at(Math.round((b.x + b.w * fx) * k), Math.round((b.y + b.h / 2) * k)));
}

async function rows(): Promise<Record<string, Rgb[]>> {
  const deadline = Date.now() + 5_000;
  for (;;) {
    const shot = readPng(await app!.screenshot());
    const out: Record<string, Rgb[]> = {};
    for (const id of ["within-reach", "within-free", "beyond-reach", "beyond-free"])
      out[id] = await row(shot, `#${id}`);
    if (near(out["beyond-free"]!.at(-1)!, BLUE) || Date.now() > deadline) return out;
    await sleep(100);
  }
}

test.skipIf(!compiler || !SCREENSHOTS)(
  "an effect reading within its declared reach draws what it draws unbounded",
  async () => {
    const { "within-reach": bounded, "within-free": free } = await rows();
    expect(free!.some((c) => near(c, GREEN)) && free!.some((c) => near(c, RED))).toBe(true);
    expect(bounded!.every((c, i) => c.every((v, j) => Math.abs(v - free![i]![j]!) <= 2))).toBe(true);
  },
  T,
);

test.skipIf(!compiler || !SCREENSHOTS)(
  "a read beyond the reach is the backdrop at its edge",
  async () => {
    const { "beyond-reach": bounded, "beyond-free": free } = await rows();
    expect(near(free!.at(-1)!, BLUE)).toBe(true);
    expect(near(bounded!.at(-1)!, RED)).toBe(true);
    expect(near(bounded![0]!, GREEN) && near(free![0]!, GREEN)).toBe(true);
  },
  T,
);
