// Web Animations: `Element.animate()`, `Animation`, `KeyframeEffect` and
// `document.timeline` (Web Animations Level 1) — the layout/CSS stream's module.
//
// # The split
//
// This module is the object model and the WHOLE state machine: play, pause,
// seek, reverse, finish, the playback rate, the pending tasks that resolve at
// the next frame, the `ready` / `finished` promises, the `finish` / `cancel` /
// `remove` events and the automatic removal of replaced animations. What it
// tells the host (crates/vn-tree, `__vn_anim_*`) is only what painting needs:
//
// * the effect — keyframes and timing — when it is created or changed;
// * the clock — start time on the document timeline, or a held current time,
//   and the playback rate — whenever the state machine moves it.
//
// The host's animator (crates/vn-css/src/animator/script.rs) samples the
// effect every frame from that clock, through the same timing model this file
// implements for `getComputedTiming()`. Nothing crosses per frame, so a roll
// that runs for a second costs JS two calls and a timer.
//
// # Where it differs from a browser (docs/RUNTIME.md §5.23)
//
// * `animate` and `getAnimations` are installed on HTMLElement.prototype and
//   SVGElement.prototype, NOT Element.prototype. motion / framer-motion decide
//   between their own JS animations and WAAPI with
//   `Object.hasOwnProperty.call(Element.prototype, "animate")`, and their JS
//   path is the proven one here; app code calling `el.animate()` on an element
//   works either way. One advisory, on the first call.
// * `document.timeline.currentTime` is `performance.now()` read live, not a
//   value frozen for the frame.
// * Composite `add` / `accumulate`, pseudo-element targets, other timelines,
//   `linear(…)` easing (refused, as by a browser without it: `CSS.supports`
//   answers no too) and properties the animator cannot interpolate
//   (crates/vn-css `TransitionProperty::animatable`) are advised once each;
//   the timing, the promises and the events still run.
// * `getAnimations()` lists script animations only: CSS animations and
//   transitions run natively and have no `CSSAnimation` objects.
//
// runtime.ts calls `installWaapi(…)` once; without the host functions (an
// older host, the bun harness) nothing is installed and `animate` stays
// undefined, as it was.

/** The host's half (crates/vn-tree/src/lib.rs, "script animations"). */
export interface WaapiHost {
  /** Animation `id`'s effect on `node`. Answers notes, one per line: `E` refused, `W` property ignored, `D` diagnostic. */
  set(id: number, node: number, frames: string, timing: string, easing: string): string;
  /** The clock: start time (timeline ms), hold time, playback rate. NaN is unresolved. */
  clock(id: number, start: number, hold: number, rate: number): void;
  drop(id: number): void;
  /** `commitStyles()`: `name ␝ value` pairs joined by U+001C, or null. */
  values(id: number): string | null;
}

export interface WaapiDeps {
  host: WaapiHost;
  /** Where `animate` / `getAnimations` go: HTMLElement.prototype, SVGElement.prototype. */
  protos: object[];
  /** The host node a target paints as, or 0 for anything the host does not render. */
  nodeOf(target: object): number;
  /** `target.style.setProperty(name, value)`, for `commitStyles()`. */
  setStyle(target: object, name: string, value: string): void;
  /** Is `node` inside `root` (for `getAnimations({ subtree: true })`)? */
  contains(root: object, node: object): boolean;
  warn(key: string, message: string): void;
  /** `performance.now()`. */
  now(): number;
  /** `requestAnimationFrame`: a pending play or pause resolves at the next frame. */
  frame(cb: (timestamp: number) => void): void;
  setTimer(cb: () => void, ms: number): unknown;
  clearTimer(handle: unknown): void;
  defineGlobal(name: string, value: unknown): void;
  doc: Record<string, unknown> | null;
  EventTarget?: new () => object;
  Event?: new (type: string, init?: object) => object;
  DOMException?: new (message: string, name: string) => Error;
}

type FillMode = "none" | "forwards" | "backwards" | "both" | "auto";
type PlaybackDirection = "normal" | "reverse" | "alternate" | "alternate-reverse";
type PlayState = "idle" | "running" | "paused" | "finished";
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

const DEFAULT_TIMING: Timing = {
  delay: 0,
  endDelay: 0,
  fill: "auto",
  iterationStart: 0,
  iterations: 1,
  duration: "auto",
  direction: "normal",
  easing: "linear",
};

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
    const [x1, y1, x2, y2] = m.slice(1).map(Number);
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
// crates/vn-css/src/animator/script.rs, which paints from it
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
  if (frames.length > 1 && frames[0].offset === null) frames[0].computed = 0;
  const last = frames[frames.length - 1];
  if (last.offset === null) last.computed = 1;
  let prev = 0;
  for (let i = 1; i < frames.length; i++) {
    if (Number.isNaN(frames[i].computed)) continue;
    const gap = i - prev;
    for (let j = prev + 1; j < i; j++) {
      frames[j].computed = frames[prev].computed + ((frames[i].computed - frames[prev].computed) * (j - prev)) / gap;
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
    if (i < frames.length) frames[i].offset = offsetOf(v, where);
  });
  const easings = list<unknown>(source.easing).map((v) => easingOf(v, where));
  if (easings.length) frames.forEach((f, i) => (f.easing = easings[i % easings.length]));
  const composites = list<unknown>(source.composite).map((v) => compositeOf(v, where));
  if (composites.length) frames.forEach((f, i) => (f.composite = composites[i % composites.length]));
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

type Handler = ((this: unknown, ev: unknown) => unknown) | null;

interface Deferred<T> {
  promise: Promise<T>;
  resolve(v: T): void;
  reject(e: unknown): void;
  settled: boolean;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((a, b) => {
    resolve = a;
    reject = b;
  });
  const d: Deferred<T> = {
    promise,
    settled: false,
    resolve(v) {
      if (d.settled) return;
      d.settled = true;
      resolve(v);
    },
    reject(e) {
      if (d.settled) return;
      d.settled = true;
      // A rejected `ready` / `finished` nobody awaits is not an unhandled
      // rejection: the DOM marks them handled.
      promise.catch(() => {});
      reject(e);
    },
  };
  return d;
}

const microtask = (f: () => void): void => {
  if (typeof queueMicrotask === "function") queueMicrotask(f);
  else void Promise.resolve().then(f);
};

export function installWaapi(d: WaapiDeps): { Animation: unknown; KeyframeEffect: unknown; timeline: unknown } {
  const host = d.host;
  const domError = (message: string, name: string): Error => {
    if (typeof d.DOMException === "function") return new d.DOMException(message, name);
    const e = new Error(message);
    e.name = name;
    return e;
  };
  const EventBase = (d.Event ?? class {
    type: string;
    constructor(type: string) {
      this.type = type;
    }
  }) as new (type: string, init?: object) => object;
  const TargetBase = (d.EventTarget ?? class {}) as new () => {
    dispatchEvent?(ev: object): boolean;
  };

  /** Animations that paint or may still: what `getAnimations()` lists. Creation order is composite order. */
  const live = new Set<AnimationImpl>();
  let nextId = 0;

  // -- the timeline --------------------------------------------------------

  class AnimationTimelineImpl {
    get currentTime(): number | null {
      return d.now();
    }
    get duration(): number | null {
      return null;
    }
  }
  class DocumentTimelineImpl extends AnimationTimelineImpl {
    #origin: number;
    constructor(options?: { originTime?: number }) {
      super();
      this.#origin = Number(options?.originTime ?? 0) || 0;
    }
    override get currentTime(): number | null {
      return d.now() - this.#origin;
    }
  }
  const timeline = new DocumentTimelineImpl();

  // -- the effect ----------------------------------------------------------

  class AnimationEffectImpl {
    /** @internal */ _timing: Timing = { ...DEFAULT_TIMING };
    /** @internal */ _animation: AnimationImpl | null = null;

    getTiming(): Timing {
      return { ...this._timing };
    }

    getComputedTiming(): Record<string, unknown> {
      const r = resolveTiming(this._timing);
      const anim = this._animation;
      const local = anim ? anim.currentTime : null;
      const s = sample(r, local, (anim?.playbackRate ?? 1) < 0);
      return {
        ...this._timing,
        fill: r.fill,
        duration: r.duration,
        endTime: endTimeOf(r),
        activeDuration: activeDuration(r),
        localTime: local,
        progress: s.progress,
        currentIteration: s.currentIteration,
      };
    }

    updateTiming(timing?: unknown): void {
      this._timing = processTiming(this._timing, timing ?? {}, "updateTiming");
      this._changed();
    }

    /** @internal */ _end(): number {
      return endTimeOf(resolveTiming(this._timing));
    }
    /** @internal */ _inEffect(local: number | null, backwards: boolean): boolean {
      return sample(resolveTiming(this._timing), local, backwards).progress !== null;
    }
    /** @internal */ _changed(): void {
      this._animation?._effectChanged();
    }
    /** @internal */ _push(_id: number): boolean {
      return false;
    }
    /** @internal */ _target(): object | null {
      return null;
    }
    /** @internal */ _properties(): Set<string> {
      return new Set();
    }
  }

  let advisedPlacement = false;

  class KeyframeEffectImpl extends AnimationEffectImpl {
    #target: object | null;
    #frames: Frame[] = [];
    #composite: Composite = "replace";
    #pseudo: string | null = null;

    constructor(target: unknown, keyframes?: unknown, options?: unknown) {
      super();
      if (target instanceof KeyframeEffectImpl) {
        // `new KeyframeEffect(source)`: a copy.
        this.#target = target.#target;
        this.#frames = target.#frames.map((f) => ({ ...f, props: new Map(f.props) }));
        this._timing = { ...target._timing };
        this.#composite = target.#composite;
        return;
      }
      this.#target = target === null || target === undefined ? null : (target as object);
      this.#frames = processKeyframes(keyframes, "KeyframeEffect");
      this._timing = processTiming(DEFAULT_TIMING, options, "KeyframeEffect");
      if (options && typeof options === "object") {
        const o = options as Record<string, unknown>;
        if (o.composite !== undefined) this.composite = o.composite as Composite;
        if (o.pseudoElement !== undefined && o.pseudoElement !== null) this.pseudoElement = String(o.pseudoElement);
        if (o.iterationComposite !== undefined && o.iterationComposite !== "replace") {
          d.warn("waapi:iterationComposite", "Element.animate(): iterationComposite other than 'replace' is not supported — iterations do not accumulate");
        }
      }
      this.#adviseComposites();
    }

    get target(): object | null {
      return this.#target;
    }
    set target(v: object | null) {
      this.#target = v ?? null;
      this._changed();
    }
    get pseudoElement(): string | null {
      return this.#pseudo;
    }
    set pseudoElement(v: string | null) {
      this.#pseudo = v === null || v === undefined ? null : String(v);
      if (this.#pseudo !== null) {
        d.warn("waapi:pseudo", `Element.animate(): a pseudo-element target (${this.#pseudo}) cannot be animated here — the timing and events run, nothing is painted`);
      }
      this._changed();
    }
    get composite(): Composite {
      return this.#composite;
    }
    set composite(v: Composite) {
      const c = String(v) as Composite;
      if (!COMPOSITES.includes(c) || c === "auto") return;
      this.#composite = c;
      this.#adviseComposites();
      this._changed();
    }
    get iterationComposite(): string {
      return "replace";
    }

    getKeyframes(): Record<string, unknown>[] {
      return this.#frames.map((f) => {
        const out: Record<string, unknown> = { offset: f.offset, computedOffset: f.computed, easing: f.easing, composite: f.composite };
        for (const [k, v] of f.props) out[k] = v;
        return out;
      });
    }

    setKeyframes(keyframes: unknown): void {
      this.#frames = processKeyframes(keyframes, "setKeyframes");
      this.#adviseComposites();
      this._changed();
    }

    #adviseComposites(): void {
      const odd = this.#composite !== "replace" || this.#frames.some((f) => f.composite === "add" || f.composite === "accumulate");
      if (odd) {
        d.warn("waapi:composite", "Element.animate(): composite 'add' / 'accumulate' is not supported — the keyframes replace the underlying value");
      }
    }

    override _target(): object | null {
      return this.#target;
    }

    override _properties(): Set<string> {
      const out = new Set<string>();
      for (const f of this.#frames) for (const k of f.props.keys()) out.add(k);
      return out;
    }

    /** Hands the effect to the host as animation `id`. False when there is nothing it can paint. */
    override _push(id: number): boolean {
      const target = this.#target;
      if (!target || this.#pseudo !== null) return false;
      const node = d.nodeOf(target);
      if (!node) {
        d.warn("waapi:target", "Element.animate() on an element vue-native does not render (not a mirror node) runs its timing and events but paints nothing");
        return false;
      }
      const frames = this.#frames
        .map((f) => {
          const decls: string[] = [];
          for (const [idl, value] of f.props) {
            const css = cssName(idl);
            if (css === null) {
              d.warn(`waapi:hyphen:${idl}`, `Element.animate(): the keyframe member '${idl}' is ignored, as in a browser — keyframes take camelCase names ('${idl.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase())}')`);
              continue;
            }
            decls.push(`${css}\u001d${value}`);
          }
          return `${f.computed}\u001f${f.easing}\u001f${decls.join("\u001c")}`;
        })
        .join("\u001e");
      const r = resolveTiming(this._timing);
      const timing = [
        r.delay,
        r.endDelay,
        r.duration,
        r.iterations,
        r.iterationStart,
        DIRECTIONS.indexOf(r.direction),
        ["none", "forwards", "backwards", "both"].indexOf(r.fill),
      ].join(",");
      const notes = host.set(id, node, frames, timing, this._timing.easing);
      let ok = true;
      for (const line of notes ? notes.split("\n") : []) {
        const kind = line[0];
        const text = line.slice(1);
        if (kind === "E") {
          ok = false;
          d.warn(`waapi:refused:${text}`, `Element.animate(): the host refused the effect (${text}) — nothing is painted`);
        } else if (kind === "W") {
          d.warn(
            `waapi:property:${text}`,
            text.startsWith("--")
              ? `Element.animate(): custom property '${text}' is not animated here — the rest of the keyframes still are`
              : `Element.animate(): '${text}' is not animatable here (the animator's set is the transition one) — the rest of the keyframes still animate`,
          );
        } else if (kind === "D") {
          d.warn(`waapi:diag:${text}`, `Element.animate(): ${text}`);
        }
      }
      return ok;
    }
  }

  // -- the animation -------------------------------------------------------

  class AnimationPlaybackEventImpl extends EventBase {
    declare readonly currentTime: number | null;
    declare readonly timelineTime: number | null;
    constructor(type: string, init?: { currentTime?: number | null; timelineTime?: number | null }) {
      super(type, init);
      Object.defineProperty(this, "currentTime", { value: init?.currentTime ?? null, enumerable: true });
      Object.defineProperty(this, "timelineTime", { value: init?.timelineTime ?? null, enumerable: true });
    }
  }

  class AnimationImpl extends TargetBase {
    id = "";
    onfinish: Handler = null;
    oncancel: Handler = null;
    onremove: Handler = null;
    #nid = ++nextId;
    #effect: AnimationEffectImpl | null = null;
    #timeline: AnimationTimelineImpl | null;
    #start: number | null = null;
    #hold: number | null = null;
    #rate = 1;
    #pendingRate: number | null = null;
    #pending: "play" | "pause" | null = null;
    #token = 0;
    #prev: number | null = null;
    #ready: Deferred<AnimationImpl>;
    #finished: Deferred<AnimationImpl> = deferred();
    #notifyQueued = false;
    #replace: "active" | "removed" | "persisted" = "active";
    #timer: unknown = null;
    #native = false;
    #pushed: string | null = null;

    constructor(effect?: unknown, tl?: unknown) {
      super();
      this.#ready = deferred();
      this.#ready.resolve(this);
      this.#timeline = tl === undefined ? timeline : (tl as AnimationTimelineImpl | null);
      if (tl !== undefined && tl !== null && tl !== timeline) {
        d.warn("waapi:timeline", "new Animation(effect, timeline): only document.timeline drives animations here — the animation runs on it");
        this.#timeline = timeline;
      }
      if (effect !== undefined && effect !== null) this.effect = effect as AnimationEffectImpl;
    }

    // -- the model's quantities --------------------------------------------

    #timelineTime(): number | null {
      return this.#timeline ? this.#timeline.currentTime : null;
    }
    #current(): number | null {
      if (this.#hold !== null) return this.#hold;
      const t = this.#timelineTime();
      if (t === null || this.#start === null) return null;
      return (t - this.#start) * this.#rate;
    }
    #end(): number {
      return this.#effect ? this.#effect._end() : 0;
    }
    #effectiveRate(): number {
      return this.#pendingRate ?? this.#rate;
    }
    #applyPendingRate(): void {
      if (this.#pendingRate === null) return;
      this.#rate = this.#pendingRate;
      this.#pendingRate = null;
    }
    #cancelPending(): void {
      this.#pending = null;
      this.#token++;
    }

    // -- the IDL -------------------------------------------------------------

    get effect(): AnimationEffectImpl | null {
      return this.#effect;
    }
    set effect(e: AnimationEffectImpl | null) {
      if (e === this.#effect) return;
      if (e !== null && !(e instanceof AnimationEffectImpl)) throw new TypeError("Failed to set the 'effect' property on 'Animation': The provided value is not of type 'AnimationEffect'.");
      if (this.#native) {
        host.drop(this.#nid);
        this.#native = false;
      }
      if (this.#effect) this.#effect._animation = null;
      if (e?._animation) e._animation.effect = null;
      this.#effect = e;
      if (e) e._animation = this;
      this.#updateFinished(false, false);
      this.#sync();
    }

    get timeline(): AnimationTimelineImpl | null {
      return this.#timeline;
    }
    set timeline(v: AnimationTimelineImpl | null) {
      if (v === this.#timeline) return;
      d.warn("waapi:timeline", "Animation.timeline: only document.timeline drives animations here — the animation stays on it");
    }

    get startTime(): number | null {
      return this.#start;
    }
    set startTime(v: number | null) {
      const t = v === null || v === undefined ? null : Number(v);
      const tl = this.#timelineTime();
      if (tl === null && t !== null) this.#hold = null;
      const previous = this.#current();
      this.#applyPendingRate();
      this.#start = t;
      if (t !== null) {
        if (this.#rate !== 0) this.#hold = null;
      } else this.#hold = previous;
      if (this.#pending) {
        this.#cancelPending();
        this.#ready.resolve(this);
      }
      this.#updateFinished(true, false);
      this.#sync();
    }

    get currentTime(): number | null {
      return this.#current();
    }
    set currentTime(v: number | null) {
      const seek = v === null || v === undefined ? null : Number(v);
      if (seek !== null && Number.isNaN(seek)) throw new TypeError("Failed to set the 'currentTime' property on 'Animation': The provided double value is non-finite.");
      this.#silentSeek(seek);
      if (this.#pending === "pause") {
        this.#hold = seek;
        this.#applyPendingRate();
        this.#start = null;
        this.#cancelPending();
        this.#ready.resolve(this);
      }
      this.#updateFinished(true, false);
      this.#sync();
    }

    get playbackRate(): number {
      return this.#rate;
    }
    set playbackRate(v: number) {
      this.#pendingRate = null;
      const previous = this.#current();
      this.#rate = Number(v);
      if (previous !== null) this.currentTime = previous;
      else this.#sync();
    }

    get playState(): PlayState {
      const cur = this.#current();
      if (cur === null && this.#pending === null) return "idle";
      if (this.#pending === "pause" || (this.#start === null && this.#pending !== "play")) return "paused";
      const rate = this.#effectiveRate();
      if (cur !== null && ((rate > 0 && cur >= this.#end()) || (rate < 0 && cur <= 0))) return "finished";
      return "running";
    }
    get pending(): boolean {
      return this.#pending !== null;
    }
    get replaceState(): string {
      return this.#replace;
    }
    get ready(): Promise<AnimationImpl> {
      return this.#ready.promise;
    }
    get finished(): Promise<AnimationImpl> {
      return this.#finished.promise;
    }

    play(): void {
      this.#play(true);
    }

    pause(): void {
      if (this.#pending === "pause" || this.playState === "paused") return;
      let seek: number | null = null;
      if (this.#current() === null) {
        if (this.#rate >= 0) seek = 0;
        else {
          if (this.#end() === Infinity) throw domError("Cannot pause an animation with an infinite end, playing backwards.", "InvalidStateError");
          seek = this.#end();
        }
      }
      if (seek !== null) this.#hold = seek;
      let hadPendingReady = false;
      if (this.#pending === "play") {
        this.#cancelPending();
        hadPendingReady = true;
      }
      if (!hadPendingReady) this.#ready = deferred();
      this.#pending = "pause";
      this.#schedulePending();
      this.#updateFinished(false, false);
      this.#sync();
    }

    finish(): void {
      const rate = this.#effectiveRate();
      const end = this.#end();
      if (rate === 0 || (rate > 0 && end === Infinity)) {
        throw domError("Cannot finish an animation with a playback rate of zero, or an infinite end.", "InvalidStateError");
      }
      this.#applyPendingRate();
      const limit = this.#rate > 0 ? end : 0;
      this.#silentSeek(limit);
      const tl = this.#timelineTime();
      if (this.#start === null && tl !== null) this.#start = tl - limit / this.#rate;
      if (this.#pending === "pause" && this.#start !== null) {
        this.#hold = null;
        this.#cancelPending();
        this.#ready.resolve(this);
      }
      if (this.#pending === "play" && this.#start !== null) {
        this.#cancelPending();
        this.#ready.resolve(this);
      }
      this.#updateFinished(true, true);
      this.#sync();
    }

    cancel(): void {
      if (this.playState !== "idle") {
        if (this.#pending) {
          this.#cancelPending();
          this.#applyPendingRate();
          this.#ready.reject(domError("The user aborted a request.", "AbortError"));
          this.#ready = deferred();
          this.#ready.resolve(this);
        }
        this.#finished.reject(domError("The user aborted a request.", "AbortError"));
        this.#finished = deferred();
        const tl = this.#timelineTime();
        microtask(() => this.#fire("cancel", null, tl));
      }
      this.#hold = null;
      this.#start = null;
      this.#sync();
    }

    reverse(): void {
      if (this.#timelineTime() === null) throw domError("Cannot reverse an animation with no active timeline.", "InvalidStateError");
      const original = this.#pendingRate;
      this.#pendingRate = -this.#effectiveRate();
      try {
        this.#play(true);
      } catch (e) {
        this.#pendingRate = original;
        throw e;
      }
    }

    updatePlaybackRate(rate: number): void {
      const previous = this.playState;
      this.#pendingRate = Number(rate);
      if (this.#pending) return;
      if (previous === "idle" || previous === "paused") {
        this.#applyPendingRate();
        this.#sync();
      } else if (previous === "finished") {
        const tl = this.#timelineTime();
        const unconstrained = tl !== null && this.#start !== null ? (tl - this.#start) * this.#rate : null;
        if (unconstrained !== null && tl !== null) {
          this.#start = this.#pendingRate !== 0 ? tl - unconstrained / this.#pendingRate : tl;
        }
        this.#applyPendingRate();
        this.#updateFinished(false, false);
        this.#sync();
      } else {
        this.#play(false);
      }
    }

    persist(): void {
      this.#replace = "persisted";
    }

    commitStyles(): void {
      const target = this.#effect?._target();
      if (!target) throw domError("Animation.commitStyles(): the effect has no target element.", "NoModificationAllowedError");
      if (!this.#native) return;
      const raw = host.values(this.#nid);
      if (!raw) return;
      for (const pair of raw.split("\u001c")) {
        const [name, value] = pair.split("\u001d");
        if (name && value !== undefined) d.setStyle(target, name, value);
      }
    }

    // -- the procedures ------------------------------------------------------

    #silentSeek(seek: number | null): void {
      if (seek === null) {
        if (this.#current() !== null) throw new TypeError("Failed to set the 'currentTime' property on 'Animation': currentTime may not be changed from resolved to unresolved.");
        return;
      }
      const tl = this.#timelineTime();
      if (this.#hold !== null || this.#start === null || tl === null || this.#rate === 0) this.#hold = seek;
      else this.#start = tl - seek / this.#rate;
      if (tl === null) this.#start = null;
      this.#prev = null;
    }

    #play(autoRewind: boolean): void {
      const abortedPause = this.#pending === "pause";
      let hadPendingReady = false;
      let seek: number | null = null;
      const rate = this.#effectiveRate();
      const cur = this.#current();
      const end = this.#end();
      if (rate > 0 && autoRewind && (cur === null || cur < 0 || cur >= end)) seek = 0;
      else if (rate < 0 && autoRewind && (cur === null || cur <= 0 || cur > end)) {
        if (end === Infinity) throw domError("Cannot play reversed an animation with an infinite end.", "InvalidStateError");
        seek = end;
      } else if (rate === 0 && cur === null) seek = 0;
      if (seek !== null) {
        this.#start = null;
        this.#hold = seek;
      }
      if (this.#hold !== null) this.#start = null;
      if (this.#pending) {
        this.#cancelPending();
        hadPendingReady = true;
      }
      if (this.#hold === null && seek === null && !abortedPause && this.#pendingRate === null) return;
      if (!hadPendingReady) this.#ready = deferred();
      this.#pending = "play";
      this.#schedulePending();
      this.#updateFinished(false, false);
      this.#sync();
    }

    /** A pending play or pause completes at the next frame, with that frame's time as the ready time. */
    #schedulePending(): void {
      const token = ++this.#token;
      d.frame((timestamp) => {
        if (token !== this.#token || this.#pending === null) return;
        const task = this.#pending;
        this.#pending = null;
        const readyTime = Number.isFinite(timestamp) ? timestamp : (this.#timelineTime() ?? 0);
        if (task === "play") {
          if (this.#hold !== null) {
            this.#applyPendingRate();
            this.#start = this.#rate === 0 ? readyTime : readyTime - this.#hold / this.#rate;
            if (this.#rate !== 0) this.#hold = null;
          } else if (this.#start !== null && this.#pendingRate !== null) {
            const match = (readyTime - this.#start) * this.#rate;
            this.#applyPendingRate();
            if (this.#rate === 0) this.#hold = match;
            else this.#start = readyTime - match / this.#rate;
          }
        } else {
          if (this.#start !== null && this.#hold === null) this.#hold = (readyTime - this.#start) * this.#rate;
          this.#applyPendingRate();
          this.#start = null;
        }
        this.#ready.resolve(this);
        this.#updateFinished(false, false);
        this.#sync();
      });
    }

    /** §4.4.17, "updating the finished state". */
    #updateFinished(didSeek: boolean, synchronously: boolean): void {
      const tl = this.#timelineTime();
      const unconstrained = didSeek
        ? this.#current()
        : this.#start === null || tl === null
          ? null
          : (tl - this.#start) * this.#rate;
      if (unconstrained !== null && this.#start !== null && this.#pending === null) {
        const end = this.#end();
        if (this.#rate > 0 && unconstrained >= end) {
          this.#hold = didSeek ? unconstrained : this.#prev === null ? end : Math.max(this.#prev, end);
        } else if (this.#rate < 0 && unconstrained <= 0) {
          this.#hold = didSeek ? unconstrained : this.#prev === null ? 0 : Math.min(this.#prev, 0);
        } else if (this.#rate !== 0 && tl !== null) {
          if (didSeek && this.#hold !== null) this.#start = tl - this.#hold / this.#rate;
          this.#hold = null;
        }
      }
      this.#prev = this.#current();
      const finished = this.playState === "finished";
      if (finished && !this.#finished.settled) {
        if (synchronously) this.#notifyFinished();
        else if (!this.#notifyQueued) {
          this.#notifyQueued = true;
          microtask(() => {
            this.#notifyQueued = false;
            this.#notifyFinished();
          });
        }
      }
      if (!finished && this.#finished.settled) this.#finished = deferred();
    }

    #notifyFinished(): void {
      if (this.playState !== "finished" || this.#finished.settled) return;
      this.#finished.resolve(this);
      const cur = this.#current();
      const tl = this.#timelineTime();
      this.#sync();
      removeReplaced(this);
      // After the promise's reactions, as a browser's frame runs its
      // microtask checkpoint before it dispatches the animation events.
      microtask(() => this.#fire("finish", cur, tl));
    }

    #fire(type: string, currentTime: number | null, timelineTime: number | null): void {
      const ev = new AnimationPlaybackEventImpl(type, { currentTime, timelineTime });
      try {
        this.dispatchEvent?.(ev);
      } catch (e) {
        console.error(e);
      }
      const h = (this as unknown as Record<string, unknown>)[`on${type}`];
      if (typeof h === "function") {
        try {
          (h as (ev: unknown) => unknown).call(this, ev);
        } catch (e) {
          console.error(e);
        }
      }
    }

    /** Does it paint, or might it without a state change? What `getAnimations()` calls relevant. */
    #relevant(): boolean {
      if (!this.#effect || this.#replace === "removed") return false;
      const state = this.playState;
      if (state === "idle") return false;
      if (state !== "finished") return true;
      return this.#effect._inEffect(this.#current(), this.#rate < 0);
    }

    /** Brings the host in line with the state machine, and arms the finish timer. */
    #sync(): void {
      const want = this.#relevant();
      if (want) live.add(this);
      else live.delete(this);
      if (want && !this.#native) {
        this.#native = this.#effect!._push(this.#nid);
        this.#pushed = null;
      } else if (!want && this.#native) {
        host.drop(this.#nid);
        this.#native = false;
      }
      if (this.#native) {
        const key = `${this.#start}|${this.#hold}|${this.#rate}`;
        if (key !== this.#pushed) {
          this.#pushed = key;
          host.clock(this.#nid, this.#start ?? NaN, this.#hold ?? NaN, this.#rate);
        }
      }
      this.#arm();
    }

    /** One timer to the moment the current time crosses the end it is heading for. */
    #arm(): void {
      if (this.#timer !== null) {
        d.clearTimer(this.#timer);
        this.#timer = null;
      }
      if (this.#pending !== null || this.#start === null || this.#hold !== null || this.#rate === 0) return;
      if (this.playState !== "running") return;
      const cur = this.#current();
      if (cur === null) return;
      const target = this.#rate > 0 ? this.#end() : 0;
      const ms = (target - cur) / this.#rate;
      if (!Number.isFinite(ms)) return;
      this.#timer = d.setTimer(() => {
        this.#timer = null;
        this.#updateFinished(false, false);
        this.#sync();
      }, Math.max(0, ms));
    }

    /** @internal The effect's keyframes, timing or target changed. */
    _effectChanged(): void {
      if (this.#native) {
        host.drop(this.#nid);
        this.#native = false;
      }
      this.#updateFinished(false, false);
      this.#sync();
    }

    /** @internal */ _id(): number {
      return this.#nid;
    }
    /** @internal Auto-removal (§4.5): replaced by later finished animations. */ _remove(): void {
      this.#replace = "removed";
      const cur = this.#current();
      const tl = this.#timelineTime();
      this.#sync();
      microtask(() => this.#fire("remove", cur, tl));
    }
  }

  /**
   * §4.5, "removing replaced animations": once `latest` has finished, an
   * earlier finished, filling animation on the same target whose every
   * property a later finished one covers is removed — what keeps `fill:
   * "forwards"` animations from piling up.
   */
  const removeReplaced = (latest: AnimationImpl): void => {
    const target = latest.effect?._target();
    if (!target) return;
    // `latest` counts even when it stopped painting (finished, not filling):
    // a finished animation replaces whatever it covers either way.
    const mine = [...new Set([...live, latest])].filter((a) => a.effect?._target() === target).sort(byComposite);
    for (let i = 0; i < mine.length; i++) {
      const a = mine[i];
      if (a.replaceState !== "active" || a.playState !== "finished") continue;
      const later = mine.slice(i + 1).filter((b) => b.playState === "finished" && b.replaceState !== "removed");
      if (!later.length) continue;
      const covered = new Set<string>();
      for (const b of later) for (const p of b.effect!._properties()) covered.add(p);
      const props = a.effect!._properties();
      if ([...props].every((p) => covered.has(p))) a._remove();
    }
  };

  // -- the surface ---------------------------------------------------------

  const byComposite = (a: AnimationImpl, b: AnimationImpl): number => a._id() - b._id();

  function animate(this: object, keyframes: unknown, options?: unknown): AnimationImpl {
    if (!advisedPlacement) {
      advisedPlacement = true;
      d.warn(
        "waapi:placement",
        "Element.animate() runs natively here, on HTMLElement.prototype and SVGElement.prototype only — Element.prototype has no 'animate', so motion / framer-motion keep their own JS animations (docs/RUNTIME.md §5.23)",
      );
    }
    const effect = new KeyframeEffectImpl(this, keyframes, options);
    const o = options && typeof options === "object" ? (options as Record<string, unknown>) : null;
    if (o && o.timeline !== undefined && o.timeline !== timeline) {
      d.warn("waapi:timeline", "Element.animate({ timeline }): only document.timeline drives animations here — the animation runs on it");
    }
    if (o && (o.rangeStart !== undefined || o.rangeEnd !== undefined)) {
      d.warn("waapi:range", "Element.animate({ rangeStart, rangeEnd }): scroll-driven ranges are not supported — the animation runs on document.timeline");
    }
    const anim = new AnimationImpl(effect, timeline);
    if (o && o.id !== undefined) anim.id = String(o.id);
    anim.play();
    return anim;
  }

  function getAnimations(this: object, options?: { subtree?: boolean }): AnimationImpl[] {
    const subtree = !!options?.subtree;
    return [...live]
      .filter((a) => {
        const t = a.effect?._target();
        return !!t && (t === this || (subtree && d.contains(this, t)));
      })
      .sort(byComposite);
  }

  for (const proto of d.protos) {
    Object.defineProperty(proto, "animate", { value: animate, configurable: true, writable: true });
    Object.defineProperty(proto, "getAnimations", { value: getAnimations, configurable: true, writable: true });
  }

  d.defineGlobal("Animation", AnimationImpl);
  d.defineGlobal("AnimationEffect", AnimationEffectImpl);
  d.defineGlobal("KeyframeEffect", KeyframeEffectImpl);
  d.defineGlobal("AnimationTimeline", AnimationTimelineImpl);
  d.defineGlobal("DocumentTimeline", DocumentTimelineImpl);
  d.defineGlobal("AnimationPlaybackEvent", AnimationPlaybackEventImpl);
  if (d.doc) {
    Object.defineProperty(d.doc, "timeline", { value: timeline, configurable: true, writable: true });
    Object.defineProperty(d.doc, "getAnimations", {
      value: () => [...live].sort(byComposite),
      configurable: true,
      writable: true,
    });
  }
  return { Animation: AnimationImpl, KeyframeEffect: KeyframeEffectImpl, timeline };
}
