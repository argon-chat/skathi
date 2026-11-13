// Web Animations: `Element.animate()`, `Animation`, `KeyframeEffect` and `document.timeline`: the object model and
// lifecycle state machine. It tells the native core (crates/vn-tree/src/waapi.rs) only the effect (keyframes, timing)
// and the clock (start or held time, rate); the host animator (crates/vn-css/src/animator/script.rs) samples every
// frame from that, so nothing crosses per frame.
// Differences from a browser:
// * `animate` / `getAnimations` live on HTMLElement.prototype and SVGElement.prototype, not
//   Element.prototype: motion / framer-motion probe Element.prototype to choose WAAPI, and their JS
//   path is better here (a spring would fall back to a 300 ms ease-out, a `filter` would jump at
//   its end). One advisory, on the first call.
// * `document.timeline.currentTime` is held for the frame (the rAF timestamp once a rAF has run,
//   else the first read, held from then on). It never goes back.
// * A finish is dispatched one frame after the frame that reached it: its promise settles in
//   microtasks that run after the rAF batch (`queueEvent`). A filling animation paints its end
//   value meanwhile.
// * Composite `add`/`accumulate`, pseudo-element targets, other timelines, `linear(…)` easing and
//   properties the animator cannot interpolate are advised once each; timing, promises and events
//   still run.
// * `getAnimations()` lists script animations only; CSS animations and transitions have no objects.
// Without the host functions nothing is installed and `animate` stays undefined.

export interface WaapiHost {
  setEffect(id: number, node: number, frames: Frame[], timing: Timing): WaapiEffectNotes;
  clock(id: number, start: number, hold: number, rate: number): void;
  drop(id: number): void;
  values(id: number): [string, string][] | null;
}

export interface WaapiDeps {
  host: WaapiHost;
  core: WaapiCore;
  protos: object[];
  /** The host node a target paints as, or 0 for anything the host does not render. */
  nodeOf(target: object): number;
  setStyle(target: object, name: string, value: string): void;
  contains(root: object, node: object): boolean;
  warn(key: string, message: string): void;
  now(): number;
  frame(cb: (timestamp: number) => void): void;
  onFrame?(stamp: (timestamp: number) => void): void;
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
interface Frame {
  offset: number | null;
  computed: number;
  easing: string;
  composite: Composite;
  props: [string, string][];
}
interface ComputedTiming {
  fill: Exclude<FillMode, "auto">;
  duration: number;
  endTime: number;
  activeDuration: number;
  localTime: number | null;
  progress: number | null;
  currentIteration: number | null;
}
export interface WaapiEffectNotes {
  accepted: boolean;
  error: string | null;
  ignored: string[];
  diagnostics: string[];
  hyphenated: string[];
}
export interface WaapiCore {
  keyframes(input: unknown, where: string): Frame[];
  timing(base: Timing, input: unknown, where: string): Timing;
  computed(timing: Timing, local: number | null, backwards: boolean): ComputedTiming;
  endTime(timing: Timing): number;
  inEffect(timing: Timing, local: number | null, backwards: boolean): boolean;
  validEasing(text: string): boolean;
  evaluateEasing(text: string, progress: number): number;
}
export interface NativeWaapiCore extends WaapiCore, WaapiHost {}

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
  const core = d.core;
  const processTiming = core.timing;
  const processKeyframes = core.keyframes;
  const domError = (message: string, name: string): Error => {
    if (typeof d.DOMException === "function") return new d.DOMException(message, name);
    const e = new Error(message);
    e.name = name;
    return e;
  };
  const EventBase = (d.Event ??
    class {
      type: string;
      constructor(type: string) {
        this.type = type;
      }
    }) as new (type: string, init?: object) => object;
  const TargetBase = (d.EventTarget ?? class {}) as new () => {
    dispatchEvent?(ev: object): boolean;
  };

  const live = new Set<AnimationImpl>();
  let nextId = 0;

  // Events wait for the next frame's update step, as in a browser. Here that is a timer: the host
  // runs due timers before the rAF batch, so a handler that writes the end value from a rAF
  // (motion's onfinish) paints it in the frame `cancel()` drops the effect, with no base-value
  // frame between. Promises settle first, in the microtasks before.
  const events: (() => void)[] = [];
  let eventsArmed = false;
  const queueEvent = (send: () => void): void => {
    events.push(send);
    if (eventsArmed) return;
    eventsArmed = true;
    d.setTimer(() => {
      eventsArmed = false;
      for (const f of events.splice(0)) f();
    }, 0);
  };

  // The timeline's time is constant within a frame (the rAF timestamp via `d.onFrame`), so two reads in one frame
  // compare equal (Argon's chat scroller relies on it). Outside a frame it is read once and held, as Chromium holds it
  // for a task, until the host's next wave starts (a 0 ms timer). It never goes back.
  let heldTime: number | null = null;
  let lastTime = -Infinity;
  let releaseArmed = false;
  const holdTime = (t: number): number => {
    heldTime = lastTime = Math.max(t, lastTime);
    if (!releaseArmed) {
      releaseArmed = true;
      d.setTimer(() => {
        releaseArmed = false;
        heldTime = null;
      }, 0);
    }
    return heldTime;
  };
  const timelineNow = (): number => heldTime ?? holdTime(d.now());
  d.onFrame?.((ts) => {
    if (heldTime === null || ts > heldTime) holdTime(ts);
  });

  class AnimationTimelineImpl {
    get currentTime(): number | null {
      return timelineNow();
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
      return timelineNow() - this.#origin;
    }
  }
  const timeline = new DocumentTimelineImpl();

  class AnimationEffectImpl {
    /** @internal */ _timing: Timing = { ...DEFAULT_TIMING };
    /** @internal */ _animation: AnimationImpl | null = null;

    getTiming(): Timing {
      return { ...this._timing };
    }

    getComputedTiming(): Record<string, unknown> {
      const anim = this._animation;
      return {
        ...this._timing,
        ...core.computed(this._timing, anim ? anim.currentTime : null, (anim?.playbackRate ?? 1) < 0),
      };
    }

    updateTiming(timing?: unknown): void {
      this._timing = processTiming(this._timing, timing ?? {}, "updateTiming");
      this._changed();
    }

    /** @internal */ _end(): number {
      return core.endTime(this._timing);
    }
    /** @internal */ _inEffect(local: number | null, backwards: boolean): boolean {
      return core.inEffect(this._timing, local, backwards);
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
        this.#target = target.#target;
        this.#frames = target.#frames.map((f) => ({
          ...f,
          props: f.props.map(([key, value]) => [key, value] as [string, string]),
        }));
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
          d.warn(
            "waapi:iterationComposite",
            "Element.animate(): iterationComposite other than 'replace' is not supported — iterations do not accumulate",
          );
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
        d.warn(
          "waapi:pseudo",
          `Element.animate(): a pseudo-element target (${this.#pseudo}) cannot be animated here — the timing and events run, nothing is painted`,
        );
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
        const out: Record<string, unknown> = {
          offset: f.offset,
          computedOffset: f.computed,
          easing: f.easing,
          composite: f.composite,
        };
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
      const odd =
        this.#composite !== "replace" ||
        this.#frames.some((f) => f.composite === "add" || f.composite === "accumulate");
      if (odd) {
        d.warn(
          "waapi:composite",
          "Element.animate(): composite 'add' / 'accumulate' is not supported — the keyframes replace the underlying value",
        );
      }
    }

    override _target(): object | null {
      return this.#target;
    }

    override _properties(): Set<string> {
      const out = new Set<string>();
      for (const f of this.#frames) for (const [key] of f.props) out.add(key);
      return out;
    }

    override _push(id: number): boolean {
      const target = this.#target;
      if (!target || this.#pseudo !== null) return false;
      const node = d.nodeOf(target);
      if (!node) {
        d.warn(
          "waapi:target",
          "Element.animate() on an element vue-native does not render (not a mirror node) runs its timing and events but paints nothing",
        );
        return false;
      }
      const notes = host.setEffect(id, node, this.#frames, this._timing);
      for (const idl of notes.hyphenated) {
        d.warn(
          `waapi:hyphen:${idl}`,
          `Element.animate(): the keyframe member '${idl}' is ignored, as in a browser — keyframes take camelCase names ('${idl.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase())}')`,
        );
      }
      if (!notes.accepted) {
        const why = notes.error ?? "invalid effect";
        d.warn(`waapi:refused:${why}`, `Element.animate(): the host refused the effect (${why}) — nothing is painted`);
      }
      for (const property of notes.ignored) {
        d.warn(
          `waapi:property:${property}`,
          property.startsWith("--")
            ? `Element.animate(): custom property '${property}' is not animated here — the rest of the keyframes still are`
            : `Element.animate(): '${property}' is not animatable here (the animator's set is the transition one) — the rest of the keyframes still animate`,
        );
      }
      for (const diagnostic of notes.diagnostics) {
        d.warn(`waapi:diag:${diagnostic}`, `Element.animate(): ${diagnostic}`);
      }
      const ok = notes.accepted;
      return ok;
    }
  }

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
        d.warn(
          "waapi:timeline",
          "new Animation(effect, timeline): only document.timeline drives animations here — the animation runs on it",
        );
        this.#timeline = timeline;
      }
      if (effect !== undefined && effect !== null) this.effect = effect as AnimationEffectImpl;
    }

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

    get effect(): AnimationEffectImpl | null {
      return this.#effect;
    }
    set effect(e: AnimationEffectImpl | null) {
      if (e === this.#effect) return;
      if (e !== null && !(e instanceof AnimationEffectImpl))
        throw new TypeError(
          "Failed to set the 'effect' property on 'Animation': The provided value is not of type 'AnimationEffect'.",
        );
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
      d.warn(
        "waapi:timeline",
        "Animation.timeline: only document.timeline drives animations here — the animation stays on it",
      );
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
      if (seek !== null && Number.isNaN(seek))
        throw new TypeError(
          "Failed to set the 'currentTime' property on 'Animation': The provided double value is non-finite.",
        );
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
          if (this.#end() === Infinity)
            throw domError("Cannot pause an animation with an infinite end, playing backwards.", "InvalidStateError");
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
        throw domError(
          "Cannot finish an animation with a playback rate of zero, or an infinite end.",
          "InvalidStateError",
        );
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
        queueEvent(() => this.#fire("cancel", null, tl));
      }
      this.#hold = null;
      this.#start = null;
      this.#sync();
    }

    reverse(): void {
      if (this.#timelineTime() === null)
        throw domError("Cannot reverse an animation with no active timeline.", "InvalidStateError");
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
      if (!target)
        throw domError("Animation.commitStyles(): the effect has no target element.", "NoModificationAllowedError");
      if (!this.#native) return;
      const raw = host.values(this.#nid);
      if (!raw) return;
      for (const [name, value] of raw) d.setStyle(target, name, value);
    }

    #silentSeek(seek: number | null): void {
      if (seek === null) {
        if (this.#current() !== null)
          throw new TypeError(
            "Failed to set the 'currentTime' property on 'Animation': currentTime may not be changed from resolved to unresolved.",
          );
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
        if (end === Infinity)
          throw domError("Cannot play reversed an animation with an infinite end.", "InvalidStateError");
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
      queueEvent(() => this.#fire("finish", cur, tl));
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

    #relevant(): boolean {
      if (!this.#effect || this.#replace === "removed") return false;
      const state = this.playState;
      if (state === "idle") return false;
      if (state !== "finished") return true;
      return this.#effect._inEffect(this.#current(), this.#rate < 0);
    }

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
      this.#timer = d.setTimer(
        () => {
          this.#timer = null;
          this.#updateFinished(false, false);
          this.#sync();
        },
        Math.max(0, ms),
      );
    }

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
    /** @internal */ _remove(): void {
      this.#replace = "removed";
      const cur = this.#current();
      const tl = this.#timelineTime();
      this.#sync();
      queueEvent(() => this.#fire("remove", cur, tl));
    }
  }

  /** Once `latest` finished, removes earlier finished, filling animations on the same target whose
   *  every property a later finished one covers (keeps `fill: "forwards"` from piling up). */
  const removeReplaced = (latest: AnimationImpl): void => {
    const target = latest.effect?._target();
    if (!target) return;
    // `latest` counts even when it stopped painting: a finished animation replaces what it covers either way.
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

  const byComposite = (a: AnimationImpl, b: AnimationImpl): number => a._id() - b._id();

  function animate(this: object, keyframes: unknown, options?: unknown): AnimationImpl {
    if (!advisedPlacement) {
      advisedPlacement = true;
      d.warn(
        "waapi:placement",
        "Element.animate() runs natively here, on HTMLElement.prototype and SVGElement.prototype only — Element.prototype has no 'animate', so motion / framer-motion keep their own JS animations",
      );
    }
    const effect = new KeyframeEffectImpl(this, keyframes, options);
    const o = options && typeof options === "object" ? (options as Record<string, unknown>) : null;
    if (o && o.timeline !== undefined && o.timeline !== timeline) {
      d.warn(
        "waapi:timeline",
        "Element.animate({ timeline }): only document.timeline drives animations here — the animation runs on it",
      );
    }
    if (o && (o.rangeStart !== undefined || o.rangeEnd !== undefined)) {
      d.warn(
        "waapi:range",
        "Element.animate({ rangeStart, rangeEnd }): scroll-driven ranges are not supported — the animation runs on document.timeline",
      );
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
