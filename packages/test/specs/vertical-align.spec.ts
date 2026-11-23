import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/vertical-align/src/main.ts",
  window: [480, 900],
  label: "vertical-align",
});
afterAll(() => app.close());

const T = 30_000;

const box = (selector: string) => app.locator(selector).boundingBox();

/** The item, the reference word beside it and the line they share. */
async function line(name: string) {
  const [item, ref, row] = await Promise.all([box(`#${name}`), box(`#${name}-ref`), box(`#${name}-line`)]);
  return { item, ref, row };
}

describe("vertical-align in a line of text", () => {
  test(
    "sup and sub move off the baseline and the line grows to hold them",
    async () => {
      const plain = await box("#plain-line");
      const sup = await line("sup");
      const raised = sup.ref.y - sup.item.y;
      expect(raised).toBeGreaterThan(4);
      expect(raised).toBeLessThan(9);
      expect(sup.row.h).toBeGreaterThan(plain.h + 4);
      const sub = await line("sub");
      const lowered = sub.item.y - sub.ref.y;
      expect(lowered).toBeGreaterThan(2.5);
      expect(lowered).toBeLessThan(6.5);
      expect(sub.row.h).toBeGreaterThan(plain.h + 2);
    },
    T,
  );

  test(
    "a length raises text by that much",
    async () => {
      const shift = await line("shift");
      expect(Math.abs(shift.ref.y - shift.item.y - 8)).toBeLessThan(1.5);
    },
    T,
  );

  /**
   * How far below the reference word's top a baseline-aligned item's bottom sits, and how far above
   * that a `middle` item's centre sits. Each is read from snapped edges: the bottom and the ref's top,
   * then the middle item's top and bottom (half each) and its ref's top, every edge up to half a
   * pixel off, so the rise is good to 2px. The lines use 48px text so that is small against it.
   */
  async function middleRise(baseName: string, middleName: string) {
    const base = await line(baseName);
    const middle = await line(middleName);
    const baselineBelowRef = base.item.y + base.item.h - base.ref.y;
    const centreBelowRef = middle.item.y + middle.item.h / 2 - middle.ref.y;
    return { baselineBelowRef, rise: baselineBelowRef - centreBelowRef };
  }

  /** Half the x-height of the lines' font, from a 10ex box: its one snapped edge costs 1/20px. */
  const halfExOf = async () => (await box("#ex-ruler")).h / 20;
  const SNAP = 2 + 1 / 20;

  test(
    "an inline-block: baseline puts its bottom on the text's baseline, middle its centre at half the x-height above",
    async () => {
      const { rise } = await middleRise("base", "middle");
      const halfEx = await halfExOf();
      expect(halfEx).toBeGreaterThan(48 * 0.15);
      expect(Math.abs(rise - halfEx)).toBeLessThanOrEqual(SNAP);
    },
    T,
  );

  test(
    "an image aligns like an inline-block: its bottom on the baseline, or its middle with middle",
    async () => {
      const block = await middleRise("base", "middle");
      const image = await middleRise("img-base", "img");
      expect(Math.abs(image.baselineBelowRef - block.baselineBelowRef)).toBeLessThanOrEqual(2);
      expect(Math.abs(image.rise - (await halfExOf()))).toBeLessThanOrEqual(SNAP);
    },
    T,
  );

  test(
    "top and bottom meet the line's edges; text-top and text-bottom the text's",
    async () => {
      const top = await line("top");
      expect(Math.abs(top.item.y - top.row.y)).toBeLessThan(1);
      const bottom = await line("bottom");
      expect(Math.abs(bottom.item.y + bottom.item.h - (bottom.row.y + bottom.row.h))).toBeLessThan(1);
      const textTop = await line("text-top");
      expect(Math.abs(textTop.item.y - textTop.ref.y)).toBeLessThan(3);
      const textBottom = await line("text-bottom");
      expect(Math.abs(textBottom.item.y + textBottom.item.h - (textBottom.ref.y + textBottom.ref.h))).toBeLessThan(3);
    },
    T,
  );
});
