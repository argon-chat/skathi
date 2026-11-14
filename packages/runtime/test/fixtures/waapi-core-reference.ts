// TEST ONLY: pre-native WAAPI numeric/normalization oracle. Never imported by src/.
// Native JSC conformance tests exercise the Rust implementation independently.
import type { WaapiCore, WaapiHost, WaapiEffectNotes } from "../../src/waapi";

type FillMode = "none" | "forwards" | "backwards" | "both" | "auto";
type PlaybackDirection = "normal" | "reverse" | "alternate" | "alternate-reverse";
type Composite = "replace" | "add" | "accumulate" | "auto";

const FILLS: readonly FillMode[] = ["none", "forwards", "backwards", "both", "auto"];
const DIRECTIONS: readonly PlaybackDirection[] = ["normal", "reverse", "alternate", "alternate-reverse"];
const COMPOSITES: readonly Composite[] = ["replace", "add", "accumulate", "auto"];

interface Timing {
  delay: number;
  endDelay: number;
  fill: FillMode;
  iterationStart: number;
  iterations: number;
  duration: number | "auto";
  direction: PlaybackDirection;
  easing: string;
}

// ---------------------------------------------------------------------------
// easing (CSS Easing Functions Level 1, minus `linear(…)`)
// ---------------------------------------------------------------------------

type EasingFn = (t: number) => number;

const NAMED_BEZIERS: Record<string, [number, number, number, number]> = {
  ease: [0.25, 0.1, 0.25, 1],
  "ease-in": [0.42, 0, 1, 1],
  "ease-out": [0, 0, 0.58, 1],
  "ease-in-out": [0.42, 0, 0.58, 1],
};

function cubicBezier(x1: number, y1: number, x2: number, y2: number): EasingFn {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const x = (t: number) => ((ax * t + bx) * t + cx) * t;
  const y = (t: number) => ((ay * t + by) * t + cy) * t;
  const dx = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  return (p) => {
    if (p <= 0) return 0;
    if (p >= 1) return 1;
    let t = p;
    for (let i = 0; i < 8; i++) {
      const err = x(t) - p;
      if (Math.abs(err) < 1e-7) return y(t);
      const d = dx(t);
      if (Math.abs(d) < 1e-6) break;
      t -= err / d;
    }
    let lo = 0;
    let hi = 1;
    t = p;
    for (let i = 0; i < 40; i++) {
      const v = x(t);
      if (Math.abs(v - p) < 1e-7) break;
      if (v < p) lo = t;
      else hi = t;
      t = (lo + hi) / 2;
    }
    return y(t);
  };
}

type StepPosition = "jump-start" | "jump-end" | "jump-none" | "jump-both";

function steps(count: number, position: StepPosition): EasingFn {
  return (t) => {
    let current = Math.floor(t * count);
    if (position === "jump-start" || position === "jump-both") current += 1;
    if (t >= 0 && current < 0) current = 0;
    const jumps = position === "jump-none" ? count - 1 : position === "jump-both" ? count + 1 : count;
    if (t <= 1 && current > jumps) current = jumps;
    return current / jumps;
  };
}

const NUM = "\\s*([+-]?(?:\\d+\\.?\\d*|\\.\\d+)(?:e[+-]?\\d+)?)\\s*";
const CUBIC_RE = new RegExp(`^cubic-bezier\\(${NUM},${NUM},${NUM},${NUM}\\)$`);
const STEPS_RE = /^steps\(\s*(\d+)\s*(?:,\s*(jump-start|jump-end|jump-none|jump-both|start|end)\s*)?\)$/;

/** An easing the host's cascade also takes, or null (a TypeError at the call site). */
export function parseEasing(text: string): EasingFn | null {
  const s = text.trim().toLowerCase();
  if (s === "linear") return (t) => t;
  const named = NAMED_BEZIERS[s];
  if (named) return cubicBezier(...named);
  if (s === "step-start") return steps(1, "jump-start");
  if (s === "step-end") return steps(1, "jump-end");
  let m = CUBIC_RE.exec(s);
  if (m) {
    const [x1, y1, x2, y2] = m.slice(1).map(Number) as [number, number, number, number];
    if (!(x1 >= 0 && x1 <= 1 && x2 >= 0 && x2 <= 1)) return null;
    return cubicBezier(x1, y1, x2, y2);
  }
  m = STEPS_RE.exec(s);
  if (m) {
    const count = Number(m[1]);
    const raw = m[2] ?? "end";
    const position = (raw === "start" ? "jump-start" : raw === "end" ? "jump-end" : raw) as StepPosition;
    if (count < 1 || (position === "jump-none" && count < 2)) return null;
    return steps(count, position);
  }
  return null;
}

// ---------------------------------------------------------------------------
// the timing model (Web Animations 1 §4.6–4.10) — the same arithmetic as
// crates/sk-css/src/animator/script.rs, which paints from it
// ---------------------------------------------------------------------------

interface Resolved {
  delay: number;
  endDelay: number;
  fill: Exclude<FillMode, "auto">;
  iterationStart: number;
  iterations: number;
  duration: number;
  direction: PlaybackDirection;
  easing: EasingFn;
}

function resolveTiming(t: Timing): Resolved {
  return {
    delay: t.delay,
    endDelay: t.endDelay,
    fill: t.fill === "auto" ? "none" : t.fill,
    iterationStart: t.iterationStart,
    iterations: t.iterations,
    duration: t.duration === "auto" ? 0 : t.duration,
    direction: t.direction,
    easing: parseEasing(t.easing) ?? ((x) => x),
  };
}

const activeDuration = (r: Resolved): number => (r.duration === 0 || r.iterations === 0 ? 0 : r.duration * r.iterations);
const endTimeOf = (r: Resolved): number => Math.max(r.delay + activeDuration(r) + r.endDelay, 0);

interface Sample {
  progress: number | null;
  currentIteration: number | null;
}

/** The (eased) iteration progress at `local`, or null progress when the effect is not in effect. */
function sample(r: Resolved, local: number | null, backwards: boolean): Sample {
  if (local === null) return { progress: null, currentIteration: null };
  const active = activeDuration(r);
  const end = endTimeOf(r);
  const beforeActive = Math.max(Math.min(r.delay, end), 0);
  const activeAfter = Math.max(Math.min(r.delay + active, end), 0);
  const phase =
    local < beforeActive || (backwards && local === beforeActive)
      ? "before"
      : local > activeAfter || (!backwards && local === activeAfter)
        ? "after"
        : "active";
  let activeTime: number | null;
  if (phase === "before") activeTime = r.fill === "backwards" || r.fill === "both" ? Math.max(local - r.delay, 0) : null;
  else if (phase === "active") activeTime = local - r.delay;
  else activeTime = r.fill === "forwards" || r.fill === "both" ? Math.max(Math.min(local - r.delay, active), 0) : null;
  if (activeTime === null) return { progress: null, currentIteration: null };
  const overall =
    r.duration === 0
      ? phase === "before"
        ? r.iterationStart
        : r.iterationStart + r.iterations
      : activeTime / r.duration + r.iterationStart;
  let simple = Number.isFinite(overall) ? overall % 1 : r.iterationStart % 1;
  if (simple === 0 && phase !== "before" && activeTime === active && r.iterations !== 0) simple = 1;
  const iteration =
    phase === "after" && r.iterations === Infinity ? Infinity : simple === 1 ? Math.floor(overall) - 1 : Math.floor(overall);
  let forwards = true;
  if (r.direction === "reverse") forwards = false;
  else if (r.direction === "alternate" || r.direction === "alternate-reverse") {
    const d = r.direction === "alternate-reverse" ? iteration + 1 : iteration;
    forwards = !Number.isFinite(d) || d % 2 === 0;
  }
  const directed = forwards ? simple : 1 - simple;
  return { progress: r.easing(directed), currentIteration: iteration };
}

// ---------------------------------------------------------------------------
// keyframes (Web Animations 1 §6.6.3, "processing a keyframes argument")
// ---------------------------------------------------------------------------

interface Frame {
  /** As the script specified it (`null` = to be computed). */
  offset: number | null;
  computed: number;
  easing: string;
  composite: Composite;
  /** IDL name → value, in the order given. */
  props: Map<string, string>;
}

const RESERVED = new Set(["offset", "easing", "composite"]);

/** The CSS name of a keyframe member, or null for one WAAPI ignores (a hyphenated name). */
function cssName(idl: string): string | null {
  if (idl.startsWith("--")) return idl;
  if (idl === "cssFloat") return "float";
  if (idl === "cssOffset") return "offset";
  if (idl.includes("-")) return null;
  const h = idl.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
  return h.startsWith("webkit-") ? "-" + h : h;
}

function offsetOf(v: unknown, where: string): number | null {
  if (v === undefined || v === null) return null;
  const n = Number(v);
  if (!Number.isFinite(n)) throw new TypeError(`Failed to execute '${where}': Non-finite offset.`);
  return n;
}

function easingOf(v: unknown, where: string): string {
  if (v === undefined) return "linear";
  const s = String(v);
  if (!parseEasing(s)) throw new TypeError(`Failed to execute '${where}': '${s}' is not a valid value for easing`);
  return s.trim();
}

function compositeOf(v: unknown, where: string): Composite {
  if (v === undefined || v === null) return "auto";
  const s = String(v) as Composite;
  if (!COMPOSITES.includes(s)) throw new TypeError(`Failed to execute '${where}': '${s}' is not a valid value for composite`);
  return s;
}

/** Fills `computed` for the keyframes whose offset is null: §6.6.2, "compute missing keyframe offsets". */
function computeMissingOffsets(frames: Frame[]): void {
  if (frames.length === 0) return;
  for (const f of frames) f.computed = f.offset ?? NaN;
  if (frames.length > 1 && frames[0]!.offset === null) frames[0]!.computed = 0;
  const last = frames[frames.length - 1]!;
  if (last.offset === null) last.computed = 1;
  let prev = 0;
  for (let i = 1; i < frames.length; i++) {
    if (Number.isNaN(frames[i]!.computed)) continue;
    const gap = i - prev;
    for (let j = prev + 1; j < i; j++) {
      frames[j]!.computed = frames[prev]!.computed + ((frames[i]!.computed - frames[prev]!.computed) * (j - prev)) / gap;
    }
    prev = i;
  }
}

function checkOffsets(frames: Frame[], where: string): void {
  let last = -Infinity;
  for (const f of frames) {
    if (f.offset === null) continue;
    if (f.offset < 0 || f.offset > 1) throw new TypeError(`Failed to execute '${where}': Offsets must be null or in the range [0,1].`);
    if (f.offset < last) throw new TypeError(`Failed to execute '${where}': Offsets must be monotonically non-decreasing.`);
    last = f.offset;
  }
}

function processKeyframes(input: unknown, where: string): Frame[] {
  if (input === undefined || input === null) return [];
  if (typeof input !== "object" && typeof input !== "function") {
    throw new TypeError(`Failed to execute '${where}': The provided value is not of type '(sequence<object> or object)'.`);
  }
  const source = input as Record<string | symbol, unknown>;
  if (typeof source[Symbol.iterator] === "function") {
    const frames: Frame[] = [];
    for (const kf of source as unknown as Iterable<unknown>) {
      if (kf !== undefined && kf !== null && typeof kf !== "object") {
        throw new TypeError(`Failed to execute '${where}': Keyframes must be objects.`);
      }
      const o = (kf ?? {}) as Record<string, unknown>;
      const props = new Map<string, string>();
      for (const key of Object.keys(o)) if (!RESERVED.has(key)) props.set(key, String(o[key]));
      frames.push({
        offset: offsetOf(o.offset, where),
        computed: 0,
        easing: easingOf(o.easing, where),
        composite: compositeOf(o.composite, where),
        props,
      });
    }
    checkOffsets(frames, where);
    computeMissingOffsets(frames);
    return frames;
  }
  // The property-indexed form: `{ opacity: [0, 1], transform: [...], offset?, easing? }`.
  const merged = new Map<number, Frame>();
  for (const key of Object.keys(source)) {
    if (RESERVED.has(key)) continue;
    const raw = source[key];
    if (raw === undefined) continue;
    const values = Array.isArray(raw) ? raw.map(String) : [String(raw)];
    values.forEach((value, i) => {
      // A single value is the end keyframe; the start is the underlying value.
      const at = values.length === 1 ? 1 : i / (values.length - 1);
      let f = merged.get(at);
      if (!f) merged.set(at, (f = { offset: null, computed: at, easing: "linear", composite: "auto", props: new Map() }));
      f.props.set(key, value);
    });
  }
  const frames = [...merged.values()].sort((a, b) => a.computed - b.computed);
  const list = <T>(v: unknown): T[] => (v === undefined ? [] : Array.isArray(v) ? (v as T[]) : [v as T]);
  list<unknown>(source.offset).forEach((v, i) => {
    if (i < frames.length) frames[i]!.offset = offsetOf(v, where);
  });
  const easings = list<unknown>(source.easing).map((v) => easingOf(v, where));
  if (easings.length) frames.forEach((f, i) => (f.easing = easings[i % easings.length]!));
  const composites = list<unknown>(source.composite).map((v) => compositeOf(v, where));
  if (composites.length) frames.forEach((f, i) => (f.composite = composites[i % composites.length]!));
  checkOffsets(frames, where);
  for (const f of frames) if (f.offset !== null) f.computed = f.offset;
  return frames;
}

// ---------------------------------------------------------------------------
// timing (Web Animations 1 §6.5, `EffectTiming` / `OptionalEffectTiming`)
// ---------------------------------------------------------------------------

function processTiming(base: Timing, input: unknown, where: string): Timing {
  const t: Timing = { ...base };
  if (input === undefined || input === null) return t;
  if (typeof input === "number") {
    if (Number.isNaN(input) || input < 0) throw new TypeError(`Failed to execute '${where}': duration must be non-negative or auto.`);
    t.duration = input;
    return t;
  }
  if (typeof input !== "object") return t;
  const o = input as Record<string, unknown>;
  const finite = (v: unknown, name: string): number => {
    const n = Number(v);
    if (!Number.isFinite(n)) throw new TypeError(`Failed to execute '${where}': ${name} must be a finite number.`);
    return n;
  };
  if (o.delay !== undefined) t.delay = finite(o.delay, "delay");
  if (o.endDelay !== undefined) t.endDelay = finite(o.endDelay, "endDelay");
  if (o.fill !== undefined) {
    const f = String(o.fill) as FillMode;
    if (!FILLS.includes(f)) throw new TypeError(`Failed to execute '${where}': '${f}' is not a valid value for fill.`);
    t.fill = f;
  }
  if (o.iterationStart !== undefined) {
    const n = finite(o.iterationStart, "iterationStart");
    if (n < 0) throw new TypeError(`Failed to execute '${where}': iterationStart must be non-negative.`);
    t.iterationStart = n;
  }
  if (o.iterations !== undefined) {
    const n = Number(o.iterations);
    if (Number.isNaN(n) || n < 0) throw new TypeError(`Failed to execute '${where}': iterations must be non-negative.`);
    t.iterations = n;
  }
  if (o.duration !== undefined) {
    if (typeof o.duration === "string") {
      if (o.duration !== "auto") throw new TypeError(`Failed to execute '${where}': duration must be non-negative or auto.`);
      t.duration = "auto";
    } else {
      const n = Number(o.duration);
      if (Number.isNaN(n) || n < 0) throw new TypeError(`Failed to execute '${where}': duration must be non-negative or auto.`);
      t.duration = n;
    }
  }
  if (o.direction !== undefined) {
    const d = String(o.direction) as PlaybackDirection;
    if (!DIRECTIONS.includes(d)) throw new TypeError(`Failed to execute '${where}': '${d}' is not a valid value for direction.`);
    t.direction = d;
  }
  if (o.easing !== undefined) t.easing = easingOf(o.easing, where);
  return t;
}

// ---------------------------------------------------------------------------
// the install
// ---------------------------------------------------------------------------

export function createReferenceWaapiCore(): WaapiCore {
  return {
    keyframes(input, where) {
      return processKeyframes(input, where).map((frame) => ({ ...frame, props: [...frame.props] }));
    },
    timing: processTiming,
    computed(timing, local, backwards) {
      const resolved = resolveTiming(timing), result = sample(resolved, local, backwards);
      return { fill: resolved.fill, duration: resolved.duration,
        endTime: endTimeOf(resolved), activeDuration: activeDuration(resolved),
        localTime: local, progress: result.progress, currentIteration: result.currentIteration };
    },
    endTime: (timing) => endTimeOf(resolveTiming(timing)),
    inEffect: (timing, local, backwards) => sample(resolveTiming(timing), local, backwards).progress !== null,
    validEasing: (text) => parseEasing(text) !== null,
    evaluateEasing: (text, progress) => parseEasing(text)?.(progress) ?? NaN,
  };
}

/** Test host projection of the old painter boundary; no production string wire. */
export function referenceEffect(frames: Parameters<WaapiHost["setEffect"]>[2], notes = "") {
  const hyphenated = new Set<string>();
  const paint = frames.map((frame) => ({ ...frame, props: frame.props.flatMap(([name, value]) => {
    const css = cssName(name);
    if (css === null) { hyphenated.add(name); return []; }
    return [[css, value] as [string, string]];
  }) }));
  const result: WaapiEffectNotes = { accepted: true, error: null, ignored: [], diagnostics: [], hyphenated: [...hyphenated] };
  for (const line of notes.split("\n")) {
    if (line[0] === "E") { result.accepted = false; result.error = line.slice(1); }
    if (line[0] === "W") result.ignored.push(line.slice(1));
    if (line[0] === "D") result.diagnostics.push(line.slice(1));
  }
  return { frames: paint, notes: result };
}
export function referenceValues(value: string | null): [string, string][] | null {
  return value === null ? null : value.split("\u001c").map((pair) => pair.split("\u001d") as [string, string]);
}
