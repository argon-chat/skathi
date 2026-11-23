import { afterAll, expect, test } from "bun:test";
import { type Bounds, launch, ON_SCREEN, readPng, SCREENSHOTS } from "../src/index";

const W = 480;
const app = await launch({
  app: "packages/test/fixtures/lottie/src/main.ts",
  window: [W, 240],
  label: "lottie",
});

const steps = await Bun.file(new URL("../fixtures/lottie/src/steps.json", import.meta.url)).text();
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(req) {
    if (new URL(req.url).pathname !== "/steps.json") return new Response("no", { status: 404 });
    return new Response(steps, { headers: { "content-type": "application/json" } });
  },
});
afterAll(async () => {
  server.stop(true);
  await app.close();
});

const T = 60_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Rgb = [number, number, number];

async function box(selector: string): Promise<Bounds> {
  const { nodes } = await app.query(selector);
  const b = nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
}

async function centres(boxes: Bounds[]): Promise<Rgb[]> {
  const shot = readPng(await app.screenshot(undefined, ON_SCREEN));
  const k = shot.width / W;
  return boxes.map((b) => shot.at(Math.round((b.x + b.w / 2) * k), Math.round((b.y + b.h / 2) * k)));
}

const near = (got: Rgb, want: Rgb, tol = 10) => got.every((v, i) => Math.abs(v - want[i]!) <= tol);
/** Which second of `steps.json` a pixel shows: "r", "g", "b", or "?" for anything else. */
const step = ([r, g, b]: Rgb) =>
  r > 200 && g < 60 && b < 60 ? "r" : g > 200 && r < 60 && b < 60 ? "g" : b > 200 && r < 60 && g < 60 ? "b" : "?";

const events = async () => JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_LOTTIE__.events)"))) as string[];
async function until(what: string, ok: () => Promise<boolean>, ms = 15_000) {
  const end = Date.now() + ms;
  while (!(await ok())) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

test.skipIf(!SCREENSHOTS)(
  "a <lottie> paints once its load, off the UI thread, lands",
  async () => {
    await until("#play painted", async () => step((await centres([await box("#play")]))[0]!) !== "?");
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "a playing <lottie> moves through its loop, by presents and not builds",
  async () => {
    const b = await box("#play");
    const seen = new Set<string>();
    for (let i = 0; i < 12; i++) {
      seen.add(step((await centres([b]))[0]!));
      await sleep(270);
    }
    expect(
      [...seen]
        .filter((s) => s !== "?")
        .sort()
        .join(""),
    ).toBe("bgr");

    const a = (await app.send("perf")) as { builds: number; presents: number };
    const f0 = Number(await app.eval("__SK_TEST_LOTTIE__.frames.play"));
    await sleep(1000);
    const z = (await app.send("perf")) as { builds: number; presents: number };
    const f1 = Number(await app.eval("__SK_TEST_LOTTIE__.frames.play"));
    console.log(
      `[lottie] 1 s: ${z.presents - a.presents} presents, ${z.builds - a.builds} builds, ${f1 - f0} enterFrame`,
    );
    expect(z.presents - a.presents).toBeGreaterThanOrEqual(10);
    expect(z.builds - a.builds).toBeLessThanOrEqual(1);
    expect(f1 - f0).toBeGreaterThanOrEqual(20);
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  'loop="1" plays twice, then completes: one loopComplete, one complete',
  async () => {
    await until("complete:count", async () => (await events()).some((e) => e.startsWith("complete:count")), 10_000);
    await sleep(300);
    const mine = (await events()).filter((e) => e.includes(":count"));
    expect(mine).toEqual(["loopComplete:count:1", "complete:count:1"]);
    expect(step((await centres([await box("#count")]))[0]!)).toBe("b");
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "no autoplay holds the first frame; a segment plays only its frames; pixels are straight alpha",
  async () => {
    const [still, seg, half] = await centres(await Promise.all(["#still", "#seg", "#half"].map(box)));
    expect(step(still!)).toBe("r");
    expect(step(seg!)).toBe("g");
    // 50% black over white: a premultiplied pipeline that skipped the divide would read ~64.
    expect(near(half!, [128, 128, 128])).toBe(true);
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "autoplay is edge-triggered: a change plays, a change back pauses where it is",
  async () => {
    const b = await box("#toggle");
    expect(step((await centres([b]))[0]!)).toBe("r");
    await app.eval("__SK_TEST_LOTTIE__.state.toggle = true; 'ok'");
    await until("#toggle left red", async () => step((await centres([b]))[0]!) !== "r", 5000);
    await app.eval("__SK_TEST_LOTTIE__.state.toggle = false; 'ok'");
    // The pause lands with the next frame that syncs the element, which a slow host can take a
    // while to reach; the frame it stops on is drawn and presented after that.
    const entered = async () => Number(await app.eval("__SK_TEST_LOTTIE__.frames.toggle"));
    let last = await entered();
    let still = 0;
    await until("#toggle stopped entering frames", async () => {
      const now = await entered();
      still = now === last ? still + 1 : 0;
      last = now;
      return still >= 3;
    });
    const held = step((await centres([b]))[0]!);
    await sleep(1200);
    expect(await entered()).toBe(last);
    expect(step((await centres([b]))[0]!)).toBe(held);
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "an http(s) src loads, off the UI thread, and plays",
  async () => {
    await app.eval(`__SK_TEST_LOTTIE__.add("net", "http://127.0.0.1:${server.port}/steps.json", 100); 'ok'`);
    const b = await box("#net");
    await until("#net painted", async () => step((await centres([b]))[0]!) !== "?", 10_000);
    const seen = new Set<string>();
    for (let i = 0; i < 12; i++) {
      seen.add(step((await centres([b]))[0]!));
      await sleep(270);
    }
    expect([...seen].filter((s) => s !== "?").length).toBeGreaterThanOrEqual(2);
  },
  T,
);
