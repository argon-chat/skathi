import { afterAll, describe, expect, test } from "bun:test";
import { launch, type Png, readPng, SCREENSHOTS } from "../src/index";

const W = 600;
const app = await launch({
  app: "packages/test/fixtures/transformed-text/src/main.ts",
  window: [W, 300],
  label: "transformed-text",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Box = [number, number, number, number];
type Matrix = [number, number, number, number, number, number];

/** A line's untransformed layout box and its `transform` about the box centre, both in CSS px. */
interface Line {
  box: Box;
  matrix: Matrix;
}

const layoutBox = (id: string) =>
  app.eval(`(() => {
    const e = document.getElementById(${JSON.stringify(id)});
    return [e.offsetLeft, e.offsetTop, e.offsetWidth, e.offsetHeight];
  })()`) as Promise<Box>;

/** A line whose transform only translates, by where its box landed. */
const moved = async (id: string): Promise<Line> => {
  const box = await layoutBox(id);
  const [x, y] = (await app.eval(
    `(() => { const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return [r.left, r.top]; })()`,
  )) as [number, number];
  return { box, matrix: [1, 0, 0, 1, x - box[0], y - box[1]] };
};

const rotated = async (id: string, deg: number): Promise<Line> => {
  const [cos, sin] = [Math.cos((deg * Math.PI) / 180), Math.sin((deg * Math.PI) / 180)];
  return { box: await layoutBox(id), matrix: [cos, sin, -sin, cos, 0, 0] };
};

/** Pen positions along the line, after "S", "Save", "Saved", "Saved ", "Saved draw" and "Saved draw-". */
const pens = () =>
  app.eval(`Object.fromEntries(
    ["s", "save", "saved", "space", "draw", "hyphen"].map((key) => [
      key,
      document.getElementById("prefix-" + key).getBoundingClientRect().width,
    ]),
  )`) as Promise<Record<"s" | "save" | "saved" | "space" | "draw" | "hyphen", number>>;

/** The luma of every device pixel whose centre, mapped back into the line's own frame, falls in `x0..x1`. */
function* local(shot: Png, k: number, { box, matrix }: Line, x0: number, x1: number) {
  const [left, top, w, h] = box;
  const [a, b, c, d, e, f] = matrix;
  const det = a * d - b * c;
  const [cx, cy] = [left + w / 2, top + h / 2];
  for (let py = 0; py < shot.height; py++) {
    for (let px = 0; px < shot.width; px++) {
      const gx = (px + 0.5) / k - cx - e;
      const gy = (py + 0.5) / k - cy - f;
      const x = (d * gx - c * gy) / det + w / 2;
      const y = (-b * gx + a * gy) / det + h / 2;
      if (x < x0 || x > x1 || y < 0 || y > h) continue;
      const [r, g, bl] = shot.at(px, py);
      yield { y, luma: (r + g + bl) / 3 };
    }
  }
}

describe("transformed text", () => {
  test.skipIf(!SCREENSHOTS)(
    "draws nothing beside the ascenders of the words a fractional transform moves",
    async () => {
      await sleep(500);
      const pen = await pens();
      const shot = readPng(await app.screenshot());
      const k = shot.width / W;
      const lines = {
        centred: await moved("centred"),
        nudged: await moved("nudged"),
        rotated: await rotated("rotated", 3),
      };
      // The bars stood at the left edge of the "d" starting "draw" and of the "f" after the
      // hyphen, full height. Above the x-height (the top of "ave"), only the ascenders of the
      // letters either side reach those gaps.
      const report = Object.entries(lines).map(([id, line]) => {
        let xHeight = Infinity;
        for (const p of local(shot, k, line, pen.s + 1, pen.save - 1)) {
          if (p.luma < 128) xHeight = Math.min(xHeight, p.y);
        }
        const darkest = (x0: number, x1: number) => {
          let min = 255;
          for (const p of local(shot, k, line, x0, x1)) if (p.y < xHeight - 0.5) min = Math.min(min, p.luma);
          return Math.round(min);
        };
        return {
          id,
          inked: xHeight < line.box[3],
          space: darkest(pen.saved + 0.5, pen.space + 2),
          hyphen: darkest(pen.draw + 0.5, pen.hyphen + 0.25),
        };
      });
      expect(report.filter((r) => !r.inked || r.space < 240 || r.hyphen < 240)).toEqual([]);
    },
    T,
  );
});
