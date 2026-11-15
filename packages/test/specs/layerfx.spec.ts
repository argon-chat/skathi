import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launch, readPng, type Bounds } from "../src/index";

const REPO = join(import.meta.dir, "../../..");
const effectsDir = mkdtempSync(join(tmpdir(), "sk-layerfx-"));
const built = Bun.spawnSync(
  ["bun", "scripts/shaders.ts", "--app", "packages/test/fixtures/layerfx", "--out", effectsDir],
  { cwd: REPO, stdout: "pipe", stderr: "pipe" },
);
if (built.exitCode !== 0) throw new Error(`compiling the fixture's effects failed:\n${built.stderr.toString()}`);

const app = await launch({
  app: "packages/test/fixtures/layerfx/src/main.ts",
  window: [360, 200],
  label: "layerfx",
  env: { SK_EFFECTS_DIR: effectsDir },
});
afterAll(async () => {
  await app.close();
  rmSync(effectsDir, { recursive: true, force: true });
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Rgb = [number, number, number];

const box = async (selector: string): Promise<Bounds> => {
  const b = (await app.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

const pixel = async (b: Bounds, dx: number, dy: number): Promise<Rgb> => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 360;
  return shot.at(Math.floor((b.x + dx + 0.5) * k), Math.floor((b.y + dy + 0.5) * k));
};

async function until(read: () => Promise<Rgb>, ok: (rgb: Rgb) => boolean, ms = 5_000): Promise<Rgb> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(100);
    v = await read();
  }
  return v;
}

const near = (rgb: Rgb, expected: Rgb, slack = 24) => rgb.every((c, i) => Math.abs(c - expected[i]!) <= slack);

test(
  "a content effect draws the element and its children in their place, shaded",
  async () => {
    const b = await box("#swapped");
    expect(
      near(
        await until(
          () => pixel(b, 8, 8),
          (rgb) => near(rgb, [0, 0, 255]),
        ),
        [0, 0, 255],
      ),
    ).toBe(true);
    expect(near(await pixel(b, 40, 30), [0, 255, 0])).toBe(true);
  },
  T,
);

test(
  "opacity applies once, to the shaded result",
  async () => {
    const b = await box("#faded");
    expect(
      near(
        await until(
          () => pixel(b, 8, 8),
          (rgb) => near(rgb, [128, 128, 255]),
        ),
        [128, 128, 255],
      ),
    ).toBe(true);
  },
  T,
);

test(
  "a history carries its state from frame to frame",
  async () => {
    const b = await box("#counter");
    const [r, g, bl] = await until(
      () => pixel(b, 40, 30),
      ([r]) => r > 200,
      10_000,
    );
    expect([r > 200, g < 30, bl < 30]).toEqual([true, true, true]);
  },
  T,
);
