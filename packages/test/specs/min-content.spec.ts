// Text's min-content width in a flex row (crates/gpui, `min_content_size` in elements/text.rs).

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/min-content/src/main.ts",
  window: [560, 420],
  label: "min-content",
});
afterAll(() => app.close());

type Box = { left: number; right: number; width: number; height: number };
const boxes = async (id: string) =>
  (await app.eval(`(() => {
    const r = (sel) => { const b = document.getElementById(sel).getBoundingClientRect(); return { left: b.left, right: b.right, width: b.width, height: b.height }; };
    return { row: r("row-${id}"), left: r("left-${id}"), label: r("label-${id}"), value: r("value-${id}") };
  })()`)) as { row: Box; left: Box; label: Box; value: Box };

const T = 30_000;

describe("text min-content in a flex row", () => {
  test(
    "a wide row keeps the label on one line",
    async () => {
      const wide = await boxes("r240");
      const oneLine = wide.label.height;
      expect(oneLine).toBeGreaterThan(10);
      expect(oneLine).toBeLessThan(24);
      expect(wide.left.width).toBeGreaterThan(90);
      expect(Math.abs(wide.value.right - (wide.row.right - 9))).toBeLessThan(1);
    },
    T,
  );

  test(
    "a narrow row wraps the label and keeps the value inside",
    async () => {
      const oneLine = (await boxes("r240")).label.height;
      const full = (await boxes("r240")).left.width;
      for (const w of [120, 140]) {
        const b = await boxes(`r${w}`);
        expect(b.left.width).toBeLessThan(full - 10);
        // …because the label wrapped…
        expect(b.label.height).toBeGreaterThan(oneLine * 1.5);
        expect(b.value.right).toBeLessThanOrEqual(b.row.right - 9 + 1);
      }
      const narrow = await boxes("r100");
      expect(narrow.left.width).toBeLessThan(80);
      expect(narrow.left.width).toBeGreaterThan(50);
    },
    T,
  );

  test(
    "nowrap text keeps its one line as its min-content",
    async () => {
      const full = (await boxes("n240")).left.width;
      for (const w of [100, 120, 140]) {
        const b = await boxes(`n${w}`);
        expect(Math.abs(b.left.width - full)).toBeLessThan(1);
        expect(b.label.height).toBeLessThan(24);
      }
    },
    T,
  );
});
