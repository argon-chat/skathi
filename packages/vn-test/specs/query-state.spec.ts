// A script's `matches` / `querySelectorAll` / `closest` filter `:hover`,
// `:active`, `:focus`, `:focus-visible` and `:focus-within` by the state the
// element is in now, as in a browser (m26, docs/RUNTIME.md §5.27,
// crates/vn-tree/src/live_state.rs). They used to match as "would match when
// in that state", the cascade's meaning: every element matched `:focus`, and a
// reka Select's trigger matched `:focus-visible` after a mouse pick, which
// opens reka's keyboard-only tooltips on a click.
//
//   bun test packages/vn-test/specs/query-state.spec.ts
//
// Presses, moves and keys are REAL platform input through gpui (devtools
// `mouse` and `key`), which is what sets the modality `:focus-visible` reads.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/query-state/src/main.ts",
  window: [460, 340],
  label: "query-state",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Rgb = [number, number, number];

async function js<V>(source: string): Promise<V> {
  return JSON.parse(String(await app.eval(`JSON.stringify((() => { ${source} })())`))) as V;
}

/** `el.matches(sel)` for each `[id, selector]`. */
const matches = (pairs: [string, string][]) =>
  js<boolean[]>(`return ${JSON.stringify(pairs)}.map(([id, s]) => document.getElementById(id).matches(s))`);

/** The ids `document.querySelectorAll(sel)` finds, in document order. */
const ids = (sel: string) =>
  js<string[]>(`return Array.from(document.querySelectorAll(${JSON.stringify(sel)}), (e) => e.id || e.tagName)`);

/** Polls `read` until `ok` holds, and returns the last answer either way. */
async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const box = async (selector: string): Promise<Bounds> => {
  const b = (await app.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

/** A screenshot, read at CSS pixel `(x, y)`'s centre whatever the scale. */
const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 460;
  return { at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)) };
};
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 60 && g < 60;

describe("focus in a query", () => {
  test("a click focuses a button without showing it; the focus is the only :focus", async () => {
    await app.locator("#a").click({ real: true });
    await until(() => js<string>("return document.activeElement.id"), (id) => id === "a");
    bunExpect(
      await matches([
        ["a", ":focus"],
        ["a", ":focus-visible"],
        ["row", ":focus-within"],
        ["page", ":focus-within"],
        ["b", ":focus"],
        ["b", ":focus-within"],
      ]),
    ).toEqual([true, false, true, true, false, false]);
    bunExpect(await ids(":focus")).toEqual(["a"]);
    bunExpect(await ids("button:not(:focus)")).toEqual(["b"]);
    bunExpect(await js<string>('return document.getElementById("a").closest(":focus-within:not(:focus)").id')).toBe("row");
  }, T);

  test("Tab shows the focus, in the query and on screen", async () => {
    await app.press("Tab");
    await until(() => js<string>("return document.activeElement.id"), (id) => id === "b");
    bunExpect(await matches([["b", ":focus-visible"], ["a", ":focus"]])).toEqual([true, false]);
    bunExpect(await ids(":focus-visible")).toEqual(["b"]);
    const b = await box("#b");
    const s = await until(shoot, (s) => blue(s.at(b.x - 2, b.y + b.h / 2)));
    bunExpect(blue(s.at(b.x - 2, b.y + b.h / 2))).toBe(true);
  }, T);

  test("a click into a text field shows its focus, as in Chromium", async () => {
    await app.locator("#field").click({ real: true });
    await until(() => js<string>("return document.activeElement.id"), (id) => id === "field");
    bunExpect(await matches([["field", ":focus-visible"], ["b", ":focus-visible"]])).toEqual([true, false]);
    const f = await box("#field");
    const s = await until(shoot, (s) => red(s.at(f.x - 2, f.y + f.h / 2)));
    bunExpect(red(s.at(f.x - 2, f.y + f.h / 2))).toBe(true);
    // The blue ring left with the focus.
    const b = await box("#b");
    bunExpect(blue(s.at(b.x - 2, b.y + b.h / 2))).toBe(false);
  }, T);
});

describe("the pointer in a query", () => {
  test(":hover is the element under the pointer and its ancestors", async () => {
    await app.locator("#pad-inner").hover();
    const hovered = await until(() => ids("#page :hover"), (v) => v.length === 2);
    bunExpect(hovered).toEqual(["pad", "pad-inner"]);
    bunExpect(await matches([["pad-inner", ":hover"], ["a", ":hover"], ["page", ":hover"]])).toEqual([true, false, true]);
    bunExpect(await ids("#row > :not(:hover)")).toEqual(["a", "b"]);
    await app.locator("#pad-inner").unhover();
    bunExpect(await until(() => ids("#page :hover"), (v) => v.length === 0)).toEqual([]);
  }, T);

  test(":active is the pressed element and its ancestors, until the release", async () => {
    await app.send("mouse", { selector: "#pad-inner", index: 0, kind: "down" });
    bunExpect(await matches([["pad-inner", ":active"], ["pad", ":active"], ["a", ":active"]])).toEqual([true, true, false]);
    bunExpect(await ids("#page :active")).toEqual(["pad", "pad-inner"]);
    await app.send("mouse", { selector: "#pad-inner", index: 0, kind: "up" });
    bunExpect(await ids("#page :active")).toEqual([]);
  }, T);
});
