import { Backoff, Deadline, RateLimiter, RetryPolicy, Stopwatch, TaskQueue, Timer, TimeSpan } from "skathi:time";
import { Angle, DataSize, Frequency } from "skathi:units";

const attempt = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

const rejection = (promise: Promise<unknown>): Promise<string> =>
  promise.then(
    () => "resolved",
    (e: unknown) => (e as Error).name,
  );

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

async function taskQueue() {
  const queue = new TaskQueue({ concurrency: 2 });
  const batch = new AbortController();
  const starts: string[] = [];
  let release!: () => void;
  const hold = new Promise<void>((resolve) => {
    release = resolve;
  });
  const first = rejection(
    queue.run((signal) => {
      starts.push(signal === batch.signal ? "first" : "wrong signal");
      return hold;
    }, batch.signal),
  );
  const second = queue.run(() => {
    starts.push("second");
    return hold;
  });
  const cancelled = rejection(queue.run(() => starts.push("cancelled"), batch.signal));
  const failure = rejection(
    queue.run(() => {
      starts.push("throws");
      throw new TypeError();
    }),
  );
  const last = queue.run(() => {
    starts.push("last");
    return 42;
  });
  await Promise.resolve();
  batch.abort(new DOMException("folder changed", "AbortError"));
  const reasons = await Promise.all([first, cancelled, rejection(queue.run(() => starts.push("late"), batch.signal))]);
  const beforeRelease = [...starts];
  release();
  await second;
  return { beforeRelease, starts, reasons, failure: await failure, last: await last };
}

export async function time() {
  const span = TimeSpan.parse("1h30m").add(TimeSpan.fromSeconds(30));
  const duration = span.toDuration();

  const stopwatch = Stopwatch.startNew();
  await sleep(30);
  stopwatch.stop();
  const stopped = stopwatch.elapsed.totalMilliseconds;
  await sleep(20);
  const held = stopwatch.elapsed.totalMilliseconds === stopped;

  const backoff = new Backoff({ initial: 100, max: 350, jitter: "none" });
  const delays = [backoff.next(), backoff.next(), backoff.next()];
  const jittered = new Backoff({ initial: 100 }).next();

  const deadline = Deadline.after(40);
  const before = [deadline.expired(), deadline.remaining() > 0, deadline.signal.aborted];
  const sameSignal = deadline.signal === deadline.signal;
  await sleep(80);
  const reason = deadline.signal.reason as Error;

  const limiter = new RateLimiter({ tokens: 2, per: 40 });
  const taken = [limiter.tryAcquire(), limiter.tryAcquire(), limiter.tryAcquire()];
  const waited = Stopwatch.startNew();
  await limiter.acquire();
  const waitedMs = waited.elapsed.totalMilliseconds;
  const gate = new AbortController();
  const queued = rejection(new RateLimiter({ tokens: 1, per: 60_000, burst: 1 }).acquire(1, gate.signal));
  const blocked = new RateLimiter({ tokens: 1, per: 60_000 });
  blocked.tryAcquire();
  const refused = rejection(blocked.acquire(1, gate.signal));
  gate.abort(new DOMException("stop", "AbortError"));

  const calls: number[] = [];
  const policy = new RetryPolicy({ attempts: 3, backoff: { initial: 1, jitter: "none" } });
  const retried = await policy.run(async (n) => {
    calls.push(n);
    if (n < 3) throw new Error("again");
    return "done";
  });
  let gaveUp = 0;
  const exhausted = await rejection(
    new RetryPolicy({ attempts: 2, backoff: { initial: 1 } }).run(async () => {
      gaveUp++;
      throw new TypeError("no");
    }),
  );
  let picky = 0;
  await rejection(
    new RetryPolicy({ attempts: 5, retryIf: (e) => !(e instanceof TypeError) }).run(async () => {
      picky++;
      throw new TypeError("no");
    }),
  );

  const timer = new Timer(30);
  let ticks = 0;
  timer.addEventListener("tick", () => ticks++);
  timer.start();
  timer.pause();
  const paused = timer.remaining;
  await sleep(60);
  const stillPaused = [ticks, timer.remaining === paused, timer.running];
  timer.resume();
  await sleep(80);
  const fired = [ticks, timer.running];
  const repeating = new Timer(TimeSpan.fromMilliseconds(15), { repeat: true });
  let repeats = 0;
  repeating.addEventListener("tick", () => repeats++);
  repeating.start();
  await sleep(100);
  repeating.reset();
  const afterReset = repeats;
  await sleep(40);

  return {
    span: [span.totalMilliseconds, span.totalSeconds, String(span), +span, JSON.stringify({ span })],
    localized: span.toString("en"),
    duration: [String(duration), duration.constructor.name],
    iso: TimeSpan.parse("P1DT0.5S").totalMilliseconds,
    badSpan: attempt(() => TimeSpan.parse("90")),
    ordered: TimeSpan.fromMinutes(1) < TimeSpan.fromHours(1),
    stopped: stopped >= 25 && stopped < 2_000,
    held,
    running: stopwatch.running,
    delays,
    jittered: jittered >= 0 && jittered <= 100,
    attempts: backoff.attempt,
    badBackoff: attempt(() => new Backoff({ factor: 0.5 })),
    before,
    sameSignal,
    after: [deadline.expired(), deadline.remaining(), deadline.signal.aborted, reason.name],
    passed: attempt(() => deadline.throwIfExpired()),
    taken,
    waited: waitedMs >= 10 && waitedMs < 2_000,
    queued: await queued,
    refused: await refused,
    retried,
    calls,
    exhausted,
    gaveUp,
    picky,
    stillPaused,
    fired,
    repeated: afterReset >= 2 && repeats === afterReset,
    queue: await taskQueue(),
    globals: ["TimeSpan", "Stopwatch", "DataSize"].filter((name) => name in globalThis),
  };
}

export function units() {
  const size = DataSize.parse("1.5 GiB");
  return {
    bytes: size.bytes,
    to: [size.to("MiB"), DataSize.bytes(2_500).to("kB")],
    format: [
      DataSize.bytes(1_500_000_000).format({ locale: "en" }),
      size.format({ binary: true, locale: "en" }),
      DataSize.bytes(999).format({ locale: "en" }),
      DataSize.bytes(1_500_000).format({ locale: "de" }),
    ],
    larger: DataSize.parse("1 MB") > DataSize.parse("999 kB"),
    badSize: attempt(() => DataSize.parse("3 parsecs")),
    frequency: [Frequency.hz(50).period.totalMilliseconds, Frequency.fromPeriod(TimeSpan.fromSeconds(2)).hz],
    frequencyText: [String(Frequency.hz(44_100)), String(Frequency.fromPeriod(1))],
    noPeriod: attempt(() => Frequency.hz(0).period),
    angle: [Angle.rad(Math.PI).degrees, Angle.deg(180).radians === Math.PI, Angle.deg(-90).normalize().degrees],
    angleText: String(Angle.deg(450).normalize()),
  };
}
