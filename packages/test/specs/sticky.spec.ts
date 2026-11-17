// `position: sticky`: the shift is applied at paint time from the scroll offset, so the rectangle a
// script reads, the pixels and the hit test must all agree on where the stuck box is.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, MACOS, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/sticky/src/main.ts",
  window: [720, 520],
  label: "sticky",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];
type Rect = { left: number; top: number; width: number; height: number };

const red = ([r, g, b]: Rgb) => r > 200 && g < 60 && b < 60;
const lime = ([r, g, b]: Rgb) => g > 160 && r < 60 && b < 60;
const blue = ([r, g, b]: Rgb) => b > 200 && r < 60 && g < 80;
const yellow = ([r, g, b]: Rgb) => r > 200 && g > 160 && b < 60;

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 720;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};

const rect = async (selector: string): Promise<Rect> =>
  JSON.parse(
    (await app.eval(
      `(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();
        return JSON.stringify({ left: Math.round(r.left), top: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height) }); })()`,
    )) as string,
  ) as Rect;

const scroll = (selector: string, left: number, top: number) =>
  app.eval(
    `(() => { const s = document.querySelector(${JSON.stringify(selector)}); s.scrollLeft = ${left}; s.scrollTop = ${top}; return s.scrollTop; })()`,
  );

const hitId = (x: number, y: number) => app.eval(`document.elementFromPoint(${x}, ${y})?.id ?? ""`);

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const painted = () =>
  until(
    () => rect("#head-1"),
    (r) => r.height === 30,
    10_000,
  );

describe("a section header in a scrolling list", () => {
  test(
    "stays at the top of the scrollport while its section scrolls under it",
    async () => {
      await painted();
      const list = await rect("#list");
      bunExpect(await scroll("#list", 0, 200)).toBe(200);
      await sleep(300);
      bunExpect((await rect("#head-1")).top).toBe(list.top);
      bunExpect((await rect('[data-row="1-0"]')).top).toBe(list.top + 30 - 200);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "paints over the rows after it, and takes the hit there",
    async () => {
      await scroll("#list", 0, 200);
      await sleep(300);
      const at = await shoot();
      bunExpect(red(at(200, 15))).toBe(true);
      bunExpect(lime(at(200, 60))).toBe(true);
      bunExpect(await hitId(200, 15)).toBe("head-1");
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "is pushed out by the end of its section",
    async () => {
      const list = await rect("#list");
      await scroll("#list", 0, 420);
      await sleep(300);
      // Section 1 ends 10px into the scrollport: its header's bottom edge rides that edge.
      bunExpect((await rect("#head-1")).top).toBe(list.top + 10 - 30);
      bunExpect((await rect("#head-2")).top).toBe(list.top + 10);
      const at = await shoot();
      bunExpect(red(at(200, 5))).toBe(true);
      bunExpect(blue(at(200, 25))).toBe(true);

      await scroll("#list", 0, 600);
      await sleep(300);
      bunExpect((await rect("#head-2")).top).toBe(list.top);
      bunExpect((await rect("#head-1")).top).toBeLessThan(list.top - 30);
      bunExpect(await hitId(200, 15)).toBe("head-2");
    },
    T,
  );

  test(
    "a read in the same task as the scroll already sees the stuck position",
    async () => {
      const list = await rect("#list");
      await scroll("#list", 0, 0);
      await sleep(300);
      const top = await app.eval(
        `(() => { const l = document.querySelector("#list"); l.scrollTop = 250;
          return Math.round(document.querySelector("#head-1").getBoundingClientRect().top); })()`,
      );
      bunExpect(top).toBe(list.top);
    },
    T,
  );

  test(
    "offsetTop and getComputedStyle report it as a browser does",
    async () => {
      await scroll("#list", 0, 0);
      await sleep(300);
      const before = await app.eval(`document.querySelector("#head-1").offsetTop`);
      await scroll("#list", 0, 200);
      await sleep(300);
      bunExpect(await app.eval(`document.querySelector("#head-1").offsetTop`)).toBe(before);
      bunExpect(await app.eval(`getComputedStyle(document.querySelector("#head-1")).position`)).toBe("sticky");
    },
    T,
  );

  test.skipIf(MACOS)(
    "IntersectionObserver sees the stuck header inside the list",
    async () => {
      await scroll("#list", 0, 0);
      await sleep(300);
      await app.eval(
        `(() => {
          globalThis.__stickyIo = [];
          const head = document.querySelector("#head-1");
          new IntersectionObserver((entries) => {
            for (const e of entries) globalThis.__stickyIo.push(e.isIntersecting);
          }, { root: document.querySelector("#list") }).observe(head);
          return 0;
        })()`,
      );
      const last = () => app.eval(`globalThis.__stickyIo.length ? globalThis.__stickyIo.at(-1) : null`);
      bunExpect(await until(last, (v) => v === true)).toBe(true);
      await scroll("#list", 0, 300);
      await sleep(500);
      bunExpect(await last()).toBe(true);
      await scroll("#list", 0, 800);
      bunExpect(await until(last, (v) => v === false)).toBe(false);
    },
    T,
  );
});

describe("a 2D grid with a sticky header row, first column and corner", () => {
  // macOS CI: the sticky boxes do not stay put when the scroller scrolls.
  test.skipIf(MACOS)(
    "keeps all three in place when scrolled on both axes",
    async () => {
      await painted();
      const grid = await rect("#grid");
      await scroll("#grid", 300, 300);
      await sleep(300);
      bunExpect(await rect("#corner")).toEqual({ left: grid.left, top: grid.top, width: 100, height: 30 });
      bunExpect((await rect("#col-3")).top).toBe(grid.top);
      bunExpect((await rect("#col-3")).left).toBe(grid.left + 300 - 300);
      bunExpect((await rect("#cell-20-0")).left).toBe(grid.left);
      bunExpect((await rect("#cell-20-1")).left).toBe(grid.left + 100 - 300);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS || MACOS)(
    "paints the corner over the header, and both over the column and the body",
    async () => {
      const grid = await rect("#grid");
      await scroll("#grid", 300, 300);
      await sleep(300);
      const at = await shoot();
      const x = grid.left;
      const y = grid.top;
      bunExpect(yellow(at(x + 50, y + 15))).toBe(true);
      bunExpect(blue(at(x + 150, y + 15))).toBe(true);
      bunExpect(red(at(x + 50, y + 100))).toBe(true);
      bunExpect(lime(at(x + 150, y + 100))).toBe(true);
      bunExpect(await hitId(x + 50, y + 15)).toBe("corner");
      bunExpect(await hitId(x + 150, y + 15)).toBe("col-4");
    },
    T,
  );
});

describe("nested scroll containers", () => {
  test(
    "the nearest one is the reference, and a calc() threshold resolves against its height",
    async () => {
      await painted();
      await scroll("#outer", 0, 0);
      await scroll("#inner", 0, 0);
      await sleep(300);
      const threshold = Math.round(120 * 0.1 + 5);
      let inner = await rect("#inner");
      bunExpect((await rect("#inner-head")).top).toBe(inner.top + threshold);

      await scroll("#inner", 0, 100);
      await sleep(300);
      bunExpect((await rect("#inner-head")).top).toBe(inner.top + threshold);

      await scroll("#outer", 0, 30);
      await sleep(300);
      inner = await rect("#inner");
      bunExpect(inner.top).toBe((await rect("#outer")).top + 50 - 30);
      bunExpect((await rect("#inner-head")).top).toBe(inner.top + threshold);
    },
    T,
  );
});
