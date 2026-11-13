import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { CF_DIB, CF_DIBV5, clipboardFormat, clipboardRestore, clipboardSnapshot, dibHeader, launch } from "../src/index";
import { png } from "../fixtures/http-images/server";

const T = 120_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const bodies = new Map<string, Uint8Array>();
const body = (key: string, seed: number, size: number) => {
  let b = bodies.get(key);
  if (!b) bodies.set(key, (b = png(seed, size)));
  return b;
};
const server = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  fetch(req) {
    const m = /^\/(thumb|full|big)\/(\d+)\.png$/.exec(new URL(req.url).pathname);
    if (!m) return new Response("not found", { status: 404 });
    const seed = Number(m[2]);
    const size = m[1] === "thumb" ? 160 : m[1] === "full" ? 1600 : 2400;
    return new Response(body(`${m[1]}-${seed}`, seed, size), {
      headers: { "content-type": "image/png", "cache-control": "max-age=3600" },
    });
  },
});
const base = `http://127.0.0.1:${server.port}`;
for (let i = 0; i < 24; i++) body(`full-${i}`, i, 1600);
for (const i of [1, 2]) body(`big-${i}`, i, 2400);
const RUN = process.pid;
const item = (i: number) => ({ thumb: `${base}/thumb/${i}.png?r=${RUN}`, full: `${base}/full/${i}.png?r=${RUN}`, messageId: `m-${i}` });

const exe = process.env.VN_LIGHTBOX_EXE;
const app = await launch({
  app: "packages/vn-test/fixtures/lightbox/src/main.ts",
  window: [480, 320],
  label: "lightbox",
  ...(exe ? { exe } : {}),
});
afterAll(() => {
  app.close();
  server.stop(true);
});

// reading the windows back

type Popout = {
  node: number;
  windowId: number;
  title: string;
  visible: boolean;
  shown: boolean;
  foreground: boolean;
  cover: boolean;
  rect: [number, number, number, number] | null;
  hwnd: number;
  ownedByMain: boolean;
  appWindow: boolean;
  toolWindow: boolean;
  popup: boolean;
  thickFrame: boolean;
  nonRude: boolean;
  lastShowMs: number;
  mainMonitor: [number, number, number, number] | null;
};
const popouts = (): Promise<Popout[]> => app.eval("JSON.parse(__vn_window_popouts())");
const viewer = async () => (await popouts()).find((p) => p.title === "Lightbox")!;
const lb = <T>(expr: string): Promise<T> => app.eval(`JSON.parse(JSON.stringify(__VN_TEST_LB__.${expr}))`);

async function until<V>(what: string, probe: () => Promise<V>, ms = 8000): Promise<V> {
  const deadline = Date.now() + ms;
  let last: unknown;
  for (;;) {
    const v = await probe();
    if (v) return v;
    last = v;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
    await sleep(20);
  }
}

const user32 = dlopen("user32.dll", {
  GetForegroundWindow: { args: [], returns: FFIType.u64 },
  GetWindow: { args: [FFIType.u64, FFIType.u32], returns: FFIType.u64 },
});
const GW_HWNDPREV = 3;
function behindForeground(hwnd: number): boolean {
  const fg = user32.symbols.GetForegroundWindow();
  if (fg === 0n || fg === BigInt(hwnd)) return false;
  let h = user32.symbols.GetWindow(BigInt(hwnd), GW_HWNDPREV);
  for (let n = 0; h !== 0n && n < 10_000; n++) {
    if (h === fg) return true;
    h = user32.symbols.GetWindow(h, GW_HWNDPREV);
  }
  return false;
}

type Mem = { privateWsMB: number; dedicatedMB: number; sharedMB: number; residency: any };
async function mem(): Promise<Mem> {
  const m = await app.send("mem", {});
  const gpu = (m.gpu ?? []) as { dedicatedMB: number; sharedMB: number }[];
  const find = (o: any): any => {
    if (!o || typeof o !== "object") return null;
    if ("residency" in o) return o.residency;
    for (const v of Object.values(o)) {
      const r = find(v);
      if (r) return r;
    }
    return null;
  };
  return {
    privateWsMB: m.process.privateWorkingSetMB,
    dedicatedMB: gpu.reduce((a, g) => a + g.dedicatedMB, 0),
    sharedMB: gpu.reduce((a, g) => a + g.sharedMB, 0),
    residency: find(m.images),
  };
}

const pct = (xs: number[], p: number) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.max(0, Math.ceil((p / 100) * s.length) - 1))]!;
};
const round = (n: number) => Math.round(n * 10) / 10;

async function openByClick(i: number) {
  const before = (await lb<unknown[]>("S.events")).length;
  await app.locator(`#thumb-${i}`).click({ real: true });
  return until(`the show of item ${i}`, async () => {
    const ev = await lb<{ kind: string; firstFrameMs: number; jsMs: number; width: number; height: number }[]>("S.events");
    return ev.slice(before).find((e) => e.kind === "show");
  });
}

async function closeByEscape() {
  const before = (await lb<unknown[]>("S.events")).length;
  await app.send("key", { selector: "#lb", key: "Escape" });
  await until("the hide", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "hide"));
  await until("the window to be off screen", async () => !(await viewer()).shown);
}

const report: Record<string, unknown> = {};

describe("lightbox", () => {
  // A failed test must not leave a monitor-sized window up for the rest.
  afterEach(async () => {
    if ((await viewer())?.visible) await app.eval(`__VN_TEST_LB__.close("cleanup"); "ok"`);
  });

  test("the viewer is warm before anybody asks: hidden, owned, frameless and small", async () => {
    await app.eval(`__VN_TEST_LB__.S.items = ${JSON.stringify(Array.from({ length: 24 }, (_, i) => item(i)))}; "ok"`);
    const v = await until("the warm viewer", async () => {
      const p = await viewer();
      return p && p.hwnd ? p : null;
    });
    expect(v.visible).toBe(false);
    expect(v.shown).toBe(false);
    expect(v.cover).toBe(true);
    expect(v.ownedByMain).toBe(true);
    expect(v.appWindow).toBe(false);
    expect(v.toolWindow).toBe(false);
    expect(v.popup).toBe(true);
    expect(v.thickFrame).toBe(false);
    expect(v.nonRude).toBe(false);
    const [l, t, r, b] = v.rect!;
    expect(r - l).toBe(64);
    expect(b - t).toBe(64);
    await until("the thumbnails to load", async () => (await app.query(".thumb")).count === 24);
    await sleep(1500);
    report.warmHidden = await mem();
  }, T);

  test("a click shows the same window over the whole monitor, and Esc hides it with the main window's focus intact", async () => {
    await app.send("focus", { selector: "#composer" });
    const before = await viewer();
    const shown = await openByClick(0);
    const v = await viewer();
    const scale = await scaleOf(v);
    report.quietBehindForeground = behindForeground(v.hwnd);
    // Closed before anything is asserted: a failure must not leave a monitor-sized window up for
    // the rest of the file.
    await closeByEscape();
    report.firstShow = { shown, scale, rect: v.rect, monitor: v.mainMonitor };
    expect(v.windowId).toBe(before.windowId);
    expect(v.shown).toBe(true);
    expect(v.rect).toEqual(v.mainMonitor);
    expect(shown.firstFrameMs).toBeGreaterThan(0);
    const [l, t, r, b] = v.mainMonitor!;
    expect(Math.abs(shown.width * scale - (r - l))).toBeLessThan(2);
    expect(Math.abs(shown.height * scale - (b - t))).toBeLessThan(2);
    expect(await lb<string[]>("S.closes")).toEqual(["escape"]);
    const after = await popouts();
    expect(after.length).toBe(2);
    expect((await viewer()).windowId).toBe(before.windowId);
  }, T);

  test("the viewer's focus is its own: the main window's field keeps the main window's", async () => {
    // Opened the way a shortcut would open it: a click on a thumbnail is a press on something
    // unfocusable, which blurs the field in a browser too.
    await app.send("focus", { selector: "#composer" });
    const composer = (await app.query("#composer")).nodes[0]!.id;
    const mainFocus = async () => (await app.send("mouse", { x: 470, y: 310, kind: "move" })).focused;
    expect(await mainFocus()).toBe(composer);
    const before = (await lb<unknown[]>("S.events")).length;
    await app.eval(`__VN_TEST_LB__.open(1); "ok"`);
    await until("the show", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "show"));
    await app.send("key", { selector: "#lb", key: "ArrowRight" });
    await until("the step", async () => (await lb<number>("S.index")) === 2);
    // …and the main window's focus never moved.
    expect(await mainFocus()).toBe(composer);
    await closeByEscape();
    expect(await mainFocus()).toBe(composer);
  }, T);

  test("the arrows step, and a click on the dim closes", async () => {
    await openByClick(2);
    await app.send("key", { selector: "#lb", key: "ArrowRight" });
    await until("the step to 4", async () => (await lb<number>("S.index")) === 3);
    await app.send("key", { selector: "#lb", key: "ArrowLeft" });
    await app.send("key", { selector: "#lb", key: "ArrowLeft" });
    await until("the step back to 2", async () => (await lb<number>("S.index")) === 1);
    const v = await viewer();
    const before = (await lb<unknown[]>("S.events")).length;
    expect(await app.eval(`__vn_window_click(${v.node}, 4, 4)`)).toBe(true);
    await until("the hide", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "hide"));
    expect((await lb<string[]>("S.closes")).at(-1)).toBe("dim");
  }, T);

  test("a thumbnail maps into the viewer through the screen", async () => {
    const v = await viewer();
    const r = await app.eval(`JSON.stringify(__VN_TEST_LB__.mapThumb(0))`);
    const rect = JSON.parse(r) as { x: number; y: number; width: number; height: number };
    expect(Math.abs(rect.width - 40)).toBeLessThan(0.5);
    expect(Math.abs(rect.height - 40)).toBeLessThan(0.5);
    const [l, t, rr, b] = v.mainMonitor!;
    const s = await scaleOf(v);
    expect(rect.x).toBeGreaterThanOrEqual(-1);
    expect(rect.y).toBeGreaterThanOrEqual(-1);
    expect(rect.x + rect.width).toBeLessThanOrEqual((rr - l) / s + 1);
    expect(rect.y + rect.height).toBeLessThanOrEqual((b - t) / s + 1);
  }, T);

  test("click → first frame over 20 opens: a decoded picture, and one that is not", async () => {
    const cached: { first: number; js: number }[] = [];
    for (let n = 0; n < 20; n++) {
      const s = await openByClick(0);
      cached.push({ first: s.firstFrameMs, js: s.jsMs });
      if (n === 0) report.open = await mem();
      await closeByEscape();
    }
    // Not decoded: a picture never asked for before, every time.
    const uncached: { first: number; js: number; load: number }[] = [];
    for (let n = 0; n < 20; n++) {
      const i = 4 + n;
      const loadsBefore = (await lb<unknown[]>("S.loads")).length;
      const s = await openByClick(i);
      const load = await until(`item ${i}'s full picture`, async () =>
        (await lb<{ index: number; ms: number }[]>("S.loads")).slice(loadsBefore).find((l) => l.index === i), 15_000);
      uncached.push({ first: s.firstFrameMs, js: s.jsMs, load: load.ms });
      await closeByEscape();
    }
    const sum = (xs: number[]) => ({ p50: round(pct(xs, 50)), p95: round(pct(xs, 95)) });
    report.timings = {
      cached: { hostFirstFrameMs: sum(cached.map((c) => c.first)), clickToShownMs: sum(cached.map((c) => c.js)) },
      uncached: {
        hostFirstFrameMs: sum(uncached.map((c) => c.first)),
        clickToShownMs: sum(uncached.map((c) => c.js)),
        clickToFullLoadMs: sum(uncached.map((c) => c.load)),
      },
    };
    expect(cached.every((c) => c.first > 0 && c.js > 0)).toBe(true);
  }, 240_000);

  test("hidden 30 s after a close, the viewer is warm-small again", async () => {
    report.hiddenNow = await mem();
    const since = (await app.logs(0)).next;
    await sleep(32_000);
    const { entries } = await app.logs(since);
    report.shrinkLog = entries.filter((e) => /warm size/.test(e.text)).map((e) => e.text);
    const v = await viewer();
    const [l, t, r, b] = v.rect!;
    expect(r - l).toBe(64);
    expect(b - t).toBe(64);
    report.hiddenAfter30s = await mem();
  }, T);

  test("a fullResolution window keeps its picture's full decode; the main window's is reduced", async () => {
    const before = (await mem()).residency;
    await app.eval(
      `__VN_TEST_LB__.S.controlSrc = ${JSON.stringify(`${base}/big/1.png?r=${RUN}`)};` +
        `__VN_TEST_LB__.S.pinSrc = ${JSON.stringify(`${base}/big/2.png?r=${RUN}`)};` +
        `__VN_TEST_LB__.S.pinOpen = true; "ok"`,
    );
    const pinned = await until(
      "a reduce in the main window and a pin in the other",
      async () => {
        const r = (await mem()).residency;
        return r.reduced > before.reduced && r.keptFull.fullResWindow >= 1 ? r : null;
      },
      20_000,
    );
    expect(pinned.keptFull.fullResWindow).toBe(1);
    await app.eval(`__VN_TEST_LB__.S.pinOpen = false; "ok"`);
    await until("the pin to go", async () => (await mem()).residency.keptFull.fullResWindow === 0, 10_000);
    report.residency = { before, pinned };
  }, T);

  test("Ctrl+wheel zooms about the pointer; a plain wheel does nothing", async () => {
    await ensureItems();
    await openByClick(0);
    try {
      const id = (await viewer()).windowId;
      const r0 = await until("the picture's box", () => lb<PicRect | null>("picRect()"));
      const x = r0.left + r0.width * 0.62;
      const y = r0.top + r0.height * 0.57;
      const u = (x - r0.left) / r0.width;
      const v = (y - r0.top) / r0.height;
      const wheels = async () => lb<{ ctrl: boolean; x: number; y: number; dy: number }[]>("S.wheels");
      const before = (await wheels()).length;
      await app.send("wheel", { x, y, dy: 120, window: id });
      const plain = await until("the plain wheel", async () => (await wheels()).slice(before)[0]);
      expect(plain.ctrl).toBe(false);
      expect(Math.abs(plain.x - x)).toBeLessThan(1.5);
      expect(Math.abs(plain.y - y)).toBeLessThan(1.5);
      await sleep(100);
      expect(await lb<number>("S.zoom")).toBe(1);
      const steps: { zoom: number; dx: number; dy: number }[] = [];
      for (let n = 1; n <= 3; n++) {
        const zoomBefore = await lb<number>("S.zoom");
        await app.send("wheel", { x, y, dy: -120, ctrl: true, window: id });
        const zoom = await until(`notch ${n}`, async () => {
          const z = await lb<number>("S.zoom");
          return z > zoomBefore + 0.01 ? z : 0;
        });
        const r = await until(`notch ${n}'s box`, async () => {
          const rr = await lb<PicRect>("picRect()");
          return Math.abs(rr.width - r0.width * zoom) < 1 ? rr : null;
        });
        steps.push({ zoom: round(zoom * 1000) / 1000, dx: round(r.left + u * r.width - x), dy: round(r.top + v * r.height - y) });
      }
      report.zoomAboutPointer = { at: { x, y, r0 }, steps, log: await lb<unknown[]>("S.zoomLog") };
      for (const s of steps) {
        expect(Math.abs(s.dx)).toBeLessThan(1.5);
        expect(Math.abs(s.dy)).toBeLessThan(1.5);
      }
    } finally {
      await closeByEscape();
    }
  }, T);

  test("a drag pans a zoomed picture no further than its edges; a drag that ends on the dim does not close", async () => {
    await ensureItems();
    await openByClick(0);
    try {
      const id = (await viewer()).windowId;
      const r0 = await until("the picture's box", () => lb<PicRect | null>("picRect()"));
      const zoomTo = async (z: number) => {
        await app.eval(`__VN_TEST_LB__.setZoom(${z}); "ok"`);
        return until(`zoom ${z}`, async () => {
          const r = await lb<PicRect>("picRect()");
          return Math.abs(r.width - r0.width * z) < 1 ? r : null;
        });
      };
      const dragTo = async (from: [number, number], to: [number, number]) => {
        await app.send("mouse", { x: from[0], y: from[1], kind: "down", window: id });
        await app.send("mouse", { x: (from[0] + to[0]) / 2, y: (from[1] + to[1]) / 2, kind: "move", held: true, window: id });
        await app.send("mouse", { x: to[0], y: to[1], kind: "move", held: true, window: id });
        await app.send("mouse", { x: to[0], y: to[1], kind: "up", window: id });
        await sleep(150);
        return lb<PicRect>("picRect()");
      };
      const z2 = await zoomTo(2);
      const centre: [number, number] = [z2.vw / 2, z2.vh / 2];
      const toBottomRight = await dragTo(centre, [z2.vw - 3, z2.vh - 3]);
      await zoomTo(2);
      const toTopLeft = await dragTo(centre, [3, 3]);
      report.panClamp = { zoom2: z2, toBottomRight, toTopLeft };
      expect(Math.abs(toBottomRight.left)).toBeLessThan(1.5);
      expect(Math.abs(toBottomRight.top)).toBeLessThan(1.5);
      expect(Math.abs(toTopLeft.left + toTopLeft.width - toTopLeft.vw)).toBeLessThan(1.5);
      expect(Math.abs(toTopLeft.top + toTopLeft.height - toTopLeft.vh)).toBeLessThan(1.5);
      // Barely zoomed: it fits, so it stays centred whatever the drag — and the drag ends on the
      // dim, which must not close the viewer.
      const small = await zoomTo(1.04);
      const clicks = await lb<number>("S.dimClicks");
      const closes = (await lb<string[]>("S.closes")).length;
      const after = await dragTo([small.vw / 2, small.vh / 2], [4, 4]);
      await sleep(250);
      report.dragEndingOnDim = { dimClicksArrived: (await lb<number>("S.dimClicks")) - clicks };
      expect(Math.abs(after.left + after.width / 2 - after.vw / 2)).toBeLessThan(1.5);
      expect(Math.abs(after.top + after.height / 2 - after.vh / 2)).toBeLessThan(1.5);
      expect((await lb<string[]>("S.closes")).length).toBe(closes);
      expect((await viewer()).visible).toBe(true);
      await dragTo([small.vw / 2, small.vh / 2], [small.vw / 2 + 60, small.vh / 2 + 40]);
      const before = (await lb<unknown[]>("S.events")).length;
      await app.send("mouse", { x: 4, y: 4, kind: "click", window: id });
      await until("the hide", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "hide"));
      expect((await lb<string[]>("S.closes")).at(-1)).toBe("dim");
    } finally {
      if ((await viewer()).visible) await closeByEscape();
    }
  }, T);

  test("a right click opens the menu in the viewer, at the click; Go to message closes the viewer", async () => {
    await ensureItems();
    await openByClick(1);
    try {
      const m = await openMenu(0.3, 0.4);
      report.menu = m;
      expect(m.inWindow).toBe(true);
      expect(Math.abs(m.left - m.x)).toBeLessThan(1.5);
      expect(Math.abs(m.top - m.y)).toBeLessThan(1.5);
      expect(m.sameDocument).toBe(false);
      expect(m.bodyIsRoot).toBe(true);
      expect(Math.abs(m.docWidth - m.vw)).toBeLessThan(1.5);
      expect(m.mainWidth).toBeLessThan(m.docWidth);
      expect(m.rootClass.split(" ").sort()).toEqual(["lb-window", "vn-window-root"]);
      expect(m.bodyPointerEvents).toBe("none");
      expect(await app.eval(`JSON.stringify(document.body.style.pointerEvents)`)).toBe(`""`);
      // A Teleport that mounted with the content, before that was in the document, is the viewer's
      // too.
      expect(m.hintInWindow).toBe(true);
      const before = (await lb<unknown[]>("S.events")).length;
      await clickInViewer("#lb-menu-goto");
      await until("the hide", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "hide"));
      expect((await lb<string[]>("S.actions")).at(-1)).toBe("goto:m-1");
      expect((await lb<string[]>("S.closes")).at(-1)).toBe("goto");
    } finally {
      if ((await viewer()).visible) await closeByEscape();
    }
  }, T);

  test("a press outside the menu dismisses it and is no click on the dim; the next click on the dim closes", async () => {
    await ensureItems();
    await openByClick(1);
    try {
      await openMenu(0.5, 0.5);
      const id = (await viewer()).windowId;
      const closes = (await lb<string[]>("S.closes")).length;
      const dims = await lb<number>("S.dimClicks");
      const dismissals = (await lb<string[]>("S.dismissals")).length;
      // The dim's corner, which the layer's `body { pointer-events: none }` covers: the window's
      // root takes the press, as a browser's document would, and the layer's `document` listener
      // hears it.
      await app.send("mouse", { x: 4, y: 4, kind: "click", window: id });
      await until("the dismissal", async () => (await lb<string[]>("S.dismissals")).length > dismissals);
      await until("the menu gone", async () => (await lb<MenuInfo | null>("menuInfo()")) === null);
      expect((await lb<string[]>("S.closes")).length).toBe(closes);
      expect(await lb<number>("S.dimClicks")).toBe(dims);
      expect((await viewer()).visible).toBe(true);
      const before = (await lb<unknown[]>("S.events")).length;
      await app.send("mouse", { x: 4, y: 4, kind: "click", window: id });
      await until("the hide", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "hide"));
      expect((await lb<string[]>("S.closes")).at(-1)).toBe("dim");
    } finally {
      if ((await viewer()).visible) await closeByEscape();
    }
  }, T);

  test("Copy puts the picture on the clipboard as its PNG and as the bitmaps other apps paste", async () => {
    await ensureItems();
    const saved = clipboardSnapshot();
    try {
      await openByClick(2);
      await openMenu(0.5, 0.5);
      const before = (await lb<string[]>("S.actions")).length;
      await clickInViewer("#lb-menu-copy");
      const done = await until("the copy", async () => (await lb<string[]>("S.actions")).slice(before).find((a) => a.startsWith("copy:")), 15_000);
      const bytes = body("full-2", 2, 1600);
      expect(done).toBe(`copy:ok:${bytes.length}`);
      const asPng = clipboardFormat("PNG");
      expect(asPng?.length).toBe(bytes.length);
      expect(Buffer.from(asPng!).equals(Buffer.from(bytes))).toBe(true);
      const v5 = dibHeader(clipboardFormat(CF_DIBV5)!);
      const dib = dibHeader(clipboardFormat(CF_DIB)!);
      report.copy = { pngBytes: asPng?.length, v5, dib };
      expect(v5).toMatchObject({ headerSize: 124, width: 1600, height: 1600, bits: 32 });
      expect(dib).toMatchObject({ width: 1600, height: 1600, bits: 32 });
      await app.eval(`__VN_TEST_LB__.readBack(); "ok"`);
      const back = await until("the read back", () => lb<string>("S.readBack"));
      expect(back).toBe(`image/png:${bytes.length}:137`);
    } finally {
      clipboardRestore(saved);
      if ((await viewer()).visible) await closeByEscape();
    }
  }, T);

  test("Save As asks the save dialog with the picture's name and writes its own bytes", async () => {
    await ensureItems();
    const dir = join(tmpdir(), `vn-lightbox-save-${process.pid}`);
    mkdirSync(dir, { recursive: true });
    try {
      await openByClick(3);
      await openMenu(0.5, 0.5);
      const before = (await lb<string[]>("S.actions")).length;
      await clickInViewer("#lb-menu-save");
      const d = await app.waitForDialog(10_000);
      report.saveDialog = { kind: d.kind, suggestedName: d.suggestedName, filters: d.filters };
      expect(d.kind).toBe("save");
      expect(d.suggestedName).toBe("picture-3.png");
      const out = join(dir, "picture-3.png");
      await app.answerDialog(d.id, out);
      const done = await until("the save", async () => (await lb<string[]>("S.actions")).slice(before).find((a) => a.startsWith("save:")), 15_000);
      const bytes = body("full-3", 3, 1600);
      expect(done).toBe(`save:ok:picture-3.png:${bytes.length}`);
      expect(Buffer.from(readFileSync(out)).equals(Buffer.from(bytes))).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
      if ((await viewer()).visible) await closeByEscape();
    }
  }, T);

  test("a muted preview is silent, the viewer's video is heard, and closing the viewer silences it", async () => {
    await ensureItems();
    // The chat's preview: muted and played through its properties before its `src`, as Argon's
    // VideoPlayer does in `onMounted`.
    await app.eval(`__VN_TEST_LB__.S.inlineSrc = "/src/av-silent.mp4"; __VN_TEST_LB__.S.inlineOn = true; "ok"`);
    try {
      const playing = await until("the preview to play", async () => {
        const s = await lb<{ muted: boolean; paused: boolean } | null>("inlineState()");
        return s && !s.paused ? s : null;
      }, 20_000);
      expect(playing.muted).toBe(true);
      await sleep(300);
      const preview = await videos();
      expect(preview).toEqual({ videos: 1, audible: 0 });
      const n = (await lb<unknown[]>("S.items")).length;
      await app.eval(`__VN_TEST_LB__.S.items.push(${JSON.stringify({ ...item(99), video: "/src/av-silent.mp4" })}); "ok"`);
      const before = (await lb<unknown[]>("S.events")).length;
      await app.eval(`__VN_TEST_LB__.open(${n}); "ok"`);
      await until("the show", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "show"));
      const heard = await until("the viewer's video to be heard", async () => {
        const v = await videos();
        return v.audible === 1 ? v : null;
      }, 20_000);
      expect(heard.videos).toBe(2);
      expect((await lb<{ muted: boolean }>("inlineState()")).muted).toBe(true);
      const t0 = performance.now();
      await closeByEscape();
      const quiet = await until("silence", async () => {
        const v = await videos();
        return v.audible === 0 ? v : null;
      }, 5_000);
      report.audio = { preview, heard, quietAfterCloseMs: round(performance.now() - t0), quiet };
      expect(quiet.videos).toBe(1);
      await sleep(900);
      expect((await videos()).videos).toBe(1);
      await app.eval(`__VN_TEST_LB__.S.items.pop(); "ok"`);
    } finally {
      await app.eval(`__VN_TEST_LB__.S.inlineOn = false; "ok"`);
    }
  }, T);

  test("a video taken out of the document is paused, hears it, and stays paused when it comes back", async () => {
    await app.eval(`__VN_TEST_LB__.S.inlineEvents = []; __VN_TEST_LB__.S.inlineSrc = "/src/av-silent.mp4"; __VN_TEST_LB__.S.inlineOn = true; "ok"`);
    try {
      await until("the preview to play", async () => {
        const s = await lb<{ paused: boolean } | null>("inlineState()");
        return s && !s.paused;
      }, 20_000);
      const before = (await lb<string[]>("S.inlineEvents")).length;
      expect(await app.eval(`__VN_TEST_LB__.detachInline()`)).toBe(true);
      await until("its pause", async () => (await lb<string[]>("S.inlineEvents")).slice(before).includes("pause"), 5_000);
      expect((await lb<{ paused: boolean }>("inlineState()")).paused).toBe(true);
      expect(await app.eval(`__VN_TEST_LB__.attachInline()`)).toBe(true);
      await sleep(600);
      const back = await lb<{ paused: boolean }>("inlineState()");
      report.detach = { events: await lb<string[]>("S.inlineEvents"), pausedAfterReinsert: back.paused };
      expect(back.paused).toBe(true);
    } finally {
      await app.eval(`__VN_TEST_LB__.S.inlineOn = false; "ok"`);
    }
  }, T);

  test("report", () => {
    console.log(JSON.stringify(report, null, 2));
  });
});

type PicRect = { left: number; top: number; width: number; height: number; vw: number; vh: number };

async function ensureItems() {
  if ((await lb<unknown[]>("S.items")).length >= 24) return;
  await app.eval(`__VN_TEST_LB__.S.items = ${JSON.stringify(Array.from({ length: 24 }, (_, i) => item(i)))}; "ok"`);
  await until("the warm viewer", async () => (await viewer())?.hwnd);
}

interface MenuInfo {
  left: number;
  top: number;
  inWindow: boolean;
  sameDocument: boolean;
  docWidth: number;
  bodyIsRoot: boolean;
  mainWidth: number;
  rootClass: string;
  bodyPointerEvents: string;
  hintInWindow: boolean;
}

async function openMenu(fx: number, fy: number) {
  const id = (await viewer()).windowId;
  const r = await until("the picture's box", () => lb<PicRect | null>("picRect()"));
  const x = r.left + r.width * fx;
  const y = r.top + r.height * fy;
  await app.send("mouse", { x, y, kind: "click", button: 2, window: id });
  const m = await until("the menu", () => lb<MenuInfo | null>("menuInfo()"));
  return { ...m, x, y, vw: r.vw };
}

async function clickInViewer(selector: string) {
  const id = (await viewer()).windowId;
  const r = JSON.parse(
    await app.eval(`JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`),
  ) as { left: number; top: number; width: number; height: number };
  await app.send("mouse", { x: r.left + r.width / 2, y: r.top + r.height / 2, kind: "click", window: id });
}

async function videos(): Promise<{ videos: number; audible: number }> {
  const m = await app.send("mem", {});
  const find = (o: any, key: string): any => {
    if (!o || typeof o !== "object") return undefined;
    if (key in o) return o[key];
    for (const v of Object.values(o)) {
      const r = find(v, key);
      if (r !== undefined) return r;
    }
    return undefined;
  };
  return { videos: find(m, "videos") ?? -1, audible: find(m, "videosAudible") ?? -1 };
}

async function scaleOf(_v: Popout): Promise<number> {
  return Number(await app.eval("devicePixelRatio"));
}
