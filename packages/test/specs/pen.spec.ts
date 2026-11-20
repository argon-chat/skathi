// A pen, as a platform reports one (devtools `pen` dispatches gpui's PenEvent): the page's pointer events carry
// pressure, tilt, the eraser and the readings a frame folded together; the mouse events follow; capture holds.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, LINUX, MACOS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/pen/src/main.ts",
  window: [420, 320],
  label: "pen",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function pen(kind: string, x: number, y: number, extra: Record<string, unknown> = {}): Promise<void> {
  app.notePointer({ kind: "point", x, y });
  await app.send("pen", { x, y, kind, ...extra });
  // Moves are delivered once a frame.
  await sleep(80);
}
async function log(): Promise<string[]> {
  return JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_PEN__.log.splice(0))"))) as string[];
}
async function moves(): Promise<unknown[]> {
  return JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_PEN__.moves.splice(0))"))) as unknown[];
}

describe("pen input", () => {
  test(
    "haptics reports availability and rejects an unknown pointer",
    async () => {
      const values = await app.eval(`JSON.stringify((() => {
      const api = __SK_TEST_PEN__.haptics;
      return [typeof api.supported, api.supportsPointer(999), api.perform("alignment", {pointerId:999, when:"next-frame"}),
        api.perform("ink-continuous"), api.stop(999), api.supported, api.supported ? null : api.perform("generic")];
    })())`);
      const [type, pointer, queued, continuous, stopped, supported, played] = JSON.parse(String(values));
      expect([type, pointer, queued, continuous, stopped]).toEqual(["boolean", false, false, false, false]);
      if (LINUX || process.env.SK_TEST_NO_HAPTICS === "1") expect(supported).toBe(false);
      if (!supported) expect(played).toBe(false);
    },
    T,
  );

  test(
    "stationary trackpad pressure produces mouse pointermoves and clears on release",
    async () => {
      const pos = { x: 70, y: 70 };
      const force = async (stage: number, pressure: number) => {
        await app.send("mouse_pressure", { ...pos, stage, pressure });
        await sleep(100);
        return (await log()).filter((s) => s.startsWith("pointermove:mouse"));
      };
      await app.send("mouse", { ...pos, kind: "down" });
      await sleep(100);
      expect(await log()).toContain("pointerdown:mouse:1:b0/1:p0.50:t0,0");
      expect(await force(1, 0.8)).toEqual(["pointermove:mouse:1:b-1/1:p0.40:t0,0:c1:70@0.40"]);
      expect(await force(2, 0.6)).toEqual(["pointermove:mouse:1:b-1/1:p0.80:t0,0:c1:70@0.80"]);
      expect(await force(2, 0.6)).toEqual([]);
      await app.send("mouse", { ...pos, kind: "up" });
      await sleep(100);
      expect(await log()).toContain("pointerup:mouse:1:b0/0:p0.00:t0,0");
      expect(await force(2, 1)).toEqual([]);
      await app.send("mouse", { ...pos, kind: "click" });
      expect(await log()).toContain("pointerdown:mouse:1:b0/1:p0.50:t0,0");
      await moves();
    },
    T,
  );

  // macOS CI: the window does not get frames for synthetic input when it is not key.
  test.skipIf(MACOS)(
    "hover, a pressure stroke with coalesced readings, lift and leave",
    async () => {
      await pen("hover", 360, 260);
      await log();
      await pen("hover", 60, 60, { tiltX: 20, tiltY: -10 });
      expect(await log()).toEqual([
        "pointerover:pen",
        "pointerenter:pen",
        "pointermove:pen:2:b-1/0:p0.00:t20,-10:c1:60@0.00",
      ]);
      await moves();
      await pen("down", 60, 60, { pressure: 0.3 });
      await pen("move", 100, 60, { pressure: 0.8, fromX: 60, fromY: 60, fromPressure: 0.4, steps: 4, interval: 4 });
      await pen("up", 100, 60);
      // The readings' own times, 4 ms apart on the page's clock; four readings into a stroke the
      // predictor is unsure yet, so it carries the stroke on by one 1/180 s step.
      expect(await moves()).toEqual([{ steps: [4, 4, 4], beforeEvent: true, recent: true, ahead: [6], forward: true }]);
      expect(await log()).toEqual([
        "pointerdown:pen:2:b0/1:p0.30:t0,0",
        "got:2",
        "mousedown:b0",
        "pointermove:pen:2:b-1/1:p0.80:t0,0:c4:70@0.50,80@0.60,90@0.70,100@0.80",
        "pointerup:pen:2:b0/0:p0.00:t0,0",
        "mouseup:b0",
        "lost:2",
      ]);
      await pen("leave", 100, 60);
      expect(await log()).toEqual(["pointerout:pen", "pointerleave:pen"]);
    },
    T,
  );

  test.skipIf(MACOS)(
    "the eraser presses button 5, the barrel button 2",
    async () => {
      await pen("hover", 80, 80, { eraser: true });
      await log();
      await pen("down", 80, 80, { eraser: true, pressure: 1 });
      await pen("up", 80, 80, { eraser: true });
      await pen("down", 80, 80, { barrel: true });
      await pen("up", 80, 80, { barrel: true });
      const seen = (await log()).filter((l) => l.startsWith("pointerdown") || l.startsWith("mousedown"));
      expect(seen).toEqual([
        "pointerdown:pen:2:b5/32:p1.00:t0,0",
        "mousedown:b0",
        "pointerdown:pen:2:b2/2:p0.50:t0,0",
        "mousedown:b2",
      ]);
    },
    T,
  );
});
