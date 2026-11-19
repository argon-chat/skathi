// One scenario per process, chosen by SK_SCENARIO. Each prints `NOJITBENCH <json>` and closes the window.
import { nextTick } from "vue";
import { offset, ROW_HEIGHT, ROWS } from "./list";
import { WASM } from "./wasm";

type Result = Record<string, number | string>;

// The engine has WebAssembly; the app's lib (ESNext, no DOM) does not declare it.
interface WasmApi {
  Module: new (bytes: Uint8Array) => object;
  Instance: new (module: object) => { exports: unknown };
}
const { Module, Instance } = (globalThis as unknown as { WebAssembly: WasmApi }).WebAssembly;

const frame = () => new Promise<number>((resolve) => requestAnimationFrame(() => resolve(performance.now())));
const quantile = (xs: number[], q: number) => {
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * q))]!;
};

/** One warm-up round, then `rounds` timed ones; the median of each part. */
function rounds(count: number, parts: Record<string, () => unknown>): Result {
  const times: Record<string, number[]> = {};
  let sink = 0;
  for (let round = 0; round <= count; round++) {
    for (const [name, part] of Object.entries(parts)) {
      const t0 = performance.now();
      const value = part();
      const ms = performance.now() - t0;
      sink += typeof value === "number" ? value : String(value).length;
      if (round > 0) (times[name] ??= []).push(ms);
    }
  }
  const result: Result = { sink };
  let total = 0;
  for (const [name, xs] of Object.entries(times)) {
    result[name] = quantile(xs, 0.5);
    total += result[name];
  }
  result.total = total;
  return result;
}

async function scroll(): Promise<Result> {
  for (let i = 0; i < 20; i++) await frame();
  const work: number[] = [];
  const stamps: number[] = [await frame()];
  const end = ROWS * ROW_HEIGHT - 20_000;
  for (let i = 0; i < 600; i++) {
    const t0 = performance.now();
    offset.value = (offset.value + 37) % end;
    await nextTick();
    work.push(performance.now() - t0);
    stamps.push(await frame());
  }
  const intervals = stamps.slice(1).map((t, i) => t - stamps[i]!);
  return {
    frameP50: quantile(intervals, 0.5),
    frameP95: quantile(intervals, 0.95),
    workP50: quantile(work, 0.5),
    workP95: quantile(work, 0.95),
    workTotal: work.reduce((a, b) => a + b, 0),
  };
}

function wasm(): Result {
  const t0 = performance.now();
  const module = new Module(WASM);
  const exports = new Instance(module).exports as { mix(n: number): number; sieve(n: number): number };
  const compile = performance.now() - t0;
  if (exports.sieve(1_000_000) !== 78_498) throw Error("sieve(1e6) is not 78498");
  return { compile, ...rounds(5, { mix: () => exports.mix(10_000_000), sieve: () => exports.sieve(4_000_000) }) };
}

const LEVELS = ["INFO", "WARN", "DEBUG", "ERROR"];
const METHODS = ["GET", "POST", "PUT", "DELETE"];
const USERS = ["alice", "bob", "carol", "dave", "erin", "Frank", "Grace"];
function logText(lines: number): string {
  const out: string[] = [];
  for (let i = 0; i < lines; i++) {
    const s = String(i % 60).padStart(2, "0");
    out.push(
      `2026-11-19T12:${s}:${s}.${String(i % 1000).padStart(3, "0")}Z ${LEVELS[i % 4]} [worker-${i % 16}] ` +
        `user=${USERS[i % 7]}@example${i % 5}.com id=0x${(i * 2654435761).toString(16).slice(-6)} ` +
        `${METHODS[i % 4]} /api/v1/items/${i}?sort=desc&page=${i % 9} ${200 + (i % 3) * 100} ${i % 250}ms ` +
        `"Mozilla/5.0 Über Straße ${i % 11 === 0 ? "the the" : "once"} Ünïcode"`,
    );
  }
  return out.join("\n");
}

function regexp(): Result {
  const text = logText(20_000);
  const count = (re: RegExp) => {
    let n = 0;
    for (const _ of text.matchAll(re)) n++;
    return n;
  };
  return rounds(5, {
    timestamps: () => count(/\b\d{4}-\d{2}-\d{2}T[\d:.]+Z\b/g),
    emails: () => count(/user=([\w.+-]+)@([\w-]+\.[a-z]{2,})/g),
    routes: () => count(/(GET|POST|PUT|DELETE) (\/[^\s?]+)(\?\S*)?/g),
    backreference: () => count(/\b(\w+)\s+\1\b/g),
    unicode: () => count(/\p{Lu}\p{Ll}+/gu),
    replace: () => text.replace(/(\d+)ms/g, (_, n: string) => `${Number(n) / 1000}s`).length,
    split: () => text.split(/\s+/).length,
    test: () => text.split("\n").filter((line) => /ERROR|WARN/.test(line) && /page=[0-4]\b/.test(line)).length,
  });
}

function evaluate(): Result {
  const generated = Array.from({ length: 2000 }, (_, i) => `function f${i}(x) { return x * ${i} + ${i % 7}; }`).join(
    "\n",
  );
  const calls = Array.from({ length: 2000 }, (_, i) => `s = (s + f${i}(s & 1023)) % 1000003;`).join("\n");
  return rounds(5, {
    newFunctionLoop: () =>
      new Function("n", "let s = 0; for (let i = 0; i < n; i++) s = (s + i * 7) % 1000003; return s;")(5_000_000),
    evalModule: () =>
      (0, eval)(`(() => { ${generated}\nlet s = 0; for (let k = 0; k < 50; k++) { ${calls} } return s; })()`),
    templates: () => {
      let length = 0;
      for (let i = 0; i < 2000; i++) {
        const render = new Function(
          "o",
          `return "<li class=\\"r${i % 13}\\">" + o.name + ": " + o.v * ${i} + "</li>";`,
        );
        for (let k = 0; k < 100; k++) length += (render({ name: "item", v: k }) as string).length;
      }
      return length;
    },
  });
}

export async function run(mainAt: number): Promise<void> {
  const scenario = globalThis.__SK_SCENARIO__ || "start";
  let result: Result;
  try {
    await frame();
    const firstFrame = performance.now() - mainAt;
    if (scenario === "start") result = { mainToFrame: firstFrame };
    else if (scenario === "scroll") result = await scroll();
    else if (scenario === "wasm") result = wasm();
    else if (scenario === "regexp") result = regexp();
    else if (scenario === "eval") result = evaluate();
    else throw Error(`unknown scenario ${scenario}`);
  } catch (e) {
    result = { error: String(e) };
  }
  console.log(`NOJITBENCH ${JSON.stringify({ scenario, ...result })}`);
  setTimeout(() => window.close(), 50);
}
