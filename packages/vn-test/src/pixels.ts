// Reading the PIXELS of a screenshot.
//
// Some defects are only visible in a frame. A glyph run that is painted
// underneath its own element's background is at the right place, in the right
// colour, at the right size and reports the right `getBoundingClientRect` — the
// only thing wrong with it is the order it was drawn in, and nothing in the DOM
// can see that. m19 learned the same lesson about animations ("an animation is
// only proved by frames"); this is that lesson with a decoder attached.
//
// Deliberately tiny: `vn_screenshot` writes 8-bit non-interlaced RGBA, which is
// the one shape below. Anything else throws rather than guessing.

import { readFileSync } from "node:fs";
// PNG's IDAT is zlib-wrapped, not raw deflate — `node:zlib` reads the wrapper,
// `Bun.inflateSync` does not.
import { inflateSync } from "node:zlib";

const SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/** A decoded screenshot: RGB per pixel, plus the helpers assertions need. */
export class Png {
  constructor(
    readonly width: number,
    readonly height: number,
    /** `width * height * 4` bytes, RGBA. */
    readonly data: Uint8Array,
  ) {}

  /** `[r, g, b]` at `(x, y)`; out-of-range reads are black rather than a throw. */
  at(x: number, y: number): [number, number, number] {
    if (x < 0 || y < 0 || x >= this.width || y >= this.height) return [0, 0, 0];
    const i = (y * this.width + x) * 4;
    return [this.data[i], this.data[i + 1], this.data[i + 2]];
  }

  /** The mean of the three channels at `(x, y)` — "how light is this pixel". */
  luma(x: number, y: number): number {
    const [r, g, b] = this.at(x, y);
    return (r + g + b) / 3;
  }

  /** How many pixels in `rect` satisfy `hit`. */
  count(rect: Rect, hit: (luma: number, rgb: [number, number, number]) => boolean): number {
    let n = 0;
    for (let y = Math.max(0, Math.floor(rect.y)); y < Math.min(this.height, rect.y + rect.h); y++) {
      for (let x = Math.max(0, Math.floor(rect.x)); x < Math.min(this.width, rect.x + rect.w); x++) {
        const rgb = this.at(x, y);
        if (hit((rgb[0] + rgb[1] + rgb[2]) / 3, rgb)) n++;
      }
    }
    return n;
  }

  /** The bounding box of every pixel in `rect` that satisfies `hit`, or `null`. */
  bounds(rect: Rect, hit: (luma: number, rgb: [number, number, number]) => boolean): Rect | null {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (let y = Math.max(0, Math.floor(rect.y)); y < Math.min(this.height, rect.y + rect.h); y++) {
      for (let x = Math.max(0, Math.floor(rect.x)); x < Math.min(this.width, rect.x + rect.w); x++) {
        const rgb = this.at(x, y);
        if (!hit((rgb[0] + rgb[1] + rgb[2]) / 3, rgb)) continue;
        if (x < minX) minX = x;
        if (y < minY) minY = y;
        if (x > maxX) maxX = x;
        if (y > maxY) maxY = y;
      }
    }
    if (maxX < minX) return null;
    return { x: minX, y: minY, w: maxX - minX + 1, h: maxY - minY + 1 };
  }
}

/** An axis-aligned region, in the same CSS pixels a `boundingBox()` reports. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Shrinks a rect on every side — the usual way to skip a rounded corner's antialiasing. */
export function inset(rect: Rect, by: number): Rect {
  return { x: rect.x + by, y: rect.y + by, w: rect.w - by * 2, h: rect.h - by * 2 };
}

function diffBounds(a: Png, b: Png, threshold: number, rect: Rect): Rect | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const h = Math.min(a.height, b.height, rect.y + rect.h);
  const w = Math.min(a.width, b.width, rect.x + rect.w);
  for (let y = Math.max(0, Math.floor(rect.y)); y < h; y++) {
    for (let x = Math.max(0, Math.floor(rect.x)); x < w; x++) {
      const p = a.at(x, y);
      const q = b.at(x, y);
      const d = Math.abs(p[0] - q[0]) + Math.abs(p[1] - q[1]) + Math.abs(p[2] - q[2]);
      if (d <= threshold) continue;
      if (x < minX) minX = x;
      if (y < minY) minY = y;
      if (x > maxX) maxX = x;
      if (y > maxY) maxY = y;
    }
  }
  if (maxX < minX) return null;
  return { x: minX, y: minY, w: maxX - minX + 1, h: maxY - minY + 1 };
}

/** The bounding box of the pixels where `a` and `b` differ, or `null` if identical. */
export function changedRegion(a: Png, b: Png, threshold = 12, within?: Rect): Rect | null {
  return diffBounds(a, b, threshold, within ?? { x: 0, y: 0, w: a.width, h: a.height });
}

/** Decodes an 8-bit, non-interlaced RGB/RGBA PNG — what `vn_screenshot` writes. */
export function readPng(path: string): Png {
  const file = readFileSync(path);
  for (let i = 0; i < SIGNATURE.length; i++) {
    if (file[i] !== SIGNATURE[i]) throw new Error(`${path} is not a PNG`);
  }
  let width = 0;
  let height = 0;
  let channels = 4;
  const idat: Uint8Array[] = [];
  let at = 8;
  while (at + 8 <= file.length) {
    const length = file.readUInt32BE(at);
    const type = file.toString("ascii", at + 4, at + 8);
    const body = file.subarray(at + 8, at + 8 + length);
    at += 12 + length; // length + type + data + crc
    if (type === "IHDR") {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const depth = body[8];
      const colorType = body[9];
      const interlace = body[12];
      if (depth !== 8 || interlace !== 0 || (colorType !== 6 && colorType !== 2)) {
        throw new Error(
          `${path}: only 8-bit non-interlaced RGB/RGBA is decoded here ` +
            `(depth ${depth}, colour type ${colorType}, interlace ${interlace})`,
        );
      }
      channels = colorType === 6 ? 4 : 3;
    } else if (type === "IDAT") {
      idat.push(body);
    } else if (type === "IEND") {
      break;
    }
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const out = new Uint8Array(width * height * 4);
  // Un-filter in place, one scanline at a time (PNG §9). `prior` is the
  // reconstructed line above; the first line's is all zero.
  const line = new Uint8Array(stride);
  let prior = new Uint8Array(stride);
  let src = 0;
  for (let y = 0; y < height; y++) {
    const filter = raw[src++];
    for (let i = 0; i < stride; i++) {
      const x = raw[src + i];
      const a = i >= channels ? line[i - channels] : 0;
      const b = prior[i];
      const c = i >= channels ? prior[i - channels] : 0;
      switch (filter) {
        case 0:
          line[i] = x;
          break;
        case 1:
          line[i] = (x + a) & 0xff;
          break;
        case 2:
          line[i] = (x + b) & 0xff;
          break;
        case 3:
          line[i] = (x + ((a + b) >> 1)) & 0xff;
          break;
        case 4: {
          const p = a + b - c;
          const pa = Math.abs(p - a);
          const pb = Math.abs(p - b);
          const pc = Math.abs(p - c);
          const pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
          line[i] = (x + pred) & 0xff;
          break;
        }
        default:
          throw new Error(`${path}: unknown PNG filter ${filter} on row ${y}`);
      }
    }
    src += stride;
    for (let x = 0; x < width; x++) {
      const o = (y * width + x) * 4;
      const i = x * channels;
      out[o] = line[i];
      out[o + 1] = line[i + 1];
      out[o + 2] = line[i + 2];
      out[o + 3] = channels === 4 ? line[i + 3] : 255;
    }
    prior = Uint8Array.prototype.slice.call(line);
  }
  return new Png(width, height, out);
}
