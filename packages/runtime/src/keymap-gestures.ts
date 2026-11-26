import type { SkKeyMap, SkKeyMapOptions } from "./keymap";
import { reportException } from "./runtime/report-exception";

export type SkKeyMapEventType = "down" | "up" | "press" | "click" | "doubleClick" | "hold";
export type KeymapInput = KeyboardEvent | MouseEvent | null;
export interface SkKeyMapEvent {
  readonly type: SkKeyMapEventType;
  readonly map: SkKeyMap;
  /** Zero-based index in the declared alternatives. */
  readonly alternative: number;
  /** The local input, or null for a global gesture or a reset without a DOM input. */
  readonly originalEvent: KeymapInput;
  /** A balanced up after interrupted input, focus loss or deactivation. */
  readonly cancelled: boolean;
}
export type SkKeyMapHandler = (event: SkKeyMapEvent) => void;
export interface SkKeyMapListenerOptions {
  readonly once?: boolean;
  readonly when?: (event: SkKeyMapEvent) => boolean;
}
export type KeymapListener = Pick<SkKeyMapListenerOptions, "when"> & {
  type: SkKeyMapEventType;
  handler: SkKeyMapHandler;
};

type Input = { alternative: number; event: KeymapInput };
type Timer = ReturnType<typeof setTimeout>;
type Hold = Input & { start: number; long: boolean; timer?: Timer };
type Click = Input & { origin: KeymapInput; time: number; timer: Timer };
const now = () => (globalThis as { performance?: { now(): number } }).performance?.now() ?? Date.now();

function passes<T>(predicate: ((value: T) => boolean) | undefined, value: T) {
  try {
    return predicate?.(value) ?? true;
  } catch (error) {
    reportException(error);
    return false;
  }
}

function sameClickTarget(first: KeymapInput, second: KeymapInput) {
  if (!first || !second || !("button" in first) || !("button" in second)) return true;
  return (
    first.target === second.target &&
    Math.abs((first.clientX ?? 0) - (second.clientX ?? 0)) <= 4 &&
    Math.abs((first.clientY ?? 0) - (second.clientY ?? 0)) <= 4
  );
}

function delay(value: number | undefined, fallback: number, name: string) {
  if (value === undefined) return fallback;
  if (!Number.isInteger(value) || value < 1 || value > 60_000)
    throw new RangeError(name + " must be an integer from 1 to 60000 milliseconds.");
  return value;
}

export class KeymapGestures {
  readonly listeners = new Set<KeymapListener>();
  private readonly pressed = new Set<number>();
  private active: Hold | undefined;
  private pending: Click | undefined;
  private revision = 0;
  private readonly holdMs: number;
  private readonly doubleClickMs: number;

  constructor(
    private readonly map: SkKeyMap,
    private readonly options: SkKeyMapOptions,
  ) {
    this.holdMs = delay(options.holdMs, 500, "holdMs");
    this.doubleClickMs = delay(options.doubleClickMs, 500, "doubleClickMs");
  }

  allows(event: KeymapInput) {
    const revision = this.revision;
    return this.map.enabled && passes(this.options.when, event) && revision === this.revision;
  }

  private has(type: SkKeyMapEventType, input?: Input) {
    const event = input && this.event(type, input);
    for (const listener of [...this.listeners])
      if (listener.type === type && (!event || passes(listener.when, event))) return true;
    return false;
  }

  private event(type: SkKeyMapEventType, input: Input, cancelled = false): SkKeyMapEvent {
    return { type, map: this.map, alternative: input.alternative, originalEvent: input.event, cancelled };
  }

  private emit(type: SkKeyMapEventType, input: Input, cancelled = false) {
    const event = this.event(type, input, cancelled);
    const revision = this.revision;
    let delivered = false;
    for (const listener of [...this.listeners]) {
      if (type !== "up" && revision !== this.revision) break;
      if (listener.type !== type || !this.listeners.has(listener)) continue;
      if (!passes(listener.when, event) || !this.listeners.has(listener)) continue;
      if (type !== "up" && revision !== this.revision) break;
      delivered = true;
      try {
        listener.handler(event);
      } catch (error) {
        reportException(error);
      }
    }
    return delivered;
  }

  down(alternative: number, event: KeymapInput) {
    if (!this.map.enabled) return;
    this.pressed.add(alternative);
    if (this.active) return;
    const hold: Hold = { alternative, event, start: now(), long: false };
    this.active = hold;
    if (this.has("hold") && event?.type !== "wheel") {
      hold.timer = setTimeout(() => this.fireHold(hold), this.holdMs);
    }
    this.emit("down", hold);
  }

  private fireHold(hold: Hold) {
    if (this.active !== hold || hold.long || !this.has("hold")) return;
    if (!this.allows(hold.event)) return this.reset();
    if (this.active !== hold) return;
    hold.long = true;
    if (!this.emit("hold", hold)) hold.long = false;
  }

  up(alternative: number, event: KeymapInput, cancelled = false) {
    const hold = this.active;
    if (!this.pressed.delete(alternative) || !hold) return;
    if (cancelled || !this.allows(event)) {
      this.reset(event);
      return;
    }
    if (this.pressed.size) return;
    if (hold.timer !== undefined && now() - hold.start >= this.holdMs) this.fireHold(hold);
    if (this.active !== hold) return;
    this.active = undefined;
    clearTimeout(hold.timer);
    const revision = this.revision;
    const input = { alternative: hold.alternative, event };
    this.emit("up", input);
    if (revision !== this.revision || !this.allows(event)) return;
    this.emit("press", input);
    if (revision === this.revision && !hold.long && sameClickTarget(hold.event, event) && this.allows(event))
      this.click(input, hold.event);
  }

  private click(input: Input, origin: KeymapInput) {
    const time = now();
    const revision = this.revision;
    const first = this.pending;
    if (first) {
      clearTimeout(first.timer);
      this.pending = undefined;
      if (
        first.alternative === input.alternative &&
        sameClickTarget(first.origin, origin) &&
        time - first.time <= this.doubleClickMs &&
        this.emit("doubleClick", input)
      ) {
        return;
      }
      if (revision !== this.revision) return;
      if (this.allows(first.event)) this.emit("click", first);
      if (revision !== this.revision) return;
    }
    const waitForDouble = this.has("doubleClick", input);
    if (revision !== this.revision) return;
    if (!waitForDouble) {
      this.emit("click", input);
      return;
    }
    const timer = setTimeout(() => {
      if (this.pending?.timer !== timer) return;
      this.pending = undefined;
      if (this.allows(input.event)) this.emit("click", input);
    }, this.doubleClickMs);
    this.pending = { ...input, origin, time, timer };
  }

  reset(event: KeymapInput = null) {
    this.revision++;
    this.pressed.clear();
    clearTimeout(this.pending?.timer);
    this.pending = undefined;
    const hold = this.active;
    this.active = undefined;
    if (hold) {
      clearTimeout(hold.timer);
      this.emit("up", { alternative: hold.alternative, event }, true);
    }
  }
}
