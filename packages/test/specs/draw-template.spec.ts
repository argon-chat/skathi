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

async function crashAndRestart() {
  const pid = app.pid;
  await app.close();
  const deadline = Date.now() + 8_000;
  for (;;) {
    try {
      process.kill(pid, 0);
    } catch {
      break;
    }
    if (Date.now() >= deadline) throw new Error(`Host ${pid} survived the forced shutdown.`);
    await Bun.sleep(50);
  }
  app = await host();
}

/** The editor, whatever a failed test or attempt left behind: a host that is gone, an open replay. */
async function backToEditor() {
  if (app.child.exitCode !== null || app.child.signalCode !== null) app = await host();
  await attrIs("ready", "true");
  if ((await attr("player")) === "true") {
    await press("Escape");
    await click("#player-close");
    await attrIs("player", "false");
  }
  if (["recording", "paused"].includes(await attr("recording"))) {
    await click("#recording");
    await click("#record-stop");
    await attrIs("recording", "stopped");
    await click("#recording");
  }
}

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

  test("the drawing comes back from IndexedDB after the host is killed", async () => {
    const saved = await until(storedStrokes, (s) => JSON.parse(s).length === 3);
    await crashAndRestart();
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
  test("records painting, recovers after restart and replays saved pixels", async () => {
    const theme = async (value: "light" | "dark") => {
      await app.eval(`__SK_WINDOW__.appWindow.theme = "${value}"`);
      await Bun.sleep(180);
    };
    const panel = () => click("#recording");
    const stroke = async (key: string, xy: [number, number, number, number], count: number) => {
      await press(key);
      await line(...xy, { tiltX: 25, tiltY: -12, twist: 22 });
      await attrIs("strokes", String(count));
    };
    await backToEditor();
    await click("#menu");
    await click("#new-drawing");
    await attrIs("strokes", "0");
    await theme("light");
    await panel();
    await click("#record-start");
    await attrIs("recording", "recording");
    await press("u");
    await drag([
      [260, 180],
      [370, 245],
      [470, 330],
    ]);
    await attrIs("strokes", "1");
    await click("#swatch-1");
    // Ink readback is in flight when the same pointerup reaches the window.
    await app.eval(`window.addEventListener('pointerup', () => {
      globalThis.__attemptedUndoDuringFill = true;
      document.querySelector('#undo').click();
    }, {once:true});`);
    await stroke("k", [350, 250, 350, 250], 2);
    expect(await app.eval("globalThis.__attemptedUndoDuringFill")).toBe(true);
    await inkIs(
      [
        [350, 250],
        [500, 300],
      ],
      [true, false],
    );
    await click("#swatch-2");
    await stroke("s", [285, 400, 555, 465], 3);
    await click("#swatch-4");
    await stroke("a", [315, 430, 570, 420], 4);
    await stroke("d", [400, 428, 455, 475], 5);
    await press("f");
    await click("#swatch-3");
    await drag([
      [595, 315],
      [695, 265],
      [755, 340],
      [690, 390],
      [595, 315],
    ]);
    await attrIs("strokes", "6");
    await click("#layers");
    await click("#add-layer");
    await click("#layers");
    await stroke("b", [600, 260, 780, 350], 7);
    await click("#layers");
    await click("#layer-clipping");
    await click("#layer-lock");
    await app.eval(
      '(()=>{const s=document.querySelector("#layer-blend");s.value="multiply";s.dispatchEvent(new Event("change",{bubbles:true}))})()',
    );
    await click("#layers");
    for (const [k, x, n] of [
      ["p", 340, 8],
      ["n", 380, 9],
      ["m", 420, 10],
      ["e", 460, 11],
    ] as const)
      await stroke(k, [x, 210, x + 100, 320], n);
    await click("#undo");
    await attrIs("strokes", "10");
    await click("#redo");
    await attrIs("strokes", "11");
    await app.send("mouse", { kind: "move", x: 3, y: 4 });
    await Bun.sleep(300);
    const source = readPng(await app.screenshot(join(dir, "source-frame.png")));
    await panel();
    await click("#record-pause");
    await attrIs("recording", "paused");
    await crashAndRestart();
    await attrIs("ready", "true");
    await attrIs("recording", "paused");
    await attrIs("strokes", "11");
    await panel();
    await click("#record-resume");
    await attrIs("recording", "recording");
    await theme("dark");
    await theme("light");
    await crashAndRestart();
    await attrIs("recording", "recording");
    await attrIs("strokes", "11");
    await panel();
    await click("#record-stop");
    await attrIs("recording", "stopped");
    expect(await app.eval('document.querySelector(".recording-panel button").id')).toBe("record-save");
    await click("#record-new");
    await click("#record-cancel-new");
    await attrIs("recording", "stopped");
    await app.locator("#record-save").click({ real: true });
    const file = join(dir, "painting.skev");
    await app.answerDialog((await app.waitForDialog()).id, file);
    await until(async () => existsSync(file) && readFileSync(file).length > 32, Boolean);
    await crashAndRestart();
    await attrIs("ready", "true");
    await panel();
    await app.locator("#record-open").click({ real: true });
    await app.answerDialog((await app.waitForDialog()).id, file);
    await attrIs("player", "true");
    const duration = Number(await attr("duration", "#player-transport"));
    await app.eval(
      `globalThis.__seeked = false; window.__draw.seek(${duration}).then(() => globalThis.__seeked = true)`,
    );
    await until(async () => await app.eval("globalThis.__seeked"), Boolean);
    await attrIs("strokes", "11");
    const scale = source.width / Number(await app.eval("innerWidth"));
    const paper = source.at(Math.round(185 * scale), Math.round(140 * scale));
    // The renderer draws on its next animation frame after the seek settles.
    const compared = await until(
      async () => {
        const last = readPng(await app.screenshot(join(dir, "last-frame.png")));
        let different = 0,
          painted = 0;
        if (last.width !== source.width || last.height !== source.height)
          return { size: [last.width, last.height], painted, ratio: 1 };
        for (let y = Math.round(150 * scale); y < 490 * scale; y++)
          for (let x = Math.round(220 * scale); x < 820 * scale; x++) {
            const a = source.at(x, y),
              b = last.at(x, y);
            if (a.some((c, i) => Math.abs(c - paper[i]!) > 40)) painted++;
            if (a.some((c, i) => Math.abs(c - b[i]!) > 12)) different++;
          }
        return { size: [last.width, last.height], painted, ratio: different / painted };
      },
      (c) => c.ratio < 0.005,
    );
    expect(compared.size).toEqual([source.width, source.height]);
    expect(compared.painted).toBeGreaterThan(5000);
    await app.eval("window.__draw.seek(0)");
    await app.eval(
      "globalThis.__pauseResult = ''; window.__draw.play({ speed: 0.1 }).then(() => globalThis.__pauseResult = 'paused');window.__draw.pause()",
    );
    await until(
      async () => await app.eval("globalThis.__pauseResult"),
      (value) => value === "paused",
    );
    await line(300, 500, 400, 500);
    await attrIs("strokes", "0");
    await app.eval(
      "globalThis.__fitStart=performance.now();window.__draw.play({fitTo:30,hideUi:true}).then(()=>{globalThis.__fitElapsed=performance.now()-globalThis.__fitStart})",
    );
    await Bun.sleep(29_000);
    const elapsed = await until(
      async () => Number(await app.eval("globalThis.__fitElapsed || 0")),
      (v) => v > 0,
    );
    expect(Math.abs(elapsed - 30_000)).toBeLessThan(500);
    await attrIs("strokes", "11");
    await press("Escape");
    await app.eval(`window.__draw.seek(${duration * 0.45})`);
    const partial = await strokes();
    await click("#player-options");
    await click("#player-continue");
    await attrIs("player", "false");
    await attrIs("strokes", String(partial));
    await click("#layers");
    await click("#add-layer");
    await click("#layers");
    await stroke("b", [250, 500, 450, 500], partial + 1);
    await panel();
    await click("#record-protect");
    await click("#record-preview");
    await attrIs("player", "true");
    await click("#player-options");
    expect(await app.eval('document.querySelector("#player-continue").disabled')).toBe(true);
    await click("#player-close");
    await attrIs("strokes", String(partial + 1));
    await app.expectNoErrors();
  }, 150_000);
  test("a hard kill during pen contact restores the durable live checkpoint", async () => {
    await backToEditor();
    await app.eval('__SK_WINDOW__.appWindow.theme = "light"');
    await click("#menu");
    await click("#new-drawing");
    await attrIs("strokes", "0");
    await click("#recording");
    await click("#record-new");
    await click("#record-start");
    await attrIs("recording", "recording");
    await press("p");
    await click("#swatch-0");
    await line(300, 240, 460, 240);
    await attrIs("strokes", "1");
    await inkIs([[390, 240]], [true]);
    await app.send("pen", { kind: "down", x: 300, y: 370, pressure: 0.8 });
    await app.send("pen", { kind: "move", fromX: 300, fromY: 370, x: 525, y: 370, pressure: 0.8, steps: 36 });
    await until(async () => {
      await app.eval(`(() => {
        globalThis.__checkpoint = null;
        const req = indexedDB.open('skathi-draw-events');
        req.onsuccess = () => {
          const db = req.result, draft = db.transaction('session').objectStore('session').get('draft');
          draft.onsuccess = () => { globalThis.__checkpoint = !!draft.result; db.close(); };
        };
      })()`);
      return until(
        async () => await app.eval("globalThis.__checkpoint"),
        (v) => typeof v === "boolean",
      );
    }, Boolean);
    // No pointerup and no window.close(). Only the incremental journal can supply this stroke.
    await attrIs("strokes", "1");
    await crashAndRestart();
    await attrIs("recording", "recording");
    await attrIs("strokes", "2");
    await attrIs("ready", "true");
    await inkIs(
      [
        [390, 240],
        [390, 370],
      ],
      [true, true],
    );
    await crashAndRestart();
    await attrIs("strokes", "2");
    await app.expectNoErrors();
  }, 60_000);
});
