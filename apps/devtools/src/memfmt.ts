// The Performance panel's memory half, minus Vue: how `mem`'s breakdown
// becomes bar segments, table rows and history columns.
//
// The host does the accounting (`crates/vn-app/src/devtools/mem.rs`,
// `breakdown`): categories that SUM to the committed address space, in bytes,
// with whatever no allocator accounts for as a category of its own. This file
// only lays that out — so a rounding choice here can never hide a megabyte.

import type { MemBreakdown, MemCategory, MemReply, MemUnsupported } from "./protocol";

export const GPU_COLOR = "#3a9e96";
/** One colour per category key; unknown keys (a newer host) get the grey. */
export const MEM_COLORS: Record<string, string> = {
  images: "#4a6fa5",
  mapped: "#7d6bb3",
  jsc: "#c99a2e",
  rust: "#d0703a",
  // macOS: unified memory puts the GPU's allocations in the bar.
  graphics: GPU_COLOR,
  other: "#5c6b80",
};

/**
 * Why a `mem` reply has no breakdown to draw, or `null` when it has one. A
 * host on an OS without a port answers `{supported: false, reason}`
 * (`mem_stub.rs`); anything else without `breakdown.categories` is a reply
 * this inspector cannot read, said as such rather than thrown as a TypeError.
 */
export function memUnsupported(reply: MemReply | MemUnsupported | null | undefined): string | null {
  if (reply && "supported" in reply && reply.supported === false) {
    return reply.reason || "the target has no memory breakdown on this OS";
  }
  const b = reply && "breakdown" in reply ? reply.breakdown : undefined;
  if (!b || !Array.isArray(b.categories)) return "the target's mem reply has no breakdown";
  return null;
}

/** What the bar's total is called: `committed` (Windows), `footprint` (macOS). */
export function totalLabel(b: MemBreakdown): string {
  return b.totalLabel || "committed";
}

export function memColor(key: string): string {
  return MEM_COLORS[key] ?? MEM_COLORS.other;
}

/** `512 KB`, `48.3 MB`, `1.24 GB` — a memory figure, not a transfer size. */
export function formatMem(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return "—";
  const b = Math.max(0, bytes);
  if (b < 1024 * 1024) return `${Math.round(b / 1024)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(1)} MB`;
  return `${(b / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** `12.5 %` of a total; `—` when there is no total. */
export function percent(part: number, total: number): string {
  if (!total) return "—";
  const p = (100 * part) / total;
  return p < 0.1 && p > 0 ? "<0.1 %" : `${p.toFixed(1)} %`;
}

export interface Segment {
  key: string;
  label: string;
  bytes: number;
  /** Share of the bar, 0..100. */
  pct: number;
  color: string;
}

/** The stacked bar: one segment per non-empty category, in the host's order. */
export function segments(b: MemBreakdown): Segment[] {
  const total = b.totalBytes || b.categories.reduce((s, c) => s + c.bytes, 0);
  if (!total) return [];
  return b.categories
    .filter((c) => c.bytes > 0)
    .map((c) => ({ key: c.key, label: c.label, bytes: c.bytes, pct: (100 * c.bytes) / total, color: memColor(c.key) }));
}

/** A category's non-empty slices, biggest first; the non-slice `info` rows after them. */
export function itemsOf(c: MemCategory): MemCategory["items"] {
  const slices = c.items.filter((i) => !i.info && i.bytes > 0).sort((a, b) => b.bytes - a.bytes);
  return [...slices, ...c.items.filter((i) => i.info)];
}

/** One history sample: bytes per category key at one moment. */
export interface MemSample {
  at: number;
  total: number;
  values: Record<string, number>;
}

export function sampleOf(b: MemBreakdown, at: number): MemSample {
  const values: Record<string, number> = {};
  for (const c of b.categories) values[c.key] = c.bytes;
  return { at, total: b.totalBytes, values };
}

export interface Column {
  /** Stacked bottom-up, heights in % of the chart. */
  parts: { key: string; pct: number; color: string }[];
  title: string;
}

/**
 * The history chart: one stacked column per sample, scaled to the largest
 * total in the window (so growth reads as growth, not as a rescale).
 */
export function historyColumns(history: MemSample[], keys: string[]): { columns: Column[]; ceiling: number } {
  const ceiling = history.reduce((m, s) => Math.max(m, s.total), 0);
  if (!ceiling) return { columns: [], ceiling: 0 };
  const columns = history.map((s) => ({
    parts: keys
      .map((key) => ({ key, pct: (100 * (s.values[key] ?? 0)) / ceiling, color: memColor(key) }))
      .filter((p) => p.pct > 0),
    title: `${new Date(s.at).toLocaleTimeString()} · ${formatMem(s.total)}`,
  }));
  return { columns, ceiling };
}

/** The `Force GC` acknowledgement. */
export function gcSummary(gc: { ms: number; freedPrivateBytes: number; freedJscHeapBytes: number }): string {
  return `GC in ${Math.round(gc.ms)} ms · private −${formatMem(gc.freedPrivateBytes)} · JSC heap −${formatMem(gc.freedJscHeapBytes)}`;
}
