// Records the docs' demo clips from a demo app on the secondary monitor.
//
//   bun scripts/record/record.ts <app> [clip...] --out <dir> [options]
//
//   <app>            a plan in scripts/record/plans/<app>.ts (the app is apps/<plan.app>)
//   clip...          the clips to record; all of the plan's when none is named
//   --out <dir>      where the lossless captures go (large: keep it off a nearly full drive)
//   --media <dir>    where .webm, .mp4 and -poster.webp go; docs/public/media/<plan.group> by default
//   --display <dev>  the monitor, e.g. \\.\DISPLAY1; by default the only non-primary one
//   --build          cargo build the host (and the surface example, if a clip needs it) first
//   --shots          one still per clip into <out>/shots, nothing recorded
//   --encode-only    re-encode existing captures
//   --hold <s>       launch, place and show the first clip, then wait, for poking with devtools
//
// Before launching it checks the monitor (never the primary), that the monitor has a wallpaper
// (the acrylic shows it), and after launching that the window's backdrop is not "none"; an
// `opaque` plan skips those two. Every window is moved to the monitor before it is first shown and is never activated; input is
// synthetic (see driver.ts), and the pointer is drawn by the app.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { DEFAULT_EXE, launch, REPO, TARGET, WORK_DIR, type SkApp } from "../../packages/test/src/index";
import { Driver } from "./driver";
import { encode, startCapture, still, stopCapture } from "./ffmpeg";
import { centre, earlyPlacer, keepActive, recordingMonitor, sleep, wallpapers, type Monitor, type Rect } from "./win32";

export type Loop = { start: number; end: number };
export type Clip = {
  plan: (d: Driver) => Promise<Loop>;
  /** Where the poster frame is, as a fraction of the clip. */
  poster: number;
  /** How long the page gets after `__demo.show` before the capture starts, ms. */
  settle?: number;
  /** `surface`: run on the `surface` example of sk-app, which presents frames to `<sk-surface>`. */
  host?: "surface";
  /** Readies the page instead of `__demo.show(id)`. */
  show?: (app: SkApp) => Promise<void>;
};
export type AppPlans = {
  /** A name under `apps/`, or an entry module relative to the repo root. */
  app: string;
  /** The folder under docs/public/media, "" for the folder itself. */
  group: string;
  window: [number, number];
  /**
   * `opaque`: a finished app recorded as it ships, for the landing page. No wallpaper or backdrop
   * checks, and no margin round the window.
   */
  backdrop?: "acrylic" | "opaque";
  /** Makes the app before launch (from a template, say); returns its entry, its environment and a cleanup. */
  prepare?: () => { app: string; env?: Record<string, string>; dispose(): void };
  clips: Record<string, Clip>;
};

const MARGIN_CSS = 48;
const SURFACE_EXE = join(REPO, "target", TARGET, "debug", "examples", "surface.exe");

const log = (...a: unknown[]) => console.log("[record]", ...a);
const argv = process.argv.slice(2);
const VALUED = new Set(["--out", "--media", "--display", "--hold"]);
const option = (name: string) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
};
const flag = (name: string) => argv.includes(name);
const positional = argv.filter((a, i) => !a.startsWith("--") && !VALUED.has(argv[i - 1] ?? ""));
const [planName, ...requested] = positional;
if (!planName)
  throw new Error(
    "usage: bun scripts/record/record.ts <app> [clip...] --out <dir> [--media <dir>] [--display <dev>] [--build] [--shots] [--encode-only] [--hold <s>]",
  );

const { plans } = (await import(`./plans/${planName}.ts`)) as { plans: AppPlans };
const out = option("--out");
if (!out) throw new Error("--out <dir> is required: the captures are large and lossless");
const OUT = resolve(out);
const MEDIA = resolve(option("--media") ?? join(REPO, "docs/public/media", plans.group));
const clips = requested.length ? requested : Object.keys(plans.clips);
for (const id of clips) {
  if (!plans.clips[id])
    throw new Error(`no clip ${id} in plans/${planName}.ts; have ${Object.keys(plans.clips).join(", ")}`);
}

function encodeClip(id: string) {
  const meta = JSON.parse(readFileSync(join(OUT, `${id}.json`), "utf8")) as Loop;
  log(`${id}: ${encode(join(OUT, `${id}.mkv`), join(MEDIA, id), meta.start, meta.end, plans.clips[id]!.poster)}`);
}

function cargo(args: string[]) {
  log(`cargo ${args.join(" ")}`);
  const r = spawnSync("cargo", args, { cwd: REPO, stdio: "inherit" });
  if (r.status !== 0) throw new Error(`cargo ${args.join(" ")} failed`);
}

function preflight(): Monitor {
  const mon = recordingMonitor(option("--display"));
  log(`monitor ${mon.device}: ${mon.width}x${mon.height} at ${mon.left},${mon.top}, ${mon.dpi} dpi`);
  if (plans.backdrop !== "opaque") {
    const wall = wallpapers().get(`${mon.left},${mon.top},${mon.width},${mon.height}`);
    if (wall === undefined) log("could not read this monitor's wallpaper; check it shows the one meant for the clips");
    else if (!wall)
      throw new Error(`${mon.device} has no wallpaper (a solid colour); the acrylic would have nothing to show`);
    else log(`wallpaper ${wall}`);
  }
  const needsSurface = clips.some((id) => plans.clips[id]!.host === "surface");
  if (flag("--build")) {
    cargo(["build", "--target", TARGET, "-p", "sk-app"]);
    if (needsSurface) cargo(["build", "--target", TARGET, "-p", "sk-app", "--example", "surface"]);
  }
  for (const exe of [DEFAULT_EXE, ...(needsSurface ? [SURFACE_EXE] : [])]) {
    if (!existsSync(exe)) throw new Error(`${exe} is missing; run with --build`);
  }
  return mon;
}

async function place(app: SkApp, hwnd: bigint, mon: Monitor): Promise<Rect> {
  const [w, h] = plans.window;
  await app.eval(`(() => {
    const win = __SK_WINDOW__.appWindow;
    if (win.isMinimized() || win.isMaximized()) win.unmaximize();
    if (!win.isVisible()) win.show();
    win.setContentBounds({ width: ${w}, height: ${h} });
    win.theme = "dark";
    return "ok";
  })()`);
  await sleep(350);
  const scale = Number(await app.eval("__SK_WINDOW__.appWindow.scaleFactor")) || 1;
  centre(hwnd, mon);
  await sleep(250);
  const client = centre(hwnd, mon);
  if (client.left < mon.left || client.left + client.width > mon.left + mon.width) {
    throw new Error(`the window is at ${client.left},${client.top}, outside ${mon.device}`);
  }
  const backdrop = String(await app.eval("__SK_WINDOW__.appWindow.effectiveBackdrop"));
  if (backdrop === "none" && plans.backdrop !== "opaque")
    throw new Error(`the window has no backdrop material; set "backdrop" in apps/${plans.app}/sk.config.json`);
  log(`window ${client.width}x${client.height} at ${client.left},${client.top}, backdrop ${backdrop}`);
  const m = plans.backdrop === "opaque" ? 0 : Math.round(MARGIN_CSS * scale);
  const left = Math.max(mon.left, client.left - m);
  const top = Math.max(mon.top, client.top - m);
  const right = Math.min(mon.left + mon.width, client.left + client.width + m);
  const bottom = Math.min(mon.top + mon.height, client.top + client.height + m);
  return { left, top, width: (right - left) & ~1, height: (bottom - top) & ~1 };
}

async function withHost(mon: Monitor, host: "main" | "surface", ids: string[]) {
  let exe: string | undefined;
  let image = `sk-app-sktest-${process.pid}.exe`;
  if (host === "surface") {
    image = `${plans.app}-surface-${process.pid}.exe`;
    exe = join(WORK_DIR, "bin", image);
    mkdirSync(join(WORK_DIR, "bin"), { recursive: true });
    copyFileSync(SURFACE_EXE, exe);
  }
  const placer = earlyPlacer(mon, image);
  const made = plans.prepare?.();
  let app: SkApp;
  try {
    app = await launch({
      app: made?.app ?? plans.app,
      ...(exe ? { exe } : {}),
      label: `${plans.app}-record`,
      window: plans.window,
      env: { ...made?.env, SK_WINDOW_FOCUS: "never", SK_LAUNCH_RAISE: "off" },
      launchTimeout: 120_000,
      verbose: true,
    });
  } catch (e) {
    made?.dispose();
    throw e;
  } finally {
    placer.stop();
  }
  const active = keepActive(app.pid);
  try {
    const shotPath = join(tmpdir(), `sk-record-${process.pid}.png`);
    const shot = await app.send("screenshot", { path: shotPath, backend: "render_to_image" });
    rmSync(shot.path ?? shotPath, { force: true });
    const region = await place(app, BigInt(shot.hwnd), mon);
    log(`region ${region.width}x${region.height} at ${region.left},${region.top}`);
    for (const id of ids) {
      const clip = plans.clips[id]!;
      // The synthetic pointer stays where the last clip left it; parked in a corner, it hovers nothing.
      await app.send("mouse", { x: 2, y: plans.window[1] - 2, kind: "move" });
      if (clip.show) await clip.show(app);
      else await app.eval(`__demo.show(${JSON.stringify(id)})`);
      await sleep(clip.settle ?? 1500);
      if (option("--hold")) {
        log(`holding ${option("--hold")} s on ${id}; devtools slot ${app.slotDir}`);
        await sleep(Number(option("--hold")) * 1000);
        return;
      }
      if (flag("--shots")) {
        still(region, join(OUT, "shots", `${id}.png`));
        log(`${id}: ${join(OUT, "shots", `${id}.png`)}`);
        continue;
      }
      const raw = join(OUT, `${id}.mkv`);
      log(`${id}: capturing ${region.width}x${region.height} at ${region.left},${region.top} -> ${raw}`);
      const { proc, t0 } = await startCapture(region, raw);
      await sleep(500);
      let loop: Loop;
      try {
        loop = await clip.plan(new Driver(app));
      } finally {
        await stopCapture(proc);
      }
      writeFileSync(
        join(OUT, `${id}.json`),
        JSON.stringify({ id, region, start: loop.start - t0, end: loop.end - t0 }),
      );
      encodeClip(id);
    }
  } finally {
    active.stop();
    await app.close();
    made?.dispose();
    if (exe) {
      await sleep(500);
      rmSync(exe, { force: true });
    }
  }
}

mkdirSync(OUT, { recursive: true });
if (flag("--encode-only")) {
  for (const id of clips) encodeClip(id);
} else {
  const mon = preflight();
  const main = clips.filter((id) => plans.clips[id]!.host !== "surface");
  const surface = clips.filter((id) => plans.clips[id]!.host === "surface");
  if (main.length) await withHost(mon, "main", main);
  if (surface.length && !option("--hold")) await withHost(mon, "surface", surface);
}
process.exit(0);
