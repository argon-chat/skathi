import { computed, nextTick, reactive, ref, shallowRef, type Ref } from "vue";

export interface Row {
  id: number;
  label: string;
  value: number;
  done: boolean;
}

export interface BenchState {
  mainAt: number;
  mountedAt: number;
  rounds: Record<string, number>[];
  done: boolean;
  error: string | null;
}

export const bench: BenchState = { mainAt: 0, mountedAt: 0, rounds: [], done: false, error: null };
(globalThis as unknown as { __aotbench: BenchState }).__aotbench = bench;

const scenario = Number.parseInt(globalThis.__SK_SCENARIO__ ?? "", 10);
export const ROWS = Number.isFinite(scenario) && scenario > 0 ? scenario : 3000;
const ROUNDS = 3;

export const rows: Ref<Row[]> = shallowRef([]);
export const churn = ref(false);

let seed = 1;
const random = () => {
  seed = (seed * 16807) % 2147483647;
  return seed / 2147483647;
};

const make = (n: number): Row[] =>
  Array.from({ length: n }, (_, i) => ({ id: i, label: `row ${i}`, value: i % 97, done: i % 5 === 0 }));

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 30));

async function timed(into: Record<string, number>, name: string, work: () => void | Promise<void>) {
  await settle();
  const t0 = performance.now();
  await work();
  await nextTick();
  into[name] = performance.now() - t0;
}

function computedChain(): void {
  const source = ref(0);
  let tail = computed(() => source.value + 1);
  for (let i = 0; i < 200; i++) {
    const previous = tail;
    tail = computed(() => previous.value + (i & 1));
  }
  const items = ref(make(ROWS));
  const sum = computed(() => items.value.reduce((total, row) => total + (row.done ? row.value : 0), 0));
  const labels = computed(() => items.value.filter((row) => row.value > 50).length);
  let sink = 0;
  for (let k = 0; k < 300; k++) {
    source.value = k;
    const row = items.value[(k * 31) % ROWS]!;
    row.value = (row.value + 7) % 97;
    row.done = !row.done;
    sink += tail.value + sum.value + labels.value;
  }
  if (sink === -1) throw new Error("unreachable");
}

async function round(): Promise<Record<string, number>> {
  const r: Record<string, number> = {};
  seed = 1;
  await timed(r, "create", () => {
    rows.value = make(ROWS).map((row) => reactive(row));
  });
  await timed(r, "update", async () => {
    for (let pass = 0; pass < 10; pass++) {
      const list = rows.value;
      for (let i = pass; i < list.length; i += 10) {
        const row = list[i]!;
        row.value = (row.value + 1) % 97;
        row.label = `row ${row.id} #${pass}`;
      }
      await nextTick();
    }
  });
  await timed(r, "reorder", async () => {
    for (let pass = 0; pass < 6; pass++) {
      const next = rows.value.slice();
      if (pass % 2 === 0) next.reverse();
      else
        for (let i = next.length - 1; i > 0; i--) {
          const j = Math.floor(random() * (i + 1));
          [next[i], next[j]] = [next[j]!, next[i]!];
        }
      rows.value = next;
      await nextTick();
    }
  });
  await timed(r, "computed", computedChain);
  await timed(r, "churn", async () => {
    for (let pass = 0; pass < 20; pass++) {
      churn.value = !churn.value;
      await nextTick();
    }
  });
  await timed(r, "clear", () => {
    rows.value = [];
  });
  r.total = Object.values(r).reduce((a, b) => a + b, 0);
  return r;
}

export async function run(): Promise<void> {
  try {
    for (let i = 0; i < ROUNDS; i++) bench.rounds.push(await round());
  } catch (e) {
    bench.error = String((e as Error)?.stack ?? e);
  }
  bench.done = true;
}
