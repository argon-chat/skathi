import { afterAll, expect, test } from "bun:test";
import { type Bounds, launch, ON_SCREEN, readPng, SCREENSHOTS } from "../src/index";

// The card is centred with half a pixel left over, so its subtree sits at a fractional origin.
// Laid out inline and laid out alone (a retained-render cache boundary) it must snap the same way.
const W = 1280;
const app = await launch({
  app: "packages/test/fixtures/text-jitter/src/main.ts",
  window: [W, 800],
  label: "text-jitter",
});
afterAll(() => app.close());

const T = 60_000;
const SAMPLES = 10;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function box(selector: string): Promise<Bounds> {
  const b = (await app.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
}

async function pixels(regions: Bounds[]): Promise<string> {
  const shot = readPng(await app.screenshot(undefined, ON_SCREEN));
  const k = shot.width / W;
  const out: number[] = [];
  for (const b of regions)
    for (let y = Math.floor(b.y * k); y < Math.ceil((b.y + b.h) * k); y++)
      for (let x = Math.floor(b.x * k); x < Math.ceil((b.x + b.w) * k); x++) out.push(...shot.at(x, y));
  return Bun.hash(new Uint8Array(out)).toString(16);
}

/** Distinct images of the static header and messages, before and after each change. */
async function sample(change: () => Promise<void>): Promise<string[]> {
  const regions = [await box(".channel"), await box(".messages")];
  const seen = new Set([await pixels(regions)]);
  for (let i = 0; i < SAMPLES; i++) {
    await change();
    seen.add(await pixels(regions));
  }
  return [...seen];
}

test.skipIf(!SCREENSHOTS)(
  "static text keeps its device pixels while a sibling subtree starts and stops changing every frame",
  async () => {
    await sleep(800);
    let on = false;
    const seen = await sample(async () => {
      on = !on;
      await app.eval(`__SK_TEST_JITTER__.${on ? "start" : "stop"}(); 'ok'`);
      await sleep(150);
    });
    await app.eval("__SK_TEST_JITTER__.stop(); 'ok'");
    expect(Number(await app.eval("__SK_TEST_JITTER__.tick()"))).toBeGreaterThan(SAMPLES);
    expect(seen).toHaveLength(1);
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "typing into the composer leaves the messages and avatars where they were",
  async () => {
    await sleep(300);
    const seen = await sample(async () => {
      await app.locator("#ed").type("a");
      await sleep(120);
    });
    expect(seen).toHaveLength(1);
  },
  T,
);
