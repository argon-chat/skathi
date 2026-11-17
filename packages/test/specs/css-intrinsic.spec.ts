import { afterAll, describe, expect, setDefaultTimeout, test } from "bun:test";
import { type Bounds, launch } from "../src/index";

setDefaultTimeout(30_000);

const app = await launch({
  app: "packages/test/fixtures/css-intrinsic/src/main.ts",
  window: [720, 560],
  label: "css-intrinsic",
});
afterAll(() => app.close());

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(30);
    v = await read();
  }
  return v;
}

const box = async (id: string): Promise<Bounds> => {
  const b = (await app.query(`#${id}`)).nodes[0]?.bounds;
  if (!b) throw new Error(`#${id} has no box`);
  return b;
};
const size = async (id: string) => {
  const b = await until(
    () => box(id),
    (b) => b.w > 0 && b.h > 0,
  );
  return [Math.round(b.w), Math.round(b.h)];
};
const computed = (id: string, property: string) =>
  app.eval(
    `getComputedStyle(document.getElementById(${JSON.stringify(id)})).getPropertyValue(${JSON.stringify(property)})`,
  ) as Promise<string>;
const height = async (id: string) => Number.parseFloat(await computed(id, "height"));

describe("interpolate-size and calc-size()", () => {
  test("`height: auto` transitions from and back to 0 under allow-keywords", async () => {
    expect(await computed("panel", "interpolate-size")).toBe("allow-keywords");
    expect(await height("panel")).toBe(0);
    const sample = async (open: boolean, end: number) => {
      await app.eval(`document.getElementById("panel").classList.toggle("open", ${open}); 0`);
      const seen: number[] = [];
      await until(
        async () => {
          const h = await height("panel");
          seen.push(h);
          return h;
        },
        (h) => h === end,
        5_000,
      );
      return seen;
    };
    const opening = await sample(true, 100);
    expect(opening.at(-1)).toBe(100);
    expect(opening.some((h) => h > 5 && h < 95)).toBe(true);
    const closing = await sample(false, 0);
    expect(closing.at(-1)).toBe(0);
    expect(closing.some((h) => h > 5 && h < 95)).toBe(true);
  });

  test("a calc-size() box is its basis size, then the calculation", async () => {
    expect(await size("fit")).toEqual([80, 10]);
    expect((await size("half"))[1]).toBe(50);
    expect(await size("basis")).toEqual([60, 10]);
  });

  test("logical sizes map to width and height", async () => {
    expect(await size("logical")).toEqual([50, 30]);
  });
});

describe("@function", () => {
  test("results size boxes, with defaults, conditionals and var() arguments", async () => {
    expect(await size("fn-size")).toEqual([40, 8]);
    expect(await size("fn-var")).toEqual([19, 10]);
  });

  test("a result can be a colour", async () => {
    const [r, g, b] = (await computed("fn-color", "color")).match(/\d+(\.\d+)?/g)!.map(Number);
    expect(Math.abs(r! - 128)).toBeLessThanOrEqual(1);
    expect(Math.abs(g! - 128)).toBeLessThanOrEqual(1);
    expect(b).toBe(255);
  });

  test("@container in the body asks the calling element's container", async () => {
    expect(await size("fn-cq")).toEqual([70, 10]);
  });

  test("a function that calls itself makes the declaration invalid", async () => {
    expect(await size("fn-cycle")).toEqual([220, 10]);
  });
});

describe("dynamic-range-limit", () => {
  test("computes, inherits and mixes", async () => {
    expect(await computed("drl-default", "dynamic-range-limit")).toBe("no-limit");
    expect(await computed("drl-parent", "dynamic-range-limit")).toBe("constrained");
    expect(await computed("drl-child", "dynamic-range-limit")).toBe("constrained");
    expect(await computed("drl-mix", "dynamic-range-limit")).toBe(
      "dynamic-range-limit-mix(standard 40%, no-limit 60%)",
    );
  });
});
