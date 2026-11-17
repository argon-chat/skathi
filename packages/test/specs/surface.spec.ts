import { afterAll, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { launch, readPng, REPO, type Bounds } from "../src/index";

const EXE = join(REPO, "target", "x86_64-pc-windows-msvc", "debug", "examples", "surface.exe");
const build = spawnSync("cargo", ["build", "-p", "sk-app", "--example", "surface"], {
  cwd: REPO,
  encoding: "utf8",
  timeout: 900_000,
  windowsHide: true,
});
if (build.status !== 0) throw new Error(`building the surface example failed:\n${build.stderr}`);

const app = await launch({
  app: "packages/test/fixtures/surface/src/main.ts",
  exe: EXE,
  window: [720, 480],
  label: "surface",
});
afterAll(() => app.close());

const T = 60_000;
type Rgb = [number, number, number];
type Stats = { presented: number; shown: number; width: number; height: number; visible: boolean };

/** `appHost.call(name, ...args)` in the page; its settled value. */
async function call(name: string, ...args: unknown[]): Promise<unknown> {
  await app.eval(
    `globalThis.__sv = undefined; globalThis.hostCall(${JSON.stringify(name)}, ...${JSON.stringify(args)}).then(` +
      `v => { globalThis.__sv = JSON.stringify({ ok: v }); }, e => { globalThis.__sv = JSON.stringify({ err: String(e && e.message) }); }); 0`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
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

const stats = (name: string) => call("stats", name) as Promise<Stats>;

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
  const k = shot.width / 720;
  return (x: number, y: number): Rgb => shot.at(Math.floor((x + 0.5) * k), Math.floor((y + 0.5) * k));
};

const near = (got: Rgb, want: Rgb) => got.every((c, i) => Math.abs(c - want[i]!) <= 3);
const WHITE: Rgb = [255, 255, 255];

test(
  "frames from each path fill their element under object-fit, radius and z-order",
  async () => {
    // [b, g, r, a]
    await call("present", "fill", "bgra", [255, 0, 0, 255], 32, 32);
    await call("present", "contain", "texture", [0, 0, 255, 255], 50, 50);
    await call("present", "round", "shared", [0, 160, 0, 255], 64, 64);
    for (const name of ["fill", "contain", "round"])
      await until(
        () => stats(name),
        (s) => s.shown === 1,
      );

    const fill = await box("#fill");
    const contain = await box("#contain");
    const round = await box("#round");
    const px = await shoot();
    expect(px(fill.x + fill.w - 10, fill.y + fill.h - 10)).toEqual([0, 0, 255]);
    expect(px(fill.x + 10, fill.y + 10), "the box above it wins").toEqual(WHITE);
    expect(px(contain.x + contain.w / 2, contain.y + contain.h / 2)).toEqual([255, 0, 0]);
    expect(px(contain.x + 10, contain.y + contain.h / 2), "contain letterboxes a square").toEqual(WHITE);
    expect(near(px(round.x + round.w / 2, round.y + round.h / 2), [0, 160, 0])).toBe(true);
    expect(px(round.x + 3, round.y + 3), "the radius clips the corner").toEqual(WHITE);

    const events = (await app.eval("globalThis.surfaceEvents.join('|')")) as string;
    const s = await stats("fill");
    expect(s.width / fill.w).toBeCloseTo(s.height / fill.h, 2);
    expect(events).toContain(`fill:resize:${s.width}x${s.height}@`);
    expect(events).toContain("fill:ready:32x32");
    expect(events).toContain("contain:ready:50x50");
    expect(events).toContain("round:ready:64x64");
  },
  T,
);

test(
  "a hidden element takes no frames and shows the latest once it is shown",
  async () => {
    for (const color of [
      [0, 0, 0, 255],
      [255, 255, 0, 255],
    ])
      await call("present", "hidden", "bgra", color, 8, 8);
    await Bun.sleep(300);
    expect(await stats("hidden")).toMatchObject({ presented: 2, shown: 0, visible: false });

    await app.eval("globalThis.showHidden()");
    const shown = await until(
      () => stats("hidden"),
      (s) => s.shown === 1,
    );
    expect(shown).toMatchObject({ presented: 2, shown: 1, visible: true });
    const b = await box("#hidden");
    expect((await shoot())(b.x + b.w / 2, b.y + b.h / 2)).toEqual([0, 255, 255]);
  },
  T,
);

test(
  "a producer thread renders at the painted size, frame after frame",
  async () => {
    await call("animate", "live");
    const s = await until(
      () => stats("live"),
      (v) => v.shown >= 3,
    );
    expect(s.shown).toBeGreaterThanOrEqual(3);
    expect(await app.eval("globalThis.surfaceEvents.join('|')")).toContain(`live:ready:${s.width}x${s.height}`);
    const b = await box("#live");
    const first = (await shoot())(b.x + b.w / 2, b.y + b.h / 2);
    const moved = await until(shoot, (p) => !near(p(b.x + b.w / 2, b.y + b.h / 2), first), 3_000);
    expect(near(moved(b.x + b.w / 2, b.y + b.h / 2), first)).toBe(false);
  },
  T,
);

test(
  "D3D12 producers on the renderer's device and on their own, and a D3D11 fence, hand frames over",
  async () => {
    const colors: Record<string, [number, number, number, number]> = {
      d3d12: [255, 0, 255, 255],
      "d3d12-shared": [0, 128, 255, 255],
      "d3d11-fence": [64, 192, 0, 255],
    };
    // Each mode waits on the CPU until the surface released its texture (or fence value 2).
    for (const [name, color] of Object.entries(colors)) await call("present", name, name, color, 48, 32);
    for (const name of Object.keys(colors))
      await until(
        () => stats(name),
        (s) => s.shown === 1,
      );
    const px = await shoot();
    for (const [name, [b, g, r]] of Object.entries(colors)) {
      const at = await box(`#${name}`);
      expect(near(px(at.x + at.w / 2, at.y + at.h / 2), [r, g, b]), name).toBe(true);
      expect(near(px(at.x + 4, at.y + 4), [r, g, b]), `${name} fills its box`).toBe(true);
    }
    const events = (await app.eval("globalThis.surfaceEvents.join('|')")) as string;
    for (const name of Object.keys(colors)) expect(events).toContain(`${name}:ready:48x32`);
  },
  T,
);

test(
  "clearing withdraws the frame and tells the element",
  async () => {
    await call("clear", "fill");
    const b = await box("#fill");
    const px = await until(shoot, (p) => near(p(b.x + b.w - 10, b.y + b.h - 10), WHITE));
    expect(px(b.x + b.w - 10, b.y + b.h - 10)).toEqual(WHITE);
    expect(await app.eval("globalThis.surfaceEvents.join('|')")).toContain("fill:lost:cleared");
  },
  T,
);
