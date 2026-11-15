// SKPK v1, the writer half. The reader that matters is the host's (crates/sk-payload); it parses the same
// `fixtures/golden.skpk` this file pins, so the two sides cannot drift.

import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  compareBytes,
  crc32,
  HEADER_SIZE,
  KIND,
  NAMES,
  PAGE,
  readHeader,
  readSkpk,
  skewFor,
  TOC_ENTRY_SIZE,
  SkpkError,
  writeSkpk,
} from "../cli/skpk";
import { GOLDEN_META, GOLDEN_SKEW, goldenInputs } from "./fixtures/golden-skpk";

const HERE = dirname(fileURLToPath(import.meta.url));
const GOLDEN = join(HERE, "fixtures", "golden.skpk");

describe("the golden fixture", () => {
  test("the writer still produces golden.skpk byte for byte", () => {
    const bytes = writeSkpk(goldenInputs(), { skew: GOLDEN_SKEW });
    if (process.env.SK_UPDATE_GOLDEN === "1" || !existsSync(GOLDEN)) {
      writeFileSync(GOLDEN, bytes);
    }
    const golden = new Uint8Array(readFileSync(GOLDEN));
    expect(Buffer.from(bytes).equals(Buffer.from(golden))).toBe(true);
  });

  test("and it reads back as what went in", () => {
    const pk = readSkpk(new Uint8Array(readFileSync(GOLDEN)));
    expect(pk.header.format).toBe(1);
    expect(pk.header.skew).toBe(GOLDEN_SKEW);
    expect(pk.header.entryCount).toBe(7);
    expect(pk.header.tocOff).toBe(HEADER_SIZE);
    expect(pk.header.dataOff).toBe(pk.header.namesOff + pk.header.namesLen + GOLDEN_SKEW);
    // Sorted by (kind, name BYTES): "L" (0x4c) < "d" < "h" < "ü" (0xc3 0xbc).
    expect(pk.entries.map((e) => [e.kind, e.name])).toEqual([
      [KIND.META, NAMES.META],
      [KIND.WINDOW, NAMES.WINDOW],
      [KIND.SCRIPT, NAMES.SCRIPT],
      [KIND.ASSET, "assets/Logo.PNG"],
      [KIND.ASSET, "assets/dir/data.bin"],
      [KIND.ASSET, "assets/hello.txt"],
      [KIND.ASSET, "assets/ünïcode.txt"],
    ]);
    const meta = JSON.parse(new TextDecoder().decode(pk.bytes(pk.find(KIND.META, NAMES.META)!)));
    expect(meta).toEqual(GOLDEN_META);
    for (const e of pk.entries) {
      expect((e.start - pk.header.dataOff) % (1 << e.alignLog2)).toBe(0);
      expect(crc32(pk.bytes(e))).toBe(e.crc32);
    }
  });
});

describe("layout", () => {
  const enc = new TextEncoder();
  const small = [
    { name: "assets/b.txt", kind: KIND.ASSET, data: enc.encode("b") },
    { name: "assets/a.txt", kind: KIND.ASSET, data: enc.encode("aaa") },
    { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: enc.encode("1") },
  ];

  test("two builds of one input are byte-identical (no timestamps)", () => {
    expect(Buffer.from(writeSkpk(small)).equals(Buffer.from(writeSkpk([...small].reverse())))).toBe(true);
  });

  test("the skew moves only the data region", () => {
    const a = readSkpk(writeSkpk(small, { skew: 0 }));
    const b = readSkpk(writeSkpk(small, { skew: 777 }));
    expect(b.header.dataOff - a.header.dataOff).toBe(777);
    expect(b.header.blobLen - a.header.blobLen).toBe(777);
    for (let i = 0; i < a.entries.length; i++) {
      expect(Buffer.from(b.bytes(b.entries[i]!)).equals(Buffer.from(a.bytes(a.entries[i]!)))).toBe(true);
      expect(b.entries[i]!.start - a.entries[i]!.start).toBe(777);
    }
  });

  test("skewFor lands data_off on a page in the image", () => {
    const h = readHeader(writeSkpk(small));
    for (const rva of [0x65cacfc, 0x1000, 0x1004, 0x2ffc]) {
      const skew = skewFor(rva, h);
      expect(skew).toBeGreaterThanOrEqual(0);
      expect(skew).toBeLessThan(PAGE);
      expect((rva + h.namesOff + h.namesLen + skew) % PAGE).toBe(0);
    }
  });

  test("BYTECODE is page-aligned inside the data region, and flags say so", () => {
    const pk = readSkpk(writeSkpk([...small, { name: NAMES.BYTECODE, kind: KIND.BYTECODE, data: new Uint8Array(10) }]));
    const bc = pk.find(KIND.BYTECODE, NAMES.BYTECODE)!;
    expect(bc.alignLog2).toBe(12);
    expect((bc.start - pk.header.dataOff) % PAGE).toBe(0);
    expect(pk.header.flags & 1).toBe(1);
  });

  test("duplicates and bad skews are refused", () => {
    expect(() => writeSkpk([small[0]!, small[0]!])).toThrow();
    expect(() => writeSkpk(small, { skew: PAGE })).toThrow();
    expect(() => writeSkpk(small, { skew: -1 })).toThrow();
  });

  test("the reader refuses what the host refuses", () => {
    const good = writeSkpk(small);
    const bad = (mutate: (b: Uint8Array, dv: DataView) => void) => {
      const b = good.slice();
      mutate(b, new DataView(b.buffer));
      return () => readSkpk(b);
    };
    expect(bad((b) => (b[0] = 0x58))).toThrow(SkpkError);
    expect(bad((_, dv) => dv.setUint16(4, 2, true))).toThrow(SkpkError);
    expect(bad((_, dv) => dv.setBigUint64(HEADER_SIZE + 16, 1n << 40n, true))).toThrow(SkpkError);
    expect(bad((_, dv) => dv.setUint32(8, 1000, true))).toThrow(SkpkError);
    expect(() => readSkpk(good.subarray(0, 10))).toThrow(SkpkError);
    // Unsorted TOC: swap the kinds of the first and last entries.
    expect(
      bad((_, dv) => {
        const last = HEADER_SIZE + 2 * TOC_ENTRY_SIZE + 24;
        dv.setUint16(last, 1, true);
      }),
    ).toThrow(SkpkError);
  });

  test("names compare as bytes, the way Rust's str does", () => {
    const e = new TextEncoder();
    expect(compareBytes(e.encode("Z"), e.encode("a"))).toBeLessThan(0);
    expect(compareBytes(e.encode("a"), e.encode("ä"))).toBeLessThan(0);
    expect(compareBytes(e.encode("ab"), e.encode("abc"))).toBeLessThan(0);
  });
});
