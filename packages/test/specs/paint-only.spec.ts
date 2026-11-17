import { afterAll, expect, test } from "bun:test";
import { type Bounds, launch, ON_SCREEN, readPng, SCREENSHOTS, WINDOWS } from "../src/index";

const W = 360;
const app = await launch({
  app: "packages/test/fixtures/paint-only/src/main.ts",
  window: [W, 140],
  label: "paint-only",
});
afterAll(() => app.close());

const T = 60_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function counts(): Promise<{ builds: number; presents: number }> {
  const p = (await app.send("perf")) as { builds: number; presents: number };
  return { builds: p.builds, presents: p.presents };
}

async function box(selector: string): Promise<Bounds> {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
}

async function centre(b: Bounds, backend: typeof ON_SCREEN | "render_to_image"): Promise<[number, number, number]> {
  const shot = readPng(await app.screenshot(undefined, backend));
  const k = shot.width / W;
  return shot.at(Math.round((b.x + b.w / 2) * k), Math.round((b.y + b.h / 2) * k));
}

async function digest(b: Bounds): Promise<string> {
  const shot = readPng(await app.screenshot(undefined, ON_SCREEN));
  const k = shot.width / W;
  const out: number[] = [];
  for (let y = Math.ceil(b.y * k) + 2; y < Math.floor((b.y + b.h) * k) - 2; y += 3)
    for (let x = Math.ceil(b.x * k) + 2; x < Math.floor((b.x + b.w) * k) - 2; x += 3) out.push(...shot.at(x, y));
  return out.join(",");
}

const near = (got: [number, number, number], want: [number, number, number]) =>
  got.every((v, i) => Math.abs(v - want[i]!) <= 8);

test.skipIf(!SCREENSHOTS || !WINDOWS)(
  "a playing, visible <video> is shown by presents, not builds",
  async () => {
    const deadline = Date.now() + 15_000;
    while (Number(await app.eval("document.getElementById('vid').currentTime")) <= 0.2) {
      if (Date.now() > deadline) throw new Error("the clip never started playing");
      await sleep(100);
    }
    await sleep(300);
    const vid = await box("#vid");
    const before = await counts();
    const first = await digest(vid);
    await sleep(1000);
    const second = await digest(vid);
    const after = await counts();
    const presents = after.presents - before.presents;
    const builds = after.builds - before.builds;
    console.log(`[paint-only] video, 1 s: ${presents} presents, ${builds} builds`);
    // ~24-30 decoded frames a second; the window is not focused, so gpui may hold it near 30 fps.
    expect(presents).toBeGreaterThanOrEqual(10);
    expect(builds).toBeLessThanOrEqual(1);
    expect(second).not.toBe(first);
  },
  T,
);

// Presents are counted by the Windows compositor path only.
test.skipIf(!SCREENSHOTS || !WINDOWS)(
  "a canvas that keeps drawing is shown by presents, with its newest frame",
  async () => {
    await app.eval("document.getElementById('vid').pause(); 'ok'");
    await sleep(300);
    const cv = await box("#cv");
    await app.eval("__SK_TEST_PAINT__.start('#ff0000'); 'ok'");
    await sleep(500);
    expect(near(await centre(cv, ON_SCREEN), [255, 0, 0])).toBe(true);

    const before = await counts();
    await app.eval("__SK_TEST_PAINT__.state.color = '#00ff00'; 'ok'");
    await sleep(600);
    const after = await counts();
    const presents = after.presents - before.presents;
    const builds = after.builds - before.builds;
    console.log(`[paint-only] canvas, 0.6 s: ${presents} presents, ${builds} builds`);
    expect(Number(await app.eval("__SK_TEST_PAINT__.state.frames"))).toBeGreaterThan(20);
    expect(presents).toBeGreaterThanOrEqual(10);
    expect(builds).toBeLessThanOrEqual(1);
    // The colour changed with no build: the scene drawn before it samples the tile the new frames
    // went into, and the window presented it.
    expect(near(await centre(cv, "render_to_image"), [0, 255, 0])).toBe(true);
    expect(near(await centre(cv, ON_SCREEN), [0, 255, 0])).toBe(true);

    await app.eval("__SK_TEST_PAINT__.stop(); 'ok'");
    await sleep(800);
    expect(near(await centre(cv, ON_SCREEN), [0, 255, 0])).toBe(true);
  },
  T,
);
