// A media viewer as its own window, the way Telegram Desktop's is
// (docs/MULTIWINDOW.md §7.9).
//
//   bun test packages/vn-test/specs/lightbox.spec.ts
//
// `fixtures/lightbox` keeps a `<VnWindow visible=false cover="monitor"
// :taskbar=false fullResolution>` warm from boot, and a thumbnail's click
// shows it. The claims, in order:
//
//   * the warm window exists before anybody asks for it: hidden, owned by the
//     main window, no taskbar button (`WS_EX_APPWINDOW` off, not a tool
//     window), no frame (`WS_POPUP`, no `WS_THICKFRAME`), and small;
//   * a click SHOWS it — the same window every time — covering the whole
//     monitor the main window is on, its first frame drawn before it appears;
//   * Esc, a click on the dim and the arrows reach the viewer, and closing
//     hides it, leaves the main window's focused field where it was, and the
//     next open is a show again;
//   * click → first frame, p50 and p95 over 20 opens, for a picture already
//     decoded and for one that is not (the thumbnail is what the first frame
//     shows; the full picture's load is reported beside it);
//   * the private working set and the GPU memory warm-hidden, open, and
//     hidden 30 s after a close, when the window has gone back to its warm
//     size;
//   * a picture in a `fullResolution` window keeps its full decode while the
//     same kind of picture drawn as small in the main window is reduced, and
//     the pin goes when the window stops showing it.
//
// Quiet: vn-test runs `VN_WINDOW_FOCUS=never`, so the viewer is shown without
// activation and stays in the z-order just above its owner, which vn-test
// sends to the bottom — the spec logs whether it was behind the foreground
// window. Each show is closed again within a second or two. What this file
// CANNOT prove quietly is that a foreground viewer is above the taskbar (the
// shell drops the taskbar only for an ACTIVE full-screen window) and that a
// close hands the activation back to the main window: both need the
// foreground, which a spec on a machine in use must not take.

import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { launch } from "../src/index";
import { png } from "../fixtures/http-images/server";

const T = 120_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------
// pictures: a thumbnail and a full one per item, served over loopback
// ---------------------------------------------------------------------------

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
// Made up front, so a load time is the host's and not this process's deflate.
for (let i = 0; i < 24; i++) body(`full-${i}`, i, 1600);
for (const i of [1, 2]) body(`big-${i}`, i, 2400);
const RUN = process.pid;
/** Distinct per run, so nothing comes out of a previous run's disk cache. */
const item = (i: number) => ({ thumb: `${base}/thumb/${i}.png?r=${RUN}`, full: `${base}/full/${i}.png?r=${RUN}` });

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

// ---------------------------------------------------------------------------
// reading the windows back
// ---------------------------------------------------------------------------

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
/** Is the foreground window above `hwnd` in the z-order? */
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

/** Clicks thumbnail `i` (a real platform click in the main window) and waits
 *  for the show. Returns the show's event. */
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
    // Owned by the main window and not an app window: no taskbar button, one
    // Alt-Tab entry for the pair. Not a tool window either (§7.9).
    expect(v.ownedByMain).toBe(true);
    expect(v.appWindow).toBe(false);
    expect(v.toolWindow).toBe(false);
    // No frame: a popup, no sizing border, and not opted out of the shell's
    // full-screen detection.
    expect(v.popup).toBe(true);
    expect(v.thickFrame).toBe(false);
    expect(v.nonRude).toBe(false);
    // Warm SMALL: a hidden window keeps its swapchain.
    const [l, t, r, b] = v.rect!;
    expect(r - l).toBe(64);
    expect(b - t).toBe(64);
    // The thumbnails are in, and the memory is the warm-hidden one.
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
    // Closed before anything is asserted: a failure must not leave a
    // monitor-sized window up for the rest of the file.
    await closeByEscape();
    report.firstShow = { shown, scale, rect: v.rect, monitor: v.mainMonitor };
    expect(v.windowId).toBe(before.windowId);
    expect(v.shown).toBe(true);
    expect(v.rect).toEqual(v.mainMonitor);
    expect(shown.firstFrameMs).toBeGreaterThan(0);
    // The viewport the first frame was laid out in is the monitor's.
    const [l, t, r, b] = v.mainMonitor!;
    expect(Math.abs(shown.width * scale - (r - l))).toBeLessThan(2);
    expect(Math.abs(shown.height * scale - (b - t))).toBeLessThan(2);
    expect(await lb<string[]>("S.closes")).toEqual(["escape"]);
    // Still the one warm window, hidden.
    const after = await popouts();
    expect(after.length).toBe(2);
    expect((await viewer()).windowId).toBe(before.windowId);
  }, T);

  test("the viewer's focus is its own: the main window's field keeps the main window's", async () => {
    // Opened the way a shortcut would open it: a click on a thumbnail is a
    // press on something unfocusable, which blurs the field in a browser too.
    await app.send("focus", { selector: "#composer" });
    const composer = (await app.query("#composer")).nodes[0]!.id;
    const mainFocus = async () => (await app.send("mouse", { x: 470, y: 310, kind: "move" })).focused;
    expect(await mainFocus()).toBe(composer);
    const before = (await lb<unknown[]>("S.events")).length;
    await app.eval(`__VN_TEST_LB__.open(1); "ok"`);
    await until("the show", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "show"));
    // The keys reach the viewer's own focused element…
    await app.send("key", { selector: "#lb", key: "ArrowRight" });
    await until("the step", async () => (await lb<number>("S.index")) === 2);
    // …and the main window's focus never moved. That is what the keyboard
    // reaches once the main window is active again. (`document.activeElement`
    // is the realm's one answer and reads `body` after the close, as it does
    // after a browser's in-page viewer closes; with real activation gpui
    // reports the field's focus again when the main window comes back, which
    // a quiet run never sees.)
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
    // The dim's own corner: not the stage, which starts at 5 %.
    const before = (await lb<unknown[]>("S.events")).length;
    expect(await app.eval(`__vn_window_click(${v.node}, 4, 4)`)).toBe(true);
    await until("the hide", async () => (await lb<{ kind: string }[]>("S.events")).slice(before).some((e) => e.kind === "hide"));
    expect((await lb<string[]>("S.closes")).at(-1)).toBe("dim");
  }, T);

  test("a thumbnail maps into the viewer through the screen", async () => {
    const v = await viewer();
    const r = await app.eval(`JSON.stringify(__VN_TEST_LB__.mapThumb(0))`);
    const rect = JSON.parse(r) as { x: number; y: number; width: number; height: number };
    // 40 CSS px in the main window; the same physical size in the viewer,
    // which is on the same monitor at the same scale.
    expect(Math.abs(rect.width - 40)).toBeLessThan(0.5);
    expect(Math.abs(rect.height - 40)).toBeLessThan(0.5);
    // Inside the monitor the viewer covers.
    const [l, t, rr, b] = v.mainMonitor!;
    const s = await scaleOf(v);
    expect(rect.x).toBeGreaterThanOrEqual(-1);
    expect(rect.y).toBeGreaterThanOrEqual(-1);
    expect(rect.x + rect.width).toBeLessThanOrEqual((rr - l) / s + 1);
    expect(rect.y + rect.height).toBeLessThanOrEqual((b - t) / s + 1);
  }, T);

  test("click → first frame over 20 opens: a decoded picture, and one that is not", async () => {
    // Decoded: item 0's full picture was shown above; open it again and again.
    const cached: { first: number; js: number }[] = [];
    for (let n = 0; n < 20; n++) {
      const s = await openByClick(0);
      cached.push({ first: s.firstFrameMs, js: s.jsMs });
      if (n === 0) report.open = await mem();
      await closeByEscape();
    }
    // Not decoded: a picture never asked for before, every time. The first
    // frame shows the thumbnail, which the main window already has.
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
    // Sanity, not the target: the target is a release build's (the report).
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
    // Hidden, the window stops showing it, and the pin goes with the next sweep.
    await app.eval(`__VN_TEST_LB__.S.pinOpen = false; "ok"`);
    await until("the pin to go", async () => (await mem()).residency.keptFull.fullResWindow === 0, 10_000);
    report.residency = { before, pinned };
  }, T);

  test("report", () => {
    console.log(JSON.stringify(report, null, 2));
  });
});

/** The viewer's device px per CSS px — the main window's, on the same monitor. */
async function scaleOf(_v: Popout): Promise<number> {
  return Number(await app.eval("devicePixelRatio"));
}
