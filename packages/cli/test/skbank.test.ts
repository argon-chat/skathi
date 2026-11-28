import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { crc64, isSkBank, readSkBank, writeSkBank } from "../cli/skbank";
import { KIND, PAGE } from "../cli/skpk";
import { goldenInputs } from "./fixtures/golden-skpk";

test("CRC64 matches the ECMA bit recurrence for unaligned inputs and each tail length", () => {
  const data = Uint8Array.from({ length: 777 }, (_, i) => ((i * 31) ^ (i >>> 2)) & 255);
  for (let offset = 0; offset < 8; offset++) {
    let expected = 0n;
    for (let length = 0; length <= data.length - 8; length++) {
      if (length < 32 || length >= data.length - 16) {
        expect(crc64(data.subarray(offset, offset + length))).toBe(expected);
      }
      expected ^= BigInt(data[offset + length]!) << 56n;
      for (let bit = 0; bit < 8; bit++) {
        expected = BigInt.asUintN(64, (expected << 1n) ^ ((expected >> 63n) * 0x42f0e1eba9ea3693n));
      }
    }
  }
});

describe("SkBank payloads", () => {
  test("matches the shared Skathi writer fixture regardless of input order", () => {
    const bytes = readFileSync(resolve(import.meta.dir, "fixtures/golden-skbank.skpk"));
    expect(writeSkBank(goldenInputs().reverse())).toEqual(bytes);
    const pk = readSkBank(bytes);
    expect(pk.entries.length).toBe(goldenInputs().length);
    expect(pk.dataOffset % PAGE).toBe(0);
    for (const input of goldenInputs()) {
      const entry = pk.find(input.kind, input.name)!;
      expect(Buffer.from(pk.bytes(entry))).toEqual(Buffer.from(input.data));
      expect(crc64(input.data)).toBe(entry.crc64);
    }
    expect(isSkBank(bytes)).toBe(true);
    expect(isSkBank(new TextEncoder().encode("SKPK"))).toBe(false);
  });

  test("reads the .NET writer without conflating public, private, case or Unicode keys", () => {
    const bytes = readFileSync(resolve(import.meta.dir, "../../../crates/sk-payload/tests/fixtures/dehive-typed.skpk"));
    const bank = readSkBank(bytes);
    for (const [kind, name, text] of [
      [KIND.SCRIPT, "sk/app.js", "private script"],
      [KIND.ASSET, "sk/app.js", "public asset"],
      [KIND.ASSET, "ui/Logo.png", "upper"],
      [KIND.ASSET, "ui/copy.png", "upper"],
      [KIND.ASSET, "ui/logo.png", "lower"],
      [KIND.ASSET, "ui/café.txt", "SkBank"],
    ] as const) {
      const entry = bank.find(kind, name)!;
      expect(new TextDecoder().decode(bank.bytes(entry))).toBe(text);
      expect(crc64(bank.bytes(entry))).toBe(entry.crc64);
    }
    expect(bank.find(KIND.ASSET, "ui/copy.png")!.start).toBe(bank.find(KIND.ASSET, "ui/Logo.png")!.start);
  });

  test("places mapped bytecode and ICU at their requested addresses with a nonzero resource RVA", () => {
    for (const baseOffset of [0, 4, 932, 0x1234, 0x2000]) {
      const files = [
        { kind: KIND.SCRIPT, name: "sk/app.js", data: Uint8Array.of(31) },
        { kind: KIND.ICU_DATA, name: "sk/icu/icudt.dat", data: Uint8Array.of(42), alignLog2: 15 },
        { kind: KIND.BYTECODE, name: "sk/app.bc", data: Uint8Array.of(53) },
      ];
      const bank = readSkBank(writeSkBank(files, { baseOffset }));
      for (const { kind, name, alignLog2 } of files) {
        const entry = bank.find(kind, name)!;
        expect((baseOffset + entry.start) % 2 ** (alignLog2 ?? 12)).toBe(0);
      }
    }
  });

  test("enforces UTF-8 for text entries and preserves arbitrary asset bytes", () => {
    const invalid = Uint8Array.of(0xc0, 0x80);
    for (const kind of [KIND.META, KIND.WINDOW, KIND.SCRIPT, KIND.NATIVE_MANIFEST]) {
      const file = { kind, name: "sk/entry", data: invalid };
      expect(() => writeSkBank([file])).toThrow("UTF-8");
      const bank = writeSkBank([{ ...file, data: Uint8Array.of(0x61, 0x62) }]);
      bank.set(invalid, readSkBank(bank).find(kind, file.name)!.start);
      expect(() => readSkBank(bank)).toThrow("UTF-8");
    }
    const asset = readSkBank(writeSkBank([{ kind: KIND.ASSET, name: "font.ttf", data: invalid }]));
    expect([...asset.bytes(asset.entries[0]!)]).toEqual([0xc0, 0x80]);
  });

  test("handles an empty bank and rejects invalid keys and malformed wire ranges", () => {
    const empty = writeSkBank([]);
    expect(readSkBank(empty).entries).toEqual([]);
    const world = Buffer.from(empty);
    world.writeBigUInt64LE(8n, 12);
    expect(readSkBank(world).entries).toEqual([]);
    expect(crc64(new TextEncoder().encode("123456789"))).toBe(0x6c40df5f0b497347n);
    const make = (name: string) => ({ kind: KIND.ASSET, name, data: Uint8Array.of(3) });
    expect(() => writeSkBank([make("a"), make("a")])).toThrow("duplicate");
    for (const bad of ["", "/abs", "C:/abs", "d:relative", "../file", "a/./b", "a\\b", "a\0b", "\ud800"]) {
      expect(() => writeSkBank([make(bad)])).toThrow("entry name");
    }
    const bank = Buffer.from(writeSkBank([make("hello")]));
    expect(() => readSkBank(bank.subarray(0, -1))).toThrow();
    const toc = Number(bank.readBigUInt64LE(bank.length - 16));
    bank.writeBigUInt64LE(2n ** 60n, toc + 16 + 4 + bank.readUInt32LE(toc + 16));
    expect(() => readSkBank(bank)).toThrow("data outside bank");
  });
});
