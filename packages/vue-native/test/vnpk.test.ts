// VNPK v1, the writer half — research/reports/m25/packaging-design.md §4.2.
//
//   bun test packages/vue-native/test/vnpk.test.ts
//
// The reader that matters is the host's (crates/vn-payload); it parses the
// same `fixtures/golden.vnpk` this file pins, so the two sides cannot drift.

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
  readVnpk,
  skewFor,
  TOC_ENTRY_SIZE,
  VnpkError,
  writeVnpk,
} from "../cli/vnpk";
import { GOLDEN_META, GOLDEN_SKEW, goldenInputs } from "./fixtures/golden-vnpk";

const HERE = dirname(fileURLToPath(import.meta.url));
const GOLDEN = join(HERE, "fixtures", "golden.vnpk");

describe("the golden fixture", () => {
  test("the writer still produces golden.vnpk byte for byte", () => {
    const bytes = writeVnpk(goldenInputs(), { skew: GOLDEN_SKEW });
    if (process.env.VN_UPDATE_GOLDEN === "1" || !existsSync(GOLDEN)) {
      writeFileSync(GOLDEN, bytes);
    }
    const golden = new Uint8Array(readFileSync(GOLDEN));
    expect(Buffer.from(bytes).equals(Buffer.from(golden))).toBe(true);
  });

  test("and it reads back as what went in", () => {
    const pk = readVnpk(new Uint8Array(readFileSync(GOLDEN)));
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
    expect(Buffer.from(writeVnpk(small)).equals(Buffer.from(writeVnpk([...small].reverse())))).toBe(true);
  });

  test("the skew moves only the data region", () => {
    const a = readVnpk(writeVnpk(small, { skew: 0 }));
    const b = readVnpk(writeVnpk(small, { skew: 777 }));
    expect(b.header.dataOff - a.header.dataOff).toBe(777);
    expect(b.header.blobLen - a.header.blobLen).toBe(777);
    for (let i = 0; i < a.entries.length; i++) {
      expect(Buffer.from(b.bytes(b.entries[i]!)).equals(Buffer.from(a.bytes(a.entries[i]!)))).toBe(true);
      expect(b.entries[i]!.start - a.entries[i]!.start).toBe(777);
    }
  });

  test("skewFor lands data_off on a page in the image", () => {
    const h = readHeader(writeVnpk(small));
    for (const rva of [0x65cacfc, 0x1000, 0x1004, 0x2ffc]) {
      const skew = skewFor(rva, h);
      expect(skew).toBeGreaterThanOrEqual(0);
      expect(skew).toBeLessThan(PAGE);
      expect((rva + h.namesOff + h.namesLen + skew) % PAGE).toBe(0);
    }
  });

  test("BYTECODE is page-aligned inside the data region, and flags say so", () => {
    const pk = readVnpk(
      writeVnpk([...small, { name: NAMES.BYTECODE, kind: KIND.BYTECODE, data: new Uint8Array(10) }]),
    );
    const bc = pk.find(KIND.BYTECODE, NAMES.BYTECODE)!;
    expect(bc.alignLog2).toBe(12);
    expect((bc.start - pk.header.dataOff) % PAGE).toBe(0);
    expect(pk.header.flags & 1).toBe(1);
  });

  test("duplicates and bad skews are refused", () => {
    expect(() => writeVnpk([small[0]!, small[0]!])).toThrow(/duplicate/);
    expect(() => writeVnpk(small, { skew: PAGE })).toThrow(/skew/);
    expect(() => writeVnpk(small, { skew: -1 })).toThrow(/skew/);
  });

  test("the reader refuses what the host refuses", () => {
    const good = writeVnpk(small);
    const bad = (mutate: (b: Uint8Array, dv: DataView) => void) => {
      const b = good.slice();
      mutate(b, new DataView(b.buffer));
      return () => readVnpk(b);
    };
    expect(bad((b) => (b[0] = 0x58))).toThrow(/magic/);
    expect(bad((_, dv) => dv.setUint16(4, 2, true))).toThrow(/newer vn/);
    expect(bad((_, dv) => dv.setBigUint64(HEADER_SIZE + 16, 1n << 40n, true))).toThrow(VnpkError);
    expect(bad((_, dv) => dv.setUint32(8, 1000, true))).toThrow(/outside/);
    expect(() => readVnpk(good.subarray(0, 10))).toThrow(/shorter/);
    // Unsorted TOC: swap the kinds of the first and last entries.
    expect(
      bad((_, dv) => {
        const last = HEADER_SIZE + 2 * TOC_ENTRY_SIZE + 24;
        dv.setUint16(last, 1, true);
      }),
    ).toThrow(/sorted/);
  });

  test("names compare as bytes, the way Rust's str does", () => {
    const e = new TextEncoder();
    expect(compareBytes(e.encode("Z"), e.encode("a"))).toBeLessThan(0);
    expect(compareBytes(e.encode("a"), e.encode("ä"))).toBeLessThan(0);
    expect(compareBytes(e.encode("ab"), e.encode("abc"))).toBeLessThan(0);
  });

  test("crc32 is IEEE", () => {
    expect(crc32(new TextEncoder().encode("123456789"))).toBe(0xcbf43926);
  });
});
