import { describe, expect, test } from "bun:test";
import {
  allocationDiff,
  byteLabel,
  frameBuckets,
  rowWindow,
  spanRows,
  timeRange,
  zoomRange,
} from "../src/inspector-model";
import type { AllocationRow, TraceFrame, TraceSpan } from "../src/inspector-model";
function span(id: number, parentId: number | null, startMs: number): TraceSpan {
  return { id, parentId, startMs, name: `call ${id}`, category: "js", durationMs: 1, selfMs: 1, depth: 0, count: 1 };
}
function allocation(site: string, liveBytes: number, allocatedBytes = liveBytes, freedBytes = 0): AllocationRow {
  return {
    category: "rust",
    site,
    sizeClass: "64",
    liveBytes,
    allocatedBytes,
    freedBytes,
    allocations: 2,
    frees: 0,
    liveBlocks: 2,
  };
}
describe("bounded inspector rows", () => {
  test("ten million entries render only viewport and overscan", () => {
    const value = rowWindow(10_000_000, 240_000, 480);
    expect(value.end - value.start).toBe(26);
    expect(value.padTop + (value.end - value.start) * 24 + value.padBottom).toBe(240_000_000);
  });
  test("stale scroll is clamped after the result shrinks", () => {
    const value = rowWindow(5, 240_000, 48);
    expect(value.top).toBe(72);
    expect(value.end).toBe(5);
    expect(rowWindow(0, Number.NaN, 300)).toEqual({ start: 0, end: 0, top: 0, padTop: 0, padBottom: 0 });
  });
});
describe("ordered call hierarchy", () => {
  test("parents precede children even when pages arrive out of order", () => {
    const rows = spanRows([span(4, 1, 3), span(2, 1, 1), span(1, null, 0), span(3, 2, 2)]);
    expect(rows.map((row) => [row.span.id, row.depth])).toEqual([
      [1, 0],
      [2, 1],
      [3, 2],
      [4, 1],
    ]);
  });
  test("collapse hides descendants without promoting them into roots", () => {
    const rows = spanRows([span(1, null, 0), span(2, 1, 1), span(3, 2, 2), span(4, null, 4)], new Set([1]));
    expect(rows.map((row) => row.span.id)).toEqual([1, 4]);
    expect(rows[0].hasChildren).toBe(true);
  });
  test("missing parents, cycles and duplicate page entries stay finite and inspectable", () => {
    const rows = spanRows([span(2, 1, 0), span(1, 2, 1), span(3, 99, 2), span(2, 1, 0)]);
    expect(rows).toHaveLength(3);
    expect(new Set(rows.map((row) => row.span.id)).size).toBe(3);
  });
});
describe("timeline navigation", () => {
  test("overview is bounded and retains the worst frame of each group", () => {
    const frames = Array.from(
      { length: 1000 },
      (_, id) =>
        ({
          id,
          durationMs: id % 7,
          startMs: id,
          cpuMs: 1,
          spanCount: 1,
          droppedSpans: 0,
          categories: [],
          hints: [],
        }) satisfies TraceFrame,
    );
    const buckets = frameBuckets(frames, 100);
    expect(buckets).toHaveLength(100);
    expect(buckets.reduce((sum, value) => sum + value.count, 0)).toBe(1000);
    expect(buckets[0].frame.durationMs).toBe(6);
  });
  test("zoom retains its center and pan clamps to the captured duration", () => {
    expect(zoomRange(0, 20, 20, 0.5)).toEqual({ start: 5, end: 15 });
    expect(timeRange(18, 28, 20)).toEqual({ start: 10, end: 20 });
    expect(zoomRange(0, 20, 20, 10)).toEqual({ start: 0, end: 20 });
    expect(timeRange(0, 0, 0).end).toBeGreaterThan(0);
  });
});
describe("allocation comparison", () => {
  test("churn and retained bytes remain independent, and released groups are visible", () => {
    const rows = allocationDiff(
      [allocation("A", 60, 100, 40), allocation("released", 64)],
      [allocation("A", 100, 200, 100), allocation("new", 128)],
    );
    expect(rows.find((row) => row.site === "A")).toMatchObject({ liveBytes: 40, allocatedBytes: 100, freedBytes: 60 });
    expect(rows.find((row) => row.site === "released")?.liveBytes).toBe(-64);
    expect(rows[0].site).toBe("new");
  });
  test("comparison never mutates either snapshot", () => {
    const before = [allocation("A", 64)],
      after = [allocation("A", 128)];
    allocationDiff(before, after);
    expect(before[0].liveBytes).toBe(64);
    expect(after[0].liveBytes).toBe(128);
    expect(byteLabel(-64, true)).toBe("−64 B");
    expect(byteLabel(0, true)).toBe("0 B");
  });
});
