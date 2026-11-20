// Fullscreen covers the monitor and the app's own titlebar stays on screen, for each way in and out.

import { afterAll, expect, test } from "bun:test";
import { dlopen, FFIType, JSCallback, ptr } from "bun:ffi";
import { launch, MACOS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/fullscreen/src/main.ts",
  window: [480, 320],
  label: "fullscreen",
});
const user32 =
  process.platform === "win32"
    ? dlopen("user32.dll", {
        GetWindowRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
        GetClientRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
        ClientToScreen: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
        MonitorFromWindow: { args: [FFIType.u64, FFIType.u32], returns: FFIType.u64 },
        GetMonitorInfoW: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
        EnumDisplayMonitors: { args: [FFIType.u64, FFIType.ptr, FFIType.function, FFIType.i64], returns: FFIType.i32 },
        IsZoomed: { args: [FFIType.u64], returns: FFIType.i32 },
        SetWindowPos: {
          args: [FFIType.u64, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.u32],
          returns: FFIType.i32,
        },
      })
    : null;
afterAll(async () => {
  user32?.close();
  await app.close();
});

const T = 60_000;
const win = () => {
  if (!user32) throw new Error("Win32 only");
  return user32.symbols;
};
type Rect = { left: number; top: number; right: number; bottom: number };
type Monitor = { monitor: Rect; work: Rect };
type Page = {
  fullscreen: boolean;
  media: boolean;
  element: string | null;
  bar: { top: number; height: number };
  brandTop: number;
  inner: [number, number];
  scale: number;
};

const rectOf = (b: Int32Array, at: number): Rect => ({
  left: b[at]!,
  top: b[at + 1]!,
  right: b[at + 2]!,
  bottom: b[at + 3]!,
});
const monitorInfo = (handle: bigint | number): Monitor => {
  const mi = new Int32Array(10);
  mi[0] = 40;
  win().GetMonitorInfoW(BigInt(handle), ptr(mi));
  return { monitor: rectOf(mi, 1), work: rectOf(mi, 5) };
};
const monitors = (): Monitor[] => {
  const found: Monitor[] = [];
  const each = new JSCallback(
    (handle: bigint | number) => {
      found.push(monitorInfo(handle));
      return 1;
    },
    { args: [FFIType.u64, FFIType.u64, FFIType.ptr, FFIType.i64], returns: FFIType.i32 },
  );
  win().EnumDisplayMonitors(0n, null, each.ptr, 0n);
  each.close();
  return found;
};

const hwnd = await app.hwnd();

function native() {
  const w = new Int32Array(4);
  win().GetWindowRect(hwnd, ptr(w));
  const c = new Int32Array(4);
  win().GetClientRect(hwnd, ptr(c));
  const o = new Int32Array(2);
  win().ClientToScreen(hwnd, ptr(o));
  return {
    window: rectOf(w, 0),
    client: { left: o[0]!, top: o[1]!, right: o[0]! + c[2]!, bottom: o[1]! + c[3]! },
    ...monitorInfo(win().MonitorFromWindow(hwnd, 2 /* MONITOR_DEFAULTTONEAREST */)),
    zoomed: win().IsZoomed(hwnd) !== 0,
  };
}

const page = async (): Promise<Page> =>
  JSON.parse(
    await app.eval(`(() => {
      const bar = document.getElementById("bar").getBoundingClientRect();
      const el = document.fullscreenElement;
      return JSON.stringify({
        fullscreen: __SK_WINDOW__.appWindow.isFullscreen(),
        media: matchMedia("(display-mode: fullscreen)").matches,
        element: el ? el.tagName.toLowerCase() : null,
        bar: { top: bar.top, height: bar.height },
        brandTop: document.getElementById("brand").getBoundingClientRect().top,
        inner: [innerWidth, innerHeight],
        scale: devicePixelRatio,
      });
    })()`),
  );

async function until<V>(read: () => Promise<V> | V, ok: (v: V) => boolean, what: string): Promise<V> {
  const deadline = Date.now() + 10_000;
  let v = await read();
  while (!ok(v)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}: ${JSON.stringify(v)}`);
    await Bun.sleep(50);
    v = await read();
  }
  await Bun.sleep(150);
  return read();
}

const sameRect = (a: Rect, b: Rect) =>
  a.left === b.left && a.top === b.top && a.right === b.right && a.bottom === b.bottom;

type ButtonRect = { x: number; y: number; width: number; height: number };
type ButtonFrame = [x: number, y: number, width: number, height: number];
const buttonRect = (): Promise<ButtonRect | null> =>
  app.eval("__SK_WINDOW__.refresh(); __SK_WINDOW__.appWindow.getWindowButtonsRect()");

async function expectTrafficLights(x: number, y: number): Promise<ButtonRect> {
  const rect = await until(
    buttonRect,
    (r) => !!r && Math.abs(r.x - x) <= 1 && Math.abs(r.y - y) <= 1,
    `traffic lights at (${x}, ${y})`,
  );
  if (!rect) throw new Error("Visible traffic lights lost their rect");
  const reply = await app.send("windows", { native: true });
  const windows = reply.windows as {
    kind: string;
    native: { traffic_light_frames: [ButtonFrame, ButtonFrame, ButtonFrame] };
  }[];
  const frames = windows.find((w) => w.kind === "main")!.native.traffic_light_frames;
  expect(frames).toHaveLength(3);
  const [close, minimize, zoom] = frames;
  expect(await app.eval("__SK_WINDOW__.appWindow.chrome.trafficLightPosition")).toEqual({ x: close[0], y: close[1] });
  expect(Math.abs(close[0] - x)).toBeLessThanOrEqual(1);
  expect(minimize[0]).toBeGreaterThan(close[0] + close[2]);
  expect(zoom[0]).toBeGreaterThan(minimize[0] + minimize[2]);
  for (const [, top, width, height] of frames) {
    expect(Math.abs(top - y)).toBeLessThanOrEqual(1);
    expect(width).toBeGreaterThan(0);
    expect(Math.abs(rect.height - height)).toBeLessThanOrEqual(1);
  }
  expect(Math.abs(rect.width - (zoom[0] + zoom[2] - close[0]))).toBeLessThanOrEqual(1);
  return rect;
}

test.if(MACOS)("the first window reveal reports the three centred native traffic lights", async () => {
  // The fixture opens with a 40 px title bar; no geometry setter has run.
  const rect = await expectTrafficLights(12, 13);
  const { inner } = await page();
  expect(rect.x + rect.width).toBeLessThan(inner[0] / 2);
  expect(rect.y + rect.height).toBeLessThan(inner[1] / 2);
});

test.if(MACOS)(
  "showing the traffic lights preserves the current title-bar layout",
  async () => {
    const call = (body: string) => app.eval(`__SK_WINDOW__.appWindow.${body}; true`);
    try {
      await app.eval("__SK_WINDOW__.defineWindow({titleBarHeight:48}); true");
      const before = await expectTrafficLights(12, 17);
      console.log("48 px title bar, native button cluster", before);
      for (const fade of [0, 150]) {
        await call(`setWindowButtonsVisible(false,{fade:${fade}})`);
        expect(await until(buttonRect, (r) => r === null, "hidden buttons")).toBeNull();
        await call(`setWindowButtonsVisible(true,{fade:${fade}})`);
        expect(await expectTrafficLights(12, 17)).toEqual(before);
      }
      await call("setTrafficLightPosition({x:20,y:11})");
      const custom = await expectTrafficLights(20, 11);
      await call("setWindowButtonsVisible(false)");
      expect(await until(buttonRect, (r) => r === null, "hidden custom-position buttons")).toBeNull();
      await call("setWindowButtonsVisible(true)");
      expect(await expectTrafficLights(20, 11)).toEqual(custom);
    } finally {
      await call("setTrafficLightPosition(null)");
      await call("setWindowButtonsVisible(true)");
      await app.eval("__SK_WINDOW__.defineWindow({titleBarHeight:40}); true");
    }
  },
  T,
);

async function expectFullscreenCoversTheMonitor(element: string | null) {
  await until(page, (p) => p.fullscreen, "isFullscreen()");
  if (!user32) {
    const p = await page();
    expect(p.media).toBe(true);
    expect(p.element).toBe(element);
    expect(p.bar.top).toBeGreaterThanOrEqual(0);
    expect(p.brandTop).toBeGreaterThanOrEqual(0);
    expect(p.bar.height).toBe(40);
    return;
  }
  const n = await until(native, (n) => sameRect(n.client, n.monitor), "the client rect to reach the monitor");
  expect(n.window).toEqual(n.monitor);
  expect(n.client).toEqual(n.monitor);
  const p = await until(
    page,
    (p) => Math.round(p.inner[1] * p.scale) === n.monitor.bottom - n.monitor.top,
    "the viewport to reach the monitor",
  );
  expect(p.media).toBe(true);
  expect(p.element).toBe(element);
  expect(Math.round(p.inner[0] * p.scale)).toBe(n.monitor.right - n.monitor.left);
  expect(p.bar.top).toBeGreaterThanOrEqual(0);
  expect(p.brandTop).toBeGreaterThanOrEqual(0);
  // Undefined in fullscreen, so the fallback applies; 0 here collapses the strip and centres its row across the top edge.
  expect(p.bar.height).toBe(40);
}

async function leave(how: string) {
  await app.eval(how);
  await until(page, (p) => !p.fullscreen, "isFullscreen() to clear");
}

test.skipIf(MACOS)(
  "windowed -> fullscreen covers the monitor, and leaving restores the bounds exactly",
  async () => {
    const before = user32 ? native() : null;
    expect(before?.zoomed ?? false).toBe(false);
    await app.eval("__SK_WINDOW__.appWindow.setFullscreen(true); 'ok'");
    await expectFullscreenCoversTheMonitor(null);

    await leave("__SK_WINDOW__.appWindow.setFullscreen(false); 'ok'");
    if (before) {
      const after = await until(native, (n) => sameRect(n.window, before.window), "the windowed rect to return");
      expect(after.window).toEqual(before.window);
      expect(after.zoomed).toBe(false);
    }
    const p = await page();
    expect(p.media).toBe(false);
    expect(p.bar).toEqual({ top: 0, height: 40 });
  },
  T,
);

test.skipIf(MACOS)(
  "requestFullscreen() on the document reports fullscreenElement and covers the monitor",
  async () => {
    const before = user32 ? native() : null;
    await app.eval("document.documentElement.requestFullscreen(); 'ok'");
    await expectFullscreenCoversTheMonitor("html");
    await leave("document.exitFullscreen(); 'ok'");
    if (before) {
      const after = await until(native, (n) => sameRect(n.window, before.window), "the windowed rect to return");
      expect(after.window).toEqual(before.window);
    }
    expect((await page()).element).toBe(null);
  },
  T,
);

async function maximizedThenFullscreen() {
  await app.eval("__sk_window_verb('maximize'); 'ok'");
  if (!user32) {
    const maximized = () => app.eval("__SK_WINDOW__.appWindow.isMaximized()") as Promise<boolean>;
    await until(maximized, (m) => m, "isMaximized()");
    expect((await page()).bar.top).toBe(0);
    await app.eval("__SK_WINDOW__.appWindow.setFullscreen(true); 'ok'");
    await expectFullscreenCoversTheMonitor(null);
    await leave("__SK_WINDOW__.appWindow.setFullscreen(false); 'ok'");
    expect(await until(maximized, (m) => m, "isMaximized() after fullscreen")).toBe(true);
    await app.eval("__sk_window_verb('unmaximize'); 'ok'");
    await until(maximized, (m) => !m, "the window to unmaximize");
    return;
  }
  const max = await until(native, (n) => n.zoomed && sameRect(n.client, n.work), "the maximized client rect");
  // A frameless maximized window's frame hangs off the monitor; its client area must not.
  expect(max.client).toEqual(max.work);
  expect((await page()).bar.top).toBe(0);

  await app.eval("__SK_WINDOW__.appWindow.setFullscreen(true); 'ok'");
  await expectFullscreenCoversTheMonitor(null);

  await leave("__SK_WINDOW__.appWindow.setFullscreen(false); 'ok'");
  const back = await until(native, (n) => sameRect(n.window, max.window), "the maximized rect to return");
  expect(back.zoomed).toBe(true);
  expect(back.client).toEqual(max.work);
  expect(await app.eval("__SK_WINDOW__.appWindow.isMaximized()")).toBe(true);

  await app.eval("__sk_window_verb('unmaximize'); 'ok'");
  await until(native, (n) => !n.zoomed, "the window to unmaximize");
}

// macOS CI: leaving fullscreen never clears isFullscreen().
test.skipIf(MACOS)("maximized -> fullscreen -> back to maximized", maximizedThenFullscreen, T);

test.skipIf(!user32)(
  "the same on a non-primary monitor",
  async () => {
    const primary = native().monitor;
    const other = monitors().find((m) => !sameRect(m.monitor, primary));
    if (!other) {
      console.log("[fullscreen] one monitor: the non-primary case is not exercised");
      return;
    }
    const SWP_NOSIZE = 0x1;
    const SWP_NOZORDER = 0x4;
    const SWP_NOACTIVATE = 0x10;
    win().SetWindowPos(
      hwnd,
      0n,
      other.work.left + 40,
      other.work.top + 40,
      0,
      0,
      SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    await until(native, (n) => sameRect(n.monitor, other.monitor), "the window to reach the other monitor");
    await maximizedThenFullscreen();
    expect(native().monitor).toEqual(other.monitor);
  },
  T,
);
