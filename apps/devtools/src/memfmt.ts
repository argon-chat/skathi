// This file only lays that out — so a rounding choice here can never hide a megabyte.

import type { MemBreakdown, MemCategory, MemReply, MemUnsupported } from "./protocol";

export const GPU_COLOR = "#3a9e96";
export const MEM_COLORS: Record<string, string> = {
  images: "#4a6fa5",
  mapped: "#7d6bb3",
  jsc: "#c99a2e",
  rust: "#d0703a",
  // macOS: unified memory puts the GPU's allocations in the bar.
  graphics: GPU_COLOR,
  other: "#5c6b80",
};

/** Why a `mem` reply has no breakdown to draw, or `null` when it has one. */
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

export function formatMem(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return "—";
  const b = Math.max(0, bytes);
  if (b < 1024 * 1024) return `${Math.round(b / 1024)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(1)} MB`;
  return `${(b / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function percent(part: number, total: number): string {
  if (!total) return "—";
  const p = (100 * part) / total;
  return p < 0.1 && p > 0 ? "<0.1 %" : `${p.toFixed(1)} %`;
}

export interface Segment {
  key: string;
  label: string;
  bytes: number;
  pct: number;
  color: string;
}

export function segments(b: MemBreakdown): Segment[] {
  const total = b.totalBytes || b.categories.reduce((s, c) => s + c.bytes, 0);
  if (!total) return [];
  return b.categories
    .filter((c) => c.bytes > 0)
    .map((c) => ({ key: c.key, label: c.label, bytes: c.bytes, pct: (100 * c.bytes) / total, color: memColor(c.key) }));
}

export function itemsOf(c: MemCategory): MemCategory["items"] {
  const slices = c.items.filter((i) => !i.info && i.bytes > 0).sort((a, b) => b.bytes - a.bytes);
  return [...slices, ...c.items.filter((i) => i.info)];
}

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
  parts: { key: string; pct: number; color: string }[];
  title: string;
}

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

export function gcSummary(gc: { ms: number; freedPrivateBytes: number; freedJscHeapBytes: number }): string {
  return `GC in ${Math.round(gc.ms)} ms · private −${formatMem(gc.freedPrivateBytes)} · JSC heap −${formatMem(gc.freedJscHeapBytes)}`;
}
