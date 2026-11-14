import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/stacking/src/main.ts",
  window: [720, 520],
  label: "stacking",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const lime = ([r, g, b]: Rgb) => g > 160 && r < 60 && b < 60;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 80 && g < 100;

async function painted() {
  const deadline = Date.now() + 10_000;
  let at = await shoot();
  while (!lime(at(100, 200)) && Date.now() < deadline) {
    await sleep(100);
    at = await shoot();
  }
  return at;
}

const clicks = async (): Promise<string[]> => {
  const r = (await app.eval("JSON.stringify(globalThis.__SK_TEST_STACKING__.clicks)")) as { value?: unknown } | string;
  return JSON.parse(String(typeof r === "object" && r !== null && "value" in r ? r.value : r));
};

describe("two `fixed z-50` overlays, the earlier with z 0 / 1 / 2 layers", () => {
  test(
    "the menu's items paint over the popover's glass zone",
    async () => {
      const at = await painted();
      bunExpect(lime(at(100, 200))).toBe(true);
      bunExpect(lime(at(280, 300))).toBe(true);
      bunExpect(blue(at(100, 80))).toBe(true);
      for (const y of [176, 208, 240]) {
        bunExpect(red(at(280, y))).toBe(true);
        bunExpect(red(at(380, y))).toBe(true);
      }
    },
    T,
  );

  test(
    "a click on an item reaches the menu, not the glass zone",
    async () => {
      await app.locator("#item-3").click({ real: true });
      await app.locator("#item-1").click({ real: true });
      await app.send("mouse", { x: 100, y: 200, kind: "click" });
      await sleep(300);
      bunExpect(await clicks()).toEqual(["item-3", "item-1", "glass"]);
    },
    T,
  );
});

describe("a negative z-index on an in-flow box", () => {
  test(
    "paints over its context's background and under its in-flow content",
    async () => {
      const at = await painted();
      bunExpect(lime(at(480, 85))).toBe(true);
      bunExpect(red(at(480, 115))).toBe(true);
      bunExpect(blue(at(480, 145))).toBe(true);
    },
    T,
  );
});

describe("`opacity < 1` at z auto", () => {
  test(
    "keeps its z-100 box inside it, under a later z-10 sibling",
    async () => {
      const at = await painted();
      bunExpect(lime(at(450, 230))).toBe(true);
    },
    T,
  );
});
