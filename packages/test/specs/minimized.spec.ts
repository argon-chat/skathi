// A browser throttles a hidden page; it never stops it.

import { afterAll, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { launch, WINDOWS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "minimized",
});
const user32 =
  process.platform === "win32"
    ? dlopen("user32.dll", {
        ShowWindow: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i32 },
        IsIconic: { args: [FFIType.u64], returns: FFIType.i32 },
      })
    : null;
afterAll(async () => {
  user32?.close();
  await app.close();
});

const T = 60_000;

// Off Windows show() does not bring a minimized window back, and the host logs no background tick.
test.skipIf(!WINDOWS)(
  "timers, promises and devtools run while minimised; rAF waits for the next frame",
  async () => {
    const hwnd = await app.hwnd();

    await app.eval(`(() => {
    const m = (globalThis.__MIN__ = { interval: 0, chain: 0, raf: 0, rafHidden: 0, vis: [] });
    setInterval(() => { m.interval++; }, 100);
    const step = () => new Promise((r) => setTimeout(r, 50)).then(() => { m.chain++; step(); });
    step();
    const frame = () => { m.raf++; if (document.hidden) m.rafHidden++; requestAnimationFrame(frame); };
    requestAnimationFrame(frame);
    document.addEventListener("visibilitychange", () => m.vis.push(document.visibilityState));
    return "ok";
  })()`);
    await Bun.sleep(300);

    await app.eval("__sk_window_verb('minimize'); 'ok'");
    const iconic = async () =>
      user32 ? user32.symbols.IsIconic(hwnd) !== 0 : await app.eval("__SK_WINDOW__.appWindow.isMinimized()");
    const until = Date.now() + 10_000;
    while (!(await iconic()) && Date.now() < until) await Bun.sleep(50);
    expect(await iconic()).toBe(true);
    await Bun.sleep(800);

    const before = JSON.parse(await app.eval("JSON.stringify(__MIN__)"));
    expect(before.vis).toEqual(["hidden"]);
    expect(await app.eval("document.hidden")).toBe(true);
    await Bun.sleep(2_000);
    const after = JSON.parse(await app.eval("JSON.stringify(__MIN__)"));
    expect(after.interval - before.interval).toBeGreaterThanOrEqual(8);
    expect(after.chain - before.chain).toBeGreaterThanOrEqual(8);
    expect(after.raf).toBe(before.raf);
    expect(after.rafHidden).toBe(0);

    if (user32) user32.symbols.ShowWindow(hwnd, 4 /* SW_SHOWNOACTIVATE */);
    else await app.eval("__sk_window_verb('show'); 'ok'");
    let back = after;
    const restoreBy = Date.now() + 10_000;
    while (Date.now() < restoreBy) {
      back = JSON.parse(await app.eval("JSON.stringify(__MIN__)"));
      if (back.raf > after.raf + 5 && back.vis.length >= 2) break;
      await Bun.sleep(50);
    }
    expect(back.vis).toEqual(["hidden", "visible"]);
    expect(back.raf).toBeGreaterThan(after.raf + 5);
    expect(await app.eval("document.hidden")).toBe(false);
  },
  T,
);

test.skipIf(!WINDOWS)(
  "the host said when the tick took over and when frames came back",
  async () => {
    const { entries } = await app.logs(0);
    const text = entries.map((e: { text: string }) => e.text).join("\n");
    expect(text).toContain("the background tick");
    expect(text).toContain("frames resumed");
  },
  T,
);

test(
  "the app logged nothing broken",
  async () => {
    await app.expectNoErrors();
  },
  T,
);
