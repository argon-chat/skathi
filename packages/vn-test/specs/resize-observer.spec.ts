import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/resize-observer/src/main.ts",
  window: [480, 360],
  label: "resize-observer",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const js = <V>(src: string) => app.eval(src) as Promise<V>;
const state = <V>(path: string) => js<V>(`JSON.parse(JSON.stringify(globalThis.__VN_TEST_RO__.${path} ?? null))`);

type Grow = {
  at: number;
  seen: { frame: number; height: number }[];
  next: { frame: number; panel: number; list: number } | null;
};
type Err = { via: string; frame: number; message: string; args?: unknown[] };
const LOOP = "ResizeObserver loop completed with undelivered notifications.";

await until(
  () => state<{ width: number }[]>("outer"),
  (o) => (o?.length ?? 0) >= 1,
  10_000,
);

describe("ResizeObserver after layout", () => {
  test(
    "the callback sees the size this frame lays out, in this frame, and the frame paints its response",
    async () => {
      await app.send("trace", { op: "start" });
      await js("__VN_TEST_RO__.run('grow')");
      const g = await until(
        () => state<Grow>("grow"),
        (v) => !!v?.next,
      );
      const trace = await app.send("trace", { op: "stop" });
      bunExpect(g.seen).toEqual([{ frame: g.at, height: 100 }]);
      // Its response did not wait a frame.
      bunExpect(g.next).toEqual({ frame: g.at + 1, panel: 100, list: 100 });
      const obs = trace.summary.observers;
      bunExpect(obs.n).toBeGreaterThanOrEqual(1);
      bunExpect(obs.relayouts).toBeGreaterThanOrEqual(1);
      bunExpect(obs.loopErrors).toBe(0);
      bunExpect(trace.summary.sync.n).toBe(0);
    },
    T,
  );

  test(
    "a change above the depth just reported is skipped: the loop error, and the next frame delivers it",
    async () => {
      const before = (await state<{ frame: number; width: number }[]>("outer"))!.length;
      await app.send("trace", { op: "start" });
      await js("__VN_TEST_RO__.run('loop')");
      const outer = await until(
        () => state<{ frame: number; width: number }[]>("outer"),
        (o) => (o?.length ?? 0) > before,
      );
      const at = await state<number>("loop.at");
      const errors = (await state<Err[]>("errors"))!;
      const trace = await app.send("trace", { op: "stop" });
      bunExpect(errors).toEqual([
        { via: "listener", frame: at, message: LOOP },
        { via: "onerror", frame: at, message: LOOP, args: [LOOP, "", 0, 0, null] },
      ]);
      bunExpect(outer.slice(before)).toEqual([{ frame: at + 1, width: 260 }]);
      bunExpect(trace.summary.observers.loopErrors).toBe(1);
      // A browser does not print it, and neither do we.
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes("ResizeObserver loop"))).toBe(false);
    },
    T,
  );
});
