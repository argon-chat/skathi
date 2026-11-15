import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, type Bounds } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/cssmisc/src/main.ts",
  window: [720, 520],
  label: "cssmisc",
});
afterAll(() => app.close());

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const box = async (selector: string): Promise<Bounds> => {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(30);
    v = await read();
  }
  return v;
}

const xs = async (row: string) => {
  const [a, b, c] = await Promise.all(["a", "b", "c"].map((k) => box(`#${row}-${k}`)));
  return { a: a.x, b: b.x, c: c.x };
};

describe("order", () => {
  for (const row of ["flex", "grid"]) {
    test(`${row} items are placed in order-modified document order, and a changed order reflows`, async () => {
      const left = (await box(`#${row}`)).x;
      bunExpect(await xs(row)).toEqual({ a: left + 80, b: left, c: left + 40 });

      await app.eval(`document.querySelector("#${row}-b").className = "item o-last"`);
      const moved = await until(
        () => xs(row),
        (p) => p.b === left + 80,
      );
      bunExpect(moved).toEqual({ a: left + 40, b: left + 80, c: left });
    });
  }
});
