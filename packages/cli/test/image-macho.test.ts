import { describe, expect, test } from "bun:test";
import { appendMachOImage, readMachOImage } from "../cli/image-macho";

const page = 16384;

function template() {
  const bytes = new Uint8Array(page + 128);
  const header = new DataView(bytes.buffer);
  const write = (at: number, value: number) => header.setUint32(at, value, true);
  write(0, 0xfeedfacf);
  write(4, 0x0100000c);
  write(12, 6);
  write(16, 4);
  write(20, 72 * 2 + 24 * 2);
  const text = 32;
  write(text, 0x19);
  write(text + 4, 72);
  bytes.set(new TextEncoder().encode("__TEXT"), text + 8);
  write(text + 32, page);
  write(text + 48, page);
  const link = text + 72;
  write(link, 0x19);
  write(link + 4, 72);
  bytes.set(new TextEncoder().encode("__LINKEDIT"), link + 8);
  write(link + 24, page);
  write(link + 32, page);
  write(link + 40, page);
  write(link + 48, 128);
  const uuid = link + 72;
  write(uuid, 0x1b);
  write(uuid + 4, 24);
  const symbols = uuid + 24;
  write(symbols, 2);
  write(symbols + 4, 24);
  write(symbols + 8, page);
  write(symbols + 16, page + 64);
  bytes[page] = 44;
  return bytes;
}

function image() {
  const bytes = new Uint8Array(page * 3);
  const header = new DataView(bytes.buffer);
  header.setBigUint64(0, 0x3130544f414e5542n, true);
  header.setUint32(32, bytes.length, true);
  header.setUint32(40, page, true);
  header.setUint32(64, page, true);
  header.setUint32(72, 5, true);
  header.setUint8(93, 1);
  header.setUint32(120, 1024, true);
  header.setUint32(124, 8, true);
  header.setUint32(328, 1, true);
  [0xa9bf7bfd, 0x910003fd, 0x94000000, 0xa8c17bfd, 0xd65f03c0].forEach((word, i) => {
    header.setUint32(page + 4 * i, word, true);
  });
  return bytes;
}

describe("Mach-O native image container", () => {
  test("preserves page-aligned image bytes and emits deterministic static unwind info", () => {
    const source = image();
    const empty = template();
    const dylib = appendMachOImage(empty, source);
    expect(appendMachOImage(empty, source)).toEqual(dylib);
    expect(readMachOImage(empty)).toBeNull();
    expect(readMachOImage(dylib)).toEqual(source);
    expect(dylib.subarray(2 * page, 5 * page)).toEqual(source);
    expect(dylib[5 * page]).toBe(44);
    const fields = new DataView(dylib.buffer, page);
    const entries = fields.getUint32(fields.getUint32(20, true) + 4, true) + 8;
    expect(fields.getUint32(entries, true)).toBe(3 * page);
    expect(fields.getUint32(entries + 4, true)).toBe(0x02000000);
    expect(fields.getUint32(entries + 8, true)).toBe(3 * page + 8);
    expect(fields.getUint32(entries + 12, true)).toBe(0x04000000);
    expect(fields.getUint32(entries + 16, true)).toBe(3 * page + 16);
    expect(fields.getUint32(entries + 20, true)).toBe(0x02000000);
    expect(new DataView(dylib.buffer).getUint32(32 + 152 * 4 + 72 + 24 + 8, true)).toBe(5 * page);
  });

  test("rejects an absolute image, an invalid command, and a truncated payload", () => {
    const input = image();
    input[93] = 0;
    expect(() => appendMachOImage(template(), input)).toThrow("position-independent");
    const empty = template();
    empty[36] = 7;
    expect(() => appendMachOImage(empty, image())).toThrow("invalid Mach-O load command size");
    const dylib = appendMachOImage(template(), image());
    expect(() => readMachOImage(dylib.subarray(0, 5 * page - 1))).toThrow("exceeds the container size");
  });
});
