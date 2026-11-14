// The frame meter's model: one bar per frame, split into the work the frame took and the time the
// clock then waited, and why it waited.

export const FLAG_PACED = 1;
export const FLAG_DREW = 2;
export const FLAG_THROTTLED = 4;

/** What a `perf` reply carries per frame: frame-to-frame interval, work and flags, oldest first. */
export interface FrameSamples {
  samples: number[];
  work?: number[] | undefined;
  flags?: number[] | undefined;
}

export type WorkClass = "" | "warn" | "bad";

export interface FrameBar {
  work: number;
  /** The rest of the interval: the clock waiting for the next frame. */
  wait: number;
  paced: boolean;
  drew: boolean;
  throttled: boolean;
  cls: WorkClass;
  workPct: number;
  waitPct: number;
  title: string;
}

/** A display refresh when the target does not report one. */
export const FALLBACK_BUDGET_MS = 1000 / 60;

/**
 * Work against the display's frame: past the budget the frame cost a refresh, past 60 % of it there
 * is little headroom left. Waiting never counts, so a paced idle window stays clean.
 */
export function workClass(ms: number, budgetMs: number): WorkClass {
  if (ms > budgetMs) return "bad";
  if (ms > budgetMs * 0.6) return "warn";
  return "";
}

/** The chart's top: room for twice the budget and the worst work, on a round step. */
export function frameCeiling(work: number[], budgetMs: number): number {
  const top = Math.max(budgetMs * 2, ...work.map((ms) => ms * 1.25));
  const step = top <= 10 ? 2 : top <= 40 ? 5 : top <= 100 ? 10 : 50;
  return Math.ceil(top / step) * step;
}

function ms(v: number): string {
  return v < 10 ? v.toFixed(2) : v.toFixed(1);
}

export function hasFrameWork(reply: FrameSamples): boolean {
  return Array.isArray(reply.work) && reply.work.length === reply.samples.length;
}

export function frameBars(reply: FrameSamples, budgetMs: number, ceiling: number): FrameBar[] {
  const work = reply.work ?? [];
  const flags = reply.flags ?? [];
  return reply.samples.map((interval, i) => {
    const w = Math.min(work[i] ?? 0, interval);
    const f = flags[i] ?? 0;
    const paced = (f & FLAG_PACED) !== 0;
    const drew = (f & FLAG_DREW) !== 0;
    const throttled = (f & FLAG_THROTTLED) !== 0;
    const wait = Math.max(0, interval - w);
    const workPct = Math.min(100, (w / ceiling) * 100);
    const waitPct = Math.min(100 - workPct, (wait / ceiling) * 100);
    const parts = [
      `${ms(w)} ms work`,
      `${ms(wait)} ms waiting${paced ? " (idle pacing)" : ""}`,
      drew ? "drew" : "nothing to redraw",
    ];
    if (throttled) parts.push("a request held by the frame-rate limit");
    return {
      work: w,
      wait,
      paced,
      drew,
      throttled,
      cls: workClass(w, budgetMs),
      workPct,
      waitPct,
      title: parts.join(" · "),
    };
  });
}

export function share(value: number | undefined): string {
  return value === undefined ? "—" : `${Math.round(value * 100)} %`;
}

const CATEGORY_ORDER = ["js", "microtask", "dom", "style", "layout", "paint", "render", "native"];

export function categoryColor(category: string): string {
  const key = category.toLowerCase();
  return `var(--cat-${CATEGORY_ORDER.includes(key) || key === "wait" ? key : "native"})`;
}

/** A recorded frame as the timeline gets it: its interval, its CPU work and the work by category. */
export interface RecordedFrame {
  id: number;
  durationMs: number;
  cpuMs: number;
  categories: { category: string; selfMs: number }[];
}

export interface TraceSegment {
  category: string;
  color: string;
  pct: number;
}

export interface TraceBar {
  work: number;
  wait: number;
  cls: WorkClass;
  segments: TraceSegment[];
  workPct: number;
  waitPct: number;
  /** The wait ran past the chart's top. */
  clipped: boolean;
}

/** One overview bar: the frame's work stacked by category, then the wait until the next frame. */
export function traceBar(frame: RecordedFrame, budgetMs: number, ceiling: number): TraceBar {
  const work = Math.min(frame.cpuMs, frame.durationMs);
  const wait = Math.max(0, frame.durationMs - work);
  const workPct = Math.min(100, (work / ceiling) * 100);
  const waitFull = (wait / ceiling) * 100;
  const waitPct = Math.min(100 - workPct, waitFull);
  const parts = frame.categories
    .filter((c) => c.category !== "wait" && c.selfMs > 0)
    .sort((a, b) => rank(a.category) - rank(b.category));
  const total = parts.reduce((sum, c) => sum + c.selfMs, 0);
  const segments = total
    ? parts.map((c) => ({ category: c.category, color: categoryColor(c.category), pct: (c.selfMs / total) * workPct }))
    : [];
  return { work, wait, cls: workClass(work, budgetMs), segments, workPct, waitPct, clipped: waitFull > waitPct + 0.01 };
}

function rank(category: string): number {
  const at = CATEGORY_ORDER.indexOf(category.toLowerCase());
  return at < 0 ? CATEGORY_ORDER.length : at;
}

export interface TraceSummary {
  p50: number;
  p95: number;
  worst: number;
  overBudget: number;
  /** Of all the recorded time, the part spent waiting rather than working. */
  waitShare: number;
}

export function traceSummary(frames: RecordedFrame[], budgetMs: number): TraceSummary | null {
  if (!frames.length) return null;
  const work = frames.map((f) => Math.min(f.cpuMs, f.durationMs)).sort((a, b) => a - b);
  const at = (q: number) => work[Math.min(work.length - 1, Math.floor(q * work.length))]!;
  const span = frames.reduce((sum, f) => sum + f.durationMs, 0);
  const busy = work.reduce((sum, w) => sum + w, 0);
  return {
    p50: at(0.5),
    p95: at(0.95),
    worst: work[work.length - 1]!,
    overBudget: work.filter((w) => w > budgetMs).length,
    waitShare: span > 0 ? Math.max(0, 1 - busy / span) : 0,
  };
}
