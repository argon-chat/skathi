import { describe, expect, test } from "bun:test";
import {
  FLAG_DREW,
  FLAG_PACED,
  FLAG_THROTTLED,
  frameBars,
  frameCeiling,
  hasFrameWork,
  share,
  traceBar,
  traceSummary,
  workClass,
} from "../src/perffmt";

const BUDGET = 6.06;

describe("the frame meter", () => {
  test("a paced idle frame is short work and a long wait, never a slow frame", () => {
    const [bar] = frameBars({ samples: [33.4], work: [0.8], flags: [FLAG_PACED] }, BUDGET, frameCeiling([0.8], BUDGET));
    expect(bar!.cls).toBe("");
    expect(bar!.work).toBe(0.8);
    expect(bar!.wait).toBeCloseTo(32.6);
    expect(bar!.paced).toBe(true);
    expect(bar!.drew).toBe(false);
    expect(bar!.title).toContain("idle pacing");
    expect(bar!.workPct + bar!.waitPct).toBeLessThanOrEqual(100);
  });

  test("work is judged against the display's budget", () => {
    expect(workClass(2, BUDGET)).toBe("");
    expect(workClass(4, BUDGET)).toBe("warn");
    expect(workClass(9, BUDGET)).toBe("bad");
  });

  test("the flags come through", () => {
    const [bar] = frameBars({ samples: [6], work: [5], flags: [FLAG_DREW | FLAG_THROTTLED] }, BUDGET, 20);
    expect(bar!.drew).toBe(true);
    expect(bar!.throttled).toBe(true);
    expect(bar!.paced).toBe(false);
    expect(bar!.title).toContain("frame-rate limit");
  });

  test("work never reads larger than its interval", () => {
    const [bar] = frameBars({ samples: [4], work: [5], flags: [0] }, BUDGET, 20);
    expect(bar!.work).toBe(4);
    expect(bar!.wait).toBe(0);
  });

  test("the ceiling leaves room for twice the budget and the worst frame", () => {
    expect(frameCeiling([1, 2], BUDGET)).toBe(15);
    expect(frameCeiling([1, 2], 4)).toBe(8);
    expect(frameCeiling([1, 30], BUDGET)).toBe(40);
  });

  test("a recorded frame stacks its work by category under the wait", () => {
    const frame = {
      id: 1,
      durationMs: 33,
      cpuMs: 3,
      categories: [
        { category: "render", selfMs: 1 },
        { category: "wait", selfMs: 30 },
        { category: "js", selfMs: 2 },
      ],
    };
    const bar = traceBar(frame, BUDGET, 15);
    expect(bar.work).toBe(3);
    expect(bar.wait).toBe(30);
    expect(bar.cls).toBe("");
    expect(bar.segments.map((s) => s.category)).toEqual(["js", "render"]);
    expect(bar.segments[0]!.color).toBe("var(--cat-js)");
    expect(bar.segments.reduce((sum, s) => sum + s.pct, 0)).toBeCloseTo(20);
    expect(bar.workPct + bar.waitPct).toBe(100);
    expect(bar.clipped).toBe(true);
  });

  test("the recording summary counts work over budget and the time spent waiting", () => {
    const frames = [1, 2, 8].map((cpuMs, id) => ({ id, durationMs: 10, cpuMs, categories: [] }));
    const s = traceSummary(frames, BUDGET)!;
    expect(s.p50).toBe(2);
    expect(s.worst).toBe(8);
    expect(s.overBudget).toBe(1);
    expect(s.waitShare).toBeCloseTo(19 / 30);
    expect(traceSummary([], BUDGET)).toBeNull();
  });

  test("an older target without per-frame work falls back", () => {
    expect(hasFrameWork({ samples: [16, 17] })).toBe(false);
    expect(hasFrameWork({ samples: [16, 17], work: [1, 2], flags: [0, 0] })).toBe(true);
    expect(share(undefined)).toBe("—");
    expect(share(0.734)).toBe("73 %");
  });
});
