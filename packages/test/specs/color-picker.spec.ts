// <input type=color>'s picker. It is no element, so it is found in screenshots: its light surface, 4 px
// under the input. On Windows it is a popup window of its own, listed by `windows` and driven through
// its id; elsewhere it is drawn in the window, over a dark blue page.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { artifactPath, type Bounds, launch, MACOS, readPng, SCREENSHOTS, type SkApp, WINDOWS } from "../src/index";

const W = 460;
const app = await launch({
  app: "packages/test/fixtures/color-picker/src/main.ts",
  window: [W, 720],
  label: "color-picker",
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

type Win = { id: number; kind: string; focused: boolean; owner?: number; margin?: number; screen: Bounds };
/** Popups are windows of their own on Windows. */
const NATIVE = WINDOWS;
/** The transparent room around a popup window's panel, for its shadow. */
const MARGIN = 24;
const windows = async (of: SkApp = app) => (await of.send("windows")).windows as Win[];
const popupOf = async (of: SkApp = app) => (await windows(of)).find((w) => w.kind === "popup");

/** Coordinates are the picker's: its popup window's on Windows, else the main window's. */
const shoot = async (path?: string) => {
  const popup = NATIVE ? await popupOf() : undefined;
  if (NATIVE && !popup) return { at: (): Rgb => [0, 0, 0] };
  const file = popup
    ? (await app.send("screenshot", { path: path ?? artifactPath("color-picker-popup", "png"), window: popup.id })).path
    : await app.screenshot(path);
  const shot = readPng(file as string);
  const k = popup ? 1 : shot.width / W;
  return { at: (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k)) };
};
const near = (c: Rgb, [r, g, b]: Rgb) => Math.abs(c[0] - r) < 6 && Math.abs(c[1] - g) < 6 && Math.abs(c[2] - b) < 6;

/** The panel's top-left: under the input with a 4 px gap, kept inside the window. Its content starts 13 px in (border, padding). */
const POPOVER_W = 242;
const origin = (b: Bounds) =>
  NATIVE ? { x: MARGIN, y: MARGIN } : { x: Math.min(b.x, W - POPOVER_W), y: b.y + b.h + 4 };
/** The content's top-left; the shape tabs (28 px and a 12 px gap) come first. */
const content = (b: Bounds) => ({ x: origin(b).x + 13, y: origin(b).y + 13 });
const SQUARE = { w: 216, h: 144 };
const square = (b: Bounds) => ({ x: content(b).x, y: content(b).y + 40 });
const shown = async (b: Bounds, surface: Rgb = [249, 249, 249]) => {
  const o = origin(b);
  return near((await shoot()).at(o.x + 5, o.y + 40), surface);
};
const opened = (b: Bounds, open: boolean) =>
  until(
    () => shown(b),
    (v) => v === open,
  );

/** A gesture in the picker. */
const mouse = async (x: number, y: number, kind: "down" | "move" | "up") => {
  const popup = NATIVE ? await popupOf() : undefined;
  return app.send("mouse", { x, y, kind, ...(popup ? { window: popup.id } : {}) });
};
/** A gesture in the app's window. */
const pageMouse = (x: number, y: number, kind: "down" | "move" | "up") => app.send("mouse", { x, y, kind });
const log = () => js<string[]>("return globalThis.log.slice()");
const value = (sel: string) => js<string>(`return document.querySelector(${JSON.stringify(sel)}).value`);
const since = async (n: number) => (await log()).slice(n);

const shots = process.env.SK_PICKER_SHOTS;
const snap = async (name: string) => {
  await sleep(150);
  if (shots) await shoot(join(shots, `picker-v3-${name}.png`));
};
const openOn = async (selector: string) => {
  const b = await box(selector);
  await app.locator(selector).click({ real: true });
  expect(await opened(b, true)).toBe(true);
  return b;
};
const escape = async (b: Bounds) => {
  await app.press("Escape");
  expect(await opened(b, false)).toBe(false);
};
const press = async (keys: string[]) => {
  for (const key of keys) await app.press(key);
};

describe.skipIf(!SCREENSHOTS)("the picker", () => {
  // The shape and the value format are remembered app-wide: start from the square and hex.
  beforeAll(async () => {
    const b = await openOn("#plain");
    await press(["Shift+Tab", "Home", "Tab", "Tab", "Tab", "Home"]);
    await escape(b);
  }, T);

  test(
    "a click fires showpicker and opens it under the input; dragging in the square fires input, Escape restores",
    async () => {
      const b = await box("#plain");
      const before = (await log()).length;
      await app.locator("#plain").click({ real: true });
      expect(await opened(b, true)).toBe(true);
      expect(await since(before)).toEqual(["showpicker:plain:#ff0000"]);
      const sq = square(b);
      await mouse(sq.x + SQUARE.w / 2, sq.y + SQUARE.h / 2, "down");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0000",
        ),
      ).toBe("#804040");
      // Past the square's top-left corner: no saturation, full value.
      await mouse(sq.x - 40, sq.y - 40, "move");
      await mouse(sq.x - 40, sq.y - 40, "up");
      expect(
        await until(
          () => value("#plain"),
          (v) => v === "#ffffff",
        ),
      ).toBe("#ffffff");
      expect(await shown(b)).toBe(true);
      expect(await since(before + 1)).toEqual(["input:plain:#804040", "input:plain:#ffffff"]);
      await app.press("Escape");
      expect(await opened(b, false)).toBe(false);
      expect(await value("#plain")).toBe("#ff0000");
      expect(await since(before + 3)).toEqual(["input:plain:#ff0000"]);
    },
    T,
  );

  test(
    "the arrows move in the square, Tab moves to the hue slider, Enter commits with change",
    async () => {
      const b = await box("#plain");
      await app.locator("#plain").click({ real: true });
      expect(await opened(b, true)).toBe(true);
      const before = (await log()).length;
      await app.press("ArrowLeft");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0000",
        ),
      ).toBe("#ff0303");
      await app.press("Tab");
      await app.press("Shift+ArrowRight");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0303",
        ),
      ).toBe("#ff2d03");
      await app.press("Enter");
      expect(await opened(b, false)).toBe(false);
      expect(await since(before)).toEqual(["input:plain:#ff0303", "input:plain:#ff2d03", "change:plain:#ff2d03"]);
    },
    T,
  );

  // macOS CI: synthetic presses outside do not reach the window's hit test.
  test.skipIf(MACOS)(
    "showPicker() from a button opens it; a press outside commits and goes nowhere",
    async () => {
      await js("document.getElementById('plain').value = '#0000ff'; return 0");
      const b = await box("#plain");
      await app.locator("#pick").click({ real: true });
      expect(await opened(b, true)).toBe(true);
      const before = (await log()).length;
      const sq = square(b);
      await mouse(sq.x + SQUARE.w - 1, sq.y + 1, "down");
      await mouse(sq.x + SQUARE.w - 1, sq.y + 1, "up");
      await pageMouse(W - 10, 500, "down");
      await pageMouse(W - 10, 500, "up");
      expect(await opened(b, false)).toBe(false);
      const after = await since(before);
      expect(after.at(-1)).toMatch(/^change:plain:#/);
      expect(after.filter((e) => e.startsWith("change"))).toHaveLength(1);
    },
    T,
  );

  test(
    "a cancelled showpicker keeps it closed",
    async () => {
      const b = await box("#custom");
      const before = (await log()).length;
      await app.locator("#custom").click({ real: true });
      expect(await until(log, (l) => l.length > before)).toContain("showpicker:custom:#00ff00");
      await sleep(300);
      expect(await shown(b)).toBe(false);
    },
    T,
  );

  test(
    "with alpha it shows the alpha slider, and screenshots in light and dark",
    async () => {
      const b = await box("#alpha");
      expect(await value("#alpha")).toBe("color(srgb 0.2 0.4 0.8 / 0.501961)");
      const dir = process.env.SK_PICKER_SHOTS;
      for (const dark of [false, true]) {
        await js(`globalThis.setDark(${dark}); return 0`);
        await sleep(150);
        await app.locator("#alpha").click({ real: true });
        const surface: Rgb = dark ? [44, 44, 44] : [249, 249, 249];
        expect(
          await until(
            () => shown(b, surface),
            (v) => v,
          ),
        ).toBe(true);
        await app.press("Tab");
        await app.press("Tab");
        await sleep(150);
        if (dir) await shoot(join(dir, `picker-v3-${dark ? "dark" : "light"}.png`));
        await app.press("Escape");
        expect(
          await until(
            () => shown(b, surface),
            (v) => !v,
          ),
        ).toBe(false);
      }
      await js("globalThis.setDark(false); return 0");
    },
    T,
  );

  test(
    "the shape tabs switch to a hue wheel and a triangle, and the choice is remembered",
    async () => {
      await js("document.getElementById('plain').value = '#ff0000'; return 0");
      const b = await openOn("#plain");
      await snap("square");
      const c = content(b);
      await mouse(c.x + 108, c.y + 14, "down");
      await mouse(c.x + 108, c.y + 14, "up");
      // A 196 px disc under the tabs; the ring's middle is 89 px out, hue 0 at the top, clockwise.
      const disc = { x: c.x + 108, y: c.y + 40 + 98 };
      const cyanBelow = async () => near((await shoot()).at(disc.x, disc.y + 89), [0, 255, 255]);
      expect(await until(cyanBelow, (v) => v)).toBe(true);
      await snap("wheel");
      await mouse(disc.x + 89, disc.y, "down");
      await mouse(disc.x + 89, disc.y, "up");
      expect(
        await until(
          () => value("#plain"),
          (v) => v !== "#ff0000",
        ),
      ).toBe("#80ff00");
      await escape(b);
      expect(await value("#plain")).toBe("#ff0000");

      await openOn("#plain");
      expect(await until(cyanBelow, (v) => v)).toBe(true);
      await press(["Shift+Tab", "ArrowRight"]);
      // The triangle's middle mixes its red, white and black corners equally.
      expect(
        await until(
          async () => near((await shoot()).at(disc.x, disc.y), [170, 85, 85]),
          (v) => v,
        ),
      ).toBe(true);
      await snap("triangle");
      // Past the white corner, inside the ring.
      const white = { x: disc.x + 78 * Math.cos(Math.PI / 6), y: disc.y + 78 * Math.sin(Math.PI / 6) };
      await mouse(white.x, white.y, "down");
      await mouse(white.x, white.y, "up");
      expect(
        await until(
          () => value("#plain"),
          (v) => v === "#ffffff",
        ),
      ).toBe("#ffffff");
      await press(["Shift+Tab", "Home"]);
      await escape(b);
    },
    T,
  );

  test(
    "colorspace=display-p3 converts any color to P3 and the picker works in P3 coordinates",
    async () => {
      expect(await value("#p3")).toBe("color(display-p3 0.917488 0.200287 0.138561)");
      const b = await openOn("#p3");
      await snap("p3");
      const sq = square(b);
      await mouse(sq.x + SQUARE.w - 2, sq.y + 2, "down");
      await mouse(sq.x + SQUARE.w + 20, sq.y - 20, "move");
      await mouse(sq.x + SQUARE.w + 20, sq.y - 20, "up");
      expect(
        await until(
          () => value("#p3"),
          (v) => !v.endsWith("0.138561)"),
        ),
      ).toMatch(/^color\(display-p3 1 0\.0\d+ 0\)$/);
      await escape(b);
      expect(await value("#p3")).toBe("color(display-p3 0.917488 0.200287 0.138561)");
    },
    T,
  );

  test(
    "hdr adds an intensity in stops; the value stays linear light past 1",
    async () => {
      expect(await value("#hdr")).toBe("color(srgb-linear 4 2 0.5)");
      const b = await openOn("#hdr");
      // Under the tabs, the square, the hue and alpha sliders: base on the left, output on the right.
      const c = content(b);
      const preview = c.y + 40 + 144 + 12 + 18 + 12 + 18 + 12 + 12;
      const shot = await shoot();
      expect(shot.at(c.x + 40, preview)).toSatisfy((px) => near(px, [255, 188, 99]));
      // Output: 4 2 0.5 is past white, so it glows white under zebra stripes.
      expect(Math.min(...shot.at(c.x + 162, preview))).toBeGreaterThan(190);
      const before = (await log()).length;
      await press(["Tab", "Tab", "Tab", "End"]);
      expect(
        await until(
          () => value("#hdr"),
          (v) => v.startsWith("color(srgb-linear 1024"),
        ),
      ).toBe("color(srgb-linear 1024 512 128)");
      await app.press("ArrowLeft");
      expect(
        await until(
          () => value("#hdr"),
          (v) => v.startsWith("color(srgb-linear 512"),
        ),
      ).toBe("color(srgb-linear 512 256 64)");
      await snap("hdr");
      await app.press("Enter");
      expect(await opened(b, false)).toBe(false);
      expect(
        await until(
          async () => (await since(before)).at(-1),
          (e) => e?.startsWith("change") === true,
        ),
      ).toBe("change:hdr:color(srgb-linear 512 256 64)");

      // The page re-renders with its static value, which puts the input back at +2.
      await js("globalThis.setDark(true); return 0");
      await sleep(150);
      await app.locator("#hdr").click({ real: true });
      expect(
        await until(
          () => shown(b, [44, 44, 44]),
          (v) => v,
        ),
      ).toBe(true);
      await press(["Tab", "Tab", "Tab", "End", "ArrowLeft"]);
      await snap("hdr-dark");
      await app.press("Escape");
      expect(
        await until(
          () => shown(b, [44, 44, 44]),
          (v) => !v,
        ),
      ).toBe(false);
      await js("globalThis.setDark(false); return 0");
      expect(
        await until(
          () => value("#hdr"),
          (v) => v.includes(" 4 2 "),
        ),
      ).toBe("color(srgb-linear 4 2 0.5)");
    },
    T,
  );

  test(
    "the format switch gives RGB, HSL, HSV and OKLCH fields, and typing in one sets the value",
    async () => {
      await js("document.getElementById('plain').value = '#ff0000'; return 0");
      const b = await openOn("#plain");
      await press(["Tab", "Tab"]);
      await snap("format-hex");
      for (const name of ["rgb", "hsl", "hsv", "oklch"]) {
        await app.press("ArrowRight");
        await snap(`format-${name}`);
      }
      await press(["Home", "ArrowRight", "Tab", "1", "2", "8"]);
      expect(
        await until(
          () => value("#plain"),
          (v) => v === "#800000",
        ),
      ).toBe("#800000");
      await press(["Shift+Tab", "Home"]);
      await escape(b);
      expect(await value("#plain")).toBe("#ff0000");
    },
    T,
  );
});

describe.skipIf(!NATIVE)("the popup window", () => {
  test("extends past a small window, takes presses there, and closes when the window is resized", async () => {
    const small = await launch({
      app: "packages/test/fixtures/color-picker/src/main.ts",
      window: [W, 240],
      label: "color-picker-small",
    });
    try {
      await small.locator("#plain").click({ real: true });
      const popup = await until(
        () => popupOf(small),
        (p) => p !== undefined,
      );
      if (!popup) throw new Error("no popup window");
      const main = (await windows(small)).find((w) => w.kind === "main");
      if (!main) throw new Error("no main window");
      expect(popup.owner).toBe(main.id);
      const panel = {
        top: popup.screen.y + MARGIN,
        bottom: popup.screen.y + popup.screen.h - MARGIN,
      };
      const below = panel.top > main.screen.y;
      expect(below ? panel.bottom > main.screen.y + main.screen.h : panel.top < main.screen.y).toBe(true);
      expect(popup.screen.w - 2 * MARGIN).toBe(POPOVER_W);
      expect((await windows(small)).find((w) => w.kind === "main")?.focused).toBe(true);

      // The square's bottom-left corner, past the window's bottom edge when the panel hangs below.
      const sq = { x: MARGIN + 13, y: MARGIN + 13 + 40 };
      const at = { x: sq.x + 2, y: sq.y + SQUARE.h - 2 };
      if (below) expect(popup.screen.y + at.y).toBeGreaterThan(main.screen.y + main.screen.h);
      await small.send("mouse", { window: popup.id, x: at.x, y: at.y, kind: "down" });
      await small.send("mouse", { window: popup.id, x: at.x, y: at.y, kind: "up" });
      const read = () => small.eval(`document.getElementById("plain").value`).then(String);
      expect(await until(read, (v) => v !== "#ff0000")).toMatch(/^#0[0-9a-f]0[0-9a-f]0[0-9a-f]$/);

      await small.eval(`resizeTo(${W + 20}, 300); "ok"`);
      expect(
        await until(
          () => popupOf(small),
          (p) => p === undefined,
        ),
      ).toBeUndefined();
      const events = JSON.parse(String(await small.eval("JSON.stringify(globalThis.log)"))) as string[];
      expect(events.filter((e) => e.startsWith("change:plain"))).toHaveLength(1);
    } finally {
      await small.close();
    }
  }, 60_000);
});
