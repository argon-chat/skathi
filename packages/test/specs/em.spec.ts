import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/em/src/main.ts",
  window: [520, 360],
  label: "em",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Box = { x: number; y: number; w: number; h: number; fontSize: string };
const near = (got: number, want: number) => expect(Math.abs(got - want)).toBeLessThanOrEqual(1);
const boxes = async (): Promise<Record<string, Box>> =>
  JSON.parse(await app.eval(`JSON.stringify(__SK_TEST_EM__.boxes())`));

describe("em", () => {
  test(
    "a 9px badge's padding, gap and children are in its own 9px",
    async () => {
      await sleep(300);
      const b = await boxes();
      expect(b.chip!.fontSize).toBe("9px");
      near(b["chip-a"]!.w, 18);
      near(b["chip-a"]!.h, 9);
      near(b["chip-a"]!.x - b.chip!.x, 9);
      near(b["chip-a"]!.y - b.chip!.y, 4.5);
      near(b["chip-b"]!.x - b["chip-a"]!.x, 27);
      near(b.chip!.w, 63);
      near(b.chip!.h, 18);
    },
    T,
  );

  test(
    "a font-size em or percentage is of the parent's, and the leaf inherits the px",
    async () => {
      const b = await boxes();
      expect([b.l1!.fontSize, b.l2!.fontSize, b.l3!.fontSize]).toEqual(["21px", "15.75px", "15.75px"]);
      near(b.l3!.w, 157.5);
      near(b.l3!.h, 15.75);
      near(b.l3!.x - b.l2!.x, 31.5);
    },
    T,
  );

  test(
    "rem stays on the root",
    async () => {
      const b = await boxes();
      expect(b.rem!.fontSize).toBe("30px");
      near(b.rem!.w, 56);
    },
    T,
  );

  test(
    "a number line-height inherits as the number, an em one as px",
    async () => {
      const b = await boxes();
      expect(b["lh-num"]!.fontSize).toBe("20px");
      near(b["lh-num"]!.h, 40);
      near(b["lh-em"]!.h, 20);
    },
    T,
  );

  test(
    "ch is half an em of the element's own size",
    async () => {
      const b = await boxes();
      near(b.ch!.w, 24);
    },
    T,
  );

  test(
    "a hover rule that changes the font size moves the em of the base's padding",
    async () => {
      const hov = app.locator("#hov");
      await hov.unhover();
      const rest = (await boxes()).hov!;
      expect(rest.fontSize).toBe("10px");
      near(rest.w, 21);
      await hov.hover();
      let hovered = rest;
      for (let i = 0; i < 20 && hovered.w < 40; i++) {
        await sleep(100);
        hovered = (await boxes()).hov!;
      }
      near(hovered.w, 41);
      near(hovered.h, 41);
      await hov.unhover();
    },
    T,
  );
});
