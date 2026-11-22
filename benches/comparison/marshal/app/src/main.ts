import { createApp, h } from "@skathi-native/runtime";
import {
  flip,
  items,
  count,
  add,
  addi,
  strlen,
  word,
  len,
  bytesOut,
  bytesInTask,
  bytesOutTask,
  direct,
  fire,
  fireCost,
  on,
} from "native:bench";
import { bytesIn } from "native:bytes";

declare const benchRaw: { add(a: number, b: number): number; len(bytes: Uint8Array): number };

const nb = { add, addi, strlen, word, len, bytesOut, bytesInTask, bytesOutTask, direct } as {
  add(a: number, b: number): number;
  addi(a: number, b: number): number;
  strlen(s: string): number;
  word(): string;
  len(s: string): number;
  bytesOut(n: number): ArrayLike<number>;
  bytesInTask(data: unknown): Promise<number>;
  bytesOutTask(n: number): Promise<ArrayLike<number>>;
  direct(): boolean;
};

function signal(state: string) {
  document.title = `Marshal ${state}`;
}

function time(f: () => void): number {
  const t = performance.now();
  f();
  return performance.now() - t;
}

async function timeAsync(f: () => Promise<void>): Promise<number> {
  const t = performance.now();
  await f();
  return performance.now() - t;
}

const N = 10000;
const MB = 1024 * 1024;
const us = (ms: number) => ((ms * 1000) / N).toFixed(3);
const per50 = (ms: number) => (ms / 50).toFixed(3);

async function bench() {
  const direct = nb.direct();
  let s = 0;
  const add = time(() => {
    for (let i = 0; i < N; i++) s = nb.add(s, 1);
  });
  let si = 0;
  const addi = time(() => {
    for (let i = 0; i < N; i++) si = nb.addi(si, 1);
  });
  let r = 0;
  const rawAdd = time(() => {
    for (let i = 0; i < N; i++) r = benchRaw.add(r, 1);
  });
  const structs = typeof flip === "function";
  let p = { x: 1, y: 2 };
  const flipT = !structs
    ? NaN
    : time(() => {
        for (let i = 0; i < N; i++) p = (flip as (q: typeof p) => typeof p)(p);
      });
  type It = { id: number; name: string; tags: string[]; score: number | null };
  let objs: It[] = [];
  const itemsT = !structs
    ? NaN
    : time(() => {
        for (let i = 0; i < 100; i++) objs = (items as (n: number) => It[])(1000);
      });
  let c = 0;
  const countT = !structs
    ? NaN
    : time(() => {
        for (let i = 0; i < 100; i++) c += (count as (l: It[]) => number)(objs);
      });
  const w32 = "abcdefghijklmnopqrstuvwxyz012345";
  let sl = 0;
  const str32 = time(() => {
    for (let i = 0; i < N; i++) sl += nb.strlen(w32);
  });
  let wl = 0;
  const word = time(() => {
    for (let i = 0; i < N; i++) wl += nb.word().length;
  });
  const text = "x".repeat(MB);
  let n = 0;
  const strMb = time(() => {
    for (let i = 0; i < 50; i++) n += nb.len(text);
  });
  const bytes = new Uint8Array(MB);
  for (let i = 0; i < bytes.length; i++) bytes[i] = i & 255;
  const list = Array.from(bytes);
  let arg: unknown = bytes;
  try {
    bytesIn(bytes);
  } catch {
    arg = list;
  }
  const typed = arg === bytes;
  let m = 0;
  const rawMb = time(() => {
    for (let i = 0; i < 50; i++) m += benchRaw.len(bytes);
  });
  let bi = 0;
  const inMb = time(() => {
    for (let i = 0; i < 50; i++) bi += bytesIn(arg);
  });
  let ba = 0;
  const inArrMb = time(() => {
    for (let i = 0; i < 50; i++) ba += bytesIn(list);
  });
  let bo = 0;
  const outMb = time(() => {
    for (let i = 0; i < 50; i++) bo += nb.bytesOut(MB).length;
  });
  let ti = 0;
  const taskIn = await timeAsync(async () => {
    for (let i = 0; i < 50; i++) ti += await nb.bytesInTask(arg);
  });
  let to = 0;
  const taskOut = await timeAsync(async () => {
    for (let i = 0; i < 50; i++) to += (await nb.bytesOutTask(MB)).length;
  });
  const events = await new Promise<string>((resolve) => {
    const total = 400;
    const deltas: number[] = [];
    let last = 0;
    let seen = 0;
    const done = () => {
      off();
      deltas.sort((x, y) => x - y);
      const median = deltas[Math.floor(deltas.length / 2)] ?? NaN;
      resolve(
        `event_us=${(median * 1000).toFixed(3)} emit_us=${(fireCost as () => number)().toFixed(3)} events=${seen}`,
      );
    };
    const off = (on as (e: string, f: (d: { seq: number; values: number[] }) => void) => () => void)("tick", (d) => {
      const now = performance.now();
      if (seen > 0 && d.values.length === 4) deltas.push(now - last);
      last = now;
      if (++seen === total) done();
    });
    setTimeout(() => {
      if (seen < total) done();
    }, 30000);
    (fire as (n: number) => void)(total);
  });
  signal(
    `bench direct=${direct} typed=${typed} add_us=${us(add)} addi_us=${us(addi)} raw_add_us=${us(rawAdd)} ` +
      `flip_us=${us(flipT)} items1k_ms=${(itemsT / 100).toFixed(3)} count1k_ms=${(countT / 100).toFixed(3)} str32_us=${us(str32)} word_us=${us(word)} str_mb_ms=${per50(strMb)} raw_mb_ms=${per50(rawMb)} ` +
      `in_mb_ms=${per50(inMb)} in_arr_mb_ms=${per50(inArrMb)} out_mb_ms=${per50(outMb)} task_in_mb_ms=${per50(taskIn)} task_out_mb_ms=${per50(taskOut)} ${events} ` +
      `check=${p.x}/${c}/${s}/${si}/${r}/${sl}/${wl}/${n}/${m}/${bi}/${ba}/${bo}/${ti}/${to}`,
  );
}

createApp({ render: () => h("main", { style: "padding:16px" }, [h("h1", "Marshal bench")]) }).mount();

requestAnimationFrame(() =>
  requestAnimationFrame(() => {
    signal("ready");
    setTimeout(() => {
      bench().catch((e: unknown) => signal(`bench error ${String(e)}`));
    }, 500);
  }),
);
