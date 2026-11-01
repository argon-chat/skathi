// Text's min-content width in a flex row (crates/gpui, `min_content_size` in
// elements/text.rs).
//
// A flex item may not shrink below its automatic minimum size, which is its
// min-content size (CSS Flexbox 1 §4.5). Text's min-content width is its
// widest unbreakable stretch (CSS Sizing 3 §5.1). Before this, gpui answered
// the min-content probe with the one-line width, so a label could not wrap to
// let a row narrower than its one line fit: Argon's dashboard stat rows ran
// their value out of the card instead.
//
// The expectations are Chrome's, measured on the same DOM before browsers were
// off-limits for agents (row widths 100 / 120 / 140 → left group 65 / 74 / 89,
// label wrapped; 180 / 240 → 108, one line). Fonts differ slightly, so the
// spec checks the shape, not the pixels.
//
//   bun test packages/vn-test/specs/min-content.spec.ts

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/min-content/src/main.ts",
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
      // Icon (28) + gap (8) + the label's one line.
      expect(wide.left.width).toBeGreaterThan(90);
      // The value sits at the row's content edge (border 1 + padding 8).
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
        // The left group shrank below its one-line width…
        expect(b.left.width).toBeLessThan(full - 10);
        // …because the label wrapped…
        expect(b.label.height).toBeGreaterThan(oneLine * 1.5);
        // …and the value stays inside the row's content box.
        expect(b.value.right).toBeLessThanOrEqual(b.row.right - 9 + 1);
      }
      // At 100 the row is narrower than both min-contents together, as in
      // Chrome: the left group is at its min-content, the icon plus "Voice".
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
