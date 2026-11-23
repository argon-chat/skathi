import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Bounds, launch, readPng, type SkApp, WINDOWS } from "../src/index";

// Text under a transform that has held still is rasterized at its device size, as sharp as the same
// text laid out at that size, whether it repaints or sits in a cached subtree that is replayed.
const T = 60_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Edge contrast per unit of ink: lower when glyphs are resampled between pixels. */
function sharpness(file: string, b: Bounds, scale: number, grow: number): number {
  const png = readPng(file);
  const x0 = Math.floor(b.x * scale);
  const y0 = Math.floor(b.y * scale);
  const x1 = Math.ceil((b.x + b.w * grow) * scale);
  const y1 = Math.ceil((b.y + b.h * grow) * scale);
  let ink = 0;
  let edge = 0;
  for (let y = y0; y < y1; y++)
    for (let x = x0; x < x1; x++) {
      const v = 255 - png.luma(x, y);
      ink += v;
      edge += Math.abs(v - (255 - png.luma(x + 1, y))) + Math.abs(v - (255 - png.luma(x, y + 1)));
    }
  return edge / Math.max(ink, 1);
}

for (const scale of WINDOWS ? [1, 1.5] : []) {
  describe(`at scale ${scale}`, () => {
    let app: SkApp;
    beforeAll(async () => {
      app = await launch({ app: "packages/test/fixtures/text-settle/src/main.ts", scale, label: "text-settle" });
    }, T);
    afterAll(() => app.close());

    async function measure(): Promise<{ scaled: number; laidOut: number }> {
      const boxes = (await app.query(".box")).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
      expect(boxes.length).toBeGreaterThan(3);
      const file = join(tmpdir(), `sk-text-settle-${process.pid}.png`);
      await app.send("screenshot", { path: file, backend: "render_to_image" });
      return {
        scaled: sharpness(file, boxes[0]!, scale, 1.14),
        laidOut: sharpness(file, boxes[1]!, scale, 1),
      };
    }

    test("scaled text in a subtree replayed from the cache while a sibling repaints every frame becomes as sharp as text laid out at that size", async () => {
      await app.eval("__SK_TEST_SETTLE__.start(); __SK_TEST_SETTLE__.show(true); 'ok'");
      await sleep(900);
      const { scaled, laidOut } = await measure();
      await app.eval("__SK_TEST_SETTLE__.stop(); __SK_TEST_SETTLE__.show(false); 'ok'");
      expect(scaled).toBeGreaterThan(laidOut * 0.96);
    });

    test("so does scaled text with nothing else painting", async () => {
      await sleep(300);
      await app.eval("__SK_TEST_SETTLE__.show(true); 'ok'");
      await sleep(900);
      const { scaled, laidOut } = await measure();
      expect(scaled).toBeGreaterThan(laidOut * 0.96);
    });

    test("so does scaled text in a list that has scrolled and stopped", async () => {
      for (let top = 40; top <= 520; top += 40) {
        await app.eval(`document.getElementById("list").scrollTop = ${top}; 'ok'`);
        await sleep(30);
      }
      await sleep(900);
      const bounds = async (selector: string) =>
        (await app.query(selector)).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
      const [list] = await bounds("#list");
      const scaled = await bounds(".list-scaled");
      const big = await bounds(".list-big");
      const row = scaled.findIndex((b) => b.y >= list!.y && b.y + b.h * 1.14 <= list!.y + list!.h);
      expect(row).toBeGreaterThan(8);
      const file = join(tmpdir(), `sk-text-settle-list-${process.pid}.png`);
      await app.send("screenshot", { path: file, backend: "render_to_image" });
      expect(sharpness(file, scaled[row]!, scale, 1.14)).toBeGreaterThan(sharpness(file, big[row]!, scale, 1) * 0.96);
    });

    test("text in a translating will-change: transform box moves with its rules, without stepping", async () => {
      await app.eval("__SK_TEST_SETTLE__.glide('moving'); 'ok'");
      await sleep(300);
      const [layer, plain] = (await app.query(".glider")).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
      const offsets: { layer: number[]; plain: number[] } = { layer: [], plain: [] };
      for (let i = 0; i < 12; i++) {
        const file = join(tmpdir(), `sk-text-settle-glide-${process.pid}.png`);
        await app.send("screenshot", { path: file, backend: "render_to_image" });
        offsets.layer.push(textAgainstRules(file, layer!, scale));
        offsets.plain.push(textAgainstRules(file, plain!, scale));
        await sleep(47 + ((i * 53) % 41));
      }
      await app.eval("__SK_TEST_SETTLE__.glide(''); 'ok'");
      const spread = (v: number[]) => Math.max(...v) - Math.min(...v);
      console.log(
        `scale ${scale}: text vs rules spread, will-change ${spread(offsets.layer).toFixed(3)}, plain ${spread(offsets.plain).toFixed(3)}`,
      );
      expect(spread(offsets.layer)).toBeLessThan(0.35);
    });

    test("and is as sharp as still text once it stops", async () => {
      await app.eval("__SK_TEST_SETTLE__.glide('pinned'); 'ok'");
      await sleep(900);
      const [layer, , still] = (await app.query(".hs")).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
      const file = join(tmpdir(), `sk-text-settle-pinned-${process.pid}.png`);
      await app.send("screenshot", { path: file, backend: "render_to_image" });
      await app.eval("__SK_TEST_SETTLE__.glide(''); 'ok'");
      const moved = { x: layer!.x, y: layer!.y, w: layer!.w + 5, h: layer!.h + 3 };
      expect(sharpness(file, moved, scale, 1)).toBeGreaterThan(sharpness(file, still!, scale, 1) * 0.96);
    });

    test("text under a running animation is resampled, not rasterized every frame", async () => {
      await app.eval("__SK_TEST_SETTLE__.spin(true); 'ok'");
      await sleep(700);
      const before = (await app.send("mem", {})) as unknown;
      await sleep(700);
      const after = (await app.send("mem", {})) as unknown;
      await app.eval("__SK_TEST_SETTLE__.spin(false); 'ok'");
      expect(glyphTiles(after)).toBe(glyphTiles(before));
    });
  });
}

/** The text's ink centre less the midpoint of the two rules around it, in device pixels. */
function textAgainstRules(file: string, b: Bounds, scale: number): number {
  const png = readPng(file);
  const x0 = Math.floor(b.x * scale);
  const x1 = Math.ceil((b.x + b.w + 10) * scale);
  const y0 = Math.floor((b.y - 2) * scale);
  const y1 = Math.ceil((b.y + b.h + 6) * scale);
  const rows: number[] = [];
  for (let y = y0; y < y1; y++) {
    let ink = 0;
    for (let x = x0; x < x1; x++) ink += 255 - png.luma(x, y);
    rows.push(ink);
  }
  const max = Math.max(...rows);
  const ruled = rows.map((v) => v > max * 0.3);
  const top = ruled.indexOf(true);
  const bottom = ruled.lastIndexOf(true);
  const centre = (from: number, to: number) => {
    let mass = 0;
    let moment = 0;
    for (let y = from; y <= to; y++) {
      mass += rows[y]!;
      moment += rows[y]! * y;
    }
    return moment / Math.max(mass, 1);
  };
  let topEnd = top;
  while (ruled[topEnd + 1]) topEnd++;
  let bottomStart = bottom;
  while (ruled[bottomStart - 1]) bottomStart--;
  const ruleTop = centre(Math.max(0, top - 1), topEnd + 1);
  const ruleBottom = centre(bottomStart - 1, Math.min(rows.length - 1, bottom + 1));
  const text = centre(topEnd + 2, bottomStart - 2);
  return text - (ruleTop + ruleBottom) / 2;
}

function glyphTiles(mem: unknown): number {
  const find = (o: unknown): unknown => {
    if (!o || typeof o !== "object") return undefined;
    const r = o as Record<string, unknown>;
    if ("atlas" in r) return r.atlas;
    for (const v of Object.values(r)) {
      const f = find(v);
      if (f !== undefined) return f;
    }
    return undefined;
  };
  const atlas = find(mem) as Record<string, { tiles?: number }> | undefined;
  return (atlas?.monochrome?.tiles ?? 0) + (atlas?.subpixel?.tiles ?? 0);
}
