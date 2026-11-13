// The exe icon pipeline: `.ico` verbatim, a square PNG resized in TS.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import { decodePng, encodePng, groupIcon, iconImagesFrom, parseIco, resizeRgba, type Rgba } from "../cli/icon";
import { crc32 } from "../cli/vnpk";

const dir = mkdtempSync(join(tmpdir(), "vn-icon-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

function solid(size: number, rgba: [number, number, number, number]): Rgba {
  const data = new Uint8Array(size * size * 4);
  for (let i = 0; i < size * size; i++) data.set(rgba, i * 4);
  return { width: size, height: size, data };
}

function rawPng(width: number, height: number, color: number, rows: Uint8Array[], plte?: Uint8Array): Uint8Array {
  const chunk = (type: string, body: Uint8Array) => {
    const out = new Uint8Array(12 + body.length);
    const dv = new DataView(out.buffer);
    dv.setUint32(0, body.length);
    for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
    out.set(body, 8);
    dv.setUint32(8 + body.length, crc32(out.subarray(4, 8 + body.length)));
    return out;
  };
  const ihdr = new Uint8Array(13);
  new DataView(ihdr.buffer).setUint32(0, width);
  new DataView(ihdr.buffer).setUint32(4, height);
  ihdr[8] = 8;
  ihdr[9] = color;
  const raw = Buffer.concat(rows.map((r) => Buffer.from(r)));
  const parts = [
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    ...(plte ? [chunk("PLTE", plte)] : []),
    chunk("IDAT", new Uint8Array(deflateSync(raw))),
    chunk("IEND", new Uint8Array(0)),
  ];
  return new Uint8Array(Buffer.concat(parts.map((p) => Buffer.from(p))));
}

describe("PNG", () => {
  test("encode → decode round-trips", () => {
    const img = solid(7, [10, 20, 30, 200]);
    img.data[0] = 255;
    const back = decodePng(encodePng(img));
    expect(back.width).toBe(7);
    expect(Buffer.from(back.data).equals(Buffer.from(img.data))).toBe(true);
  });

  test("every filter type and the RGB / palette colour types decode", () => {
    const rgb = rawPng(2, 2, 2, [
      new Uint8Array([1, 10, 20, 30, 5, 5, 5]), // Sub: pixel 2 = (15,25,35)
      new Uint8Array([4, 1, 1, 1, 0, 0, 0]), // Paeth over row 1
    ]);
    const a = decodePng(rgb);
    expect([...a.data.subarray(0, 8)]).toEqual([10, 20, 30, 255, 15, 25, 35, 255]);
    expect([...a.data.subarray(8, 12)]).toEqual([11, 21, 31, 255]);
    const up = decodePng(rawPng(1, 2, 0, [new Uint8Array([0, 100]), new Uint8Array([2, 5])]));
    expect(up.data[4]).toBe(105);
    const avg = decodePng(rawPng(2, 1, 0, [new Uint8Array([3, 100, 10])]));
    expect([avg.data[0], avg.data[4]]).toEqual([100, 60]);
    const pal = decodePng(rawPng(1, 1, 3, [new Uint8Array([0, 1])], new Uint8Array([0, 0, 0, 9, 8, 7])));
    expect([...pal.data]).toEqual([9, 8, 7, 255]);
  });

  test("16-bit PNGs are refused with a pointer to .ico", () => {
    const png = rawPng(1, 1, 6, [new Uint8Array(9)]);
    png[24] = 16; // bit depth byte inside IHDR
    expect(() => decodePng(png)).toThrow(/\.ico/);
  });
});

describe("resize", () => {
  test("a box filter averages, in premultiplied alpha", () => {
    const img: Rgba = { width: 2, height: 1, data: new Uint8Array([255, 0, 0, 255, 0, 255, 0, 0]) };
    const one = resizeRgba(img, 1, 1);
    expect([...one.data]).toEqual([255, 0, 0, 128]);
    const big = resizeRgba(solid(256, [40, 80, 120, 255]), 48, 48);
    expect(big.width).toBe(48);
    expect([...big.data.subarray(0, 4)]).toEqual([40, 80, 120, 255]);
  });
});

describe("icon images", () => {
  test("a 256² PNG becomes 256/48/32/24/16, the 256 one verbatim", () => {
    const path = join(dir, "a.png");
    const bytes = encodePng(solid(256, [1, 2, 3, 255]));
    writeFileSync(path, bytes);
    const images = iconImagesFrom(path);
    expect(images.map((i) => i.width)).toEqual([256, 48, 32, 24, 16]);
    expect(Buffer.from(images[0]!.data).equals(Buffer.from(bytes))).toBe(true);
    expect(decodePng(images[3]!.data).width).toBe(24);
  });

  test("a 1024² PNG is scaled down, a 64² one keeps its own size", () => {
    const big = join(dir, "big.png");
    writeFileSync(big, encodePng(solid(1024, [9, 9, 9, 255])));
    expect(iconImagesFrom(big).map((i) => i.width)).toEqual([256, 48, 32, 24, 16]);
    const small = join(dir, "small.png");
    writeFileSync(small, encodePng(solid(64, [9, 9, 9, 255])));
    expect(iconImagesFrom(small).map((i) => i.width)).toEqual([64, 48, 32, 24, 16]);
  });

  test("a non-square PNG is refused", () => {
    const path = join(dir, "wide.png");
    writeFileSync(path, encodePng({ width: 32, height: 16, data: new Uint8Array(32 * 16 * 4) }));
    expect(() => iconImagesFrom(path)).toThrow(/square/);
  });

  test(".ico in, group icon out", () => {
    const imgs = [encodePng(solid(16, [1, 1, 1, 255])), encodePng(solid(32, [2, 2, 2, 255]))];
    const head = new Uint8Array(6 + imgs.length * 16);
    const dv = new DataView(head.buffer);
    dv.setUint16(2, 1, true);
    dv.setUint16(4, imgs.length, true);
    let off = head.length;
    imgs.forEach((img, i) => {
      const e = 6 + i * 16;
      head[e] = i ? 32 : 16;
      head[e + 1] = i ? 32 : 16;
      dv.setUint16(e + 4, 1, true);
      dv.setUint16(e + 6, 32, true);
      dv.setUint32(e + 8, img.length, true);
      dv.setUint32(e + 12, off, true);
      off += img.length;
    });
    const ico = new Uint8Array(Buffer.concat([Buffer.from(head), ...imgs.map((i) => Buffer.from(i))]));
    const path = join(dir, "x.ico");
    writeFileSync(path, ico);
    const parsed = iconImagesFrom(path);
    expect(parsed.map((p) => p.width)).toEqual([16, 32]);
    expect(Buffer.from(parsed[1]!.data).equals(Buffer.from(imgs[1]!))).toBe(true);

    const { icons, group } = groupIcon(parsed, 1);
    expect(icons.map((i) => i.id)).toEqual([1, 2]);
    const g = new DataView(group.buffer);
    expect(g.getUint16(4, true)).toBe(2);
    expect(group[6]).toBe(16);
    expect(g.getUint32(6 + 14 + 8, true)).toBe(imgs[1]!.length);
    expect(g.getUint16(6 + 14 + 12, true)).toBe(2);
    expect(() => parseIco(new Uint8Array(6))).toThrow(/ICONDIR/);
  });
});
