import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const W = 560;
const app = await launch({
  app: "packages/vn-test/fixtures/svg-paint/src/main.ts",
  window: [W, 360],
  label: "svg-paint",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

const box = async (selector: string): Promise<Bounds> => {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / W;
  return { at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)) };
};
type Shot = Awaited<ReturnType<typeof shoot>>;

async function until(ok: (s: Shot) => boolean, ms = 5_000): Promise<Shot> {
  const deadline = Date.now() + ms;
  let s = await shoot();
  while (!ok(s) && Date.now() < deadline) {
    await sleep(80);
    s = await shoot();
  }
  return s;
}

const near =
  (want: Rgb, tol = 40) =>
  (got: Rgb) =>
    got.every((v, i) => Math.abs(v - want[i]) <= tol);
const red = near([255, 0, 0]);
const green = near([0, 160, 0]);
const magenta = near([255, 0, 255]);
const blue = near([0, 0, 255]);
const white = near([255, 255, 255], 12);

const said = (s: Shot, x: number, y: number) => `rgb(${s.at(x, y).join(", ")}) at (${x}, ${y})`;

describe("an inline <svg> painted from CSS rules", () => {
  test(
    "fill from a class rule through var(), stroke colour and width from the rule",
    async () => {
      const b = await box("#outline");
      expect(b).toMatchObject({ w: 100, h: 60 });
      const s = await until((s) => red(s.at(b.x + 50, b.y + 30)));
      expect(red(s.at(b.x + 50, b.y + 30)), said(s, b.x + 50, b.y + 30)).toBe(true);
      for (const x of [7, 10, 13]) expect(green(s.at(b.x + x, b.y + 30)), said(s, b.x + x, b.y + 30)).toBe(true);
      expect(white(s.at(b.x + 3, b.y + 30)), said(s, b.x + 3, b.y + 30)).toBe(true);
      expect(red(s.at(b.x + 17, b.y + 30)), said(s, b.x + 17, b.y + 30)).toBe(true);
      expect(green(s.at(b.x + 50, b.y + 10)), said(s, b.x + 50, b.y + 10)).toBe(true);
    },
    T,
  );

  test(
    "a class toggle repaints the stroke, and toggling back restores it",
    async () => {
      const b = await box("#outline");
      expect(await app.eval(`globalThis.__VN_TEST_SVG__.toggle()`)).toBe(true);
      let s = await until((s) => magenta(s.at(b.x + 10, b.y + 30)));
      expect(magenta(s.at(b.x + 10, b.y + 30)), said(s, b.x + 10, b.y + 30)).toBe(true);
      expect(red(s.at(b.x + 50, b.y + 30)), said(s, b.x + 50, b.y + 30)).toBe(true);
      expect(await app.eval(`globalThis.__VN_TEST_SVG__.toggle()`)).toBe(false);
      s = await until((s) => green(s.at(b.x + 10, b.y + 30)));
      expect(green(s.at(b.x + 10, b.y + 30)), said(s, b.x + 10, b.y + 30)).toBe(true);
    },
    T,
  );

  test(
    "a presentation attribute loses to the rule on its own element",
    async () => {
      const b = await box("#attr-loses");
      const s = await until((s) => !white(s.at(b.x + 50, b.y + 30)));
      expect(red(s.at(b.x + 50, b.y + 30)), said(s, b.x + 50, b.y + 30)).toBe(true);
      expect(white(s.at(b.x + 4, b.y + 4)), said(s, b.x + 4, b.y + 4)).toBe(true);
    },
    T,
  );

  test(
    "a rule that matches a shape inside the svg beats the shape's attribute",
    async () => {
      const b = await box("#inner-rule");
      const s = await until((s) => !white(s.at(b.x + 50, b.y + 30)));
      expect(red(s.at(b.x + 50, b.y + 30)), said(s, b.x + 50, b.y + 30)).toBe(true);
    },
    T,
  );

  test(
    "the inline style beats the rule",
    async () => {
      const b = await box("#inline-wins");
      const s = await until((s) => !white(s.at(b.x + 50, b.y + 30)));
      expect(green(s.at(b.x + 50, b.y + 30)), said(s, b.x + 50, b.y + 30)).toBe(true);
    },
    T,
  );

  test(
    "a shape's own attribute beats what it would inherit from the svg's rule",
    async () => {
      const b = await box("#inherit-loses");
      const s = await until((s) => !white(s.at(b.x + 50, b.y + 30)));
      expect(green(s.at(b.x + 50, b.y + 30)), said(s, b.x + 50, b.y + 30)).toBe(true);
      for (const sel of ["#outline", "#attr-loses", "#inner-rule", "#inline-wins", "#inherit-loses"]) {
        const c = await box(sel);
        expect(blue(s.at(c.x + 50, c.y + 30)), `${sel}: ${said(s, c.x + 50, c.y + 30)}`).toBe(false);
      }
    },
    T,
  );
});

describe("filter: drop-shadow() on an inline <svg>", () => {
  test(
    "the shadow follows the circle, not the box, and reaches past the box",
    async () => {
      const b = await box("#shadowed");
      expect(b).toMatchObject({ w: 60, h: 60 });
      const [cx, cy] = [b.x + 30, b.y + 30];
      const s = await until((s) => red(s.at(cx + 30, cy)));
      const dark = near([0, 0, 0], 30);
      expect(dark(s.at(cx, cy)), said(s, cx, cy)).toBe(true);
      expect(red(s.at(cx + 30, cy)), said(s, cx + 30, cy)).toBe(true);
      expect(red(s.at(cx + 35, cy)), said(s, cx + 35, cy)).toBe(true);
      // The box's own shadow would cover its offset corner; the circle's does not.
      expect(white(s.at(b.x + 22, b.y + 2)), said(s, b.x + 22, b.y + 2)).toBe(true);
      expect(white(s.at(b.x + 78, b.y + 2)), said(s, b.x + 78, b.y + 2)).toBe(true);
      expect(white(s.at(cx - 25, cy)), said(s, cx - 25, cy)).toBe(true);
      const logs = JSON.stringify((await app.logs()).entries);
      expect(logs).not.toContain("drop-shadow()` is honoured only");
      expect(logs).not.toContain("cast no shadow");
      expect(logs).not.toContain("currentColor`)");
    },
    T,
  );
});
