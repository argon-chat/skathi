import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/line-height/src/main.ts",
  window: [480, 600],
  label: "line-height",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const rect = async (id: string) =>
  JSON.parse(
    String(
      await app.eval(
        `JSON.stringify((({ width, height }) => ({ width, height }))(document.getElementById(${JSON.stringify(id)}).getBoundingClientRect()))`,
      ),
    ),
  ) as { width: number; height: number };

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 10_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

describe("white space at the edges of a flex item's text", () => {
  test(
    "a button's label is as wide with the framework's line breaks around it as without",
    async () => {
      const plain = (await rect("btn-plain")).width;
      expect((await rect("btn-ws")).width).toBe(plain);
      const icon = (await rect("btn-icon")).width;
      expect(icon).toBeGreaterThan(plain);
      for (const id of ["btn-icon-ws", "btn-icon-end", "btn-icon-start"]) {
        expect({ id, width: (await rect(id)).width }).toEqual({ id, width: icon });
      }
      expect((await rect("item-ws")).width).toBe((await rect("item-plain")).width);
    },
    T,
  );
});

test(
  "white space between two block-level flex boxes takes no line",
  async () => {
    expect((await rect("rows-ws")).height).toBe((await rect("rows-plain")).height);
  },
  T,
);

describe("line-height: normal", () => {
  test(
    "text with no line-height declared is as tall as text declared normal",
    async () => {
      const n13 = (await rect("n13")).height;
      expect((await rect("u13")).height).toBe(n13);
      expect((await rect("u15")).height).toBe((await rect("n15")).height);
      expect((await rect("padded")).height).toBe(n13 + 16);
      expect(n13).toBeLessThan(13 * 1.5);
    },
    T,
  );

  test(
    "normal is the font's ascent, descent and line gap, resolved per element",
    async () => {
      const loaded = await until(
        () => rect("pixel-normal"),
        (r) => r.width === 100,
      );
      expect(loaded).toEqual({ width: 100, height: 20 });
      expect((await rect("pixel-undeclared")).height).toBe(20);
      expect((await rect("pixel-30")).height).toBe(30);
      expect((await rect("serif-inherited")).height).toBe((await rect("serif-declared")).height);
    },
    T,
  );
});
