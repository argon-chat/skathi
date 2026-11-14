import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, parseHsla, styleValue } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/hover-target/src/main.ts",
  window: [520, 440],
  label: "hover-target",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Box = { x: number; y: number; w: number; h: number };
async function box(sel: string): Promise<Box> {
  return (await app.query(sel)).nodes[0]!.bounds!;
}
/** A real move to a window point, a few frames to resolve it in, and the log since. */
async function moveTo(x: number, y: number): Promise<string[]> {
  await app.eval("__SK_TEST_HOVER__.log.length = 0");
  app.notePointer({ kind: "point", x, y });
  await app.send("mouse", { x, y, kind: "move" });
  await sleep(150);
  return JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_HOVER__.log)"))) as string[];
}
async function hovered(sel: string): Promise<boolean> {
  return !!(await app.locator(sel).styles()).hovered;
}
async function tinted(sel: string): Promise<boolean> {
  const { computed } = await app.locator(sel).styles();
  const bg = parseHsla(styleValue(computed.live ?? [], "background") ?? "");
  return (bg?.s ?? 0) > 0.5;
}
const away = () => moveTo(500, 430);

describe("hover follows the topmost element under the pointer", () => {
  test(
    "a cover that listens for nothing takes the hover from the box under it",
    async () => {
      await away();
      const u = await box("#under");
      bunExpect(await moveTo(u.x + u.w / 2, u.y + u.h * 0.75)).toEqual(["mouseenter:under"]);
      bunExpect(await hovered("#under")).toBe(true);
      bunExpect(await tinted("#under")).toBe(true);
      bunExpect(await moveTo(u.x + u.w / 2, u.y + u.h * 0.25)).toEqual(["mouseleave:under"]);
      bunExpect(await hovered("#under")).toBe(false);
      bunExpect(await tinted("#under")).toBe(false);
      bunExpect(await hovered("#cover")).toBe(true);
    },
    T,
  );

  test(
    "from one nested pair to another: leaves innermost first, then enters outermost first",
    async () => {
      const a = await box("#a");
      const b = await box("#b");
      await moveTo(a.x + a.w / 2, a.y + a.h / 2);
      bunExpect(await moveTo(b.x + b.w / 2, b.y + b.h / 2)).toEqual([
        "mouseleave:a",
        "mouseleave:a-wrap",
        "mouseenter:b-wrap",
        "mouseenter:b",
      ]);
    },
    T,
  );

  test(
    "a dropdown's panel, laid out outside its trigger, keeps the trigger hovered",
    async () => {
      await away();
      const m = await box("#menu");
      bunExpect(await moveTo(m.x + m.w / 2, m.y + m.h / 2)).toEqual(["mouseenter:menu"]);
      bunExpect(await tinted("#menu")).toBe(true);
      const s = await box("#sub");
      bunExpect(s.y).toBeGreaterThanOrEqual(m.y + m.h);
      bunExpect(await moveTo(s.x + s.w / 2, s.y + s.h / 2)).toEqual(["mouseenter:sub"]);
      bunExpect(await hovered("#menu")).toBe(true);
      bunExpect(await tinted("#menu")).toBe(true);
      bunExpect(await away()).toEqual(["mouseleave:sub", "mouseleave:menu"]);
      bunExpect(await tinted("#menu")).toBe(false);
    },
    T,
  );

  test(
    "group-hover follows the hover path, so not through a cover",
    async () => {
      const g = await box("#grp");
      await moveTo(g.x + 30, g.y + g.h / 2);
      bunExpect(await tinted("#dot")).toBe(true);
      await moveTo(g.x + g.w - 30, g.y + g.h / 2);
      bunExpect(await tinted("#dot")).toBe(false);
      bunExpect(await hovered("#grp")).toBe(false);
    },
    T,
  );

  test(
    "pointer-events: none keeps an element from being hit, not from being hovered",
    async () => {
      await away();
      const l = await box("#live");
      bunExpect(await moveTo(l.x + l.w / 2, l.y + l.h / 2)).toEqual(["mouseenter:inert"]);
      bunExpect(await hovered("#inert")).toBe(true);
      bunExpect(await tinted("#inert")).toBe(true);
      const i = await box("#inert");
      bunExpect(await moveTo(i.x + i.w - 20, i.y + i.h / 2)).toEqual(["mouseleave:inert"]);
      bunExpect(await tinted("#inert")).toBe(false);
    },
    T,
  );
});
