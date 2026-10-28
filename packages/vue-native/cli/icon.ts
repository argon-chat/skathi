// The exe's icon: `RT_ICON` 1..n plus `RT_GROUP_ICON` **1**.
//
// Id 1 is not a convention we picked — gpui's window class already loads
// `LoadImageW(GetModuleHandleW(NULL), MAKEINTRESOURCE(1), IMAGE_ICON, …)`
// (vendor/zed/crates/gpui_windows/src/platform.rs `load_icon`), so stamping a
// group icon under that id is what puts it on the window, the taskbar and
// Alt-Tab, as well as on the file in Explorer, with no host change at all.
//
// Two inputs:
//   * an `.ico` — taken verbatim, every image it carries;
//   * a SQUARE 8-bit `.png` — becomes PNG-compressed icon images at 256 (the
//     file itself when it is already 256²), 48, 32, 24 and 16. Windows has read
//     PNG at every size since Vista. The small sizes are made by a box filter in
//     premultiplied alpha, which is what keeps a soft edge from going dark.
// Anything else is an error that asks for an `.ico`, rather than a guess.

import { readFileSync } from "node:fs";
import { extname } from "node:path";
import { deflateSync, inflateSync } from "node:zlib";
import { crc32 } from "./vnpk";

export interface IconImage {
  width: number;
  height: number;
  bitCount: number;
  planes: number;
  colorCount: number;
  /** The `RT_ICON` payload: a PNG, or a BMP DIB as an `.ico` carries it. */
  data: Uint8Array;
}

/** Sizes made from a PNG, largest first (the order Explorer reads them in). */
export const PNG_ICON_SIZES = [256, 48, 32, 24, 16];

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

const PNG_SIG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

export interface Rgba {
  width: number;
  height: number;
  /** `width * height * 4`, straight (not premultiplied) alpha. */
  data: Uint8Array;
}

export function isPng(bytes: Uint8Array): boolean {
  return PNG_SIG.every((b, i) => bytes[i] === b);
}

const be32 = (b: Uint8Array, o: number) => ((b[o]! << 24) | (b[o + 1]! << 16) | (b[o + 2]! << 8) | b[o + 3]!) >>> 0;

/** The 8-bit, non-interlaced PNGs an icon is made from. Everything else throws. */
export function decodePng(bytes: Uint8Array, label = "png"): Rgba {
  if (!isPng(bytes)) throw new Error(`${label}: not a PNG`);
  let o = 8;
  let width = 0;
  let height = 0;
  let depth = 0;
  let color = 0;
  let interlace = 0;
  let palette: Uint8Array | null = null;
  let trns: Uint8Array | null = null;
  const idat: Uint8Array[] = [];
  while (o + 8 <= bytes.length) {
    const len = be32(bytes, o);
    const type = String.fromCharCode(...bytes.subarray(o + 4, o + 8));
    const body = bytes.subarray(o + 8, o + 8 + len);
    o += 12 + len;
    if (type === "IHDR") {
      width = be32(body, 0);
      height = be32(body, 4);
      depth = body[8]!;
      color = body[9]!;
      interlace = body[12]!;
    } else if (type === "PLTE") palette = body;
    else if (type === "tRNS") trns = body;
    else if (type === "IDAT") idat.push(body);
    else if (type === "IEND") break;
  }
  if (!width || !height) throw new Error(`${label}: no IHDR`);
  if (depth !== 8 || interlace !== 0 || ![0, 2, 3, 4, 6].includes(color)) {
    throw new Error(
      `${label}: only 8-bit, non-interlaced PNGs are converted (this one is ${depth}-bit, colour type ${color}` +
        `${interlace ? ", interlaced" : ""}) — give windows.icon an .ico instead`,
    );
  }
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[color]!;
  const stride = width * channels;
  const total = idat.reduce((n, c) => n + c.length, 0);
  const z = new Uint8Array(total);
  let zo = 0;
  for (const c of idat) {
    z.set(c, zo);
    zo += c.length;
  }
  const raw = new Uint8Array(inflateSync(z));
  if (raw.length < (stride + 1) * height) throw new Error(`${label}: truncated image data`);
  const px = new Uint8Array(stride * height);
  const bpp = channels;
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]!;
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const cur = px.subarray(y * stride, (y + 1) * stride);
    const prev = y ? px.subarray((y - 1) * stride, y * stride) : new Uint8Array(stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? cur[x - bpp]! : 0;
      const b = prev[x]!;
      const c = x >= bpp ? prev[x - bpp]! : 0;
      let v = line[x]!;
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      } else if (filter !== 0) throw new Error(`${label}: bad filter ${filter} on row ${y}`);
      cur[x] = v & 0xff;
    }
  }
  const out = new Uint8Array(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    let r = 0;
    let g = 0;
    let b = 0;
    let a = 255;
    if (color === 0) r = g = b = px[i]!;
    else if (color === 4) {
      r = g = b = px[i * 2]!;
      a = px[i * 2 + 1]!;
    } else if (color === 2) {
      r = px[i * 3]!;
      g = px[i * 3 + 1]!;
      b = px[i * 3 + 2]!;
    } else if (color === 6) {
      r = px[i * 4]!;
      g = px[i * 4 + 1]!;
      b = px[i * 4 + 2]!;
      a = px[i * 4 + 3]!;
    } else {
      const idx = px[i]!;
      if (!palette || idx * 3 + 2 >= palette.length) throw new Error(`${label}: palette index ${idx} out of range`);
      r = palette[idx * 3]!;
      g = palette[idx * 3 + 1]!;
      b = palette[idx * 3 + 2]!;
      a = trns && idx < trns.length ? trns[idx]! : 255;
    }
    out[i * 4] = r;
    out[i * 4 + 1] = g;
    out[i * 4 + 2] = b;
    out[i * 4 + 3] = a;
  }
  return { width, height, data: out };
}

function chunk(type: string, body: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + body.length);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, body.length);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  out.set(body, 8);
  dv.setUint32(8 + body.length, crc32(out.subarray(4, 8 + body.length)));
  return out;
}

/** RGBA → PNG (colour type 6, filter 0, zlib level 9). Deterministic for one input. */
export function encodePng(img: Rgba): Uint8Array {
  const { width, height, data } = img;
  const stride = width * 4;
  const raw = new Uint8Array((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    raw.set(data.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  }
  const ihdr = new Uint8Array(13);
  const dv = new DataView(ihdr.buffer);
  dv.setUint32(0, width);
  dv.setUint32(4, height);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const parts = [
    new Uint8Array(PNG_SIG),
    chunk("IHDR", ihdr),
    chunk("IDAT", new Uint8Array(deflateSync(raw, { level: 9 }))),
    chunk("IEND", new Uint8Array(0)),
  ];
  const n = parts.reduce((a, p) => a + p.length, 0);
  const out = new Uint8Array(n);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

/**
 * Area-averaging ("box") downscale, in premultiplied alpha so a transparent
 * neighbour cannot drag an edge pixel's colour towards black. Fractional
 * source coverage is weighted exactly, so 256 → 48 is not a nearest-neighbour
 * shimmer.
 */
export function resizeRgba(src: Rgba, dw: number, dh: number): Rgba {
  const { width: sw, height: sh, data } = src;
  const out = new Uint8Array(dw * dh * 4);
  const sx = sw / dw;
  const sy = sh / dh;
  for (let y = 0; y < dh; y++) {
    const y0 = y * sy;
    const y1 = y0 + sy;
    for (let x = 0; x < dw; x++) {
      const x0 = x * sx;
      const x1 = x0 + sx;
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      let wsum = 0;
      for (let yy = Math.floor(y0); yy < Math.ceil(y1); yy++) {
        const wy = Math.min(yy + 1, y1) - Math.max(yy, y0);
        if (wy <= 0) continue;
        for (let xx = Math.floor(x0); xx < Math.ceil(x1); xx++) {
          const wx = Math.min(xx + 1, x1) - Math.max(xx, x0);
          if (wx <= 0) continue;
          const w = wx * wy;
          const i = (Math.min(yy, sh - 1) * sw + Math.min(xx, sw - 1)) * 4;
          const pa = data[i + 3]! / 255;
          r += data[i]! * pa * w;
          g += data[i + 1]! * pa * w;
          b += data[i + 2]! * pa * w;
          a += pa * w;
          wsum += w;
        }
      }
      const o = (y * dw + x) * 4;
      const alpha = wsum ? a / wsum : 0;
      if (a > 0) {
        out[o] = Math.round(r / a);
        out[o + 1] = Math.round(g / a);
        out[o + 2] = Math.round(b / a);
      }
      out[o + 3] = Math.round(alpha * 255);
    }
  }
  return { width: dw, height: dh, data: out };
}

// ---------------------------------------------------------------------------
// ICO
// ---------------------------------------------------------------------------

/** Every image in an `.ico` (ICONDIR + ICONDIRENTRYs), bytes verbatim. */
export function parseIco(bytes: Uint8Array, label = "ico"): IconImage[] {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.length < 6 || dv.getUint16(0, true) !== 0 || dv.getUint16(2, true) !== 1) {
    throw new Error(`${label}: not an .ico (ICONDIR type must be 1)`);
  }
  const count = dv.getUint16(4, true);
  if (!count) throw new Error(`${label}: the .ico holds no images`);
  const images: IconImage[] = [];
  for (let i = 0; i < count; i++) {
    const e = 6 + i * 16;
    if (e + 16 > bytes.length) throw new Error(`${label}: truncated ICONDIRENTRY ${i}`);
    const size = dv.getUint32(e + 8, true);
    const off = dv.getUint32(e + 12, true);
    if (off + size > bytes.length) throw new Error(`${label}: image ${i} runs past the end of the file`);
    images.push({
      width: bytes[e]! || 256,
      height: bytes[e + 1]! || 256,
      colorCount: bytes[e + 2]!,
      planes: dv.getUint16(e + 4, true),
      bitCount: dv.getUint16(e + 6, true),
      data: bytes.slice(off, off + size),
    });
  }
  return images;
}

/** The icon images for a configured `windows.icon` path. */
export function iconImagesFrom(path: string): IconImage[] {
  const bytes = new Uint8Array(readFileSync(path));
  const ext = extname(path).toLowerCase();
  if (ext === ".ico") return parseIco(bytes, path);
  if (ext !== ".png" && !isPng(bytes)) {
    throw new Error(`${path}: windows.icon must be an .ico or a square .png`);
  }
  const img = decodePng(bytes, path);
  if (img.width !== img.height) {
    throw new Error(`${path}: a PNG icon must be square (this one is ${img.width}×${img.height}) — give an .ico instead`);
  }
  if (img.width < 16) throw new Error(`${path}: ${img.width}px is smaller than the smallest icon size (16)`);
  const sizes = PNG_ICON_SIZES.filter((s) => s <= img.width);
  if (img.width < 256 && !sizes.includes(img.width)) sizes.unshift(img.width);
  return sizes.map((s) => ({
    width: s,
    height: s,
    bitCount: 32,
    planes: 1,
    colorCount: 0,
    // The file itself when it is already the right size: re-encoding would
    // only throw away whatever the author's tool did better than zlib -9.
    data: s === img.width ? bytes : encodePng(resizeRgba(img, s, s)),
  }));
}

/**
 * `RT_ICON` payloads (ids `firstId..`) and the `RT_GROUP_ICON` that names them
 * (GRPICONDIR + 14-byte GRPICONDIRENTRYs — an ICONDIRENTRY whose file offset
 * became a resource id).
 */
export function groupIcon(images: IconImage[], firstId = 1): { icons: { id: number; data: Uint8Array }[]; group: Uint8Array } {
  const group = new Uint8Array(6 + images.length * 14);
  const dv = new DataView(group.buffer);
  dv.setUint16(0, 0, true);
  dv.setUint16(2, 1, true);
  dv.setUint16(4, images.length, true);
  const icons = images.map((img, i) => {
    const o = 6 + i * 14;
    group[o] = img.width >= 256 ? 0 : img.width;
    group[o + 1] = img.height >= 256 ? 0 : img.height;
    group[o + 2] = img.colorCount;
    group[o + 3] = 0;
    dv.setUint16(o + 4, img.planes || 1, true);
    dv.setUint16(o + 6, img.bitCount || 32, true);
    dv.setUint32(o + 8, img.data.length, true);
    dv.setUint16(o + 12, firstId + i, true);
    return { id: firstId + i, data: img.data };
  });
  return { icons, group };
}
