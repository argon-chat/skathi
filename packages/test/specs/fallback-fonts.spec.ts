import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { brotliDecompressSync } from "node:zlib";
import { launch, type SkApp, WINDOWS } from "../src/index";

// Manrope draws the text and has no Private Use Area glyphs; Alcarin Tengwar, next in the list,
// does. The expected heights and sizes come from the two fonts' own metrics.

const WOFF2_TAGS = (
  "cmap head hhea hmtx maxp name OS/2 post cvt  fpgm glyf loca prep CFF  VORG EBDT EBLC gasp hdmx kern LTSH " +
  "PCLT VDMX vhea vmtx BASE GDEF GPOS GSUB EBSC JSTF MATH CBDT CBLC COLR CPAL SVG  sbix acnt avar bdat bloc bsln " +
  "cvar fdsc feat fmtx fvar gvar hsty just lcar mort morx opbd prop trak Zapf Silf Glat Gloc Feat Sill"
)
  .match(/.{4}\s?/g)!
  .map((tag) => tag.slice(0, 4));

/** Per em: OS/2's typographic ascent, descent and line gap (both fonts set USE_TYPO_METRICS) and x-height. */
function woff2Metrics(path: string) {
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
  return {
    ascent: os2.getInt16(68) / em,
    descent: -os2.getInt16(70) / em,
    gap: os2.getInt16(72) / em,
    xHeight: os2.getInt16(86) / em,
  };
}

const FONTS = join(import.meta.dir, "../fixtures/font-subsets/src/fonts");
const MANROPE = woff2Metrics(join(FONTS, "manrope-latin-wght-normal.woff2"));
const TENGWAR = woff2Metrics(join(FONTS, "AlcarinTengwarVF.woff2"));

/** What `line-height: normal` of a font reaches above and below the baseline, as Blink unites fonts. */
function normalBox(m: ReturnType<typeof woff2Metrics>, size: number): [number, number] {
  const half = Math.round(m.gap * size) / 2;
  return [Math.round(m.ascent * size) + half, Math.round(m.descent * size) + half];
}

/** The line of Manrope at `size` with a run of Tengwar at `fallbackSize`. */
function unitedHeight(size: number, fallbackSize: number): number {
  const [a, d] = normalBox(MANROPE, size);
  const [fa, fd] = normalBox(TENGWAR, fallbackSize);
  return Math.max(a, fa) + Math.max(d, fd);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("fallback fonts", () => {
  let app: SkApp;
  type Rect = { w: number; h: number };
  const rect = async (id: string): Promise<Rect> =>
    JSON.parse(
      String(
        await app.eval(
          `JSON.stringify((({ width, height }) => ({ w: width, h: height }))(document.getElementById(${JSON.stringify(id)}).getBoundingClientRect()))`,
        ),
      ),
    ) as Rect;
  const computed = async (id: string) =>
    String(await app.eval(`getComputedStyle(document.getElementById(${JSON.stringify(id)})).fontSizeAdjust`));

  beforeAll(async () => {
    app = await launch({ app: "packages/test/fixtures/fallback-fonts/src/main.ts", scale: 1, label: "fallback-fonts" });
    const plain = normalBox(MANROPE, 16);
    const deadline = Date.now() + 10_000;
    // Both faces have loaded once Manrope's line is its own height and the Tengwar reference has width.
    while (Date.now() < deadline) {
      const [p, ref] = [await rect("plain"), await rect("pua-ref")];
      if (p.h === plain[0] + plain[1] && ref.w > 0 && Math.abs((await rect("pua")).w - ref.w) < 0.5) break;
      await sleep(50);
    }
    await sleep(200);
  }, 60_000);
  afterAll(() => app.close());

  test("a character the first family lacks is drawn by the next family of the list", async () => {
    expect(Math.abs((await rect("pua")).w - (await rect("pua-ref")).w)).toBeLessThan(0.5);
  });

  test("under line-height: normal a fallback font's taller box grows the line, and only then", async () => {
    const heights = {
      plain: (await rect("plain")).h,
      mixed: (await rect("mixed")).h,
      fixed: (await rect("mixed-fixed")).h,
      snapped: (await rect("mixed-snapped")).h,
    };
    const want = unitedHeight(16, 16);
    console.log("heights at 16px", heights, { want, manrope: normalBox(MANROPE, 16), tengwar: normalBox(TENGWAR, 16) });
    expect(heights.plain).toBe(normalBox(MANROPE, 16).reduce((a, b) => a + b));
    expect(want).toBeGreaterThan(heights.plain);
    expect(heights.mixed).toBe(want);
    expect(heights.fixed).toBe(30);
    // Snapped, Tengwar's whole-pixel box (12 + 5) fits in Manrope's (17 + 5).
    expect(heights.snapped).toBe(heights.plain);
  });

  test("font-size-adjust: from-font sizes the fallback font to the first font's x-height", async () => {
    const adjusted = await rect("adjusted");
    const reference = await rect("adjusted-ref");
    const plain = await rect("pua");
    const size = (16 * MANROPE.xHeight) / TENGWAR.xHeight;
    console.log("from-font", { adjusted, reference, plain, size });
    expect(Math.abs(adjusted.w - reference.w)).toBeLessThan(0.5);
    expect(adjusted.w).toBeGreaterThan(plain.w + 2);
    // The adjusted run's own box grows the line from there.
    expect(adjusted.h).toBe(unitedHeight(16, size));
    expect((await rect("adjusted-mixed")).h).toBe(adjusted.h);
    // `from-font` computes to the first font's x-height per em.
    expect(await computed("adjusted")).toBe(String(Math.round(MANROPE.xHeight * 1e4) / 1e4));
  });

  test("a number sizes the first font too, so that its x-height is that fraction of the font size", async () => {
    const ratio = await rect("ratio");
    const reference = await rect("ratio-ref");
    console.log("0.5", { ratio, reference });
    expect(Math.abs(ratio.w - reference.w)).toBeLessThan(0.5);
    expect(ratio.h).toBe(reference.h);
    expect(await computed("ratio")).toBe("0.5");
    expect(await computed("plain")).toBe("none");
  });

  test.skipIf(!WINDOWS)("a web family after an installed one draws what the installed one lacks", async () => {
    expect(Math.abs((await rect("installed-first")).w - (await rect("pua-ref")).w)).toBeLessThan(0.5);
  });

  test.skipIf(!WINDOWS)("a character no font has is the first font's missing glyph", async () => {
    const [uncovered, reference] = [await rect("uncovered"), await rect("uncovered-ref")];
    expect(uncovered.w).toBeGreaterThan(0);
    expect(uncovered.w).toBe(reference.w);
  });

  test("the app logged nothing broken", async () => {
    await app.expectNoErrors();
  });
});
