export type InspectorRequest = <T>(command: Record<string, unknown>) => Promise<T>;
export interface TraceHint {
  code: string;
  severity: string;
  message: string;
  count: number;
  spanId?: number;
}
export interface TraceCategory {
  category: string;
  selfMs: number;
  totalMs: number;
  count: number;
}
export interface TraceFrame {
  id: number;
  startMs: number;
  durationMs: number;
  cpuMs: number;
  spanCount: number;
  droppedSpans: number;
  categories: TraceCategory[];
  hints: TraceHint[];
}
export interface TraceSpan {
  id: number;
  parentId: number | null;
  category: string;
  name: string;
  startMs: number;
  durationMs: number;
  selfMs: number;
  depth: number;
  count: number;
  callsite?: { kind: string; function?: string; url?: string; line?: number; column?: number };
  counters?: Record<string, number>;
  hints?: string[];
}
export interface TraceStatus {
  recording: boolean;
  sessionId: number;
  frameCount: number;
  droppedFrames: number;
  droppedSpans: number;
  limits: { frames: number; spansPerFrame: number; totalSpans: number };
}
export interface TraceFrames {
  sessionId: number;
  recording: boolean;
  total: number;
  offset: number;
  nextOffset: number | null;
  frames: TraceFrame[];
}
export interface TraceDetail {
  sessionId: number;
  frame: TraceFrame;
  offset: number;
  total: number;
  nextOffset: number | null;
  truncated: boolean;
  spans: TraceSpan[];
}
export interface Preview {
  type: string;
  preview: string;
  children?: { name: string; value: Preview }[];
  size?: number;
  mime?: string;
  name?: string;
  ref?: number;
  truncated?: boolean;
}
export interface DatabaseList {
  available: boolean;
  backend: string;
  databases: { name: string; version: number }[];
}
export interface DatabaseIndex {
  name: string;
  keyPath: unknown;
  unique: boolean;
  multiEntry: boolean;
  count: number;
}
export interface DatabaseStore {
  name: string;
  keyPath: unknown;
  autoIncrement: boolean;
  count: number;
  indexes: DatabaseIndex[];
}
export interface DatabaseSchema {
  database: string;
  version: number;
  stores: DatabaseStore[];
}
export interface DatabaseRecord {
  key: Preview;
  primaryKey: Preview;
  value: Preview;
  encodedBytes: number;
}
export interface DatabaseRecords {
  records: DatabaseRecord[];
  nextCursor: string | null;
  hasMore: boolean;
  scanned: number;
  truncated: boolean;
}
export interface AllocationRow {
  category: string;
  site: string;
  sizeClass: string | number;
  allocatedBytes: number;
  freedBytes: number;
  liveBytes: number;
  allocations: number;
  frees: number;
  liveBlocks: number;
}
export interface AllocationSnapshot {
  session: number;
  recording: boolean;
  available: boolean;
  coverage: Record<string, boolean | string | number>;
  droppedEvents: number;
  totals: Omit<AllocationRow, "category" | "site" | "sizeClass"> & { netBytes?: number };
  sampleEvery: number;
  capturedEvents?: number;
  maxEvents?: number;
  heap?: {
    liveBytes: number;
    liveBlocks: number;
    deltaBytes: number;
    deltaBlocks: number;
    allocationsSinceStart: number;
  } | null;
  matchedRows?: number;
  truncated?: boolean;
  rows: AllocationRow[];
  startedAtMs: number;
  elapsedMs: number;
  complete: boolean;
}

export function milliseconds(value: number): string {
  return Number.isFinite(value) ? `${value < 1 ? value.toFixed(3) : value.toFixed(2)} ms` : "—";
}
export function byteLabel(value: number, signed = false): string {
  if (!Number.isFinite(value)) return "—";
  const absolute = Math.abs(value);
  const unit = absolute >= 1048576 ? 1048576 : absolute >= 1024 ? 1024 : 1;
  return `${value < 0 ? "−" : signed && value > 0 ? "+" : ""}${(absolute / unit).toFixed(unit === 1 ? 0 : 1)} ${unit === 1048576 ? "MB" : unit === 1024 ? "KB" : "B"}`;
}
export function rowWindow(count: number, scroll: number, height: number, rowHeight = 24, overscan = 3) {
  count = Math.max(0, Math.floor(count));
  const top = Math.max(
    0,
    Math.min(Math.max(0, count * rowHeight - Math.max(0, height)), Number.isFinite(scroll) ? scroll : 0),
  );
  const start = Math.max(0, Math.floor(top / rowHeight) - overscan);
  const end = Math.min(count, Math.ceil((top + Math.max(0, height)) / rowHeight) + overscan);
  return { start, end, top, padTop: start * rowHeight, padBottom: Math.max(0, (count - end) * rowHeight) };
}
export function frameBuckets(frames: TraceFrame[], maximum = 180) {
  const stride = Math.max(1, Math.ceil(frames.length / Math.max(1, maximum)));
  const buckets: { frame: TraceFrame; count: number }[] = [];
  for (let offset = 0; offset < frames.length; offset += stride) {
    let frame = frames[offset]!;
    const end = Math.min(frames.length, offset + stride);
    for (let index = offset + 1; index < end; index++)
      if (
        frames[index]!.cpuMs > frame.cpuMs ||
        (frames[index]!.cpuMs === frame.cpuMs && frames[index]!.durationMs > frame.durationMs)
      )
        frame = frames[index]!;
    buckets.push({ frame, count: end - offset });
  }
  return buckets;
}
export function spanRows(spans: TraceSpan[], collapsed: ReadonlySet<number> = new Set()) {
  const compare = (left: TraceSpan, right: TraceSpan) => left.startMs - right.startMs || left.id - right.id;
  const unique = new Map<number, TraceSpan>();
  for (const span of spans) if (!unique.has(span.id)) unique.set(span.id, span);
  const children = new Map<number, TraceSpan[]>();
  const roots: TraceSpan[] = [];
  for (const span of unique.values()) {
    if (span.parentId !== null && span.parentId !== span.id && unique.has(span.parentId)) {
      const group = children.get(span.parentId) || [];
      group.push(span);
      children.set(span.parentId, group);
    } else roots.push(span);
  }
  for (const group of children.values()) group.sort(compare);
  roots.sort(compare);
  const visited = new Set<number>();
  const rows: { span: TraceSpan; depth: number; hasChildren: boolean }[] = [];
  function visit(root: TraceSpan) {
    const stack = [{ span: root, depth: 0, hidden: false }];
    while (stack.length) {
      const item = stack.pop()!;
      if (visited.has(item.span.id)) continue;
      visited.add(item.span.id);
      const nested = children.get(item.span.id) || [];
      if (!item.hidden) rows.push({ span: item.span, depth: item.depth, hasChildren: nested.length > 0 });
      for (let index = nested.length - 1; index >= 0; index--)
        stack.push({ span: nested[index]!, depth: item.depth + 1, hidden: item.hidden || collapsed.has(item.span.id) });
    }
  }
  for (const root of roots) visit(root);
  for (const span of [...unique.values()].sort(compare)) if (!visited.has(span.id)) visit(span);
  return rows;
}
export function timeRange(start: number, end: number, duration: number) {
  const total = Math.max(0.001, Number.isFinite(duration) ? duration : 0.001);
  const width = Math.max(Math.min(total, 0.001), Math.min(total, Number.isFinite(end - start) ? end - start : total));
  const left = Math.max(0, Math.min(total - width, Number.isFinite(start) ? start : 0));
  return { start: left, end: left + width };
}
export function zoomRange(start: number, end: number, duration: number, factor: number) {
  const center = (start + end) / 2;
  const width = (end - start) * Math.max(0.01, factor);
  return timeRange(center - width / 2, center + width / 2, duration);
}
export function allocationKey(row: AllocationRow): string {
  return JSON.stringify([row.category, row.site, row.sizeClass]);
}
export function allocationDiff(before: AllocationRow[], after: AllocationRow[]): AllocationRow[] {
  const rows = new Map(after.map((row) => [allocationKey(row), { ...row }]));
  for (const old of before) {
    const key = allocationKey(old);
    const row = rows.get(key) || {
      ...old,
      allocatedBytes: 0,
      freedBytes: 0,
      liveBytes: 0,
      allocations: 0,
      frees: 0,
      liveBlocks: 0,
    };
    for (const field of ["allocatedBytes", "freedBytes", "liveBytes", "allocations", "frees", "liveBlocks"] as const)
      row[field] -= old[field];
    rows.set(key, row);
  }
  return [...rows.values()].sort(
    (left, right) =>
      Math.abs(right.liveBytes) - Math.abs(left.liveBytes) || allocationKey(left).localeCompare(allocationKey(right)),
  );
}
export function callsiteLabel(span: TraceSpan): string {
  const site = span.callsite;
  if (!site) return "No callsite captured";
  return (
    [
      site.function,
      site.url &&
        `${site.url}${site.line === undefined ? "" : `:${site.line}${site.column === undefined ? "" : `:${site.column}`}`}`,
    ]
      .filter(Boolean)
      .join(" · ") || site.kind
  );
}
