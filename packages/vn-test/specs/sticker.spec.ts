import { afterAll, expect, test } from "bun:test";
import { launch, readPng, type Bounds } from "../src/index";

const W = 480;
const app = await launch({
  app: "packages/vn-test/fixtures/sticker/src/main.ts",
  window: [W, 320],
  label: "sticker",
});

let served = 0;
const steps = await Bun.file(new URL("../fixtures/sticker/src/steps.json", import.meta.url)).text();
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(req) {
    if (new URL(req.url).pathname !== "/steps.json") return new Response("no", { status: 404 });
    served++;
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
  const shot = readPng(await app.screenshot(undefined, "print_window"));
  const k = shot.width / W;
  return boxes.map((b) => shot.at(Math.round((b.x + b.w / 2) * k), Math.round((b.y + b.h / 2) * k)));
}

const near = (got: Rgb, want: Rgb, tol = 10) => got.every((v, i) => Math.abs(v - want[i]) <= tol);
/** Which second of `steps.json` a pixel shows: "r", "g", "b", or "?" for anything else. */
const step = ([r, g, b]: Rgb) =>
  r > 200 && g < 60 && b < 60 ? "r" : g > 200 && r < 60 && b < 60 ? "g" : b > 200 && r < 60 && g < 60 ? "b" : "?";

const events = async () => JSON.parse(String(await app.eval("JSON.stringify(__VN_TEST_STICKER__.events)"))) as string[];
async function until(what: string, ok: () => Promise<boolean>, ms = 15_000) {
  const end = Date.now() + ms;
  while (!(await ok())) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

test(
  "stickers load, and a missing file is an error event",
  async () => {
    await until("load:a", async () => (await events()).includes("load:a"));
    await until("error:bad", async () => (await events()).includes("error:bad"));
    const ev = await events();
    for (const id of ["a", "b", "small", "half", "star", "off", "once", "tint", "tint-half", "deep"])
      expect(ev).toContain(`load:${id}`);
    expect(ev.filter((e) => e.endsWith(":bad"))).toEqual(["error:bad"]);
  },
  T,
);

test(
  "copies of one source, at any size, show the same frame of one clock",
  async () => {
    const ids = ["#a", "#b", "#small", "#deep"];
    const boxes = await Promise.all(ids.map(box));
    const seen = new Set<string>();
    let agree = 0;
    const n = 12;
    for (let i = 0; i < n; i++) {
      const px = (await centres(boxes)).map(step);
      seen.add(px[0]!);
      if (px.every((p) => p === px[0] && p !== "?")) agree++;
      await sleep(270);
    }
    console.log(`[sticker] ${agree}/${n} samples agree; seen ${[...seen].join("")}`);
    expect(agree).toBeGreaterThanOrEqual(n - 1);
    expect([...seen].sort().join("")).toBe("bgr");
  },
  T,
);

test(
  "pixels are straight alpha, and play=off / once hold their frame",
  async () => {
    await until("ended:once", async () => (await events()).includes("ended:once"), 10_000);
    const [half, off, once] = await centres(await Promise.all(["#half", "#off", "#once"].map(box)));
    // 50% black over white: a premultiplied pipeline that skipped the divide would read ~64.
    expect(near(half!, [128, 128, 128])).toBe(true);
    expect(step(off!)).toBe("r");
    expect(step(once!)).toBe("b");
    expect((await events()).filter((e) => e === "ended:once")).toHaveLength(1);
  },
  T,
);

test(
  "tint paints the frame in the element's colour, by the frame's alpha",
  async () => {
    const [tint, tintHalf] = await centres(await Promise.all(["#tint", "#tint-half"].map(box)));
    expect(near(tint!, [255, 128, 128])).toBe(true);
    expect(near(tintHalf!, [128, 0, 128])).toBe(true);
  },
  T,
);

test(
  "a colour change, as a theme switch makes, re-tints the frame shown",
  async () => {
    await app.eval("document.getElementById('tint').style.color = 'rgb(0, 0, 255)'; 'ok'");
    await sleep(300);
    const [tint] = await centres([await box("#tint")]);
    expect(near(tint!, [128, 128, 255])).toBe(true);
    await app.eval("document.getElementById('tint').style.color = 'rgb(255, 0, 0)'; 'ok'");
  },
  T,
);

test(
  "a sticker clipped out of sight is never loaded",
  async () => {
    await sleep(500);
    expect((await events()).some((e) => e.endsWith(":far"))).toBe(false);
  },
  T,
);

test(
  "animating stickers are presents, not builds",
  async () => {
    const a = (await app.send("perf")) as { builds: number; presents: number };
    await sleep(1000);
    const b = (await app.send("perf")) as { builds: number; presents: number };
    const builds = b.builds - a.builds;
    const presents = b.presents - a.presents;
    console.log(`[sticker] 1 s: ${presents} presents, ${builds} builds`);
    expect(presents).toBeGreaterThanOrEqual(10);
    expect(builds).toBeLessThanOrEqual(1);
  },
  T,
);

test(
  "a sticker in a cached subtree shows its new size's frames after a resize",
  async () => {
    await app.eval("__VN_TEST_STICKER__.state.deep = 56; 'ok'");
    await sleep(600);
    const deep = await box("#deep");
    expect(Math.round(deep.w)).toBe(56);
    const boxes = [await box("#a"), deep];
    let agree = 0;
    for (let i = 0; i < 6; i++) {
      const [a, d] = (await centres(boxes)).map(step);
      if (a === d && a !== "?") agree++;
      await sleep(270);
    }
    expect(agree).toBeGreaterThanOrEqual(5);
  },
  T,
);

test(
  "an http(s) source is fetched once for every copy",
  async () => {
    const src = `http://127.0.0.1:${server.port}/steps.json`;
    await app.eval(
      `__VN_TEST_STICKER__.add("net", ${JSON.stringify(src)}, 300); __VN_TEST_STICKER__.add("net2", ${JSON.stringify(src)}, 360); 'ok'`,
    );
    await until("load:net and load:net2", async () => {
      const ev = await events();
      return ev.includes("load:net") && ev.includes("load:net2");
    });
    await sleep(300);
    const [net, net2] = (await centres(await Promise.all(["#net", "#net2"].map(box)))).map(step);
    expect(net).not.toBe("?");
    expect(net2).toBe(net);
    expect(served).toBe(1);
  },
  T,
);

test(
  "the engine's stats count what is on screen",
  async () => {
    const mem = (await app.send("mem")) as { stickers: Record<string, number> | null };
    const s = mem.stickers!;
    console.log(`[sticker] mem: ${JSON.stringify(s)}`);
    expect(s.sources).toBeGreaterThanOrEqual(4);
    expect(s.failures).toBeGreaterThanOrEqual(1);
    expect(s.visibleTargets).toBeGreaterThanOrEqual(5);
    expect(s.drawn + s.fromCache).toBeGreaterThan(0);
  },
  T,
);
