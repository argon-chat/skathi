// The native `<select>` (docs/RUNTIME.md §5.28, crates/vn-tree/src/select.rs,
// the select model in packages/vue-native/src/runtime.ts): Chromium's closed
// box, the popup list, the keyboard, and `v-model.number` as the Argon
// client's settings use it.
//
//   bun test packages/vn-test/specs/select.spec.ts
//
// Presses and keys are REAL platform input through gpui (devtools `mouse` and
// `key`). The popup is no element, so it is found in screenshots: it takes
// the select's own yellow background, on a dark gray page.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const W = 460;
const app = await launch({
  app: "packages/vn-test/fixtures/select/src/main.ts",
  window: [W, 420],
  label: "select",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

async function js<V>(source: string): Promise<V> {
  return JSON.parse(String(await app.eval(`JSON.stringify((() => { ${source} })())`))) as V;
}

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

const box = async (selector: string): Promise<Bounds> => {
  const b = (await app.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

/** A screenshot, read at CSS pixel `(x, y)`'s centre whatever the scale. */
const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / W;
  return { at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)) };
};
const yellow = ([r, g, b]: Rgb) => r > 220 && g > 220 && b < 60;
const page = ([r, g, b]: Rgb) => Math.abs(r - 40) < 12 && Math.abs(g - 40) < 12 && Math.abs(b - 40) < 12;
const dark = ([r, g, b]: Rgb) => r + g + b < 300;
/** The highlighted row: Chromium's blue, light or dark. */
const lit = ([r, g, b]: Rgb) => b > 200 && r < 170 && g > 120;

/** Is a popup painted at `(x, y)`? Polled: it opens on the next frame. */
const popupAt = (x: number, y: number, open: boolean) =>
  until(
    async () => yellow((await shoot()).at(x, y)),
    (v) => v === open,
  );

/** A popup row's height: 14px text at line height 1.2, and 1px above and below. */
const ROW = 14 * 1.2 + 2;
/** The top of row `i` in a popup under `b` (inside its 1px border). */
const rowTop = (b: Bounds, i: number) => b.y + b.h + 1 + ROW * i;
/** Row `i`'s centre. */
const row = (b: Bounds, i: number) => ({ x: b.x + 20, y: rowTop(b, i) + ROW / 2 });
/**
 * Where a popup under `b` is looked for: the top of row `i`, which must not
 * be the highlighted one (that is not yellow), over the bare page.
 */
const probe = (b: Bounds, i: number) => ({ x: b.x + 20, y: rowTop(b, i) + 4 });

const click = (x: number, y: number) => app.send("mouse", { x, y, kind: "click" });
const log = () => js<string[]>("return globalThis.log.slice()");
const text = (sel: string) => js<string>(`return document.querySelector(${JSON.stringify(sel)}).textContent`);
const value = (sel: string) => js<string>(`return document.querySelector(${JSON.stringify(sel)}).value`);
const active = () => js<string>("return document.activeElement && document.activeElement.id");

describe("the closed box", () => {
  test(
    "Chromium's internal padding, arrow and line height, which appearance: none takes away",
    async () => {
      const m = await box("#model");
      const f = await box("#flat");
      // 4px at the start, 1px + a 15px arrow box at the end.
      expect(m.w - f.w).toBeCloseTo(20, 0);
      // The line height is `normal` on a menulist, plus 1px above and below;
      // `appearance: none` gives the author's 30px back.
      expect(Math.abs(m.h - (2 + 12 + 2 + 14 * 1.2))).toBeLessThan(1.5);
      expect(Math.abs(f.h - (2 + 12 + 30))).toBeLessThan(1.5);
      // The chevron, in the text colour, in the last 15px of the padding box.
      const shot = await shoot();
      const arrow = (b: Bounds) => {
        let n = 0;
        const mid = b.y + b.h / 2;
        for (let x = b.x + b.w - 14; x < b.x + b.w - 3; x++) {
          for (let y = mid - 3; y <= mid + 3; y++) if (dark(shot.at(x, y))) n++;
        }
        return n;
      };
      expect(arrow(m)).toBeGreaterThan(3);
      expect(arrow(f)).toBe(0);
      // The selected option's label: v-model.number's 1.
      expect(await js<number>("return document.getElementById('model').selectedIndex")).toBe(1);
      expect(await value("#model")).toBe("1");
    },
    T,
  );
});

describe("the popup", () => {
  test(
    "a click opens it under the box, and a click on an option picks it — over another select",
    async () => {
      const m = await box("#model");
      const below = probe(m, 2);
      await app.locator("#model").click({ real: true });
      expect(await popupAt(below.x, below.y, true)).toBe(true);
      expect(await active()).toBe("model");
      // The selected option is highlighted: not the popup's yellow.
      const r1 = row(m, 1);
      expect(yellow((await shoot()).at(r1.x, r1.y))).toBe(false);
      // The bottom of row 2 lies over #groups: the press is the popup's.
      const r2 = { x: m.x + 20, y: rowTop(m, 3) - 3 };
      expect(r2.y).toBeGreaterThan((await box("#groups")).y);
      await click(r2.x, r2.y);
      expect(await until(log, (l) => l.length >= 2)).toEqual(["input", "change:2"]);
      expect(await until(() => text("#out"), (t) => t === "model 2")).toBe("model 2");
      expect(await popupAt(below.x, below.y, false)).toBe(false);
      expect(await active()).toBe("model");
    },
    T,
  );

  test(
    "a press outside closes it and goes nowhere; the next one does",
    async () => {
      const m = await box("#model");
      // The model is 2 now: row 0 is not highlighted.
      const below = probe(m, 0);
      await app.locator("#model").click({ real: true });
      expect(await popupAt(below.x, below.y, true)).toBe(true);
      await app.locator("#behind").click({ real: true });
      expect(await popupAt(below.x, below.y, false)).toBe(false);
      await sleep(200);
      expect(await text("#behind")).toBe("behind 0");
      expect(await active()).toBe("model");
      await app.locator("#behind").click({ real: true });
      expect(await until(() => text("#behind"), (t) => t === "behind 1")).toBe("behind 1");
    },
    T,
  );

  test(
    "a cancelled mousedown opens nothing",
    async () => {
      const g = await box("#guarded");
      await app.locator("#guarded").click({ real: true });
      await sleep(300);
      expect(page((await shoot()).at(g.x + 10, g.y + g.h + 5))).toBe(true);
    },
    T,
  );

  test(
    "near the bottom of the window it opens above the box",
    async () => {
      const f = await box("#far");
      await app.locator("#far").click({ real: true });
      expect(await popupAt(f.x + 10, f.y - 5, true)).toBe(true);
      expect(page((await shoot()).at(f.x + 10, f.y + f.h + 5))).toBe(true);
      await app.press("Escape");
      expect(await popupAt(f.x + 10, f.y - 5, false)).toBe(false);
    },
    T,
  );

  test(
    "a list longer than the room is capped to it, scrolls by wheel, and keeps the highlight in view",
    async () => {
      const l = await box("#long");
      // Right of the labels, so the samples are the rows' own colour.
      const x = l.x + l.w - 10;
      // The popup fills the 360px above the box: row 0's centre while the
      // list is at its top, and the bottom row's, just above the box.
      const first = 1 + ROW / 2;
      const last = l.y - 1 - ROW / 2;
      await app.locator("#long").click({ real: true });
      expect(await popupAt(x, last, true)).toBe(true);
      expect(lit((await shoot()).at(x, first))).toBe(true);
      // End highlights the last option and scrolls it into view.
      await app.press("End");
      expect(await until(async () => lit((await shoot()).at(x, last)), (v) => v)).toBe(true);
      expect(yellow((await shoot()).at(x, first))).toBe(true);
      await app.press("Home");
      expect(await until(async () => lit((await shoot()).at(x, first)), (v) => v)).toBe(true);
      // The wheel scrolls the list, which stays open. The pointer it comes
      // with rests on row 0, the highlighted one, so the highlight stays.
      await app.send("wheel", { x, y: first, dy: 120 });
      expect(await until(async () => yellow((await shoot()).at(x, first)), (v) => v)).toBe(true);
      expect(yellow((await shoot()).at(x, last))).toBe(true);
      // Home lands on row 0 again, scrolled out above: it comes back into
      // view at the top.
      await app.press("Home");
      expect(await until(async () => lit((await shoot()).at(x, first)), (v) => v)).toBe(true);
      // The pointer, moved over a row, highlights it, as in Chromium; the
      // keys go on from there.
      await app.send("mouse", { x, y: first + ROW * 3, kind: "move" });
      expect(await until(async () => lit((await shoot()).at(x, first + ROW * 3)), (v) => v)).toBe(true);
      await app.press("ArrowDown");
      expect(await until(async () => lit((await shoot()).at(x, first + ROW * 4)), (v) => v)).toBe(true);
      await app.press("Escape");
      expect(await popupAt(x, last, false)).toBe(false);
      expect(await value("#long")).toBe("0");
      expect((await log()).filter((e) => e.startsWith("long"))).toEqual([]);
    },
    T,
  );
});

describe("the keyboard", () => {
  test(
    "closed, the arrows and typing change the value at once, with input and change",
    async () => {
      await app.locator("#model").focus();
      const before = (await log()).length;
      await app.press("ArrowUp");
      expect(await until(() => text("#out"), (t) => t === "model 1")).toBe("model 1");
      await app.press("End");
      expect(await until(() => text("#out"), (t) => t === "model 2")).toBe("model 2");
      // Type-ahead: "e" finds "Everybody".
      await app.press("e");
      expect(await until(() => text("#out"), (t) => t === "model 0")).toBe("model 0");
      expect((await log()).slice(before)).toEqual(["input", "change:1", "input", "change:2", "input", "change:0"]);
    },
    T,
  );

  test(
    "Alt+ArrowDown opens it; the arrows move the highlight, Enter picks, Escape does not",
    async () => {
      const m = await box("#model");
      // The model is 0 now, then 1: row 2 is never the highlighted one.
      const below = probe(m, 2);
      await app.locator("#model").focus();
      const before = (await log()).length;
      await app.press("Alt+ArrowDown");
      expect(await popupAt(below.x, below.y, true)).toBe(true);
      await app.press("ArrowDown");
      await sleep(150);
      // Only the highlight moved: no event, and the page heard no key.
      expect((await log()).length).toBe(before);
      expect(yellow((await shoot()).at(row(m, 1).x, row(m, 1).y))).toBe(false);
      await app.press("Enter");
      expect(await popupAt(below.x, below.y, false)).toBe(false);
      expect(await until(() => text("#out"), (t) => t === "model 1")).toBe("model 1");
      // Space opens, once no type-ahead search is going on (Space would be
      // part of the search).
      await sleep(1_100);
      await app.press("Space");
      expect(await popupAt(below.x, below.y, true)).toBe(true);
      await app.press("ArrowDown");
      await app.press("Escape");
      expect(await popupAt(below.x, below.y, false)).toBe(false);
      await sleep(150);
      expect(await text("#out")).toBe("model 1");
      expect((await log()).slice(before)).toEqual(["input", "change:1"]);
    },
    T,
  );

  test(
    "the highlight skips an optgroup's label and a disabled option",
    async () => {
      await app.locator("#groups").focus();
      await app.press("ArrowDown");
      expect(await until(() => value("#groups"), (v) => v === "c")).toBe("c");
      await app.press("ArrowUp");
      expect(await until(() => value("#groups"), (v) => v === "a")).toBe("a");
    },
    T,
  );

  test(
    "a cancelled keydown changes nothing",
    async () => {
      await app.locator("#guarded").focus();
      await app.press("ArrowDown");
      await sleep(200);
      expect(await value("#guarded")).toBe("0");
      await app.press("End");
      expect(await until(() => value("#guarded"), (v) => v === "1")).toBe("1");
    },
    T,
  );
});
