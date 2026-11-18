// Capture with gdigrab and encode the docs' clip formats: 1280 wide, 30 fps, AV1 in .webm, H.264
// High in .mp4, and a .webp poster.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { sleep, type Rect } from "./win32";

const CHOCO = "C:/ProgramData/chocolatey/lib/ffmpeg/tools/ffmpeg/bin/ffmpeg.exe";
export const FFMPEG = process.env.FFMPEG ?? Bun.which("ffmpeg") ?? CHOCO;
const FFPROBE = join(dirname(FFMPEG), `ffprobe${FFMPEG.endsWith(".exe") ? ".exe" : ""}`);
const MAX_BYTES = 1.5 * 1024 * 1024;

function ff(args: string[]): void {
  const r = spawnSync(FFMPEG, ["-hide_banner", "-loglevel", "error", "-y", ...args], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`ffmpeg ${args.join(" ")}\n${r.stderr}`);
}

export function av1Encoder(): string {
  const r = spawnSync(FFMPEG, ["-hide_banner", "-encoders"], { encoding: "utf8" });
  const list = `${r.stdout ?? ""}${r.stderr ?? ""}`;
  if (list.includes("libsvtav1")) return "libsvtav1";
  if (list.includes("libaom-av1")) return "libaom-av1";
  throw new Error(`${FFMPEG} has neither libsvtav1 nor libaom-av1`);
}

const input = (region: Rect) => [
  "-f",
  "gdigrab",
  "-draw_mouse",
  "0",
  "-offset_x",
  String(region.left),
  "-offset_y",
  String(region.top),
  "-video_size",
  `${region.width}x${region.height}`,
];

/** Lossless 60 fps capture of `region`; resolves once ffmpeg is grabbing, with that moment. */
export function startCapture(region: Rect, file: string): Promise<{ proc: ChildProcess; t0: number }> {
  mkdirSync(dirname(file), { recursive: true });
  const args = [
    "-hide_banner",
    "-y",
    ...input(region),
    "-framerate",
    "60",
    "-i",
    "desktop",
    "-c:v",
    "libx264rgb",
    "-preset",
    "ultrafast",
    "-crf",
    "0",
    file,
  ];
  const proc = spawn(FFMPEG, args, { stdio: ["pipe", "ignore", "pipe"] });
  return new Promise((resolve, reject) => {
    let text = "";
    let done = false;
    proc.stderr!.on("data", (d: Buffer) => {
      text += d.toString();
      if (!done && /Press \[q\]|Output #0/.test(text)) {
        done = true;
        resolve({ proc, t0: Date.now() });
      }
    });
    proc.on("exit", (code) => {
      if (!done) reject(new Error(`ffmpeg capture exited ${code}:\n${text.slice(-2000)}`));
    });
  });
}

export async function stopCapture(proc: ChildProcess): Promise<void> {
  const exited = new Promise<void>((r) => proc.on("exit", () => r()));
  proc.stdin!.write("q");
  proc.stdin!.end();
  if ((await Promise.race([exited, sleep(15_000).then(() => "timeout")])) === "timeout") proc.kill();
}

export function still(region: Rect, file: string): void {
  mkdirSync(dirname(file), { recursive: true });
  ff([...input(region), "-i", "desktop", "-frames:v", "1", file]);
}

function duration(file: string): number {
  const r = spawnSync(FFPROBE, ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", file], {
    encoding: "utf8",
  });
  return Number(r.stdout.trim()) || 0;
}

const mb = (file: string) => `${(statSync(file).size / 1e6).toFixed(2)} MB`;

/**
 * `<raw>.mkv` trimmed to [startMs, endMs) -> `<base>.webm`, `<base>.mp4`, `<base>-poster.webp`.
 * The CRF goes up until each file is at most 1.5 MB.
 */
export function encode(raw: string, base: string, startMs: number, endMs: number, posterAt: number): string {
  const comp = raw.replace(/\.mkv$/, ".comp.mkv");
  const len = (endMs - startMs) / 1000;
  ff([
    "-ss",
    (startMs / 1000).toFixed(3),
    "-i",
    raw,
    "-t",
    len.toFixed(3),
    "-vf",
    "scale=1280:-2:flags=lanczos",
    "-c:v",
    "libx264rgb",
    "-preset",
    "veryfast",
    "-crf",
    "0",
    comp,
  ]);
  mkdirSync(dirname(base), { recursive: true });
  const common = ["-an", "-r", "30", "-pix_fmt", "yuv420p"];
  const av1 = av1Encoder();
  for (const bump of [0, 6, 12]) {
    ff(
      av1 === "libsvtav1"
        ? [
            "-i",
            comp,
            ...common,
            "-c:v",
            "libsvtav1",
            "-preset",
            "6",
            "-crf",
            String(38 + bump),
            "-g",
            "300",
            `${base}.webm`,
          ]
        : [
            "-i",
            comp,
            ...common,
            "-c:v",
            "libaom-av1",
            "-crf",
            String(34 + bump),
            "-b:v",
            "0",
            "-cpu-used",
            "4",
            "-row-mt",
            "1",
            "-tiles",
            "2x2",
            "-g",
            "300",
            `${base}.webm`,
          ],
    );
    if (statSync(`${base}.webm`).size <= MAX_BYTES) break;
  }
  for (const bump of [0, 6, 12]) {
    ff([
      "-i",
      comp,
      ...common,
      "-c:v",
      "libx264",
      "-profile:v",
      "high",
      "-preset",
      "slow",
      "-crf",
      String(26 + bump),
      "-movflags",
      "+faststart",
      `${base}.mp4`,
    ]);
    if (statSync(`${base}.mp4`).size <= MAX_BYTES) break;
  }
  const at = (duration(comp) * posterAt).toFixed(2);
  ff(["-ss", at, "-i", comp, "-frames:v", "1", "-c:v", "libwebp", "-quality", "82", `${base}-poster.webp`]);
  return `${len.toFixed(2)} s, .webm ${mb(`${base}.webm`)}, .mp4 ${mb(`${base}.mp4`)}, -poster.webp ${mb(`${base}-poster.webp`)}`;
}
