import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/hit-target/src/main.ts",
  window: [520, 400],
  label: "hit-target",
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
async function centre(sel: string): Promise<[number, number]> {
  const b = await box(sel);
  return [b.x + b.w / 2, b.y + b.h / 2];
}
async function mouse(x: number, y: number, kind: string, extra: Record<string, unknown> = {}) {
  await app.send("mouse", { x, y, kind, ...extra });
}
async function clickAt(x: number, y: number): Promise<string[]> {
  await app.eval("__VN_TEST_HIT__.log.length = 0");
  await mouse(x, y, "click");
  await sleep(150);
  return JSON.parse(String(await app.eval("JSON.stringify(__VN_TEST_HIT__.log)"))) as string[];
}
const activeId = async () => String(await app.eval("document.activeElement ? document.activeElement.id : ''"));
const scrollTop = async (sel: string) => Number(await app.eval(`document.querySelector('${sel}').scrollTop`));

describe("a mouse event is for the topmost element under the pointer", () => {
  test("a card painted over a listening sibling takes the press; the backdrop beside it still does", async () => {
    const [x, y] = await centre("#card-text");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=card-text"]);
    const b = await box("#backdrop");
    bunExpect(await clickAt(b.x + 6, b.y + 6)).toEqual([
      "mousedown:backdrop t=backdrop",
      "mousedown:document t=backdrop",
      "click:backdrop t=backdrop",
    ]);
  }, T);

  test("the press over the card is named in the dev log, once", async () => {
    const { entries } = await app.logs(0);
    const lines = entries.filter((e) => e.kind === "dev" && e.text.includes("[hit]") && e.text.includes('id="backdrop"'));
    bunExpect(lines.length).toBe(1);
    bunExpect(lines[0]!.text).toContain('id="card-text"');
  }, T);

  test("`click.self` does not fire for a click on the picture inside it", async () => {
    const [x, y] = await centre("#pic");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=pic"]);
    const o = await box("#overlay");
    bunExpect(await clickAt(o.x + 6, o.y + 6)).toEqual(["mousedown:document t=overlay", "click:overlay-self t=overlay"]);
  }, T);

  test("the target is the label inside the button, and the button hears it", async () => {
    const [x, y] = await centre("#btn-label");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=btn-label", "click:btn t=btn-label"]);
  }, T);

  test("pointer-events: none lets the press through, and auto inside it takes it back", async () => {
    const d = await box("#decor");
    bunExpect(await clickAt(d.x + 20, d.y + d.h / 2)).toEqual(["mousedown:document t=under", "click:under t=under"]);
    const [x, y] = await centre("#decor-auto");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=decor-auto"]);
  }, T);

  test("a child laid out outside its parent's box still bubbles to the parent", async () => {
    const [x, y] = await centre("#outside");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=outside", "click:parent t=outside"]);
  }, T);

  test("a text field under a cover neither focuses nor takes the press", async () => {
    bunExpect(await activeId()).not.toBe("field");
    const [x, y] = await centre("#field-cover");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=field-cover"]);
    await sleep(100);
    bunExpect(await activeId()).not.toBe("field");
  }, T);

  test("a wheel over a cover does not scroll the box under it", async () => {
    const c = await box("#list-cover");
    await app.send("wheel", { x: c.x + c.w / 2, y: c.y + c.h / 2, dx: 0, dy: 60 });
    await sleep(250);
    bunExpect(await scrollTop("#list")).toBe(0);
    const l = await box("#list");
    await app.send("wheel", { x: l.x + l.w / 2, y: l.y + l.h - 10, dx: 0, dy: 60 });
    await sleep(250);
    bunExpect(await scrollTop("#list")).toBeGreaterThan(0);
  }, T);

  test("a press on one child and a release on another click their common ancestor", async () => {
    await app.eval("__VN_TEST_HIT__.log.length = 0");
    const [ax, ay] = await centre("#ra");
    const [bx, by] = await centre("#rb");
    await mouse(ax, ay, "down");
    await mouse(bx, by, "move", { held: true });
    await mouse(bx, by, "up");
    await sleep(150);
    const log = JSON.parse(String(await app.eval("JSON.stringify(__VN_TEST_HIT__.log)"))) as string[];
    bunExpect(log).toEqual(["mousedown:document t=ra", "click:row t=row"]);
  }, T);

  test("inside a replayed cache boundary the cover is still on top", async () => {
    await sleep(600);
    const mem = (await app.send("mem", {})) as { data?: Record<string, unknown> };
    const counters = JSON.stringify(mem.data ?? mem);
    bunExpect(/"cacheBoundaryEntities":\s*[1-9]/.test(counters)).toBe(true);
    const [x, y] = await centre("#cc1");
    bunExpect(await clickAt(x, y)).toEqual(["mousedown:document t=cc1", "click:crow1 t=cc1"]);
    bunExpect(await app.eval(`document.elementFromPoint(${x}, ${y}).id`)).toBe("cc1");
  }, T);
});

describe("document.elementFromPoint / elementsFromPoint", () => {
  const at = async (x: number, y: number) => String(await app.eval(`(document.elementFromPoint(${x}, ${y}) || {}).id || ''`));
  const all = async (x: number, y: number) =>
    JSON.parse(String(await app.eval(`JSON.stringify(document.elementsFromPoint(${x}, ${y}).map((e) => e.id || e.tagName))`))) as string[];

  test("answer what a press there is for", async () => {
    const [x, y] = await centre("#card-text");
    bunExpect(await at(x, y)).toBe("card-text");
    const d = await box("#decor");
    bunExpect(await at(d.x + 20, d.y + d.h / 2)).toBe("under");
    const [ax, ay] = await centre("#decor-auto");
    bunExpect(await at(ax, ay)).toBe("decor-auto");
  }, T);

  test("elementsFromPoint is every box there, topmost first, pointer-events: none left out", async () => {
    const [x, y] = await centre("#card-text");
    const list = await all(x, y);
    bunExpect(list.slice(0, 4)).toEqual(["card-text", "card", "backdrop", "page"]);
    const d = await box("#decor");
    const under = await all(d.x + 20, d.y + d.h / 2);
    bunExpect(under[0]).toBe("under");
    bunExpect(under).not.toContain("decor");
  }, T);

  test("a point outside the viewport is null", async () => {
    bunExpect(await app.eval("document.elementFromPoint(-10, 5)")).toBeNull();
    bunExpect(await app.eval("document.elementsFromPoint(5, 100000).length")).toBe(0);
  }, T);
});
