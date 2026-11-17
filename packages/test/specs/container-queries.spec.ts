import { afterAll, describe, expect, test } from "bun:test";
import { type Bounds, launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/container-queries/src/main.ts",
  window: [640, 480],
  label: "container-queries",
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
const state = <V>(path: string) => js<V>(`JSON.parse(JSON.stringify(globalThis.__SK_TEST_CQ__.${path} ?? null))`);

async function box(id: string): Promise<Bounds> {
  const { nodes } = await app.query(`#${id}`);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`#${id} has no box`);
  return b;
}

type Sample = { container: number; child: number };

describe("container queries", () => {
  test(
    "a card switches layout at its container's width, in the frame the window resizes it",
    async () => {
      const wide = await until(
        () => box("media"),
        (b) => b.w === 120,
      );
      expect(wide.w).toBe(120);
      expect((await box("body")).x).toBeGreaterThan(wide.x);
      expect(await js("getComputedStyle(document.getElementById('shelf')).containerType")).toBe("inline-size");

      const outer = Number(await js("String(__SK_TEST_CQ__.appWindow.getBounds().width)"));
      await js(`__SK_TEST_CQ__.appWindow.setBounds({ width: ${outer - 200} }); "ok"`);
      const narrow = await until(
        () => box("media"),
        (b) => b.w !== 120,
      );
      const shelf = await box("shelf");
      expect(shelf.w).toBeLessThan(500);
      expect(narrow.w).toBe(shelf.w);
      expect((await box("body")).y).toBeGreaterThan(narrow.y);

      await js(`__SK_TEST_CQ__.appWindow.setBounds({ width: ${outer} }); "ok"`);
      await until(
        () => box("media"),
        (b) => b.w === 120,
      );
      const observed = (await state<Sample[]>("observed"))!;
      expect(observed.some((s) => s.container < 500)).toBe(true);
      // Each ResizeObserver callback runs after the frame's layout: the card already answers the
      // width the shelf is being reported at.
      for (const s of observed) expect([s.container, s.child === 120]).toEqual([s.container, s.container >= 500]);
    },
    T,
  );

  test(
    "nested named containers answer their own queries",
    async () => {
      const before = await box("badge");
      expect([before.w, before.h]).toEqual([20, 30]);
      await js("document.getElementById('panel').style.width = '450px'; 'ok'");
      const after = await until(
        () => box("badge"),
        (b) => b.w !== 20,
      );
      expect([after.w, after.h]).toEqual([80, 30]);
      await js("document.getElementById('page').style.width = '400px'; 'ok'");
      const narrow = await until(
        () => box("badge"),
        (b) => b.h !== 30,
      );
      expect([narrow.w, narrow.h]).toEqual([80, 10]);
    },
    T,
  );

  test(
    "container query units resolve against the container's content box",
    async () => {
      const units = await box("units");
      const content = { w: units.w - 20, h: units.h - 20 };
      const u = await box("u");
      expect(u.w).toBeCloseTo(content.w / 2, 1);
      expect(u.h).toBeCloseTo(content.h / 2, 1);
      expect((await box("u-min")).w).toBeCloseTo(Math.min(content.w, content.h) / 10, 1);
      await js("document.getElementById('units').style.width = '300px'; 'ok'");
      const resized = await until(
        () => box("u"),
        (b) => Math.abs(b.w - u.w) > 1,
      );
      expect(resized.w).toBeCloseTo((await box("units")).w / 2 - 10, 1);
    },
    T,
  );

  test(
    "a container a transition resizes flips its query in the frame it crosses",
    async () => {
      await js("__SK_TEST_CQ__.grow(true); 'ok'");
      await until(
        () => box("grow"),
        (b) => b.w === 600,
        3_000,
      );
      await sleep(100);
      await js("__SK_TEST_CQ__.sampling = false; 'ok'");
      const samples = (await state<Sample[]>("sampled"))!;
      const between = samples.filter((s) => s.container > 200 && s.container < 600);
      expect(between.length).toBeGreaterThan(3);
      expect(samples.some((s) => s.child === 10)).toBe(true);
      expect(samples.at(-1)).toEqual({ container: 600, child: 50 });
      for (const s of samples) expect([s.container, s.child === 50]).toEqual([s.container, s.container >= 400]);
    },
    T,
  );

  test(
    "frames that move no container do no container restyle",
    async () => {
      await sleep(300);
      const before = (await app.send("perf")) as { containerRelayouts: number; containersMoved: number };
      await sleep(1_000);
      const after = (await app.send("perf")) as { containerRelayouts: number; containersMoved: number };
      expect((await box("beat")).w).toBe(100);
      expect(after.containersMoved - before.containersMoved).toBe(0);
      expect(after.containerRelayouts - before.containerRelayouts).toBe(0);
    },
    T,
  );
});
