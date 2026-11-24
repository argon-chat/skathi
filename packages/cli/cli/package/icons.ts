import { readFileSync } from "node:fs";
import { extname } from "node:path";
import { decodePng, encodePng, isPng, parseIco, resizeRgba, type Rgba } from "../icon";

/** The square images of the given `.png` and `.ico` files, decoded; an `.ico`'s BMP images are skipped. */
export function iconSources(paths: readonly string[]): Rgba[] {
  const out: Rgba[] = [];
  for (const path of paths) {
    const bytes = new Uint8Array(readFileSync(path));
    if (extname(path).toLowerCase() === ".ico") {
      for (const image of parseIco(bytes, path)) if (isPng(image.data)) out.push(decodePng(image.data, path));
    } else out.push(decodePng(bytes, path));
  }
  return out.filter((img) => img.width === img.height).sort((a, b) => b.width - a.width);
}

/** Why the file cannot be a package icon, or null. */
export function iconProblem(path: string): string | null {
  let bytes: Uint8Array;
  try {
    bytes = new Uint8Array(readFileSync(path));
  } catch {
    return "does not exist or cannot be read";
  }
  if (extname(path).toLowerCase() === ".ico") {
    try {
      const pngs = parseIco(bytes, path).filter((i) => isPng(i.data));
      return pngs.length ? null : "is an .ico without PNG images; give a square .png of 256px or more instead";
    } catch (e) {
      return (e as Error).message;
    }
  }
  if (!isPng(bytes)) return "is not a PNG; give a square .png (512px or more) or an .ico";
  const width = new DataView(bytes.buffer, bytes.byteOffset).getUint32(16);
  const height = new DataView(bytes.buffer, bytes.byteOffset).getUint32(20);
  if (width !== height) return `is ${width}×${height}; a package icon must be square`;
  return null;
}

/** A `size` px PNG scaled from the smallest source at least that big, else from the largest. */
export function iconAt(sources: readonly Rgba[], size: number): Uint8Array | null {
  const fit = [...sources].reverse().find((s) => s.width >= size) ?? sources[0];
  if (!fit) return null;
  return encodePng(fit.width === size ? fit : resizeRgba(fit, size, size));
}

/** The icon centred on a transparent `width`×`height` canvas, `size` px square. */
export function iconOnCanvas(sources: readonly Rgba[], size: number, width: number, height: number): Uint8Array | null {
  const png = iconAt(sources, size);
  if (!png) return null;
  const icon = decodePng(png);
  const data = new Uint8Array(width * height * 4);
  const left = Math.floor((width - size) / 2);
  const top = Math.floor((height - size) / 2);
  for (let y = 0; y < size; y++) {
    data.set(icon.data.subarray(y * size * 4, (y + 1) * size * 4), ((top + y) * width + left) * 4);
  }
  return encodePng({ width, height, data });
}
