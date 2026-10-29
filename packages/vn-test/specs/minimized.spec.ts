// The JS event loop keeps running while the window is minimised (m26 fixes,
// research/reports/m26/client-sweep-1.md finding 4; crates/vn-app/src/background.rs).
//
//   bun test packages/vn-test/specs/minimized.spec.ts
//
// A minimised window draws no frames, and the frame pump used to be the only
// thing driving timers, sockets and devtools: the Argon client's realtime
// socket closed with 1006, and its sleep watcher reloaded the app on restore.
// A browser throttles a hidden page; it never stops it. Here a background tick
// takes over: timers and devtools run, `requestAnimationFrame` waits for the
// next drawn frame.
//
// The window is minimised by the app itself and restored from outside with
// `SW_SHOWNOACTIVATE`: no focus, no cursor, no input. Any fixture will do;
// this one borrows `events`.

import { afterAll, expect, test } from "bun:test";
import { dlopen, FFIType } from "bun:ffi";
import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "minimized",
});
const user32 = dlopen("user32.dll", {
  ShowWindow: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i32 },
  IsIconic: { args: [FFIType.u64], returns: FFIType.i32 },
});
afterAll(async () => {
  user32.close();
  await app.close();
});

const T = 60_000;

test("timers, promises and devtools run while minimised; rAF waits for the next frame", async () => {
  const shot = await app.send("screenshot", {
    path: join(tmpdir(), `vn-minimized-hwnd-${process.pid}.png`),
    backend: "render_to_image",
  });
  rmSync(shot.path, { force: true });
  const hwnd = BigInt(shot.hwnd);

  // A 100 ms interval, a chain of 50 ms timeouts each resolving a promise,
  // and a rAF loop, all counted.
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

  await app.eval("__vn_window_verb('minimize'); 'ok'");
  const until = Date.now() + 10_000;
  while (!user32.symbols.IsIconic(hwnd) && Date.now() < until) await Bun.sleep(50);
  expect(user32.symbols.IsIconic(hwnd)).not.toBe(0);
  // Let the observer's foreground task deliver `hidden` and the tick take over.
  await Bun.sleep(800);

  // devtools answers while minimised — every eval below is one more proof.
  const before = JSON.parse(await app.eval("JSON.stringify(__MIN__)"));
  expect(before.vis).toEqual(["hidden"]);
  expect(await app.eval("document.hidden")).toBe(true);
  await Bun.sleep(2_000);
  const after = JSON.parse(await app.eval("JSON.stringify(__MIN__)"));
  // ~20 interval ticks and ~40 chained timeouts in 2 s at full rate; the
  // background tick is 100 ms, so ask for well under half of that.
  expect(after.interval - before.interval).toBeGreaterThanOrEqual(8);
  expect(after.chain - before.chain).toBeGreaterThanOrEqual(8);
  // No animation frame ran while hidden.
  expect(after.raf).toBe(before.raf);
  expect(after.rafHidden).toBe(0);

  user32.symbols.ShowWindow(hwnd, 4 /* SW_SHOWNOACTIVATE */);
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
}, T);

test("the host said when the tick took over and when frames came back", async () => {
  const { entries } = await app.logs(0);
  const text = entries.map((e: { text: string }) => e.text).join("\n");
  expect(text).toContain("the background tick");
  expect(text).toContain("frames resumed");
}, T);

test("the app logged nothing broken", async () => {
  await app.expectNoErrors();
}, T);
