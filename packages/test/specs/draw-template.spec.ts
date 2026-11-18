// `sk create --template draw`: strokes drawn with a pen, erased on one layer, undone and redone,
// restored from IndexedDB after a restart, a view that pans and zooms without touching them, and
// the canvas tools: shapes, lasso selection, the eyedropper, symmetry, layer merging and SVG export.

import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { create } from "../../cli/cli/create";
import { appDataDir, launch, readPng, REPO, SCREENSHOTS, WORK_DIR } from "../src/index";

mkdirSync(WORK_DIR, { recursive: true });
const dir = mkdtempSync(join(WORK_DIR, "draw-template-"));
const profile = `sk-test-${basename(dir)}`;
create(dir, { name: "Drawing", template: "draw" });
const entry = relative(REPO, join(dir, "src/main.ts")).replaceAll("\\", "/");
const host = () =>
  launch({
    app: entry,
    label: "draw-template",
    window: [1000, 700],
    env: { SK_APP_NAME: profile },
  });
let app = await host();
afterAll(async () => {
  await app.close();
  rmSync(dir, { recursive: true, force: true });
  // Windows releases the IndexedDB WAL and its directory lock after the child exits.
  const deadline = Date.now() + 5_000;
  for (;;) {
    try {
      rmSync(appDataDir(profile), { recursive: true, force: true });
      break;
    } catch (error) {
      if (Date.now() >= deadline) throw error;
      await Bun.sleep(100);
    }
  }
});

async function until<T>(read: () => Promise<T>, matches: (value: T) => boolean): Promise<T> {
  const deadline = Date.now() + 10_000;
  for (;;) {
    const value = await read();
    if (matches(value)) return value;
    if (Date.now() >= deadline)
      throw new Error(`Unexpected draw state: ${JSON.stringify(value)}\n${await app.tailLogs()}`);
    await Bun.sleep(50);
  }
}

const attr = async (name: string, selector = "#draw") =>
  String(await app.eval(`document.querySelector(${JSON.stringify(selector)})?.getAttribute("data-${name}")`));
const attrIs = (name: string, value: string) =>
  until(
    () => attr(name),
    (v) => v === value,
  );
const strokes = async () => Number(await attr("strokes"));
const click = (selector: string) => app.locator(selector).click();
const press = async (key: string) => {
  await app.press(key);
  await Bun.sleep(60);
};

async function line(x0: number, y0: number, x1: number, y1: number, readings: Record<string, number | boolean> = {}) {
  await app.send("pen", {
    kind: "down",
    x: x0,
    y: y0,
    pressure: 0.8,
    ...readings,
  });
  await app.send("pen", {
    kind: "move",
    x: x1,
    y: y1,
    fromX: x0,
    fromY: y0,
    pressure: 0.8,
    steps: 24,
    ...readings,
  });
  // Coalesced moves are delivered on the host's next frame.
  await Bun.sleep(80);
  await app.send("pen", { kind: "up", x: x1, y: y1, ...readings });
}

/** A mouse drag through `points`, the button held between them. */
async function drag(points: [number, number][]) {
  const [[x0, y0]] = points as [[number, number]];
  await app.send("mouse", { kind: "move", x: x0, y: y0 });
  await app.send("mouse", { kind: "down", x: x0, y: y0 });
  for (const [x, y] of points.slice(1)) {
    await app.send("mouse", { kind: "move", x, y, held: true });
    await Bun.sleep(16);
  }
  const [x1, y1] = points.at(-1)!;
  await app.send("mouse", { kind: "up", x: x1, y: y1 });
  await Bun.sleep(80);
}

let shot = 0;
const inkIs = (points: [number, number][], expected: boolean[]) =>
  until(
    () => inked(points),
    (seen) => seen.every((v, i) => v === expected[i]),
  );

/** Whether the window shows ink, rather than paper, at each point. */
async function inked(points: [number, number][]): Promise<boolean[]> {
  const png = readPng(await app.screenshot(join(dir, `shot-${shot++}.png`)));
  const scale = png.width / Number(await app.eval("innerWidth"));
  const paper = png.at(Math.round(150 * scale), Math.round(100 * scale));
  return points.map(([x, y]) => {
    const p = png.at(Math.round(x * scale), Math.round(y * scale));
    return p.some((c, i) => Math.abs(c - paper[i]!) > 80);
  });
}

async function storedStrokes(): Promise<string> {
  await app.eval(`(() => {
    globalThis.__drawSaved = "";
    const open = indexedDB.open("skathi-draw-ink");
    open.onsuccess = () => {
      const db = open.result;
      const request = db.transaction("strokes").objectStore("strokes").getAll();
      request.onsuccess = () => {
        globalThis.__drawSaved = JSON.stringify(request.result.map((s) => [s.layer, s.seq, s.tool, s.points.length]));
        db.close();
      };
    };
  })()`);
  return until(
    async () => String(await app.eval("globalThis.__drawSaved")),
    (s) => s.length > 0,
  );
}

describe.if(SCREENSHOTS)("the draw template", () => {
  test("the eraser end erases only the active layer, then the pen returns; undo and redo replay it", async () => {
    await attrIs("ready", "true");
    await app.eval('__SK_WINDOW__.appWindow.theme = "light"');
    await line(300, 300, 600, 300);
    await attrIs("strokes", "1");
    await click("#layers");
    await click("#add-layer");
    await click("#layers");
    await attrIs("layers", "2");
    await line(300, 380, 600, 380);
    await attrIs("strokes", "2");
    await inkIs(
      [
        [450, 300],
        [450, 380],
      ],
      [true, true],
    );

    await line(450, 250, 450, 430, { eraser: true });
    await attrIs("strokes", "3");
    expect(await attr("last-tool")).toBe("eraser");
    expect(await attr("effective-tool")).toBe("pen");
    await inkIs(
      [
        [450, 300],
        [450, 380],
        [350, 380],
      ],
      [true, false, true],
    );

    await click("#undo");
    await attrIs("strokes", "2");
    await inkIs([[450, 380]], [true]);
    await click("#redo");
    await attrIs("strokes", "3");
    await inkIs(
      [
        [450, 300],
        [450, 380],
      ],
      [true, false],
    );
    await app.expectNoErrors();
  }, 60_000);

  test("the drawing comes back from IndexedDB after a restart", async () => {
    const saved = await until(storedStrokes, (s) => JSON.parse(s).length === 3);
    await app.close();
    app = await host();
    await attrIs("ready", "true");
    await app.eval('__SK_WINDOW__.appWindow.theme = "light"');
    await attrIs("strokes", "3");
    expect(await storedStrokes()).toBe(saved);
    await inkIs(
      [
        [450, 300],
        [450, 380],
        [350, 380],
      ],
      [true, false, true],
    );
    await app.expectNoErrors();
  }, 60_000);

  test("panning and zooming move the view, not the drawing", async () => {
    const saved = await storedStrokes();
    await app.send("wheel", { x: 500, y: 350, dy: 100 });
    await inkIs(
      [
        [350, 200],
        [350, 300],
      ],
      [true, false],
    );
    await app.send("wheel", { x: 350, y: 200, ctrl: true, dy: -120 });
    await until(
      async () => Number(await attr("zoom", "#view-controls")),
      (z) => z > 1,
    );
    await inkIs([[350, 200]], [true]);
    await Bun.sleep(600);
    expect(await attr("strokes")).toBe("3");
    expect(await storedStrokes()).toBe(saved);
    await app.expectNoErrors();
  }, 30_000);

  test("a rectangle from the shapes tool is one stroke, inked on its edges and not inside", async () => {
    await click("#menu");
    await click("#new-drawing");
    await attrIs("strokes", "0");
    await press("u");
    await attrIs("tool", "shape");
    await drag([
      [300, 200],
      [400, 260],
      [500, 320],
    ]);
    await attrIs("strokes", "1");
    await inkIs(
      [
        [400, 200],
        [300, 260],
        [500, 260],
        [400, 260],
      ],
      [true, true, true, false],
    );
    await click("#undo");
    await attrIs("strokes", "0");
    await inkIs([[400, 200]], [false]);
    await app.expectNoErrors();
  }, 30_000);

  test("the lasso selects a stroke; dragging moves it, Delete removes it, undo brings it back", async () => {
    await press("p");
    await line(300, 450, 500, 450);
    await line(650, 450, 850, 450);
    await attrIs("strokes", "2");
    await press("l");
    await drag([
      [260, 400],
      [540, 400],
      [540, 500],
      [260, 500],
      [262, 402],
    ]);
    await attrIs("selected", "1");
    await drag([
      [400, 450],
      [400, 400],
      [400, 350],
    ]);
    await inkIs(
      [
        [400, 350],
        [400, 450],
        [750, 450],
      ],
      [true, false, true],
    );
    await press("Delete");
    await attrIs("strokes", "1");
    await inkIs([[400, 350]], [false]);
    await click("#undo");
    await attrIs("strokes", "2");
    await inkIs([[400, 350]], [true]);
    await app.expectNoErrors();
  }, 30_000);

  test("the eyedropper takes the colour drawn under it", async () => {
    await press("b");
    await click("#swatch-1");
    await line(300, 560, 600, 560);
    await click("#swatch-0");
    await attrIs("color", "ink");
    await press("i");
    await app.send("mouse", { kind: "move", x: 450, y: 560 });
    await app.send("mouse", { kind: "click", x: 450, y: 560 });
    await attrIs("color", "#3B82F6");
    expect(await attr("tool")).toBe("brush");
    await app.expectNoErrors();
  }, 30_000);

  test("mirror symmetry adds the mirrored copy in the same step", async () => {
    await click("#menu");
    await click("#new-drawing");
    await attrIs("strokes", "0");
    await press("p");
    await press("y");
    await attrIs("symmetry", "mirror");
    await line(300, 250, 380, 330);
    await attrIs("strokes", "2");
    // The axis is the middle of the window: 500, so the copy runs from 700 to 620.
    await inkIs(
      [
        [340, 290],
        [660, 290],
        [340, 400],
      ],
      [true, true, false],
    );
    await click("#undo");
    await attrIs("strokes", "0");
    await press("y");
    await attrIs("symmetry", "off");
    await app.expectNoErrors();
  }, 30_000);

  test("merge down moves a layer's strokes into the one below, as one undoable step", async () => {
    await line(300, 250, 500, 250);
    await click("#layers");
    await click("#add-layer");
    await attrIs("layers", "2");
    await line(300, 330, 500, 330);
    await click("#layers");
    await click("#merge-layer");
    await attrIs("layers", "1");
    expect(await strokes()).toBe(2);
    await inkIs(
      [
        [400, 250],
        [400, 330],
      ],
      [true, true],
    );
    await click("#undo");
    await attrIs("layers", "2");
    await app.expectNoErrors();
  }, 30_000);

  test("SVG export writes a path for every stroke", async () => {
    await click("#export");
    await click("#format-svg");
    await app.locator("#export-save").click({ real: true });
    const dialog = await app.waitForDialog();
    const target = join(dir, "drawing.svg");
    await app.answerDialog(dialog.id, target);
    const text = await until(
      async () => (existsSync(target) ? readFileSync(target, "utf8") : ""),
      (t) => t.endsWith("</svg>\n"),
    );
    expect(text.match(/<path d=/g)?.length).toBe(await strokes());
    await app.expectNoErrors();
  }, 30_000);
});
