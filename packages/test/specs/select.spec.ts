// The popup is no element, so it is found in screenshots: it takes the select's own yellow
// background, on a dark gray page. It is a window of its own, hanging under the box; drawn in the
// window only when that window cannot be opened.

import { afterAll, describe, expect, test } from "bun:test";
import { artifactPath, type Bounds, launch, LINUX, MACOS, readPng, SCREENSHOTS, WINDOWS } from "../src/index";

const W = 460;
const app = await launch({
  app: "packages/test/fixtures/select/src/main.ts",
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

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / W;
  return { at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)) };
};
const yellow = ([r, g, b]: Rgb) => r > 220 && g > 220 && b < 60;
const page = ([r, g, b]: Rgb) => Math.abs(r - 40) < 12 && Math.abs(g - 40) < 12 && Math.abs(b - 40) < 12;
const dark = ([r, g, b]: Rgb) => r + g + b < 300;
const lit = ([r, g, b]: Rgb) => b > 200 && r < 170 && g > 120;

type Win = { id: number; kind: string; screen: Bounds };
/** Popups are windows of their own. */
const NATIVE = WINDOWS || MACOS || LINUX;
const windows = async () => (await app.send("windows")).windows as Win[];
const popupWindow = async () => (await windows()).find((w) => w.kind === "popup");

/**
 * The list under the box `b`, read at window coordinates: from its own window on Windows, where it
 * hangs under the box (what is not on it reads as the page), else from the window's screenshot.
 */
const shootList = async (b: Bounds) => {
  if (!NATIVE) return shoot();
  const popup = await popupWindow();
  const bare = { at: (): Rgb => [40, 40, 40] };
  if (!popup) return bare;
  const sent = await app.send("screenshot", { path: artifactPath("select-popup", "png"), window: popup.id });
  const shot = readPng(sent.path as string);
  const k = Number(sent.scale) || 1;
  const top = b.y + b.h;
  return {
    at: (x: number, y: number): Rgb => {
      const [px, py] = [Math.floor((x - b.x + 0.5) * k), Math.floor((y - top + 0.5) * k)];
      return px < 0 || py < 0 || px >= shot.width || py >= shot.height ? bare.at() : shot.at(px, py);
    },
  };
};
/** A gesture at window coordinates on the list under `b`. */
const onList = async (b: Bounds, args: Record<string, unknown> & { x: number; y: number }) => {
  const popup = NATIVE ? await popupWindow() : undefined;
  if (!popup) return app.send(args.dy === undefined ? "mouse" : "wheel", args);
  return app.send(args.dy === undefined ? "mouse" : "wheel", {
    ...args,
    x: args.x - b.x,
    y: args.y - b.y - b.h,
    window: popup.id,
  });
};

const popupAt = (b: Bounds, x: number, y: number, open: boolean) =>
  until(
    async () => yellow((await shootList(b)).at(x, y)),
    (v) => v === open,
  );

const ROW = 14 * 1.2 + 2;
const rowTop = (b: Bounds, i: number) => b.y + b.h + 1 + ROW * i;
const row = (b: Bounds, i: number) => ({ x: b.x + 20, y: rowTop(b, i) + ROW / 2 });
/**
 * Where a popup under `b` is looked for: the top of row `i`, which must not be the highlighted one
 * (that is not yellow), over the bare page.
 */
const probe = (b: Bounds, i: number) => ({ x: b.x + 20, y: rowTop(b, i) + 4 });

const log = () => js<string[]>("return globalThis.log.slice()");
const text = (sel: string) => js<string>(`return document.querySelector(${JSON.stringify(sel)}).textContent`);
const value = (sel: string) => js<string>(`return document.querySelector(${JSON.stringify(sel)}).value`);
const active = () => js<string>("return document.activeElement && document.activeElement.id");

describe("the closed box", () => {
  test.skipIf(!SCREENSHOTS)(
    "centres the label of a fixed-height select beside its arrow",
    async () => {
      const b = await box("#tinted");
      const shot = await shoot();
      const inkMidpoint = (start: number, end: number) => {
        const rows: number[] = [];
        for (let y = 2; y < b.h - 2; y++) {
          for (let x = start; x < end; x++) {
            if (Math.min(...shot.at(b.x + x, b.y + y)) > 160) {
              rows.push(y);
              break;
            }
          }
        }
        expect(rows.length).toBeGreaterThan(1);
        return (rows[0]! + rows.at(-1)!) / 2;
      };
      expect(Math.abs(inkMidpoint(12, 80) - inkMidpoint(b.w - 15, b.w - 2))).toBeLessThanOrEqual(3);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "Chromium's internal padding, arrow and line height, which appearance: none takes away",
    async () => {
      const m = await box("#model");
      const f = await box("#flat");
      expect(m.w - f.w).toBeCloseTo(20, 0);
      expect(Math.abs(m.h - (2 + 12 + 2 + 14 * 1.2))).toBeLessThan(1.5);
      expect(Math.abs(f.h - (2 + 12 + 30))).toBeLessThan(1.5);
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
      expect(await js<number>("return document.getElementById('model').selectedIndex")).toBe(1);
      expect(await value("#model")).toBe("1");
    },
    T,
  );
});

describe("the popup", () => {
  test.skipIf(!SCREENSHOTS)(
    "paints a translucent control colour over an opaque menu surface",
    async () => {
      const control = app.locator("#tinted");
      for (const [scheme, expected] of [
        ["dark", 44],
        ["light", 191],
      ] as const) {
        await app.eval(`document.querySelector("#tinted").style.setProperty("color-scheme", "${scheme}")`);
        await control.click({ real: true });
        try {
          const popup = await until(popupWindow, (p) => p !== undefined);
          if (!popup) throw new Error("missing select popup");
          const { path } = await app.send("screenshot", {
            window: popup.id,
            backend: "render_to_image",
            path: artifactPath("select-tinted-" + scheme, "png"),
          });
          const shot = readPng(path as string);
          const x = shot.width - 12,
            y = Math.floor(shot.height / 2);
          expect(shot.data[(y * shot.width + x) * 4 + 3]).toBe(255);
          for (const channel of shot.at(x, y)) expect(Math.abs(channel - expected)).toBeLessThanOrEqual(2);
        } finally {
          await app.press("Escape");
          await until(popupWindow, (p) => p === undefined);
        }
      }
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "a click opens it under the box, and a click on an option picks it — over another select",
    async () => {
      const m = await box("#model");
      const below = probe(m, 2);
      await app.locator("#model").click({ real: true });
      expect(await popupAt(m, below.x, below.y, true)).toBe(true);
      expect(await active()).toBe("model");
      const r1 = row(m, 1);
      expect(yellow((await shootList(m)).at(r1.x, r1.y))).toBe(false);
      const r2 = { x: m.x + 20, y: rowTop(m, 3) - 3 };
      expect(r2.y).toBeGreaterThan((await box("#groups")).y);
      await onList(m, { x: r2.x, y: r2.y, kind: "click" });
      expect(await until(log, (l) => l.length >= 2)).toEqual(["input", "change:2"]);
      expect(
        await until(
          () => text("#out"),
          (t) => t === "model 2",
        ),
      ).toBe("model 2");
      expect(await popupAt(m, below.x, below.y, false)).toBe(false);
      expect(await active()).toBe("model");
    },
    T,
  );

  // macOS CI: hover, focus and popups do not follow the harness's synthetic input.
  test.skipIf(!SCREENSHOTS || MACOS)(
    "a press outside closes it and goes nowhere; the next one does",
    async () => {
      const m = await box("#model");
      const below = probe(m, 0);
      await app.locator("#model").click({ real: true });
      expect(await popupAt(m, below.x, below.y, true)).toBe(true);
      await app.locator("#behind").click({ real: true });
      expect(await popupAt(m, below.x, below.y, false)).toBe(false);
      await sleep(200);
      expect(await text("#behind")).toBe("behind 0");
      expect(await active()).toBe("model");
      await app.locator("#behind").click({ real: true });
      expect(
        await until(
          () => text("#behind"),
          (t) => t === "behind 1",
        ),
      ).toBe("behind 1");
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "a cancelled mousedown opens nothing",
    async () => {
      const g = await box("#guarded");
      await app.locator("#guarded").click({ real: true });
      await sleep(300);
      expect(page((await shoot()).at(g.x + 10, g.y + g.h + 5))).toBe(true);
      if (NATIVE) expect(await popupWindow()).toBeUndefined();
    },
    T,
  );

  test.skipIf(!SCREENSHOTS || MACOS || NATIVE)(
    "near the bottom of the window it opens above the box",
    async () => {
      const f = await box("#far");
      await app.locator("#far").click({ real: true });
      expect(await popupAt(f, f.x + 10, f.y - 5, true)).toBe(true);
      expect(page((await shoot()).at(f.x + 10, f.y + f.h + 5))).toBe(true);
      await app.press("Escape");
      expect(await popupAt(f, f.x + 10, f.y - 5, false)).toBe(false);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS || !NATIVE)(
    "near the bottom of the window it opens under the box all the same, past the window's edge",
    async () => {
      const f = await box("#far");
      await app.locator("#far").click({ real: true });
      const below = probe(f, 4);
      expect(await popupAt(f, below.x, below.y, true)).toBe(true);
      const all = await windows();
      const main = all.find((w) => w.kind === "main");
      const popup = all.find((w) => w.kind === "popup");
      if (!main || !popup) throw new Error("no popup window");
      expect(popup.screen.y + popup.screen.h).toBeGreaterThan(main.screen.y + main.screen.h);
      expect(page((await shoot()).at(f.x + 10, f.y - 5))).toBe(true);
      expect(page((await shoot()).at(f.x + 10, f.y + f.h + 5))).toBe(true);
      await app.press("ArrowDown");
      await app.press("Enter");
      expect(await popupAt(f, below.x, below.y, false)).toBe(false);
      expect(await value("#far")).toBe("1");
    },
    T,
  );

  test.skipIf(!SCREENSHOTS || MACOS)(
    "a list longer than the room is capped to it, scrolls by wheel, and keeps the highlight in view",
    async () => {
      const l = await box("#long");
      // Right of the labels, so the samples are the rows' own colour.
      const x = l.x + l.w - 10;
      // In the window it opens above the box, capped to the room there; in a window of its own,
      // under the box with its 20 rows.
      const first = NATIVE ? rowTop(l, 0) + ROW / 2 : 1 + ROW / 2;
      const last = NATIVE ? rowTop(l, 19) + ROW / 2 : l.y - 1 - ROW / 2;
      const at = async (y: number) => (await shootList(l)).at(x, y);
      await app.locator("#long").click({ real: true });
      expect(await popupAt(l, x, last, true)).toBe(true);
      expect(lit(await at(first))).toBe(true);
      await app.press("End");
      expect(
        await until(
          async () => lit(await at(last)),
          (v) => v,
        ),
      ).toBe(true);
      expect(yellow(await at(first))).toBe(true);
      await app.press("Home");
      expect(
        await until(
          async () => lit(await at(first)),
          (v) => v,
        ),
      ).toBe(true);
      // The pointer it comes with rests on row 0, the highlighted one, so the highlight stays.
      await onList(l, { x, y: first, dy: 120 });
      expect(
        await until(
          async () => yellow(await at(first)),
          (v) => v,
        ),
      ).toBe(true);
      expect(yellow(await at(last))).toBe(true);
      await app.press("Home");
      expect(
        await until(
          async () => lit(await at(first)),
          (v) => v,
        ),
      ).toBe(true);
      await onList(l, { x, y: first + ROW * 3, kind: "move" });
      expect(
        await until(
          async () => lit(await at(first + ROW * 3)),
          (v) => v,
        ),
      ).toBe(true);
      await app.press("ArrowDown");
      expect(
        await until(
          async () => lit(await at(first + ROW * 4)),
          (v) => v,
        ),
      ).toBe(true);
      await app.press("Escape");
      expect(await popupAt(l, x, last, false)).toBe(false);
      expect(await value("#long")).toBe("0");
      expect((await log()).filter((e) => e.startsWith("long"))).toEqual([]);
    },
    T,
  );
});

describe("the keyboard", () => {
  // macOS and Linux: typing picks a different option.
  test.skipIf(!WINDOWS)(
    "closed, the arrows and typing change the value at once, with input and change",
    async () => {
      await app.locator("#model").focus();
      const before = (await log()).length;
      await app.press("ArrowUp");
      expect(
        await until(
          () => text("#out"),
          (t) => t === "model 1",
        ),
      ).toBe("model 1");
      await app.press("End");
      expect(
        await until(
          () => text("#out"),
          (t) => t === "model 2",
        ),
      ).toBe("model 2");
      await app.press("e");
      expect(
        await until(
          () => text("#out"),
          (t) => t === "model 0",
        ),
      ).toBe("model 0");
      expect((await log()).slice(before)).toEqual(["input", "change:1", "input", "change:2", "input", "change:0"]);
    },
    T,
  );

  // macOS: Alt+ArrowDown does not open the popup.
  test.skipIf(!SCREENSHOTS || MACOS)(
    "Alt+ArrowDown opens it; the arrows move the highlight, Enter picks, Escape does not",
    async () => {
      const m = await box("#model");
      // The model is 0 now, then 1: row 2 is never the highlighted one.
      const below = probe(m, 2);
      await app.locator("#model").focus();
      const before = (await log()).length;
      await app.press("Alt+ArrowDown");
      expect(await popupAt(m, below.x, below.y, true)).toBe(true);
      await app.press("ArrowDown");
      await sleep(150);
      expect((await log()).length).toBe(before);
      expect(yellow((await shootList(m)).at(row(m, 1).x, row(m, 1).y))).toBe(false);
      await app.press("Enter");
      expect(await popupAt(m, below.x, below.y, false)).toBe(false);
      expect(
        await until(
          () => text("#out"),
          (t) => t === "model 1",
        ),
      ).toBe("model 1");
      // Space opens, once no type-ahead search is going on (Space would be part of the search).
      await sleep(1_100);
      await app.press("Space");
      expect(await popupAt(m, below.x, below.y, true)).toBe(true);
      await app.press("ArrowDown");
      await app.press("Escape");
      expect(await popupAt(m, below.x, below.y, false)).toBe(false);
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
      expect(
        await until(
          () => value("#groups"),
          (v) => v === "c",
        ),
      ).toBe("c");
      await app.press("ArrowUp");
      expect(
        await until(
          () => value("#groups"),
          (v) => v === "a",
        ),
      ).toBe("a");
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
      expect(
        await until(
          () => value("#guarded"),
          (v) => v === "1",
        ),
      ).toBe("1");
    },
    T,
  );
});
