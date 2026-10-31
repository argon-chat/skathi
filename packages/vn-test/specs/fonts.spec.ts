// Web fonts in a real host: `@font-face` and `FontFace` (m26 client sweep #29,
// crates/vn-tree/src/fonts.rs + packages/vue-native/src/fonts.ts,
// docs/RUNTIME.md §5.15).
//
//   bun test packages/vn-test/specs/fonts.spec.ts
//
// `fixtures/fonts` declares the Argon client's shapes: a TrueType face whose
// file names itself by another family, url()s relative to the sheet, and a
// WOFF2 variable font limited to the Private Use Area that leads a stack. Each
// probe is "WWWWiiii" at 16px; in Press Start 2P (1em a glyph) that is 128px
// wide, and in any fallback it is not. Faces load lazily, when text drawn with
// them asks, so the widths also say when a face arrived.

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/fonts/src/main.ts",
  window: [480, 240],
  label: "fonts",
});
afterAll(() => app.close());

const T = 30_000;
const PIXEL = 128;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Polls `read` until `ok` holds, and returns the last answer either way. */
async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 10_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const width = async (sel: string) => (await app.query(sel)).nodes[0]?.bounds?.w ?? -1;
const result = async (name: string) => JSON.parse(String(await app.eval(`JSON.stringify(__VN_TEST_FONTS__.results[${JSON.stringify(name)}] ?? null)`)));
const run = async (name: string) => {
  await app.eval(`__VN_TEST_FONTS__.run(${JSON.stringify(name)})`);
  return until(() => result(name), (v) => v !== "pending" && v !== null);
};

const status = async (family: string) =>
  String(await app.eval(`[...document.fonts].find((f) => f.family === ${JSON.stringify(family)}).status`));

describe("@font-face in the stylesheet", () => {
  test("the sheet's faces are document.fonts from boot, load when text asks, and ready waits for them", async () => {
    const boot = await result("boot");
    // Lazy, as in a browser: at mount no text has been drawn yet.
    expect(boot.faces).toEqual(["VN Pixel:unloaded", "VN Tengwar:unloaded", "VN Late:unloaded", "VN Unused:unloaded"]);
    expect(boot.size).toBe(4);
    expect(boot.status).toBe("loaded");
    // `ready` waited for the first frame's text to ask for its faces, and
    // for those to load. Latin text never asks for the PUA-only face.
    const ready = await until(() => result("ready"), (v) => v !== null);
    expect(ready).toEqual({
      same: true,
      status: "loaded",
      faces: ["VN Pixel:loaded", "VN Tengwar:unloaded", "VN Late:unloaded", "VN Unused:unloaded"],
    });
    const events = await app.eval("JSON.stringify(__VN_TEST_FONTS__.events)");
    expect(JSON.parse(String(events))).toEqual(["loading:VN Pixel", "loadingdone:VN Pixel"]);
  }, T);

  test("text in the declared family is drawn in the font, under the family it was declared as", async () => {
    // The file calls itself "Press Start 2P"; the rule calls it "VN Pixel".
    expect(await until(() => width("#pixel"), (w) => w === PIXEL)).toBe(PIXEL);
    // The fallback, for scale.
    expect(await width("#plain")).not.toBe(PIXEL);
  }, T);

  test("an inline style that names the family is parsed again once the font is in", async () => {
    expect(await until(() => width("#inline"), (w) => w === PIXEL)).toBe(PIXEL);
  }, T);

  test("a PUA-only family that leads a stack leaves the Latin text to the next family (FONT-2)", async () => {
    expect(await until(() => width("#stack"), (w) => w === PIXEL)).toBe(PIXEL);
    const { entries } = await app.logs(0);
    expect(entries.some((e) => e.kind === "advisory" && e.text.includes("`VN Tengwar` covers only part of Unicode"))).toBe(true);
  }, T);

  test("a face loads when text is first drawn with it, and the text swaps to it", async () => {
    expect(await width("#late")).toBe(-1);
    const r = await run("late");
    // The frame that first drew `#late` drew the fallback and asked for the face.
    expect(r.before).toBe("unloaded");
    expect(r.fallback).toBeGreaterThan(0);
    expect(r.fallback).not.toBe(PIXEL);
    expect(r.status).toBe("loaded");
    // …and once it loaded, the sheet was compiled against it and the text swapped.
    expect(await until(() => width("#late"), (w) => w === PIXEL)).toBe(PIXEL);
    // A family whose rule matches only an empty box never loads.
    expect(r.unused).toBe("unloaded");
  }, T);

  test("check() and load() select by family and text; the WOFF2 face loaded", async () => {
    const r = await run("check");
    expect(r).toEqual({
      pixel: true,
      tengwarPua: true,
      loaded: ["VN Tengwar:loaded:U+E000-E0FF"],
      system: true,
      bad: "SyntaxError",
    });
  }, T);
});

describe("FontFace from script", () => {
  test("an ArrayBuffer face loads at once, joins the set, and an element naming it takes it", async () => {
    expect(await width("#script")).not.toBe(PIXEL);
    const r = await run("script");
    expect(r).toEqual({ first: "loading", status: "loaded", has: true, size: 5, check: true });
    expect(await until(() => width("#script"), (w) => w === PIXEL)).toBe(PIXEL);
  }, T);

  test("a face whose file is not there is an error, and its load() rejects", async () => {
    expect(await run("missing")).toEqual({ name: "NetworkError", status: "error" });
  }, T);

  test("the app logged nothing broken, and no missing-font advisory for a declared family", async () => {
    // Nothing ever drew text with it.
    expect(await status("VN Unused")).toBe("unloaded");
    const { entries } = await app.logs(0);
    expect(entries.filter((e) => e.text.includes("missing-font") || /no font installed for `font-family: VN/.test(e.text))).toEqual([]);
    expect(entries.some((e) => e.kind === "advisory" && e.text.includes("local() is not consulted"))).toBe(true);
    await app.expectNoErrors({ ignore: [/VN Missing/] });
  }, T);
});
