// getBoundingClientRect() under CSS transforms. A browser reports where the box
// was DRAWN, transforms applied, from getBoundingClientRect(), getClientRects()
// and IntersectionObserver, and the LAYOUT box from offsetWidth, clientWidth and
// ResizeObserver. Ours reported the layout box everywhere, so a popper anchored
// to anything inside a translated popper or a centred dialog opened in the
// window's top-left corner: the Argon profile popover's three-dot menu.
//
//   bun test packages/vn-test/specs/transform-rects.spec.ts
//
// `fixtures/transform-rects` lays the shapes out at fixed positions on white.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/transform-rects/src/main.ts",
  window: [720, 520],
  label: "transform-rects",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];
type Rect = [number, number, number, number];

/** Polls `read` until `ok` holds, and returns the last answer either way. */
async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const js = <V>(src: string) => app.eval(src) as Promise<V>;
const state = <V>(path: string) => js<V>(`JSON.parse(JSON.stringify(globalThis.__VN_TEST_TRANSFORM__.${path} ?? null))`);
const bcr = (id: string) =>
  js<Rect>(`(() => { const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map((v) => Math.round(v * 10) / 10); })()`);
const layout = (id: string) =>
  js<number[]>(`(() => { const e = document.getElementById(${JSON.stringify(id)}); return [e.offsetLeft, e.offsetTop, e.offsetWidth, e.offsetHeight, e.clientWidth, e.clientHeight]; })()`);

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};
const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const lime = ([r, g, b]: Rgb) => g > 200 && r < 60 && b < 60;

describe("a popper moved by `translate`, and one anchored inside it", () => {
  test("the wrapper and what is inside it report where they are drawn", async () => {
    // The trigger at (520, 20), 120x32: floating-ui's `bottom-start` with a
    // 4px offset puts the wrapper at translate(520px, 56px).
    await until(() => state<Rect | null>("anchors.pop"), (a) => a !== null);
    bunExpect(await bcr("pop")).toEqual([520, 56, 160, 100]);
    // The menu button's layout box is at the wrapper's (0, 0) plus its margins…
    const [ol, ot, ow, oh] = await layout("menu-btn");
    bunExpect([ol, ot, ow, oh]).toEqual([110, 10, 30, 30]);
    // …and it is drawn 520, 56 further on.
    bunExpect(await bcr("menu-btn")).toEqual([630, 66, 30, 30]);
    const rects = await js<number[]>(
      `(() => { const r = document.getElementById("menu-btn").getClientRects(); return [r.length, r[0].left, r[0].top]; })()`,
    );
    bunExpect(rects).toEqual([1, 630, 66]);
  }, T);

  test("the menu it opens is placed under the button, not the window's corner", async () => {
    await app.locator("#menu-btn").click({ real: true });
    const anchor = await until(() => state<Rect | null>("anchors.menu"), (a) => a !== null);
    bunExpect(anchor).toEqual([630, 66, 30, 30]);
    bunExpect(await bcr("menu")).toEqual([630, 100, 60, 40]);
    const at = await until(shoot, (at) => red(at(660, 120)));
    bunExpect(red(at(660, 120))).toBe(true);
    // Where the layout box would have put it.
    bunExpect(red(at(140, 64))).toBe(false);
  }, T);
});

describe("a dialog centred with `translate(-50%, -50%)`, with a select inside", () => {
  test("the dialog and its trigger report the centred box", async () => {
    // 300x200, `left: 50%; top: 50%` of 720x520 lays it out at (360, 260).
    bunExpect((await layout("dialog")).slice(2, 4)).toEqual([300, 200]);
    bunExpect(await bcr("dialog")).toEqual([210, 160, 300, 200]);
    bunExpect(await bcr("select-trigger")).toEqual([230, 180, 120, 28]);
  }, T);

  test("the select's list opens under its trigger", async () => {
    await app.locator("#select-trigger").click({ real: true });
    const anchor = await until(() => state<Rect | null>("anchors.select"), (a) => a !== null);
    bunExpect(anchor).toEqual([230, 180, 120, 28]);
    bunExpect(await bcr("select")).toEqual([230, 212, 120, 60]);
    const at = await until(shoot, (at) => lime(at(290, 240)));
    bunExpect(lime(at(290, 240))).toBe(true);
  }, T);
});

describe("a `scale(0.95)` open animation", () => {
  test("offsetWidth and ResizeObserver keep the layout box while the drawn one grows", async () => {
    await js(`globalThis.__VN_TEST_TRANSFORM__.startZoom()`);
    type Sample = { offsetWidth: number; clientWidth: number; rect: Rect; clientRectWidth: number };
    const sample = () => js<Sample>(`globalThis.__VN_TEST_TRANSFORM__.sampleZoom()`);
    // 3s from 0.95 to 1 about the centre: early on the drawn box is ~191 wide.
    const s = await until(sample, (s) => s.rect[2] < 199.5);
    bunExpect(s.offsetWidth).toBe(200);
    bunExpect(s.clientWidth).toBe(200);
    bunExpect(s.rect[2]).toBeLessThan(199.5);
    bunExpect(s.rect[2]).toBeGreaterThan(188);
    bunExpect(s.rect[0]).toBeGreaterThan(40);
    bunExpect(s.clientRectWidth).toBeCloseTo(s.rect[2], 0);
    // Give ResizeObserver a few frames: the layout box did not change.
    await sleep(400);
    bunExpect((await sample()).offsetWidth).toBe(200);
    bunExpect(await state<number>("zoomResizes")).toBe(0);
  }, T);
});

describe("IntersectionObserver", () => {
  test("a target translated out of the window does not intersect it", async () => {
    type Io = { isIntersecting: boolean; rect: Rect };
    const io = await until(
      () => state<Record<string, Io>>("io"),
      (io) => !!io && "far" in io && "near" in io,
    );
    // Laid out at (40, 330), drawn at (840, 330): past the 720px window.
    bunExpect(io.far.isIntersecting).toBe(false);
    bunExpect(io.far.rect).toEqual([840, 330, 40, 40]);
    bunExpect(io.near.isIntersecting).toBe(true);
  }, T);
});
