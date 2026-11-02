// Argon's call timer, the titlebar's `00:00` (apps/client AppTitlebar.vue,
// motionCounter/Counter.vue and Digit.vue), on the real host.
//
//   bun test packages/vn-test/specs/call-timer.spec.ts
//
// The owner's report: in a call the titlebar showed the call's name and no
// time at all. The time ticks (a 1 s setInterval, @argon/core startTimer).
// What hid it was each digit's clipping cell, `width: 1ch`. The mapper
// dropped `ch` as unresolved, the cell was 0 px wide, and its strip of digits
// was clipped away entirely. `ch` and `ex` are now half an em, css-values-4's
// value for a glyph that is not measured, and the em is the cell's own font
// size, the counter's 9px (crates/vn-css README, divergence 7). That is a
// 4.5px cell.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/call-timer/src/main.ts",
  window: [200, 80],
  label: "call-timer",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Cell = { w: number; h: number; digit: string | null };
const cells = async (): Promise<Cell[]> => JSON.parse(await app.eval(`JSON.stringify(__VN_TEST_TIMER__.cells())`));
const digits = async (): Promise<string> => (await cells()).map((c) => c.digit ?? "?").join("");
/** Waits out Digit's 900 ms roll. */
const settle = () => sleep(1300);

describe("the call timer", () => {
  test("each digit's `width: 1ch` cell is half an em of its 9px text, one 9px line high", async () => {
    await sleep(400);
    const cs = await cells();
    expect(cs.length).toBe(4);
    for (const c of cs) {
      // 4.5px, within the layout's whole-pixel rounding.
      expect(Math.abs(c.w - 4.5)).toBeLessThanOrEqual(0.5);
      expect(c.h).toBeCloseTo(9, 1);
    }
    expect(await digits()).toBe("0000");
  }, T);

  test("the digits show the time, after each place's roll", async () => {
    await app.eval(`__VN_TEST_TIMER__.set(83)`);
    await settle();
    expect(await digits()).toBe("0123");
    await app.eval(`__VN_TEST_TIMER__.set(599)`);
    await settle();
    expect(await digits()).toBe("0959");
    await app.eval(`__VN_TEST_TIMER__.set(0)`);
    await settle();
    expect(await digits()).toBe("0000");
  }, T);

  test("startTimer's setInterval ticks it, and the seconds follow", async () => {
    await app.eval(`__VN_TEST_TIMER__.start()`);
    await sleep(2400);
    await app.eval(`__VN_TEST_TIMER__.stop()`);
    const s = Number(await app.eval(`__VN_TEST_TIMER__.seconds`));
    expect(s).toBeGreaterThanOrEqual(2);
    await settle();
    expect(await digits()).toBe(`000${s % 10}`);
  }, T);

  test("the timer is on screen: its digits put light ink on the dark bar", async () => {
    await app.eval(`__VN_TEST_TIMER__.set(83)`);
    await settle();
    const box = JSON.parse(await app.eval(`JSON.stringify(document.getElementById("timer").getBoundingClientRect())`));
    const shot = readPng(await app.screenshot());
    const k = shot.width / 200;
    const rect = { x: Math.floor(box.x * k), y: Math.floor(box.y * k), w: Math.ceil(box.width * k), h: Math.ceil(box.height * k) };
    // #9aa0a6 on #202225: anything well above the bar's luma is a glyph.
    expect(rect.w).toBeGreaterThan(0);
    expect(shot.count(rect, (luma) => luma > 90)).toBeGreaterThan(10);
  }, T);
});
