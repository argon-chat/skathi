import { afterAll, describe, expect, setDefaultTimeout, test } from "bun:test";
import { type Bounds, launch } from "../src/index";

setDefaultTimeout(30_000);

const app = await launch({
  app: "packages/test/fixtures/css-scheme-sizing/src/main.ts",
  window: [720, 560],
  label: "css-sizing",
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

const box = async (selector: string): Promise<Bounds> => {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};
const size = async (id: string) => {
  const b = await until(
    () => box(`#${id}`),
    (b) => b.w > 0 && b.h > 0,
  );
  return [Math.round(b.w), Math.round(b.h)];
};
const computed = (id: string, property: string) =>
  app.eval(
    `getComputedStyle(document.getElementById(${JSON.stringify(id)})).getPropertyValue(${JSON.stringify(property)})`,
  );

describe("aspect-ratio", () => {
  test("fills in the auto dimension from the other one", async () => {
    expect(await size("ar-width")).toEqual([100, 50]);
    expect(await size("ar-height")).toEqual([90, 60]);
  });

  test("a max-width carries through the ratio, a min-height wins over it", async () => {
    expect(await size("ar-max")).toEqual([200, 100]);
    expect(await size("ar-min")).toEqual([100, 80]);
  });

  test("taller content grows the box past the ratio", async () => {
    expect(await size("ar-content")).toEqual([100, 80]);
  });

  test("`auto` undoes an earlier ratio, `auto <ratio>` uses the ratio on a plain box", async () => {
    expect(await size("ar-auto")).toEqual([200, 30]);
    expect(await size("ar-auto-ratio")).toEqual([100, 25]);
    expect(await computed("ar-auto", "aspect-ratio")).toBe("auto");
  });

  test("an image takes the author's ratio, keeps its natural one under `auto <ratio>`, and follows a relative width", async () => {
    expect(await size("img-ratio")).toEqual([100, 50]);
    expect(await size("img-auto-ratio")).toEqual([100, 100]);
    expect(await size("img-relative")).toEqual([120, 120]);
  });

  test("a transition passes through the ratios in between", async () => {
    expect(await size("ar-anim")).toEqual([100, 100]);
    await app.eval(`document.getElementById("ar-anim").classList.add("wide"); "ok"`);
    const heights: number[] = [];
    await until(
      async () => {
        const [, h] = await size("ar-anim");
        heights.push(h!);
        return h!;
      },
      (h) => h === 25,
      5_000,
    );
    expect(heights.at(-1)).toBe(25);
    expect(heights.some((h) => h > 26 && h < 99)).toBe(true);
  });
});

describe("flex-flow", () => {
  const at = async (id: string) => {
    const [parent, b] = await Promise.all([box(`#${id.replace(/-[abc]$/, "")}`), box(`#${id}`)]);
    return [Math.round(b.x - parent.x), Math.round(b.y - parent.y)];
  };

  test("`row wrap` wraps the third cell onto a second line", async () => {
    expect(await at("ff-wrap-a")).toEqual([0, 0]);
    expect(await at("ff-wrap-b")).toEqual([40, 0]);
    expect(await at("ff-wrap-c")).toEqual([0, 20]);
    expect(await computed("ff-wrap", "flex-wrap")).toBe("wrap");
  });

  test("`column` stacks the cells", async () => {
    expect(await at("ff-column-b")).toEqual([0, 20]);
    expect(await computed("ff-column", "flex-direction")).toBe("column");
    expect(await computed("ff-column", "flex-wrap")).toBe("nowrap");
  });

  test("`row-reverse wrap-reverse` reverses both axes", async () => {
    expect(await at("ff-reverse-a")).toEqual([60, 20]);
    expect(await at("ff-reverse-b")).toEqual([20, 20]);
    expect(await at("ff-reverse-c")).toEqual([60, 0]);
  });
});

describe("cursor", () => {
  test("a keyword the platform shows computes to itself and inherits", async () => {
    expect(await computed("cursor-wait", "cursor")).toBe("wait");
    expect(await computed("cursor-child", "cursor")).toBe("wait");
    expect(await computed("ld", "cursor")).toBe("auto");
  });
});
