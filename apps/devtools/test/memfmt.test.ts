import { describe, expect, test } from "bun:test";
import {
  GPU_COLOR,
  formatMem,
  gcSummary,
  historyColumns,
  itemsOf,
  memColor,
  memUnsupported,
  percent,
  sampleOf,
  segments,
  totalLabel,
} from "../src/memfmt";
import type { MemBreakdown, MemReply, MemUnsupported } from "../src/protocol";

const MB = 1024 * 1024;

function breakdown(): MemBreakdown {
  return {
    basis: "committed address space: private + mapped + image",
    totalBytes: 1000 * MB,
    privateBytes: 600 * MB,
    workingSetBytes: 400 * MB,
    privateWorkingSetBytes: 300 * MB,
    rustCounting: true,
    categories: [
      { key: "images", label: "Loaded images", bytes: 400 * MB, note: "", items: [] },
      { key: "mapped", label: "Mapped files", bytes: 0, note: "", items: [] },
      {
        key: "jsc",
        label: "JSC",
        bytes: 250 * MB,
        note: "",
        items: [
          { label: "GC heap in use", bytes: 70 * MB, info: true },
          { label: "mimalloc live", bytes: 150 * MB },
          { label: "JIT", bytes: 6 * MB },
          { label: "mimalloc free", bytes: 94 * MB },
        ],
      },
      { key: "rust", label: "Rust heap", bytes: 250 * MB, note: "", items: [] },
      { key: "other", label: "Other private", bytes: 100 * MB, note: "", items: [] },
    ],
    gpu: { dedicatedBytes: 160 * MB, sharedBytes: 20 * MB, items: [] },
  };
}

describe("formatMem", () => {
  test("KB below a megabyte, MB below a gigabyte, GB above", () => {
    expect(formatMem(512 * 1024)).toBe("512 KB");
    expect(formatMem(48.34 * MB)).toBe("48.3 MB");
    expect(formatMem(1.5 * 1024 * MB)).toBe("1.50 GB");
  });
  test("nothing to show is a dash, never 0", () => {
    expect(formatMem(undefined)).toBe("—");
    expect(formatMem(Number.NaN)).toBe("—");
  });
  test("a negative (a racing subtraction) reads as zero", () => {
    expect(formatMem(-5)).toBe("0 KB");
  });
});

describe("segments", () => {
  test("one per non-empty category, in the host's order, summing to 100 %", () => {
    const s = segments(breakdown());
    expect(s.map((x) => x.key)).toEqual(["images", "jsc", "rust", "other"]);
    expect(s.reduce((a, x) => a + x.pct, 0)).toBeCloseTo(100, 6);
  });
  test("an unknown category from a newer host is drawn, in the neutral colour", () => {
    const b = breakdown();
    b.categories.push({ key: "brand-new", label: "?", bytes: 1 * MB, note: "", items: [] });
    expect(segments(b).at(-1)?.color).toBe(segments(b).find((x) => x.key === "other")?.color);
  });
});

describe("itemsOf", () => {
  test("slices biggest first, then the info rows that are not slices", () => {
    const jsc = breakdown().categories[2];
    expect(itemsOf(jsc).map((i) => i.label)).toEqual(["mimalloc live", "mimalloc free", "JIT", "GC heap in use"]);
  });
});

describe("history", () => {
  test("columns are scaled to the largest total, so growth is visible", () => {
    const a = sampleOf(breakdown(), 1);
    const big = breakdown();
    big.totalBytes = 2000 * MB;
    big.categories[0].bytes = 1400 * MB;
    const b = sampleOf(big, 2);
    const { columns, ceiling } = historyColumns([a, b], ["images", "jsc", "rust", "other"]);
    expect(ceiling).toBe(2000 * MB);
    const height = (i: number) => columns[i].parts.reduce((s, p) => s + p.pct, 0);
    expect(height(0)).toBeCloseTo(50, 6);
    expect(height(1)).toBeCloseTo(100, 6);
  });
  test("an empty history draws nothing", () => {
    expect(historyColumns([], ["jsc"]).columns).toEqual([]);
  });
});

describe("memUnsupported", () => {
  test("a host without a port says why, and that is what the panel shows", () => {
    const stub: MemUnsupported = { supported: false, reason: "it has no port to this OS yet" };
    expect(memUnsupported(stub)).toBe("it has no port to this OS yet");
    expect(memUnsupported({ supported: false, reason: "" })).toContain("no memory breakdown");
  });
  test("a reply without categories is unreadable, not a TypeError", () => {
    expect(memUnsupported(undefined)).not.toBeNull();
    expect(memUnsupported({ collectMs: 1 } as unknown as MemReply)).not.toBeNull();
    expect(memUnsupported({ breakdown: {}, collectMs: 1 } as unknown as MemReply)).not.toBeNull();
  });
  test("a breakdown is drawable", () => {
    expect(memUnsupported({ breakdown: breakdown(), collectMs: 3 })).toBeNull();
  });
});

describe("a macOS breakdown", () => {
  test("names its total and draws graphics in the GPU colour", () => {
    const b = breakdown();
    expect(totalLabel(b)).toBe("committed");
    b.totalLabel = "footprint";
    expect(totalLabel(b)).toBe("footprint");
    b.categories.splice(4, 0, { key: "graphics", label: "Graphics", bytes: 0, note: "", items: [] });
    expect(memColor("graphics")).toBe(GPU_COLOR);
  });
});

test("percent and the GC acknowledgement", () => {
  expect(percent(250, 1000)).toBe("25.0 %");
  expect(percent(1, 0)).toBe("—");
  expect(gcSummary({ ms: 41.6, freedPrivateBytes: 30 * MB, freedJscHeapBytes: 12 * MB })).toBe(
    "GC in 42 ms · private −30.0 MB · JSC heap −12.0 MB",
  );
});
