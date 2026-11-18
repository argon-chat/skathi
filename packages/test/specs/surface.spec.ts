import { afterAll, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import {
  EXE_SUFFIX,
  launch,
  MACOS,
  readPng,
  REPO,
  SCREENSHOTS,
  TARGET,
  WINDOWS,
  type Bounds,
  type SkApp,
} from "../src/index";

const EXE = join(REPO, "target", TARGET, "debug", "examples", `surface${EXE_SUFFIX}`);
const build = spawnSync("cargo", ["build", "-p", "sk-app", "--target", TARGET, "--example", "surface"], {
  cwd: REPO,
  encoding: "utf8",
  timeout: 900_000,
  windowsHide: true,
});
if (build.status !== 0) throw new Error(`building the surface example failed:\n${build.stderr}`);

const open = (label: string, env?: Record<string, string>) =>
  launch({
    app: "packages/test/fixtures/surface/src/main.ts",
    exe: EXE,
    window: [720, 540],
    label,
    ...(env ? { env } : {}),
  });
const app = await open("surface");
afterAll(() => app.close());

const T = 60_000;
type Rgb = [number, number, number];
type Stats = {
  presented: number;
  shown: number;
  width: number;
  height: number;
  visible: boolean;
  dynamicRangeLimit: string;
  contentHeadroom: number;
  display: {
    known: boolean;
    hdr: boolean;
    headroom: number;
    potentialHeadroom: number;
    outputHeadroom: number;
    outputHdr: boolean;
    refreshHz: number | null;
  };
};

/** `appHost.call(name, ...args)` in `on`'s page; its settled value. */
async function callOn(on: SkApp, name: string, ...args: unknown[]): Promise<unknown> {
  await on.eval(
    `globalThis.__sv = undefined; globalThis.hostCall(${JSON.stringify(name)}, ...${JSON.stringify(args)}).then(` +
      `v => { globalThis.__sv = JSON.stringify({ ok: v }); }, e => { globalThis.__sv = JSON.stringify({ err: String(e && e.message) }); }); 0`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await on.eval("globalThis.__sv === undefined ? '' : globalThis.__sv");
    if (raw) {
      const settled = JSON.parse(String(raw)) as { ok?: unknown; err?: string };
      if (settled.err !== undefined) throw new Error(`${name}: ${settled.err}`);
      return settled.ok;
    }
    await Bun.sleep(30);
  }
  throw new Error(`${name} never settled`);
}

const call = (name: string, ...args: unknown[]) => callOn(app, name, ...args);

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

const near = (got: Rgb, want: Rgb, by = 3) => got.every((c, i) => Math.abs(c - want[i]!) <= by);
const WHITE: Rgb = [255, 255, 255];

// gpui's Linux backend has no render_to_image: there the pixel checks are skipped.
test.skipIf(!SCREENSHOTS)(
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
    if (!SCREENSHOTS) return;
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
    if (!SCREENSHOTS) return;
    const b = await box("#live");
    const first = (await shoot())(b.x + b.w / 2, b.y + b.h / 2);
    const moved = await until(shoot, (p) => !near(p(b.x + b.w / 2, b.y + b.h / 2), first), 3_000);
    expect(near(moved(b.x + b.w / 2, b.y + b.h / 2), first)).toBe(false);
  },
  T,
);

/** Presents each `[b, g, r, a]` frame in the mode its element is named after, and checks it fills its box. */
async function handOver(colors: Record<string, [number, number, number, number]>, tolerance = 3) {
  for (const [name, color] of Object.entries(colors)) await call("present", name, name, color, 48, 32);
  for (const name of Object.keys(colors))
    await until(
      () => stats(name),
      (s) => s.shown === 1,
    );
  const px = await shoot();
  for (const [name, [b, g, r]] of Object.entries(colors)) {
    const at = await box(`#${name}`);
    expect(near(px(at.x + at.w / 2, at.y + at.h / 2), [r, g, b], tolerance), name).toBe(true);
    expect(near(px(at.x + 4, at.y + 4), [r, g, b], tolerance), `${name} fills its box`).toBe(true);
  }
  const events = (await app.eval("globalThis.surfaceEvents.join('|')")) as string;
  for (const name of Object.keys(colors)) expect(events).toContain(`${name}:ready:48x32`);
}

test.skipIf(!WINDOWS)(
  "D3D12 producers on the renderer's device and on their own, and a D3D11 fence, hand frames over",
  async () => {
    // Each mode waits on the CPU until the surface released its texture (or fence value 2).
    await handOver({
      d3d12: [255, 0, 255, 255],
      "d3d12-shared": [0, 128, 255, 255],
      "d3d11-fence": [64, 192, 0, 255],
    });
  },
  T,
);

test.skipIf(!MACOS)(
  "Metal textures, IOSurfaces and CVPixelBuffers hand frames over",
  async () => {
    // metal and iosurface wait on the CPU until the surface signalled their release value.
    await handOver({
      metal: [255, 0, 255, 255],
      iosurface: [0, 128, 255, 255],
      "mach-port": [64, 192, 0, 255],
      "cv-bgra": [200, 40, 90, 255],
    });
    // 4:2:0 video range rounds each channel a little.
    await handOver({ "cv-nv12": [40, 160, 220, 255] }, 6);
  },
  T,
);

/**
 * Presents linear half-float frames in the mode each element is named after, alternating two colours: values above
 * 1.0 clamp and the rest go through the exact sRGB curve, so they show as these 8-bit colours.
 */
async function handOverHalf(names: string[]) {
  const colors: [[number, number, number, number], Rgb][] = [
    [
      [4.0, 0.2159, 0.0, 1.0],
      [255, 128, 0],
    ],
    [
      [0.05, 0.5, 1.5, 1.0],
      [63, 188, 255],
    ],
  ];
  for (const [i, name] of names.entries()) await call("present", name, name, colors[i % 2]![0], 48, 32);
  for (const name of names)
    await until(
      () => stats(name),
      (s) => s.shown === 1,
    );
  const px = await shoot();
  for (const [i, name] of names.entries()) {
    const at = await box(`#${name}`);
    expect(px(at.x + at.w / 2, at.y + at.h / 2), name).toEqual(colors[i % 2]![1]);
  }
  const events = (await app.eval("globalThis.surfaceEvents.join('|')")) as string;
  for (const name of names) expect(events).toContain(`${name}:ready:48x32`);
  // A visible half-float surface that may pass SDR white puts the window in HDR, and both sides hear it.
  // macOS keeps the current headroom at 1.0 until a window asks for EDR, so this goes by the potential one.
  const first = names[0]!;
  const wanted = (s: Stats) => s.display.hdr && s.display.potentialHeadroom > 1;
  const s = await until(
    () => stats(first),
    (s) => s.display.outputHdr === wanted(s),
  );
  expect(s.dynamicRangeLimit).toBe("no-limit");
  expect(s.display.outputHdr).toBe(wanted(s));
  const heard = ((await app.eval("globalThis.surfaceEvents.join('|')")) as string)
    .split("|")
    .filter((e) => e.startsWith(`${first}:display:`));
  expect(heard.at(-1)).toEndWith(`:${s.display.outputHdr}`);
}

test.skipIf(!WINDOWS)(
  "half-float frames from the CPU, D3D11, keyed-mutex, D3D12 and shared D3D12 paths show clamped through the sRGB curve",
  async () => {
    await handOverHalf(["cpu-rgba16f", "texture-rgba16f", "shared-rgba16f", "d3d12-rgba16f", "d3d12-shared-rgba16f"]);
  },
  T,
);

test.skipIf(!MACOS)(
  "half-float Metal textures, RGhA IOSurfaces and CVPixelBuffers show clamped through the sRGB curve",
  async () => {
    await handOverHalf(["cpu-rgba16f", "metal-rgba16f", "iosurface-rgba16f", "cv-rgha"]);
  },
  T,
);

test.skipIf(!WINDOWS && !MACOS)(
  "the display an element is on reaches the producer and the element alike",
  async () => {
    const s = await until(
      () => stats("contain"),
      (s) => s.display.known,
    );
    expect(s.display.headroom).toBeGreaterThanOrEqual(1);
    expect(s.display.refreshHz).toBeGreaterThan(0);
    const events = await until(
      () => app.eval("globalThis.surfaceEvents.join('|')") as Promise<string>,
      (e) => e.includes("contain:display:"),
    );
    const [, headroom, refresh] = /contain:display:([\d.]+):([\d.]+):/.exec(events)!;
    expect(Number(headroom)).toBeCloseTo(s.display.headroom, 2);
    expect(Number(refresh)).toBeCloseTo(s.display.refreshHz!, 2);
  },
  T,
);

type Feedback = {
  presents: number;
  events: number;
  inOrder: boolean;
  exactlyOnce: boolean;
  presented: number;
  superseded: number;
  hidden: number;
  exact: number;
  cadence: number[];
  busy: number;
  shown: number;
  error: string | null;
  visibleLate: number;
  deviceLost: number;
  /** When each `SurfaceEvent::DeviceLost` arrived, `monotonic_ns`. */
  lostAt: number[];
  /** When each present began, and whether it was taken. */
  attempts: [number, boolean][];
  /** When each frame dropped as hidden was presented. */
  hiddenAt: number[];
};

type Producer = {
  name: string;
  source: "bgra" | "d3d12" | "d3d12-rgba16f" | "shared";
  ring: number;
  on?: SkApp;
};

async function feedback(
  fps: number,
  ms: number,
  paced: boolean,
  during?: () => Promise<void>,
  { name, source, ring, on = app }: Producer = { name: "feedback", source: "bgra", ring: 3 },
): Promise<Feedback> {
  await callOn(on, "feedbackStart", name, fps, paced, source, ring);
  await Bun.sleep(ms);
  if (during) await during();
  const f = (await callOn(on, "feedbackStop", name)) as Feedback;
  expect(f.error).toBeNull();
  return f;
}

test.skipIf(!WINDOWS)(
  "every presented frame resolves exactly once, in order, on the refresh it was shown",
  async () => {
    // Paced to the display clock, as a display-synced player is.
    const f = await feedback(24, 2_000, true);
    const { refreshHz } = (await stats("feedback")).display;
    console.log(`24 fps on ${refreshHz} Hz, ${f.exact}/${f.presented} exact; refreshes held: ${f.cadence.join(" ")}`);
    expect(f.presents).toBeGreaterThan(30);
    expect(f.exactlyOnce).toBe(true);
    expect(f.inOrder).toBe(true);
    expect(f.presented).toBeGreaterThan(f.presents * 0.9);
    // Each frame is held for the refreshes 24 fps takes on this display, rounded either way; a
    // debug build's draw may miss the odd one.
    const per = refreshHz! / 24;
    const steady = f.cadence.filter((n) => n === Math.floor(per) || n === Math.ceil(per));
    expect(steady.length).toBeGreaterThan(f.cadence.length * 0.8);
  },
  T,
);

type Phase = { count: number; p05: number; p50: number; p95: number };

test.skipIf(!WINDOWS)(
  "at 24 presents a second the tick stays on the vblank",
  async () => {
    await call("phaseStart", "feedback");
    const f = await feedback(24, 4_000, true);
    const phase = (await call("phaseStop", "feedback")) as Phase;
    const { refreshHz } = (await stats("feedback")).display;
    console.log(
      `24 fps on ${refreshHz} Hz: ${phase.count} ticks wrote a frame, at ${phase.p05.toFixed(2)}-` +
        `${phase.p95.toFixed(2)} of the refresh after its vblank (p05-p95), median ${phase.p50.toFixed(2)}`,
    );
    expect(f.exactlyOnce).toBe(true);
    expect(phase.count).toBeGreaterThan(60);
    expect(phase.p95 - phase.p05).toBeLessThan(0.3);
  },
  T,
);

test.skipIf(!WINDOWS)(
  "frames presented faster than the display refreshes are superseded",
  async () => {
    const f = await feedback(1_000, 1_000, false);
    expect(f.exactlyOnce).toBe(true);
    expect(f.inOrder).toBe(true);
    expect(f.superseded).toBeGreaterThan(f.presents / 2);
    expect(f.presented).toBeGreaterThan(0);
  },
  T,
);

test.skipIf(!WINDOWS)(
  "a D3D12 producer that outruns the display still shows a new frame on most refreshes",
  async () => {
    const ring: Producer = { name: "ring", source: "d3d12", ring: 3 };
    const s = await until(
      () => stats("ring"),
      (s) => s.visible && s.display.known,
    );
    const hz = s.display.refreshHz!;
    for (const fps of [0, hz * 1.01]) {
      const f = await feedback(fps, 2_000, false, undefined, ring);
      const perRefresh = f.shown / (hz * 2);
      console.log(
        `${fps ? fps.toFixed(1) : "unthrottled"} presents/s on ${hz} Hz: ${f.presents} presents, ${f.shown} shown ` +
          `(${perRefresh.toFixed(2)} a refresh), ${f.presented} presented, ${f.superseded} superseded, ${f.busy} busy`,
      );
      expect(f.exactlyOnce).toBe(true);
      expect(f.inOrder).toBe(true);
      expect(perRefresh).toBeGreaterThan(0.5);
      expect(f.presented).toBeGreaterThan(hz * 2 * 0.5);
    }
  },
  T,
);

test.skipIf(!WINDOWS)(
  "a ring of five half-float textures presented as fast as it goes never leaves the surface busy",
  async () => {
    const ring: Producer = { name: "ring", source: "d3d12-rgba16f", ring: 5 };
    const flood = await feedback(0, 1_000, false, undefined, ring);
    const paced = await feedback(60, 1_000, false, undefined, ring);
    const { display } = await stats("ring");
    console.log(
      `HDR output ${display.outputHdr}: flood ${flood.presents} presents, ${flood.busy} busy, ${flood.shown} shown; ` +
        `60 fps ${paced.presents} presents, ${paced.busy} busy, ${paced.shown} shown`,
    );
    expect(flood.shown).toBeGreaterThan(0);
    expect(paced.busy).toBe(0);
    expect(paced.presented).toBeGreaterThan(paced.presents * 0.9);
  },
  T,
);

test.skipIf(!WINDOWS)(
  "frames presented while the window is minimised are dropped as hidden",
  async () => {
    const { dlopen, FFIType } = await import("bun:ffi");
    const user32 = dlopen("user32.dll", {
      ShowWindow: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i32 },
      IsIconic: { args: [FFIType.u64], returns: FFIType.i32 },
    });
    const hwnd = await app.hwnd();
    try {
      const f = await feedback(60, 300, false, async () => {
        await app.eval("__sk_window_verb('minimize'); 'ok'");
        const by = Date.now() + 10_000;
        while (user32.symbols.IsIconic(hwnd) === 0 && Date.now() < by) await Bun.sleep(50);
        expect(user32.symbols.IsIconic(hwnd)).not.toBe(0);
        await Bun.sleep(500);
      });
      expect(f.exactlyOnce).toBe(true);
      expect(f.inOrder).toBe(true);
      console.log(
        `minimised: ${f.hidden} hidden, ${f.visibleLate} presented after one was hidden while info().visible lagged`,
      );
      expect(f.presented).toBeGreaterThan(0);
      expect(f.hidden).toBeGreaterThan(10);
      expect(f.visibleLate).toBeLessThan(3);
    } finally {
      user32.symbols.ShowWindow(hwnd, 4 /* SW_SHOWNOACTIVATE */);
      user32.close();
    }
    await until(
      () => stats("feedback"),
      (s) => s.visible,
    );
  },
  T,
);

test.skipIf(!WINDOWS)(
  "frames under a topmost window that does not take the foreground are dropped as hidden",
  async () => {
    const { dlopen, FFIType, ptr } = await import("bun:ffi");
    const user32 = dlopen("user32.dll", {
      CreateWindowExW: {
        args: [
          FFIType.u32,
          FFIType.ptr,
          FFIType.ptr,
          FFIType.u32,
          FFIType.i32,
          FFIType.i32,
          FFIType.i32,
          FFIType.i32,
          FFIType.u64,
          FFIType.u64,
          FFIType.u64,
          FFIType.ptr,
        ],
        returns: FFIType.u64,
      },
      ShowWindow: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i32 },
      DestroyWindow: { args: [FFIType.u64], returns: FFIType.i32 },
      GetWindowRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
      GetForegroundWindow: { args: [], returns: FFIType.u64 },
      SetWindowPos: {
        args: [FFIType.u64, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.u32],
        returns: FFIType.i32,
      },
    });
    // Occlusion is off in test hosts unless asked for.
    const watched = await open("surface-occlusion", { SK_OCCLUSION: "1" });
    let cover = 0n;
    try {
      const ring: Producer = { name: "feedback", source: "bgra", ring: 3, on: watched };
      const hwnd = await watched.hwnd();
      // Above the other windows, without activating it (SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE).
      user32.symbols.SetWindowPos(hwnd, 0n, 0, 0, 0, 0, 0x13);
      const uncovered = (
        await until(
          () => callOn(watched, "stats", "feedback") as Promise<Stats>,
          (s) => s.visible,
          2_000,
        )
      ).visible;
      const before = await feedback(60, 600, false, undefined, ring);
      const rect = new Int32Array(4);
      user32.symbols.GetWindowRect(hwnd, ptr(rect));
      const [left, top, right, bottom] = rect;
      const foreground = user32.symbols.GetForegroundWindow();
      // WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, WS_POPUP: shown without activation.
      cover = BigInt(
        user32.symbols.CreateWindowExW(
          0x0800_0088,
          ptr(Buffer.from("STATIC\0", "utf16le")),
          null,
          0x8000_0000,
          left! - 8,
          top! - 8,
          right! - left! + 16,
          bottom! - top! + 16,
          0n,
          0n,
          0n,
          null,
        ),
      );
      expect(cover).not.toBe(0n);
      user32.symbols.ShowWindow(cover, 4 /* SW_SHOWNOACTIVATE */);
      expect(user32.symbols.GetForegroundWindow(), "the cover took no foreground").toBe(foreground);
      const covered = await feedback(60, 1_500, false, undefined, ring);
      console.log(
        `before the cover: ${before.presented} presented, ${before.hidden} hidden of ${before.presents}; ` +
          `covered: ${covered.presented} presented, ${covered.hidden} hidden of ${covered.presents}`,
      );
      // A window of the desktop's that stays above the harness window leaves it covered throughout.
      if (uncovered) expect(before.presented).toBeGreaterThan(before.presents * 0.5);
      expect(covered.exactlyOnce).toBe(true);
      expect(covered.hidden).toBeGreaterThan(covered.presents * 0.8);
      expect((await callOn(watched, "stats", "feedback")) as Stats).toMatchObject({ visible: false });
    } finally {
      if (cover) user32.symbols.DestroyWindow(cover);
      user32.close();
      await watched.close();
    }
  },
  T,
);

test.skipIf(!WINDOWS)(
  "frames show again within a few refreshes of a restore or of the cover going away",
  async () => {
    const { dlopen, FFIType, ptr } = await import("bun:ffi");
    const user32 = dlopen("user32.dll", {
      CreateWindowExW: {
        args: [
          FFIType.u32,
          FFIType.ptr,
          FFIType.ptr,
          FFIType.u32,
          FFIType.i32,
          FFIType.i32,
          FFIType.i32,
          FFIType.i32,
          FFIType.u64,
          FFIType.u64,
          FFIType.u64,
          FFIType.ptr,
        ],
        returns: FFIType.u64,
      },
      ShowWindow: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i32 },
      DestroyWindow: { args: [FFIType.u64], returns: FFIType.i32 },
      GetWindowRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
      IsIconic: { args: [FFIType.u64], returns: FFIType.i32 },
      SetWindowPos: {
        args: [FFIType.u64, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.u32],
        returns: FFIType.i32,
      },
    });
    const kernel32 = dlopen("kernel32.dll", {
      QueryPerformanceCounter: { args: [FFIType.ptr], returns: FFIType.i32 },
      QueryPerformanceFrequency: { args: [FFIType.ptr], returns: FFIType.i32 },
    });
    // `monotonic_ns`: QPC in nanoseconds.
    const qpc = new BigInt64Array(1);
    kernel32.symbols.QueryPerformanceFrequency(ptr(qpc));
    const frequency = qpc[0]!;
    const now = () => {
      kernel32.symbols.QueryPerformanceCounter(ptr(qpc));
      const t = qpc[0]!;
      return Number((t / frequency) * 1_000_000_000n + ((t % frequency) * 1_000_000_000n) / frequency);
    };
    const watched = await open("surface-reshown", { SK_OCCLUSION: "1" });
    let cover = 0n;
    try {
      const ring: Producer = { name: "feedback", source: "bgra", ring: 3, on: watched };
      const hwnd = await watched.hwnd();
      user32.symbols.SetWindowPos(hwnd, 0n, 0, 0, 0, 0, 0x13);
      const shown = await until(
        () => callOn(watched, "stats", "feedback") as Promise<Stats>,
        (s) => s.visible,
        2_000,
      );
      // A window of the desktop's that stays above the harness window leaves it covered throughout.
      if (!shown.visible) return;
      // How long after `at` frames still dropped as hidden.
      const hiddenFor = (f: Feedback, at: number) => Math.max(0, ...f.hiddenAt.map((t) => (t - at) / 1e6));

      let restoredAt = 0;
      const restore = await feedback(
        60,
        300,
        false,
        async () => {
          await watched.eval("__sk_window_verb('minimize'); 'ok'");
          const by = Date.now() + 10_000;
          while (user32.symbols.IsIconic(hwnd) === 0 && Date.now() < by) await Bun.sleep(20);
          await Bun.sleep(1_000);
          restoredAt = now();
          user32.symbols.ShowWindow(hwnd, 4 /* SW_SHOWNOACTIVATE */);
          await Bun.sleep(600);
        },
        ring,
      );

      const rect = new Int32Array(4);
      user32.symbols.GetWindowRect(hwnd, ptr(rect));
      const [left, top, right, bottom] = rect;
      let uncoveredAt = 0;
      const uncover = await feedback(
        60,
        300,
        false,
        async () => {
          cover = BigInt(
            user32.symbols.CreateWindowExW(
              0x0800_0088,
              ptr(Buffer.from("STATIC\0", "utf16le")),
              null,
              0x8000_0000,
              left! - 8,
              top! - 8,
              right! - left! + 16,
              bottom! - top! + 16,
              0n,
              0n,
              0n,
              null,
            ),
          );
          user32.symbols.ShowWindow(cover, 4 /* SW_SHOWNOACTIVATE */);
          await Bun.sleep(500);
          uncoveredAt = now();
          user32.symbols.DestroyWindow(cover);
          cover = 0n;
          await Bun.sleep(600);
        },
        ring,
      );
      console.log(
        `hidden after a restore for ${hiddenFor(restore, restoredAt).toFixed(1)} ms (${restore.hidden} hidden), ` +
          `after the cover went away for ${hiddenFor(uncover, uncoveredAt).toFixed(1)} ms (${uncover.hidden} hidden)`,
      );
      expect(restore.hidden).toBeGreaterThan(0);
      expect(uncover.hidden).toBeGreaterThan(0);
      expect(hiddenFor(restore, restoredAt)).toBeLessThan(100);
      expect(hiddenFor(uncover, uncoveredAt)).toBeLessThan(100);
    } finally {
      if (cover) user32.symbols.DestroyWindow(cover);
      user32.close();
      kernel32.close();
      await watched.close();
    }
  },
  T,
);

const privateBytes = (pid: number) =>
  Number(
    spawnSync("powershell", ["-NoProfile", "-Command", `(Get-Process -Id ${pid}).PrivateMemorySize64`], {
      encoding: "utf8",
      windowsHide: true,
    }).stdout.trim(),
  );

test.skipIf(!WINDOWS)(
  "a removed renderer device is reported at once, refuses presents, and is replaced",
  async () => {
    const removed = await open("surface-device-removed");
    try {
      const shared: Producer = { name: "feedback", source: "shared", ring: 1, on: removed };
      await until(
        () => callOn(removed, "stats", "feedback") as Promise<Stats>,
        (s) => s.visible,
      );
      let at = 0;
      const bytes = [0, 0, 0];
      // As fast as it goes, so a frame is usually waiting for the window when the device goes.
      const f = await feedback(
        0,
        500,
        false,
        async () => {
          bytes[0] = privateBytes(removed.pid);
          at = Number(await callOn(removed, "removeDevice", "feedback"));
          await Bun.sleep(2_000);
          bytes[1] = privateBytes(removed.pid);
          await Bun.sleep(6_000);
          bytes[2] = privateBytes(removed.pid);
        },
        shared,
      );
      const [lost, recovered = Infinity] = f.lostAt;
      const taken = (from: number, to: number) => f.attempts.filter(([t, ok]) => ok && t >= from && t < to).length;
      console.log(
        `device lost ${lost === undefined ? "never" : `${((lost - at) / 1e6).toFixed(0)} ms`} after the removal, ` +
          `a new one ${((recovered - at) / 1e6).toFixed(0)} ms after it; ${f.deviceLost} frames dropped as deviceLost; ` +
          `${taken(lost ?? at, recovered)} presents taken in between, ${taken(recovered, Infinity)} after; ` +
          `private bytes ${bytes.map((b) => (b / 2 ** 20).toFixed(0)).join(" -> ")} MiB`,
      );
      expect(lost! - at).toBeLessThan(2e9);
      expect(f.exactlyOnce).toBe(true);
      // The new device is taken just before its DeviceLost is heard.
      expect(taken(lost!, recovered - 1e6)).toBe(0);
      expect(f.lostAt.length).toBe(2);
      expect(taken(recovered, Infinity)).toBeGreaterThan(0);
      expect(bytes[2]! - bytes[1]!).toBeLessThan(32 * 2 ** 20);
      const again = await feedback(60, 1_000, false, undefined, shared);
      expect(again.exactlyOnce).toBe(true);
      expect(again.presented).toBeGreaterThan(0);
    } finally {
      await removed.close();
    }
  },
  T,
);

test(
  "clearing withdraws the frame and tells the element",
  async () => {
    const before = (await stats("fill")).shown;
    await call("present", "fill", "bgra", [255, 0, 0, 255], 32, 32);
    await until(
      () => stats("fill"),
      (s) => s.shown > before,
    );
    await call("clear", "fill");
    if (SCREENSHOTS) {
      const b = await box("#fill");
      const px = await until(shoot, (p) => near(p(b.x + b.w - 10, b.y + b.h - 10), WHITE));
      expect(px(b.x + b.w - 10, b.y + b.h - 10)).toEqual(WHITE);
    }
    const events = () => app.eval("globalThis.surfaceEvents.join('|')") as Promise<string>;
    expect(await until(events, (e) => e.includes("fill:lost:cleared"))).toContain("fill:lost:cleared");
  },
  T,
);

test(
  "the element's dynamic-range-limit reaches SurfaceInfo, inherited or its own",
  async () => {
    const set = (id: string, value: string) =>
      app.eval(`document.getElementById("${id}").style.setProperty("dynamic-range-limit", "${value}"); 0`);
    const computed = () =>
      app.eval(`getComputedStyle(document.getElementById("contain")).getPropertyValue("dynamic-range-limit")`);
    const limit = (want: string) =>
      until(
        () => stats("contain"),
        (s) => s.dynamicRangeLimit === want,
      );
    expect((await limit("no-limit")).dynamicRangeLimit).toBe("no-limit");
    try {
      await set("root", "standard");
      expect(await computed()).toBe("standard");
      const standard = await limit("standard");
      expect(standard.dynamicRangeLimit).toBe("standard");
      expect(standard.contentHeadroom).toBe(1);

      const mix = "dynamic-range-limit-mix(standard 25%, constrained 75%)";
      await set("contain", "dynamic-range-limit-mix(constrained 75%, standard 25%)");
      expect(await computed()).toBe(mix);
      expect((await limit(mix)).dynamicRangeLimit).toBe(mix);
    } finally {
      await set("root", "");
      await set("contain", "");
    }
  },
  T,
);

test.skipIf(!WINDOWS && !MACOS)(
  "dynamic-range answers for the window's display and the limit caps the headroom a frame is shown with",
  async () => {
    const s = await until(
      () => stats("contain"),
      (s) => s.display.known,
    );
    const output = s.display.hdr ? s.display.headroom : 1;
    expect(s.display.outputHeadroom).toBeCloseTo(output, 3);
    const matches = (query: string) => app.eval(`matchMedia(${JSON.stringify(query)}).matches`) as Promise<boolean>;
    for (const feature of ["dynamic-range", "video-dynamic-range"]) {
      expect(
        await until(
          () => matches(`(${feature}: high)`),
          (m) => m === s.display.hdr,
        ),
      ).toBe(s.display.hdr);
      expect(await matches(`(${feature}: standard)`)).toBe(true);
    }

    const set = (value: string) =>
      app.eval(`document.getElementById("contain").style.setProperty("dynamic-range-limit", "${value}"); 0`);
    const headroom = (limit: string) =>
      until(
        () => stats("contain"),
        (s) => s.dynamicRangeLimit === limit,
      ).then((s) => s.contentHeadroom);
    try {
      expect(await headroom("no-limit")).toBeCloseTo(output, 3);
      await set("standard");
      expect(await headroom("standard")).toBe(1);
      await set("constrained");
      expect(await headroom("constrained")).toBeCloseTo(Math.min(2, output), 3);
    } finally {
      await set("");
    }
  },
  T,
);
