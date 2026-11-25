// Rules on different interaction states meet in the cascade: `#l input:focus { outline: none }`
// outweighs `:focus-visible { outline: 2px solid }` on a focused field, as in a browser.

import { afterAll, expect, test } from "bun:test";
import { type Bounds, launch, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/state-specificity/src/main.ts",
  window: [360, 200],
  label: "state-specificity",
});
afterAll(() => app.close());

const T = 30_000;

const box = async (id: string): Promise<Bounds> => {
  const b = (await app.query(`#${id}`)).nodes[0]?.bounds;
  if (!b) throw new Error(`#${id} has no box`);
  return b;
};

const focusOn = async (id: string) => {
  await app.eval(`(document.getElementById(${JSON.stringify(id)}).focus(), 0)`);
  await Bun.sleep(300);
};

const outlineStyle = (id: string): Promise<string> =>
  app.eval(`getComputedStyle(document.getElementById(${JSON.stringify(id)})).outlineStyle`);

/** Whether the ring colour (#818cf8) is painted in a band around the field's edges. */
const ringAround = async (id: string): Promise<boolean> => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 360;
  const b = await box(id);
  const ring = ([r, g, bl]: [number, number, number]) =>
    Math.abs(r - 0x81) < 30 && Math.abs(g - 0x8c) < 30 && Math.abs(bl - 0xf8) < 30;
  const points: [number, number][] = [];
  for (let d = -4; d <= 4; d++) {
    points.push([b.x + d, b.y + b.h / 2], [b.x + b.w + d, b.y + b.h / 2]);
    points.push([b.x + b.w / 2, b.y + d], [b.x + b.w / 2, b.y + b.h + d]);
  }
  return points.some(([x, y]) => ring(shot.at(Math.floor(x * k), Math.floor(y * k))));
};

test.skipIf(!SCREENSHOTS)(
  "a more specific :focus rule removes the :focus-visible outline, and getComputedStyle agrees",
  async () => {
    await focusOn("j");
    expect(await outlineStyle("j")).toBe("solid");
    expect(await ringAround("j")).toBe(true);

    await focusOn("i");
    expect(await outlineStyle("i")).toBe("none");
    expect(await ringAround("i")).toBe(false);
  },
  T,
);
