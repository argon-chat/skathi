import { OK, section } from "./shared";

const TIME_ARG = "Wherever a time is taken, it is a number of milliseconds or a [TimeSpan](/std/time-span/).";

export const time = section("skathi:time", [
  {
    slug: "time-span",
    name: "TimeSpan",
    kind: "class",
    summary: "A length of time held as one number of milliseconds, which compares with `<` and adds up.",
    declaration: true,
    notes: [
      "A `Temporal.Duration` keeps separate fields and cannot be compared without a reference date. A `TimeSpan` is one number, so it compares with `<` and adds up.",
      "A span is made by the static methods; the type has no public constructor. A span is immutable: `add()` and `subtract()` return a new span. A number passed to them is a number of milliseconds.",
      '`parse()` reads a list of numbers with units: `"1h30m"`, `"1.5s"`, `"250ms"`, `"2d 4h"`. The units are `d`, `h`, `m`, `s` and `ms`, in either case. Spaces are allowed, a number may have a fraction, and a leading `-` makes the span negative.',
      '`parse()` also reads an ISO 8601 duration of weeks, days and time: `"P1DT2H"`, `"PT1H30M"`, `"P2W"`.',
      '`toString()` without a locale writes the form `parse()` reads: `"1h30m"`, `"1m1.001s"`. A span under one second is written in milliseconds (`"250ms"`), and a span of zero is `"0s"`.',
      '`toString(locale)` writes the words of that language with `Intl.DurationFormat` in its short style: `"1 hr, 30 min"`. With [`"intl": "invariant"`](/sk-config/#intl) in sk.config.json, `Intl` answers from the root locale, whatever locale is asked for.',
      "`toDuration()` gives a `Temporal.Duration` of days, hours, minutes, seconds and whole milliseconds. The span is rounded to a whole millisecond first. Every field of a negative span is negative.",
    ],
    examples: [
      {
        title: "Parse, add and compare",
        lang: "ts",
        code: `import { TimeSpan } from "skathi:time";

const span = TimeSpan.parse("1h30m").add(TimeSpan.fromSeconds(30));

span.totalMilliseconds; // 5430000
span.totalSeconds; // 5430
String(span); // "1h30m30s"
JSON.stringify({ span }); // '{"span":5430000}'
String(span.toDuration()); // "PT1H30M30S"

TimeSpan.parse("P1DT0.5S").totalMilliseconds; // 86400500
TimeSpan.fromMinutes(1) < TimeSpan.fromHours(1); // true`,
      },
      {
        title: "Read a timeout from a settings file",
        lang: "ts",
        code: `import { Deadline, TimeSpan } from "skathi:time";

export function deadlineFor(setting: string | undefined): Deadline {
  let timeout = TimeSpan.fromSeconds(10);
  try {
    if (setting !== undefined) timeout = TimeSpan.parse(setting);
  } catch (error) {
    console.warn("The timeout setting is not a duration:", error);
  }
  return Deadline.after(timeout);
}`,
      },
    ],
    edgeCases: [
      '`TimeSpan.parse()` throws a `SyntaxError` for text it cannot read: empty text, a number without a unit (`"90"`), or an unknown unit.',
      '`TimeSpan.parse()` throws a `SyntaxError` for years and months (`"P1M"`), which have no fixed length. It throws a `TypeError` when the argument is not a string.',
      "`fromMilliseconds()`, `fromSeconds()`, `fromMinutes()`, `fromHours()` and `fromDays()` throw a `RangeError` when the value is not a finite number.",
      "`add()` and `subtract()` throw a `RangeError` when the other value is not a finite number or a `TimeSpan`.",
    ],
    related: ["stopwatch", "deadline", "timer", "frequency"],
    aliases: [
      "duration",
      "timespan",
      "milliseconds",
      "ISO 8601 duration",
      "Temporal.Duration",
      "TimeSpan.parse",
      "TimeSpan.fromMilliseconds",
      "TimeSpan.fromSeconds",
      "TimeSpan.fromMinutes",
      "TimeSpan.fromHours",
      "TimeSpan.fromDays",
      "TimeSpan.add",
      "TimeSpan.subtract",
      "TimeSpan.toDuration",
      "TimeSpan.toString",
      "TimeSpan.totalMilliseconds",
      "TimeSpan.totalSeconds",
    ],
  },
  {
    slug: "stopwatch",
    name: "Stopwatch",
    kind: "class",
    summary: "Measures elapsed time on the operating system's monotonic clock.",
    declaration: true,
    notes: [
      "`new Stopwatch()` is stopped. `Stopwatch.startNew()` is already running.",
      "`stop()` keeps what has elapsed, and `start()` goes on from there. `reset()` stops and clears. `restart()` clears and starts.",
      "`elapsed` is a [TimeSpan](/std/time-span/). `running` says whether the stopwatch counts.",
      "It reads the OS's monotonic clock, so a change of the wall clock does not move it.",
      "`start()` on a running stopwatch and `stop()` on a stopped one do nothing.",
    ],
    examples: [
      {
        title: "Time an operation",
        lang: "ts",
        code: `import { Stopwatch } from "skathi:time";

export async function timed<T>(label: string, run: () => Promise<T>): Promise<T> {
  const watch = Stopwatch.startNew();
  try {
    return await run();
  } finally {
    console.log(label, "took", String(watch.elapsed));
  }
}`,
        note: 'An elapsed time of 1284 ms prints as `"1.284s"`.',
      },
      {
        title: "Leave a pause out of the measurement",
        lang: "ts",
        code: `import { Stopwatch } from "skathi:time";

const active = new Stopwatch();

export function onFocus(): void {
  active.start();
}

export function onBlur(): void {
  active.stop();
}

export function activeSeconds(): number {
  return active.elapsed.totalSeconds;
}`,
      },
    ],
    related: ["time-span", "timer", "deadline"],
    aliases: [
      "elapsed time",
      "monotonic clock",
      "measure time",
      "performance.now",
      "Stopwatch.startNew",
      "Stopwatch.start",
      "Stopwatch.stop",
      "Stopwatch.reset",
      "Stopwatch.restart",
      "Stopwatch.elapsed",
      "Stopwatch.running",
    ],
  },
  {
    slug: "deadline",
    name: "Deadline",
    kind: "class",
    summary: "A point in time a chain of calls must finish by, with an `AbortSignal` and the time left.",
    declaration: true,
    notes: [
      TIME_ARG,
      "`signal` is an `AbortSignal` that aborts with a `TimeoutError` when the deadline passes. Pass it to `fetch()` and other calls that take a signal. Every read gives the same signal.",
      "`remaining()` is the milliseconds left, and 0 once the deadline has passed. `expired()` says whether it has passed.",
      "Unlike `AbortSignal.timeout()`, a deadline tells what is left, to hand the rest of the time to the next call.",
      "A deadline is made by `Deadline.after()`; the type has no public constructor. A time below 0 gives a deadline that has already passed.",
      "The deadline is kept on the OS's monotonic clock.",
    ],
    examples: [
      {
        title: "Share one time budget between two requests",
        lang: "ts",
        code: `import { Deadline, TimeSpan } from "skathi:time";

export async function loadProfile(id: string): Promise<{ user: unknown; avatar: Blob }> {
  const deadline = Deadline.after(TimeSpan.fromSeconds(10));

  const user: unknown = await (await fetch("https://api.example.com/users/" + id, { signal: deadline.signal })).json();
  deadline.throwIfExpired();

  console.log("Time left for the avatar:", deadline.remaining(), "ms");
  const avatar = await (await fetch("https://api.example.com/avatars/" + id, { signal: deadline.signal })).blob();
  return { user, avatar };
}`,
      },
      {
        title: "What a passed deadline reports",
        lang: "ts",
        code: `import { Deadline } from "skathi:time";

const deadline = Deadline.after(40);
deadline.expired(); // false
deadline.signal.aborted; // false

await new Promise((resolve) => setTimeout(resolve, 80));

deadline.expired(); // true
deadline.remaining(); // 0
deadline.signal.aborted; // true
deadline.signal.reason.name; // "TimeoutError"`,
      },
    ],
    edgeCases: [
      "`throwIfExpired()` throws a `DOMException` named `TimeoutError` once the deadline has passed. Before that it does nothing.",
      "`signal.reason` is a `DOMException` named `TimeoutError` once the deadline has passed.",
      "`Deadline.after()` throws a `RangeError` when the time is not a finite number or a `TimeSpan`, or when it is too far away to keep.",
    ],
    related: ["time-span", "retry-policy", "rate-limiter", "stopwatch"],
    aliases: [
      "timeout",
      "AbortSignal",
      "AbortSignal.timeout",
      "TimeoutError",
      "time budget",
      "Deadline.after",
      "Deadline.signal",
      "Deadline.remaining",
      "Deadline.expired",
      "Deadline.throwIfExpired",
    ],
  },
  {
    slug: "backoff",
    name: "Backoff",
    kind: "class",
    summary: "Gives growing delays for repeated attempts, with a cap and random jitter.",
    declaration: true,
    params: [
      {
        name: "options",
        type: "BackoffOptions",
        description:
          "The first delay, the cap, the factor and the jitter. See [BackoffOptions](/std/backoff-options/).",
        optional: true,
      },
    ],
    notes: [
      "`next()` returns the next delay in milliseconds: `initial × factor^attempt`, capped at `max`. `attempt` is the number of `next()` calls since the last `reset()`.",
      "The defaults are 100 ms for `initial`, 30 s for `max` and 2 for `factor`.",
      '`jitter` is applied after the cap. `"full"` (the default) returns a random delay from 0 to that value, `"equal"` a random delay from half of it to all of it, and `"none"` returns the value itself. The random number comes from `Math.random()`.',
      "`reset()` starts over from `initial`.",
      "A `Backoff` only computes the delays. It does not wait. [RetryPolicy](/std/retry-policy/) waits them between attempts.",
    ],
    examples: [
      {
        title: "Delays without jitter",
        lang: "ts",
        code: `import { Backoff } from "skathi:time";

const backoff = new Backoff({ initial: 100, max: 350, jitter: "none" });

backoff.next(); // 100
backoff.next(); // 200
backoff.next(); // 350
backoff.attempt; // 3

backoff.reset();
backoff.next(); // 100`,
      },
      {
        title: "Reconnect a WebSocket with growing delays",
        lang: "ts",
        code: `import { Backoff, TimeSpan } from "skathi:time";

const backoff = new Backoff({ initial: 500, max: TimeSpan.fromMinutes(1) });

export function connect(url: string, onMessage: (data: unknown) => void): void {
  const socket = new WebSocket(url);
  socket.addEventListener("open", () => backoff.reset());
  socket.addEventListener("message", (event) => onMessage(event.data));
  socket.addEventListener("close", () => {
    setTimeout(() => connect(url, onMessage), backoff.next());
  });
}`,
      },
    ],
    edgeCases: [
      "The constructor throws a `RangeError` when `initial` is negative, `max` is below `initial`, or `factor` is below 1 or not a number.",
      "The constructor throws a `RangeError` when `initial` or `max` is not a finite number or a `TimeSpan`.",
      'The constructor throws a `TypeError` when `jitter` is not `"full"`, `"equal"` or `"none"`.',
    ],
    related: ["backoff-options", "retry-policy", "time-span"],
    aliases: [
      "exponential backoff",
      "jitter",
      "retry delay",
      "reconnect",
      "Backoff.next",
      "Backoff.reset",
      "Backoff.attempt",
    ],
  },
  {
    slug: "backoff-options",
    name: "BackoffOptions",
    kind: "type",
    summary: "The options of a `Backoff`, also taken by `RetryPolicy` as its `backoff` option.",
    declaration: true,
    params: [
      {
        name: "initial",
        type: "number | TimeSpan",
        description: "The first delay.",
        optional: true,
        default: "100 ms",
      },
      {
        name: "max",
        type: "number | TimeSpan",
        description: "The cap on a delay.",
        optional: true,
        default: "30 s",
      },
      {
        name: "factor",
        type: "number",
        description: "What each delay is multiplied by to give the next one.",
        optional: true,
        default: "2",
      },
      {
        name: "jitter",
        type: '"full" | "equal" | "none"',
        description: "How the delay is randomized.",
        optional: true,
        default: '"full"',
      },
    ],
    values: [
      { value: '"full"', status: OK, note: "A random delay from 0 to the computed delay. The default." },
      { value: '"equal"', status: OK, note: "A random delay from half of the computed delay to all of it." },
      { value: '"none"', status: OK, note: "The computed delay itself." },
    ],
    notes: [
      "A number for `initial` or `max` is a number of milliseconds.",
      "The computed delay is `initial × factor^attempt`, capped at `max`.",
      "`max` must not be below `initial`. An `initial` above 30 s needs a `max` as well, because the default `max` is 30 s.",
    ],
    examples: [
      {
        title: "One set of options for a Backoff and a RetryPolicy",
        lang: "ts",
        code: `import { Backoff, RetryPolicy, TimeSpan, type BackoffOptions } from "skathi:time";

const delays: BackoffOptions = {
  initial: 200,
  max: TimeSpan.fromSeconds(5),
  factor: 3,
  jitter: "equal",
};

export const reconnect = new Backoff(delays);
export const requests = new RetryPolicy({ attempts: 4, backoff: delays });`,
      },
    ],
    edgeCases: [
      "`new Backoff(options)` and `new RetryPolicy({ backoff: options })` both check the options and throw a `RangeError` when `initial` is negative, `max` is below `initial`, or `factor` is below 1 or not a number.",
      "An unknown `jitter` throws a `TypeError`.",
    ],
    related: ["backoff", "retry-policy"],
    aliases: ["backoff options", "jitter", "initial", "max", "factor"],
  },
  {
    slug: "retry-policy",
    name: "RetryPolicy",
    kind: "class",
    summary: "Runs an async function again after it fails, waiting a `Backoff` delay between the attempts.",
    declaration: true,
    params: [
      {
        name: "options.attempts",
        type: "number",
        description: "The most calls one `run()` makes, counting the first. An integer of 1 or more.",
      },
      {
        name: "options.backoff",
        type: "BackoffOptions",
        description: "The delays between attempts. See [BackoffOptions](/std/backoff-options/).",
        optional: true,
      },
      {
        name: "options.retryIf",
        type: "(error: unknown) => boolean",
        description: "Called with the error of a failed attempt. Return `false` to stop retrying.",
        optional: true,
        default: "every error is retried",
      },
    ],
    returns: "`run()` returns a promise of the value the function resolved with.",
    notes: [
      "`run(fn, signal?)` calls `fn(attempt, signal)` with attempts numbered from 1, until one succeeds. After each failure it waits a backoff delay.",
      "`run()` rejects with the last error when the attempts run out or `retryIf(error)` returns `false`.",
      "When the signal aborts before an attempt or during a wait, `run()` rejects with the signal's reason. When an attempt fails after the signal has aborted, `run()` rejects with that attempt's error and does not retry.",
      "The function gets the signal passed to `run()`. Without one it gets a signal that never aborts. Pass it on to `fetch()` so an abort also stops the attempt in progress.",
      "`backoff` takes the options of [Backoff](/std/backoff/). Every `run()` counts its own delays, so one policy can serve many calls at once.",
    ],
    examples: [
      {
        title: "Retry a request within a deadline",
        lang: "ts",
        code: `import { Deadline, RetryPolicy, Stopwatch, TimeSpan } from "skathi:time";

const policy = new RetryPolicy({ attempts: 4, backoff: { initial: 200 } });

export async function load(url: string): Promise<Response> {
  const deadline = Deadline.after(TimeSpan.parse("10s"));
  const watch = Stopwatch.startNew();
  const response = await policy.run((attempt, signal) => fetch(url, { signal }), deadline.signal);
  console.log("took", String(watch.elapsed));
  return response;
}`,
      },
      {
        title: "Retry only some errors",
        lang: "ts",
        code: `import { RetryPolicy } from "skathi:time";

class HttpError extends Error {
  constructor(readonly status: number) {
    super("HTTP " + status);
  }
}

const policy = new RetryPolicy({
  attempts: 5,
  backoff: { initial: 250, max: 4_000 },
  retryIf: (error) => !(error instanceof HttpError) || error.status >= 500,
});

export function getJson(url: string, signal?: AbortSignal): Promise<unknown> {
  return policy.run(async (attempt, stop) => {
    const response = await fetch(url, { signal: stop });
    if (!response.ok) throw new HttpError(response.status);
    return (await response.json()) as unknown;
  }, signal);
}`,
      },
      {
        title: "The attempt numbers",
        lang: "ts",
        code: `import { RetryPolicy } from "skathi:time";

const calls: number[] = [];
const policy = new RetryPolicy({ attempts: 3, backoff: { initial: 1, jitter: "none" } });

const result = await policy.run(async (attempt) => {
  calls.push(attempt);
  if (attempt < 3) throw new Error("again");
  return "done";
});

result; // "done"
calls; // [1, 2, 3]`,
      },
    ],
    edgeCases: [
      "The constructor throws a `RangeError` when `attempts` is not an integer of 1 or more.",
      "The constructor checks `backoff` and throws what `new Backoff()` throws for it: a `RangeError` for a bad `initial`, `max` or `factor`, a `TypeError` for an unknown `jitter`.",
      "With `attempts: 1` the function runs once and its error is the rejection.",
      "A signal that is already aborted rejects `run()` with its reason before the function is called.",
    ],
    related: ["backoff", "backoff-options", "deadline", "rate-limiter"],
    aliases: ["retry", "retries", "attempts", "retryIf", "exponential backoff", "RetryPolicy.run"],
  },
  {
    slug: "rate-limiter",
    name: "RateLimiter",
    kind: "class",
    summary: "A token bucket that limits how often something runs: `tokens` are added every `per`, up to `burst`.",
    declaration: true,
    params: [
      { name: "options.tokens", type: "number", description: "The tokens added every `per`. Above 0." },
      {
        name: "options.per",
        type: "number | TimeSpan",
        description: "The time in which `tokens` are added, in milliseconds or as a `TimeSpan`. Above 0.",
      },
      {
        name: "options.burst",
        type: "number",
        description: "The most tokens that wait unused. At least 1.",
        optional: true,
        default: "tokens",
      },
    ],
    notes: [
      "The bucket starts full, with `burst` tokens. Tokens are added continuously at the rate of `tokens` every `per`, not in one step at the end of each period.",
      "`tryAcquire(count?)` takes the tokens if they are there and says whether it did. It never waits. `count` is 1 by default.",
      "`acquire(count?, signal?)` resolves when the tokens are the caller's. Calls are served in the order they were made.",
      "Aborting the signal rejects `acquire()` with the signal's reason and gives the tokens back.",
      "`available` is the whole tokens that can be taken now. It is 0 while `acquire()` calls are waiting.",
      "The time is measured with `performance.now()`.",
    ],
    examples: [
      {
        title: "Send at most five requests a second",
        lang: "ts",
        code: `import { RateLimiter, TimeSpan } from "skathi:time";

const limiter = new RateLimiter({ tokens: 5, per: TimeSpan.fromSeconds(1) });

export async function send(url: string, body: string, signal?: AbortSignal): Promise<Response> {
  await limiter.acquire(1, signal);
  return fetch(url, { method: "POST", body, signal: signal ?? null });
}`,
      },
      {
        title: "Drop what is over the limit",
        lang: "ts",
        code: `import { RateLimiter } from "skathi:time";

const limiter = new RateLimiter({ tokens: 2, per: 40 });

limiter.tryAcquire(); // true
limiter.tryAcquire(); // true
limiter.tryAcquire(); // false

export function report(message: string): void {
  if (limiter.tryAcquire()) console.warn(message);
}`,
      },
    ],
    edgeCases: [
      "The constructor throws a `RangeError` when `tokens` or `per` is not above 0, when `burst` is below 1, or when `per` is not a finite time.",
      "`acquire()` rejects with a `RangeError` when `count` is not above 0 or is above `burst`: such a call could never be served.",
      "`acquire()` with a signal that is already aborted rejects with its reason and takes no tokens.",
      "`tryAcquire()` with a `count` above `burst` always returns `false`.",
    ],
    related: ["time-span", "retry-policy", "deadline", "timer"],
    aliases: [
      "token bucket",
      "rate limit",
      "throttle",
      "burst",
      "RateLimiter.acquire",
      "RateLimiter.tryAcquire",
      "RateLimiter.available",
    ],
  },
  {
    slug: "task-queue",
    name: "TaskQueue",
    since: "Unreleased",
    kind: "class",
    summary: "Run a batch of tasks with a fixed limit on how many can be in progress.",
    declaration: true,
    notes: [
      "Tasks start in submission order on the event loop. Each call resolves with its task's value or rejects with its error. A synchronous throw also releases the slot for the next task.",
      "Pass the same signal to several calls to cancel a batch. Each call rejects with the signal's reason when aborted. The queue skips cancelled tasks that are waiting. A running callback receives the signal and keeps its slot until its work settles.",
      "The constructor requires a positive safe integer for `concurrency`. A fractional, zero or negative limit throws a `RangeError`.",
    ],
    examples: [
      {
        title: "Load up to four previews at once",
        lang: "ts",
        code: `import { TaskQueue } from "skathi:time";

const previews = new TaskQueue({ concurrency: 4 });
export function loadPreviews(urls: string[], signal: AbortSignal): Promise<Blob[]> {
  return Promise.all(urls.map(url => previews.run(async stop => {
    const response = await fetch(url, { signal: stop });
    return response.blob();
  }, signal)));
}`,
      },
    ],
    related: ["rate-limiter", "deadline", "retry-policy"],
  },
  {
    slug: "timer",
    name: "Timer",
    kind: "class",
    summary: "A timer that can be paused and fires a `tick` event when its time is up, once or repeatedly.",
    declaration: true,
    params: [
      {
        name: "time",
        type: "number | TimeSpan",
        description: "The time to the tick, in milliseconds or as a `TimeSpan`. 0 or more.",
      },
      {
        name: "options.repeat",
        type: "boolean",
        description: "Keep firing every `time` after the first tick.",
        optional: true,
        default: "false",
      },
    ],
    notes: [
      "A new timer is stopped. `start()` starts it, or goes on after `pause()`. `resume()` does the same as `start()`.",
      "`pause()` stops the timer and keeps the time left. `reset()` stops it and puts the full time back.",
      "`remaining` is the milliseconds to the next tick. It stands still while the timer is paused. `running` says whether the timer counts.",
      "`tick` is a plain `Event`, dispatched on the timer. A `Timer` is an `EventTarget`: listen with `addEventListener()`.",
      "Without `repeat`, the timer stops after its tick and has its full time again, so `start()` runs it once more. With `repeat`, it keeps firing until `pause()` or `reset()`.",
      "A `Timer` runs on the host's event loop, not on frames, so it fires in a hidden window, where `requestAnimationFrame()` does not.",
    ],
    examples: [
      {
        title: "Autosave that the user can pause",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { Timer, TimeSpan } from "skathi:time";

const emit = defineEmits<{ save: [] }>();

const autosave = new Timer(TimeSpan.fromSeconds(30), { repeat: true });
const paused = ref(false);

autosave.addEventListener("tick", () => emit("save"));
autosave.start();
onUnmounted(() => autosave.reset());

function toggle() {
  paused.value = !paused.value;
  if (paused.value) autosave.pause();
  else autosave.resume();
}
</script>

<template>
  <button @click="toggle">{{ paused ? "Resume autosave" : "Pause autosave" }}</button>
</template>`,
      },
      {
        title: "Hide a notice after five seconds, but not while the pointer is over it",
        lang: "ts",
        code: `import { Timer } from "skathi:time";

export function autoHide(notice: HTMLElement): void {
  const timer = new Timer(5_000);
  timer.addEventListener("tick", () => notice.remove());
  notice.addEventListener("pointerenter", () => timer.pause());
  notice.addEventListener("pointerleave", () => timer.resume());
  timer.start();
}`,
      },
    ],
    edgeCases: [
      "The constructor throws a `RangeError` when the time is negative or not a finite time.",
      "`start()` on a running timer and `pause()` on a stopped one do nothing.",
    ],
    related: ["time-span", "stopwatch", "rate-limiter"],
    aliases: [
      "setTimeout",
      "setInterval",
      "interval",
      "countdown",
      "pausable timer",
      "tick",
      "Timer.start",
      "Timer.pause",
      "Timer.resume",
      "Timer.reset",
      "Timer.remaining",
      "Timer.running",
    ],
  },
]);
