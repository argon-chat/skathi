import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/view-transition/src/main.ts",
  window: [600, 400],
  label: "view-transition",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Rgb = [number, number, number];

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(30);
    v = await read();
  }
  return v;
}

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 600;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};

const near = (rgb: Rgb, want: Rgb, tolerance = 24) => rgb.every((c, i) => Math.abs(c - want[i]!) <= tolerance);

const log = (): Promise<string[]> => app.eval("window.__log");

const start = (options: string) =>
  app.eval(`(() => {
    window.__log = [];
    const t = document.startViewTransition(${options});
    window.__vt = t;
    for (const k of ["updateCallbackDone", "ready", "finished"])
      t[k].then(() => __log.push(k), (e) => __log.push(k + ":" + e.name));
    return document.activeViewTransition === t;
  })()`);

describe("document.startViewTransition", () => {
  test(
    "the old state stays on screen while the update runs, then the transition plays out",
    async () => {
      expect(await start(`() => { window.__flip(); return new Promise((r) => (window.__thaw = r)); }`)).toBe(true);
      await sleep(300);
      const frozen = await shoot();
      expect(near(frozen(10, 380), [255, 255, 255])).toBe(true);
      expect(near(frozen(60, 60), [255, 0, 0])).toBe(true);
      expect(await log()).toEqual([]);
      await app.eval("window.__thaw()");
      await until(log, (l) => l.includes("finished"));
      expect(await log()).toEqual(["updateCallbackDone", "ready", "finished"]);
      expect(await app.eval("document.activeViewTransition")).toBe(null);
      const at = await until(shoot, (s) => near(s(450, 250), [0, 0, 255]));
      expect(near(at(450, 250), [0, 0, 255])).toBe(true);
      expect(near(at(10, 380), [0, 0, 0])).toBe(true);
    },
    T,
  );

  test(
    "old and new captures cross-fade under the author's timing, and skipTransition ends it",
    async () => {
      expect(await start(`{ update: () => window.__flip(), types: ["hold"] }`)).toBe(true);
      await until(log, (l) => l.includes("ready"));
      expect(await log()).toEqual(["updateCallbackDone", "ready"]);
      // Halfway: the root cross-fades black into white; the card's group is halfway from 200x100
      // at (300, 200) to 100x100 at (40, 40), its old image 2:1, its new one square.
      await sleep(2700);
      const at = await shoot();
      expect(near(at(10, 380), [128, 128, 128])).toBe(true);
      expect(near(at(60, 60), [128, 128, 128])).toBe(true);
      expect(near(at(450, 250), [128, 128, 128])).toBe(true);
      expect(near(at(245, 150), [128, 0, 128], 40)).toBe(true);
      const [r, g, b] = at(245, 240);
      expect(r > 160 && g < 100 && b < 100).toBe(true);
      expect(await app.eval("document.activeViewTransition === window.__vt")).toBe(true);

      await app.eval("window.__vt.skipTransition()");
      await until(log, (l) => l.includes("finished"));
      expect(await log()).toEqual(["updateCallbackDone", "ready", "finished"]);
      expect(await app.eval("document.activeViewTransition")).toBe(null);
      const after = await until(shoot, (s) => near(s(60, 60), [255, 0, 0]));
      expect(near(after(60, 60), [255, 0, 0])).toBe(true);
      expect(near(after(450, 250), [255, 255, 255])).toBe(true);
    },
    T,
  );

  test(
    "a second transition skips the first, whose update still runs",
    async () => {
      await app.eval(`(() => {
        window.__outcome = null;
        const seen = [];
        const a = document.startViewTransition(() => { seen.push("a.update"); });
        const b = document.startViewTransition(() => { seen.push("b.update"); });
        (async () => {
          const ready = await a.ready.then(() => "ready", (e) => e.name);
          await a.finished;
          await b.finished;
          window.__outcome = [ready, ...seen].join();
        })();
      })()`);
      const outcome = await until(
        () => app.eval("window.__outcome"),
        (v) => v !== null,
      );
      expect(outcome).toBe("AbortError,a.update,b.update");
    },
    T,
  );
});
