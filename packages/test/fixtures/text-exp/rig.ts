// Text rendering experiment rig (#105): renders the text-exp fixture with each SK_TEXT_EXPERIMENT
// variant at several scales and saves screenshots, word boxes and atlas stats.
//
//   bun fixtures/text-exp/rig.ts capture <out> [variant...]    (from packages/test)
//   bun fixtures/text-exp/rig.ts report <out>
//   bun fixtures/text-exp/rig.ts mica <out>      font-smoothing in a Mica window, alpha included
//   bun fixtures/text-exp/rig.ts post <out>      before/after pairs from a capture and a mica run
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import { launch, readPng, type Bounds } from "../../src/index";

// Each variant: the `SK_TEXT_EXPERIMENT` switches and what the page turns on through CSS.
export const VARIANTS: Record<string, { env: string; css?: string }> = {
  baseline: { env: "" },
  "no-phases": { env: "no-phases" },
  contrast: { env: "", css: "contrast" },
  "no-settle": { env: "no-settle" },
  "will-change": { env: "", css: "layer" },
};
const STATIC = ["baseline", "no-phases", "contrast"];
const MOTION = ["baseline", "no-settle", "will-change"];
const SCALES = (process.env.RIG_SCALES ?? "1,1.25,1.5,2").split(",").map(Number);
const PAGES = ["light", "dark", "grey", "color", "offsets"];
const FRAMES = 16;
const CLEARTYPE = process.env.RIG_MODE === "cleartype";
const SUFFIX = CLEARTYPE ? "-cleartype" : "";
const RAW = `raw${SUFFIX}`;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Shot = { file: string; boxes: { cls: string; b: Bounds }[] };

async function capture(out: string, only: string[]) {
  for (const variant of only.length ? only : Object.keys(VARIANTS)) {
    for (const scale of SCALES) {
      const dir = join(out, RAW, variant, String(scale));
      mkdirSync(dir, { recursive: true });
      const app = await launch({
        app: "packages/test/fixtures/text-exp/src/main.ts",
        window: [900, 640],
        scale,
        label: `text-exp-${variant}`,
        env: {
          SK_TEXT_EXPERIMENT: [VARIANTS[variant]!.env, "log"].filter(Boolean).join(","),
          SK_LOG: "gpui=info,gpui_windows=info",
        },
      });
      try {
        const meta: Record<string, unknown> = { variant, scale };
        const shoot = async (name: string, withBoxes = true): Promise<Shot> => {
          const file = join(dir, `${name}.png`);
          await app.send("screenshot", { path: file, backend: "render_to_image" });
          const boxes = withBoxes
            ? (await app.query(".w")).nodes.flatMap((n) =>
                n.bounds ? [{ cls: (n.classes ?? []).find((c) => c.startsWith("k-")) ?? "", b: n.bounds }] : [],
              )
            : [];
          return { file, boxes };
        };
        // Mica, the default backdrop, keeps text grayscale; an opaque window draws ClearType.
        if (CLEARTYPE) await app.eval(`__SK_WINDOW__.appWindow.backdrop = "none"; "ok"`);
        const css = VARIANTS[variant]!.css;
        if (css) await app.eval(`__SK_TEXT_EXP__.with(${JSON.stringify(css)}); "ok"`);
        await sleep(600);
        if (STATIC.includes(variant)) {
          for (const page of PAGES) {
            await app.eval(`__SK_TEXT_EXP__.show(${JSON.stringify(page)}); 'ok'`);
            await sleep(400);
            meta[page] = await shoot(page);
          }
          meta.mem = await app.send("mem", {});
        }
        if (MOTION.includes(variant)) {
          await app.eval(`__SK_TEXT_EXP__.show("motion"); 'ok'`);
          await sleep(300);
          meta.rest = await shoot("motion-rest");
          for (const kind of ["scale", "translate"]) {
            await app.eval(`__SK_TEXT_EXP__.motion(${JSON.stringify(kind)}); 'ok'`);
            await sleep(300);
            const frames: Shot[] = [];
            for (let i = 0; i < FRAMES; i++) {
              frames.push(await shoot(`motion-${kind}-${String(i).padStart(2, "0")}`, i === 0));
              await sleep(37 + ((i * 53) % 41));
            }
            meta[`motion-${kind}`] = frames;
            await app.eval(`__SK_TEXT_EXP__.motion("none"); 'ok'`);
            await sleep(200);
          }
          await app.eval(`__SK_TEXT_EXP__.pin("scale(1.14)"); 'ok'`);
          await sleep(60);
          meta["pinned-early"] = await shoot("pinned-early");
          await sleep(900);
          meta["pinned-late"] = await shoot("pinned-late");
          await app.eval(`__SK_TEXT_EXP__.pin("translate(3.3px, 1.4px)"); 'ok'`);
          await sleep(900);
          meta["pinned-translate"] = await shoot("pinned-translate");
          await app.eval(`__SK_TEXT_EXP__.pin("none"); 'ok'`);
        }
        meta.log = [
          ...new Set(
            readFileSync(app.hostLog, "utf8")
              .split(/\r?\n/)
              .filter((t) => t.includes("SK_TEXT_EXPERIMENT"))
              .map((t) => t.replace(/^.*SK_TEXT_EXPERIMENT/, "SK_TEXT_EXPERIMENT")),
          ),
        ];
        writeFileSync(join(dir, "meta.json"), JSON.stringify(meta, null, 1));
        console.log(`captured ${variant} @ ${scale}`);
      } finally {
        await app.close();
      }
    }
  }
}

// ---- images ----

class Img {
  data: Uint8Array;
  constructor(
    readonly w: number,
    readonly h: number,
    fill = 0,
  ) {
    this.data = new Uint8Array(w * h * 4).fill(fill);
    for (let i = 3; i < this.data.length; i += 4) this.data[i] = 255;
  }
  set(x: number, y: number, [r, g, b]: number[]) {
    if (x < 0 || y < 0 || x >= this.w || y >= this.h) return;
    const i = (y * this.w + x) * 4;
    this.data[i] = r!;
    this.data[i + 1] = g!;
    this.data[i + 2] = b!;
  }
  blit(src: Img, dx: number, dy: number, zoom = 1) {
    for (let y = 0; y < src.h * zoom; y++)
      for (let x = 0; x < src.w * zoom; x++) {
        const i = (Math.floor(y / zoom) * src.w + Math.floor(x / zoom)) * 4;
        this.set(dx + x, dy + y, [src.data[i]!, src.data[i + 1]!, src.data[i + 2]!]);
      }
  }
}

const CRC = new Int32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});
function crc32(buf: Uint8Array): number {
  let c = -1;
  for (const b of buf) c = CRC[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}
function writeImg(path: string, img: Img) {
  const raw = new Uint8Array((img.w * 4 + 1) * img.h);
  for (let y = 0; y < img.h; y++)
    raw.set(img.data.subarray(y * img.w * 4, (y + 1) * img.w * 4), y * (img.w * 4 + 1) + 1);
  const chunk = (type: string, body: Uint8Array) => {
    const out = new Uint8Array(12 + body.length);
    const view = new DataView(out.buffer);
    view.setUint32(0, body.length);
    out.set(new TextEncoder().encode(type), 4);
    out.set(body, 8);
    view.setUint32(8 + body.length, crc32(out.subarray(4, 8 + body.length)));
    return out;
  };
  const ihdr = new Uint8Array(13);
  const v = new DataView(ihdr.buffer);
  v.setUint32(0, img.w);
  v.setUint32(4, img.h);
  ihdr.set([8, 6, 0, 0, 0], 8);
  const parts = [
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", new Uint8Array(deflateSync(raw))),
    chunk("IEND", new Uint8Array()),
  ];
  writeFileSync(path, Buffer.concat(parts));
}

type Rect = { x: number; y: number; w: number; h: number };
const decoded = new Map<string, ReturnType<typeof readPng>>();
function crop(file: string, r: Rect): Img {
  let png = decoded.get(file);
  if (!png) {
    if (decoded.size > 40) decoded.clear();
    png = readPng(file);
    decoded.set(file, png);
  }
  const img = new Img(r.w, r.h);
  const clamp = (v: number, n: number) => Math.max(0, Math.min(n - 1, v));
  for (let y = 0; y < r.h; y++)
    for (let x = 0; x < r.w; x++) img.set(x, y, png.at(clamp(r.x + x, png.width), clamp(r.y + y, png.height)));
  return img;
}
function lum(img: Img, x: number, y: number) {
  const i = (y * img.w + x) * 4;
  return (img.data[i]! + img.data[i + 1]! + img.data[i + 2]!) / 3;
}
function diffImg(a: Img, b: Img): Img {
  const d = new Img(a.w, a.h, 128);
  for (let y = 0; y < a.h; y++)
    for (let x = 0; x < a.w; x++) {
      const v = Math.max(-127, Math.min(127, (lum(b, x, y) - lum(a, x, y)) * 8));
      d.set(x, y, v > 0 ? [128 + v, 128 - v / 2, 128 - v / 2] : [128 + v / 2, 128 + v / 2, 128 - v]);
    }
  return d;
}
/** Ink in pixel units: how far each pixel is from the background, summed. */
function ink(img: Img, bg: number) {
  let s = 0;
  for (let y = 0; y < img.h; y++) for (let x = 0; x < img.w; x++) s += Math.abs(lum(img, x, y) - bg) / 255;
  return s;
}
/** Neighbouring-pixel contrast summed: how crisp the edges are. */
function edge(img: Img) {
  let s = 0;
  for (let y = 0; y < img.h; y++)
    for (let x = 1; x < img.w; x++) s += Math.abs(lum(img, x, y) - lum(img, x - 1, y)) / 255;
  return s;
}
function deviceRect(b: Bounds, scale: number, pad = 1): Rect {
  const x = Math.round(b.x * scale) - pad;
  const y = Math.round(b.y * scale) - pad;
  return { x, y, w: Math.round(b.w * scale) + 2 * pad, h: Math.round(b.h * scale) + 2 * pad };
}

/** Columns of ink split into blobs; the x centroid of each blob. */
function blobs(img: Img, bg: number): { cx: number[]; cy: number } {
  const col = new Float64Array(img.w);
  let sy = 0;
  let sw = 0;
  for (let x = 0; x < img.w; x++)
    for (let y = 0; y < img.h; y++) {
      const v = Math.abs(lum(img, x, y) - bg) / 255;
      col[x] = col[x]! + v;
      sy += v * y;
      sw += v;
    }
  const cx: number[] = [];
  let x = 0;
  while (x < img.w) {
    if (col[x]! < 0.05) {
      x++;
      continue;
    }
    let s = 0;
    let m = 0;
    while (x < img.w && col[x]! >= 0.05) {
      s += col[x]! * x;
      m += col[x]!;
      x++;
    }
    if (m > 1) cx.push(s / m);
  }
  return { cx, cy: sw ? sy / sw : 0 };
}

/** Per glyph, the midpoint of its outermost 50 % crossings, averaged over the rows of `ys`. */
function edgeCentres(img: Img, bg: number, ys: number[], blobsCx: number[]): number[] {
  return blobsCx.map((c, i) => {
    const lo = Math.floor(i ? (blobsCx[i - 1]! + c) / 2 : 0);
    const hi = Math.ceil(i < blobsCx.length - 1 ? (blobsCx[i + 1]! + c) / 2 : img.w - 1);
    let sum = 0;
    let n = 0;
    for (const y of ys) {
      const v = (x: number) => Math.abs(lum(img, x, y) - bg) / 255;
      let max = 0;
      for (let x = lo; x <= hi; x++) max = Math.max(max, v(x));
      if (max < 0.3) continue;
      const half = max / 2;
      let left = -1;
      for (let x = lo + 1; x <= hi; x++)
        if (v(x - 1) < half && v(x) >= half) {
          left = x - 1 + (half - v(x - 1)) / (v(x) - v(x - 1));
          break;
        }
      let right = -1;
      for (let x = hi - 1; x >= lo; x--)
        if (v(x + 1) < half && v(x) >= half) {
          right = x + (v(x) - half) / (v(x) - v(x + 1));
          break;
        }
      if (left < 0 || right < 0) continue;
      sum += (left + right) / 2;
      n++;
    }
    return n ? sum / n : c;
  });
}

function rowInk(img: Img, bg: number): number[] {
  const out: number[] = [];
  for (let y = 0; y < img.h; y++) {
    let s = 0;
    for (let x = 0; x < img.w; x++) s += Math.abs(lum(img, x, y) - bg) / 255;
    out.push(s / img.w);
  }
  return out;
}

const BG: Record<string, number> = { light: 255, dark: 28, grey: 255, color: 255, offsets: 255, motion: 255 };

function sheet(rows: Img[][], gap = 6, bg = 200): Img {
  const w = Math.max(...rows.map((r) => r.reduce((s, i) => s + i.w + gap, gap)));
  const h = rows.reduce((s, r) => s + Math.max(...r.map((i) => i.h)) + gap, gap);
  const out = new Img(w, h, bg);
  let y = gap;
  for (const r of rows) {
    let x = gap;
    for (const i of r) {
      out.blit(i, x, y);
      x += i.w + gap;
    }
    y += Math.max(...r.map((i) => i.h)) + gap;
  }
  return out;
}
function zoom(img: Img, z: number): Img {
  const out = new Img(img.w * z, img.h * z);
  out.blit(img, 0, 0, z);
  return out;
}

type Meta = { log: string[]; mem?: unknown; [key: string]: unknown };
const metaOf = (out: string, variant: string, scale: number): Meta | null => {
  const p = join(out, RAW, variant, String(scale), "meta.json");
  return existsSync(p) ? (JSON.parse(readFileSync(p, "utf8")) as Meta) : null;
};
const shot = (meta: Meta, key: string) => meta[key] as Shot;
const shots = (meta: Meta, key: string) => meta[key] as Shot[];

function atlas(meta: Meta): { tiles: number; px: number } {
  const a = findKey(meta.mem, "atlas");
  const kinds = ["monochrome", "subpixel"].map((k) => {
    const kind = a && typeof a === "object" ? (a as Record<string, unknown>)[k] : undefined;
    return (kind ?? {}) as { tiles?: number; usedMB?: number };
  });
  return {
    tiles: kinds.reduce((s, k) => s + (k.tiles ?? 0), 0),
    px: kinds.reduce((s, k) => s + (k.usedMB ?? 0), 0),
  };
}
function findKey(o: unknown, key: string): unknown {
  if (!o || typeof o !== "object") return undefined;
  const record = o as Record<string, unknown>;
  if (key in record) return record[key];
  for (const v of Object.values(record)) {
    const f = findKey(v, key);
    if (f !== undefined) return f;
  }
  return undefined;
}

function report(out: string) {
  const lines: string[] = [];
  const cmp = join(out, `compare${SUFFIX}`);
  mkdirSync(cmp, { recursive: true });
  const summary: Record<string, unknown> = {};

  // Static pages: ink per word against the baseline, sheets per page.
  for (const variant of STATIC.filter((v) => v !== "baseline")) {
    lines.push(`\n## ${variant}`);
    for (const scale of SCALES) {
      const base = metaOf(out, "baseline", scale);
      const meta = metaOf(out, variant, scale);
      if (!base || !meta) continue;
      const ab = atlas(base);
      const av = atlas(meta);
      lines.push(
        `scale ${scale}: atlas glyph tiles ${ab.tiles} -> ${av.tiles} (${((av.tiles / ab.tiles - 1) * 100).toFixed(1)} %), used MB ${ab.px.toFixed(3)} -> ${av.px.toFixed(3)}`,
      );
      for (const page of PAGES) {
        const sb: Shot = shot(base, page);
        const sv: Shot = shot(meta, page);
        const rows: Img[][] = [];
        const zrows: Img[][] = [];
        const stats: string[] = [];
        let changed = 0;
        let ratioSum = 0;
        let n = 0;
        let maxRatio = 1;
        let minRatio = 1;
        for (const { cls, b } of sb.boxes) {
          const bv = sv.boxes.find((x) => x.cls === cls)?.b;
          if (!bv) continue;
          const r = deviceRect(b, scale);
          const a = crop(sb.file, r);
          const v = crop(sv.file, deviceRect(bv, scale));
          const ia = ink(a, BG[page]!);
          const iv = ink(v, BG[page]!);
          const d = a.data.some((x, i) => Math.abs(x - v.data[i]!) > 2);
          if (d) changed++;
          if (ia > 0.5) {
            const ratio = iv / ia;
            ratioSum += ratio;
            maxRatio = Math.max(maxRatio, ratio);
            minRatio = Math.min(minRatio, ratio);
            n++;
            stats.push(
              `${cls}\t${ia.toFixed(1)}\t${iv.toFixed(1)}\t${(ratio * 100 - 100).toFixed(2)}%\tedge ${edge(a).toFixed(1)}->${edge(v).toFixed(1)}`,
            );
          }
          if (!cls.endsWith("-p") && (cls.endsWith("-0") || page === "offsets")) {
            rows.push([a, v, diffImg(a, v)]);
            if (/-(12|13|16)-0$/.test(cls) || page === "offsets")
              zrows.push([zoom(a, 4), zoom(v, 4), zoom(diffImg(a, v), 4)]);
          }
        }
        writeImg(join(cmp, `${variant}@${scale}-${page}.png`), sheet(rows));
        if (zrows.length) writeImg(join(cmp, `${variant}@${scale}-${page}-x4.png`), sheet(zrows.slice(0, 8)));
        writeFileSync(
          join(cmp, `${variant}@${scale}-${page}.tsv`),
          "word\tbaseline ink\tvariant ink\tchange\n" + stats.join("\n"),
        );
        lines.push(
          `  ${page}: ${changed}/${sb.boxes.length} words differ; ink mean ${((ratioSum / Math.max(n, 1) - 1) * 100).toFixed(2)} %, range ${((minRatio - 1) * 100).toFixed(2)} .. ${((maxRatio - 1) * 100).toFixed(2)} %`,
        );
      }
    }
  }

  // Motion: glyph spacing per frame and text against the rules.
  lines.push(`\n## motion (H H H …, 24px Segoe UI; spacing in device px)`);
  for (const variant of MOTION) {
    for (const scale of SCALES) {
      const meta = metaOf(out, variant, scale);
      if (!meta) continue;
      for (const kind of ["scale", "translate", "pinned"]) {
        const frames: Shot[] =
          kind === "pinned"
            ? [shot(meta, "rest"), shot(meta, "pinned-early"), shot(meta, "pinned-late")]
            : shots(meta, `motion-${kind}`);
        const b = (kind === "pinned" ? shot(meta, "rest") : frames[0]!).boxes.find(
          (x: { cls: string }) => x.cls === "k-card",
        )!.b;
        // The card scales about its left centre (`transform-origin: left center`) and slides right and down.
        const r = deviceRect({ x: b.x - 4, y: b.y - b.h * 0.1 - 6, w: b.w * 1.2 + 16, h: b.h * 1.2 + 12 }, scale, 0);
        let wobble = 0;
        let resid = 0;
        let relSpread: number[] = [];
        const strip: Img[][] = [];
        for (const f of frames) {
          const img = crop(f.file, r);
          const textBand = rowInk(img, 255);
          const peak = (y0: number, y1: number) => {
            let best = y0;
            for (let y = y0; y < y1; y++) if (textBand[y]! > textBand[best]!) best = y;
            return best;
          };
          const top = peak(0, Math.round((6 + b.h * 0.35) * scale));
          const bottom = peak(Math.round((6 + b.h * 0.85) * scale), img.h);
          const band = new Img(img.w, Math.max(1, bottom - top - 5));
          for (let y = 0; y < band.h; y++)
            for (let x = 0; x < img.w; x++) {
              const i = ((top + 3 + y) * img.w + x) * 4;
              band.set(x, y, [img.data[i]!, img.data[i + 1]!, img.data[i + 2]!]);
            }
          const { cx: inkCx, cy: bandCy } = blobs(band, 255);
          const bandRows = rowInk(band, 255);
          const g0 = bandRows.findIndex((v) => v > 0.01);
          const g1 = bandRows.length - 1 - [...bandRows].reverse().findIndex((v) => v > 0.01);
          const gh = g1 - g0;
          const ys = bandRows
            .map((_, y) => y)
            .filter((y) => {
              const t = (y - g0) / gh;
              return (t > 0.15 && t < 0.35) || (t > 0.65 && t < 0.85);
            });
          // Stem edges move linearly under filtering; an ink centroid also moves with the
          // non-linear contrast and gamma step.
          const cx = process.env.RIG_CENTROID ? inkCx : edgeCentres(band, 255, ys, inkCx);
          if (process.env.RIG_DEBUG)
            console.log(variant, scale, kind, top, bottom, cx.map((x) => x.toFixed(2)).join(" "));
          const cy = bandCy + top + 3;
          const hs = cx.length;
          if (hs < 3) continue;
          const d = cx.slice(1).map((x, i) => x - cx[i]!);
          const mean = d.reduce((s, x) => s + x, 0) / d.length;
          wobble = Math.max(wobble, ...d.map((x) => Math.abs(x - mean)));
          const slope = mean;
          const c0 = cx.reduce((s, x, i) => s + x - slope * i, 0) / hs;
          resid = Math.max(resid, ...cx.map((x, i) => Math.abs(x - (c0 + slope * i))));
          const ruleCentre = (y0: number) => {
            let s = 0;
            let m = 0;
            for (let y = Math.max(0, y0 - 1); y <= Math.min(img.h - 1, y0 + 1); y++) {
              s += textBand[y]! * y;
              m += textBand[y]!;
            }
            return m ? s / m : y0;
          };
          relSpread.push(cy - (ruleCentre(top) + ruleCentre(bottom)) / 2);
          if (strip.length < 6) strip.push([zoom(img, 2)]);
        }
        const spread = relSpread.length ? Math.max(...relSpread) - Math.min(...relSpread) : 0;
        lines.push(
          `${variant} @ ${scale} ${kind}: max |spacing - mean| ${wobble.toFixed(3)}, max fit residual ${resid.toFixed(3)}, text-vs-rules spread ${spread.toFixed(3)}`,
        );
        summary[`${variant}@${scale}-${kind}`] = { wobble, resid, spread };
        writeImg(join(cmp, `motion-${variant}@${scale}-${kind}.png`), sheet(strip));
      }
      const pins = ["pinned-early", "pinned-late", "pinned-translate"].map((k) => {
        const s = shot(meta, k);
        return crop(s.file, deviceRect({ x: 20, y: 20, w: 420, h: 110 }, scale, 0));
      });
      writeImg(join(cmp, `pinned-${variant}@${scale}.png`), sheet([[pins[0]!], [pins[1]!], [pins[2]!]]));
    }
  }
  for (const scale of SCALES) {
    const rows: Img[][] = [];
    for (const variant of MOTION) {
      const meta = metaOf(out, variant, scale);
      if (!meta) continue;
      rows.push([zoom(crop(shot(meta, "pinned-late").file, deviceRect({ x: 36, y: 30, w: 200, h: 60 }, scale, 0)), 3)]);
    }
    if (rows.length) writeImg(join(cmp, `pinned-1.14-all@${scale}.png`), sheet(rows));
  }
  const base = metaOf(out, "baseline", 1);
  if (base) lines.unshift(`# text-exp report\n\nlog (baseline @1):\n${base.log.slice(0, 6).join("\n")}`);
  writeFileSync(join(out, `report${SUFFIX}.txt`), lines.join("\n"));
  writeFileSync(join(out, `motion${SUFFIX}.json`), JSON.stringify(summary, null, 1));
  console.log(lines.join("\n"));
}

// ---- font-smoothing over Mica ----

const MICA_RUNS = [
  { name: "mica-auto", smooth: "", backdrop: "" },
  { name: "mica-subpixel", smooth: "subpixel", backdrop: "" },
  { name: "opaque-auto", smooth: "", backdrop: "none" },
];

async function mica(out: string) {
  const lines: string[] = ["# font-smoothing over Mica (premultiplied readback)"];
  for (const scale of SCALES) {
    const dir = join(out, "raw-mica", String(scale));
    mkdirSync(dir, { recursive: true });
    const app = await launch({
      app: "packages/test/fixtures/text-exp/src/main.ts",
      window: [900, 640],
      scale,
      label: "text-exp-mica",
    });
    const meta: Record<string, Shot> = {};
    try {
      await app.eval(`__SK_TEXT_EXP__.show("mica"); 'ok'`);
      for (const run of MICA_RUNS) {
        if (run.backdrop) await app.eval(`__SK_WINDOW__.appWindow.backdrop = ${JSON.stringify(run.backdrop)}; "ok"`);
        await app.eval(`__SK_TEXT_EXP__.smooth(${JSON.stringify(run.smooth)}); 'ok'`);
        await sleep(600);
        const file = join(dir, `${run.name}.png`);
        await app.send("screenshot", { path: file, backend: "render_to_image" });
        const boxes = (await app.query(".w")).nodes.flatMap((n) =>
          n.bounds ? [{ cls: (n.classes ?? []).find((c) => c.startsWith("k-")) ?? "", b: n.bounds }] : [],
        );
        meta[run.name] = { file, boxes };
      }
    } finally {
      await app.close();
    }
    writeFileSync(join(dir, "meta.json"), JSON.stringify(meta, null, 1));
    lines.push(`\n## scale ${scale}`, "run\tline\ttransparent px\tbad premultiplied px\tmax excess\tfringe\tink");
    for (const run of MICA_RUNS) {
      const s = meta[run.name]!;
      const png = readPng(s.file);
      for (const { cls, b } of s.boxes) {
        const r = deviceRect(b, scale);
        let clear = 0;
        let bad = 0;
        let excess = 0;
        let fringe = 0;
        let inkSum = 0;
        for (let y = r.y; y < r.y + r.h; y++)
          for (let x = r.x; x < r.x + r.w; x++) {
            const i = (y * png.width + x) * 4;
            const [cr, cg, cb, a] = [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!, png.data[i + 3]!];
            if (a < 255) clear++;
            const over = Math.max(cr, cg, cb) - a;
            if (over > 1) bad++;
            excess = Math.max(excess, over);
            fringe += Math.abs(cr - cb);
            inkSum += a < 255 ? a : 0;
          }
        lines.push(
          `${run.name}\t${cls.replace("k-mica-", "")}\t${clear}\t${bad}\t${excess}\t${(fringe / (r.w * r.h)).toFixed(2)}\t${(inkSum / (r.w * r.h)).toFixed(2)}`,
        );
      }
    }
    // The transparent plane composited over a mid grey, as a backdrop would show it.
    const rows: Img[][] = MICA_RUNS.map((run) => {
      const s = meta[run.name]!;
      const png = readPng(s.file);
      const keys = s.boxes.filter((x) => x.cls.includes("-clear-") || x.cls.includes("-card-"));
      const x0 = Math.min(...keys.map((k) => k.b.x)) - 12;
      const y0 = Math.min(...keys.map((k) => k.b.y)) - 8;
      const x1 = Math.max(...keys.map((k) => k.b.x + k.b.w)) + 12;
      const y1 = Math.max(...keys.map((k) => k.b.y + k.b.h)) + 8;
      const r = deviceRect({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, scale, 0);
      const img = new Img(r.w, r.h);
      for (let y = 0; y < r.h; y++)
        for (let x = 0; x < r.w; x++) {
          const i = ((r.y + y) * png.width + r.x + x) * 4;
          const a = png.data[i + 3]! / 255;
          img.set(
            x,
            y,
            [0, 1, 2].map((c) => Math.min(255, Math.round(png.data[i + c]! + 120 * (1 - a)))),
          );
        }
      return [img];
    });
    writeImg(join(out, `mica@${scale}.png`), sheet(rows));
  }
  writeFileSync(join(out, "report-mica.txt"), lines.join("\n"));
  console.log(lines.join("\n"));
}

// ---- pairs for the post ----

function union(
  boxes: { cls: string; b: Bounds }[],
  match: (cls: string) => boolean,
  padX: number,
  padY = padX,
): Bounds {
  const keys = boxes.filter((x) => match(x.cls));
  const x0 = Math.min(...keys.map((k) => k.b.x)) - padX;
  const y0 = Math.min(...keys.map((k) => k.b.y)) - padY;
  const x1 = Math.max(...keys.map((k) => k.b.x + k.b.w)) + padX;
  const y1 = Math.max(...keys.map((k) => k.b.y + k.b.h)) + padY;
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

function post(out: string) {
  const dir = join(out, "post");
  mkdirSync(dir, { recursive: true });
  const written: string[] = [];
  const pair = (name: string, before: Img, after: Img, z: number) => {
    for (const [tag, img] of [
      ["before", before],
      ["after", after],
    ] as const) {
      const file = join(dir, `${name}-${tag}.png`);
      writeImg(file, zoom(img, z));
      written.push(`${file} (${img.w * z}x${img.h * z})`);
    }
  };
  const base = metaOf(out, "baseline", 1);
  if (base) {
    const card = shot(base, "rest").boxes.find((x) => x.cls === "k-card")!.b;
    const r = deviceRect(
      { x: card.x - 6, y: card.y - card.h * 0.07 - 6, w: card.w * 0.6, h: card.h * 1.14 + 12 },
      1,
      0,
    );
    pair("settle", crop(shot(base, "pinned-early").file, r), crop(shot(base, "pinned-late").file, r), 2);
    const half = metaOf(out, "contrast", 1);
    if (half) {
      const dark = shot(base, "dark");
      const rows = (file: string) =>
        sheet(
          [12, 13, 14].map((size) => {
            const words = (cls: string) => new RegExp(`^k-seg-400-${size}-[01]$`).test(cls);
            return [crop(file, deviceRect(union(dark.boxes, words, 2, 0), 1, 0))];
          }),
          2,
          BG.dark,
        );
      pair("contrast", rows(dark.file), rows(shot(half, "dark").file), 2);
    }
  }
  const micaMeta = join(out, "raw-mica", "1", "meta.json");
  if (existsSync(micaMeta)) {
    const m = JSON.parse(readFileSync(micaMeta, "utf8")) as Record<string, Shot>;
    const auto = m["mica-auto"]!;
    const r = deviceRect(
      union(auto.boxes, (cls) => cls.startsWith("k-mica-card-light-"), 6),
      1,
      0,
    );
    pair("mica-subpixel", crop(auto.file, r), crop(m["mica-subpixel"]!.file, r), 2);
  }
  console.log(written.join("\n"));
}

// ---- full-size shots of the text-post page ----

/** The readback is premultiplied with the Mica backdrop left out; this fill stands in for it. */
const FILL = [0x20, 0x20, 0x20];

function flatten(file: string, r?: Rect): Img {
  const png = readPng(file);
  const area = r ?? { x: 0, y: 0, w: png.width, h: png.height };
  const img = new Img(area.w, area.h);
  for (let y = 0; y < area.h; y++)
    for (let x = 0; x < area.w; x++) {
      const i = ((area.y + y) * png.width + area.x + x) * 4;
      const a = png.data[i + 3]! / 255;
      img.set(
        x,
        y,
        [0, 1, 2].map((c) => Math.min(255, Math.round(png.data[i + c]! + FILL[c]! * (1 - a)))),
      );
    }
  return img;
}

async function post2(out: string) {
  const scale = Number(process.env.RIG_POST_SCALE ?? 2);
  const dir = join(out, "post2", scale === 2 ? "" : `x${scale}`);
  const raw = join(dir, "raw");
  mkdirSync(raw, { recursive: true });
  const app = await launch({
    app: "packages/test/fixtures/text-post/src/main.ts",
    window: [1280, 680],
    scale,
    label: "text-post",
  });
  const shots: Record<string, string> = {};
  const bounds = async (selector: string) =>
    (await app.query(selector)).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
  try {
    const page = (js: string) => app.eval(`__SK_TEXT_POST__.${js}; 'ok'`);
    const shoot = async (name: string) => {
      const file = join(raw, `${name}.png`);
      await app.send("screenshot", { path: file, backend: "render_to_image" });
      shots[name] = file;
    };
    await sleep(1500);
    await page(`zoom(true)`);
    await sleep(70);
    await shoot("zoom-before");
    await sleep(900);
    await shoot("zoom-after");
    await page(`zoom(false)`);
    await sleep(400);
    await shoot("smoothing-auto");
    await page(`with("subpixel")`);
    await sleep(400);
    await shoot("smoothing-subpixel");
    await page(`with("contrast")`);
    await sleep(400);
    await shoot("contrast-on");
    await page(`with("")`);
    await sleep(400);
    await shoot("contrast-off");
    await shoot("line-height-normal");
    await page(`with("tall")`);
    await sleep(400);
    await shoot("line-height-1.618");
    await page(`with("")`);

    const [card] = await bounds(".media");
    const [info] = await bounds(".info");
    const rows = await bounds(".row");
    const [lead] = await bounds(".lead");
    const centred = (cx: number, cy: number): Rect => {
      const w = 1200;
      const h = 500;
      return {
        x: Math.min(1280 * scale - w, Math.max(0, Math.round(cx * scale - w / 2))),
        y: Math.min(680 * scale - h, Math.max(0, Math.round(cy * scale - h / 2))),
        w,
        h,
      };
    };
    const zoomAt = (v: number, c: number) => c + (v - c) * 1.14;
    const cardCx = card!.x + card!.w / 2;
    const cardCy = card!.y + card!.h / 2;
    const zoomCrop = centred(zoomAt(info!.x + info!.w / 2, cardCx), zoomAt(info!.y + info!.h / 2, cardCy));
    const rowsTop = rows[0]!.y;
    const rowsBottom = rows[rows.length - 1]!.y + rows[rows.length - 1]!.h;
    const rowsCrop = centred(rows[0]!.x + rows[0]!.w / 2, (rowsTop + rowsBottom) / 2);
    const leadCrop = centred(lead!.x + 300, lead!.y + 60);
    const pairs: [string, string, string, Rect][] = [
      ["zoom", "zoom-before", "zoom-after", zoomCrop],
      ["smoothing", "smoothing-auto", "smoothing-subpixel", rowsCrop],
      ["contrast", "contrast-off", "contrast-on", rowsCrop],
      ["line-height", "line-height-1.618", "line-height-normal", leadCrop],
    ];
    for (const [pair, a, b, r] of pairs) {
      for (const name of [a, b]) {
        writeImg(join(dir, `${name}.png`), flatten(shots[name]!));
        writeImg(join(dir, `${name}-crop.png`), flatten(shots[name]!, r));
      }
      console.log(`${pair}: ${a} / ${b}, crop ${JSON.stringify(r)}`);
    }
  } finally {
    await app.close();
  }
}

const [cmd, out, ...rest] = process.argv.slice(2);
if (!out) throw new Error("usage: rig.ts capture|report|mica|post <out> [variant...]");
if (cmd === "capture") await capture(out, rest);
else if (cmd === "mica") await mica(out);
else if (cmd === "post") post(out);
else if (cmd === "post2") await post2(out);
else report(out);
