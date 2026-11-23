import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { brotliDecompressSync } from "node:zlib";
import { type Bounds, launch, type Png, readPng, type SkApp } from "../src/index";

// `line-metrics: snapped` and `line-leading: above` against Manrope's own metrics, read from the
// fixture's font file.

const WOFF2_TAGS = (
  "cmap head hhea hmtx maxp name OS/2 post cvt  fpgm glyf loca prep CFF  VORG EBDT EBLC gasp hdmx kern LTSH " +
  "PCLT VDMX vhea vmtx BASE GDEF GPOS GSUB EBSC JSTF MATH CBDT CBLC COLR CPAL SVG  sbix acnt avar bdat bloc bsln " +
  "cvar fdsc feat fmtx fvar gvar hsty just lcar mort morx opbd prop trak Zapf Silf Glat Gloc Feat Sill"
)
  .match(/.{4}\s?/g)!
  .map((tag) => tag.slice(0, 4));

/** Ascent, descent and line gap per em: OS/2's typographic values under USE_TYPO_METRICS, else `hhea`. */
function woff2Metrics(path: string): { ascent: number; descent: number; gap: number } {
  const file = readFileSync(path);
  const view = new DataView(file.buffer, file.byteOffset, file.byteLength);
  let at = 48;
  const base128 = () => {
    let value = 0;
    for (;;) {
      const byte = view.getUint8(at++);
      value = value * 128 + (byte & 0x7f);
      if (!(byte & 0x80)) return value;
    }
  };
  const tables: { tag: string; length: number }[] = [];
  for (let i = view.getUint16(12); i > 0; i--) {
    const flags = view.getUint8(at++);
    let tag = WOFF2_TAGS[flags & 63]!;
    if ((flags & 63) === 63) {
      tag = file.subarray(at, at + 4).toString("latin1");
      at += 4;
    }
    const original = base128();
    const transform = flags >> 6;
    const transformed = tag === "glyf" || tag === "loca" ? transform !== 3 : transform !== 0;
    tables.push({ tag, length: transformed ? base128() : original });
  }
  const data = brotliDecompressSync(file.subarray(at, at + view.getUint32(20)));
  const table: Record<string, DataView> = {};
  let offset = 0;
  for (const { tag, length } of tables) {
    table[tag] = new DataView(data.buffer, data.byteOffset + offset, length);
    offset += length;
  }
  const em = table["head"]!.getUint16(18);
  const os2 = table["OS/2"]!;
  const hhea = table["hhea"]!;
  const [ascent, descent, gap] =
    os2.getUint16(62) & 0x80
      ? [os2.getInt16(68), os2.getInt16(70), os2.getInt16(72)]
      : [hhea.getInt16(4), hhea.getInt16(6), hhea.getInt16(8)];
  return { ascent: ascent / em, descent: -descent / em, gap: gap / em };
}

const MANROPE = woff2Metrics(
  join(import.meta.dir, "../fixtures/font-subsets/src/fonts/manrope-latin-wght-normal.woff2"),
);

/** CSS `line-height: normal`: ascent, descent and line gap, each rounded. */
const normal = (size: number) =>
  Math.round(MANROPE.ascent * size) + Math.round(MANROPE.descent * size) + Math.round(MANROPE.gap * size);

/** The font's box under `line-metrics: snapped`: `[ascent, descent]` in whole pixels. */
function snapped(size: number): [number, number] {
  const descent = MANROPE.descent * size;
  const rounded = Math.round(descent);
  return [Math.round(MANROPE.ascent * size), descent - rounded > 1e-4 ? Math.ceil(descent) : rounded];
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("line-metrics and line-leading", () => {
  let app: SkApp;
  const box = async (id: string): Promise<Bounds> =>
    JSON.parse(
      String(
        await app.eval(
          `JSON.stringify((({ x, y, width, height }) => ({ x, y, w: width, h: height }))(document.getElementById(${JSON.stringify(id)}).getBoundingClientRect()))`,
        ),
      ),
    ) as Bounds;
  const shot = async (): Promise<Png> => {
    const file = join(tmpdir(), `sk-line-box-${process.pid}.png`);
    await app.send("screenshot", { path: file, backend: "render_to_image" });
    return readPng(file);
  };
  /** The baseline under `b`'s top: one row below the lowest row of dark ink, an "H"'s foot. */
  const baseline = (png: Png, b: Bounds, x0 = b.x, x1 = b.x + b.w) => {
    const ink = png.bounds({ x: x0, y: b.y, w: x1 - x0, h: b.h }, (luma) => luma < 128);
    return ink ? ink.y + ink.h - b.y : NaN;
  };

  beforeAll(async () => {
    app = await launch({ app: "packages/test/fixtures/line-box/src/main.ts", scale: 1, label: "line-box" });
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline && (await box("n-auto")).w < 1) await sleep(50);
    // The web font has loaded once a line of it is as tall as its own metrics say.
    while (Date.now() < deadline && (await box("n-auto")).h !== normal(11)) await sleep(50);
  }, 60_000);
  afterAll(() => app.close());

  test("normal is the rounded ascent, descent and gap; snapped drops the gap and keeps the descender", async () => {
    const [a, d] = snapped(11);
    const heights = {
      auto: (await box("n-auto")).h,
      snapped: (await box("n-snapped")).h,
      spanAuto: (await box("s-auto")).h,
      spanSnapped: (await box("s-snapped")).h,
    };
    console.log("11 px", heights, { normal: normal(11), snapped: a + d });
    expect(heights.auto).toBe(normal(11));
    expect(heights.snapped).toBe(a + d);
    expect(heights.spanAuto).toBe(heights.auto);
    expect(heights.spanSnapped).toBe(heights.snapped);
    expect(await app.eval(`getComputedStyle(document.getElementById("n-snapped")).lineMetrics`)).toBe("snapped");
    expect(await app.eval(`getComputedStyle(document.getElementById("s-snapped")).lineMetrics`)).toBe("snapped");
    expect(await app.eval(`getComputedStyle(document.getElementById("n-auto")).lineLeading`)).toBe("split");
    expect(await app.eval(`getComputedStyle(document.getElementById("b-above")).lineLeading`)).toBe("above");
  });

  test("the baseline sits at half the leading below the box, or under all of it", async () => {
    const png = await shot();
    const [a, d] = snapped(20);
    const split = baseline(png, await box("b-split"));
    const above = baseline(png, await box("b-above"));
    const autoAbove = baseline(png, await box("b-auto-above"));
    const want = { split: (60 - a - d) / 2 + a, above: 60 - d, autoAbove: 60 - MANROPE.descent * 20 };
    console.log("20 px in 60 px", { split, above, autoAbove }, want);
    expect(Math.abs(split - want.split)).toBeLessThanOrEqual(0.5);
    expect(above).toBe(want.above);
    expect(Math.abs(autoAbove - want.autoAbove)).toBeLessThanOrEqual(0.5);
  });

  test("runs of two sizes share a baseline; the line takes the most each reaches above and below it", async () => {
    const png = await shot();
    const [a13, d13] = snapped(13);
    const [a26, d26] = snapped(26);
    const mixed = await box("mixed");
    const above = Math.max(26 - d13, 52 - d26);
    const below = Math.max(d13, d26);
    console.log("mixed", mixed.h, baseline(png, mixed), { above, below, a13, a26 });
    expect(mixed.h).toBe(above + below);
    expect(baseline(png, mixed)).toBe(above);
  });

  test("baseline-aligned flex items of two sizes line up on the snapped, above baseline", async () => {
    const png = await shot();
    const [, d13] = snapped(13);
    const [, d26] = snapped(26);
    const small = await box("r13");
    const large = await box("r26");
    const inkSmall = small.y + baseline(png, small);
    const inkLarge = large.y + baseline(png, large);
    console.log("row", { small, large, inkSmall, inkLarge });
    expect(small.y - large.y).toBe(52 - d26 - (26 - d13));
    expect(inkSmall).toBe(inkLarge);
    expect(inkLarge - large.y).toBe(52 - d26);
  });

  test("a field's caret and selection cover its line box; its text sits on the line's baseline", async () => {
    const [, d] = snapped(20);
    const field = await box("field");
    await app.locator("#field").focus();
    await app.eval(`(document.getElementById("field").setSelectionRange(2, 2), 0)`);
    let caret: { y: number; h: number } | null = null;
    let text = NaN;
    for (let i = 0; i < 20 && !caret; i++) {
      await sleep(60);
      const png = await shot();
      // The first "H"'s stem is the text; the caret is a dark column far taller than a capital.
      text = baseline(png, field, field.x, field.x + 8);
      for (let x = Math.floor(field.x); x < field.x + field.w && !caret; x++) {
        const column = { x, y: field.y, w: 1, h: field.h };
        if (png.count(column, (luma) => luma < 128) > 30) caret = png.bounds(column, (luma) => luma < 128);
      }
    }
    await app.eval(`(document.getElementById("field").select(), 0)`);
    await sleep(200);
    const selection = (await shot()).bounds(field, (_, [r, g, b]) => g > 200 && r < 80 && b < 80);
    console.log("field", { field, caret, text, selection });
    expect(text).toBe(40 - d);
    expect(caret && caret.y - field.y).toBe(0);
    expect(caret?.h).toBe(40);
    expect(selection && selection.y - field.y).toBe(0);
    expect(selection?.h).toBe(40);
  });
});
