// <sk-layer> pictures: backdrop readers over a layer see the frames of the surface the layer names
// as its picture; the layer's proxy reports frames only while a reader samples it.
import { afterAll, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { EXE_SUFFIX, launch, readPng, REPO, SCREENSHOTS, TARGET, type Bounds } from "../src/index";

const EXE = join(REPO, "target", TARGET, "debug", "examples", `surface${EXE_SUFFIX}`);
const build = spawnSync("cargo", ["build", "-p", "sk-app", "--target", TARGET, "--example", "surface"], {
  cwd: REPO,
  encoding: "utf8",
  timeout: 900_000,
  windowsHide: true,
});
if (build.status !== 0) throw new Error(`building the surface example failed:\n${build.stderr}`);

const app = await launch({
  app: "packages/test/fixtures/layer-picture/src/main.ts",
  exe: EXE,
  window: [360, 240],
  label: "layer-picture",
  env: { SK_WINDOW_FOCUS: "never" },
});
afterAll(() => app.close());
const scale = Number(await app.eval("devicePixelRatio"));

const T = 60_000;
type Rgb = [number, number, number];
type Stats = { presented: number; shown: number; visible: boolean };

async function call(name: string, ...args: unknown[]): Promise<unknown> {
  await app.eval(
    `globalThis.__sv = undefined; globalThis.hostCall(${JSON.stringify(name)}, ...${JSON.stringify(args)}).then(` +
      `v => { globalThis.__sv = JSON.stringify({ ok: v }); }, e => { globalThis.__sv = JSON.stringify({ err: String(e && e.message) }); }); 0`,
  );
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const raw = await app.eval("globalThis.__sv === undefined ? '' : globalThis.__sv");
    if (raw) {
      const settled = JSON.parse(String(raw)) as { ok?: unknown; err?: string };
      if (settled.err !== undefined) throw new Error(`${name}: ${settled.err}`);
      return settled.ok;
    }
    await Bun.sleep(30);
  }
  throw new Error(`${name} never settled`);
}

const stats = () => call("stats", "pictured") as Promise<Stats>;
const heard: string[] = [];
const log = async () => (heard.push(...((await call("pictureLog")) as string[])), heard);

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 5_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await Bun.sleep(50);
    v = await read();
  }
  return v;
}

const box = async (selector: string): Promise<Bounds> => {
  const b = (await app.query(selector)).nodes[0]?.bounds;
  if (!b) throw new Error(`${selector} has no box`);
  return b;
};

const shoot = async () => {
  const shot = readPng(await app.screenshot());
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * scale), Math.floor((y + 0.5) * scale));
};

const red = ([r, g, b]: Rgb) => r > 200 && g < 140 && b < 140;

test(
  "a read proxy is visible to its producer and reports what it is told",
  async () => {
    expect((await until(stats, (s) => s.visible)).visible).toBe(true);
    const told = await until(log, (l) => l.some((e) => e.startsWith("submitted:")));
    expect(told).toContain("stack:0");
    await call("present", "pictured", "bgra", [0, 0, 255, 255], 64, 64);
    const shown = await until(stats, (s) => s.shown >= 1);
    expect(shown.shown).toBeGreaterThanOrEqual(1);
    const events = await until(log, (l) => l.some((e) => e.startsWith("presented:")));
    expect(events.some((e) => e.startsWith("presented:"))).toBe(true);
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "readers show the picture under the page inside the layer's rounded box, and the page alone elsewhere",
  async () => {
    await call("present", "pictured", "bgra", [0, 0, 255, 255], 64, 64);
    await until(stats, (s) => s.shown >= 1);
    const glass = await box("#glass");
    const at = await until(shoot, (px) => red(px(glass.x + glass.w / 2, glass.y + glass.h / 2)));
    expect(red(at(glass.x + glass.w / 2, glass.y + glass.h / 2))).toBe(true);
    expect(red(at(70, 70)), "the corner reader over the layer").toBe(true);
    expect(red(at(42, 42)), "outside the layer's rounded corner").toBe(false);
    expect(red(at(220, 150)), "over the layer, outside every reader").toBe(false);
  },
  T,
);

test(
  "an <sk-surface> showing the same surface keeps taking new frames with no reader left",
  async () => {
    await app.eval("globalThis.showGlass(false); document.getElementById('corner').style.display = 'none'; 0");
    await Bun.sleep(200);
    expect((await stats()).visible).toBe(true);
    const before = (await stats()).shown;
    await call("present", "pictured", "bgra", [0, 255, 0, 255], 64, 64);
    expect((await until(stats, (s) => s.shown > before)).shown).toBeGreaterThan(before);
    if (SCREENSHOTS) {
      const inline = await box("#inline");
      const green = ([r, g, b]: Rgb) => g > 200 && r < 60 && b < 60;
      const at = await until(shoot, (px) => green(px(inline.x + inline.w / 2, inline.y + inline.h / 2)));
      expect(green(at(inline.x + inline.w / 2, inline.y + inline.h / 2))).toBe(true);
    }
  },
  T,
);

test(
  "with no reader and no element left, the surface is hidden and a new frame is dropped as such",
  async () => {
    await app.eval("document.getElementById('inline').style.display = 'none'; 0");
    expect((await until(stats, (s) => !s.visible)).visible).toBe(false);
    await log();
    heard.length = 0;
    await call("present", "pictured", "bgra", [0, 255, 0, 255], 64, 64);
    const events = await until(log, (l) => l.some((e) => e.startsWith("dropped:")));
    expect(events.some((e) => e.endsWith(":Hidden"))).toBe(true);
  },
  T,
);
