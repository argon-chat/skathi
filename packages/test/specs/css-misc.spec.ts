import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

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

type Rgb = [number, number, number];
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 60 && g < 60;
const white = ([r, g, b]: Rgb) => r > 235 && g > 235 && b > 235;

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};

/** The columns of row `y` inside `b` that `hit`, as [first, last] offsets from `b.x`, or null. */
const span = (at: (x: number, y: number) => Rgb, b: Bounds, y: number, hit: (c: Rgb) => boolean) => {
  let first = -1;
  let last = -1;
  for (let x = 0; x < b.w; x++)
    if (hit(at(b.x + x, b.y + y))) {
      if (first < 0) first = x;
      last = x;
    }
  return first < 0 ? null : [first, last];
};

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

describe("object-position", () => {
  test("an edge offset places an object-fit: none image from the far edges", async () => {
    const b = await box("#op-none");
    const at = await until(shoot, (at) => red(at(b.x + 100, b.y + 20)));
    bunExpect(span(at, b, 20, red)).toEqual([70, 109]);
    bunExpect(white(at(b.x + 100, b.y + 2))).toBe(true);
    bunExpect(red(at(b.x + 100, b.y + 6))).toBe(true);
    bunExpect(red(at(b.x + 100, b.y + 44))).toBe(true);
    bunExpect(white(at(b.x + 100, b.y + 46))).toBe(true);
  });

  test("a percentage moves a cover image through its overflow", async () => {
    const b = await box("#op-cover");
    const at = await until(shoot, (at) => blue(at(b.x + 60, b.y + 30)));
    for (const y of [2, 30, 57]) bunExpect(blue(at(b.x + 60, b.y + y))).toBe(true);
  });

  test("it transitions, and lands on the new position", async () => {
    const b = await box("#op-anim");
    await until(shoot, (at) => red(at(b.x + 5, b.y + 5)));
    await app.eval('document.querySelector("#op-anim").className = "op op-anim op-end"');
    const lefts: number[] = [];
    const deadline = Date.now() + 3_000;
    while (Date.now() < deadline) {
      const left = span(await shoot(), b, 20, red)?.[0];
      if (left !== undefined) lefts.push(left);
      if (left === 80) break;
    }
    bunExpect(lefts.at(-1)).toBe(80);
    bunExpect(lefts.some((x) => x > 4 && x < 76)).toBe(true);
  });
});

describe("CSS Color 4/5 functions", () => {
  const near = (got: Rgb, want: Rgb) => got.every((v, i) => Math.abs(v - want[i]!) <= 2);
  const sample = async (selector: string, dx: number, dy: number) => {
    const b = await box(selector);
    return (await shoot())(b.x + dx, b.y + dy);
  };

  test("oklch(), lab() and color-mix() paint their sRGB colours", async () => {
    bunExpect(await sample("#c-oklch", 20, 20)).toEqual([255, 0, 0]);
    bunExpect(near(await sample("#c-border", 20, 20), [255, 0, 0])).toBe(true);
    bunExpect(near(await sample("#c-border", 3, 20), [99, 99, 99])).toBe(true);
  });

  test("color-mix() over currentColor uses the element's own colour", async () => {
    bunExpect(near(await sample("#c-mix", 20, 20), [128, 0, 128])).toBe(true);
    await app.eval('document.querySelector("#c-mix").style.color = "#00ff00"');
    const got = await until(
      () => sample("#c-mix", 20, 20),
      (c) => near(c, [128, 128, 0]),
    );
    bunExpect(near(got, [128, 128, 0])).toBe(true);
  });
});
