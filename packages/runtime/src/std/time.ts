// `skathi:time`: durations, stopwatches, deadlines, backoff, rate limits, retries and timers.
// The host installs `TimeSpan`, `Stopwatch`, `Backoff` and `Deadline`. `RateLimiter`, `RetryPolicy`
// and `Timer` are promises, timers and callbacks into the app from end to end, so they are here.

/** A length of time as one number of milliseconds. `+span` and `span < other` use that number. */
export interface TimeSpan {
  readonly totalMilliseconds: number;
  readonly totalSeconds: number;
  add(other: TimeSpan | number): TimeSpan;
  subtract(other: TimeSpan | number): TimeSpan;
  /** Days, hours, minutes, seconds and whole milliseconds as a `Temporal.Duration`. */
  toDuration(): Temporal.Duration;
  /**
   * Without a locale, the form `parse` reads: `"1h30m"`, `"250ms"`. With one, the words of that
   * language by `Intl.DurationFormat`: `"1 hr, 30 min"`.
   */
  toString(locale?: string | string[]): string;
  valueOf(): number;
  toJSON(): number;
}

export interface TimeSpanStatics {
  readonly prototype: TimeSpan;
  fromMilliseconds(ms: number): TimeSpan;
  fromSeconds(seconds: number): TimeSpan;
  fromMinutes(minutes: number): TimeSpan;
  fromHours(hours: number): TimeSpan;
  fromDays(days: number): TimeSpan;
  /**
   * `"1h30m"`, `"1.5s"`, `"250ms"`, `"2d 4h"`, or an ISO 8601 duration of weeks, days and time
   * (`"P1DT2H"`). Throws a `SyntaxError`; years and months are refused, having no fixed length.
   */
  parse(text: string): TimeSpan;
}

/** Elapsed time on the OS's monotonic clock, which a change of the wall clock does not move. */
export interface Stopwatch {
  readonly elapsed: TimeSpan;
  readonly running: boolean;
  start(): void;
  /** Keeps what has elapsed; `start()` goes on from there. */
  stop(): void;
  /** Stops and clears. */
  reset(): void;
  /** Clears and starts. */
  restart(): void;
}

export interface StopwatchConstructor {
  readonly prototype: Stopwatch;
  new (): Stopwatch;
  /** A stopwatch that is already running. */
  startNew(): Stopwatch;
}

export interface BackoffOptions {
  /** The first delay, 100 ms by default. */
  initial?: number | TimeSpan;
  /** The cap, 30 s by default. */
  max?: number | TimeSpan;
  /** What each delay is multiplied by, 2 by default. */
  factor?: number;
  /** `"full"` (default) picks from 0 to the delay, `"equal"` from half of it, `"none"` returns it. */
  jitter?: "full" | "equal" | "none";
}

export interface Backoff {
  /** Attempts taken since the last `reset()`. */
  readonly attempt: number;
  /** The next delay in milliseconds. */
  next(): number;
  reset(): void;
}

export interface BackoffConstructor {
  readonly prototype: Backoff;
  new (options?: BackoffOptions): Backoff;
}

/** A point in time a chain of calls must finish by. */
export interface Deadline {
  /** Aborts with a `TimeoutError` when the deadline passes; pass it to `fetch` and the like. */
  readonly signal: AbortSignal;
  /** Milliseconds left, 0 once passed. */
  remaining(): number;
  expired(): boolean;
  /** Throws a `TimeoutError` `DOMException` once passed. */
  throwIfExpired(): void;
}

export interface DeadlineStatics {
  readonly prototype: Deadline;
  after(time: number | TimeSpan): Deadline;
}

declare const __sk_std: {
  time: {
    TimeSpan: TimeSpanStatics;
    Stopwatch: StopwatchConstructor;
    Backoff: BackoffConstructor;
    Deadline: DeadlineStatics;
  };
};
declare const performance: { now(): number };

export const TimeSpan: TimeSpanStatics = __sk_std.time.TimeSpan;
export const Stopwatch: StopwatchConstructor = __sk_std.time.Stopwatch;
export const Backoff: BackoffConstructor = __sk_std.time.Backoff;
export const Deadline: DeadlineStatics = __sk_std.time.Deadline;

function milliseconds(value: number | TimeSpan, what: string): number {
  const ms = Number(value);
  if (!Number.isFinite(ms) || ms < 0) throw new RangeError(`${what} must be a time of 0 ms or more`);
  return ms;
}

function wait(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason);
      return;
    }
    const aborted = () => {
      clearTimeout(timer);
      reject(signal?.reason);
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", aborted);
      resolve();
    }, ms);
    signal?.addEventListener("abort", aborted, { once: true });
  });
}

/** A token bucket: `tokens` are added every `per`, up to `burst` of them saved for later. */
export class RateLimiter {
  #capacity: number;
  #perMs: number;
  #tokens: number;
  #at: number;

  constructor(options: { tokens: number; per: number | TimeSpan; burst?: number }) {
    const per = milliseconds(options.per, "RateLimiter: per");
    const capacity = options.burst ?? options.tokens;
    if (!(options.tokens > 0) || !(per > 0) || !(capacity >= 1)) {
      throw new RangeError("RateLimiter: tokens and per must be above 0, and burst at least 1");
    }
    this.#capacity = capacity;
    this.#perMs = options.tokens / per;
    this.#tokens = capacity;
    this.#at = performance.now();
  }

  #refill(): void {
    const now = performance.now();
    this.#tokens = Math.min(this.#capacity, this.#tokens + (now - this.#at) * this.#perMs);
    this.#at = now;
  }

  /** Whole tokens that can be taken now. */
  get available(): number {
    this.#refill();
    return Math.max(0, Math.floor(this.#tokens));
  }

  /** Takes `count` tokens if they are there. */
  tryAcquire(count = 1): boolean {
    this.#refill();
    if (this.#tokens < count) return false;
    this.#tokens -= count;
    return true;
  }

  /**
   * Resolves when `count` tokens are this caller's, in the order of the calls. Aborting `signal`
   * rejects with its reason and gives the tokens back.
   */
  acquire(count = 1, signal?: AbortSignal): Promise<void> {
    if (!(count > 0) || count > this.#capacity) {
      return Promise.reject(
        new RangeError(`RateLimiter.acquire: ${count} is not from 1 to the burst of ${this.#capacity}`),
      );
    }
    if (signal?.aborted) return Promise.reject(signal.reason);
    this.#refill();
    this.#tokens -= count;
    if (this.#tokens >= 0) return Promise.resolve();
    return wait(-this.#tokens / this.#perMs, signal).catch((reason: unknown) => {
      this.#tokens += count;
      throw reason;
    });
  }
}

/** Runs a function again after it fails, waiting a `Backoff` delay between the attempts. */
export class RetryPolicy {
  #attempts: number;
  #backoff: BackoffOptions | undefined;
  #retryIf: (error: unknown) => boolean;

  constructor(options: { attempts: number; backoff?: BackoffOptions; retryIf?: (error: unknown) => boolean }) {
    if (!Number.isInteger(options.attempts) || options.attempts < 1) {
      throw new RangeError("RetryPolicy: attempts must be an integer of 1 or more");
    }
    this.#attempts = options.attempts;
    this.#backoff = options.backoff;
    this.#retryIf = options.retryIf ?? (() => true);
    new Backoff(this.#backoff);
  }

  /**
   * Calls `run` with the attempt's number, from 1, and a signal to stop by. Rejects with the last
   * error when the attempts run out or `retryIf` says no, and with the signal's reason when it aborts.
   */
  async run<T>(run: (attempt: number, signal: AbortSignal) => Promise<T>, signal?: AbortSignal): Promise<T> {
    const stop = signal ?? new AbortController().signal;
    const backoff = new Backoff(this.#backoff);
    for (let attempt = 1; ; attempt++) {
      stop.throwIfAborted();
      try {
        return await run(attempt, stop);
      } catch (error) {
        if (attempt >= this.#attempts || stop.aborted || !this.#retryIf(error)) throw error;
      }
      await wait(backoff.next(), stop);
    }
  }
}

/**
 * A timer that can be paused. It fires a `tick` event when its time is up, and keeps firing with
 * `repeat`. It runs on the host's event loop, so it fires in a hidden window too.
 */
export class Timer extends EventTarget {
  #interval: number;
  #repeat: boolean;
  #handle: number | undefined;
  #dueAt = 0;
  #left: number;
  #running = false;

  constructor(time: number | TimeSpan, options?: { repeat?: boolean }) {
    super();
    this.#interval = milliseconds(time, "Timer: the time");
    this.#repeat = options?.repeat === true;
    this.#left = this.#interval;
  }

  get running(): boolean {
    return this.#running;
  }

  /** Milliseconds until the next `tick`; it stands still while paused. */
  get remaining(): number {
    return this.#running ? Math.max(0, this.#dueAt - performance.now()) : this.#left;
  }

  /** Starts, or goes on after `pause()`. */
  start(): void {
    if (this.#running) return;
    this.#arm(this.#left);
  }

  pause(): void {
    if (!this.#running) return;
    this.#left = this.remaining;
    this.#disarm();
  }

  resume(): void {
    this.start();
  }

  /** Stops and puts the full time back. */
  reset(): void {
    this.#disarm();
    this.#left = this.#interval;
  }

  #arm(ms: number): void {
    this.#running = true;
    this.#dueAt = performance.now() + ms;
    this.#handle = setTimeout(() => this.#fire(), ms);
  }

  #disarm(): void {
    clearTimeout(this.#handle);
    this.#handle = undefined;
    this.#running = false;
  }

  #fire(): void {
    this.#left = this.#interval;
    if (this.#repeat) this.#arm(this.#interval);
    else this.#running = false;
    this.dispatchEvent(new Event("tick"));
  }
}
