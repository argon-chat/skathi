import { beforeAll, describe, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Bounds, featureExe, launch, type Png, readPng, WINDOWS } from "../src/index";

// `"textRendering"`: skathi-mode draws grayscale, emboldened by the text's luminance with the
// baseline kept; optimized keeps DirectWrite with ClearType. Emoji are DirectWrite's in both.
const T = 600_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const MODES = ["skathi-mode", "optimized"] as const;
type Mode = (typeof MODES)[number];

interface Shot {
  png: Png;
  lines: Record<string, Bounds>;
}

async function shoot(mode: Mode, scale: number, exe?: string): Promise<Shot> {
  const app = await launch({
    app: "packages/test/fixtures/text-rendering/src/main.ts",
    scale,
    label: `text-rendering-${mode}`,
    env: { SK_TEXT_RENDERING: mode },
    ...(exe && { exe }),
  });
  try {
    await sleep(400);
    const lines: Record<string, Bounds> = {};
    for (const node of (await app.query(".line")).nodes) {
      const name = (node.classes ?? []).find((c) => c !== "line");
      if (name && node.bounds) lines[name] = node.bounds;
    }
    const file = join(tmpdir(), `sk-text-rendering-${mode}-${process.pid}.png`);
    await app.send("screenshot", { path: file, backend: "render_to_image" });
    return { png: readPng(file), lines };
  } finally {
    await app.close();
  }
}

function pixels(shot: Shot, line: string, scale: number): { x: number; y: number }[] {
  const b = shot.lines[line]!;
  const out = [];
  for (let y = Math.floor(b.y * scale); y < Math.ceil((b.y + b.h) * scale); y++)
    for (let x = Math.floor(b.x * scale); x < Math.ceil((b.x + b.w) * scale); x++) out.push({ x, y });
  return out;
}

function background(shot: Shot, line: string, scale: number): number {
  const b = shot.lines[line]!;
  return shot.png.luma(Math.floor(b.x * scale), Math.floor(b.y * scale));
}

function ink(shot: Shot, line: string, scale: number): number {
  const bg = background(shot, line, scale);
  return pixels(shot, line, scale).reduce((sum, { x, y }) => sum + Math.abs(shot.png.luma(x, y) - bg), 0);
}

/** The lowest row with a pixel at least half way from the background to the text. */
function lowestRow(shot: Shot, line: string, scale: number): number {
  const bg = background(shot, line, scale);
  let lowest = -1;
  for (const { x, y } of pixels(shot, line, scale))
    if (Math.abs(shot.png.luma(x, y) - bg) > 96) lowest = Math.max(lowest, y);
  return lowest;
}

function colouredPixels(shot: Shot, line: string, scale: number): number {
  return pixels(shot, line, scale).filter(({ x, y }) => {
    const [r, g, b] = shot.png.at(x, y);
    return Math.max(r, g, b) - Math.min(r, g, b) > 24;
  }).length;
}

function largestDifference(a: Shot, b: Shot, line: string, scale: number): number {
  let worst = 0;
  for (const { x, y } of pixels(a, line, scale)) {
    const [p, q] = [a.png.at(x, y), b.png.at(x, y)];
    for (let c = 0; c < 3; c++) worst = Math.max(worst, Math.abs(p[c]! - q[c]!));
  }
  return worst;
}

function specs(label: string, scales: number[], host: () => Promise<string | undefined>) {
  for (const scale of WINDOWS ? scales : []) {
    describe(`${label} at scale ${scale}`, () => {
      const shots = {} as Record<Mode, Shot>;
      beforeAll(async () => {
        const exe = await host();
        for (const mode of MODES) shots[mode] = await shoot(mode, scale, exe);
      }, T);

      test("light text on dark gains ink in skathi-mode", () => {
        const gain = ink(shots["skathi-mode"], "on-dark", scale) / ink(shots.optimized, "on-dark", scale);
        console.log(`${label} scale ${scale}: light-on-dark ink x${gain.toFixed(3)}`);
        expect(gain).toBeGreaterThan(1.05);
      });

      test("the baseline row stays", () => {
        for (const line of ["flat-dark", "flat-light"])
          expect(lowestRow(shots["skathi-mode"], line, scale)).toBe(lowestRow(shots.optimized, line, scale));
      });

      test("subpixel-antialiased draws ClearType only in optimized", () => {
        expect(colouredPixels(shots.optimized, "subpixel", scale)).toBeGreaterThan(50);
        expect(colouredPixels(shots["skathi-mode"], "subpixel", scale)).toBe(0);
      });

      test("emoji are drawn the same in both modes", () => {
        expect(largestDifference(shots["skathi-mode"], shots.optimized, "emoji", scale)).toBeLessThanOrEqual(1);
      });
    });
  }
}

specs("Direct3D 12", [1, 1.5], async () => undefined);
specs("Direct3D 11", [1], () => featureExe({ features: [], noDefaultFeatures: true }));
